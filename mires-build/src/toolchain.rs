use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};

use crate::{collect_inputs, BuildError, Fix, Pinned, Result, RustOptLevel, Step, Unmet};

/// Environment variable overriding the spike checkout root.
pub const SPIKE_ROOT_ENV: &str = "MIRES_ROOT";
/// Environment variable the checkout's scripts read the backend's emit options
/// from.
pub const EXTRA_ARGS_ENV: &str = "JS_EXTRA_ARGS";
/// Environment variable carrying an explicit rustc optimization level to the build scripts.
pub const RUST_OPT_LEVEL_ENV: &str = "JSC_RUST_OPT_LEVEL";
/// Environment variable selecting the emit-affecting options used to key the sysroot.
pub const SYSROOT_ARGS_ENV: &str = "JSC_SYSROOT_ARGS";
/// Environment variable selecting the build mode: unset (or `abort`) is the default
/// `-Cpanic=abort` build, `unwind` the unwind build mode (`scripts/sysroot-cache.sh`, `jsc_panic`).
pub const PANIC_ENV: &str = "JSC_PANIC";

/// The build mode [`PANIC_ENV`] selects.
///
/// The checkout's scripts read the same variable, and the toolchain passes it to every script it
/// runs, so a client crate, its sysroot and the scripts can never disagree about the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicMode {
    /// `-Cpanic=abort` on `wasm32-unknown-unknown`: the default.
    Abort,
    /// `-Cpanic=unwind` on `targets/mires32-unwind.json`, with `js-cleanup=off` appended by the
    /// scripts: rustc's unwind edges carry host-exception cleanup.
    Unwind,
}

impl PanicMode {
    /// The mode a [`PANIC_ENV`] value names, or `None` for a value that names none.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("" | "abort") => Some(Self::Abort),
            Some("unwind") => Some(Self::Unwind),
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
            Self::Abort => None,
            Self::Unwind => Some("unwind"),
        }
    }
}

