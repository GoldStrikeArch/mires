//! Managed Mires toolchain bundles, OS cache layout, installer (`setup`), and packager (`toolchain`).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::archive;
use crate::cli::{
    PanicMode, Profile, SetupCliOptions, ToolchainCleanOptions, ToolchainCliCommand,
    ToolchainPackageOptions, ToolchainPathOptions,
};
use crate::sysroot::{read_pinned_channel, scrub_cargo_env, sha256_file};
use crate::sysroot_build::{self, SysrootBuildRequest};

/// Default GitHub Releases base URL for downloading prebuilt Mires toolchains.
pub const DEFAULT_RELEASE_URL: &str = "https://github.com/GoldStrikeArch/mires/releases/download";

/// Fallback pinned Rust nightly channel matching the repository's `rust-toolchain.toml`.
pub const DEFAULT_PINNED_CHANNEL: &str = "nightly-2026-07-27";

/// Current `manifest.json` schema version.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Metadata stored in `manifest.json` at the root of an installed Mires toolchain bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolchainManifest {
    pub schema: u32,
    pub version: String,
    pub channel: String,
    pub host: String,
    pub dylib: String,
    pub jsc_link: String,
    pub dylib_sha256: String,
    pub target_spec: String,
    #[serde(default)]
    pub prebuilt_sysroots: BTreeMap<String, String>,
    #[serde(default)]
    pub has_sysroot_sources: bool,
}

/// Resolved source of a Mires compiler toolchain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolchainSource {
    /// Local compiler git checkout containing `rust-toolchain.toml`, `patches/`, and `mires-std/`.
    SourceCheckout(PathBuf),
    /// Self-contained installed bundle containing `manifest.json` and prebuilt binaries/sysroots.
    InstalledBundle {
        root: PathBuf,
        manifest: ToolchainManifest,
    },
}

impl ToolchainSource {
    pub fn root(&self) -> &Path {
        match self {
            Self::SourceCheckout(path) | Self::InstalledBundle { root: path, .. } => path,
        }
    }
}

/// Platform-appropriate codegen backend dylib filename for the current host OS.
pub const fn current_dylib_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "librustc_codegen_mires.dylib"
    } else if cfg!(target_os = "windows") {
        "rustc_codegen_mires.dll"
    } else {
        "librustc_codegen_mires.so"
    }
}

/// Platform-appropriate codegen backend dylib filename for a given host target triple.
pub fn dylib_filename_for_host(host: &str) -> &'static str {
    if host.contains("apple-darwin") {
        "librustc_codegen_mires.dylib"
    } else if host.contains("windows") {
        "rustc_codegen_mires.dll"
    } else {
        "librustc_codegen_mires.so"
    }
}

/// Platform-appropriate `jsc-link` executable filename for the current host OS.
pub const fn current_jsc_link_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "jsc-link.exe"
    } else {
        "jsc-link"
    }
}

/// Platform-appropriate `jsc-link` executable filename for a given host target triple.
pub fn jsc_link_filename_for_host(host: &str) -> &'static str {
    if host.contains("windows") {
        "jsc-link.exe"
    } else {
        "jsc-link"
    }
}

/// Platform-appropriate `cargo-mires` executable filename for a given host target triple.
pub fn cargo_mires_filename_for_host(host: &str) -> &'static str {
    if host.contains("windows") {
        "cargo-mires.exe"
    } else {
        "cargo-mires"
    }
}

/// Best-effort Rust target triple for the current host machine.
pub fn current_host_triple() -> String {
    if let Some(known) = compile_time_host_triple() {
        return known.to_owned();
    }
    if let Ok(output) = Command::new("rustc").arg("-vV").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(host) = line.strip_prefix("host:") {
                    let trimmed = host.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_owned();
                    }
                }
            }
        }
    }
    format!("{}-unknown-{}", env::consts::ARCH, env::consts::OS)
}

const fn compile_time_host_triple() -> Option<&'static str> {
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        Some("aarch64-unknown-linux-gnu")
    } else if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_arch = "aarch64", target_os = "windows")) {
        Some("aarch64-pc-windows-msvc")
    } else {
        None
    }
}

/// Determine the OS-appropriate Mires cache/home directory (`MIRES_HOME` override supported).
pub fn mires_home_dir() -> Result<PathBuf, String> {
    resolve_mires_home_from(
        env::var_os("MIRES_HOME").map(PathBuf::from),
        env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
        env::var_os("LOCALAPPDATA").map(PathBuf::from),
        env::var_os("USERPROFILE").map(PathBuf::from),
        env::consts::OS,
    )
}

fn resolve_mires_home_from(
    mires_home: Option<PathBuf>,
    xdg_cache_home: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    user_profile: Option<PathBuf>,
    os: &str,
) -> Result<PathBuf, String> {
    if let Some(explicit) = mires_home.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(explicit);
    }

    match os {
        "macos" => {
            if let Some(xdg) = xdg_cache_home.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(xdg.join("mires"));
            }
            if let Some(h) = home.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(h.join("Library").join("Caches").join("mires"));
            }
        }
        "windows" => {
            if let Some(local) = local_app_data.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(local.join("mires"));
            }
            if let Some(profile) = user_profile.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(profile.join(".cache").join("mires"));
            }
        }
        _ => {
            if let Some(xdg) = xdg_cache_home.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(xdg.join("mires"));
            }
            if let Some(h) = home.filter(|p| !p.as_os_str().is_empty()) {
                return Ok(h.join(".cache").join("mires"));
            }
        }
    }

    Err("could not determine Mires cache directory; set `MIRES_HOME`".to_owned())
}

/// Normalize a version string by stripping an optional leading `v`.
pub fn normalize_version(raw: &str) -> String {
    raw.trim()
        .strip_prefix('v')
        .unwrap_or(raw.trim())
        .to_owned()
}

/// Directory name under `<MIRES_HOME>/toolchains/` for a version (e.g. `v0.1.0`).
pub fn version_dir_name(version: &str) -> String {
    format!("v{}", normalize_version(version))
}

