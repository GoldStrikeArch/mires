use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use rcgjs_object::opts::{Modules as ModuleFormat, Profile, RuntimeMode};

use crate::{
    config::module_format_str, scrub, BuildError, PanicMode, Result, Toolchain, TOOLCHAIN_ROOT_ENV,
};

/// Default nested Cargo target directory inside `out_dir`, isolated from any outer
/// workspace `target/` lock during `build.rs` execution.
const DEFAULT_TARGET_SUBDIR: &str = "mires-target";

/// Select a Mires [`Profile`] from Cargo's `PROFILE` build-script environment variable.
///
/// Maps `"release"` to [`Profile::ReleaseBalanced`] and any other value (including unset)
/// to [`Profile::Dev`].
#[must_use]
pub fn profile_from_cargo() -> Profile {
    profile_from_env(env::var("PROFILE").ok().as_deref())
}

fn profile_from_env(profile: Option<&str>) -> Profile {
    match profile {
        Some("release") => Profile::ReleaseBalanced,
        _ => Profile::Dev,
    }
}

/// Artifacts emitted by [`Crate::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateArtifacts {
    /// Primary compiled JavaScript program (`.mjs` for ESM, `.js` for script).
    pub program: PathBuf,
    /// Source map emitted beside `program`, when `source_map` is enabled.
    pub source_map: Option<PathBuf>,
    /// TypeScript declarations emitted beside `program`, when `dts` is enabled.
    pub dts: Option<PathBuf>,
    /// External runtime shim emitted beside `program`, when `runtime` is [`RuntimeMode::External`].
    pub runtime: Option<PathBuf>,
    /// Generated `package.json`, when `npm` packaging is enabled.
    pub package_json: Option<PathBuf>,
    /// Directory containing all emitted artifacts.
    pub out_dir: PathBuf,
}

/// Ergonomic builder for compiling a Cargo crate (with its own `Cargo.toml`, `std`,
/// `alloc`, and crates.io dependencies) to JavaScript from a host `build.rs`.
///
/// Defaults:
/// * `format`: [`ModuleFormat::Esm`]
/// * `runtime`: [`RuntimeMode::Inline`] (zero-dependency self-contained bundle)
/// * `dts`: `true`
/// * `source_map`: `true`
/// * `profile`: [`profile_from_cargo()`] (`ReleaseBalanced` in release builds, `Dev` otherwise)
/// * `out_dir`: `$OUT_DIR`
/// * `target_dir`: `<out_dir>/mires-target` (isolated from the outer Cargo workspace lock)
///
/// ```rust,no_run
/// let artifacts = mires_build::Crate::new("../client")
///     .build()
///     .unwrap();
/// println!("built {}", artifacts.program.display());
/// ```
#[derive(Debug, Clone)]
pub struct Crate {
    manifest_path: PathBuf,
    crate_dir: PathBuf,
    package: Option<String>,
    profile: Option<Profile>,
    format: ModuleFormat,
    runtime: RuntimeMode,
    dts: bool,
    source_map: bool,
    npm: Option<bool>,
    panic_mode: Option<PanicMode>,
    out_dir: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    toolchain_root: Option<PathBuf>,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
}