/// Cargo target directory for the feature-gated keyed view macros.
const KEYED_VIEW_MACRO_TARGET: &str = "keyed-view-macros";

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

    /// The template ABI a client crate's `view!` expansions are written
    /// against. Compiled by the backend into an rlib before the crate that
    /// names it.
    pub fn view_abi(&self) -> PathBuf {
        self.root.join("view-abi").join("src").join("lib.rs")
    }

    /// The crate directories the view macros are built from, in dependency
    /// order.
    ///
    /// Editing any of them changes what a `view!` or a `#[js_extern]` expands
    /// to, so they decide whether the built macros are still current. Which
    /// dylib each one belongs to is [`view_macro_sources`] and
    /// [`extern_macro_sources`]; this is the union, which is what a build
    /// script watches.
    ///
    /// [`view_macro_sources`]: Self::view_macro_sources
    /// [`extern_macro_sources`]: Self::extern_macro_sources
    pub fn view_sources(&self) -> [PathBuf; 4] {
        [
            self.root.join("view-abi"),
            self.root.join("view-dom"),
            self.root.join("view-dom-macro"),
            self.root.join("js-extern-macro"),
        ]
    }

    /// The crate directories `view-dom-macro` is built from.
    pub fn view_macro_sources(&self) -> [PathBuf; 3] {
        [
            self.root.join("view-abi"),
            self.root.join("view-dom"),
            self.root.join("view-dom-macro"),
        ]
    }

    /// The crate directories `js-extern-macro` is built from.
    ///
    /// A standalone proc macro: it depends on none of the view crates, so
    /// editing one of those does not make it stale and cargo would not rebuild
    /// it if it did.
    pub fn extern_macro_sources(&self) -> [PathBuf; 1] {
        [self.root.join("js-extern-macro")]
    }

    /// The proc-macro crate holding the view macros, built for the host.
    ///
    /// A proc macro runs inside rustc, so this one is an ordinary cargo build
    /// for the machine doing the compiling rather than anything the backend
    /// touches.
    pub fn view_macros(&self) -> PathBuf {
        self.view_macros_with_keyed(false)
    }

    pub(crate) fn view_macros_with_keyed(&self, keyed: bool) -> PathBuf {
        self.view_macro_artifact_dir(keyed).join(macro_dylib_name())
    }

    fn view_macro_artifact_dir(&self, keyed: bool) -> PathBuf {
        if keyed {
            return self
                .root
                .join("build")
                .join(KEYED_VIEW_MACRO_TARGET)
                .join("release");
        }
        self.root.join("target").join("release")
    }

    /// The proc-macro crate holding `#[js_extern]`, built for the host.
    ///
    /// It rides the [`ViewMacros`](Step::ViewMacros) step with the view macros:
    /// a client crate that uses either needs the expansion pass, and one step
    /// keeps "the macros are current" a single answer.
    pub fn js_extern_macro(&self) -> PathBuf {
        self.js_extern_macro_with_keyed(false)
    }

    pub(crate) fn js_extern_macro_with_keyed(&self, keyed: bool) -> PathBuf {
        self.view_macro_artifact_dir(keyed)
            .join(extern_macro_dylib_name())
    }

    /// The proc macro used by the framework's `#[procedure]` attribute.
    pub fn procedure_macro(&self) -> PathBuf {
        self.root
            .join("target")
            .join("release")
            .join(procedure_macro_dylib_name())
    }

    /// The `js!` proc macro used while building the client `topcoat-dom` seam.
    pub fn js_macro(&self) -> PathBuf {
        self.root
            .join("target")
            .join("release")
            .join(js_macro_dylib_name())
    }

    /// The client async executor compiled through this backend.
    pub fn view_async(&self) -> PathBuf {
        self.root.join("view-async").join("src").join("lib.rs")
    }

    /// The client half of the procedure transport compiled through this backend.
    pub fn topcoat_dom(&self) -> PathBuf {
        self.root.join("topcoat-dom").join("src").join("lib.rs")
    }

    /// Inputs that determine procedure expansion and target support.
    pub fn procedure_sources(&self) -> Vec<PathBuf> {
        let archived = self.root.join("procedure-support").join("source");
        if archived.is_dir() {
            return vec![
                self.root.join("js-macro"),
                self.root.join("topcoat-dom"),
                self.root.join("view-async"),
                archived,
            ];
        }
        vec![
            self.root.join("js-macro"),
            self.root.join("topcoat-dom"),
            self.root.join("view-async"),
            self.root
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&self.root)
                .join("crates")
                .join("topcoat-runtime"),
        ]
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
    ///
    /// `views` asks for the view macros as well, which only a crate that uses
    /// them needs. An empty list means the checkout is ready.
    ///
    /// This is the one definition of ready: [`check`](Self::check) reports the
    /// first entry as an error, [`apply`](Self::apply) performs one, and
    /// anything driving the setup from outside a build script works off the
    /// same list.
    pub fn preconditions(&self, emit_args: &str, views: bool) -> Vec<Unmet> {
        self.preconditions_with_opt_level(emit_args, views, None)
    }

    /// Every setup step missing for the requested backend and rustc optimization settings.
    pub fn preconditions_with_opt_level(
        &self,
        emit_args: &str,
        views: bool,
        opt_level: Option<RustOptLevel>,
    ) -> Vec<Unmet> {
        Step::ALL
            .into_iter()
            .filter(|step| views || *step != Step::ViewMacros)
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
            Step::ViewMacros => self.unmet_view_macros(),
        }
    }

    /// The command that performs `step`, whether or not it is already done.
    ///
    /// [`preconditions`](Self::preconditions) reports the steps that are
    /// missing; this is how a caller asks for one anyway, to redo it.
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
            Step::ViewMacros => self.view_macro_fix(false),
        }
    }

    fn view_macro_fix(&self, keyed: bool) -> Fix {
        let fix = Fix::new(Step::ViewMacros, "cargo").args([
            "build",
            "--release",
            "-p",
            "view-dom-macro",
            "-p",
            "js-extern-macro",
        ]);
        let fix = if keyed {
            fix.args([
                "--features".to_owned(),
                "view-dom-macro/keyed-v2".to_owned(),
                "--target-dir".to_owned(),
                path_arg(&self.root.join("build").join(KEYED_VIEW_MACRO_TARGET)),
            ])
        } else {
            fix
        };
        fix.dir(&self.root)
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
        let found = fs::read_to_string(sysroot.join(".js-args")).unwrap_or_default();
        let found_opt = fs::read_to_string(sysroot.join(".rust-opt-level")).unwrap_or_default();
        let found_key = fs::read_to_string(sysroot.join(".cache-key")).unwrap_or_default();
        let wanted_opt = opt_level.map(RustOptLevel::as_str).unwrap_or_default();
        let wanted_key = sysroot.file_name().and_then(|name| name.to_str());
        if self.sysroot_lib_for(&sysroot).is_dir()
            && args_match(&found, emit_args)
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

    /// The view macros, when they are not built for the host or were built
    /// before the sources they come from were last edited.
    ///
    /// Existence alone is not enough. A proc macro that is merely present runs
    /// happily and expands a `view!` the way it did whenever it was built, so
    /// an edit to the lowering shows up as compiled output that quietly
    /// disagrees with the source it was supposedly produced from. That is
    /// harder to notice than a missing dylib, not easier, because everything
    /// succeeds.
    fn unmet_view_macros(&self) -> Option<Unmet> {
        self.unmet_view_macros_with_keyed(false)
    }

    fn unmet_view_macros_with_keyed(&self, keyed: bool) -> Option<Unmet> {
        // Each dylib is asked about the sources IT is built from, and not about
        // the other one's. The two crates share a step but not a dependency
        // graph: `js-extern-macro` is a standalone proc macro, so an edit to
        // `view-dom` does not rebuild it and its timestamp stays where it was.
        // Comparing the oldest dylib against the newest source anywhere would
        // then report a staleness the fix cannot clear -- cargo has nothing to
        // redo for that crate -- and the build would refuse for good.
        let built = [
            (
                self.view_macros_with_keyed(keyed),
                &self.view_macro_sources()[..],
            ),
            (
                self.js_extern_macro_with_keyed(keyed),
                &self.extern_macro_sources()[..],
            ),
        ];
        if let Some((missing, _)) = built.iter().find(|(dylib, _)| !dylib.is_file()) {
            let reason = format!("the view macros are not built at {}", missing.display());
            return Some(Unmet::new(reason, self.view_macro_fix(keyed)));
        }

        for (dylib, sources) in built {
            // A time that cannot be read is not evidence of staleness. Saying
            // nothing leaves the build to the compile, which reports what it
            // actually hit; claiming a rebuild is needed on no evidence would
            // send everyone down the wrong path.
            let (Some(built), Some((source, edited))) = (modified(&dylib), newest_input(sources))
            else {
                continue;
            };
            if edited <= built {
                continue;
            }
            let reason = format!(
                "the view macros are older than {source}, so a `view!` or `#[js_extern]` \
                 would expand through a macro as it was built rather than as it is written",
                source = source.display()
            );
            return Some(Unmet::new(reason, self.view_macro_fix(keyed)));
        }
        None
    }

    /// Check the backend and a sysroot built with `emit_args` are in place.
    ///
    /// # Errors
    ///
    /// Returns `Err` naming the first unmet precondition and the command that
    /// satisfies it.
    pub fn check(&self, emit_args: &str) -> Result {
        self.report(self.preconditions(emit_args, false).into_iter().next())
    }

    /// Check the backend and sysroot against both backend and rustc optimization settings.
    pub fn check_with_opt_level(&self, emit_args: &str, opt_level: Option<RustOptLevel>) -> Result {
        self.report(
            self.preconditions_with_opt_level(emit_args, false, opt_level)
                .into_iter()
                .next(),
        )
    }

    /// Check the view macros are built for the host.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the proc-macro dylib is missing, with the cargo command
    /// that builds it.
    pub fn check_view_macros(&self) -> Result {
        self.report(self.unmet_view_macros())
    }

    pub(crate) fn check_view_macros_with_keyed(&self, keyed: bool) -> Result {
        self.report(self.unmet_view_macros_with_keyed(keyed))
    }

    /// Check the framework procedure macro and its client support macro are present and current.
    pub fn check_procedure_support(&self) -> Result {
        let built = [self.procedure_macro(), self.js_macro()];
        if let Some(path) = built.iter().find(|path| !path.is_file()) {
            return Err(BuildError::ProcedureSupportMissing {
                path: path.clone(),
                script: self.script("build-procedure-support.sh"),
            });
        }
        let sources = self.procedure_sources();
        if let (Some(oldest), Some((source, edited))) = (
            built.iter().filter_map(|path| modified(path)).min(),
            newest_input(&sources),
        ) {
            if edited > oldest {
                return Err(BuildError::ProcedureSupportStale {
                    input: source,
                    script: self.script("build-procedure-support.sh"),
                });
            }
        }
        Ok(())
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
            .env_remove("JSC_TARGET")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
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
        for unmet in self.preconditions(emit_args, false) {
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
        for unmet in self.preconditions_with_opt_level(emit_args, false, opt_level) {
            self.apply(unmet.fix())?;
        }
        self.check_with_opt_level(emit_args, opt_level)
    }

    /// Expand `src` into a single Rust file at `out`, running its macros and
    /// resolving `cfgs`.
    ///
    /// This is an explicit expansion utility. [`BuildConfig`](crate::BuildConfig)
    /// compiles proc-macro clients in one pass so rustc retains their original
    /// spans for diagnostics and source maps.
    ///
    /// # Errors
    ///
    /// Returns `Err` if rustc cannot be started, rejects the crate, or the
    /// expansion cannot be written.
    pub fn expand(
        &self,
        src: &Path,
        out: &Path,
        cfgs: &[String],
        externs: &[(String, PathBuf)],
    ) -> Result {
        self.expand_against(src, out, cfgs, externs, &[], &self.sysroot())
    }

    /// Expand against the immutable sysroot selected by the compiler settings.
    pub fn expand_with_opt_level(
        &self,
        src: &Path,
        out: &Path,
        cfgs: &[String],
        externs: &[(String, PathBuf)],
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Result {
        let sysroot = self.sysroot_for(emit_args, opt_level)?;
        self.expand_against(src, out, cfgs, externs, &[], &sysroot)
    }

    fn expand_against(
        &self,
        src: &Path,
        out: &Path,
        cfgs: &[String],
        externs: &[(String, PathBuf)],
        search_paths: &[PathBuf],
        sysroot: &Path,
    ) -> Result {
        let mut command = Command::new("rustc");
        command
            .arg(format!("+{}", self.pinned().channel()))
            .args(["--edition", "2021"]);
        match self.panic {
            PanicMode::Abort => {
                command.args(["--target", "wasm32-unknown-unknown"]);
            }
            PanicMode::Unwind => {
                command
                    .arg("--target")
                    .arg(self.unwind_target_spec())
                    .args(["-Zunstable-options", "-Cpanic=unwind"]);
            }
        }
        command
            .arg("--sysroot")
            .arg(sysroot)
            .args(["--crate-type", "cdylib"])
            .arg("-Zunpretty=expanded")
            .args(
                EXPANSION_GATES
                    .iter()
                    .map(|gate| format!("-Zcrate-attr={gate}")),
            )
            .arg("--remap-path-prefix")
            .arg(format!("{}/=", self.root.display()));
        for cfg in cfgs {
            command.args(["--cfg", cfg]);
        }
        for (name, path) in externs {
            command
                .arg("--extern")
                .arg(format!("{name}={}", path.display()));
        }
        for path in search_paths {
            command.arg("-L").arg(path);
        }
        command
            .arg(src)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub(&mut command);
        procedure_identity_env(&mut command);

        let output = command.output().map_err(|source| BuildError::Io {
            path: PathBuf::from("rustc"),
            source,
        })?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            eprint!("{stderr}");
            println!(
                "cargo::error=mires-build: rustc failed expanding {}",
                src.display()
            );
            return Err(BuildError::Expand {
                status: output.status,
                stderr,
            });
        }

        let expanded = String::from_utf8_lossy(&output.stdout);
        let expanded = allow_prelude_import(&strip_data_sections(&expanded));
        fs::write(out, expanded).map_err(|source| BuildError::Io {
            path: out.to_path_buf(),
            source,
        })
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
        scrub(&mut command);
        procedure_identity_env(&mut command);

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

/// The attribute rustc refuses on `wasm32`, which the view macros put on the
/// payload static.
const DATA_SECTION: &str = r#"link_section = "rcgjs.tc.data""#;

/// Drops the lines carrying [`DATA_SECTION`] from an expansion.
///
/// The crate attributes the explicit expansion utility needs, injected here
/// rather than written in the crate root.
///
/// A crate that uses the view macros is macro expanded first and the EXPANDED
/// TEXT is what gets compiled, so anything a macro lowers to has to be spelled
/// in a source file rather than only understood by the compiler that wrote it.
/// Three of the library's own lowerings are internal, and writing them down is
/// what needs a gate:
///
/// * `panic_internals` for the `core::panicking::panic_fmt` an `unreachable!` becomes, which every
///   `#[js_extern]` declaration carries in the marker body the backend replaces;
/// * `trivial_clone` and `derive_clone_copy_internals` for the assertions `#[derive(Clone, Copy)]`
///   emits.
///
/// None of this is about the crate: the same code compiled in ONE pass needs no
/// gate at all, because a macro may use what a source file may not. It is a
/// property of compiling expanded text as source, not of the author's crate,
/// so the gates belong to the opt-in utility. [`crate::BuildConfig`] avoids
/// them by compiling proc-macro consumers in one pass.
///
/// `internal_features` is allowed for the same reason: naming an internal
/// feature warns, and the expansion names one of its own (`prelude_import`)
/// before any source line is reached, so the allow covers the STEP rather than
/// only what is asked for here.
const EXPANSION_GATES: &[&str] = &[
    "allow(internal_features)",
    "feature(panic_internals, trivial_clone, derive_clone_copy_internals)",
];

/// `-Zunpretty=expanded` stops before the check that rejects the attribute, so
/// removing it here is what lets the expansion be compiled again.
fn strip_data_sections(expanded: &str) -> String {
    if !expanded.contains(DATA_SECTION) {
        return expanded.to_owned();
    }
    let mut out = String::with_capacity(expanded.len());
    for line in expanded.lines() {
        if line.trim_start().starts_with("#[") && line.contains(DATA_SECTION) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Silences the one warning the expansion itself introduces.
///
/// `-Zunpretty=expanded` writes the compiler's injected prelude import out as
/// ordinary source. A `no_std` crate that imports what it uses explicitly has
/// no use for the line, so recompiling the file reports it unused, which is a
/// warning no author can act on. Only the first occurrence is the injected
/// one, at the head of the crate.
fn allow_prelude_import(expanded: &str) -> String {
    expanded.replacen(
        "use core::prelude::rust_2021::*;",
        "#[allow(unused_imports)]\nuse core::prelude::rust_2021::*;",
        1,
    )
}

/// The view macro crate's dylib file name on this platform.
fn macro_dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libview_dom_macro.dylib"
    } else {
        "libview_dom_macro.so"
    }
}

/// The `#[js_extern]` macro crate's dylib file name on this platform.
fn extern_macro_dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libjs_extern_macro.dylib"
    } else {
        "libjs_extern_macro.so"
    }
}

fn js_macro_dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libjs_macro.dylib"
    } else {
        "libjs_macro.so"
    }
}

