//! Pure-Rust sysroot cache key computation and native sysroot builder.
//!
//! Replaces `scripts/sysroot-cache.sh` and `scripts/build_sysroot.sh` with 100%
//! pure Rust so `cargo-mires` builds and caches sysroots identically on macOS,
//! Linux, and Windows with zero `bash`, `rsync`, or external `patch` tools.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::cli::{PanicMode, Profile};
use crate::patch;
use crate::sysroot::{scrub_cargo_env, sha256_file};
use crate::toolchain;

/// Optimizer passes in canonical execution order, matching `scripts/sysroot-cache.sh`
/// and `rcgjs-object`'s `OptimizerPass::ORDERED`.
const ORDERED_OPT_PASSES: [&str; 5] = [
    "sroa",
    "ast-dce",
    "future-poller",
    "native-representations",
    "link-dedup",
];

/// Emit-affecting backend option keys that change what object files contain.
const EMIT_KEYS: [&str; 13] = [
    "js-names",
    "js-structure",
    "js-queue",
    "js-switch",
    "js-scoped-lets",
    "js-minify",
    "js-ecma-target",
    "js-hoist-consts",
    "js-line-comments",
    "js-opt",
    "js-opt-passes",
    "js-jir",
    "js-cleanup",
];

/// Inputs needed to compute a sysroot cache key (matching `scripts/sysroot-cache.sh`).
#[derive(Clone, Debug)]
pub struct SysrootKeyInput<'a> {
    pub sources_root: &'a Path,
    pub dylib_path: &'a Path,
    pub channel: &'a str,
    pub panic: PanicMode,
    pub target_spec: Option<&'a Path>,
    pub emit_args_raw: &'a str,
    pub rust_opt_level: Option<&'a str>,
}

/// Request to ensure or build a native sysroot on disk.
#[derive(Clone, Debug)]
pub struct SysrootBuildRequest<'a> {
    /// Directory containing `patches/`, `mires-std/`, and `targets/mires32-unwind.json`.
    pub sources_root: &'a Path,
    /// Directory under which `sysroot-cache/<key>/` scratch builds are stored.
    pub cache_root: &'a Path,
    /// Directory under which final `<key>/` sysroots are installed.
    pub sysroots_dir: &'a Path,
    pub dylib_path: &'a Path,
    pub dylib_sha256: &'a str,
    pub channel: &'a str,
    pub target_spec: Option<&'a Path>,
    pub panic: PanicMode,
    pub emit_args_raw: &'a str,
    pub rust_opt_level: Option<&'a str>,
    pub clean: bool,
    pub verbose: bool,
}

/// Normalize emit-affecting backend options identically to `scripts/sysroot-cache.sh`.
pub fn normalize_sysroot_emit_args(raw: &str, panic: PanicMode) -> String {
    let mut tokens: Vec<&str> = raw.split_whitespace().collect();
    if panic == PanicMode::Unwind {
        tokens.push("js-cleanup=off");
    }

    let mut profile_expanded: Vec<String> = Vec::new();
    let mut emit_explicit: Vec<String> = Vec::new();

    for token in tokens {
        if let Some(name) = token.strip_prefix("js-profile=") {
            expand_profile_emit_args(name, &mut profile_expanded);
            continue;
        }
        let is_emit = matches!(token, "js-scoped-lets" | "js-line-comments")
            || token
                .split_once('=')
                .is_some_and(|(key, _)| EMIT_KEYS.contains(&key));
        if is_emit {
            emit_explicit.push(token.to_owned());
        }
    }

    let mut by_key: BTreeMap<String, String> = BTreeMap::new();
    for token in profile_expanded.into_iter().chain(emit_explicit) {
        let normalized = match token.strip_prefix("js-opt-passes=") {
            Some(value) => format!("js-opt-passes={}", normalize_opt_passes(value)),
            None => token,
        };
        let key = normalized
            .split_once('=')
            .map_or(normalized.as_str(), |(k, _)| k)
            .to_owned();
        if normalized == "js-opt-passes=all" {
            by_key.remove(&key);
        } else {
            by_key.insert(key, normalized);
        }
    }

    let mut sorted: Vec<String> = by_key.into_values().collect();
    sorted.sort();
    sorted.join(" ")
}

