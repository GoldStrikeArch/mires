use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{BuildError, Fix, Pinned, Result, RustOptLevel, Step, Unmet};

/// Environment variable overriding the spike checkout root.
pub const SPIKE_ROOT_ENV: &str = "MIRES_ROOT";
/// Environment variable the checkout's scripts read the backend's emit options
/// from.
pub const EXTRA_ARGS_ENV: &str = "JS_EXTRA_ARGS";
/// Environment variable carrying an explicit rustc optimization level to the build scripts.
pub const RUST_OPT_LEVEL_ENV: &str = "JSC_RUST_OPT_LEVEL";
/// Environment variable controlling rustc MIR inlining (`off` by default, or `on`).
pub const RUST_MIR_INLINE_ENV: &str = "JSC_RUST_MIR_INLINE";
/// Environment variable selecting the emit-affecting options used to key the sysroot.
pub const SYSROOT_ARGS_ENV: &str = "JSC_SYSROOT_ARGS";
/// Environment variable selecting the build mode: unset (or `unwind`) is the default
/// `-Cpanic=unwind` build, `abort` the `-Cpanic=abort` override (`scripts/sysroot-cache.sh`, `jsc_panic`).
pub const PANIC_ENV: &str = "JSC_PANIC";

/// The build mode [`PANIC_ENV`] selects.
///
/// The checkout's scripts read the same variable, and the toolchain passes it to every script it
/// runs, so a client crate, its sysroot and the scripts can never disagree about the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicMode {
    /// `-Cpanic=abort` on `wasm32-unknown-unknown`: explicit opt-in override.
    Abort,
    /// `-Cpanic=unwind` on `targets/mires32-unwind.json`, with `js-cleanup=off` appended by the
    /// scripts: rustc's unwind edges carry host-exception cleanup (the default).
    Unwind,
}

impl PanicMode {
    /// The mode a [`PANIC_ENV`] value names, or `None` for a value that names none.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("" | "unwind") => Some(Self::Unwind),
            Some("abort") => Some(Self::Abort),
            Some(_) => None,
        }
    }

    /// The mode in this process's environment.
    ///
    /// # Errors
    ///
    /// Returns `Err` when [`PANIC_ENV`] is set to anything but `abort` or `unwind`.
    pub fn from_env() -> Result<Self> {
        let value = env::var(PANIC_ENV).ok();
        Self::parse(value.as_deref()).ok_or_else(|| BuildError::PanicMode {
            value: value.unwrap_or_default(),
        })
    }

    /// The target's name: what the sysroot's `lib/rustlib/<name>/lib` is called.
    pub fn target_name(self) -> &'static str {
        match self {
            Self::Abort => "wasm32-unknown-unknown",
            Self::Unwind => "mires32-unwind",
        }
    }

    /// What [`PANIC_ENV`] is set to for a child process; `None` removes an inherited value.
    fn env_value(self) -> Option<&'static str> {
        match self {
            Self::Unwind => None,
            Self::Abort => Some("abort"),
        }
    }
}

/// The spike checkout a client crate is compiled with: the codegen backend
/// dylib, the `jsc-link` binary it execs, the `core` sysroot built against
/// them, and the scripts that produce all three.
#[derive(Debug, Clone)]
pub struct Toolchain {
    root: PathBuf,
    dylib: PathBuf,
    linker: PathBuf,
    panic: PanicMode,
}