fn procedure_macro_dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libtopcoat_runtime_macro.dylib"
    } else {
        "libtopcoat_runtime_macro.so"
    }
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

/// When `path` was last written, or `None` if that cannot be read.
fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// The most recently edited input under `dirs`, and when it was edited.
///
/// Modification times rather than a content stamp, deliberately. A stamp only
/// works when whatever performs the rebuild writes it, which is true of the
/// sysroot because one script builds it and true of nothing else here: the
/// command reported for the view macros is an ordinary `cargo build` that
/// anyone can run by hand, and a stamp it did not update would assert freshness
/// that is not there. A stale stamp is worse than no check at all. Times need
/// no cooperation, and they are what cargo itself decides freshness by, so this
/// agrees with the tool that does the rebuild.
///
/// It errs toward reporting a rebuild: touching a file without changing it
/// counts as an edit. The cost of that is one cargo build that finds nothing to
/// do, against silently compiling through a macro that no longer matches its
/// source.
fn newest_input(dirs: &[PathBuf]) -> Option<(PathBuf, SystemTime)> {
    let mut inputs = Vec::new();
    for dir in dirs {
        collect_inputs(dir, &mut inputs);
    }
    inputs
        .into_iter()
        .filter_map(|path| modified(&path).map(|time| (path, time)))
        .max_by(|(left_path, left), (right_path, right)| {
            // Ties are broken by path so the file named in the message is the
            // same one on every run. Whole directories share a timestamp after
            // a checkout, and a message that names a different file each time
            // reads as a different problem each time.
            left.cmp(right).then_with(|| right_path.cmp(left_path))
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

/// Passes package identity to procedure macros without restoring Cargo's build environment.
fn procedure_identity_env(command: &mut Command) {
    for (target, source) in [
        ("TOPCOAT_PROCEDURE_MANIFEST_DIR", "CARGO_MANIFEST_DIR"),
        ("TOPCOAT_PROCEDURE_PKG_NAME", "CARGO_PKG_NAME"),
    ] {
        command.env_remove(target);
        if let Some(value) = env::var_os(source) {
            command.env(target, value);
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
    #[cfg(not(target_os = "windows"))]
    fn the_view_stack_hangs_off_the_checkout_root() {
        let toolchain = Toolchain::new("/spike").unwrap();
        assert_eq!(
            toolchain.view_abi(),
            Path::new("/spike/view-abi/src/lib.rs")
        );
        assert_eq!(
            toolchain.view_macros(),
            Path::new("/spike/target/release").join(macro_dylib_name())
        );
        assert_eq!(
            toolchain.view_macros_with_keyed(true),
            Path::new("/spike/build/keyed-view-macros/release").join(macro_dylib_name())
        );
    }

    #[test]
    fn an_expansion_without_the_attribute_is_left_alone() {
        let expanded = "fn main() {}\n";
        assert_eq!(strip_data_sections(expanded), expanded);
    }

    #[test]
    fn the_attribute_rustc_refuses_on_wasm_is_dropped() {
        let expanded = concat!(
            "static A: u8 = 0;\n",
            "    #[link_section = \"rcgjs.tc.data\"]\n",
            "static B: u8 = 1;\n",
        );
        assert_eq!(
            strip_data_sections(expanded),
            "static A: u8 = 0;\nstatic B: u8 = 1;\n"
        );
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
        fs::write(report.join(".js-args"), "js-opt=report").unwrap();
        fs::write(report.join(".rust-opt-level"), "1").unwrap();
        fs::write(report.join(".cache-key"), "report-o1").unwrap();
        assert_eq!(
            toolchain.unmet_sysroot_with_opt_level("js-opt=report", Some(RustOptLevel::O1),),
            None,
        );
        assert!(toolchain
            .unmet_sysroot_with_opt_level("js-opt=off", None)
            .is_some());
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
        assert_eq!(PanicMode::parse(None), Some(PanicMode::Abort));
        assert_eq!(PanicMode::parse(Some("")), Some(PanicMode::Abort));
        assert_eq!(PanicMode::parse(Some("abort")), Some(PanicMode::Abort));
        assert_eq!(PanicMode::parse(Some("unwind")), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("Unwind")), None);
        assert_eq!(PanicMode::Abort.target_name(), "wasm32-unknown-unknown");
        assert_eq!(PanicMode::Unwind.target_name(), "mires32-unwind");
    }

    /// The mode travels to the scripts explicitly, and an abort-mode toolchain masks an inherited
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
        for (mode, expected) in [(PanicMode::Unwind, "unwind"), (PanicMode::Abort, "unset")] {
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
    fn checkout_steps(toolchain: &Toolchain, emit_args: &str, views: bool) -> Vec<Unmet> {
        toolchain
            .preconditions(emit_args, views)
            .into_iter()
            .filter(|unmet| unmet.step() != Step::Toolchain)
            .collect()
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_checkout_with_nothing_built_needs_every_step() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let steps: Vec<Step> = checkout_steps(&toolchain, "", true)
            .iter()
            .map(Unmet::step)
            .collect();

        assert_eq!(steps, [Step::Backend, Step::Sysroot, Step::ViewMacros]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_view_macros_are_only_asked_for_by_a_crate_that_uses_them() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let steps: Vec<Step> = checkout_steps(&toolchain, "", false)
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
            .with_panic_mode(PanicMode::Abort);

        let fixes: Vec<String> = checkout_steps(&toolchain, "js-minify=on", true)
            .iter()
            .map(|unmet| unmet.fix().to_string())
            .collect();

        assert_eq!(
            fixes,
            [
                "cd /spike && bash scripts/build.sh",
                "cd /spike && JS_EXTRA_ARGS='js-minify=on' bash scripts/build_sysroot.sh",
                "cd /spike && cargo build --release -p view-dom-macro -p js-extern-macro",
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_sysroot_fix_carries_an_explicit_rust_optimization_level() {
        // The default build mode's command, whatever JSC_PANIC the test process inherited.
        let toolchain = Toolchain::new("/spike")
            .unwrap()
            .with_panic_mode(PanicMode::Abort);
        let fix =
            toolchain.fix_with_opt_level(Step::Sysroot, "js-opt=report", Some(RustOptLevel::O1));

        assert_eq!(
            fix.to_string(),
            concat!(
                "cd /spike && JS_EXTRA_ARGS='js-opt=report' JSC_RUST_OPT_LEVEL='1' ",
                "bash scripts/build_sysroot.sh",
            )
        );
        // The unwind mode's sysroot is a different one, and its fix says so.
        let unwind = toolchain.with_panic_mode(PanicMode::Unwind);
        assert_eq!(
            unwind
                .fix_with_opt_level(Step::Sysroot, "js-opt=report", Some(RustOptLevel::O1))
                .to_string(),
            concat!(
                "cd /spike && JS_EXTRA_ARGS='js-opt=report' JSC_RUST_OPT_LEVEL='1' ",
                "JSC_PANIC='unwind' bash scripts/build_sysroot.sh",
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
    fn checking_the_view_macros_reports_the_cargo_build_that_makes_them() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let error = toolchain.check_view_macros().unwrap_err();

        let BuildError::Unmet(unmet) = &error else {
            panic!("expected an unmet precondition, got {error}");
        };
        assert_eq!(unmet.step(), Step::ViewMacros);
        assert_eq!(
            unmet.fix().to_string(),
            "cd /spike && cargo build --release -p view-dom-macro -p js-extern-macro"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn keyed_view_macros_have_an_isolated_feature_build() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let error = toolchain.check_view_macros_with_keyed(true).unwrap_err();

        let BuildError::Unmet(unmet) = &error else {
            panic!("expected an unmet precondition, got {error}");
        };
        assert_eq!(
            unmet.fix().to_string(),
            concat!(
                "cd /spike && cargo build --release -p view-dom-macro -p js-extern-macro ",
                "--features view-dom-macro/keyed-v2 --target-dir build/keyed-view-macros",
            )
        );
    }

    /// A checkout holding a built view macro dylib and one source it is built
    /// from, so the two can be given modification times a test controls.
    #[cfg(not(target_os = "windows"))]
    fn view_checkout(label: &str) -> (TempDir, Toolchain) {
        let dir = TempDir::new(label);
        fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        fs::create_dir_all(dir.path().join("view-dom").join("src")).unwrap();
        fs::write(dir.path().join("view-dom").join("src").join("lower.rs"), "").unwrap();
        fs::write(dir.path().join("view-dom").join("Cargo.toml"), "").unwrap();
        for dylib in [macro_dylib_name(), extern_macro_dylib_name()] {
            fs::write(dir.path().join("target").join("release").join(dylib), "").unwrap();
        }

        let toolchain = Toolchain::new(dir.path()).unwrap();
        (dir, toolchain)
    }

    /// A fixed instant, plus `offset` seconds. Real clocks are not used: the
    /// question is only which of two files is newer.
    #[cfg(not(target_os = "windows"))]
    fn at(offset: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + offset)
    }

    #[cfg(not(target_os = "windows"))]
    fn touch(path: &Path, time: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_view_sources_are_the_four_crates_the_macros_are_built_from() {
        let toolchain = Toolchain::new("/spike").unwrap();

        assert_eq!(
            toolchain.view_sources(),
            [
                PathBuf::from("/spike/view-abi"),
                PathBuf::from("/spike/view-dom"),
                PathBuf::from("/spike/view-dom-macro"),
                PathBuf::from("/spike/js-extern-macro"),
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn view_macros_built_after_their_sources_are_current() {
        let (dir, toolchain) = view_checkout("views-current");
        touch(&dir.path().join("view-dom/src/lower.rs"), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(0));
        touch(&toolchain.view_macros(), at(60));

        assert_eq!(toolchain.unmet_view_macros(), None);
        assert!(toolchain.check_view_macros().is_ok());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn view_macros_built_at_the_same_instant_are_current() {
        let (dir, toolchain) = view_checkout("views-tie");
        touch(&dir.path().join("view-dom/src/lower.rs"), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(0));
        touch(&toolchain.view_macros(), at(0));

        assert_eq!(toolchain.unmet_view_macros(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_source_edited_after_the_build_makes_the_view_macros_stale() {
        let (dir, toolchain) = view_checkout("views-stale");
        touch(&toolchain.view_macros(), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(0));
        touch(&dir.path().join("view-dom/src/lower.rs"), at(60));

        let unmet = toolchain.unmet_view_macros().unwrap();

        assert_eq!(unmet.step(), Step::ViewMacros);
        assert!(unmet.reason().contains("view-dom/src/lower.rs"), "{unmet}");
        assert!(unmet.reason().contains("older than"), "{unmet}");
        assert_eq!(
            unmet.fix().to_string(),
            format!(
                "cd {} && cargo build --release -p view-dom-macro -p js-extern-macro",
                dir.path().display()
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_dylib_is_only_asked_about_the_sources_it_is_built_from() {
        // `js-extern-macro` depends on none of the view crates, so an edit to
        // `view-dom` leaves its dylib alone and cargo has nothing to redo for
        // it. Asking the oldest dylib about the newest source anywhere would
        // report a staleness the fix can never clear, and the build would
        // refuse for good.
        let (dir, toolchain) = view_checkout("views-independent");
        touch(&toolchain.js_extern_macro(), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(0));
        touch(&dir.path().join("view-dom/src/lower.rs"), at(60));
        touch(&toolchain.view_macros(), at(120));

        assert_eq!(toolchain.unmet_view_macros(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn an_edited_manifest_makes_the_view_macros_stale_too() {
        let (dir, toolchain) = view_checkout("views-manifest");
        touch(&toolchain.view_macros(), at(0));
        touch(&dir.path().join("view-dom/src/lower.rs"), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(60));

        let unmet = toolchain.unmet_view_macros().unwrap();

        assert!(unmet.reason().contains("view-dom/Cargo.toml"), "{unmet}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_stale_build_is_reported_alongside_the_other_steps() {
        let (dir, toolchain) = view_checkout("views-precondition");
        touch(&toolchain.view_macros(), at(0));
        touch(&dir.path().join("view-dom/Cargo.toml"), at(0));
        touch(&dir.path().join("view-dom/src/lower.rs"), at(60));

        let steps: Vec<Step> = toolchain
            .preconditions("", true)
            .iter()
            .map(Unmet::step)
            .filter(|step| *step != Step::Toolchain)
            .collect();

        assert!(steps.contains(&Step::ViewMacros), "{steps:?}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_missing_dylib_is_reported_as_missing_and_not_as_stale() {
        let (dir, toolchain) = view_checkout("views-missing");
        fs::remove_file(toolchain.view_macros()).unwrap();
        touch(&dir.path().join("view-dom/src/lower.rs"), at(60));

        let unmet = toolchain.unmet_view_macros().unwrap();

        assert!(unmet.reason().contains("are not built at"), "{unmet}");
        assert!(!unmet.reason().contains("older than"), "{unmet}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_checkout_with_no_view_sources_at_all_is_not_called_stale() {
        let dir = TempDir::new("views-sourceless");
        fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        for dylib in [macro_dylib_name(), extern_macro_dylib_name()] {
            fs::write(dir.path().join("target").join("release").join(dylib), "").unwrap();
        }

        let toolchain = Toolchain::new(dir.path()).unwrap();

        assert_eq!(newest_input(&toolchain.view_sources()), None);
        assert_eq!(toolchain.unmet_view_macros(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_newest_input_is_named_and_ties_break_by_path() {
        let dir = TempDir::new("views-newest");
        fs::create_dir_all(dir.path().join("view-dom").join("src")).unwrap();
        for name in ["b.rs", "a.rs"] {
            fs::write(dir.path().join("view-dom").join("src").join(name), "").unwrap();
            touch(&dir.path().join("view-dom").join("src").join(name), at(60));
        }
        fs::write(dir.path().join("view-dom").join("src").join("old.rs"), "").unwrap();
        touch(&dir.path().join("view-dom/src/old.rs"), at(0));

        let toolchain = Toolchain::new(dir.path()).unwrap();
        let (path, time) = newest_input(&toolchain.view_sources()).unwrap();

        assert_eq!(time, at(60));
        assert_eq!(path, dir.path().join("view-dom/src/a.rs"));
    }
}