fn expand_profile_emit_args(name: &str, out: &mut Vec<String>) {
    match Profile::parse(name) {
        Some(Profile::Dev) => out.push("js-opt=off".to_owned()),
        Some(Profile::Report) => out.push("js-opt=report".to_owned()),
        Some(Profile::ReleaseSize | Profile::ReleaseBalanced) => {
            out.extend([
                "js-cleanup=on".to_owned(),
                "js-ecma-target=2018".to_owned(),
                "js-minify=on".to_owned(),
                "js-opt=on".to_owned(),
            ]);
        }
        Some(Profile::ReleaseSpeed) => {
            out.extend([
                "js-cleanup=on".to_owned(),
                "js-ecma-target=2018".to_owned(),
                "js-minify=locals".to_owned(),
                "js-opt=on".to_owned(),
            ]);
        }
        None => out.push(format!("js-profile={name}")),
    }
}

fn normalize_opt_passes(value: &str) -> String {
    if matches!(value, "all" | "none") {
        return value.to_owned();
    }
    let mut requested: BTreeSet<&str> = BTreeSet::new();
    for pass in value.split(',') {
        if !ORDERED_OPT_PASSES.contains(&pass) {
            return value.to_owned();
        }
        requested.insert(pass);
    }
    let selected: Vec<&str> = ORDERED_OPT_PASSES
        .iter()
        .copied()
        .filter(|pass| requested.contains(pass))
        .collect();
    if selected.is_empty() {
        "none".to_owned()
    } else if selected.len() == ORDERED_OPT_PASSES.len() {
        "all".to_owned()
    } else {
        selected.join(",")
    }
}

/// Compute the 64-character SHA-256 sysroot cache key matching `scripts/sysroot-cache.sh` byte-for-byte.
pub fn compute_sysroot_key(input: &SysrootKeyInput<'_>) -> Result<String, String> {
    if !input.dylib_path.is_file() {
        return Err(format!(
            "backend dylib not found at `{}`",
            input.dylib_path.display()
        ));
    }
    let spec_path = match input.panic {
        PanicMode::Unwind => {
            let spec = input.target_spec.map(Path::to_path_buf).unwrap_or_else(|| {
                input
                    .sources_root
                    .join("targets")
                    .join("mires32-unwind.json")
            });
            if !spec.is_file() {
                return Err(format!(
                    "unwind panic mode requires target spec `{}`",
                    spec.display()
                ));
            }
            Some(spec)
        }
        PanicMode::Abort => None,
    };

    let backend_hash = sha256_file(input.dylib_path)?;
    let patches_hash = patches_digest(&input.sources_root.join("patches"))?;
    let mires_std_hash = mires_std_digest(input.sources_root)?;
    let rustc_identity = query_rustc_identity(input.channel)?;
    let target = input.panic.target_name();
    let rust_opt = input.rust_opt_level.unwrap_or("default");
    let args = normalize_sysroot_emit_args(input.emit_args_raw, input.panic);

    let mut material = String::new();
    let _ = writeln!(material, "cache-format=1");
    let _ = writeln!(material, "backend={backend_hash}");
    let _ = writeln!(material, "patches={patches_hash}");
    let _ = writeln!(material, "mires-std={mires_std_hash}");
    let _ = writeln!(material, "channel={}", input.channel);
    let _ = writeln!(material, "rustc={rustc_identity}");
    let _ = writeln!(material, "target={target}");
    let _ = writeln!(material, "rust-opt={rust_opt}");
    let _ = writeln!(material, "js-args={args}");

    if let Some(spec) = spec_path {
        let spec_hash = sha256_file(&spec)?;
        let unwind_patches_hash =
            patches_digest(&input.sources_root.join("patches").join("unwind"))?;
        let _ = writeln!(material, "panic=unwind");
        let _ = writeln!(material, "target-spec={spec_hash}");
        let _ = writeln!(material, "unwind-patches={unwind_patches_hash}");
    }

    Ok(sha256_bytes(material.as_bytes()))
}

/// Check whether a sysroot directory has all expected core libraries (`core`,
/// `compiler_builtins`, `alloc`, and `std` when present).
pub fn sysroot_has_core_libs(sysroot: &Path, target_name: &str) -> bool {
    let lib_dir = sysroot
        .join("lib")
        .join("rustlib")
        .join(target_name)
        .join("lib");
    let Ok(entries) = fs::read_dir(&lib_dir) else {
        return false;
    };
    let mut has_core = false;
    let mut has_builtins = false;
    let mut has_alloc = false;
    let mut has_std = false;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".rlib") {
            continue;
        }
        if name.starts_with("libcore-") {
            has_core = true;
        } else if name.starts_with("libcompiler_builtins-") {
            has_builtins = true;
        } else if name.starts_with("liballoc-") {
            has_alloc = true;
        } else if name.starts_with("libstd-") {
            has_std = true;
        }
    }

    has_core && has_builtins && has_alloc && has_std
}

