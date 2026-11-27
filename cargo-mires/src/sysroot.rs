//! Mires toolchain discovery, backend building, and cached sysroot management.

use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::cli::{PanicMode, Profile, SysrootCliOptions};
use crate::metadata;

const DEFAULT_CHANNEL: &str = "nightly";

/// Resolved toolchain binaries, pinned nightly channel, target, and sysroot directory.
#[derive(Clone, Debug)]
pub struct ToolchainInfo {
    pub channel: String,
    pub dylib_path: PathBuf,
    pub dylib_sha256: String,
    pub jsc_link_path: PathBuf,
    pub sysroot_dir: PathBuf,
    pub target_arg: String,
    pub target_name: String,
    pub is_json_target: bool,
}

/// Request parameters for ensuring a matching sysroot exists.
#[derive(Clone, Debug)]
pub struct SysrootRequest<'a> {
    pub mires_root_override: Option<&'a Path>,
    pub profile: Profile,
    pub panic: PanicMode,
    pub clean: bool,
    pub verbose: bool,
}

/// Execute `cargo mires sysroot [--clean]`.
pub fn run_sysroot_cmd(opts: &SysrootCliOptions) -> Result<(), String> {
    let meta = if opts.profile.is_none() || opts.panic.is_none() {
        metadata::try_load_sysroot_metadata(opts.manifest_path.as_deref(), opts.package.as_deref())
    } else {
        None
    };

    let meta_profile = meta
        .as_ref()
        .and_then(|m| m.profile.as_deref())
        .and_then(Profile::parse);
    let meta_panic = meta
        .as_ref()
        .and_then(|m| m.panic.as_deref())
        .and_then(PanicMode::parse);

    let profile = if let Some(explicit) = opts.profile {
        explicit
    } else if opts.release {
        match meta_profile {
            Some(p) if p.is_release() => p,
            _ => Profile::ReleaseBalanced,
        }
    } else {
        meta_profile.unwrap_or(Profile::Dev)
    };
    let panic = opts.panic.or(meta_panic).unwrap_or(PanicMode::Unwind);

    let info = ensure_sysroot(&SysrootRequest {
        mires_root_override: opts.mires_root.as_deref(),
        profile,
        panic,
        clean: opts.clean,
        verbose: opts.verbose,
    })?;

    println!("{}", info.sysroot_dir.display());
    Ok(())
}

/// Ensure the backend dylib, `jsc-link`, and matching sysroot exist and return their paths.
pub fn ensure_sysroot(req: &SysrootRequest<'_>) -> Result<ToolchainInfo, String> {
    let mires_root = locate_mires_root(req.mires_root_override)?;
    let (dylib_path, jsc_link_path) = ensure_backend_built(&mires_root, req.verbose)?;
    let channel = read_pinned_channel(&mires_root);
    let (target_arg, target_name, is_json_target) = resolve_target(&mires_root, req.panic)?;

    let sysroot_dir = resolve_sysroot_path(&mires_root, req.profile, req.panic)?;
    let needs_build = req.clean || !sysroot_is_ready(&sysroot_dir, &target_name);
    if needs_build {
        build_sysroot(&mires_root, req.profile, req.panic, req.clean, req.verbose)?;
    } else if req.verbose {
        eprintln!(
            "cargo-mires: reusing cached sysroot {}",
            sysroot_dir.display()
        );
    }

    if !sysroot_is_ready(&sysroot_dir, &target_name) {
        return Err(format!(
            "sysroot build completed but expected libraries are missing under `{}`",
            sysroot_dir.display()
        ));
    }

    let dylib_sha256 = sha256_file(&dylib_path)?;

    Ok(ToolchainInfo {
        channel,
        dylib_path,
        dylib_sha256,
        jsc_link_path,
        sysroot_dir,
        target_arg,
        target_name,
        is_json_target,
    })
}