/// Active default toolchain version (`MIRES_TOOLCHAIN` env var or `CARGO_PKG_VERSION`).
pub fn default_toolchain_version() -> String {
    env::var("MIRES_TOOLCHAIN")
        .ok()
        .map(|s| normalize_version(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

/// True if `dir` looks like a Mires compiler source checkout.
pub fn is_source_checkout(dir: &Path) -> bool {
    dir.join("rust-toolchain.toml").is_file()
        && dir.join("patches").is_dir()
        && dir.join("mires-std").is_dir()
}

/// Load and validate `manifest.json` inside an installed toolchain bundle directory.
pub fn load_bundle_manifest(bundle_dir: &Path) -> Result<ToolchainManifest, String> {
    let manifest_path = bundle_dir.join("manifest.json");
    let text = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read `{}`: {err}", manifest_path.display()))?;
    let manifest: ToolchainManifest = serde_json::from_str(&text)
        .map_err(|err| format!("failed to parse `{}`: {err}", manifest_path.display()))?;
    if manifest.schema != MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "unsupported toolchain manifest schema {} in `{}` (expected {})",
            manifest.schema,
            manifest_path.display(),
            MANIFEST_SCHEMA_VERSION
        ));
    }
    Ok(manifest)
}

/// Resolve the active Mires toolchain source following the documented precedence order:
/// 1. `--mires-root` / `MIRES_ROOT` (accepts source checkout OR installed bundle)
/// 2. Sibling source checkout relative to `current_exe()` (unless `MIRES_NO_DEV_FALLBACK=1`)
/// 3. Sibling installed bundle relative to `current_exe()`
/// 4. Managed cached bundle `<MIRES_HOME>/toolchains/v<version>/`
/// 5. Compile-time `CARGO_MANIFEST_DIR/..` checkout fallback (unless `MIRES_NO_DEV_FALLBACK=1`)
/// 6. Optional auto-install when `MIRES_AUTO_INSTALL=1`
pub fn resolve_toolchain_source(cli_override: Option<&Path>) -> Result<ToolchainSource, String> {
    let env_root = env::var_os("MIRES_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let current_exe = env::current_exe().ok();
    let mires_home = mires_home_dir().ok();
    let version = default_toolchain_version();
    let allow_dev_fallback = env::var("MIRES_NO_DEV_FALLBACK").as_deref() != Ok("1");
    let auto_install = env::var("MIRES_AUTO_INSTALL").as_deref() == Ok("1");

    let resolved = resolve_toolchain_source_with(&ResolutionEnv {
        cli_override,
        env_root: env_root.as_deref(),
        current_exe: current_exe.as_deref(),
        mires_home: mires_home.as_deref(),
        version: &version,
        allow_dev_fallback,
    });

    match resolved {
        Ok(source) => Ok(source),
        Err(_err) if auto_install && cli_override.is_none() && env_root.is_none() => {
            eprintln!("cargo-mires: no toolchain found; auto-installing v{version}...");
            let installed_dir = setup_toolchain(&SetupCliOptions {
                version: Some(version),
                ..Default::default()
            })?;
            let manifest = load_bundle_manifest(&installed_dir)?;
            Ok(ToolchainSource::InstalledBundle {
                root: installed_dir,
                manifest,
            })
        }
        Err(err) => Err(err),
    }
}

struct ResolutionEnv<'a> {
    cli_override: Option<&'a Path>,
    env_root: Option<&'a Path>,
    current_exe: Option<&'a Path>,
    mires_home: Option<&'a Path>,
    version: &'a str,
    allow_dev_fallback: bool,
}

fn resolve_toolchain_source_with(env_ctx: &ResolutionEnv<'_>) -> Result<ToolchainSource, String> {
    // 1. Explicit `--mires-root` or `MIRES_ROOT`.
    if let Some(path) = env_ctx.cli_override {
        return validate_explicit_root(path, "--mires-root");
    }
    if let Some(path) = env_ctx.env_root {
        return validate_explicit_root(path, "MIRES_ROOT");
    }

    // 2. Sibling source checkout relative to `current_exe()` (for local compiler development).
    if env_ctx.allow_dev_fallback {
        if let Some(exe) = env_ctx.current_exe {
            let mut cursor = exe.parent();
            for _ in 0..4 {
                let Some(dir) = cursor else {
                    break;
                };
                if is_source_checkout(dir) {
                    return Ok(ToolchainSource::SourceCheckout(canonical_or_owned(dir)));
                }
                cursor = dir.parent();
            }
        }
    }

    // 3. Sibling installed bundle relative to `current_exe()` (`<exe_dir>` or `<exe_dir>/..`).
    if let Some(exe) = env_ctx.current_exe {
        if let Some(exe_dir) = exe.parent() {
            for candidate in [Some(exe_dir), exe_dir.parent()].into_iter().flatten() {
                if candidate.join("manifest.json").is_file() {
                    if let Ok(manifest) = load_bundle_manifest(candidate) {
                        return Ok(ToolchainSource::InstalledBundle {
                            root: canonical_or_owned(candidate),
                            manifest,
                        });
                    }
                }
            }
        }
    }

    // 4. Managed cached bundle at `<MIRES_HOME>/toolchains/v<version>/`.
    if let Some(home) = env_ctx.mires_home {
        let managed = home
            .join("toolchains")
            .join(version_dir_name(env_ctx.version));
        if managed.join("manifest.json").is_file() {
            let manifest = load_bundle_manifest(&managed)?;
            return Ok(ToolchainSource::InstalledBundle {
                root: canonical_or_owned(&managed),
                manifest,
            });
        }
    }

    // 5. Compile-time `CARGO_MANIFEST_DIR/..` checkout fallback (development only).
    if env_ctx.allow_dev_fallback {
        if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
            let candidate = Path::new(manifest_dir)
                .parent()
                .unwrap_or_else(|| Path::new(manifest_dir));
            if is_source_checkout(candidate) {
                return Ok(ToolchainSource::SourceCheckout(canonical_or_owned(
                    candidate,
                )));
            }
        }
    }

    let hint_dir = env_ctx.mires_home.map_or_else(
        || "<MIRES_HOME>/toolchains".to_owned(),
        |h| {
            h.join("toolchains")
                .join(version_dir_name(env_ctx.version))
                .display()
                .to_string()
        },
    );
    Err(format!(
        "no Mires toolchain found (checked `{hint_dir}`); \
         run `cargo mires setup` to install v{}, or pass `--mires-root <path>` / set `MIRES_ROOT`",
        normalize_version(env_ctx.version)
    ))
}

