//! Mires toolchain resolution and cached sysroot management (100% pure Rust, zero `bash`).

use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::cli::{PanicMode, Profile, SysrootCliOptions};
use crate::metadata;
use crate::sysroot_build::{self, SysrootBuildRequest};
use crate::toolchain::{
    self, ensure_checkout_backend_built, ensure_rustup_channel, resolve_toolchain_source,
    ToolchainManifest, ToolchainSource, DEFAULT_PINNED_CHANNEL,
};

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

/// Ensure the backend dylib, `jsc-link`, pinned nightly toolchain, and matching sysroot exist.
pub fn ensure_sysroot(req: &SysrootRequest<'_>) -> Result<ToolchainInfo, String> {
    match resolve_toolchain_source(req.mires_root_override)? {
        ToolchainSource::SourceCheckout(checkout_root) => {
            ensure_from_source_checkout(&checkout_root, req)
        }
        ToolchainSource::InstalledBundle { root, manifest } => {
            ensure_from_installed_bundle(&root, &manifest, req)
        }
    }
}

fn ensure_from_source_checkout(
    checkout_root: &Path,
    req: &SysrootRequest<'_>,
) -> Result<ToolchainInfo, String> {
    let channel = read_pinned_channel(checkout_root);
    ensure_rustup_channel(&channel, false, req.verbose)?;

    let (dylib_path, jsc_link_path) =
        ensure_checkout_backend_built(checkout_root, &channel, req.verbose)?;
    let dylib_sha256 = sha256_file(&dylib_path)?;
    let spec_path = checkout_root.join("targets/mires32-unwind.json");
    let (target_arg, target_name, is_json_target) = resolve_target(&spec_path, req.panic)?;

    let emit_args = format!("js-profile={}", req.profile.as_str());
    let sysroot_dir = sysroot_build::build_sysroot_native(&SysrootBuildRequest {
        sources_root: checkout_root,
        cache_root: &checkout_root.join("build"),
        sysroots_dir: &checkout_root.join("build/sysroots"),
        dylib_path: &dylib_path,
        dylib_sha256: &dylib_sha256,
        channel: &channel,
        target_spec: is_json_target.then_some(spec_path.as_path()),
        panic: req.panic,
        emit_args_raw: &emit_args,
        rust_opt_level: None,
        clean: req.clean,
        verbose: req.verbose,
    })?;

    if !sysroot_build::sysroot_has_core_libs(&sysroot_dir, &target_name) {
        return Err(format!(
            "sysroot build completed but expected libraries are missing under `{}`",
            sysroot_dir.display()
        ));
    }

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

fn ensure_from_installed_bundle(
    bundle_root: &Path,
    manifest: &ToolchainManifest,
    req: &SysrootRequest<'_>,
) -> Result<ToolchainInfo, String> {
    ensure_rustup_channel(&manifest.channel, false, req.verbose)?;

    let dylib_path = bundle_root.join(&manifest.dylib);
    if !dylib_path.is_file() {
        return Err(format!(
            "installed toolchain at `{}` is missing backend `{}`",
            bundle_root.display(),
            manifest.dylib
        ));
    }
    let jsc_link_path = env::var_os("JSC_LINK_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| bundle_root.join(&manifest.jsc_link));
    if !jsc_link_path.is_file() {
        return Err(format!(
            "installed toolchain at `{}` is missing linker `{}`",
            bundle_root.display(),
            manifest.jsc_link
        ));
    }

    let dylib_sha256 = sha256_file(&dylib_path)?;
    let spec_path = bundle_root.join(&manifest.target_spec);
    let (target_arg, target_name, is_json_target) = resolve_target(&spec_path, req.panic)?;

    // Fast path: reuse a prebuilt sysroot from `manifest.json` instantaneously (no `rust-src` needed!).
    let profile_key = format!("{}:{}", req.profile.as_str(), req.panic.as_str());
    if !req.clean {
        if let Some(rel) = manifest.prebuilt_sysroots.get(&profile_key) {
            let candidate = bundle_root.join(rel);
            if sysroot_build::sysroot_has_core_libs(&candidate, &target_name) {
                if req.verbose {
                    eprintln!(
                        "cargo-mires: using prebuilt sysroot `{profile_key}` at {}",
                        candidate.display()
                    );
                }
                return Ok(ToolchainInfo {
                    channel: manifest.channel.clone(),
                    dylib_path,
                    dylib_sha256,
                    jsc_link_path,
                    sysroot_dir: candidate,
                    target_arg,
                    target_name,
                    is_json_target,
                });
            }
        }
    }

    // Custom profile/panic fallback: build on-demand using bundled `patches/` and `mires-std/`.
    if !manifest.has_sysroot_sources
        || !bundle_root.join("patches").is_dir()
        || !bundle_root.join("mires-std").is_dir()
    {
        let available: Vec<&str> = manifest
            .prebuilt_sysroots
            .keys()
            .map(String::as_str)
            .collect();
        return Err(format!(
            "installed toolchain at `{}` has no prebuilt sysroot for `{profile_key}` \
             (bundled: [{}]) and does not include sysroot sources for on-demand rebuilds",
            bundle_root.display(),
            available.join(", ")
        ));
    }

    let emit_args = format!("js-profile={}", req.profile.as_str());
    let sysroot_dir = sysroot_build::build_sysroot_native(&SysrootBuildRequest {
        sources_root: bundle_root,
        cache_root: bundle_root,
        sysroots_dir: &bundle_root.join("sysroots"),
        dylib_path: &dylib_path,
        dylib_sha256: &dylib_sha256,
        channel: &manifest.channel,
        target_spec: is_json_target.then_some(spec_path.as_path()),
        panic: req.panic,
        emit_args_raw: &emit_args,
        rust_opt_level: None,
        clean: req.clean,
        verbose: req.verbose,
    })?;

    Ok(ToolchainInfo {
        channel: manifest.channel.clone(),
        dylib_path,
        dylib_sha256,
        jsc_link_path,
        sysroot_dir,
        target_arg,
        target_name,
        is_json_target,
    })
}

/// Discover the active Mires root (source checkout or installed bundle).
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "kept as public discovery helper for tests and callers"
    )
)]
pub fn locate_mires_root(cli_override: Option<&Path>) -> Result<PathBuf, String> {
    toolchain::resolve_toolchain_source(cli_override).map(|source| source.root().to_path_buf())
}

fn resolve_target(spec_path: &Path, panic: PanicMode) -> Result<(String, String, bool), String> {
    match panic {
        PanicMode::Unwind => {
            if !spec_path.is_file() {
                return Err(format!(
                    "unwind target spec not found at `{}`",
                    spec_path.display()
                ));
            }
            let canonical = fs::canonicalize(spec_path).unwrap_or_else(|_| spec_path.to_path_buf());
            Ok((
                canonical.to_string_lossy().into_owned(),
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
    DEFAULT_PINNED_CHANNEL.to_owned()
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

/// Remove Cargo/rustc/wrapper environment variables that would interfere with nested tool runs.
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
        assert_eq!(parse_pinned_channel(""), DEFAULT_PINNED_CHANNEL);
    }

    #[test]
    fn locates_checkout_mires_root_via_compile_time_fallback() {
        let root = locate_mires_root(None).unwrap();
        assert!(root.join("rust-toolchain.toml").is_file());
        assert!(root.join("patches").is_dir());
    }
}
