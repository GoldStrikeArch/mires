use std::{
    collections::BTreeSet,
    env,
    ffi::{OsStr, OsString},
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    checkout_root, dylib_filename, module_format_str, scrub, BuildError, DiscoverChunks,
    ModuleFormat, PanicMode, Profile, Result, RuntimeMode, AUTO_INSTALL_ENV, TOOLCHAIN_ROOT_ENV,
};

/// Default nested Cargo target directory inside `out_dir`, isolated from any outer
/// workspace `target/` lock during `build.rs` execution.
const DEFAULT_TARGET_SUBDIR: &str = "mires-target";

/// Expected JSON schema version produced by `cargo-mires build --message-format=json`.
const CARGO_MIRES_JSON_SCHEMA: u32 = 1;

/// Returns `true` when the enclosing Cargo build script is running for a Mires JS target build.
///
/// Always emits `cargo::rustc-check-cfg=cfg(mires)` so `#[cfg(mires)]` in the crate produces
/// zero `unexpected_cfgs` warnings. Call this at the top of `build.rs` in single-crate fullstack
/// packages (before [`Crate::current`]) to prevent recursive `cargo-mires` invocations:
///
/// ```rust,no_run
/// if mires_build::is_target_build() {
///     return;
/// }
/// mires_build::Crate::current()
///     .emit_rust_module("client_assets.rs")
///     .build()
///     .unwrap();
/// ```
#[must_use]
pub fn is_target_build() -> bool {
    println!("cargo::rustc-check-cfg=cfg(mires)");
    is_target_build_from(env::var_os("CARGO_CFG_MIRES").as_deref())
}

fn is_target_build_from(cargo_cfg_mires: Option<&OsStr>) -> bool {
    cargo_cfg_mires.is_some()
}

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

/// Metadata and paths for one code-split chunk emitted by [`Crate::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateChunk {
    /// Chunk name (e.g., `"counter"`, `"shared"`).
    pub name: String,
    /// Deployment role (e.g., `"island"`, `"main"`, `"worker"`, `"procedure"`, `"shared"`).
    pub role: String,
    /// Published JavaScript file path inside `out_dir`.
    pub path: PathBuf,
    /// Source map emitted beside this chunk, if present.
    pub source_map: Option<PathBuf>,
    /// TypeScript declarations emitted beside this chunk, if present.
    pub dts: Option<PathBuf>,
    /// Lowercase hexadecimal SHA-256 digest of the chunk bytes.
    pub sha256: String,
    /// Size of the chunk in bytes.
    pub bytes: u64,
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
    /// External runtime shim emitted beside `program`, when `runtime` is [`RuntimeMode::External`]
    /// with a local `./` shim specifier.
    pub runtime: Option<PathBuf>,
    /// Generated `package.json`, when `npm` packaging is enabled.
    pub package_json: Option<PathBuf>,
    /// Directory containing all emitted artifacts.
    pub out_dir: PathBuf,
    /// Split chunk artifacts emitted alongside `program` (empty for unsplit builds).
    pub chunks: Vec<CrateChunk>,
    /// Path to the copied linker `.artifacts.json` deployment manifest, if present.
    pub manifest_path: Option<PathBuf>,
    /// Raw JSON contents of the linker `.artifacts.json` deployment manifest, if present.
    pub manifest_json: Option<String>,
    /// Path to the generated Rust assets module when [`Crate::emit_rust_module`] was configured.
    pub rust_module: Option<PathBuf>,
}