impl Crate {
    /// Create a builder targeting a Cargo crate directory or direct `Cargo.toml` path.
    ///
    /// If `path` ends with `Cargo.toml` (or points to an existing file), it is used
    /// directly as the manifest path; otherwise `<path>/Cargo.toml` is resolved.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let input = path.into();
        let (manifest_path, crate_dir) = resolve_manifest_and_dir(&input);
        Self {
            manifest_path,
            crate_dir,
            package: None,
            profile: None,
            format: ModuleFormat::Esm,
            runtime: RuntimeMode::Inline,
            dts: true,
            source_map: true,
            npm: None,
            panic_mode: None,
            out_dir: None,
            target_dir: None,
            toolchain_root: None,
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
        }
    }

    /// Select a specific Cargo package name within a workspace (`--package`).
    #[must_use]
    pub fn package(mut self, name: impl Into<String>) -> Self {
        self.package = Some(name.into());
        self
    }

    /// Override the compiler profile (`--profile`). Defaults to [`profile_from_cargo()`].
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Select the output JavaScript module format (`--format`). Defaults to [`ModuleFormat::Esm`].
    #[must_use]
    pub fn format(mut self, format: ModuleFormat) -> Self {
        self.format = format;
        self
    }

    /// Select how the runtime shim is delivered (`--runtime`). Defaults to [`RuntimeMode::Inline`].
    #[must_use]
    pub fn runtime(mut self, runtime: RuntimeMode) -> Self {
        self.runtime = runtime;
        self
    }

    /// Control TypeScript `.d.ts` declaration emission (`--dts` / `--no-dts`). Defaults to `true`.
    #[must_use]
    pub fn dts(mut self, on: bool) -> Self {
        self.dts = on;
        self
    }

    /// Control `.map` source map emission (`--source-map` / `--no-source-map`). Defaults to `true`.
    #[must_use]
    pub fn source_map(mut self, on: bool) -> Self {
        self.source_map = on;
        self
    }

    /// Control npm `package.json` generation (`--npm` / `--no-npm`).
    #[must_use]
    pub fn npm(mut self, on: bool) -> Self {
        self.npm = Some(on);
        self
    }

    /// Select the target panic strategy (`--panic unwind|abort`).
    #[must_use]
    pub fn panic_mode(mut self, mode: PanicMode) -> Self {
        self.panic_mode = Some(mode);
        self
    }

    /// Override the artifact output directory (`--out-dir`). Defaults to `$OUT_DIR`.
    #[must_use]
    pub fn out_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(dir.into());
        self
    }

    /// Override Cargo's `--target-dir` for the nested `cargo-mires` build.
    ///
    /// Defaults to `<out_dir>/mires-target` so invoking `cargo-mires` inside a host
    /// `build.rs` never contends on the outer workspace's `target/` directory lock.
    #[must_use]
    pub fn target_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.target_dir = Some(dir.into());
        self
    }

    /// Override the Mires toolchain checkout root (`--mires-root`).
    #[must_use]
    pub fn toolchain_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.toolchain_root = Some(root.into());
        self
    }

    /// Activate a Cargo feature (`--features`). Calls accumulate in order.
    #[must_use]
    pub fn feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Activate all available Cargo features (`--all-features`).
    #[must_use]
    pub fn all_features(mut self, on: bool) -> Self {
        self.all_features = on;
        self
    }

    /// Disable default Cargo features (`--no-default-features`).
    #[must_use]
    pub fn no_default_features(mut self, on: bool) -> Self {
        self.no_default_features = on;
        self
    }

    /// Compile the Cargo crate via `cargo-mires build`, emit Cargo rerun directives,
    /// and return paths to the emitted artifacts.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `OUT_DIR` is unset (and [`out_dir`](Self::out_dir) was not called),
    /// if the client `Cargo.toml` is missing, if `cargo-mires` cannot be located, if
    /// `cargo-mires build` fails, or if no primary JavaScript artifact was produced.
    pub fn build(self) -> Result<CrateArtifacts> {
        let out_dir = self
            .out_dir
            .clone()
            .or_else(|| env::var_os("OUT_DIR").map(PathBuf::from))
            .ok_or(BuildError::NoOutDir)?;

        let toolchain_root = resolve_toolchain_root(self.toolchain_root.as_deref());

        // Emit rerun directives before any fallible check so Cargo knows when to retry.
        for directive in self.rerun_directives(toolchain_root.as_deref()) {
            println!("{directive}");
        }

        if !self.manifest_path.is_file() {
            return Err(BuildError::ManifestMissing {
                path: self.manifest_path,
            });
        }

        let cargo_mires = locate_cargo_mires(toolchain_root.as_deref())?;
        let target_dir = self.resolved_target_dir(&out_dir);
        let profile = self.profile.unwrap_or_else(profile_from_cargo);
        let args = self.build_args(&out_dir, &target_dir, profile, toolchain_root.as_deref());

        let mut command = Command::new(&cargo_mires);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub(&mut command);

        let output = command.output().map_err(|source| BuildError::Io {
            path: cargo_mires,
            source,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            eprint!("{stderr}");
            println!(
                "cargo::error=mires-build: cargo-mires failed building {}",
                self.manifest_path.display()
            );
            return Err(BuildError::CargoMires {
                status: output.status,
                stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.discover_artifacts(&out_dir, &stdout)
    }

    /// Resolve the Cargo target directory used by `cargo-mires`, defaulting to an
    /// isolated directory inside `out_dir`.
    fn resolved_target_dir(&self, out_dir: &Path) -> PathBuf {
        self.target_dir
            .clone()
            .unwrap_or_else(|| out_dir.join(DEFAULT_TARGET_SUBDIR))
    }

    /// Build the exact CLI arguments passed to `cargo-mires`.
    fn build_args(
        &self,
        out_dir: &Path,
        target_dir: &Path,
        profile: Profile,
        toolchain_root: Option<&Path>,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            OsString::from("build"),
            OsString::from("--manifest-path"),
            self.manifest_path.as_os_str().to_os_string(),
        ];

        if let Some(pkg) = &self.package {
            args.push(OsString::from("--package"));
            args.push(OsString::from(pkg));
        }

        args.push(OsString::from("--profile"));
        args.push(OsString::from(profile.as_str()));

        args.push(OsString::from("--format"));
        args.push(OsString::from(module_format_str(self.format)));

        args.push(OsString::from("--runtime"));
        args.push(OsString::from(self.runtime.as_str()));

        args.push(OsString::from(if self.dts { "--dts" } else { "--no-dts" }));
        args.push(OsString::from(if self.source_map {
            "--source-map"
        } else {
            "--no-source-map"
        }));

        if let Some(npm) = self.npm {
            args.push(OsString::from(if npm { "--npm" } else { "--no-npm" }));
        }

        if let Some(panic) = self.panic_mode {
            args.push(OsString::from("--panic"));
            args.push(OsString::from(panic.as_str()));
        }

        args.push(OsString::from("--out-dir"));
        args.push(out_dir.as_os_str().to_os_string());

        args.push(OsString::from("--target-dir"));
        args.push(target_dir.as_os_str().to_os_string());

        if let Some(root) = toolchain_root {
            args.push(OsString::from("--mires-root"));
            args.push(root.as_os_str().to_os_string());
        }

        if self.no_default_features {
            args.push(OsString::from("--no-default-features"));
        }
        if self.all_features {
            args.push(OsString::from("--all-features"));
        }
        for feature in &self.features {
            args.push(OsString::from("--features"));
            args.push(OsString::from(feature));
        }

        args
    }

    /// Collect all `cargo:rerun-if-*` directives for this crate build.
    fn rerun_directives(&self, toolchain_root: Option<&Path>) -> Vec<String> {
        let mut directives = vec![
            format!("cargo:rerun-if-env-changed={TOOLCHAIN_ROOT_ENV}"),
            "cargo:rerun-if-env-changed=PROFILE".to_owned(),
            format!("cargo:rerun-if-changed={}", self.manifest_path.display()),
        ];

        let src_dir = self.crate_dir.join("src");
        if src_dir.is_dir() {
            directives.push(format!("cargo:rerun-if-changed={}", src_dir.display()));
            let mut rs_files = Vec::new();
            collect_rs_paths(&src_dir, &mut rs_files);
            rs_files.sort();
            for path in rs_files {
                directives.push(format!("cargo:rerun-if-changed={}", path.display()));
            }
        }

        if let Some(root) = toolchain_root {
            if let Ok(toolchain) = Toolchain::new(root) {
                for bin in [toolchain.dylib(), toolchain.linker()] {
                    if bin.is_file() {
                        directives.push(format!("cargo:rerun-if-changed={}", bin.display()));
                    }
                }
            }
        }

        directives
    }

    /// Locate the emitted artifacts in `out_dir` after `cargo-mires build` succeeds.
    fn discover_artifacts(&self, out_dir: &Path, stdout: &str) -> Result<CrateArtifacts> {
        let expected_ext = format_extension(self.format);
        let reported = parse_reported_paths(stdout);

        let program = reported
            .iter()
            .find(|path| {
                path.is_file()
                    && path.extension().and_then(OsStr::to_str) == Some(expected_ext)
                    && !is_runtime_shim_name(path)
            })
            .cloned()
            .or_else(|| find_program_in_dir(out_dir, expected_ext, self.package_stem().as_deref()))
            .ok_or_else(|| BuildError::MissingOutput {
                path: out_dir.join(format!("*.{expected_ext}")),
            })?;

        let source_map = self.source_map.then(|| append_suffix(&program, ".map"));
        let source_map = source_map.filter(|path| path.is_file());

        let dts = self.dts.then(|| {
            let primary = if self.format == ModuleFormat::Esm {
                program.with_extension("d.mts")
            } else {
                program.with_extension("d.ts")
            };
            if primary.is_file() {
                primary
            } else {
                program.with_extension("d.ts")
            }
        });
        let dts = dts.filter(|path| path.is_file());

        let runtime = if self.runtime == RuntimeMode::External {
            find_runtime_artifact(out_dir, &program, expected_ext, &reported)
        } else {
            None
        };

        let package_json = if self.npm == Some(false) {
            None
        } else {
            let candidate = out_dir.join("package.json");
            candidate.is_file().then_some(candidate)
        };

        Ok(CrateArtifacts {
            program,
            source_map,
            dts,
            runtime,
            package_json,
            out_dir: out_dir.to_path_buf(),
        })
    }

    fn package_stem(&self) -> Option<String> {
        if let Some(pkg) = &self.package {
            return Some(pkg.clone());
        }
        parse_manifest_package_name(&self.manifest_path)
    }
}

fn resolve_manifest_and_dir(path: &Path) -> (PathBuf, PathBuf) {
    let is_manifest_file = path.file_name() == Some(OsStr::new("Cargo.toml")) || path.is_file();
    if is_manifest_file {
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        (path.to_path_buf(), dir)
    } else {
        (path.join("Cargo.toml"), path.to_path_buf())
    }
}

fn resolve_toolchain_root(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(root) = configured {
        return Some(root.to_path_buf());
    }
    if let Some(env_root) =
        env::var_os(TOOLCHAIN_ROOT_ENV).or_else(|| env::var_os("JSC_SPIKE_ROOT"))
    {
        return Some(PathBuf::from(env_root));
    }
    let checkout = Toolchain::checkout_root();
    is_mires_root(&checkout).then_some(checkout)
}

fn is_mires_root(dir: &Path) -> bool {
    dir.join("rust-toolchain.toml").is_file()
        && dir.join("scripts").join("build_sysroot.sh").is_file()
}

fn cargo_mires_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "cargo-mires.exe"
    } else {
        "cargo-mires"
    }
}