/// Check whether a keyed sysroot directory is complete and matches its metadata stamps.
pub fn keyed_sysroot_is_ready(
    sysroot: &Path,
    target_name: &str,
    expected_key: &str,
    expected_args: &str,
    expected_opt: &str,
) -> bool {
    if !sysroot_has_core_libs(sysroot, target_name) {
        return false;
    }
    let Ok(recorded_key) = fs::read_to_string(sysroot.join(".cache-key")) else {
        return false;
    };
    if recorded_key.trim() != expected_key {
        return false;
    }
    let recorded_args = fs::read_to_string(sysroot.join(".js-args")).unwrap_or_default();
    if recorded_args.trim() != expected_args.trim() {
        return false;
    }
    let recorded_opt = fs::read_to_string(sysroot.join(".rust-opt-level")).unwrap_or_default();
    if recorded_opt.trim() != expected_opt.trim() {
        return false;
    }
    if let Ok(recorded_target) = fs::read_to_string(sysroot.join(".target")) {
        if !recorded_target.trim().is_empty() && recorded_target.trim() != target_name {
            return false;
        }
    }
    true
}

/// Build (or reuse) a keyed Mires sysroot in 100% pure Rust.
pub fn build_sysroot_native(req: &SysrootBuildRequest<'_>) -> Result<PathBuf, String> {
    if let Some(opt) = req.rust_opt_level {
        if !matches!(opt, "" | "0" | "1" | "2" | "3" | "s" | "z") {
            return Err(format!("invalid rustc opt-level `{opt}`"));
        }
    }

    toolchain::ensure_rustup_channel(req.channel, false, req.verbose)?;

    let target_name = req.panic.target_name();
    let normalized_args = normalize_sysroot_emit_args(req.emit_args_raw, req.panic);
    let rust_opt_str = req.rust_opt_level.unwrap_or("");
    let key = compute_sysroot_key(&SysrootKeyInput {
        sources_root: req.sources_root,
        dylib_path: req.dylib_path,
        channel: req.channel,
        panic: req.panic,
        target_spec: req.target_spec,
        emit_args_raw: req.emit_args_raw,
        rust_opt_level: req.rust_opt_level,
    })?;

    let sysroot_dir = req.sysroots_dir.join(&key);
    let cache_dir = req.cache_root.join("sysroot-cache").join(&key);
    let stdlib_dir = cache_dir.join("stdlib");

    if !req.clean
        && keyed_sysroot_is_ready(
            &sysroot_dir,
            target_name,
            &key,
            &normalized_args,
            rust_opt_str,
        )
    {
        if req.verbose {
            eprintln!(
                "cargo-mires: reusing cached sysroot {}",
                sysroot_dir.display()
            );
        }
        return Ok(sysroot_dir);
    }

    // Need `rust-src` only when we actually have to build the sysroot.
    toolchain::ensure_rustup_channel(req.channel, true, req.verbose)?;

    let lock_parent = req.cache_root.join("sysroot-cache").join(".locks");
    let _lock = KeyLock::acquire(&lock_parent, &key)?;

    if req.clean {
        remove_if_exists(&cache_dir)?;
        remove_if_exists(&sysroot_dir)?;
    }

    // Re-check after acquiring lock in case another process just finished building this key.
    if !req.clean
        && keyed_sysroot_is_ready(
            &sysroot_dir,
            target_name,
            &key,
            &normalized_args,
            rust_opt_str,
        )
    {
        return Ok(sysroot_dir);
    }

    eprintln!(
        "cargo-mires: building sysroot (target={target_name}, key={}...)",
        &key[..12]
    );
    let started = Instant::now();

    let rust_src_library = locate_rust_src_library(req.channel)?;
    let dst_library = stdlib_dir.join("library");
    sync_library_sources(&rust_src_library, &dst_library)?;

    apply_all_patches(req.sources_root, &stdlib_dir, req.panic, req.verbose)?;

    let target_arg = resolve_target_arg(req.sources_root, req.target_spec, req.panic)?;
    compile_stdlib_crates(
        req,
        &stdlib_dir,
        &target_arg,
        target_name,
        &normalized_args,
        &key,
    )?;

    let install_dir = stage_and_install_sysroot(
        req,
        &stdlib_dir,
        &sysroot_dir,
        &target_arg,
        target_name,
        &normalized_args,
        &key,
    )?;

    if req.verbose {
        eprintln!(
            "cargo-mires: sysroot built in {:.1}s at {}",
            started.elapsed().as_secs_f64(),
            install_dir.display()
        );
    }
    Ok(install_dir)
}