/// Ergonomic builder for compiling a Cargo crate (with its own `Cargo.toml`, `std`,
/// `alloc`, and crates.io dependencies) to JavaScript from a host `build.rs`.
///
/// Defaults:
/// * `format`: [`ModuleFormat::Esm`]
/// * `runtime`: [`RuntimeMode::Inline`] (or [`RuntimeMode::External`] when chunks/islands or
///   `shim_module` are configured without an explicit `.runtime(...)`)
/// * `dts`: `true`
/// * `source_map`: `true`
/// * `profile`: [`profile_from_cargo()`] (`ReleaseBalanced` in release builds, `Dev` otherwise)
/// * `out_dir`: `$OUT_DIR`
/// * `target_dir`: `<out_dir>/mires-target` (isolated from the outer Cargo workspace lock)
/// * `auto_install`: `false`
///
/// ```rust,no_run
/// let artifacts = mires_build::Crate::new("../client")
///     .release()
///     .emit_rust_module("client_assets.rs")
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
    runtime: Option<RuntimeMode>,
    dts: bool,
    source_map: bool,
    npm: Option<bool>,
    panic_mode: Option<PanicMode>,
    out_dir: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    toolchain_root: Option<PathBuf>,
    auto_install: bool,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    cfgs: Vec<String>,
    chunks: Vec<(String, String)>,
    islands: Vec<String>,
    shared_chunk: Option<String>,
    discover_chunks: Option<DiscoverChunks>,
    shim_module: Option<String>,
    rust_module: Option<String>,
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
            runtime: None,
            dts: true,
            source_map: true,
            npm: None,
            panic_mode: None,
            out_dir: None,
            target_dir: None,
            toolchain_root: None,
            auto_install: false,
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
            cfgs: Vec::new(),
            chunks: Vec::new(),
            islands: Vec::new(),
            shared_chunk: None,
            discover_chunks: None,
            shim_module: None,
            rust_module: None,
        }
    }

    /// Create a builder targeting the current Cargo package (`CARGO_MANIFEST_DIR`, falling back
    /// to `.`).
    ///
    /// Pair with [`is_target_build`] at the top of `build.rs` to avoid infinite recursion when
    /// compiling the current package to JavaScript.
    #[must_use]
    pub fn current() -> Self {
        let dir = env::var_os("CARGO_MANIFEST_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        Self::new(dir)
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

    /// Build with the default release profile ([`Profile::ReleaseBalanced`]).
    #[must_use]
    pub fn release(mut self) -> Self {
        self.profile = Some(Profile::ReleaseBalanced);
        self
    }

    /// Select the compiler profile automatically from Cargo's `PROFILE` environment variable.
    #[must_use]
    pub fn profile_from_cargo(mut self) -> Self {
        self.profile = None;
        self
    }

    /// Select the output JavaScript module format (`--format`). Defaults to [`ModuleFormat::Esm`].
    #[must_use]
    pub fn format(mut self, format: ModuleFormat) -> Self {
        self.format = format;
        self
    }

    /// Select how the runtime shim is delivered (`--runtime`). Defaults to [`RuntimeMode::Inline`]
    /// (or [`RuntimeMode::External`] when chunks/islands or [`shim_module`](Self::shim_module) are
    /// configured).
    #[must_use]
    pub fn runtime(mut self, runtime: RuntimeMode) -> Self {
        self.runtime = Some(runtime);
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

    /// Override the Mires toolchain checkout or unpacked bundle root (`--mires-root`).
    #[must_use]
    pub fn toolchain_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.toolchain_root = Some(root.into());
        self
    }

    /// Allow `cargo-mires` to automatically download a prebuilt toolchain bundle
    /// if missing (`MIRES_AUTO_INSTALL=1`). Defaults to `false`.
    #[must_use]
    pub fn auto_install(mut self, on: bool) -> Self {
        self.auto_install = on;
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

    /// Pass a target `--cfg <SPEC>` (e.g. `"topcoat_client"` or `"mode=\"csr\""`) to `cargo-mires`
    /// and emit the corresponding `cargo::rustc-check-cfg` directive. Calls accumulate in order.
    #[must_use]
    pub fn cfg(mut self, cfg: impl Into<String>) -> Self {
        self.cfgs.push(cfg.into());
        self
    }

    /// Request a named code-split chunk (`--chunk <name>:<entry>`). Calls accumulate in order.
    #[must_use]
    pub fn chunk(mut self, name: impl Into<String>, entry: impl Into<String>) -> Self {
        self.chunks.push((name.into(), entry.into()));
        self
    }

    /// Request a code-split island chunk (`--island <name>`, default entry `__island_<name>`).
    /// Calls accumulate in order.
    #[must_use]
    pub fn island(mut self, name: impl Into<String>) -> Self {
        self.islands.push(name.into());
        self
    }

    /// Set the shared chunk name holding items common to multiple chunks (`--shared-chunk`).
    #[must_use]
    pub fn shared_chunk(mut self, name: impl Into<String>) -> Self {
        self.shared_chunk = Some(name.into());
        self
    }

    /// Control automatic chunk discovery from `#[js_export(chunk = ...)]` (`--discover-chunks`).
    #[must_use]
    pub fn discover_chunks(mut self, mode: DiscoverChunks) -> Self {
        self.discover_chunks = Some(mode);
        self
    }

    /// Override the external runtime module specifier (`--shim-module`), such as `"./runtime.mjs"`
    /// or a bare specifier like `"topcoat-dom"` or `"solid-js/web"`.
    #[must_use]
    pub fn shim_module(mut self, specifier: impl Into<String>) -> Self {
        self.shim_module = Some(specifier.into());
        self
    }

    /// Emit a zero-dependency Rust assets module `<out_dir>/<file_name>` exposing `ENTRY_NAME`,
    /// `ENTRY_JS`, `ENTRY_SHA256`, `SOURCE_MAP`, `ChunkAsset`, `CHUNKS`, `FILES`, and
    /// `MANIFEST_JSON` for inclusion via `include!(concat!(env!("OUT_DIR"), "/..."))`.
    #[must_use]
    pub fn emit_rust_module(mut self, file_name: impl Into<String>) -> Self {
        self.rust_module = Some(file_name.into());
        self
    }

    /// Compile the Cargo crate via `cargo-mires build --message-format=json`, emit Cargo
    /// check-cfg and rerun directives (including transitive local path dependencies), and
    /// return the emitted artifacts.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `OUT_DIR` is unset (and [`out_dir`](Self::out_dir) was not called),
    /// if the client `Cargo.toml` is missing, if `cargo-mires` cannot be located, if
    /// `cargo-mires build` fails, if its JSON output cannot be parsed, or if generating the
    /// optional Rust assets module fails.
    pub fn build(self) -> Result<CrateArtifacts> {
        let out_dir = self
            .out_dir
            .clone()
            .or_else(|| env::var_os("OUT_DIR").map(PathBuf::from))
            .ok_or(BuildError::NoOutDir)?;

        let toolchain_root = resolve_toolchain_root(self.toolchain_root.as_deref());

        for directive in self.check_cfg_directives() {
            println!("{directive}");
        }

        // Emit baseline rerun directives before any fallible check so Cargo knows when to retry.
        let mut emitted_paths = BTreeSet::new();
        for directive in self.rerun_directives(toolchain_root.as_deref()) {
            if let Some(path) = directive.strip_prefix("cargo:rerun-if-changed=") {
                emitted_paths.insert(PathBuf::from(path));
            }
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
        let mut command = self.build_command(
            &cargo_mires,
            &out_dir,
            &target_dir,
            profile,
            toolchain_root.as_deref(),
        );

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
        let parsed = parse_cargo_mires_json(&stdout)?;

        for dep in &parsed.dep_files {
            if emitted_paths.insert(dep.clone()) {
                println!("cargo:rerun-if-changed={}", dep.display());
            }
        }

        let mut artifacts = build_artifacts_from_json(parsed)?;

        if let Some(file_name) = &self.rust_module {
            let module_path = write_rust_module(&artifacts, file_name)?;
            artifacts.rust_module = Some(module_path);
        }

        Ok(artifacts)
    }

    fn resolved_runtime(&self) -> RuntimeMode {
        self.runtime.unwrap_or_else(|| {
            if !self.chunks.is_empty() || !self.islands.is_empty() || self.shim_module.is_some() {
                RuntimeMode::External
            } else {
                RuntimeMode::Inline
            }
        })
    }

    fn check_cfg_directives(&self) -> Vec<String> {
        let mut directives = vec!["cargo::rustc-check-cfg=cfg(mires)".to_owned()];
        for spec in &self.cfgs {
            if let Some(expr) = check_cfg_expr(spec) {
                let directive = format!("cargo::rustc-check-cfg={expr}");
                if !directives.contains(&directive) {
                    directives.push(directive);
                }
            }
        }
        directives
    }

    /// Resolve the Cargo target directory used by `cargo-mires`, defaulting to an
    /// isolated directory inside `out_dir`.
    fn resolved_target_dir(&self, out_dir: &Path) -> PathBuf {
        self.target_dir
            .clone()
            .unwrap_or_else(|| out_dir.join(DEFAULT_TARGET_SUBDIR))
    }

    /// Prepare the spawned `cargo-mires` command with scrubbed Cargo env and optional
    /// `MIRES_AUTO_INSTALL=1`.
    fn build_command(
        &self,
        cargo_mires: &Path,
        out_dir: &Path,
        target_dir: &Path,
        profile: Profile,
        toolchain_root: Option<&Path>,
    ) -> Command {
        let args = self.build_args(out_dir, target_dir, profile, toolchain_root);
        let mut command = Command::new(cargo_mires);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub(&mut command);
        if self.auto_install {
            command.env(AUTO_INSTALL_ENV, "1");
        }
        command
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
        args.push(OsString::from(self.resolved_runtime().as_str()));

        if let Some(specifier) = &self.shim_module {
            args.push(OsString::from("--shim-module"));
            args.push(OsString::from(specifier));
        }

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

        for cfg in &self.cfgs {
            args.push(OsString::from("--cfg"));
            args.push(OsString::from(cfg));
        }

        for (name, entry) in &self.chunks {
            args.push(OsString::from("--chunk"));
            args.push(OsString::from(format!("{name}:{entry}")));
        }

        for name in &self.islands {
            args.push(OsString::from("--island"));
            args.push(OsString::from(name));
        }

        if let Some(shared) = &self.shared_chunk {
            args.push(OsString::from("--shared-chunk"));
            args.push(OsString::from(shared));
        }

        if let Some(mode) = self.discover_chunks {
            args.push(OsString::from("--discover-chunks"));
            args.push(OsString::from(mode.as_str()));
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

        args.push(OsString::from("--message-format"));
        args.push(OsString::from("json"));

        args
    }

    /// Collect baseline `cargo:rerun-if-*` directives for this crate build.
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
            for bin in watched_toolchain_binaries(root) {
                if bin.is_file() {
                    directives.push(format!("cargo:rerun-if-changed={}", bin.display()));
                }
            }
        }

        directives
    }
}

#[derive(Debug, Deserialize)]
struct CargoMiresBuildJson {
    schema: u32,
    out_dir: PathBuf,
    program: PathBuf,
    #[serde(default)]
    source_map: Option<PathBuf>,
    #[serde(default)]
    dts: Option<PathBuf>,
    #[serde(default)]
    runtime_file: Option<PathBuf>,
    #[serde(default)]
    package_json: Option<PathBuf>,
    #[serde(default)]
    manifest_path: Option<PathBuf>,
    #[serde(default)]
    manifest: Option<serde_json::Value>,
    #[serde(default)]
    chunks: Vec<CargoMiresChunkJson>,
    #[serde(default)]
    dep_files: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoMiresChunkJson {
    name: String,
    role: String,
    path: PathBuf,
    #[serde(default)]
    source_map: Option<PathBuf>,
    #[serde(default)]
    dts: Option<PathBuf>,
    sha256: String,
    bytes: u64,
}

fn parse_cargo_mires_json(stdout: &str) -> Result<CargoMiresBuildJson> {
    let trimmed = stdout.trim();
    let parsed: CargoMiresBuildJson = match serde_json::from_str(trimmed) {
        Ok(val) => val,
        Err(primary_err) => {
            // Fall back to scanning from the end for a JSON line in case an upstream tool
            // (e.g., auto-installer) emitted informational lines before the final JSON payload.
            trimmed
                .lines()
                .rev()
                .map(str::trim)
                .filter(|line| line.starts_with('{'))
                .find_map(|line| serde_json::from_str::<CargoMiresBuildJson>(line).ok())
                .ok_or_else(|| BuildError::InvalidCargoMiresJson {
                    detail: primary_err.to_string(),
                })?
        }
    };

    if parsed.schema != CARGO_MIRES_JSON_SCHEMA {
        return Err(BuildError::InvalidCargoMiresJson {
            detail: format!(
                "unsupported schema version {} (expected {CARGO_MIRES_JSON_SCHEMA})",
                parsed.schema
            ),
        });
    }

    Ok(parsed)
}

fn build_artifacts_from_json(parsed: CargoMiresBuildJson) -> Result<CrateArtifacts> {
    if !parsed.program.is_file() {
        return Err(BuildError::MissingOutput {
            path: parsed.program,
        });
    }

    let manifest_json = if let Some(path) = &parsed.manifest_path {
        if path.is_file() {
            Some(fs::read_to_string(path).map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?)
        } else {
            parsed
                .manifest
                .as_ref()
                .and_then(|v| serde_json::to_string_pretty(v).ok())
        }
    } else {
        parsed
            .manifest
            .as_ref()
            .and_then(|v| serde_json::to_string_pretty(v).ok())
    };

    let chunks = parsed
        .chunks
        .into_iter()
        .map(|chunk| CrateChunk {
            name: chunk.name,
            role: chunk.role,
            path: chunk.path,
            source_map: chunk.source_map,
            dts: chunk.dts,
            sha256: chunk.sha256,
            bytes: chunk.bytes,
        })
        .collect();

    Ok(CrateArtifacts {
        program: parsed.program,
        source_map: parsed.source_map,
        dts: parsed.dts,
        runtime: parsed.runtime_file,
        package_json: parsed.package_json,
        out_dir: parsed.out_dir,
        chunks,
        manifest_path: parsed.manifest_path,
        manifest_json,
        rust_module: None,
    })
}

fn write_rust_module(artifacts: &CrateArtifacts, file_name: &str) -> Result<PathBuf> {
    validate_relative_module_path(file_name)?;
    let module_path = artifacts.out_dir.join(file_name);
    if let Some(parent) = module_path.parent() {
        fs::create_dir_all(parent).map_err(|source| BuildError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let rendered = render_rust_module(artifacts, &module_path)?;
    fs::write(&module_path, rendered.as_bytes()).map_err(|source| BuildError::Io {
        path: module_path.clone(),
        source,
    })?;
    Ok(module_path)
}

fn validate_relative_module_path(file_name: &str) -> Result<()> {
    let trimmed = file_name.trim();
    let path = Path::new(trimmed);
    let valid = !trimmed.is_empty()
        && path.components().next().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !valid {
        return Err(BuildError::InvalidRustModulePath {
            path: file_name.to_owned(),
        });
    }
    Ok(())
}

fn render_rust_module(artifacts: &CrateArtifacts, module_path: &Path) -> Result<String> {
    let module_dir = module_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(&artifacts.out_dir);

    let entry_bytes = fs::read(&artifacts.program).map_err(|source| BuildError::Io {
        path: artifacts.program.clone(),
        source,
    })?;
    let entry_sha256 = format!("{:x}", Sha256::digest(&entry_bytes));
    let entry_name = relative_asset_name(&artifacts.out_dir, &artifacts.program);
    let entry_inc = include_path_for(module_dir, &artifacts.program);

    let mut out = String::new();
    out.push_str("// @generated by mires-build; do not edit.\n\n");

    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    let _ = writeln!(
        out,
        "pub const ENTRY_NAME: &str = {};",
        rust_str_lit(&entry_name)
    );
    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    let _ = writeln!(
        out,
        "pub const ENTRY_JS: &str = include_str!({});",
        rust_str_lit(&entry_inc)
    );
    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    let _ = writeln!(
        out,
        "pub const ENTRY_SHA256: &str = {};",
        rust_str_lit(&entry_sha256)
    );

    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    if let Some(map_path) = &artifacts.source_map {
        let map_inc = include_path_for(module_dir, map_path);
        let _ = writeln!(
            out,
            "pub const SOURCE_MAP: Option<&str> = Some(include_str!({}));",
            rust_str_lit(&map_inc)
        );
    } else {
        out.push_str("pub const SOURCE_MAP: Option<&str> = None;\n");
    }

    out.push_str(
        "\n#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]\n\
         #[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct ChunkAsset {\n\
             pub name: &'static str,\n\
             pub role: &'static str,\n\
             pub file_name: &'static str,\n\
             pub sha256: &'static str,\n\
             pub bytes: &'static [u8],\n\
             pub source_map: Option<&'static [u8]>,\n\
         }\n\n",
    );

    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    out.push_str("pub const CHUNKS: &[ChunkAsset] = &[\n");
    for chunk in &artifacts.chunks {
        let chunk_file = relative_asset_name(&artifacts.out_dir, &chunk.path);
        let chunk_inc = include_path_for(module_dir, &chunk.path);
        let map_expr = match &chunk.source_map {
            Some(map) => format!(
                "Some(include_bytes!({}))",
                rust_str_lit(&include_path_for(module_dir, map))
            ),
            None => "None".to_owned(),
        };
        let _ = writeln!(
            out,
            "    ChunkAsset {{\n\
             \x20       name: {},\n\
             \x20       role: {},\n\
             \x20       file_name: {},\n\
             \x20       sha256: {},\n\
             \x20       bytes: include_bytes!({}),\n\
             \x20       source_map: {map_expr},\n\
             \x20   }},",
            rust_str_lit(&chunk.name),
            rust_str_lit(&chunk.role),
            rust_str_lit(&chunk_file),
            rust_str_lit(&chunk.sha256),
            rust_str_lit(&chunk_inc),
        );
    }
    out.push_str("];\n\n");

    let mut files: Vec<(String, String)> = Vec::new();
    let mut seen_files = BTreeSet::new();
    let mut push_file = |path: &Path| {
        let rel = relative_asset_name(&artifacts.out_dir, path);
        if seen_files.insert(rel.clone()) {
            let inc = include_path_for(module_dir, path);
            files.push((rel, inc));
        }
    };

    push_file(&artifacts.program);
    if let Some(map_path) = &artifacts.source_map {
        push_file(map_path);
    }
    if let Some(runtime_path) = &artifacts.runtime {
        push_file(runtime_path);
    }
    for chunk in &artifacts.chunks {
        push_file(&chunk.path);
        if let Some(map_path) = &chunk.source_map {
            push_file(map_path);
        }
    }

    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    out.push_str("pub const FILES: &[(&str, &[u8])] = &[\n");
    for (rel, inc) in &files {
        let _ = writeln!(
            out,
            "    ({}, include_bytes!({})),",
            rust_str_lit(rel),
            rust_str_lit(inc)
        );
    }
    out.push_str("];\n\n");

    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"generated asset constants may be used selectively\")]"
    );
    if let Some(manifest_path) = &artifacts.manifest_path {
        if manifest_path.is_file() {
            let manifest_inc = include_path_for(module_dir, manifest_path);
            let _ = writeln!(
                out,
                "pub const MANIFEST_JSON: Option<&str> = Some(include_str!({}));",
                rust_str_lit(&manifest_inc)
            );
        } else if let Some(json) = &artifacts.manifest_json {
            let _ = writeln!(
                out,
                "pub const MANIFEST_JSON: Option<&str> = Some({});",
                rust_str_lit(json)
            );
        } else {
            out.push_str("pub const MANIFEST_JSON: Option<&str> = None;\n");
        }
    } else if let Some(json) = &artifacts.manifest_json {
        let _ = writeln!(
            out,
            "pub const MANIFEST_JSON: Option<&str> = Some({});",
            rust_str_lit(json)
        );
    } else {
        out.push_str("pub const MANIFEST_JSON: Option<&str> = None;\n");
    }

    Ok(out)
}

fn relative_asset_name(out_dir: &Path, path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(out_dir) {
        return to_forward_slash(rel);
    }
    path.file_name()
        .and_then(OsStr::to_str)
        .map_or_else(|| to_forward_slash(path), str::to_owned)
}

fn include_path_for(module_dir: &Path, asset_path: &Path) -> String {
    if let Ok(rel) = asset_path.strip_prefix(module_dir) {
        return to_forward_slash(rel);
    }
    let resolved = fs::canonicalize(asset_path).unwrap_or_else(|_| asset_path.to_path_buf());
    to_forward_slash(&resolved)
}

fn to_forward_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
        .replace("//", "/")
}

fn rust_str_lit(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn is_valid_cfg_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn check_cfg_expr(spec: &str) -> Option<String> {
    let trimmed = spec.trim();
    if let Some((key, _)) = trimmed.split_once('=') {
        let key = key.trim();
        is_valid_cfg_ident(key).then(|| format!("cfg({key},values(any()))"))
    } else {
        is_valid_cfg_ident(trimmed).then(|| format!("cfg({trimmed})"))
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
    resolve_toolchain_root_from(
        configured,
        env::var_os(TOOLCHAIN_ROOT_ENV).as_deref(),
        env::var_os("JSC_SPIKE_ROOT").as_deref(),
        &checkout_root(),
    )
}

fn resolve_toolchain_root_from(
    configured: Option<&Path>,
    mires_root_env: Option<&OsStr>,
    legacy_root_env: Option<&OsStr>,
    checkout: &Path,
) -> Option<PathBuf> {
    if let Some(root) = configured {
        return Some(root.to_path_buf());
    }
    if let Some(env_root) = mires_root_env
        .filter(|value| !value.is_empty())
        .or_else(|| legacy_root_env.filter(|value| !value.is_empty()))
    {
        return Some(PathBuf::from(env_root));
    }
    is_mires_root(checkout).then(|| checkout.to_path_buf())
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

fn jsc_link_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "jsc-link.exe"
    } else {
        "jsc-link"
    }
}

fn watched_toolchain_binaries(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dylib) = dylib_filename() {
        candidates.push(root.join("target").join("release").join(dylib));
        candidates.push(root.join("lib").join(dylib));
        candidates.push(root.join("bin").join(dylib));
    }
    let linker = jsc_link_bin_name();
    candidates.push(root.join("target").join("release").join(linker));
    candidates.push(root.join("bin").join(linker));
    candidates
}

fn default_cargo_bin_dir() -> Option<PathBuf> {
    cargo_bin_dir_from_env(
        env::var_os("CARGO_HOME").as_deref(),
        env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .or_else(|| env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
            .as_deref(),
    )
}

fn cargo_bin_dir_from_env(cargo_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(cargo_home) = cargo_home.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(cargo_home).join("bin"));
    }
    home.filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".cargo").join("bin"))
}