fn validate_explicit_root(path: &Path, source: &str) -> Result<ToolchainSource, String> {
    if path.join("manifest.json").is_file() {
        let manifest = load_bundle_manifest(path)?;
        return Ok(ToolchainSource::InstalledBundle {
            root: canonical_or_owned(path),
            manifest,
        });
    }
    if is_source_checkout(path) {
        return Ok(ToolchainSource::SourceCheckout(canonical_or_owned(path)));
    }
    Err(format!(
        "`{}` (from {source}) is neither a valid Mires source checkout \
         (`rust-toolchain.toml`, `patches/`, `mires-std/`) nor an installed toolchain bundle \
         (`manifest.json`)",
        path.display()
    ))
}

/// Ensure `rustup` has the pinned nightly `channel` installed (and `rust-src` when `needs_rust_src`).
pub fn ensure_rustup_channel(
    channel: &str,
    needs_rust_src: bool,
    verbose: bool,
) -> Result<(), String> {
    let no_rustup = env::var("MIRES_NO_RUSTUP").as_deref() == Ok("1");

    if !rustc_channel_is_installed(channel) {
        if no_rustup {
            return Err(format!(
                "Rust toolchain `{channel}` is not installed (and `MIRES_NO_RUSTUP=1` is set); \
                 run `rustup toolchain install {channel} --profile minimal`"
            ));
        }
        eprintln!("cargo-mires: installing Rust toolchain `{channel}` via rustup...");
        let mut cmd = Command::new("rustup");
        cmd.args([
            "toolchain",
            "install",
            channel,
            "--profile",
            "minimal",
            "--no-self-update",
        ])
        .stdout(if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::inherit());
        scrub_cargo_env(&mut cmd);

        let status = cmd.status().map_err(|err| {
            format!(
                "failed to run `rustup` to install `{channel}`: {err} \
                 (install rustup from https://rustup.rs/)"
            )
        })?;
        if !status.success() {
            return Err(format!(
                "`rustup toolchain install {channel}` failed with {status}"
            ));
        }
    }

    if needs_rust_src && !rust_src_is_installed(channel) {
        if no_rustup {
            return Err(format!(
                "`rust-src` component for `{channel}` is not installed (`MIRES_NO_RUSTUP=1`); \
                 run `rustup component add rust-src --toolchain {channel}`"
            ));
        }
        eprintln!("cargo-mires: installing `rust-src` component for `{channel}`...");
        let mut cmd = Command::new("rustup");
        cmd.args(["component", "add", "rust-src", "--toolchain", channel])
            .stdout(if verbose {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::inherit());
        scrub_cargo_env(&mut cmd);

        let status = cmd
            .status()
            .map_err(|err| format!("failed to run `rustup component add rust-src`: {err}"))?;
        if !status.success() {
            return Err(format!(
                "`rustup component add rust-src --toolchain {channel}` failed with {status}"
            ));
        }
    }

    Ok(())
}

fn rustc_channel_is_installed(channel: &str) -> bool {
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("+{channel}"))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    scrub_cargo_env(&mut cmd);
    cmd.status().is_ok_and(|s| s.success())
}

fn rust_src_is_installed(channel: &str) -> bool {
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("+{channel}"))
        .args(["--print", "sysroot"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    scrub_cargo_env(&mut cmd);
    let Ok(output) = cmd.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    sysroot.join("lib/rustlib/src/rust/library/core").is_dir()
}

/// Parse a `SHA256SUMS` or `.sha256` file and return the lowercase 64-hex digest matching `filename`.
pub fn parse_sha256sums(content: &str, filename: &str) -> Option<String> {
    let mut bare_single: Option<String> = None;
    let mut non_empty_lines = 0usize;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        non_empty_lines += 1;

        let mut parts = line.splitn(2, char::is_whitespace);
        let hex = parts.next()?.trim();
        if !is_valid_sha256_hex(hex) {
            continue;
        }
        match parts.next().map(str::trim) {
            Some(rest) if !rest.is_empty() => {
                let candidate = rest.trim_start_matches('*');
                let base = candidate.rsplit('/').next().unwrap_or(candidate);
                if candidate == filename || base == filename {
                    return Some(hex.to_ascii_lowercase());
                }
            }
            _ => {
                bare_single = Some(hex.to_ascii_lowercase());
            }
        }
    }

    if non_empty_lines == 1 {
        return bare_single;
    }
    None
}

fn is_valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Execute `cargo mires setup`.
pub fn run_setup_cmd(opts: &SetupCliOptions) -> Result<(), String> {
    let installed_dir = setup_toolchain(opts)?;
    let manifest = load_bundle_manifest(&installed_dir)?;
    println!("Installed Mires toolchain v{}", manifest.version);
    println!("  path:    {}", installed_dir.display());
    println!("  host:    {}", manifest.host);
    println!("  channel: {}", manifest.channel);
    if !manifest.prebuilt_sysroots.is_empty() {
        let profiles: Vec<&str> = manifest
            .prebuilt_sysroots
            .keys()
            .map(String::as_str)
            .collect();
        println!("  sysroots: {}", profiles.join(", "));
    }
    Ok(())
}

/// Install a Mires toolchain bundle into `<MIRES_HOME>/toolchains/v<version>/`.
pub fn setup_toolchain(opts: &SetupCliOptions) -> Result<PathBuf, String> {
    let home = mires_home_dir()?;
    setup_toolchain_into(opts, &home)
}