/// Discover the Mires repository root via CLI flag, `MIRES_ROOT`, current executable,
/// or compile-time `CARGO_MANIFEST_DIR`.
pub fn locate_mires_root(cli_override: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = cli_override {
        return validate_mires_root(path, "--mires-root");
    }

    if let Some(env_path) = env::var_os("MIRES_ROOT") {
        let path = PathBuf::from(env_path);
        return validate_mires_root(&path, "MIRES_ROOT");
    }

    if let Ok(exe) = env::current_exe() {
        let mut cursor = exe.parent();
        for _ in 0..4 {
            let Some(dir) = cursor else {
                break;
            };
            if is_mires_root(dir) {
                return Ok(dir.to_path_buf());
            }
            cursor = dir.parent();
        }
    }

    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let candidate = Path::new(manifest_dir)
            .parent()
            .unwrap_or_else(|| Path::new(manifest_dir));
        if is_mires_root(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(
        "could not locate Mires toolchain root; pass `--mires-root <path>` or set `MIRES_ROOT`"
            .to_owned(),
    )
}

fn validate_mires_root(path: &Path, source: &str) -> Result<PathBuf, String> {
    if is_mires_root(path) {
        Ok(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    } else {
        Err(format!(
            "`{}` (from {source}) is not a valid Mires root \
             (expected `rust-toolchain.toml` and `scripts/build_sysroot.sh`)",
            path.display()
        ))
    }
}

fn is_mires_root(dir: &Path) -> bool {
    dir.join("rust-toolchain.toml").is_file()
        && dir.join("scripts").join("build_sysroot.sh").is_file()
}

fn dylib_filename() -> Result<&'static str, String> {
    if cfg!(target_os = "windows") {
        return Err(format!(
            "Mires codegen backend is not supported on `{}`",
            env::consts::OS
        ));
    }
    if cfg!(target_os = "macos") {
        Ok("librustc_codegen_mires.dylib")
    } else {
        Ok("librustc_codegen_mires.so")
    }
}

fn ensure_backend_built(mires_root: &Path, verbose: bool) -> Result<(PathBuf, PathBuf), String> {
    let release_dir = mires_root.join("target").join("release");
    let dylib_path = release_dir.join(dylib_filename()?);
    let jsc_link_path = env::var_os("JSC_LINK_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| release_dir.join("jsc-link"));

    if dylib_path.is_file() && jsc_link_path.is_file() {
        return Ok((dylib_path, jsc_link_path));
    }

    let build_script = mires_root.join("scripts").join("build.sh");
    if !build_script.is_file() {
        return Err(format!(
            "backend artifacts missing and `{}` does not exist",
            build_script.display()
        ));
    }

    eprintln!("cargo-mires: building compiler backend (`scripts/build.sh`)...");
    let mut cmd = Command::new("bash");
    cmd.arg(&build_script)
        .current_dir(mires_root)
        .stdout(if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::inherit());
    scrub_cargo_env(&mut cmd);

    let status = cmd
        .status()
        .map_err(|err| format!("failed to run `{}`: {err}", build_script.display()))?;
    if !status.success() {
        return Err(format!("`{}` failed with {status}", build_script.display()));
    }

    let final_linker = if jsc_link_path.is_file() {
        jsc_link_path
    } else {
        release_dir.join("jsc-link")
    };
    if !dylib_path.is_file() {
        return Err(format!(
            "backend build succeeded but `{}` is missing",
            dylib_path.display()
        ));
    }
    if !final_linker.is_file() {
        return Err(format!(
            "backend build succeeded but `{}` is missing",
            final_linker.display()
        ));
    }
    Ok((dylib_path, final_linker))
}

fn resolve_target(mires_root: &Path, panic: PanicMode) -> Result<(String, String, bool), String> {
    match panic {
        PanicMode::Unwind => {
            let spec = mires_root.join("targets").join("mires32-unwind.json");
            if !spec.is_file() {
                return Err(format!(
                    "unwind target spec not found at `{}`",
                    spec.display()
                ));
            }
            let spec = fs::canonicalize(&spec).unwrap_or(spec);
            Ok((
                spec.to_string_lossy().into_owned(),
                panic.target_name().to_owned(),
                true,
            ))
        }
        PanicMode::Abort => Ok((
            panic.target_name().to_owned(),
            panic.target_name().to_owned(),
            false,
        )),
    }
}

fn resolve_sysroot_path(
    mires_root: &Path,
    profile: Profile,
    panic: PanicMode,
) -> Result<PathBuf, String> {
    let script = mires_root.join("scripts").join("sysroot-cache.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script)
        .arg("path")
        .current_dir(mires_root)
        .env("JS_EXTRA_ARGS", format!("js-profile={}", profile.as_str()))
        .env("JSC_PANIC", panic.as_str())
        .env_remove("JSC_SYSROOT_ARGS")
        .env_remove("JSC_RUST_OPT_LEVEL")
        .env_remove("JSC_TARGET")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    scrub_cargo_env(&mut cmd);

    let output = cmd
        .output()
        .map_err(|err| format!("failed to run `{}`: {err}", script.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "`sysroot-cache.sh path` failed: {}",
            if stderr.is_empty() {
                output.status.to_string()
            } else {
                stderr
            }
        ));
    }

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path_str.is_empty() {
        return Err("`sysroot-cache.sh path` returned an empty path".to_owned());
    }
    Ok(PathBuf::from(path_str))
}

fn sysroot_is_ready(sysroot: &Path, target_name: &str) -> bool {
    let lib_dir = sysroot
        .join("lib")
        .join("rustlib")
        .join(target_name)
        .join("lib");
    if !lib_dir.is_dir() {
        return false;
    }
    let Some(dir_name) = sysroot.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Ok(recorded_key) = fs::read_to_string(sysroot.join(".cache-key")) else {
        return false;
    };
    if recorded_key.trim() != dir_name {
        return false;
    }
    if let Ok(recorded_target) = fs::read_to_string(sysroot.join(".target")) {
        if !recorded_target.trim().is_empty() && recorded_target.trim() != target_name {
            return false;
        }
    }
    true
}

fn build_sysroot(
    mires_root: &Path,
    profile: Profile,
    panic: PanicMode,
    clean: bool,
    verbose: bool,
) -> Result<(), String> {
    let script = mires_root.join("scripts").join("build_sysroot.sh");
    eprintln!(
        "cargo-mires: preparing sysroot (profile={profile}, panic={panic}{})...",
        if clean { ", clean" } else { "" }
    );

    let mut cmd = Command::new("bash");
    cmd.arg(&script);
    if clean {
        cmd.arg("--clean");
    }
    cmd.current_dir(mires_root)
        .env("SKIP_BUILD", "1")
        .env("JS_EXTRA_ARGS", format!("js-profile={}", profile.as_str()))
        .env("JSC_PANIC", panic.as_str())
        .env_remove("JSC_SYSROOT_ARGS")
        .env_remove("JSC_RUST_OPT_LEVEL")
        .env_remove("JSC_TARGET")
        .stdout(if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::inherit());
    scrub_cargo_env(&mut cmd);

    let status = cmd
        .status()
        .map_err(|err| format!("failed to run `{}`: {err}", script.display()))?;
    if !status.success() {
        return Err(format!("`{}` failed with {status}", script.display()));
    }
    Ok(())
}

/// Read the pinned nightly channel from `<mires_root>/rust-toolchain.toml`.
pub fn read_pinned_channel(mires_root: &Path) -> String {
    let content = fs::read_to_string(mires_root.join("rust-toolchain.toml")).unwrap_or_default();
    parse_pinned_channel(&content)
}

/// Parse the `channel = "..."` field out of a `rust-toolchain.toml` body.
pub fn parse_pinned_channel(toml_text: &str) -> String {
    for line in toml_text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "channel" {
            let trimmed = value.trim().trim_matches('"').trim_matches('\'');
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    DEFAULT_CHANNEL.to_owned()
}

/// Compute lowercase hex SHA-256 of a file on disk.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("failed to open `{}` for hashing: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .map_err(|err| format!("failed to read `{}` for hashing: {err}", path.display()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Remove Cargo/rustc/wrapper environment variables that would interfere with nested script runs.
pub fn scrub_cargo_env(cmd: &mut Command) {
    for (key, _) in env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("CARGO")
            || name.starts_with("MIRES_WRAPPER_")
            || name == "MIRES_RUSTC_WRAPPER"
            || name == "RUSTC"
            || name == "RUSTC_WRAPPER"
            || name == "RUSTC_WORKSPACE_WRAPPER"
            || name == "RUSTUP_TOOLCHAIN"
            || name == "RUSTFLAGS"
        {
            cmd.env_remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pinned_channel_ignoring_comments() {
        let text = "\
# channel = \"stable\"
[toolchain]
channel = \"nightly-2026-07-27\" # pinned
profile = \"minimal\"
";
        assert_eq!(parse_pinned_channel(text), "nightly-2026-07-27");
        assert_eq!(parse_pinned_channel(""), "nightly");
    }

    #[test]
    fn locates_checkout_mires_root_via_compile_time_fallback() {
        // When run inside the workspace tests, CARGO_MANIFEST_DIR/.. is the repo root.
        let root = locate_mires_root(None).unwrap();
        assert!(root.join("rust-toolchain.toml").is_file());
        assert!(root.join("scripts/build_sysroot.sh").is_file());
    }
}