fn compile_stdlib_crates(
    req: &SysrootBuildRequest<'_>,
    stdlib_dir: &Path,
    target_arg: &str,
    target_name: &str,
    normalized_args: &str,
    key: &str,
) -> Result<(), String> {
    let core_manifest = stdlib_dir.join("library/core/Cargo.toml");
    let builtins_manifest =
        stdlib_dir.join("library/compiler-builtins/compiler-builtins/Cargo.toml");
    let alloc_manifest = stdlib_dir.join("library/alloc/Cargo.toml");

    // Fetch dependencies while network is allowed (with warm cache this is a no-op).
    for manifest in [&core_manifest, &builtins_manifest, &alloc_manifest] {
        let mut cmd = Command::new("cargo");
        cmd.arg(format!("+{}", req.channel))
            .arg("fetch")
            .arg("--manifest-path")
            .arg(manifest)
            .stdout(stdio_for_verbose(req.verbose))
            .stderr(Stdio::inherit());
        scrub_cargo_env(&mut cmd);
        let status = cmd
            .status()
            .map_err(|err| format!("failed to run `cargo fetch`: {err}"))?;
        if !status.success() {
            return Err(format!(
                "`cargo fetch --manifest-path {}` failed with {status}",
                manifest.display()
            ));
        }
    }

    let stamp_path = stdlib_dir.join(".backend-stamp");
    let existing_stamp = fs::read_to_string(&stamp_path).unwrap_or_default();
    if existing_stamp.trim() != key {
        remove_if_exists(&stdlib_dir.join("library/target"))?;
    }

    let rustflags_vec = build_sysroot_rustflags(req, normalized_args);
    let target_env_var = format!(
        "CARGO_TARGET_{}_RUSTFLAGS",
        target_name
            .chars()
            .map(|c| match c {
                '-' | '.' => '_',
                other => other.to_ascii_uppercase(),
            })
            .collect::<String>()
    );
    let has_whitespace_flag = rustflags_vec
        .iter()
        .any(|f| f.chars().any(char::is_whitespace));

    let builds: [(&str, &Path, &[&str]); 3] = [
        ("core", &core_manifest, &[]),
        (
            "compiler_builtins",
            &builtins_manifest,
            &["--features", "compiler-builtins,mem"],
        ),
        ("alloc", &alloc_manifest, &[]),
    ];

    for (label, manifest, extra_args) in builds {
        if req.verbose {
            eprintln!("cargo-mires: building sysroot crate `{label}`...");
        }
        let mut cmd = Command::new("cargo");
        cmd.arg(format!("+{}", req.channel));
        if req.panic == PanicMode::Unwind {
            cmd.arg("-Zjson-target-spec");
        }
        cmd.arg("build")
            .arg("--release")
            .arg("--target")
            .arg(target_arg)
            .arg("--manifest-path")
            .arg(manifest)
            .args(extra_args);

        scrub_cargo_env(&mut cmd);
        cmd.env("CARGO_NET_OFFLINE", "true")
            .stdout(stdio_for_verbose(req.verbose))
            .stderr(Stdio::inherit());

        if has_whitespace_flag {
            // Use TOML-escaped target rustflags config when dylib path contains whitespace.
            let encoded = serde_json::to_string(&rustflags_vec)
                .map_err(|err| format!("failed to encode target rustflags: {err}"))?;
            cmd.arg("--config")
                .arg(format!("target.{target_name}.rustflags={encoded}"));
        } else {
            cmd.env(&target_env_var, rustflags_vec.join(" "));
        }

        let status = cmd
            .status()
            .map_err(|err| format!("failed to invoke `cargo build` for `{label}`: {err}"))?;
        if !status.success() {
            return Err(format!("sysroot build of `{label}` failed with {status}"));
        }
    }

    fs::write(&stamp_path, key.as_bytes())
        .map_err(|err| format!("failed to write `{}`: {err}", stamp_path.display()))?;
    Ok(())
}

fn build_sysroot_rustflags(req: &SysrootBuildRequest<'_>, normalized_args: &str) -> Vec<String> {
    let mut flags = Vec::new();
    if req.panic == PanicMode::Unwind {
        flags.push("-Zunstable-options".to_owned());
    }
    flags.push(format!("-Zcodegen-backend={}", req.dylib_path.display()));
    flags.push("-Zinline-mir".to_owned());
    flags.push(format!("-Cpanic={}", req.panic.as_str()));
    flags.push("-Zforce-unstable-if-unmarked".to_owned());
    flags.push("-Ccodegen-units=1".to_owned());
    flags.push("-Coverflow-checks=off".to_owned());
    flags.push("-Cdebug-assertions=off".to_owned());
    flags.push("-Cdebuginfo=0".to_owned());
    flags.push(format!("-Cllvm-args=js-producer={}", req.dylib_sha256));
    for arg in normalized_args.split_whitespace() {
        flags.push(format!("-Cllvm-args={arg}"));
    }
    if let Some(opt) = req.rust_opt_level.filter(|s| !s.is_empty()) {
        flags.push(format!("-Copt-level={opt}"));
    }
    flags
}