/// Install a Mires toolchain bundle into an explicit `mires_home` cache root.
pub fn setup_toolchain_into(opts: &SetupCliOptions, mires_home: &Path) -> Result<PathBuf, String> {
    let requested_version = opts
        .version
        .as_deref()
        .map(normalize_version)
        .unwrap_or_else(default_toolchain_version);
    let toolchains_dir = mires_home.join("toolchains");
    let default_target_dir = toolchains_dir.join(version_dir_name(&requested_version));

    if opts.from_archive.is_none()
        && !opts.force
        && default_target_dir.join("manifest.json").is_file()
    {
        let manifest = load_bundle_manifest(&default_target_dir)?;
        if !opts.no_rustup {
            ensure_rustup_channel(&manifest.channel, false, opts.verbose)?;
        }
        if opts.verbose {
            eprintln!(
                "cargo-mires: toolchain v{} already installed at {} (pass `--force` to reinstall)",
                manifest.version,
                default_target_dir.display()
            );
        }
        return Ok(default_target_dir);
    }

    let tmp_dir = unique_work_dir(&mires_home.join("tmp"), "setup")?;
    let _tmp_cleanup = DirGuard(tmp_dir.clone());

    let archive_path = match &opts.from_archive {
        Some(local_archive) => prepare_local_archive(local_archive, opts.sha256.as_deref())?,
        None => download_verified_archive(opts, &requested_version, &tmp_dir)?,
    };

    fs::create_dir_all(&toolchains_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", toolchains_dir.display()))?;
    let staging_dir = unique_work_dir(&toolchains_dir, &format!(".staging-v{requested_version}"))?;
    let _staging_cleanup = DirGuard(staging_dir.clone());

    archive::extract_tar_gz(&archive_path, &staging_dir)?;
    let manifest = verify_staged_bundle(&staging_dir)?;

    let final_version = if opts.version.is_some() {
        requested_version
    } else {
        normalize_version(&manifest.version)
    };
    let final_dir = toolchains_dir.join(version_dir_name(&final_version));

    atomic_swap_dir(&staging_dir, &final_dir)?;

    if !opts.no_rustup {
        ensure_rustup_channel(&manifest.channel, false, opts.verbose)?;
    }

    Ok(final_dir)
}

fn prepare_local_archive(
    archive_path: &Path,
    explicit_sha256: Option<&str>,
) -> Result<PathBuf, String> {
    if !archive_path.is_file() {
        return Err(format!(
            "archive `{}` does not exist",
            archive_path.display()
        ));
    }

    let expected = if let Some(sha) = explicit_sha256 {
        let trimmed = sha.trim();
        if !is_valid_sha256_hex(trimmed) {
            return Err(format!(
                "invalid `--sha256` value `{trimmed}` (expected 64 hex characters)"
            ));
        }
        Some(trimmed.to_ascii_lowercase())
    } else {
        let sibling_sha = append_suffix(archive_path, ".sha256");
        if sibling_sha.is_file() {
            let content = fs::read_to_string(&sibling_sha)
                .map_err(|err| format!("failed to read `{}`: {err}", sibling_sha.display()))?;
            let archive_name = archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            parse_sha256sums(&content, archive_name)
        } else {
            None
        }
    };

    if let Some(expected_sha) = expected {
        verify_archive_sha256(archive_path, &expected_sha)?;
    }

    Ok(archive_path.to_path_buf())
}

fn download_verified_archive(
    opts: &SetupCliOptions,
    version: &str,
    tmp_dir: &Path,
) -> Result<PathBuf, String> {
    let host = current_host_triple();
    let archive_name = format!("mires-toolchain-v{version}-{host}.tar.gz");
    let base_url = opts
        .url
        .clone()
        .or_else(|| env::var("MIRES_RELEASE_URL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| DEFAULT_RELEASE_URL.to_owned());
    let base_trimmed = base_url.trim_end_matches('/');
    let tag_suffix = format!("/v{version}");
    let tag_url = if base_trimmed.ends_with(&tag_suffix) {
        base_trimmed.to_owned()
    } else {
        format!("{base_trimmed}{tag_suffix}")
    };
    let archive_url = format!("{tag_url}/{archive_name}");

    let expected_sha = if let Some(explicit) = &opts.sha256 {
        let trimmed = explicit.trim();
        if !is_valid_sha256_hex(trimmed) {
            return Err(format!(
                "invalid `--sha256` value `{trimmed}` (expected 64 hex characters)"
            ));
        }
        trimmed.to_ascii_lowercase()
    } else {
        let sums_path = tmp_dir.join("SHA256SUMS");
        let sums_url = format!("{tag_url}/SHA256SUMS");
        let sums_downloaded = run_curl_download(&sums_url, &sums_path, opts.verbose);
        let sums_text = match sums_downloaded {
            Ok(()) => fs::read_to_string(&sums_path)
                .map_err(|err| format!("failed to read downloaded `SHA256SUMS`: {err}"))?,
            Err(primary_err) => {
                let single_url = format!("{archive_url}.sha256");
                run_curl_download(&single_url, &sums_path, opts.verbose).map_err(|_| {
                    format!(
                        "failed to download checksums from `{sums_url}` ({primary_err}); \
                         pass `--sha256 <hex>` or `--from-archive <path>`"
                    )
                })?;
                fs::read_to_string(&sums_path)
                    .map_err(|err| format!("failed to read downloaded `.sha256`: {err}"))?
            }
        };
        parse_sha256sums(&sums_text, &archive_name).ok_or_else(|| {
            format!("checksum for `{archive_name}` not found in downloaded `SHA256SUMS`")
        })?
    };

    let archive_path = tmp_dir.join(&archive_name);
    eprintln!("cargo-mires: downloading {archive_url}...");
    run_curl_download(&archive_url, &archive_path, opts.verbose)?;
    verify_archive_sha256(&archive_path, &expected_sha)?;
    Ok(archive_path)
}

fn run_curl_download(url: &str, dest: &Path, verbose: bool) -> Result<(), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-f", "-s", "-S", "-L", "-o"])
        .arg(dest)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(if verbose {
            Stdio::inherit()
        } else {
            Stdio::piped()
        });
    let output = cmd
        .output()
        .map_err(|err| format!("failed to invoke `curl` to download `{url}`: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("`curl` exited with {} downloading `{url}`", output.status)
        } else {
            format!("download of `{url}` failed: {stderr}")
        });
    }
    Ok(())
}

fn verify_archive_sha256(archive_path: &Path, expected_hex: &str) -> Result<(), String> {
    let actual = sha256_file(archive_path)?;
    if !actual.eq_ignore_ascii_case(expected_hex) {
        return Err(format!(
            "SHA-256 mismatch for `{}`: expected `{expected_hex}`, got `{actual}`",
            archive_path.display()
        ));
    }
    Ok(())
}

fn verify_staged_bundle(staging_dir: &Path) -> Result<ToolchainManifest, String> {
    let manifest = load_bundle_manifest(staging_dir)?;
    let dylib_path = staging_dir.join(&manifest.dylib);
    if !dylib_path.is_file() {
        return Err(format!(
            "toolchain bundle is missing backend dylib `{}`",
            manifest.dylib
        ));
    }
    let actual_dylib_sha = sha256_file(&dylib_path)?;
    if !actual_dylib_sha.eq_ignore_ascii_case(&manifest.dylib_sha256) {
        return Err(format!(
            "backend dylib SHA-256 mismatch in bundle: manifest says `{}`, actual `{actual_dylib_sha}`",
            manifest.dylib_sha256
        ));
    }

    let jsc_link_path = staging_dir.join(&manifest.jsc_link);
    if !jsc_link_path.is_file() {
        return Err(format!(
            "toolchain bundle is missing linker `{}`",
            manifest.jsc_link
        ));
    }

    let target_spec_path = staging_dir.join(&manifest.target_spec);
    if !target_spec_path.is_file() {
        return Err(format!(
            "toolchain bundle is missing target spec `{}`",
            manifest.target_spec
        ));
    }

    ensure_executable(&dylib_path)?;
    ensure_executable(&jsc_link_path)?;
    Ok(manifest)
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta =
        fs::metadata(path).map_err(|err| format!("failed to stat `{}`: {err}", path.display()))?;
    let mode = meta.permissions().mode();
    if mode & 0o111 == 0 {
        fs::set_permissions(path, fs::Permissions::from_mode((mode & 0o777) | 0o755))
            .map_err(|err| format!("failed to chmod `{}`: {err}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn atomic_swap_dir(staging_dir: &Path, final_dir: &Path) -> Result<(), String> {
    let mut backup: Option<PathBuf> = None;
    if final_dir.exists() {
        let parent = final_dir
            .parent()
            .ok_or_else(|| format!("invalid destination `{}`", final_dir.display()))?;
        let name = final_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("toolchain");
        let old = parent.join(format!(".old-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&old);
        fs::rename(final_dir, &old).map_err(|err| {
            format!(
                "failed to replace existing toolchain `{}`: {err}",
                final_dir.display()
            )
        })?;
        backup = Some(old);
    }

    if let Err(err) = fs::rename(staging_dir, final_dir) {
        if let Some(old) = backup {
            let _ = fs::rename(old, final_dir);
        }
        return Err(format!(
            "failed to move staged toolchain into `{}`: {err}",
            final_dir.display()
        ));
    }

    if let Some(old) = backup {
        let _ = fs::remove_dir_all(old);
    }
    Ok(())
}

/// Dispatch `cargo mires toolchain <list|path|clean|package>`.
pub fn run_toolchain_cmd(cmd: &ToolchainCliCommand) -> Result<(), String> {
    match cmd {
        ToolchainCliCommand::List => {
            let home = mires_home_dir()?;
            let output = format_toolchain_list(&home, &default_toolchain_version())?;
            print!("{output}");
            Ok(())
        }
        ToolchainCliCommand::Path(opts) => run_toolchain_path(opts),
        ToolchainCliCommand::Clean(opts) => {
            let home = mires_home_dir()?;
            let removed = clean_toolchains_in(&home, opts)?;
            if removed.is_empty() {
                if !opts.all && opts.version.is_none() {
                    println!(
                        "No temporary files to clean. Pass `--all` or `--version <v>` to remove installed toolchains."
                    );
                } else {
                    println!("Nothing to clean.");
                }
            } else {
                for path in removed {
                    println!("Removed {}", path.display());
                }
            }
            Ok(())
        }
        ToolchainCliCommand::Package(opts) => {
            let archive_path = package_toolchain(opts)?;
            let sha = sha256_file(&archive_path)?;
            let size = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
            println!("Packaged Mires toolchain:");
            println!("  archive: {}", archive_path.display());
            println!("  size:    {size} bytes");
            println!("  sha256:  {sha}");
            Ok(())
        }
    }
}

fn run_toolchain_path(opts: &ToolchainPathOptions) -> Result<(), String> {
    let source = resolve_toolchain_source(opts.mires_root.as_deref())?;
    println!("{}", source.root().display());
    Ok(())
}

/// Format installed toolchains under `<mires_home>/toolchains/` for `toolchain list`.
pub fn format_toolchain_list(mires_home: &Path, active_version: &str) -> Result<String, String> {
    let toolchains_dir = mires_home.join("toolchains");
    if !toolchains_dir.is_dir() {
        return Ok(format!(
            "No managed Mires toolchains installed under `{}`.\nRun `cargo mires setup` to install one.\n",
            toolchains_dir.display()
        ));
    }

    let mut dirs: Vec<PathBuf> = fs::read_dir(&toolchains_dir)
        .map_err(|err| format!("failed to read `{}`: {err}", toolchains_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.starts_with('.'))
        })
        .collect();
    dirs.sort();

    if dirs.is_empty() {
        return Ok(format!(
            "No managed Mires toolchains installed under `{}`.\nRun `cargo mires setup` to install one.\n",
            toolchains_dir.display()
        ));
    }

    let active_dir_name = version_dir_name(active_version);
    let mut out = String::new();
    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>");
        let marker = if name == active_dir_name { " *" } else { "" };
        match load_bundle_manifest(&dir) {
            Ok(manifest) => {
                let sysroots: Vec<&str> = manifest
                    .prebuilt_sysroots
                    .keys()
                    .map(String::as_str)
                    .collect();
                let sysroot_summary = if sysroots.is_empty() {
                    "none".to_owned()
                } else {
                    sysroots.join(", ")
                };
                out.push_str(&format!(
                    "{name}{marker}  channel={}  host={}  sysroots=[{}]  {}\n",
                    manifest.channel,
                    manifest.host,
                    sysroot_summary,
                    dir.display()
                ));
            }
            Err(_) => {
                out.push_str(&format!(
                    "{name}  (corrupted/incomplete)  {}\n",
                    dir.display()
                ));
            }
        }
    }
    Ok(out)
}

/// Remove cached toolchains / staging artifacts under `mires_home`.
pub fn clean_toolchains_in(
    mires_home: &Path,
    opts: &ToolchainCleanOptions,
) -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    let toolchains_dir = mires_home.join("toolchains");

    if let Some(version) = &opts.version {
        let target = toolchains_dir.join(version_dir_name(version));
        if !target.exists() {
            return Err(format!(
                "toolchain `{}` is not installed under `{}`",
                version_dir_name(version),
                toolchains_dir.display()
            ));
        }
        fs::remove_dir_all(&target)
            .map_err(|err| format!("failed to remove `{}`: {err}", target.display()))?;
        removed.push(target);
        return Ok(removed);
    }

    if opts.all {
        if toolchains_dir.is_dir() {
            for entry in fs::read_dir(&toolchains_dir)
                .map_err(|err| format!("failed to read `{}`: {err}", toolchains_dir.display()))?
                .flatten()
            {
                let path = entry.path();
                remove_path_recursive(&path)?;
                removed.push(path);
            }
        }
        let tmp_dir = mires_home.join("tmp");
        if tmp_dir.is_dir() {
            fs::remove_dir_all(&tmp_dir)
                .map_err(|err| format!("failed to remove `{}`: {err}", tmp_dir.display()))?;
            removed.push(tmp_dir);
        }
        return Ok(removed);
    }

    // Default safe cleanup: remove temporary download/staging dirs only.
    let tmp_dir = mires_home.join("tmp");
    if tmp_dir.is_dir() {
        fs::remove_dir_all(&tmp_dir)
            .map_err(|err| format!("failed to remove `{}`: {err}", tmp_dir.display()))?;
        removed.push(tmp_dir);
    }
    if toolchains_dir.is_dir() {
        for entry in fs::read_dir(&toolchains_dir)
            .map_err(|err| format!("failed to read `{}`: {err}", toolchains_dir.display()))?
            .flatten()
        {
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|n| n.starts_with(".staging-") || n.starts_with(".old-"))
            {
                let path = entry.path();
                remove_path_recursive(&path)?;
                removed.push(path);
            }
        }
    }
    Ok(removed)
}

/// Package a Mires source checkout into a self-contained `.tar.gz` bundle + `.sha256` file.
pub fn package_toolchain(opts: &ToolchainPackageOptions) -> Result<PathBuf, String> {
    let source = resolve_toolchain_source(opts.mires_root.as_deref())?;
    let ToolchainSource::SourceCheckout(checkout_root) = source else {
        return Err(format!(
            "`{}` is already an installed bundle; `toolchain package` requires a Mires source checkout",
            source.root().display()
        ));
    };

    let version = opts
        .version
        .as_deref()
        .map(normalize_version)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let host = opts.host.clone().unwrap_or_else(current_host_triple);
    let channel = read_pinned_channel(&checkout_root);

    ensure_rustup_channel(&channel, false, opts.verbose)?;

    let (dylib_src, jsc_link_src) =
        ensure_checkout_backend_built(&checkout_root, &channel, opts.verbose)?;
    let checkout_dylib_sha = sha256_file(&dylib_src)?;

    let dylib_name = dylib_filename_for_host(&host);
    let jsc_link_name = jsc_link_filename_for_host(&host);
    let target_spec_src = checkout_root.join("targets/mires32-unwind.json");
    if !target_spec_src.is_file() {
        return Err(format!(
            "missing target spec `{}`",
            target_spec_src.display()
        ));
    }

    let stage_parent = opts
        .out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(env::temp_dir);
    let stage_dir = unique_work_dir(&stage_parent, ".mires-package-stage")?;
    let _stage_guard = DirGuard(stage_dir.clone());

    let bin_dir = stage_dir.join("bin");
    let targets_dir = stage_dir.join("targets");
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", bin_dir.display()))?;
    fs::create_dir_all(&targets_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", targets_dir.display()))?;

    let staged_dylib = bin_dir.join(dylib_name);
    let staged_jsc_link = bin_dir.join(jsc_link_name);
    copy_with_permissions(&dylib_src, &staged_dylib)?;
    copy_with_permissions(&jsc_link_src, &staged_jsc_link)?;

    if cfg!(target_os = "macos") && host.contains("apple-darwin") {
        fixup_macos_dylib_id(&staged_dylib)?;
    }

    if opts.include_cli {
        let cli_name = cargo_mires_filename_for_host(&host);
        let release_cli = checkout_root.join("target/release").join(cli_name);
        let cli_src = if release_cli.is_file() {
            release_cli
        } else {
            env::current_exe().map_err(|err| {
                format!("failed to locate `cargo-mires` binary for `--include-cli`: {err}")
            })?
        };
        copy_with_permissions(&cli_src, &bin_dir.join(cli_name))?;
    }

    fs::copy(&target_spec_src, targets_dir.join("mires32-unwind.json"))
        .map_err(|err| format!("failed to copy `{}`: {err}", target_spec_src.display()))?;

    copy_dir_filtered(&checkout_root.join("patches"), &stage_dir.join("patches"))?;
    copy_dir_filtered(
        &checkout_root.join("mires-std"),
        &stage_dir.join("mires-std"),
    )?;

    let mut prebuilt_sysroots = BTreeMap::new();
    if !opts.no_sysroots {
        let sysroots_out = stage_dir.join("sysroots");
        fs::create_dir_all(&sysroots_out)
            .map_err(|err| format!("failed to create `{}`: {err}", sysroots_out.display()))?;
        for profile in [Profile::Dev, Profile::ReleaseBalanced] {
            let sysroot_dir = sysroot_build::build_sysroot_native(&SysrootBuildRequest {
                sources_root: &checkout_root,
                cache_root: &checkout_root.join("build"),
                sysroots_dir: &checkout_root.join("build/sysroots"),
                dylib_path: &dylib_src,
                dylib_sha256: &checkout_dylib_sha,
                channel: &channel,
                target_spec: Some(&target_spec_src),
                panic: PanicMode::Unwind,
                emit_args_raw: &format!("js-profile={}", profile.as_str()),
                rust_opt_level: None,
                clean: false,
                verbose: opts.verbose,
            })?;
            let key = sysroot_dir
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("invalid sysroot dir `{}`", sysroot_dir.display()))?;
            let dst_sysroot = sysroots_out.join(key);
            if !dst_sysroot.exists() {
                copy_dir_filtered(&sysroot_dir, &dst_sysroot)?;
            }
            prebuilt_sysroots.insert(
                format!("{}:{}", profile.as_str(), PanicMode::Unwind.as_str()),
                format!("sysroots/{key}"),
            );
        }
    }

    let final_dylib_sha = sha256_file(&staged_dylib)?;
    let manifest = ToolchainManifest {
        schema: MANIFEST_SCHEMA_VERSION,
        version,
        channel,
        host,
        dylib: format!("bin/{dylib_name}"),
        jsc_link: format!("bin/{jsc_link_name}"),
        dylib_sha256: final_dylib_sha,
        target_spec: "targets/mires32-unwind.json".to_owned(),
        prebuilt_sysroots,
        has_sysroot_sources: true,
    };

    let mut manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|err| format!("failed to serialize `manifest.json`: {err}"))?;
    manifest_json.push('\n');
    fs::write(stage_dir.join("manifest.json"), manifest_json.as_bytes())
        .map_err(|err| format!("failed to write staged `manifest.json`: {err}"))?;

    archive::create_tar_gz(&stage_dir, &opts.out)?;

    let archive_sha = sha256_file(&opts.out)?;
    let archive_name = opts
        .out
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("mires-toolchain.tar.gz");
    let sha_file = append_suffix(&opts.out, ".sha256");
    fs::write(
        &sha_file,
        format!("{archive_sha}  {archive_name}\n").as_bytes(),
    )
    .map_err(|err| format!("failed to write `{}`: {err}", sha_file.display()))?;

    Ok(opts.out.clone())
}

/// Build the compiler backend dylib and `jsc-link` inside a source checkout using Cargo directly.
pub fn ensure_checkout_backend_built(
    checkout_root: &Path,
    channel: &str,
    verbose: bool,
) -> Result<(PathBuf, PathBuf), String> {
    let release_dir = checkout_root.join("target/release");
    let dylib_path = release_dir.join(current_dylib_filename());
    let default_linker = release_dir.join(current_jsc_link_filename());
    let jsc_link_path = env::var_os("JSC_LINK_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or(default_linker);

    if dylib_path.is_file() && jsc_link_path.is_file() {
        return Ok((dylib_path, jsc_link_path));
    }

    eprintln!("cargo-mires: building compiler backend (`rustc_codegen_mires` + `jsc-link`)...");
    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{channel}"))
        .args([
            "build",
            "--release",
            "-p",
            "rustc_codegen_mires",
            "-p",
            "jsc-link",
        ])
        .current_dir(checkout_root)
        .stdout(if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::inherit());
    scrub_cargo_env(&mut cmd);

    let status = cmd
        .status()
        .map_err(|err| format!("failed to run `cargo build` for backend: {err}"))?;
    if !status.success() {
        return Err(format!("backend build failed with {status}"));
    }

    let final_linker = if jsc_link_path.is_file() {
        jsc_link_path
    } else {
        release_dir.join(current_jsc_link_filename())
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

fn fixup_macos_dylib_id(staged_dylib: &Path) -> Result<(), String> {
    let status = Command::new("install_name_tool")
        .args(["-id", "@rpath/librustc_codegen_mires.dylib"])
        .arg(staged_dylib)
        .status()
        .map_err(|err| format!("failed to run `install_name_tool`: {err}"))?;
    if !status.success() {
        return Err(format!("`install_name_tool` failed with {status}"));
    }
    // Re-sign ad-hoc so Apple Silicon macOS accepts the modified Mach-O dylib on dlopen.
    let sign = Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(staged_dylib)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Ok(sign_status) = sign {
        if !sign_status.success() {
            return Err(format!(
                "`codesign --force --sign -` failed with {sign_status}"
            ));
        }
    }
    Ok(())
}

fn copy_with_permissions(src: &Path, dst: &Path) -> Result<(), String> {
    fs::copy(src, dst).map_err(|err| {
        format!(
            "failed to copy `{}` to `{}`: {err}",
            src.display(),
            dst.display()
        )
    })?;
    if let Ok(meta) = fs::metadata(src) {
        let _ = fs::set_permissions(dst, meta.permissions());
    }
    Ok(())
}

fn copy_dir_filtered(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|err| format!("failed to create `{}`: {err}", dst.display()))?;
    let entries =
        fs::read_dir(src).map_err(|err| format!("failed to read `{}`: {err}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read `{}`: {err}", src.display()))?;
        let name = entry.file_name();
        if name == "target" || name == ".DS_Store" {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to stat `{}`: {err}", src_path.display()))?;
        if file_type.is_dir() {
            copy_dir_filtered(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            copy_with_permissions(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn unique_work_dir(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = parent.join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)
        .map_err(|err| format!("failed to create `{}`: {err}", dir.display()))?;
    Ok(dir)
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn remove_path_recursive(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove `{}`: {err}", path.display()))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove `{}`: {err}", path.display()))
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "cargo-mires-toolchain-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_sample_bundle_archive(work_dir: &Path, version: &str) -> (PathBuf, String) {
        let stage = work_dir.join("bundle-src");
        fs::create_dir_all(stage.join("bin")).unwrap();
        fs::create_dir_all(stage.join("targets")).unwrap();
        let sysroot_lib = stage.join("sysroots/deadbeef/lib/rustlib/mires32-unwind/lib");
        fs::create_dir_all(&sysroot_lib).unwrap();

        let dylib_rel = format!("bin/{}", current_dylib_filename());
        let linker_rel = format!("bin/{}", current_jsc_link_filename());
        fs::write(stage.join(&dylib_rel), b"fake-backend-dylib").unwrap();
        fs::write(stage.join(&linker_rel), b"fake-jsc-link").unwrap();
        fs::write(stage.join("targets/mires32-unwind.json"), b"{}").unwrap();
        fs::write(sysroot_lib.join("libcore-1.rlib"), b"core").unwrap();
        fs::write(sysroot_lib.join("libcompiler_builtins-1.rlib"), b"cb").unwrap();
        fs::write(sysroot_lib.join("liballoc-1.rlib"), b"alloc").unwrap();
        fs::write(sysroot_lib.join("libstd-1.rlib"), b"std").unwrap();

        let dylib_sha256 = sha256_file(&stage.join(&dylib_rel)).unwrap();
        let mut prebuilt = BTreeMap::new();
        prebuilt.insert("dev:unwind".to_owned(), "sysroots/deadbeef".to_owned());

        let manifest = ToolchainManifest {
            schema: MANIFEST_SCHEMA_VERSION,
            version: version.to_owned(),
            channel: DEFAULT_PINNED_CHANNEL.to_owned(),
            host: current_host_triple(),
            dylib: dylib_rel,
            jsc_link: linker_rel,
            dylib_sha256,
            target_spec: "targets/mires32-unwind.json".to_owned(),
            prebuilt_sysroots: prebuilt,
            has_sysroot_sources: false,
        };
        fs::write(
            stage.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let archive_path = work_dir.join(format!("mires-toolchain-v{version}.tar.gz"));
        archive::create_tar_gz(&stage, &archive_path).unwrap();
        let archive_sha = sha256_file(&archive_path).unwrap();
        (archive_path, archive_sha)
    }

    #[test]
    fn resolves_os_cache_dirs_with_mires_home_and_xdg_precedence() {
        assert_eq!(
            resolve_mires_home_from(
                Some(PathBuf::from("/custom/home")),
                Some(PathBuf::from("/xdg")),
                Some(PathBuf::from("/home/u")),
                None,
                None,
                "linux"
            )
            .unwrap(),
            PathBuf::from("/custom/home")
        );
        assert_eq!(
            resolve_mires_home_from(
                None,
                None,
                Some(PathBuf::from("/Users/alice")),
                None,
                None,
                "macos"
            )
            .unwrap(),
            PathBuf::from("/Users/alice/Library/Caches/mires")
        );
        assert_eq!(
            resolve_mires_home_from(
                None,
                Some(PathBuf::from("/Users/alice/.cache")),
                Some(PathBuf::from("/Users/alice")),
                None,
                None,
                "macos"
            )
            .unwrap(),
            PathBuf::from("/Users/alice/.cache/mires")
        );
        assert_eq!(
            resolve_mires_home_from(
                None,
                None,
                Some(PathBuf::from("/home/bob")),
                None,
                None,
                "linux"
            )
            .unwrap(),
            PathBuf::from("/home/bob/.cache/mires")
        );
        assert_eq!(
            resolve_mires_home_from(
                None,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Eve\AppData\Local")),
                Some(PathBuf::from(r"C:\Users\Eve")),
                "windows"
            )
            .unwrap(),
            PathBuf::from(r"C:\Users\Eve\AppData\Local").join("mires")
        );
    }

    #[test]
    fn round_trips_toolchain_manifest_and_parses_sha256sums() {
        let mut prebuilt = BTreeMap::new();
        prebuilt.insert("dev:unwind".to_owned(), "sysroots/abc".to_owned());
        let manifest = ToolchainManifest {
            schema: MANIFEST_SCHEMA_VERSION,
            version: "0.1.0".to_owned(),
            channel: DEFAULT_PINNED_CHANNEL.to_owned(),
            host: "aarch64-apple-darwin".to_owned(),
            dylib: "bin/librustc_codegen_mires.dylib".to_owned(),
            jsc_link: "bin/jsc-link".to_owned(),
            dylib_sha256: "a".repeat(64),
            target_spec: "targets/mires32-unwind.json".to_owned(),
            prebuilt_sysroots: prebuilt,
            has_sysroot_sources: true,
        };
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: ToolchainManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, manifest);

        let sha_a = "0123456789abcdef".repeat(4);
        let sha_b = "fedcba9876543210".repeat(4);
        let sums = format!(
            "# checksums\n{sha_a}  mires-toolchain-v0.1.0-aarch64-apple-darwin.tar.gz\n\
             {sha_b} *dist/mires-toolchain-v0.1.0-x86_64-unknown-linux-gnu.tar.gz\n"
        );
        assert_eq!(
            parse_sha256sums(&sums, "mires-toolchain-v0.1.0-aarch64-apple-darwin.tar.gz"),
            Some(sha_a)
        );
        assert_eq!(
            parse_sha256sums(
                &sums,
                "mires-toolchain-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"
            ),
            Some(sha_b)
        );
        assert_eq!(parse_sha256sums(&sums, "missing.tar.gz"), None);
    }

    #[test]
    fn setup_from_archive_installs_lists_resolves_and_cleans_toolchain() {
        let root = unique_temp_dir("lifecycle");
        let mires_home = root.join("mires-home");
        let (archive_path, archive_sha) = create_sample_bundle_archive(&root, "0.1.0");

        // Bad checksum is strictly rejected before extraction.
        let bad_res = setup_toolchain_into(
            &SetupCliOptions {
                from_archive: Some(archive_path.clone()),
                sha256: Some("0".repeat(64)),
                no_rustup: true,
                ..Default::default()
            },
            &mires_home,
        );
        assert!(bad_res.is_err());
        assert!(!mires_home.join("toolchains/v0.1.0").exists());

        // Valid checksum installs cleanly.
        let installed = setup_toolchain_into(
            &SetupCliOptions {
                from_archive: Some(archive_path),
                sha256: Some(archive_sha),
                no_rustup: true,
                ..Default::default()
            },
            &mires_home,
        )
        .unwrap();
        assert_eq!(installed, mires_home.join("toolchains/v0.1.0"));
        assert!(installed.join("manifest.json").is_file());

        // Resolution with `allow_dev_fallback: false` finds the managed bundle with zero checkout!
        let resolved = resolve_toolchain_source_with(&ResolutionEnv {
            cli_override: None,
            env_root: None,
            current_exe: None,
            mires_home: Some(&mires_home),
            version: "0.1.0",
            allow_dev_fallback: false,
        })
        .unwrap();
        assert!(matches!(
            resolved,
            ToolchainSource::InstalledBundle { ref manifest, .. } if manifest.version == "0.1.0"
        ));

        // `toolchain list` shows the installed version with active marker.
        let listed = format_toolchain_list(&mires_home, "0.1.0").unwrap();
        assert!(listed.contains("v0.1.0 *"), "unexpected list: {listed}");
        assert!(listed.contains("dev:unwind"), "unexpected list: {listed}");

        // `toolchain clean --version 0.1.0` removes it cleanly.
        let removed = clean_toolchains_in(
            &mires_home,
            &ToolchainCleanOptions {
                all: false,
                version: Some("0.1.0".to_owned()),
            },
        )
        .unwrap();
        assert_eq!(removed, vec![mires_home.join("toolchains/v0.1.0")]);
        assert!(!mires_home.join("toolchains/v0.1.0").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_pinned_channel_matches_workspace_rust_toolchain_toml() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let toml_path = repo_root.join("rust-toolchain.toml");
        if !toml_path.is_file() {
            return;
        }
        assert_eq!(read_pinned_channel(repo_root), DEFAULT_PINNED_CHANNEL);
    }
}