fn locate_cargo_mires(toolchain_root: Option<&Path>) -> Result<PathBuf> {
    let cargo_bin = default_cargo_bin_dir();
    locate_cargo_mires_in(
        toolchain_root,
        cargo_bin.as_deref(),
        env::var_os("PATH").as_deref(),
    )
}

fn locate_cargo_mires_in(
    toolchain_root: Option<&Path>,
    cargo_bin_dir: Option<&Path>,
    path_var: Option<&OsStr>,
) -> Result<PathBuf> {
    let bin = cargo_mires_bin_name();
    let mut searched = Vec::new();

    if let Some(root) = toolchain_root {
        let candidates = [
            root.join("target").join("release").join(bin),
            root.join("target").join("debug").join(bin),
            root.join("bin").join(bin),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Ok(candidate);
            }
            searched.push(candidate);
        }
    }

    if let Some(cargo_bin) = cargo_bin_dir {
        let candidate = cargo_bin.join(bin);
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate);
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
    fn is_target_build_detects_cargo_cfg_mires() {
        assert!(!is_target_build_from(None));
        assert!(is_target_build_from(Some(OsStr::new(""))));
        assert!(is_target_build_from(Some(OsStr::new("1"))));
    }

    #[test]
    fn new_and_current_resolve_manifest_and_release_profile_helpers() {
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

        let current = Crate::current();
        assert_eq!(
            current.manifest_path.file_name(),
            Some(OsStr::new("Cargo.toml"))
        );

        let released = Crate::new("../client").release();
        assert_eq!(released.profile, Some(Profile::ReleaseBalanced));
        let auto_profile = released.profile_from_cargo();
        assert_eq!(auto_profile.profile, None);
    }

    #[test]
    fn profile_from_env_maps_release_and_dev() {
        assert_eq!(profile_from_env(Some("release")), Profile::ReleaseBalanced);
        assert_eq!(profile_from_env(Some("debug")), Profile::Dev);
        assert_eq!(profile_from_env(Some("")), Profile::Dev);
        assert_eq!(profile_from_env(None), Profile::Dev);
    }

    #[test]
    fn check_cfg_directives_cover_mires_and_custom_cfgs() {
        let builder = Crate::new("../client")
            .cfg("topcoat_client")
            .cfg("mode=\"csr\"")
            .cfg("topcoat_client");
        assert_eq!(
            builder.check_cfg_directives(),
            vec![
                "cargo::rustc-check-cfg=cfg(mires)".to_owned(),
                "cargo::rustc-check-cfg=cfg(topcoat_client)".to_owned(),
                "cargo::rustc-check-cfg=cfg(mode,values(any()))".to_owned(),
            ]
        );
    }

    #[test]
    fn default_args_use_sane_defaults_and_isolated_target_dir() {
        let builder = Crate::new("../client");
        let out_dir = Path::new("/tmp/host/target/debug/build/host-123/out");
        let target_dir = builder.resolved_target_dir(out_dir);
        assert_eq!(target_dir, out_dir.join("mires-target"));

        let expected_manifest = Path::new("../client")
            .join("Cargo.toml")
            .display()
            .to_string();
        let expected_target = out_dir.join("mires-target").display().to_string();
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
                expected_manifest.as_str(),
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
                expected_target.as_str(),
                "--mires-root",
                "/opt/mires",
                "--message-format",
                "json",
            ]
        );

        let args_no_root = strings(&builder.build_args(out_dir, &target_dir, Profile::Dev, None));
        assert!(!args_no_root.iter().any(|arg| arg == "--mires-root"));
    }

    #[test]
    fn builder_methods_populate_all_cargo_mires_flags() {
        let builder = Crate::new("workspace/client/Cargo.toml")
            .package("client-pkg")
            .profile(Profile::ReleaseSize)
            .format(ModuleFormat::Script)
            .runtime(RuntimeMode::External)
            .shim_module("topcoat-dom")
            .dts(false)
            .source_map(false)
            .npm(true)
            .panic_mode(PanicMode::Abort)
            .cfg("topcoat_client")
            .cfg("mode=\"hydrate\"")
            .chunk("app", "boot")
            .island("counter")
            .shared_chunk("common")
            .discover_chunks(DiscoverChunks::On)
            .out_dir("/custom/out")
            .target_dir("/custom/target")
            .toolchain_root("/custom/mires")
            .auto_install(true)
            .feature("feat-a")
            .feature("feat-b")
            .all_features(true)
            .no_default_features(true);

        let out_dir = builder.out_dir.clone().unwrap();
        let target_dir = builder.resolved_target_dir(&out_dir);
        assert_eq!(target_dir, PathBuf::from("/custom/target"));
        assert!(builder.auto_install);

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
                "--shim-module",
                "topcoat-dom",
                "--no-dts",
                "--no-source-map",
                "--npm",
                "--panic",
                "abort",
                "--cfg",
                "topcoat_client",
                "--cfg",
                "mode=\"hydrate\"",
                "--chunk",
                "app:boot",
                "--island",
                "counter",
                "--shared-chunk",
                "common",
                "--discover-chunks",
                "on",
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
                "--message-format",
                "json",
            ]
        );
    }

    #[test]
    fn auto_install_controls_env_var_on_spawned_command() {
        let out_dir = Path::new("/tmp/out");
        let target_dir = Path::new("/tmp/out/mires-target");

        let default_cmd = Crate::new("../client").build_command(
            Path::new("cargo-mires"),
            out_dir,
            target_dir,
            Profile::Dev,
            None,
        );
        assert!(default_cmd
            .get_envs()
            .all(|(key, _)| key != OsStr::new(AUTO_INSTALL_ENV)));

        let auto_cmd = Crate::new("../client").auto_install(true).build_command(
            Path::new("cargo-mires"),
            out_dir,
            target_dir,
            Profile::Dev,
            None,
        );
        assert_eq!(
            auto_cmd
                .get_envs()
                .find(|(key, _)| *key == OsStr::new(AUTO_INSTALL_ENV)),
            Some((OsStr::new(AUTO_INSTALL_ENV), Some(OsStr::new("1"))))
        );
    }

    #[test]
    fn resolve_toolchain_root_omits_root_outside_checkout_when_unconfigured() {
        let non_checkout = TempDir::new("non-checkout-root");
        let checkout = TempDir::new("fake-checkout-root");
        fs::create_dir_all(checkout.path().join("scripts")).unwrap();
        fs::write(checkout.path().join("rust-toolchain.toml"), "").unwrap();
        fs::write(checkout.path().join("scripts/build_sysroot.sh"), "").unwrap();

        assert_eq!(
            resolve_toolchain_root_from(None, None, None, non_checkout.path()),
            None
        );
        assert_eq!(
            resolve_toolchain_root_from(
                None,
                Some(OsStr::new("")),
                Some(OsStr::new("")),
                non_checkout.path(),
            ),
            None
        );
        assert_eq!(
            resolve_toolchain_root_from(None, None, None, checkout.path()),
            Some(checkout.path().to_path_buf())
        );
        assert_eq!(
            resolve_toolchain_root_from(
                None,
                Some(OsStr::new("/env/mires")),
                None,
                checkout.path()
            ),
            Some(PathBuf::from("/env/mires"))
        );
        assert_eq!(
            resolve_toolchain_root_from(
                Some(Path::new("/explicit/mires")),
                Some(OsStr::new("/env/mires")),
                Some(OsStr::new("/legacy/mires")),
                checkout.path(),
            ),
            Some(PathBuf::from("/explicit/mires"))
        );
    }

    #[test]
    fn cargo_bin_dir_prefers_cargo_home_then_home_dot_cargo() {
        assert_eq!(
            cargo_bin_dir_from_env(
                Some(OsStr::new("/custom/cargo")),
                Some(OsStr::new("/home/u"))
            ),
            Some(PathBuf::from("/custom/cargo/bin"))
        );
        assert_eq!(
            cargo_bin_dir_from_env(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.cargo/bin"))
        );
        assert_eq!(
            cargo_bin_dir_from_env(None, Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.cargo/bin"))
        );
        assert_eq!(cargo_bin_dir_from_env(None, None), None);
    }

    #[test]
    fn locate_cargo_mires_follows_exact_precedence_and_reports_install_hints() {
        let root = TempDir::new("locate-cargo-mires");
        let cargo_bin = TempDir::new("locate-cargo-mires-cargo-bin");
        let path_dir = TempDir::new("locate-cargo-mires-path");
        let bin = cargo_mires_bin_name();

        let err = locate_cargo_mires_in(
            Some(root.path()),
            Some(cargo_bin.path()),
            Some(path_dir.path().as_os_str()),
        )
        .unwrap_err();
        assert!(matches!(err, BuildError::CargoMiresMissing { .. }), "{err}");
        let msg = err.to_string();
        assert!(msg.contains("cargo binstall cargo-mires"), "{msg}");
        assert!(msg.contains("cargo install cargo-mires"), "{msg}");
        assert!(
            msg.contains("cargo build --release -p cargo-mires"),
            "{msg}"
        );

        let on_path = path_dir.path().join(bin);
        fs::write(&on_path, "").unwrap();
        assert_eq!(
            locate_cargo_mires_in(
                Some(root.path()),
                Some(cargo_bin.path()),
                Some(path_dir.path().as_os_str()),
            )
            .unwrap(),
            on_path
        );

        let in_cargo_bin = cargo_bin.path().join(bin);
        fs::write(&in_cargo_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_in(
                Some(root.path()),
                Some(cargo_bin.path()),
                Some(path_dir.path().as_os_str()),
            )
            .unwrap(),
            in_cargo_bin
        );

        let bundle_bin = root.path().join("bin").join(bin);
        fs::create_dir_all(bundle_bin.parent().unwrap()).unwrap();
        fs::write(&bundle_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_in(
                Some(root.path()),
                Some(cargo_bin.path()),
                Some(path_dir.path().as_os_str()),
            )
            .unwrap(),
            bundle_bin
        );

        let debug_bin = root.path().join("target/debug").join(bin);
        fs::create_dir_all(debug_bin.parent().unwrap()).unwrap();
        fs::write(&debug_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_in(
                Some(root.path()),
                Some(cargo_bin.path()),
                Some(path_dir.path().as_os_str()),
            )
            .unwrap(),
            debug_bin
        );

        let release_bin = root.path().join("target/release").join(bin);
        fs::create_dir_all(release_bin.parent().unwrap()).unwrap();
        fs::write(&release_bin, "").unwrap();
        assert_eq!(
            locate_cargo_mires_in(
                Some(root.path()),
                Some(cargo_bin.path()),
                Some(path_dir.path().as_os_str()),
            )
            .unwrap(),
            release_bin
        );
    }

    #[test]
    fn rerun_directives_watch_env_manifest_src_and_toolchain_binaries() {
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
            dir.path().join("src").join("lib.rs").display()
        )));
        assert!(directives.contains(&format!(
            "cargo:rerun-if-changed={}",
            dir.path()
                .join("src")
                .join("nested")
                .join("mod.rs")
                .display()
        )));
        assert!(!directives.iter().any(|line| line.contains("notes.txt")));

        let bundle = TempDir::new("crate-rerun-bundle");
        let linker = bundle.path().join("bin").join(jsc_link_bin_name());
        fs::create_dir_all(linker.parent().unwrap()).unwrap();
        fs::write(&linker, "").unwrap();
        let bundle_directives = Crate::new(dir.path()).rerun_directives(Some(bundle.path()));
        assert!(bundle_directives.contains(&format!("cargo:rerun-if-changed={}", linker.display())));
    }

    #[test]
    fn parse_cargo_mires_json_validates_schema_and_tolerates_prefixed_lines() {
        let payload = serde_json::json!({
            "schema": 1,
            "package": "demo",
            "profile": "dev",
            "format": "esm",
            "runtime": "inline",
            "out_dir": "/tmp/out",
            "program": "/tmp/out/demo.mjs",
            "chunks": [],
            "dep_files": ["/ws/shared/src/lib.rs"],
            "elapsed_secs": 0.1
        });
        let noisy = format!("Installed Mires toolchain v0.1.0\n{payload}\n");
        let parsed = parse_cargo_mires_json(&noisy).unwrap();
        assert_eq!(parsed.program, PathBuf::from("/tmp/out/demo.mjs"));
        assert_eq!(
            parsed.dep_files,
            vec![PathBuf::from("/ws/shared/src/lib.rs")]
        );

        let wrong_schema = serde_json::json!({
            "schema": 99,
            "out_dir": "/tmp/out",
            "program": "/tmp/out/demo.mjs"
        });
        assert!(matches!(
            parse_cargo_mires_json(&wrong_schema.to_string()),
            Err(BuildError::InvalidCargoMiresJson { .. })
        ));
        assert!(matches!(
            parse_cargo_mires_json("not json at all"),
            Err(BuildError::InvalidCargoMiresJson { .. })
        ));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn build_parses_json_chunks_manifest_and_emits_compilable_rust_module() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("crate-build-e2e-json");
        let client_dir = dir.path().join("client");
        fs::create_dir_all(client_dir.join("src")).unwrap();
        fs::write(
            client_dir.join("Cargo.toml"),
            "[package]\nname = \"demo-client\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(client_dir.join("src/lib.rs"), "").unwrap();

        let toolchain_dir = dir.path().join("mires");
        let bundle_bin_dir = toolchain_dir.join("bin");
        fs::create_dir_all(&bundle_bin_dir).unwrap();
        let out_dir = dir.path().join("out");
        let recorded_args = dir.path().join("recorded-args.txt");
        let recorded_env = dir.path().join("recorded-env.txt");
        let fake_cargo_mires = bundle_bin_dir.join("cargo-mires");

        let entry_js = "export function hi() {}\n";
        let entry_sha = format!("{:x}", Sha256::digest(entry_js.as_bytes()));
        let chunk_js = "export function __island_counter() {}\n";
        let chunk_sha = format!("{:x}", Sha256::digest(chunk_js.as_bytes()));
        let manifest_text = r#"{"schema":2,"entries":{},"artifacts":{}}"#;

        let json_payload = serde_json::json!({
            "schema": 1,
            "package": "demo-client",
            "profile": "dev",
            "format": "esm",
            "runtime": "external",
            "out_dir": out_dir,
            "program": out_dir.join("demo_client.mjs"),
            "source_map": out_dir.join("demo_client.mjs.map"),
            "dts": out_dir.join("demo_client.d.mts"),
            "runtime_file": out_dir.join("shim.mjs"),
            "package_json": null,
            "manifest_path": out_dir.join("demo_client.artifacts.json"),
            "manifest": { "schema": 2, "entries": {}, "artifacts": {} },
            "chunks": [
                {
                    "name": "counter",
                    "role": "island",
                    "path": out_dir.join("counter.js"),
                    "source_map": out_dir.join("counter.js.map"),
                    "dts": out_dir.join("counter.d.ts"),
                    "sha256": chunk_sha,
                    "bytes": chunk_js.len()
                }
            ],
            "dep_files": [
                client_dir.join("Cargo.toml"),
                dir.path().join("shared/src/lib.rs")
            ],
            "elapsed_secs": 0.02
        });

        let script = format!(
            concat!(
                "#!/usr/bin/env bash\n",
                "set -euo pipefail\n",
                "printf '%s\\n' \"$@\" >'{}'\n",
                "printf '%s' \"${{MIRES_AUTO_INSTALL:-unset}}\" >'{}'\n",
                "out='{}'\n",
                "mkdir -p \"$out\"\n",
                "printf '%s' '{}' >\"$out/demo_client.mjs\"\n",
                "printf '{{\"version\":3}}' >\"$out/demo_client.mjs.map\"\n",
                "printf 'export function hi(): void;\\n' >\"$out/demo_client.d.mts\"\n",
                "printf 'export const __rt = {{}};\\n' >\"$out/shim.mjs\"\n",
                "printf '%s' '{}' >\"$out/counter.js\"\n",
                "printf '{{\"version\":3}}' >\"$out/counter.js.map\"\n",
                "printf 'export function __island_counter(): void;\\n' >\"$out/counter.d.ts\"\n",
                "printf '%s' '{}' >\"$out/demo_client.artifacts.json\"\n",
                "printf '%s\\n' '{}'\n",
            ),
            recorded_args.display(),
            recorded_env.display(),
            out_dir.display(),
            entry_js,
            chunk_js,
            manifest_text,
            json_payload,
        );
        fs::write(&fake_cargo_mires, script).unwrap();
        fs::set_permissions(&fake_cargo_mires, fs::Permissions::from_mode(0o755)).unwrap();

        let artifacts = Crate::new(&client_dir)
            .profile(Profile::Dev)
            .runtime(RuntimeMode::External)
            .cfg("topcoat_client")
            .island("counter")
            .discover_chunks(DiscoverChunks::On)
            .out_dir(&out_dir)
            .toolchain_root(&toolchain_dir)
            .auto_install(true)
            .emit_rust_module("client_assets.rs")
            .build()
            .unwrap();

        assert_eq!(artifacts.program, out_dir.join("demo_client.mjs"));
        assert_eq!(
            artifacts.source_map,
            Some(out_dir.join("demo_client.mjs.map"))
        );
        assert_eq!(artifacts.dts, Some(out_dir.join("demo_client.d.mts")));
        assert_eq!(artifacts.runtime, Some(out_dir.join("shim.mjs")));
        assert_eq!(artifacts.package_json, None);
        assert_eq!(artifacts.out_dir, out_dir);
        assert_eq!(
            artifacts.manifest_path,
            Some(out_dir.join("demo_client.artifacts.json"))
        );
        assert_eq!(artifacts.manifest_json.as_deref(), Some(manifest_text));
        assert_eq!(artifacts.chunks.len(), 1);
        assert_eq!(artifacts.chunks[0].name, "counter");
        assert_eq!(artifacts.chunks[0].role, "island");
        assert_eq!(artifacts.chunks[0].sha256, chunk_sha);

        let module_path = artifacts.rust_module.as_ref().expect("rust_module emitted");
        assert_eq!(module_path, &out_dir.join("client_assets.rs"));
        let generated = fs::read_to_string(module_path).unwrap();
        assert!(generated.contains("pub const ENTRY_NAME: &str = \"demo_client.mjs\";"));
        assert!(generated.contains(&format!("pub const ENTRY_SHA256: &str = \"{entry_sha}\";")));
        assert!(generated.contains("pub const CHUNKS: &[ChunkAsset]"));
        assert!(generated.contains("pub const FILES: &[(&str, &[u8])]"));
        assert!(generated.contains("pub const MANIFEST_JSON: Option<&str>"));

        // Verify the generated Rust module compiles cleanly with `rustc --deny warnings`.
        let wrapper_rs = dir.path().join("check_assets.rs");
        fs::write(
            &wrapper_rs,
            format!(
                "#![deny(warnings)]\n\
                 pub mod assets {{\n\
                     include!({});\n\
                 }}\n\
                 pub fn check() -> (&'static str, &'static str, &'static str, usize, usize, bool) {{\n\
                     (\n\
                         assets::ENTRY_NAME,\n\
                         assets::ENTRY_JS,\n\
                         assets::ENTRY_SHA256,\n\
                         assets::CHUNKS.len(),\n\
                         assets::FILES.len(),\n\
                         assets::SOURCE_MAP.is_some() && assets::MANIFEST_JSON.is_some(),\n\
                     )\n\
                 }}\n",
                rust_str_lit(&module_path.display().to_string()),
            ),
        )
        .unwrap();
        let rmeta = dir.path().join("libcheck_assets.rmeta");
        let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
        let compile = Command::new(&rustc)
            .args([
                "--crate-type",
                "lib",
                "--edition",
                "2021",
                "--emit=metadata",
                "-o",
            ])
            .arg(&rmeta)
            .arg(&wrapper_rs)
            .output()
            .expect("invoke rustc to check generated module");
        assert!(
            compile.status.success(),
            "generated Rust module failed to compile:\n{}\n---\n{generated}",
            String::from_utf8_lossy(&compile.stderr),
        );

        assert_eq!(fs::read_to_string(&recorded_env).unwrap(), "1");
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
        assert!(recorded
            .windows(2)
            .any(|w| w == ["--message-format", "json"]));
        assert!(recorded
            .windows(2)
            .any(|w| w == ["--cfg", "topcoat_client"]));
        assert!(recorded.windows(2).any(|w| w == ["--island", "counter"]));
        assert!(recorded
            .windows(2)
            .any(|w| w == ["--discover-chunks", "on"]));
    }

    #[test]
    fn rejects_escaping_rust_module_paths() {
        assert!(validate_relative_module_path("").is_err());
        assert!(validate_relative_module_path("../escape.rs").is_err());
        assert!(validate_relative_module_path("/abs/path.rs").is_err());
        assert!(validate_relative_module_path("client_assets.rs").is_ok());
        assert!(validate_relative_module_path("gen/client_assets.rs").is_ok());
    }
}