fn locate_cargo_mires(toolchain_root: Option<&Path>) -> Result<PathBuf> {
    locate_cargo_mires_with_path(toolchain_root, env::var_os("PATH").as_deref())
}

fn locate_cargo_mires_with_path(
    toolchain_root: Option<&Path>,
    path_var: Option<&OsStr>,
) -> Result<PathBuf> {
    let bin = cargo_mires_bin_name();
    let mut searched = Vec::new();

    if let Some(root) = toolchain_root {
        for profile_dir in ["release", "debug"] {
            let candidate = root.join("target").join(profile_dir).join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
            searched.push(candidate);
        }
    }

    if let Some(paths) = path_var {
        for dir in env::split_paths(paths) {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    searched.push(PathBuf::from(format!("$PATH/{bin}")));

    Err(BuildError::CargoMiresMissing { searched })
}

fn collect_rs_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect_rs_paths(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn format_extension(format: ModuleFormat) -> &'static str {
    match format {
        ModuleFormat::Esm => "mjs",
        ModuleFormat::Script => "js",
    }
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn parse_reported_paths(stdout: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let Some((path_part, size_part)) = rest.rsplit_once("  ") else {
            continue;
        };
        if !size_part.trim().ends_with("KB") {
            continue;
        }
        let candidate = path_part.trim();
        if !candidate.is_empty() {
            paths.push(PathBuf::from(candidate));
        }
    }
    paths
}

fn is_runtime_shim_name(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some("shim.mjs" | "shim.js")
    )
}

fn find_program_in_dir(out_dir: &Path, ext: &str, package_hint: Option<&str>) -> Option<PathBuf> {
    if let Some(pkg) = package_hint {
        for stem in [pkg.replace('-', "_"), pkg.to_owned()] {
            let candidate = out_dir.join(format!("{stem}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let mut candidates: Vec<PathBuf> = fs::read_dir(out_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(OsStr::to_str) == Some(ext)
                && !is_runtime_shim_name(path)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn find_runtime_artifact(
    out_dir: &Path,
    program: &Path,
    expected_ext: &str,
    reported: &[PathBuf],
) -> Option<PathBuf> {
    // Prefer an explicitly reported runtime file from cargo-mires stdout (handles custom shim_module).
    if let Some(path) = reported.iter().find(|path| {
        path.is_file()
            && *path != program
            && matches!(path.extension().and_then(OsStr::to_str), Some("mjs" | "js"))
    }) {
        return Some(path.clone());
    }

    let primary = out_dir.join(format!("shim.{expected_ext}"));
    if primary.is_file() {
        return Some(primary);
    }
    let secondary = out_dir.join(if expected_ext == "mjs" {
        "shim.js"
    } else {
        "shim.mjs"
    });
    secondary.is_file().then_some(secondary)
}

fn parse_manifest_package_name(manifest_path: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest_path).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "name" {
            let name = value.trim().trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TempDir;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn new_accepts_crate_directory_or_direct_manifest_path() {
        let from_dir = Crate::new("../client");
        assert_eq!(
            from_dir.manifest_path,
            PathBuf::from("../client/Cargo.toml")
        );
        assert_eq!(from_dir.crate_dir, PathBuf::from("../client"));

        let from_manifest = Crate::new("../client/Cargo.toml");
        assert_eq!(
            from_manifest.manifest_path,
            PathBuf::from("../client/Cargo.toml")
        );
        assert_eq!(from_manifest.crate_dir, PathBuf::from("../client"));

        let bare_manifest = Crate::new("Cargo.toml");
        assert_eq!(bare_manifest.manifest_path, PathBuf::from("Cargo.toml"));
        assert_eq!(bare_manifest.crate_dir, PathBuf::from("."));
    }

    #[test]
    fn profile_from_env_maps_release_and_dev() {
        assert_eq!(profile_from_env(Some("release")), Profile::ReleaseBalanced);
        assert_eq!(profile_from_env(Some("debug")), Profile::Dev);
        assert_eq!(profile_from_env(Some("")), Profile::Dev);
        assert_eq!(profile_from_env(None), Profile::Dev);
    }

    #[test]
    fn default_args_use_sane_defaults_and_isolated_target_dir() {
        let builder = Crate::new("../client");
        let out_dir = Path::new("/tmp/host/target/debug/build/host-123/out");
        let target_dir = builder.resolved_target_dir(out_dir);
        assert_eq!(target_dir, out_dir.join("mires-target"));

        let args = strings(&builder.build_args(
            out_dir,
            &target_dir,
            Profile::Dev,
            Some(Path::new("/opt/mires")),
        ));
        assert_eq!(
            args,
            [
                "build",
                "--manifest-path",
                "../client/Cargo.toml",
                "--profile",
                "dev",
                "--format",
                "esm",
                "--runtime",
                "inline",
                "--dts",
                "--source-map",
                "--out-dir",
                "/tmp/host/target/debug/build/host-123/out",
                "--target-dir",
                "/tmp/host/target/debug/build/host-123/out/mires-target",
                "--mires-root",
                "/opt/mires",
            ]
        );
    }

    #[test]
    fn builder_methods_populate_all_cargo_mires_flags() {
        let builder = Crate::new("workspace/client/Cargo.toml")
            .package("client-pkg")
            .profile(Profile::ReleaseSize)
            .format(ModuleFormat::Script)
            .runtime(RuntimeMode::External)
            .dts(false)
            .source_map(false)
            .npm(true)
            .panic_mode(PanicMode::Abort)
            .out_dir("/custom/out")
            .target_dir("/custom/target")
            .toolchain_root("/custom/mires")
            .feature("feat-a")
            .feature("feat-b")
            .all_features(true)
            .no_default_features(true);

        let out_dir = builder.out_dir.clone().unwrap();
        let target_dir = builder.resolved_target_dir(&out_dir);
        assert_eq!(target_dir, PathBuf::from("/custom/target"));

        let args = strings(&builder.build_args(
            &out_dir,
            &target_dir,
            builder.profile.unwrap(),
            builder.toolchain_root.as_deref(),
        ));
        assert_eq!(
            args,
            [
                "build",
                "--manifest-path",
                "workspace/client/Cargo.toml",
                "--package",
                "client-pkg",
                "--profile",
                "release-size",
                "--format",
                "script",
                "--runtime",
                "external",
                "--no-dts",
                "--no-source-map",
                "--npm",
                "--panic",
                "abort",
                "--out-dir",
                "/custom/out",
                "--target-dir",
                "/custom/target",
                "--mires-root",
                "/custom/mires",
                "--no-default-features",
                "--all-features",
                "--features",
                "feat-a",
                "--features",
                "feat-b",
            ]
        );
    }

    #[test]
    fn locate_cargo_mires_prefers_release_then_debug_then_path() {
        let root = TempDir::new("locate-cargo-mires");
        let path_dir = TempDir::new("locate-cargo-mires-path");
        let bin = cargo_mires_bin_name();

        let err =
            locate_cargo_mires_with_path(Some(root.path()), Some(path_dir.path().as_os_str()))
                .unwrap_err();
        assert!(matches!(err, BuildError::CargoMiresMissing { .. }), "{err}");

        // Found on PATH when neither release nor debug exists.
        let on_path = path_dir.path().join(bin);
        fs::write(&on_path, "").unwrap();
        assert_eq!(
            locate_cargo_mires_with_path(Some(root.path()), Some(path_dir.path().as_os_str()))
                .unwrap(),
            on_path
        );

        // Debug binary under toolchain root beats PATH.
        let debug_bin = root.path().join("target/debug").join(bin);
        fs::create_dir_all(debug_bin.parent().unwrap()).unwrap();
        fs::write(&debug_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_with_path(Some(root.path()), Some(path_dir.path().as_os_str()))
                .unwrap(),
            debug_bin
        );

        // Release binary under toolchain root beats debug.
        let release_bin = root.path().join("target/release").join(bin);
        fs::create_dir_all(release_bin.parent().unwrap()).unwrap();
        fs::write(&release_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_with_path(Some(root.path()), Some(path_dir.path().as_os_str()))
                .unwrap(),
            release_bin
        );
    }

    #[test]
    fn rerun_directives_watch_env_manifest_and_src_rs_files() {
        let dir = TempDir::new("crate-rerun");
        fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        fs::write(dir.path().join("src/nested/mod.rs"), "").unwrap();
        fs::write(dir.path().join("src/notes.txt"), "").unwrap();

        let directives = Crate::new(dir.path()).rerun_directives(None);
        assert!(directives.contains(&"cargo:rerun-if-env-changed=MIRES_ROOT".to_owned()));
        assert!(directives.contains(&"cargo:rerun-if-env-changed=PROFILE".to_owned()));
        assert!(directives.contains(&format!(
            "cargo:rerun-if-changed={}",
            dir.path().join("Cargo.toml").display()
        )));
        assert!(directives.contains(&format!(
            "cargo:rerun-if-changed={}",
            dir.path().join("src").display()
        )));
        assert!(directives.contains(&format!(
            "cargo:rerun-if-changed={}",
            dir.path().join("src/lib.rs").display()
        )));
        assert!(directives.contains(&format!(
            "cargo:rerun-if-changed={}",
            dir.path().join("src/nested/mod.rs").display()
        )));
        assert!(!directives.iter().any(|line| line.contains("notes.txt")));
    }

    #[test]
    fn discover_artifacts_finds_program_and_sidecars() {
        let dir = TempDir::new("crate-discover");
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-client\"\n",
        )
        .unwrap();

        let program = out.join("my_client.mjs");
        let map = out.join("my_client.mjs.map");
        let dts = out.join("my_client.d.ts");
        let shim = out.join("shim.mjs");
        let pkg = out.join("package.json");
        for path in [&program, &map, &dts, &shim, &pkg] {
            fs::write(path, "// artifact\n").unwrap();
        }

        let artifacts = Crate::new(dir.path())
            .runtime(RuntimeMode::External)
            .npm(true)
            .discover_artifacts(&out, "")
            .unwrap();
        assert_eq!(
            artifacts,
            CrateArtifacts {
                program,
                source_map: Some(map),
                dts: Some(dts),
                runtime: Some(shim),
                package_json: Some(pkg),
                out_dir: out,
            }
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn build_invokes_cargo_mires_with_isolated_target_dir_and_collects_outputs() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("crate-build-e2e");
        let client_dir = dir.path().join("client");
        fs::create_dir_all(client_dir.join("src")).unwrap();
        fs::write(
            client_dir.join("Cargo.toml"),
            "[package]\nname = \"demo-client\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(client_dir.join("src/lib.rs"), "").unwrap();

        let toolchain_dir = dir.path().join("mires");
        let release_dir = toolchain_dir.join("target/release");
        fs::create_dir_all(&release_dir).unwrap();
        let out_dir = dir.path().join("out");
        let recorded_args = dir.path().join("recorded-args.txt");
        let fake_cargo_mires = release_dir.join("cargo-mires");
        fs::write(
            &fake_cargo_mires,
            format!(
                concat!(
                    "#!/usr/bin/env bash\n",
                    "set -euo pipefail\n",
                    "printf '%s\\n' \"$@\" >'{}'\n",
                    "out='{}'\n",
                    "mkdir -p \"$out\"\n",
                    "printf 'export function hi() {{}}\\n' >\"$out/demo_client.mjs\"\n",
                    "printf '{{\"version\":3}}' >\"$out/demo_client.mjs.map\"\n",
                    "printf 'export function hi(): void;\\n' >\"$out/demo_client.d.ts\"\n",
                    "printf 'Built `demo-client` (esm, inline runtime, profile=dev) in 0.01s:\\n'\n",
                    "printf '  %s/demo_client.mjs  0.03 KB\\n' \"$out\"\n",
                    "printf '  %s/demo_client.mjs.map  0.01 KB\\n' \"$out\"\n",
                    "printf '  %s/demo_client.d.ts  0.03 KB\\n' \"$out\"\n",
                ),
                recorded_args.display(),
                out_dir.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&fake_cargo_mires, fs::Permissions::from_mode(0o755)).unwrap();

        let artifacts = Crate::new(&client_dir)
            .profile(Profile::Dev)
            .out_dir(&out_dir)
            .toolchain_root(&toolchain_dir)
            .build()
            .unwrap();

        assert_eq!(artifacts.program, out_dir.join("demo_client.mjs"));
        assert_eq!(
            artifacts.source_map,
            Some(out_dir.join("demo_client.mjs.map"))
        );
        assert_eq!(artifacts.dts, Some(out_dir.join("demo_client.d.ts")));
        assert_eq!(artifacts.runtime, None);
        assert_eq!(artifacts.package_json, None);
        assert_eq!(artifacts.out_dir, out_dir);

        let recorded: Vec<String> = fs::read_to_string(&recorded_args)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let target_idx = recorded
            .iter()
            .position(|arg| arg == "--target-dir")
            .expect("--target-dir must always be passed");
        assert_eq!(
            PathBuf::from(&recorded[target_idx + 1]),
            out_dir.join("mires-target")
        );
    }
}
