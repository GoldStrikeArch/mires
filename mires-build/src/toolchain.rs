use std::{
    collections::BTreeMap,
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use rcgjs_object::{
    artifact::sha256,
    opts::{normalize_optimizer_passes, Profile},
};

use crate::{
    checkout_root, dylib_filename, scrub, BuildError, Fix, PanicMode, Pinned, Result, RustOptLevel,
    Step, Unmet, PANIC_ENV, TOOLCHAIN_ROOT_ENV,
};

/// Environment variable the checkout's scripts read the backend's emit options
/// from.
pub const EXTRA_ARGS_ENV: &str = "JS_EXTRA_ARGS";
/// Environment variable carrying an explicit rustc optimization level to the build scripts.
pub const RUST_OPT_LEVEL_ENV: &str = "JSC_RUST_OPT_LEVEL";
/// Environment variable controlling rustc MIR inlining (`off` by default, or `on`).
pub const RUST_MIR_INLINE_ENV: &str = "JSC_RUST_MIR_INLINE";
/// Environment variable selecting the emit-affecting options used to key the sysroot.
pub const SYSROOT_ARGS_ENV: &str = "JSC_SYSROOT_ARGS";
/// Environment variable remapping checkout paths in compiler diagnostics and source maps.
pub const REMAP_ROOT_ENV: &str = "JSC_REMAP_ROOT";

/// Inputs identifying a cached sysroot build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysrootKey<'a> {
    pub emit_args: &'a str,
    pub opt_level: Option<RustOptLevel>,
}

impl<'a> SysrootKey<'a> {
    #[must_use]
    pub const fn new(emit_args: &'a str, opt_level: Option<RustOptLevel>) -> Self {
        Self {
            emit_args,
            opt_level,
        }
    }
}

/// Settings for one client crate compilation.
#[derive(Debug, Clone, Copy)]
pub struct CompileOptions<'a> {
    pub extra_args: &'a str,
    pub sysroot_args: Option<&'a str>,
    pub script_args: &'a [String],
    pub opt_level: Option<RustOptLevel>,
}

/// The Mires toolchain checkout a client crate is compiled with: the codegen
/// backend dylib, the `jsc-link` binary it execs, the sysroot (`core`, `alloc`,
/// `std`) built against them, and the scripts that produce all three.
#[derive(Debug, Clone)]
pub struct Toolchain {
    root: PathBuf,
    dylib: PathBuf,
    linker: PathBuf,
    panic: PanicMode,
}

impl Toolchain {
    /// The toolchain in the Mires checkout at `root`.
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
    #[must_use]
    pub fn panic_mode(&self) -> PanicMode {
        self.panic
    }

    /// The unwind mode's target spec.
    #[must_use]
    pub fn unwind_target_spec(&self) -> PathBuf {
        self.root.join("targets").join("mires32-unwind.json")
    }

    /// The toolchain in the Mires checkout named by [`TOOLCHAIN_ROOT_ENV`], else
    /// by `configured`, else the one this crate is checked out in.
    ///
    /// # Errors
    ///
    /// Returns `Err` on a platform the backend is not built for.
    pub fn resolve(configured: Option<PathBuf>) -> Result<Self> {
        if let Some(root) =
            env::var_os(TOOLCHAIN_ROOT_ENV).or_else(|| env::var_os("JSC_SPIKE_ROOT"))
        {
            return Self::new(root);
        }
        Self::new(configured.unwrap_or_else(Self::checkout_root))
    }

    /// The Mires checkout this crate was compiled from: the parent of its own
    /// package directory.
    pub(crate) fn checkout_root() -> PathBuf {
        checkout_root()
    }

    /// The Mires checkout root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The codegen backend dylib rustc is pointed at.
    #[must_use]
    pub fn dylib(&self) -> &Path {
        &self.dylib
    }