fn stage_and_install_sysroot(
    req: &SysrootBuildRequest<'_>,
    stdlib_dir: &Path,
    sysroot_dir: &Path,
    target_arg: &str,
    target_name: &str,
    normalized_args: &str,
    key: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(req.sysroots_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", req.sysroots_dir.display()))?;

    let install_dir = req
        .sysroots_dir
        .join(format!(".install-{key}-{}", std::process::id()));
    remove_if_exists(&install_dir)?;
    let _staging_guard = TempDirCleanup(install_dir.clone());

    let install_lib = install_dir
        .join("lib")
        .join("rustlib")
        .join(target_name)
        .join("lib");
    fs::create_dir_all(&install_lib)
        .map_err(|err| format!("failed to create `{}`: {err}", install_lib.display()))?;

    let deps_dir = stdlib_dir
        .join("library/target")
        .join(target_name)
        .join("release/deps");

    for prefix in ["libcore-", "libcompiler_builtins-", "liballoc-"] {
        let newest = find_newest_rlib(&deps_dir, prefix).ok_or_else(|| {
            format!(
                "expected `{prefix}*.rlib` in `{}` after sysroot build",
                deps_dir.display()
            )
        })?;
        let file_name = newest
            .file_name()
            .ok_or_else(|| format!("missing filename on `{}`", newest.display()))?;
        fs::copy(&newest, install_lib.join(file_name))
            .map_err(|err| format!("failed to copy `{}` into sysroot: {err}", newest.display()))?;
    }

    let mires_std_lib_rs = req.sources_root.join("mires-std/src/lib.rs");
    if mires_std_lib_rs.is_file() {
        compile_mires_std(
            req,
            &mires_std_lib_rs,
            &install_dir,
            &install_lib,
            target_arg,
            normalized_args,
            key,
        )?;
    }

    let opt_str = req.rust_opt_level.unwrap_or("");
    fs::write(install_dir.join(".js-args"), normalized_args.as_bytes())
        .map_err(|err| format!("failed to write `.js-args`: {err}"))?;
    fs::write(install_dir.join(".rust-opt-level"), opt_str.as_bytes())
        .map_err(|err| format!("failed to write `.rust-opt-level`: {err}"))?;
    fs::write(install_dir.join(".cache-key"), key.as_bytes())
        .map_err(|err| format!("failed to write `.cache-key`: {err}"))?;
    fs::write(install_dir.join(".target"), target_name.as_bytes())
        .map_err(|err| format!("failed to write `.target`: {err}"))?;

    remove_if_exists(sysroot_dir)?;
    fs::rename(&install_dir, sysroot_dir).map_err(|err| {
        format!(
            "failed to install sysroot `{}` -> `{}`: {err}",
            install_dir.display(),
            sysroot_dir.display()
        )
    })?;

    Ok(sysroot_dir.to_path_buf())
}

fn compile_mires_std(
    req: &SysrootBuildRequest<'_>,
    lib_rs: &Path,
    install_dir: &Path,
    install_lib: &Path,
    target_arg: &str,
    normalized_args: &str,
    key: &str,
) -> Result<(), String> {
    if req.verbose {
        eprintln!("cargo-mires: building sysroot crate `std` (mires-std)...");
    }
    let std_hash = &key[..16.min(key.len())];
    let opt_level = req.rust_opt_level.filter(|s| !s.is_empty()).unwrap_or("3");
    let mut remap = req.sources_root.to_string_lossy().into_owned();
    if !remap.ends_with(std::path::MAIN_SEPARATOR) {
        remap.push(std::path::MAIN_SEPARATOR);
    }

    let mut cmd = Command::new("rustc");
    cmd.arg(format!("+{}", req.channel))
        .arg("--crate-name")
        .arg("std")
        .arg("--crate-type")
        .arg("rlib")
        .arg("--edition")
        .arg("2024")
        .arg("--target")
        .arg(target_arg);
    if req.panic == PanicMode::Unwind {
        cmd.arg("-Zunstable-options");
    }
    cmd.arg("--sysroot")
        .arg(install_dir)
        .arg(format!("-Zcodegen-backend={}", req.dylib_path.display()))
        .arg("-Zinline-mir")
        .arg(format!("-Cpanic={}", req.panic.as_str()))
        .arg("-Ccodegen-units=1")
        .arg("-Coverflow-checks=off")
        .arg("-Cdebug-assertions=off")
        .arg("-Cdebuginfo=0")
        .arg(format!("-Copt-level={opt_level}"))
        .arg(format!("-Cmetadata=mires_std_{std_hash}"))
        .arg(format!("-Cextra-filename=-{std_hash}"))
        .arg("--remap-path-prefix")
        .arg(format!("{remap}="))
        .arg(format!("-Cllvm-args=js-producer={}", req.dylib_sha256));

    for arg in normalized_args.split_whitespace() {
        cmd.arg(format!("-Cllvm-args={arg}"));
    }
    cmd.arg("--out-dir")
        .arg(install_lib)
        .arg(lib_rs)
        .stdout(stdio_for_verbose(req.verbose))
        .stderr(Stdio::inherit());
    scrub_cargo_env(&mut cmd);

    let status = cmd
        .status()
        .map_err(|err| format!("failed to run `rustc` for `mires-std`: {err}"))?;
    if !status.success() {
        return Err(format!("building `mires-std` failed with {status}"));
    }

    let expected_rlib = install_lib.join(format!("libstd-{std_hash}.rlib"));
    if !expected_rlib.is_file() {
        return Err(format!(
            "expected `{}` after building `mires-std`",
            expected_rlib.display()
        ));
    }
    Ok(())
}

fn find_newest_rlib(deps_dir: &Path, prefix: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(deps_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(prefix) || !name.ends_with(".rlib") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(best_t, _)| modified >= *best_t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

fn resolve_target_arg(
    sources_root: &Path,
    target_spec: Option<&Path>,
    panic: PanicMode,
) -> Result<String, String> {
    match panic {
        PanicMode::Abort => Ok(panic.target_name().to_owned()),
        PanicMode::Unwind => {
            let spec = target_spec
                .map(Path::to_path_buf)
                .unwrap_or_else(|| sources_root.join("targets/mires32-unwind.json"));
            if !spec.is_file() {
                return Err(format!(
                    "unwind target spec not found at `{}`",
                    spec.display()
                ));
            }
            let canonical = fs::canonicalize(&spec).unwrap_or(spec);
            Ok(canonical.to_string_lossy().into_owned())
        }
    }
}

fn locate_rust_src_library(channel: &str) -> Result<PathBuf, String> {
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("+{channel}"))
        .args(["--print", "sysroot"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    scrub_cargo_env(&mut cmd);

    let output = cmd
        .output()
        .map_err(|err| format!("failed to run `rustc +{channel} --print sysroot`: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "`rustc +{channel} --print sysroot` failed with {}",
            output.status
        ));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let library = sysroot.join("lib/rustlib/src/rust/library");
    if !library.join("core").is_dir() {
        return Err(format!(
            "missing `rust-src` component for `{channel}` at `{}`; \
             run `rustup component add rust-src --toolchain {channel}`",
            library.display()
        ));
    }
    Ok(library)
}

/// Pure-Rust recursive directory sync mirroring `rsync -a --delete --exclude target/`.
fn sync_library_sources(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|err| format!("failed to create `{}`: {err}", dst.display()))?;
    sync_dir_recursive(src, dst, true)
}

fn sync_dir_recursive(src: &Path, dst: &Path, is_top_level: bool) -> Result<(), String> {
    let mut src_names: BTreeSet<std::ffi::OsString> = BTreeSet::new();
    let src_entries =
        fs::read_dir(src).map_err(|err| format!("failed to read `{}`: {err}", src.display()))?;

    for entry in src_entries {
        let entry =
            entry.map_err(|err| format!("failed to read entry in `{}`: {err}", src.display()))?;
        let name = entry.file_name();
        if is_top_level && name == "target" {
            continue;
        }
        src_names.insert(name.clone());
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to stat `{}`: {err}", src_path.display()))?;

        if file_type.is_dir() {
            if dst_path.is_file() || dst_path.is_symlink() {
                fs::remove_file(&dst_path)
                    .map_err(|err| format!("failed to remove `{}`: {err}", dst_path.display()))?;
            }
            fs::create_dir_all(&dst_path)
                .map_err(|err| format!("failed to create `{}`: {err}", dst_path.display()))?;
            sync_dir_recursive(&src_path, &dst_path, false)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            if dst_path.is_dir() {
                fs::remove_dir_all(&dst_path)
                    .map_err(|err| format!("failed to remove `{}`: {err}", dst_path.display()))?;
            }
            let src_bytes = fs::read(&src_path)
                .map_err(|err| format!("failed to read `{}`: {err}", src_path.display()))?;
            let unchanged = dst_path.is_file()
                && fs::read(&dst_path).is_ok_and(|existing| existing == src_bytes);
            if !unchanged {
                fs::write(&dst_path, &src_bytes)
                    .map_err(|err| format!("failed to write `{}`: {err}", dst_path.display()))?;
            }
        }
    }

    // Remove stale files/dirs in destination that no longer exist in source.
    if let Ok(dst_entries) = fs::read_dir(dst) {
        for entry in dst_entries.flatten() {
            let name = entry.file_name();
            if is_top_level && name == "target" {
                continue;
            }
            if !src_names.contains(&name) {
                let path = entry.path();
                remove_if_exists(&path)?;
            }
        }
    }
    Ok(())
}

fn apply_all_patches(
    sources_root: &Path,
    stdlib_dir: &Path,
    panic: PanicMode,
    verbose: bool,
) -> Result<(), String> {
    let mut patch_files = collect_sorted_patches(&sources_root.join("patches"))?;
    if panic == PanicMode::Unwind {
        patch_files.extend(collect_sorted_patches(
            &sources_root.join("patches").join("unwind"),
        )?);
    }

    for patch_path in patch_files {
        if verbose {
            if let Some(name) = patch_path.file_name().and_then(|n| n.to_str()) {
                eprintln!("cargo-mires: applying patch {name}");
            }
        }
        let text = fs::read_to_string(&patch_path)
            .map_err(|err| format!("failed to read patch `{}`: {err}", patch_path.display()))?;
        patch::apply_patch_p1(stdlib_dir, &text)
            .map_err(|err| format!("failed to apply patch `{}`: {err}", patch_path.display()))?;
    }
    Ok(())
}

fn collect_sorted_patches(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut patches: Vec<(String, PathBuf)> = Vec::new();
    let entries = fs::read_dir(dir).map_err(|err| {
        format!(
            "failed to read patches directory `{}`: {err}",
            dir.display()
        )
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.ends_with(".patch") {
            continue;
        }
        let path = entry.path();
        if path.is_file() {
            patches.push((name, path));
        }
    }
    patches.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(patches.into_iter().map(|(_, p)| p).collect())
}

fn patches_digest(dir: &Path) -> Result<String, String> {
    let patches = collect_sorted_patches(dir)?;
    if patches.is_empty() {
        return Ok(sha256_bytes(b"none\n"));
    }
    let mut material = String::new();
    for path in patches {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("non-UTF-8 patch filename `{}`", path.display()))?;
        let digest = sha256_file(&path)?;
        let _ = writeln!(material, "{name} {digest}");
    }
    Ok(sha256_bytes(material.as_bytes()))
}

fn mires_std_digest(sources_root: &Path) -> Result<String, String> {
    let dir = sources_root.join("mires-std");
    let mut material = String::new();
    let mut found = false;

    let manifest = dir.join("Cargo.toml");
    if manifest.is_file() {
        found = true;
        let digest = sha256_file(&manifest)?;
        let _ = writeln!(material, "Cargo.toml {digest}");
    }

    let src_dir = dir.join("src");
    if src_dir.is_dir() {
        let mut sources = Vec::new();
        collect_rs_files("src", &src_dir, &mut sources)?;
        sources.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (rel, path) in sources {
            found = true;
            let digest = sha256_file(&path)?;
            let _ = writeln!(material, "{rel} {digest}");
        }
    }

    if !found {
        material.push_str("none\n");
    }
    Ok(sha256_bytes(material.as_bytes()))
}

fn collect_rs_files(
    prefix: &str,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read `{}`: {err}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}/{name}");
        if file_type.is_dir() {
            collect_rs_files(&rel, &path, out)?;
        } else if file_type.is_file() && name.ends_with(".rs") {
            out.push((rel, path));
        }
    }
    Ok(())
}

fn query_rustc_identity(channel: &str) -> Result<String, String> {
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("+{channel}"))
        .arg("-vV")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    scrub_cargo_env(&mut cmd);

    let output = cmd
        .output()
        .map_err(|err| format!("failed to execute `rustc +{channel} -vV`: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot identify pinned Rust toolchain `{channel}` (`rustc +{channel} -vV` failed)"
        ));
    }
    let raw = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    Ok(raw.trim_end_matches('\n').to_owned())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn stdio_for_verbose(verbose: bool) -> Stdio {
    if verbose {
        Stdio::inherit()
    } else {
        Stdio::null()
    }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_dir() {
                fs::remove_dir_all(path)
                    .map_err(|err| format!("failed to remove `{}`: {err}", path.display()))
            } else {
                fs::remove_file(path)
                    .map_err(|err| format!("failed to remove `{}`: {err}", path.display()))
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to inspect `{}`: {err}", path.display())),
    }
}