impl Toolchain {
    /// The toolchain in the spike checkout at `root`.
    ///
    /// # Errors
    ///
    /// Returns `Err` on a platform the backend is not built for, or when [`PANIC_ENV`] names no
    /// build mode.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let dylib = root.join("target").join("release").join(dylib_name()?);
        let linker = root.join("target").join("release").join("jsc-link");
        Ok(Self {
            root,
            dylib,
            linker,
            panic: PanicMode::from_env()?,
        })
    }

    /// The same toolchain in another build mode.
    #[must_use]
    pub fn with_panic_mode(mut self, panic: PanicMode) -> Self {
        self.panic = panic;
        self
    }

    /// The build mode this toolchain compiles in.
    pub fn panic_mode(&self) -> PanicMode {
        self.panic
    }

    /// The unwind mode's target spec.
    pub fn unwind_target_spec(&self) -> PathBuf {
        self.root.join("targets").join("mires32-unwind.json")
    }

    /// The toolchain in the spike checkout named by [`SPIKE_ROOT_ENV`], else
    /// by `configured`, else the one this crate is checked out in.
    ///
    /// # Errors
    ///
    /// Returns `Err` on a platform the backend is not built for.
    pub fn resolve(configured: Option<PathBuf>) -> Result<Self> {
        if let Some(root) = env::var_os(SPIKE_ROOT_ENV).or_else(|| env::var_os("JSC_SPIKE_ROOT")) {
            return Self::new(root);
        }
        Self::new(configured.unwrap_or_else(Self::checkout_root))
    }

    /// The spike checkout this crate was compiled from: the parent of its own
    /// package directory.
    fn checkout_root() -> PathBuf {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf()
    }

    /// The spike checkout root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The codegen backend dylib rustc is pointed at.
    pub fn dylib(&self) -> &Path {
        &self.dylib
    }

    /// The `jsc-link` binary the dylib execs to turn objects into a program.
    ///
    /// The dylib finds it as its own sibling, so nothing has to be passed on
    /// the command line; this is here so that the setup check can say which of
    /// the two halves is missing, and by name.
    pub fn linker(&self) -> &Path {
        &self.linker
    }

    /// Whether this checkout uses the split object-file linker introduced by v2.
    fn has_split_linker(&self) -> bool {
        self.root.join("jsc-link").join("Cargo.toml").is_file()
    }

    /// The compatibility path for the default compiler settings' sysroot.
    pub fn sysroot(&self) -> PathBuf {
        self.root.join("build").join("sysroot")
    }

    /// The file the sysroot records the emit options it was built with in.
    pub fn sysroot_args(&self) -> PathBuf {
        self.sysroot().join(".js-args")
    }

    /// The file recording the explicit rustc optimization level used for the sysroot.
    pub fn sysroot_opt_level(&self) -> PathBuf {
        self.sysroot().join(".rust-opt-level")
    }

    /// The immutable sysroot selected by the backend, toolchain, target, and compiler settings.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the checkout's key helper cannot identify those inputs.
    pub fn sysroot_for(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result<PathBuf> {
        let script = self.script("sysroot-cache.sh");
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .arg("path")
            .env(SYSROOT_ARGS_ENV, emit_args.trim())
            // This Toolchain always targets wasm32, and typed settings must not inherit a
            // selector supplied to the parent build process.
            .env_remove("JSC_TARGET")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        optional_env(
            &mut command,
            RUST_OPT_LEVEL_ENV,
            opt_level.map(RustOptLevel::as_str),
        );
        optional_env(&mut command, PANIC_ENV, self.panic.env_value());
        if env::var_os(RUST_MIR_INLINE_ENV).is_none() {
            command.env(RUST_MIR_INLINE_ENV, "off");
        }
        scrub(&mut command);

        let output = command.output().map_err(|source| BuildError::Io {
            path: script.clone(),
            source,
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildError::Io {
                path: script,
                source: io::Error::other(stderr.trim().to_owned()),
            });
        }
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.trim();
        if path.is_empty() {
            return Err(BuildError::Io {
                path: script,
                source: io::Error::other("sysroot key helper returned an empty path"),
            });
        }
        Ok(PathBuf::from(path))
    }

    fn sysroot_lib_for(&self, sysroot: &Path) -> PathBuf {
        sysroot
            .join("lib")
            .join("rustlib")
            .join(self.panic.target_name())
            .join("lib")
    }

    /// The JavaScript runtime shim the compiled program calls into.
    pub fn shim(&self) -> PathBuf {
        self.root.join("runtime").join("shim.js")
    }

    /// The toolchain the checkout pins, which every child process that
    /// compiles a client crate asks for by name.
    pub fn pinned(&self) -> Pinned {
        Pinned::read(&self.root)
    }

    /// The script named `name` in the checkout.
    pub fn script(&self, name: &str) -> PathBuf {
        self.root.join("scripts").join(name)
    }

    /// Every setup step missing before a client crate compiled with
    /// `emit_args` can be built, in the order they have to be performed.
    pub fn preconditions(&self, emit_args: &str) -> Vec<Unmet> {
        self.preconditions_with_opt_level(emit_args, None)
    }

    /// Every setup step missing for the requested backend and rustc optimization settings.
    pub fn preconditions_with_opt_level(
        &self,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Vec<Unmet> {
        Step::ALL
            .into_iter()
            .filter_map(|step| match step {
                Step::Sysroot => self.unmet_sysroot_with_opt_level(emit_args, opt_level),
                _ => self.unmet(step, emit_args),
            })
            .collect()
    }

    /// Whether `step` is missing, and why.
    fn unmet(&self, step: Step, emit_args: &str) -> Option<Unmet> {
        match step {
            Step::Toolchain => self.unmet_toolchain(),
            Step::Backend => self.unmet_backend(),
            Step::Sysroot => self.unmet_sysroot(emit_args),
        }
    }

    /// The command that performs `step`, whether or not it is already done.
    pub fn fix(&self, step: Step, emit_args: &str) -> Fix {
        self.fix_with_opt_level(step, emit_args, None)
    }

    /// The command that performs `step` with an explicit rustc optimization level.
    pub fn fix_with_opt_level(
        &self,
        step: Step,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Fix {
        match step {
            Step::Toolchain => Fix::new(step, "rustup").args(self.pinned().install_args()),
            Step::Backend => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build.sh")))
                .dir(&self.root),
            Step::Sysroot => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build_sysroot.sh")))
                .env(EXTRA_ARGS_ENV, emit_args.trim())
                .maybe_env(RUST_OPT_LEVEL_ENV, opt_level.map(RustOptLevel::as_str))
                .maybe_env(PANIC_ENV, self.panic.env_value())
                .dir(&self.root),
        }
    }

    /// The pinned nightly, when `rustup` does not have it.
    fn unmet_toolchain(&self) -> Option<Unmet> {
        let pinned = self.pinned();
        if pinned.is_installed() {
            return None;
        }
        Some(Unmet::new(
            format!("the pinned toolchain {} is not installed", pinned.channel()),
            self.fix(Step::Toolchain, ""),
        ))
    }

    /// The backend, when its required executable pieces are not built.
    ///
    /// A v2 checkout is two programs: the dylib rustc loads and `jsc-link`.
    /// The v1 compatibility checkout emits its final program directly and has
    /// no `jsc-link` package, so the same stable build bridge can invoke it
    /// without inventing a binary that compiler never used.
    fn unmet_backend(&self) -> Option<Unmet> {
        let (what, path) = if !self.dylib.is_file() {
            ("the codegen backend", &self.dylib)
        } else if self.has_split_linker() && !self.linker.is_file() {
            ("the `jsc-link` binary", &self.linker)
        } else {
            return None;
        };
        Some(Unmet::new(
            format!("{what} is not built at {}", path.display()),
            self.fix(Step::Backend, ""),
        ))
    }

    /// The sysroot, when it is missing or was built with other emit options:
    /// `core`'s items and the client crate's have to agree on how they are
    /// spelled.
    fn unmet_sysroot(&self, emit_args: &str) -> Option<Unmet> {
        self.unmet_sysroot_with_opt_level(emit_args, None)
    }

    fn unmet_sysroot_with_opt_level(
        &self,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Option<Unmet> {
        let sysroot = match self.sysroot_for(emit_args, opt_level) {
            Ok(sysroot) => sysroot,
            Err(error) => {
                return Some(Unmet::new(
                    format!("the keyed sysroot cannot be resolved: {error}"),
                    self.fix_with_opt_level(Step::Sysroot, emit_args, opt_level),
                ));
            }
        };
        let wanted_args = match self.panic {
            PanicMode::Unwind => format!("{emit_args} js-cleanup=off"),
            PanicMode::Abort => emit_args.to_owned(),
        };
        let found = fs::read_to_string(sysroot.join(".js-args")).unwrap_or_default();
        let found_opt = fs::read_to_string(sysroot.join(".rust-opt-level")).unwrap_or_default();
        let found_key = fs::read_to_string(sysroot.join(".cache-key")).unwrap_or_default();
        let wanted_opt = opt_level.map(RustOptLevel::as_str).unwrap_or_default();
        let wanted_key = sysroot.file_name().and_then(|name| name.to_str());
        if self.sysroot_lib_for(&sysroot).is_dir()
            && args_match(&found, &wanted_args)
            && found_opt.trim() == wanted_opt
            && wanted_key.is_some_and(|key| found_key.trim() == key)
        {
            return None;
        }

        let reason = format!(
            "there is no complete matching sysroot at {}",
            sysroot.display()
        );

        Some(Unmet::new(
            reason,
            self.fix_with_opt_level(Step::Sysroot, emit_args, opt_level),
        ))
    }

    /// Check the backend and a sysroot built with `emit_args` are in place.
    ///
    /// # Errors
    ///
    /// Returns `Err` naming the first unmet precondition and the command that
    /// satisfies it.
    pub fn check(&self, emit_args: &str) -> Result {
        self.report(self.preconditions(emit_args).into_iter().next())
    }

    /// Check the backend and sysroot against both backend and rustc optimization settings.
    pub fn check_with_opt_level(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result {
        self.report(
            self.preconditions_with_opt_level(emit_args, opt_level)
                .into_iter()
                .next(),
        )
    }

    /// Turn an unmet precondition into the error that reports it.
    fn report(&self, unmet: Option<Unmet>) -> Result {
        match unmet {
            Some(unmet) => Err(BuildError::Unmet(Box::new(unmet))),
            None => Ok(()),
        }
    }

    /// Perform one setup step.
    ///
    /// The command prints to this process's stderr as it runs: the steps take
    /// minutes, and a build that looks hung is worse than a noisy one. Its
    /// stdout is dropped, because a build script's stdout carries cargo
    /// directives and nothing else.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the command cannot be started or exits non-zero.
    pub fn apply(&self, fix: &Fix) -> Result {
        let mut command = Command::new(fix.program());
        command
            .args(fix.arguments())
            .env_remove(RUST_OPT_LEVEL_ENV)
            .env_remove(SYSROOT_ARGS_ENV)
            .env_remove(PANIC_ENV)
            .env_remove("JSC_TARGET")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if env::var_os(RUST_MIR_INLINE_ENV).is_none() {
            command.env(RUST_MIR_INLINE_ENV, "off");
        }
        for (key, value) in fix.environment() {
            command.env(key, value);
        }
        if let Some(dir) = fix.directory() {
            command.current_dir(dir);
        }
        scrub(&mut command);

        let status = command.status().map_err(|source| BuildError::Io {
            path: PathBuf::from(fix.program()),
            source,
        })?;
        if !status.success() {
            return Err(BuildError::Fix {
                step: fix.step(),
                status,
            });
        }
        Ok(())
    }

    /// Perform every setup step a client crate compiled with `emit_args`
    /// needs, for an environment that has none of them.
    ///
    /// # Errors
    ///
    /// Returns `Err` if a step cannot be started, exits non-zero, or leaves
    /// its precondition unmet.
    pub fn bootstrap(&self, emit_args: &str) -> Result {
        for unmet in self.preconditions(emit_args) {
            self.apply(unmet.fix())?;
        }
        self.check(emit_args)
    }

    /// Bootstrap and check the toolchain with an explicit rustc optimization level.
    pub fn bootstrap_with_opt_level(
        &self,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        for unmet in self.preconditions_with_opt_level(emit_args, opt_level) {
            self.apply(unmet.fix())?;
        }
        self.check_with_opt_level(emit_args, opt_level)
    }

    /// Compile `client_root` to `out`, passing `extra_args` to the backend and
    /// `script_args` to the compile script.
    ///
    /// Anything rustc wrote to stderr on a successful run is re-emitted as
    /// `cargo::warning` lines, and on a failed one is written straight through
    /// to this process's stderr, where cargo shows it under the build script's
    /// output.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the script cannot be started or rustc rejects the
    /// client crate.
    pub fn compile(
        &self,
        client_root: &Path,
        out: &Path,
        extra_args: &str,
        script_args: &[String],
    ) -> Result {
        self.compile_with_opt_level(client_root, out, extra_args, script_args, None)
    }

    /// Compile with an explicit rustc optimization level.
    pub fn compile_with_opt_level(
        &self,
        client_root: &Path,
        out: &Path,
        extra_args: &str,
        script_args: &[String],
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        self.compile_inner(client_root, out, extra_args, None, script_args, opt_level)
    }

    /// Compile while selecting the sysroot from only the emit-affecting options.
    pub fn compile_with_settings(
        &self,
        client_root: &Path,
        out: &Path,
        extra_args: &str,
        sysroot_args: &str,
        script_args: &[String],
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        self.compile_inner(
            client_root,
            out,
            extra_args,
            Some(sysroot_args),
            script_args,
            opt_level,
        )
    }

    fn compile_inner(
        &self,
        client_root: &Path,
        out: &Path,
        extra_args: &str,
        sysroot_args: Option<&str>,
        script_args: &[String],
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        let script = self.script("compile_core.sh");
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .arg(client_root)
            .arg(out)
            .args(script_args)
            .env(EXTRA_ARGS_ENV, extra_args)
            // The script echoes the output path on success, and anything a
            // build script writes to stdout is a cargo directive.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        optional_env(&mut command, SYSROOT_ARGS_ENV, sysroot_args);
        optional_env(
            &mut command,
            RUST_OPT_LEVEL_ENV,
            opt_level.map(RustOptLevel::as_str),
        );
        optional_env(&mut command, PANIC_ENV, self.panic.env_value());
        if env::var_os(RUST_MIR_INLINE_ENV).is_none() {
            command.env(RUST_MIR_INLINE_ENV, "off");
        }
        scrub(&mut command);

        if env::var_os("JSC_REMAP_ROOT").is_none() {
            if let Some(root) = env::var_os("CARGO_MANIFEST_DIR") {
                command.env("JSC_REMAP_ROOT", root);
            }
        }

        let output = command.output().map_err(|source| BuildError::Io {
            path: script,
            source,
        })?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            eprint!("{stderr}");
            println!(
                "cargo::error=mires-build: rustc failed compiling {}",
                client_root.display()
            );
            return Err(BuildError::Compile {
                status: output.status,
                stderr,
            });
        }
        for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
            println!("cargo::warning={line}");
        }
        Ok(())
    }

    /// Extract the transitive subset of the runtime reached by `programs`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if Node cannot start or the extractor reports an unknown runtime member.
    pub fn minimal_shim(&self, programs: &[PathBuf], out: &Path, module: bool) -> Result {
        self.extract_shim(programs, None, out, module, true)
    }

    /// Extract only the runtime closure recorded by a verified linker entry manifest.
    pub fn minimal_shim_from_manifest(&self, manifest: &Path, out: &Path, module: bool) -> Result {
        self.extract_shim(&[], Some(manifest), out, module, true)
    }

    /// Bind a linker manifest to the complete or minimal canonical runtime provider.
    pub fn runtime_from_manifest(
        &self,
        manifest: &Path,
        out: &Path,
        module: bool,
        minimal: bool,
    ) -> Result {
        self.extract_shim(&[], Some(manifest), out, module, minimal)
    }

    fn extract_shim(
        &self,
        programs: &[PathBuf],
        manifest: Option<&Path>,
        out: &Path,
        module: bool,
        minimal: bool,
    ) -> Result {
        let script = self.script("minimal-shim.mjs");
        let mut command = Command::new("node");
        command.arg(&script);
        if module {
            command.arg("--module");
        }
        if !minimal {
            command.arg("--full");
        }
        if let Some(manifest) = manifest {
            command.arg("--manifest").arg(manifest);
        }
        command.arg(self.shim()).arg(out).args(programs);
        scrub(&mut command);

        let output = command.output().map_err(|source| BuildError::Io {
            path: script,
            source,
        })?;
        if !output.status.success() {
            return Err(BuildError::MinimalShim {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }
}

/// A path as a command argument. A path that is not valid UTF-8 is passed on
/// with the invalid parts replaced, which is what the shell line printed beside
/// it would show anyway.
fn path_arg(path: &Path) -> String {
    path.display().to_string()
}

/// The codegen backend dylib's file name on this platform.
///
/// # Errors
///
/// Returns `Err` on a platform the backend is not built for.
fn dylib_name() -> Result<&'static str> {
    if cfg!(target_os = "windows") {
        return Err(BuildError::UnsupportedPlatform {
            os: env::consts::OS,
        });
    }
    if cfg!(target_os = "macos") {
        Ok("librustc_codegen_mires.dylib")
    } else {
        Ok("librustc_codegen_mires.so")
    }
}

/// Whether two whitespace-delimited backend option sets are equivalent.
fn args_match(found: &str, wanted: &str) -> bool {
    fn normalized(args: &str) -> std::collections::BTreeMap<&str, String> {
        let mut normalized = std::collections::BTreeMap::new();
        for argument in args.split_whitespace() {
            let key = argument.split_once('=').map_or(argument, |(key, _)| key);
            // rustc-codegen-js options use last-value-wins semantics. This must match
            // scripts/sysroot-cache.sh or a cache can be built successfully but never satisfy
            // this precondition check.
            let argument = match argument.strip_prefix("js-opt-passes=") {
                Some(value) => format!("js-opt-passes={}", normalize_optimizer_passes(value)),
                None => argument.to_owned(),
            };
            if argument == "js-opt-passes=all" {
                normalized.remove(key);
            } else {
                normalized.insert(key, argument);
            }
        }
        normalized
    }
    normalized(found) == normalized(wanted)
}

fn normalize_optimizer_passes(value: &str) -> String {
    // The registered passes in compiler execution order. A normalized list names the selected
    // ones in that order, so two spellings of one selection share a cache key. This must match
    // scripts/sysroot-cache.sh's `jsc_normalize_opt_passes`.
    const ORDERED: [&str; 5] = [
        "sroa",
        "ast-dce",
        "future-poller",
        "native-representations",
        "link-dedup",
    ];

    if matches!(value, "all" | "none") {
        return value.to_owned();
    }
    let mut requested = Vec::new();
    for pass in value.split(',') {
        if !ORDERED.contains(&pass) {
            return value.to_owned();
        }
        requested.push(pass);
    }
    let selected: Vec<&str> = ORDERED
        .into_iter()
        .filter(|pass| requested.contains(pass))
        .collect();
    match selected.len() {
        0 => "none".to_owned(),
        count if count == ORDERED.len() => "all".to_owned(),
        _ => selected.join(","),
    }
}

/// Sets a typed child-process selector, explicitly masking an inherited value when absent.
fn optional_env(command: &mut Command, key: &str, value: Option<&str>) {
    match value {
        Some(value) => {
            command.env(key, value);
        }
        None => {
            command.env_remove(key);
        }
    }
}

/// Drop the environment cargo set for this build script.
///
/// The compile script asks for the spike's pinned nightly with an explicit
/// `+toolchain`, which an inherited `RUSTUP_TOOLCHAIN` would silently override.
/// The rest is dropped for the reason a nested `cargo build` drops it: those
/// variables shift the fingerprint of everything the script builds, so leaving
/// them set cache-busts every run.
pub(crate) fn scrub(command: &mut Command) {
    for (key, _) in env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("CARGO")
            || name == "RUSTC"
            || name == "RUSTC_WRAPPER"
            || name == "RUSTC_WORKSPACE_WRAPPER"
            || name == "RUSTUP_TOOLCHAIN"
            || name == "RUSTFLAGS"
        {
            command.env_remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TempDir;

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn dylib_name_follows_the_platform() {
        let expected = if cfg!(target_os = "macos") {
            "librustc_codegen_mires.dylib"
        } else {
            "librustc_codegen_mires.so"
        };
        assert_eq!(dylib_name().unwrap(), expected);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn dylib_sits_under_the_release_profile() {
        let toolchain = Toolchain::new("/spike").unwrap();
        assert_eq!(
            toolchain.dylib(),
            Path::new("/spike/target/release").join(dylib_name().unwrap())
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn paths_hang_off_the_checkout_root() {
        let toolchain = Toolchain::new("/spike").unwrap();
        assert_eq!(toolchain.sysroot(), Path::new("/spike/build/sysroot"));
        assert_eq!(
            toolchain.sysroot_args(),
            Path::new("/spike/build/sysroot/.js-args")
        );
        assert_eq!(
            toolchain.sysroot_opt_level(),
            Path::new("/spike/build/sysroot/.rust-opt-level")
        );
        assert_eq!(toolchain.shim(), Path::new("/spike/runtime/shim.js"));
        assert_eq!(
            toolchain.script("compile_core.sh"),
            Path::new("/spike/scripts/compile_core.sh")
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn legacy_checkout_does_not_require_the_v2_linker() {
        let dir = TempDir::new("legacy-backend");
        let toolchain = Toolchain::new(dir.path()).unwrap();
        fs::create_dir_all(toolchain.dylib().parent().unwrap()).unwrap();
        fs::write(toolchain.dylib(), "legacy").unwrap();

        assert_eq!(toolchain.unmet_backend(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn split_checkout_requires_its_linker() {
        let dir = TempDir::new("split-backend");
        let toolchain = Toolchain::new(dir.path()).unwrap();
        fs::create_dir_all(toolchain.dylib().parent().unwrap()).unwrap();
        fs::create_dir_all(dir.path().join("jsc-link")).unwrap();
        fs::write(dir.path().join("jsc-link/Cargo.toml"), "").unwrap();
        fs::write(toolchain.dylib(), "v2").unwrap();

        let unmet = toolchain.unmet_backend().unwrap();
        assert!(unmet.reason().contains("jsc-link"), "{unmet}");
    }

    #[test]
    fn empty_args_match_an_unstamped_sysroot() {
        assert!(args_match("", ""));
        assert!(args_match("\n", ""));
        assert!(args_match("", "  "));
    }

    #[test]
    fn args_match_ignores_surrounding_whitespace() {
        assert!(args_match(" js-names=mangled\n", "js-names=mangled"));
        assert!(!args_match("js-names=mangled", "js-minify=on"));
        assert!(!args_match("", "js-minify=on"));
        assert!(!args_match("js-minify=on", ""));
        assert!(args_match(
            "js-minify=on js-queue=off",
            "js-queue=off js-minify=on"
        ));
        assert!(args_match("js-minify=on js-minify=on", "js-minify=on"));
        assert!(args_match(
            "js-opt=off js-minify=on js-opt=on",
            "js-opt=on js-minify=on"
        ));
        assert!(!args_match("js-opt=on js-opt=off", "js-opt=on"));
        assert!(args_match(
            "js-opt=on js-opt-passes=link-dedup,native-representations,future-poller,ast-dce,sroa",
            "js-opt-passes=all js-opt=on"
        ));
        assert!(args_match("js-opt=on", "js-opt-passes=all js-opt=on"));
        assert!(!args_match(
            "js-opt=on js-opt-passes=native-representations,future-poller,ast-dce",
            "js-opt-passes=all js-opt=on"
        ));
        assert!(!args_match(
            "js-opt=on js-opt-passes=ast-dce",
            "js-opt=on js-opt-passes=future-poller"
        ));
    }

    #[test]
    fn absent_typed_selectors_mask_values_already_present_on_a_command() {
        let mut command = Command::new("ignored");
        command.env(RUST_OPT_LEVEL_ENV, "3");
        optional_env(&mut command, RUST_OPT_LEVEL_ENV, None);

        let value = command
            .get_envs()
            .find(|(key, _)| *key == RUST_OPT_LEVEL_ENV)
            .expect("the removal is explicit")
            .1;
        assert_eq!(value, None);

        optional_env(&mut command, RUST_OPT_LEVEL_ENV, Some("1"));
        let value = command
            .get_envs()
            .find(|(key, _)| *key == RUST_OPT_LEVEL_ENV)
            .expect("the value is explicit")
            .1;
        assert_eq!(value, Some(std::ffi::OsStr::new("1")));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn compiler_settings_select_coexisting_sysroots() {
        let dir = TempDir::new("keyed-sysroots");
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        fs::write(
            dir.path().join("scripts/sysroot-cache.sh"),
            format!(
                concat!(
                    "case \"${{JSC_SYSROOT_ARGS:-}}:${{JSC_RUST_OPT_LEVEL:-}}\" in\n",
                    "js-opt=report:1) key=report-o1 ;;\n",
                    "*) key=other ;;\n",
                    "esac\n",
                    "printf '%s/build/sysroots/%s\\n' '{}' \"$key\"\n",
                ),
                dir.path().display(),
            ),
        )
        .unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();

        let report = toolchain
            .sysroot_for("js-opt=report", Some(RustOptLevel::O1))
            .unwrap();
        let other = toolchain.sysroot_for("js-opt=off", None).unwrap();

        assert_eq!(report, dir.path().join("build/sysroots/report-o1"));
        assert_eq!(other, dir.path().join("build/sysroots/other"));
        assert_ne!(report, other);

        fs::create_dir_all(toolchain.sysroot_lib_for(&report)).unwrap();
        fs::write(report.join(".js-args"), "js-cleanup=off js-opt=report").unwrap();
        fs::write(report.join(".rust-opt-level"), "1").unwrap();
        fs::write(report.join(".cache-key"), "report-o1").unwrap();
        assert_eq!(
            toolchain.unmet_sysroot_with_opt_level("js-opt=report", Some(RustOptLevel::O1),),
            None,
        );
        assert!(toolchain
            .unmet_sysroot_with_opt_level("js-opt=off", None)
            .is_some());

        let abort_toolchain = toolchain.with_panic_mode(PanicMode::Abort);
        let abort_report = abort_toolchain
            .sysroot_for("js-opt=report", Some(RustOptLevel::O1))
            .unwrap();
        fs::create_dir_all(abort_toolchain.sysroot_lib_for(&abort_report)).unwrap();
        fs::write(abort_report.join(".js-args"), "js-opt=report").unwrap();
        assert_eq!(
            abort_toolchain.unmet_sysroot_with_opt_level("js-opt=report", Some(RustOptLevel::O1)),
            None,
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn compile_carries_separate_program_and_sysroot_options() {
        let dir = TempDir::new("compile-sysroot-settings");
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        let recorded = dir.path().join("settings");
        fs::write(
            dir.path().join("scripts/compile_core.sh"),
            format!(
                "printf '%s\\n%s\\n%s' \"$JS_EXTRA_ARGS\" \"$JSC_SYSROOT_ARGS\" \
                 \"$JSC_RUST_OPT_LEVEL\" >'{}'\n",
                recorded.display(),
            ),
        )
        .unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();

        toolchain
            .compile_with_settings(
                Path::new("client.rs"),
                Path::new("out.js"),
                "js-opt=report js-comments=off",
                "js-opt=report",
                &[],
                Some(RustOptLevel::O1),
            )
            .unwrap();

        assert_eq!(
            fs::read_to_string(recorded).unwrap(),
            "js-opt=report js-comments=off\njs-opt=report\n1",
        );
    }

    #[test]
    fn panic_mode_names_the_two_build_modes() {
        assert_eq!(PanicMode::parse(None), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("")), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("unwind")), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("abort")), Some(PanicMode::Abort));
        assert_eq!(PanicMode::parse(Some("Unwind")), None);
        assert_eq!(PanicMode::Abort.target_name(), "wasm32-unknown-unknown");
        assert_eq!(PanicMode::Unwind.target_name(), "mires32-unwind");
    }

    /// The mode travels to the scripts explicitly, and an unwind-mode toolchain masks an inherited
    /// selector, so a client crate is never compiled in a mode its toolchain does not describe.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn compile_passes_the_build_mode_through() {
        let dir = TempDir::new("compile-panic-mode");
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        let recorded = dir.path().join("mode");
        fs::write(
            dir.path().join("scripts/compile_core.sh"),
            format!(
                "printf '%s' \"${{JSC_PANIC-unset}}\" >'{}'\n",
                recorded.display()
            ),
        )
        .unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();
        for (mode, expected) in [(PanicMode::Unwind, "unset"), (PanicMode::Abort, "abort")] {
            let toolchain = toolchain.clone().with_panic_mode(mode);
            assert_eq!(toolchain.panic_mode(), mode);
            toolchain
                .compile(Path::new("client.rs"), Path::new("out.js"), "", &[])
                .unwrap();
            assert_eq!(fs::read_to_string(&recorded).unwrap(), expected);
        }
        let unwind = toolchain.with_panic_mode(PanicMode::Unwind);
        assert_eq!(
            unwind.sysroot_lib_for(Path::new("/sysroot")),
            Path::new("/sysroot/lib/rustlib/mires32-unwind/lib")
        );
        assert_eq!(
            unwind.unwind_target_spec(),
            dir.path().join("targets/mires32-unwind.json")
        );
    }

    /// The steps a checkout is missing, less the toolchain: whether the pinned
    /// nightly is installed is a property of the machine running the tests, not
    /// of the checkout they describe.
    #[cfg(not(target_os = "windows"))]
    fn checkout_steps(toolchain: &Toolchain, emit_args: &str) -> Vec<Unmet> {
        toolchain
            .preconditions(emit_args)
            .into_iter()
            .filter(|unmet| unmet.step() != Step::Toolchain)
            .collect()
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_checkout_with_nothing_built_needs_every_step() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let steps: Vec<Step> = checkout_steps(&toolchain, "")
            .iter()
            .map(Unmet::step)
            .collect();

        assert_eq!(steps, [Step::Backend, Step::Sysroot]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn every_fix_names_the_checkout_and_the_emit_options() {
        // The default build mode's commands, whatever JSC_PANIC the test process inherited.
        let toolchain = Toolchain::new("/spike")
            .unwrap()
            .with_panic_mode(PanicMode::Unwind);

        let fixes: Vec<String> = checkout_steps(&toolchain, "js-minify=on")
            .iter()
            .map(|unmet| unmet.fix().to_string())
            .collect();

        assert_eq!(
            fixes,
            [
                "cd /spike && bash scripts/build.sh",
                "cd /spike && JS_EXTRA_ARGS='js-minify=on' bash scripts/build_sysroot.sh",
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_sysroot_fix_carries_an_explicit_rust_optimization_level() {
        // The default build mode's command, whatever JSC_PANIC the test process inherited.
        let toolchain = Toolchain::new("/spike")
            .unwrap()
            .with_panic_mode(PanicMode::Unwind);
        let fix =
            toolchain.fix_with_opt_level(Step::Sysroot, "js-opt=report", Some(RustOptLevel::O1));

        assert_eq!(
            fix.to_string(),
            concat!(
                "cd /spike && JS_EXTRA_ARGS='js-opt=report' JSC_RUST_OPT_LEVEL='1' ",
                "bash scripts/build_sysroot.sh",
            )
        );
        // The abort mode's sysroot is a different one, and its fix says so.
        let abort = toolchain.with_panic_mode(PanicMode::Abort);
        assert_eq!(
            abort
                .fix_with_opt_level(Step::Sysroot, "js-opt=report", Some(RustOptLevel::O1))
                .to_string(),
            concat!(
                "cd /spike && JS_EXTRA_ARGS='js-opt=report' JSC_RUST_OPT_LEVEL='1' ",
                "JSC_PANIC='abort' bash scripts/build_sysroot.sh",
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn checking_reports_the_first_unmet_precondition_with_its_fix() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let error = toolchain.check("").unwrap_err();

        let BuildError::Unmet(unmet) = &error else {
            panic!("expected an unmet precondition, got {error}");
        };
        assert!(error.to_string().contains(&unmet.fix().to_string()));
    }
}