    /// The `jsc-link` binary the dylib execs to turn objects into a program.
    ///
    /// The dylib finds it as its own sibling, so nothing has to be passed on
    /// the command line; this is here so that the setup check can say which of
    /// the two halves is missing, and by name.
    #[must_use]
    pub fn linker(&self) -> &Path {
        &self.linker
    }

    /// Whether this checkout uses the split object-file linker introduced by v2.
    fn has_split_linker(&self) -> bool {
        self.root.join("jsc-link").join("Cargo.toml").is_file()
    }

    /// The compatibility path for the default compiler settings' sysroot.
    #[must_use]
    pub fn sysroot(&self) -> PathBuf {
        self.root.join("build").join("sysroot")
    }

    /// The file the sysroot records the emit options it was built with in.
    #[must_use]
    pub fn sysroot_args(&self) -> PathBuf {
        self.sysroot().join(".js-args")
    }

    /// The file recording the explicit rustc optimization level used for the sysroot.
    #[must_use]
    pub fn sysroot_opt_level(&self) -> PathBuf {
        self.sysroot().join(".rust-opt-level")
    }

    /// Compute the SHA-256 sysroot cache key in pure Rust, matching
    /// `scripts/sysroot-cache.sh`'s `jsc_sysroot_key` (including the `mires-std` digest).
    ///
    /// # Errors
    ///
    /// Returns `Err` if the backend dylib, unwind target spec, or pinned `rustc` toolchain
    /// cannot be read or identified.
    pub fn sysroot_key(&self, key: SysrootKey<'_>) -> Result<String> {
        let script = self.script("sysroot-cache.sh");
        if !self.dylib.is_file() {
            return Err(BuildError::SysrootKey {
                script,
                detail: format!("backend not built: {}", self.dylib.display()),
            });
        }
        let spec = self.unwind_target_spec();
        if self.panic == PanicMode::Unwind && !spec.is_file() {
            return Err(BuildError::SysrootKey {
                script,
                detail: format!("JSC_PANIC=unwind needs the target spec {}", spec.display()),
            });
        }

        let pinned = self.pinned();
        let channel = pinned.channel();
        let rustc = rustc_identity(channel, &script)?;
        let backend_hash = sha256_file(&self.dylib)?;
        let patches_hash = patches_digest(&self.root.join("patches"))?;
        let mires_std_hash = mires_crate_digest(&self.root, "mires-std")?;
        let mires_test_hash = mires_crate_digest(&self.root, "mires-test")?;
        let target = self.panic.target_name();
        let rust_opt = key.opt_level.map_or("default", RustOptLevel::as_str);
        let args = normalize_sysroot_emit_args(key.emit_args, self.panic);

        let mut material = String::new();
        let _ = writeln!(material, "cache-format=1");
        let _ = writeln!(material, "backend={backend_hash}");
        let _ = writeln!(material, "patches={patches_hash}");
        let _ = writeln!(material, "mires-std={mires_std_hash}");
        let _ = writeln!(material, "mires-test={mires_test_hash}");
        let _ = writeln!(material, "channel={channel}");
        let _ = writeln!(material, "rustc={rustc}");
        let _ = writeln!(material, "target={target}");
        let _ = writeln!(material, "rust-opt={rust_opt}");
        let _ = writeln!(material, "js-args={args}");
        if self.panic == PanicMode::Unwind {
            let spec_hash = sha256_file(&spec)?;
            let unwind_patches_hash = patches_digest(&self.root.join("patches").join("unwind"))?;
            let _ = writeln!(material, "panic=unwind");
            let _ = writeln!(material, "target-spec={spec_hash}");
            let _ = writeln!(material, "unwind-patches={unwind_patches_hash}");
        }
        Ok(sha256(material.as_bytes()))
    }