struct KeyLock {
    dir: PathBuf,
}

impl KeyLock {
    fn acquire(lock_parent: &Path, key: &str) -> Result<Self, String> {
        fs::create_dir_all(lock_parent)
            .map_err(|err| format!("failed to create `{}`: {err}", lock_parent.display()))?;
        let lock_dir = lock_parent.join(key);
        if fs::create_dir(&lock_dir).is_err() {
            let owner = fs::read_to_string(lock_dir.join("pid")).unwrap_or_default();
            let owner_trimmed = owner.trim();
            if !owner_trimmed.is_empty() && is_pid_stale(owner_trimmed) {
                let _ = fs::remove_dir_all(&lock_dir);
                fs::create_dir(&lock_dir).map_err(|err| {
                    format!(
                        "sysroot `{key}` is already being built (failed to reclaim lock): {err}"
                    )
                })?;
            } else {
                let suffix = if owner_trimmed.is_empty() {
                    String::new()
                } else {
                    format!(" by pid {owner_trimmed}")
                };
                return Err(format!("sysroot `{key}` is already being built{suffix}"));
            }
        }
        let _ = fs::write(lock_dir.join("pid"), std::process::id().to_string());
        Ok(Self { dir: lock_dir })
    }
}

impl Drop for KeyLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
fn is_pid_stale(pid: &str) -> bool {
    if !pid.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    Command::new("kill")
        .args(["-0", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| !status.success())
}

#[cfg(not(unix))]
fn is_pid_stale(_pid: &str) -> bool {
    false
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_profile_and_emit_args_matching_shell_script() {
        assert_eq!(
            normalize_sysroot_emit_args("js-profile=dev", PanicMode::Unwind),
            "js-cleanup=off js-opt=off"
        );
        assert_eq!(
            normalize_sysroot_emit_args("js-profile=dev", PanicMode::Abort),
            "js-opt=off"
        );
        assert_eq!(
            normalize_sysroot_emit_args("js-profile=release-balanced", PanicMode::Unwind),
            "js-cleanup=off js-ecma-target=2018 js-minify=on js-opt=on"
        );
        assert_eq!(
            normalize_sysroot_emit_args("js-profile=release-balanced", PanicMode::Abort),
            "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on"
        );
        assert_eq!(
            normalize_sysroot_emit_args(
                "js-opt=on js-opt-passes=link-dedup,sroa js-modules=esm",
                PanicMode::Abort
            ),
            "js-opt-passes=sroa,link-dedup js-opt=on"
        );
        assert_eq!(
            normalize_sysroot_emit_args(
                "js-opt-passes=link-dedup,native-representations,future-poller,ast-dce,sroa",
                PanicMode::Abort
            ),
            ""
        );
    }

    #[test]
    #[cfg(unix)]
    fn compute_sysroot_key_matches_shell_script_byte_for_byte() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let script = repo_root.join("scripts/sysroot-cache.sh");
        let dylib = repo_root
            .join("target/release")
            .join(toolchain::current_dylib_filename());
        if !script.is_file() || !dylib.is_file() {
            return;
        }
        let channel = crate::sysroot::read_pinned_channel(repo_root);
        let spec = repo_root.join("targets/mires32-unwind.json");

        let cases = [
            ("js-profile=dev", PanicMode::Unwind),
            ("js-profile=dev", PanicMode::Abort),
            ("js-profile=release-balanced", PanicMode::Unwind),
            ("js-profile=release-balanced", PanicMode::Abort),
        ];

        for (raw_args, panic) in cases {
            let rust_key = compute_sysroot_key(&SysrootKeyInput {
                sources_root: repo_root,
                dylib_path: &dylib,
                channel: &channel,
                panic,
                target_spec: (panic == PanicMode::Unwind).then_some(spec.as_path()),
                emit_args_raw: raw_args,
                rust_opt_level: None,
            })
            .unwrap();

            let mut cmd = Command::new("bash");
            cmd.arg(&script)
                .arg("path")
                .current_dir(repo_root)
                .env("JS_EXTRA_ARGS", raw_args)
                .env("JSC_PANIC", panic.as_str())
                .env_remove("JSC_SYSROOT_ARGS")
                .env_remove("JSC_RUST_OPT_LEVEL")
                .env_remove("JSC_TARGET")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            scrub_cargo_env(&mut cmd);

            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "sysroot-cache.sh failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let shell_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let shell_key = shell_path.file_name().unwrap().to_str().unwrap();

            assert_eq!(
                rust_key, shell_key,
                "key mismatch for args=`{raw_args}` panic={panic}"
            );
        }
    }
}