    /// The immutable sysroot directory computed in pure Rust for `emit_args` and `opt_level`.
    ///
    /// Matches `bash scripts/sysroot-cache.sh path` identically without spawning a shell.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any cache-key input cannot be read or identified.
    pub fn sysroot_dir(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result<PathBuf> {
        self.sysroot_dir_for_key(SysrootKey::new(emit_args, opt_level))
    }

    /// The immutable sysroot directory computed in pure Rust for `key`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any cache-key input cannot be read or identified.
    pub fn sysroot_dir_for_key(&self, key: SysrootKey<'_>) -> Result<PathBuf> {
        let digest = self.sysroot_key(key)?;
        Ok(self.root.join("build").join("sysroots").join(digest))
    }

    /// The immutable sysroot selected by the backend, toolchain, target, and compiler settings.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the checkout's key helper cannot identify those inputs.
    pub fn sysroot_for(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result<PathBuf> {
        self.sysroot_for_key(SysrootKey::new(emit_args, opt_level))
    }

    /// The immutable sysroot selected by `key`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the checkout's key helper cannot identify those inputs.
    pub fn sysroot_for_key(&self, key: SysrootKey<'_>) -> Result<PathBuf> {
        let script = self.script("sysroot-cache.sh");
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .arg("path")
            .env(SYSROOT_ARGS_ENV, key.emit_args.trim())
            // This Toolchain always targets wasm32, and typed settings must not inherit a
            // selector supplied to the parent build process.
            .env_remove("JSC_TARGET")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        optional_env(
            &mut command,
            RUST_OPT_LEVEL_ENV,
            key.opt_level.map(RustOptLevel::as_str),
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
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let detail = if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            };
            return Err(BuildError::SysrootKey { script, detail });
        }
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.trim();
        if path.is_empty() {
            return Err(BuildError::SysrootKey {
                script,
                detail: "sysroot key helper returned an empty path".to_owned(),
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
    #[must_use]
    pub fn shim(&self) -> PathBuf {
        self.root.join("runtime").join("shim.js")
    }

    /// The toolchain the checkout pins, which every child process that
    /// compiles a client crate asks for by name.
    #[must_use]
    pub fn pinned(&self) -> Pinned {
        Pinned::read(&self.root)
    }

    /// The script named `name` in the checkout.
    #[must_use]
    pub fn script(&self, name: &str) -> PathBuf {
        self.root.join("scripts").join(name)
    }

    /// Every setup step missing before a client crate compiled with
    /// `emit_args` can be built, in the order they have to be performed.
    #[must_use]
    pub fn preconditions(&self, emit_args: &str) -> Vec<Unmet> {
        self.preconditions_for(SysrootKey::new(emit_args, None))
    }

    /// Every setup step missing for the requested backend and rustc optimization settings.
    #[must_use]
    pub fn preconditions_with_opt_level(
        &self,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Vec<Unmet> {
        self.preconditions_for(SysrootKey::new(emit_args, opt_level))
    }

    /// Every setup step missing for `key`, in the order they have to be performed.
    #[must_use]
    pub fn preconditions_for(&self, key: SysrootKey<'_>) -> Vec<Unmet> {
        Step::ALL
            .into_iter()
            .filter_map(|step| self.unmet(step, key))
            .collect()
    }

    /// Whether `step` is missing, and why.
    fn unmet(&self, step: Step, key: SysrootKey<'_>) -> Option<Unmet> {
        match step {
            Step::Toolchain => self.unmet_toolchain(),
            Step::Backend => self.unmet_backend(),
            Step::Sysroot => self.unmet_sysroot(key),
        }
    }

    /// The command that performs `step`, whether or not it is already done.
    #[must_use]
    pub fn fix(&self, step: Step, emit_args: &str) -> Fix {
        self.fix_for(step, SysrootKey::new(emit_args, None))
    }

    /// The command that performs `step` with an explicit rustc optimization level.
    #[must_use]
    pub fn fix_with_opt_level(
        &self,
        step: Step,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Fix {
        self.fix_for(step, SysrootKey::new(emit_args, opt_level))
    }

    /// The command that performs `step` for `key`.
    #[must_use]
    pub fn fix_for(&self, step: Step, key: SysrootKey<'_>) -> Fix {
        match step {
            Step::Toolchain => Fix::new(step, "rustup").args(self.pinned().install_args()),
            Step::Backend => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build.sh")))
                .dir(&self.root),
            Step::Sysroot => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build_sysroot.sh")))
                .env(EXTRA_ARGS_ENV, key.emit_args.trim())
                .maybe_env(RUST_OPT_LEVEL_ENV, key.opt_level.map(RustOptLevel::as_str))
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
    fn unmet_sysroot(&self, key: SysrootKey<'_>) -> Option<Unmet> {
        let sysroot = match self.sysroot_for_key(key) {
            Ok(sysroot) => sysroot,
            Err(error) => {
                return Some(Unmet::new(
                    format!("the keyed sysroot cannot be resolved: {error}"),
                    self.fix_for(Step::Sysroot, key),
                ));
            }
        };
        let wanted_args = match self.panic {
            PanicMode::Unwind => format!("{} js-cleanup=off", key.emit_args),
            PanicMode::Abort => key.emit_args.to_owned(),
        };
        let found = fs::read_to_string(sysroot.join(".js-args")).unwrap_or_default();
        let found_opt = fs::read_to_string(sysroot.join(".rust-opt-level")).unwrap_or_default();
        let found_key = fs::read_to_string(sysroot.join(".cache-key")).unwrap_or_default();
        let wanted_opt = key.opt_level.map(RustOptLevel::as_str).unwrap_or_default();
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

        Some(Unmet::new(reason, self.fix_for(Step::Sysroot, key)))
    }

    /// Check the backend and a sysroot built with `emit_args` are in place.
    ///
    /// # Errors
    ///
    /// Returns `Err` naming the first unmet precondition and the command that
    /// satisfies it.
    pub fn check(&self, emit_args: &str) -> Result {
        self.check_with_opt_level(emit_args, None)
    }

    /// Check the backend and sysroot against both backend and rustc optimization settings.
    pub fn check_with_opt_level(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result {
        self.report(
            self.preconditions_for(SysrootKey::new(emit_args, opt_level))
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
        self.bootstrap_with_opt_level(emit_args, None)
    }

    /// Bootstrap and check the toolchain with an explicit rustc optimization level.
    pub fn bootstrap_with_opt_level(
        &self,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        for unmet in self.preconditions_for(SysrootKey::new(emit_args, opt_level)) {
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
        self.compile_with_options(
            client_root,
            out,
            CompileOptions {
                extra_args,
                sysroot_args: None,
                script_args,
                opt_level: None,
            },
        )
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
        self.compile_with_options(
            client_root,
            out,
            CompileOptions {
                extra_args,
                sysroot_args: None,
                script_args,
                opt_level,
            },
        )
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
        self.compile_with_options(
            client_root,
            out,
            CompileOptions {
                extra_args,
                sysroot_args: Some(sysroot_args),
                script_args,
                opt_level,
            },
        )
    }

    /// Compile `client_root` to `out` using grouped [`CompileOptions`].
    pub fn compile_with_options(
        &self,
        client_root: &Path,
        out: &Path,
        options: CompileOptions<'_>,
    ) -> Result {
        let script = self.script("compile_core.sh");
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .arg(client_root)
            .arg(out)
            .args(options.script_args)
            .env(EXTRA_ARGS_ENV, options.extra_args)
            // The script echoes the output path on success, and anything a
            // build script writes to stdout is a cargo directive.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        optional_env(&mut command, SYSROOT_ARGS_ENV, options.sysroot_args);
        optional_env(
            &mut command,
            RUST_OPT_LEVEL_ENV,
            options.opt_level.map(RustOptLevel::as_str),
        );
        optional_env(&mut command, PANIC_ENV, self.panic.env_value());
        if env::var_os(RUST_MIR_INLINE_ENV).is_none() {
            command.env(RUST_MIR_INLINE_ENV, "off");
        }
        scrub(&mut command);

        if env::var_os(REMAP_ROOT_ENV).is_none() {
            if let Some(root) = env::var_os("CARGO_MANIFEST_DIR") {
                command.env(REMAP_ROOT_ENV, root);
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
    /// Prefer [`runtime_from_manifest`](Self::runtime_from_manifest) or
    /// [`minimal_shim_from_manifest`](Self::minimal_shim_from_manifest), which run pure-Rust
    /// `jsc-link --emit-runtime` with no Node.js dependency.
    ///
    /// # Errors
    ///
    /// Returns `Err` if Node cannot start or the extractor reports an unknown runtime member.
    pub fn minimal_shim(&self, programs: &[PathBuf], out: &Path, module: bool) -> Result {
        let script = self.script("minimal-shim.mjs");
        let mut command = Command::new("node");
        command.arg(&script);
        if module {
            command.arg("--module");
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

    /// Extract only the runtime closure recorded by a verified linker entry manifest,
    /// using `jsc-link --emit-runtime` in pure Rust (no Node.js required).
    ///
    /// # Errors
    ///
    /// Returns `Err` if `jsc-link` cannot start or rejects the manifest.
    pub fn minimal_shim_from_manifest(&self, manifest: &Path, out: &Path, module: bool) -> Result {
        self.runtime_from_manifest(manifest, out, module, true)
    }

    /// Bind a linker manifest to the complete or minimal canonical runtime provider,
    /// invoking `jsc-link --emit-runtime` directly in pure Rust (no Node.js required).
    ///
    /// # Errors
    ///
    /// Returns `Err` if `jsc-link` cannot start or rejects the manifest.
    pub fn runtime_from_manifest(
        &self,
        manifest: &Path,
        out: &Path,
        module: bool,
        minimal: bool,
    ) -> Result {
        let mut command = Command::new(&self.linker);
        command.arg("--emit-runtime");
        if module {
            command.arg("--module");
        }
        if !minimal {
            command.arg("--full");
        }
        command
            .arg("--manifest")
            .arg(manifest)
            .arg(out)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub(&mut command);

        let output = command.output().map_err(|source| BuildError::Io {
            path: self.linker.clone(),
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

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|source| BuildError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(sha256(&bytes))
}

fn rustc_identity(channel: &str, script: &Path) -> Result<String> {
    let mut command = Command::new("rustc");
    command
        .arg(format!("+{channel}"))
        .arg("-vV")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    scrub(&mut command);

    let output = command.output().map_err(|source| BuildError::Io {
        path: PathBuf::from("rustc"),
        source,
    })?;
    if !output.status.success() {
        return Err(BuildError::SysrootKey {
            script: script.to_path_buf(),
            detail: format!("cannot identify pinned toolchain {channel}"),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_owned())
}

/// Digest of every non-hidden `*.patch` regular file directly inside `dir`, matching
/// `jsc_patches_digest` in `scripts/sysroot-cache.sh`.
fn patches_digest(dir: &Path) -> Result<String> {
    if !dir.is_dir() {
        return Ok(sha256(b"none\n"));
    }
    let mut patches = Vec::new();
    let entries = fs::read_dir(dir).map_err(|source| BuildError::Io {
        path: dir.to_path_buf(),
        source,
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
    if patches.is_empty() {
        return Ok(sha256(b"none\n"));
    }
    patches.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut material = String::new();
    for (name, path) in patches {
        let digest = sha256_file(&path)?;
        let _ = writeln!(material, "{name} {digest}");
    }
    Ok(sha256(material.as_bytes()))
}

/// Digest of `<dir_name>/Cargo.toml` and every `.rs` file under `<dir_name>/src/`, matching
/// `jsc_mires_std_digest` / `jsc_mires_test_digest` in `scripts/sysroot-cache.sh`.
fn mires_crate_digest(root: &Path, dir_name: &str) -> Result<String> {
    let dir = root.join(dir_name);
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
        sources.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (rel, path) in sources {
            found = true;
            let digest = sha256_file(&path)?;
            let _ = writeln!(material, "{rel} {digest}");
        }
    }

    if !found {
        material.push_str("none\n");
    }
    Ok(sha256(material.as_bytes()))
}

#[cfg(test)]
fn mires_std_digest(root: &Path) -> Result<String> {
    mires_crate_digest(root, "mires-std")
}

fn collect_rs_files(prefix: &str, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result {
    let entries = fs::read_dir(dir).map_err(|source| BuildError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
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

/// Normalize emit-affecting backend arguments for a sysroot cache key, matching
/// `jsc_mode_backend_args` + `jsc_normalize_emit_args` + `jsc_normalize_args`
/// in `scripts/sysroot-cache.sh`.
fn normalize_sysroot_emit_args(raw: &str, panic: PanicMode) -> String {
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

    let mut tokens: Vec<&str> = raw.split_whitespace().collect();
    if panic == PanicMode::Unwind {
        tokens.push("js-cleanup=off");
    }

    let mut profile_expanded: Vec<String> = Vec::new();
    let mut emit_explicit: Vec<String> = Vec::new();
    for token in tokens {
        if let Some(name) = token.strip_prefix("js-profile=") {
            match Profile::parse(name) {
                Some(profile) => {
                    profile_expanded.extend(profile.emit_args().iter().map(|s| (*s).to_owned()));
                }
                None => profile_expanded.push(token.to_owned()),
            }
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
            Some(value) => format!("js-opt-passes={}", normalize_optimizer_passes(value)),
            None => token,
        };
        let key = normalized
            .split_once('=')
            .map_or(normalized.as_str(), |(key, _)| key)
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
    dylib_filename().ok_or(BuildError::UnsupportedPlatform {
        os: env::consts::OS,
    })
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
            toolchain.unmet_sysroot(SysrootKey::new("js-opt=report", Some(RustOptLevel::O1))),
            None,
        );
        assert!(toolchain
            .unmet_sysroot(SysrootKey::new("js-opt=off", None))
            .is_some());

        let abort_toolchain = toolchain.with_panic_mode(PanicMode::Abort);
        let abort_report = abort_toolchain
            .sysroot_for("js-opt=report", Some(RustOptLevel::O1))
            .unwrap();
        fs::create_dir_all(abort_toolchain.sysroot_lib_for(&abort_report)).unwrap();
        fs::write(abort_report.join(".js-args"), "js-opt=report").unwrap();
        assert_eq!(
            abort_toolchain.unmet_sysroot(SysrootKey::new("js-opt=report", Some(RustOptLevel::O1))),
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

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn sysroot_key_script_failures_return_sysroot_key_error() {
        let dir = TempDir::new("sysroot-key-errors");
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        fs::write(
            dir.path().join("scripts/sysroot-cache.sh"),
            "echo 'bad key input' >&2; exit 2\n",
        )
        .unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();
        let error = toolchain.sysroot_for("js-opt=on", None).unwrap_err();
        assert!(
            matches!(&error, BuildError::SysrootKey { detail, .. } if detail == "bad key input"),
            "{error}"
        );

        fs::write(dir.path().join("scripts/sysroot-cache.sh"), "printf ''\n").unwrap();
        let empty = toolchain.sysroot_for("js-opt=on", None).unwrap_err();
        assert!(matches!(empty, BuildError::SysrootKey { .. }), "{empty}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn compile_error_display_includes_stderr() {
        let dir = TempDir::new("compile-error-display");
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        fs::write(
            dir.path().join("scripts/compile_core.sh"),
            "echo 'error[E0425]: cannot find value' >&2; exit 1\n",
        )
        .unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();
        let error = toolchain
            .compile(Path::new("client.rs"), Path::new("out.js"), "", &[])
            .unwrap_err();

        assert!(matches!(error, BuildError::Compile { .. }));
        assert!(
            error
                .to_string()
                .contains("error[E0425]: cannot find value"),
            "{error}"
        );
    }

    #[test]
    fn mires_std_digest_hashes_cargo_toml_and_sorted_rs_files() {
        let dir = TempDir::new("mires-std-digest");
        let empty = mires_std_digest(dir.path()).unwrap();
        assert_eq!(empty, sha256(b"none\n"));

        fs::create_dir_all(dir.path().join("mires-std/src/nested")).unwrap();
        fs::write(dir.path().join("mires-std/Cargo.toml"), "[package]\n").unwrap();
        fs::write(dir.path().join("mires-std/src/lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(
            dir.path().join("mires-std/src/nested/mod.rs"),
            "pub fn b() {}\n",
        )
        .unwrap();
        fs::write(dir.path().join("mires-std/src/README.md"), "ignored\n").unwrap();

        let first = mires_std_digest(dir.path()).unwrap();
        assert_ne!(first, empty);

        // Non-.rs files inside mires-std/src do not affect the digest.
        fs::write(dir.path().join("mires-std/src/README.md"), "changed\n").unwrap();
        assert_eq!(mires_std_digest(dir.path()).unwrap(), first);

        // Editing a Rust source inside mires-std/src changes the digest.
        fs::write(
            dir.path().join("mires-std/src/nested/mod.rs"),
            "pub fn b() { let _ = 1; }\n",
        )
        .unwrap();
        assert_ne!(mires_std_digest(dir.path()).unwrap(), first);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn sysroot_dir_matches_sysroot_cache_script_on_repo_root() {
        let root = Toolchain::checkout_root();
        let toolchain = Toolchain::new(&root).unwrap();
        if !toolchain.dylib().is_file() {
            return;
        }

        let cases = [
            (PanicMode::Unwind, "", None),
            (PanicMode::Abort, "", None),
            (
                PanicMode::Unwind,
                "js-profile=release-balanced",
                Some(RustOptLevel::O2),
            ),
            (
                PanicMode::Abort,
                "js-opt=on js-opt-passes=ast-dce,sroa",
                None,
            ),
            (
                PanicMode::Unwind,
                "js-opt=on js-opt-passes=all",
                Some(RustOptLevel::SizeMin),
            ),
        ];

        for (panic, emit_args, opt_level) in cases {
            let tc = toolchain.clone().with_panic_mode(panic);
            let rust_path = tc.sysroot_dir(emit_args, opt_level).unwrap();
            let script_path = tc.sysroot_for(emit_args, opt_level).unwrap();
            assert_eq!(
                rust_path, script_path,
                "pure-Rust sysroot_dir disagreed with scripts/sysroot-cache.sh for \
                 panic={panic:?}, emit_args={emit_args:?}, opt_level={opt_level:?}"
            );
        }
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn runtime_from_manifest_invokes_jsc_link_directly_without_node() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("runtime-jsc-link");
        let release_dir = dir.path().join("target/release");
        fs::create_dir_all(&release_dir).unwrap();
        let recorded = dir.path().join("args.txt");
        let fake_linker = release_dir.join("jsc-link");
        fs::write(
            &fake_linker,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" >'{}'\n",
                recorded.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&fake_linker, fs::Permissions::from_mode(0o755)).unwrap();

        let toolchain = Toolchain::new(dir.path()).unwrap();
        let manifest = dir.path().join("app.js.artifacts.json");
        let out = dir.path().join("shim.js");

        // Minimal ESM runtime: --emit-runtime --module --manifest <m> <out>
        toolchain
            .runtime_from_manifest(&manifest, &out, true, true)
            .unwrap();
        assert_eq!(
            fs::read_to_string(&recorded).unwrap(),
            format!(
                "--emit-runtime\n--module\n--manifest\n{}\n{}\n",
                manifest.display(),
                out.display()
            )
        );

        // Full script runtime: --emit-runtime --full --manifest <m> <out>
        toolchain
            .runtime_from_manifest(&manifest, &out, false, false)
            .unwrap();
        assert_eq!(
            fs::read_to_string(&recorded).unwrap(),
            format!(
                "--emit-runtime\n--full\n--manifest\n{}\n{}\n",
                manifest.display(),
                out.display()
            )
        );
    }
}
