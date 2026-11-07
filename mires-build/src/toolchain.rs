use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};

use crate::{collect_inputs, BuildError, Fix, Pinned, ProcMacro, Result, Step, Unmet};

/// Environment variable overriding the mires checkout root.
pub const ROOT_ENV: &str = "MIRES_ROOT";
/// Environment variable the checkout's scripts read the backend's emit options
/// from.
pub const EXTRA_ARGS_ENV: &str = "JS_EXTRA_ARGS";

/// The spike checkout a client crate is compiled with: the codegen backend
/// dylib, the `core` sysroot built against it, and the scripts that produce
/// both.
#[derive(Debug, Clone)]
pub struct Toolchain {
    root: PathBuf,
    dylib: PathBuf,
}

impl Toolchain {
    /// The toolchain in the spike checkout at `root`.
    ///
    /// # Errors
    ///
    /// Returns `Err` on a platform the backend is not built for.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let dylib = root.join("target").join("release").join(dylib_name()?);
        Ok(Self { root, dylib })
    }

    /// The toolchain in the spike checkout named by [`ROOT_ENV`], else
    /// by `configured`, else the one this crate is checked out in.
    ///
    /// # Errors
    ///
    /// Returns `Err` on a platform the backend is not built for.
    pub fn resolve(configured: Option<PathBuf>) -> Result<Self> {
        if let Some(root) = env::var_os(ROOT_ENV) {
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

    /// The sysroot holding the `core` built by the backend.
    pub fn sysroot(&self) -> PathBuf {
        self.root.join("build").join("sysroot")
    }

    /// The file the sysroot records the emit options it was built with in.
    pub fn sysroot_args(&self) -> PathBuf {
        self.sysroot().join(".js-args")
    }

    /// The directory the sysroot's compiled `core` lands in. Its absence is
    /// what distinguishes a missing sysroot from a half built one.
    fn sysroot_lib(&self) -> PathBuf {
        self.sysroot()
            .join("lib")
            .join("rustlib")
            .join("wasm32-unknown-unknown")
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

    /// The crate directories `js-extern-macro` is built from. Editing any of
    /// them changes what a `#[js_extern]` expands to, so they decide whether
    /// the built macro is still current.
    pub fn extern_macro_sources(&self) -> [PathBuf; 1] {
        [self.root.join("js-extern-macro")]
    }

    /// The proc-macro crate holding `#[js_extern]`, built for the host.
    ///
    /// A proc macro runs inside rustc, so this is an ordinary cargo build for
    /// the machine doing the compiling rather than anything the backend
    /// touches. It is the [`ExternMacro`](Step::ExternMacro) step.
    pub fn js_extern_macro(&self) -> PathBuf {
        self.root
            .join("target")
            .join("release")
            .join(extern_macro_dylib_name())
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
    /// `extern_macro` asks for the `#[js_extern]` macro as well, which only a
    /// crate that uses it needs. An empty list means the checkout is ready.
    ///
    /// This is the one definition of ready: [`check`](Self::check) reports the
    /// first entry as an error, [`apply`](Self::apply) performs one, and
    /// anything driving the setup from outside a build script works off the
    /// same list.
    pub fn preconditions(&self, emit_args: &str, extern_macro: bool) -> Vec<Unmet> {
        Step::ALL
            .into_iter()
            .filter(|step| extern_macro || *step != Step::ExternMacro)
            .filter_map(|step| self.unmet(step, emit_args))
            .collect()
    }

    /// Whether `step` is missing, and why.
    fn unmet(&self, step: Step, emit_args: &str) -> Option<Unmet> {
        match step {
            Step::Toolchain => self.unmet_toolchain(),
            Step::Backend => self.unmet_backend(),
            Step::Sysroot => self.unmet_sysroot(emit_args),
            Step::ExternMacro => self.unmet_extern_macro(),
        }
    }

    /// The command that performs `step`, whether or not it is already done.
    ///
    /// [`preconditions`](Self::preconditions) reports the steps that are
    /// missing; this is how a caller asks for one anyway, to redo it.
    pub fn fix(&self, step: Step, emit_args: &str) -> Fix {
        match step {
            Step::Toolchain => Fix::new(step, "rustup").args(self.pinned().install_args()),
            Step::Backend => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build.sh")))
                .dir(&self.root),
            Step::Sysroot => Fix::new(step, "bash")
                .arg(path_arg(&self.script("build_sysroot.sh")))
                .env(EXTRA_ARGS_ENV, emit_args.trim())
                .dir(&self.root),
            Step::ExternMacro => Fix::new(step, "cargo")
                .args(["build", "--release", "-p", "js-extern-macro"])
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

    /// The backend dylib, when it is not built.
    fn unmet_backend(&self) -> Option<Unmet> {
        if self.dylib.is_file() {
            return None;
        }
        Some(Unmet::new(
            format!(
                "the codegen backend is not built at {}",
                self.dylib.display()
            ),
            self.fix(Step::Backend, ""),
        ))
    }

    /// The sysroot, when it is missing or was built with other emit options:
    /// `core`'s items and the client crate's have to agree on how they are
    /// spelled.
    fn unmet_sysroot(&self, emit_args: &str) -> Option<Unmet> {
        let reason = if self.sysroot_lib().is_dir() {
            // A sysroot from before the stamp existed reads as no options at
            // all, which is what a default build asks for anyway.
            let found = fs::read_to_string(self.sysroot_args()).unwrap_or_default();
            if args_match(&found, emit_args) {
                return None;
            }
            format!(
                "the sysroot was built with different emit options: found {:?}, wanted {:?}",
                found.trim(),
                emit_args.trim()
            )
        } else {
            format!("there is no sysroot at {}", self.sysroot().display())
        };

        Some(Unmet::new(reason, self.fix(Step::Sysroot, emit_args)))
    }

    /// The `#[js_extern]` macro, when it is not built for the host or was
    /// built before the sources it comes from were last edited.
    ///
    /// Existence alone is not enough. A proc macro that is merely present runs
    /// happily and expands the way it did whenever it was built, so an edit to
    /// its source shows up as compiled output that quietly disagrees with the
    /// macro as written. That is harder to notice than a missing dylib, not
    /// easier, because everything succeeds.
    fn unmet_extern_macro(&self) -> Option<Unmet> {
        let reason = match staleness(&self.js_extern_macro(), &self.extern_macro_sources()) {
            None => return None,
            Some(Staleness::Missing) => format!(
                "the `#[js_extern]` macro is not built at {}",
                self.js_extern_macro().display()
            ),
            Some(Staleness::Stale(source)) => format!(
                "the `#[js_extern]` macro is older than {}, so a `#[js_extern]` would expand \
                 through the macro as it was built rather than as it is written",
                source.display()
            ),
        };
        Some(Unmet::new(reason, self.fix(Step::ExternMacro, "")))
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

    /// Check the `#[js_extern]` macro is built for the host and current.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the proc-macro dylib is missing or stale, with the
    /// cargo command that builds it.
    pub fn check_extern_macro(&self) -> Result {
        self.report(self.unmet_extern_macro())
    }

    /// Check a proc macro the caller named is built and current.
    ///
    /// # Errors
    ///
    /// Returns `Err` if its dylib is missing, or older than one of the sources
    /// it was built from.
    pub fn check_proc_macro(&self, proc_macro: &ProcMacro) -> Result {
        match staleness(proc_macro.dylib(), proc_macro.sources()) {
            None => Ok(()),
            Some(found) => Err(BuildError::ProcMacro {
                crate_name: proc_macro.crate_name().to_owned(),
                dylib: proc_macro.dylib().to_path_buf(),
                newer: match found {
                    Staleness::Missing => None,
                    Staleness::Stale(source) => Some(source),
                },
            }),
        }
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

    /// Expand `src` into a single Rust file at `out`, running its macros and
    /// resolving `cfgs`.
    ///
    /// A crate that uses a proc macro is compiled in two passes because the
    /// backend cannot run one: this pass is plain rustc, and what it writes is
    /// the ordinary Rust the backend then compiles. The cfgs are
    /// resolved here too, which is what lets one source hold both halves of an
    /// island.
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
        let mut command = Command::new("rustc");
        command
            .arg(format!("+{}", self.pinned().channel()))
            .args(["--edition", "2021"])
            .args(["--target", "wasm32-unknown-unknown"])
            .arg("--sysroot")
            .arg(self.sysroot())
            .args(["--crate-type", "cdylib"])
            .arg("-Zunpretty=expanded")
            .args(EXPANSION_GATES.iter().map(|gate| format!("-Zcrate-attr={gate}")))
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
        command
            .arg(src)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub(&mut command);

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
        let expanded = allow_prelude_import(&expanded);
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
        scrub(&mut command);

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
}

/// A path as a command argument. A path that is not valid UTF-8 is passed on
/// with the invalid parts replaced, which is what the shell line printed beside
/// it would show anyway.
fn path_arg(path: &Path) -> String {
    path.display().to_string()
}

/// The crate attributes every two-pass client crate needs, injected here rather
/// than written in the crate root.
///
/// A crate that uses a proc macro is macro expanded first and the EXPANDED
/// TEXT is what gets compiled, so anything a macro lowers to has to be spelled
/// in a source file rather than only understood by the compiler that wrote it.
/// Three of the library's own lowerings are internal, and writing them down is
/// what needs a gate:
///
/// * `panic_internals` for the `core::panicking::panic_fmt` an `unreachable!`
///   becomes, which every `#[js_extern]` declaration carries in the marker body
///   the backend replaces;
/// * `trivial_clone` and `derive_clone_copy_internals` for the assertions
///   `#[derive(Clone, Copy)]` emits.
///
/// None of this is about the crate: the same code compiled in ONE pass needs no
/// gate at all, because a macro may use what a source file may not. It is a
/// property of the two-pass arrangement, which is mires-build's and not the
/// author's -- so the gates belong here. The first island rediscovered all
/// three the hard way, one `error[E0658]` at a time, and the next one would
/// have rediscovered them again.
///
/// `internal_features` is allowed for the same reason: naming an internal
/// feature warns, and the expansion names one of its own (`prelude_import`)
/// before any source line is reached, so the allow covers the STEP rather than
/// only what is asked for here. `unused_features` likewise: a crate whose
/// expansion happens to need none of the three gates would otherwise be warned
/// about attributes it never wrote.
const EXPANSION_GATES: &[&str] = &[
    "allow(internal_features, unused_features)",
    "feature(panic_internals, trivial_clone, derive_clone_copy_internals)",
];

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

/// The `#[js_extern]` macro crate's dylib file name on this platform.
fn extern_macro_dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libjs_extern_macro.dylib"
    } else {
        "libjs_extern_macro.so"
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

/// Whether a proc-macro dylib is missing or older than the sources it is
/// built from.
///
/// A time that cannot be read is not evidence of staleness. Saying nothing
/// leaves the build to the compile, which reports what it actually hit;
/// claiming a rebuild is needed on no evidence would send everyone down the
/// wrong path.
fn staleness(dylib: &Path, sources: &[PathBuf]) -> Option<Staleness> {
    if !dylib.is_file() {
        return Some(Staleness::Missing);
    }
    let (built, (source, edited)) = (modified(dylib)?, newest_input(sources)?);
    (edited > built).then_some(Staleness::Stale(source))
}

/// Why a proc-macro dylib cannot be loaded as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Staleness {
    /// There is no dylib.
    Missing,
    /// A source it is built from was edited after it was built.
    Stale(PathBuf),
}

/// The most recently edited input under `dirs`, and when it was edited.
///
/// Modification times rather than a content stamp, deliberately. A stamp only
/// works when whatever performs the rebuild writes it, which is true of the
/// sysroot because one script builds it and true of nothing else here: the
/// command reported for a macro is an ordinary `cargo build` that
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

/// Whether a sysroot recording `found` was built for a crate compiled with
/// `wanted`. Both sides are trimmed, so a sysroot built with no options at all
/// matches a default build.
fn args_match(found: &str, wanted: &str) -> bool {
    found.trim() == wanted.trim()
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
            toolchain.sysroot_lib(),
            Path::new("/spike/build/sysroot/lib/rustlib/wasm32-unknown-unknown/lib")
        );
        assert_eq!(toolchain.shim(), Path::new("/spike/runtime/shim.js"));
        assert_eq!(
            toolchain.script("compile_core.sh"),
            Path::new("/spike/scripts/compile_core.sh")
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_macro_stack_hangs_off_the_checkout_root() {
        let toolchain = Toolchain::new("/spike").unwrap();
        assert_eq!(
            toolchain.view_abi(),
            Path::new("/spike/view-abi/src/lib.rs")
        );
        assert_eq!(
            toolchain.js_extern_macro(),
            Path::new("/spike/target/release").join(extern_macro_dylib_name())
        );
        assert_eq!(
            toolchain.extern_macro_sources(),
            [PathBuf::from("/spike/js-extern-macro")]
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
        // Only the ends are trimmed: the order and spacing inside the string
        // are part of what the sysroot was built with.
        assert!(!args_match(
            "js-minify=on js-queue=off",
            "js-queue=off js-minify=on"
        ));
    }

    /// The steps a checkout is missing, less the toolchain: whether the pinned
    /// nightly is installed is a property of the machine running the tests, not
    /// of the checkout they describe.
    #[cfg(not(target_os = "windows"))]
    fn checkout_steps(toolchain: &Toolchain, emit_args: &str, extern_macro: bool) -> Vec<Unmet> {
        toolchain
            .preconditions(emit_args, extern_macro)
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

        assert_eq!(steps, [Step::Backend, Step::Sysroot, Step::ExternMacro]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_extern_macro_is_only_asked_for_by_a_crate_that_uses_it() {
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
        let toolchain = Toolchain::new("/spike").unwrap();

        let fixes: Vec<String> = checkout_steps(&toolchain, "js-minify=on", true)
            .iter()
            .map(|unmet| unmet.fix().to_string())
            .collect();

        assert_eq!(
            fixes,
            [
                "cd /spike && bash scripts/build.sh",
                "cd /spike && JS_EXTRA_ARGS='js-minify=on' bash scripts/build_sysroot.sh",
                "cd /spike && cargo build --release -p js-extern-macro",
            ]
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
    fn checking_the_extern_macro_reports_the_cargo_build_that_makes_it() {
        let toolchain = Toolchain::new("/spike").unwrap();

        let error = toolchain.check_extern_macro().unwrap_err();

        let BuildError::Unmet(unmet) = &error else {
            panic!("expected an unmet precondition, got {error}");
        };
        assert_eq!(unmet.step(), Step::ExternMacro);
        assert_eq!(
            unmet.fix().to_string(),
            "cd /spike && cargo build --release -p js-extern-macro"
        );
    }

    /// A checkout holding a built `#[js_extern]` dylib and one source it is
    /// built from, so the two can be given modification times a test controls.
    #[cfg(not(target_os = "windows"))]
    fn macro_checkout(label: &str) -> (TempDir, Toolchain) {
        let dir = TempDir::new(label);
        fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        fs::create_dir_all(dir.path().join("js-extern-macro").join("src")).unwrap();
        fs::write(dir.path().join("js-extern-macro/src/lib.rs"), "").unwrap();
        fs::write(dir.path().join("js-extern-macro/Cargo.toml"), "").unwrap();
        fs::write(
            dir.path()
                .join("target")
                .join("release")
                .join(extern_macro_dylib_name()),
            "",
        )
        .unwrap();

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
    fn a_macro_built_after_its_sources_is_current() {
        let (dir, toolchain) = macro_checkout("macro-current");
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(0));
        touch(&dir.path().join("js-extern-macro/Cargo.toml"), at(0));
        touch(&toolchain.js_extern_macro(), at(60));

        assert_eq!(toolchain.unmet_extern_macro(), None);
        assert!(toolchain.check_extern_macro().is_ok());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_macro_built_at_the_same_instant_is_current() {
        let (dir, toolchain) = macro_checkout("macro-tie");
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(0));
        touch(&dir.path().join("js-extern-macro/Cargo.toml"), at(0));
        touch(&toolchain.js_extern_macro(), at(0));

        assert_eq!(toolchain.unmet_extern_macro(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_source_edited_after_the_build_makes_the_macro_stale() {
        let (dir, toolchain) = macro_checkout("macro-stale");
        touch(&toolchain.js_extern_macro(), at(0));
        touch(&dir.path().join("js-extern-macro/Cargo.toml"), at(0));
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(60));

        let unmet = toolchain.unmet_extern_macro().unwrap();

        assert_eq!(unmet.step(), Step::ExternMacro);
        assert!(unmet.reason().contains("js-extern-macro/src/lib.rs"), "{unmet}");
        assert!(unmet.reason().contains("older than"), "{unmet}");
        assert_eq!(
            unmet.fix().to_string(),
            format!(
                "cd {} && cargo build --release -p js-extern-macro",
                dir.path().display()
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn an_edited_manifest_makes_the_macro_stale_too() {
        let (dir, toolchain) = macro_checkout("macro-manifest");
        touch(&toolchain.js_extern_macro(), at(0));
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(0));
        touch(&dir.path().join("js-extern-macro/Cargo.toml"), at(60));

        let unmet = toolchain.unmet_extern_macro().unwrap();

        assert!(unmet.reason().contains("js-extern-macro/Cargo.toml"), "{unmet}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_stale_build_is_reported_alongside_the_other_steps() {
        let (dir, toolchain) = macro_checkout("macro-precondition");
        touch(&toolchain.js_extern_macro(), at(0));
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(60));

        let steps: Vec<Step> = toolchain
            .preconditions("", true)
            .iter()
            .map(Unmet::step)
            .filter(|step| *step != Step::Toolchain)
            .collect();

        assert!(steps.contains(&Step::ExternMacro), "{steps:?}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_missing_dylib_is_reported_as_missing_and_not_as_stale() {
        let (dir, toolchain) = macro_checkout("macro-missing");
        fs::remove_file(toolchain.js_extern_macro()).unwrap();
        touch(&dir.path().join("js-extern-macro/src/lib.rs"), at(60));

        let unmet = toolchain.unmet_extern_macro().unwrap();

        assert!(unmet.reason().contains("is not built at"), "{unmet}");
        assert!(!unmet.reason().contains("older than"), "{unmet}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_checkout_with_no_macro_sources_at_all_is_not_called_stale() {
        let dir = TempDir::new("macro-sourceless");
        fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        fs::write(
            dir.path()
                .join("target")
                .join("release")
                .join(extern_macro_dylib_name()),
            "",
        )
        .unwrap();

        let toolchain = Toolchain::new(dir.path()).unwrap();

        assert_eq!(newest_input(&toolchain.extern_macro_sources()), None);
        assert_eq!(toolchain.unmet_extern_macro(), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_newest_input_is_named_and_ties_break_by_path() {
        let dir = TempDir::new("macro-newest");
        fs::create_dir_all(dir.path().join("js-extern-macro").join("src")).unwrap();
        for name in ["b.rs", "a.rs"] {
            fs::write(dir.path().join("js-extern-macro/src").join(name), "").unwrap();
            touch(&dir.path().join("js-extern-macro/src").join(name), at(60));
        }
        fs::write(dir.path().join("js-extern-macro/src/old.rs"), "").unwrap();
        touch(&dir.path().join("js-extern-macro/src/old.rs"), at(0));

        let toolchain = Toolchain::new(dir.path()).unwrap();
        let (path, time) = newest_input(&toolchain.extern_macro_sources()).unwrap();

        assert_eq!(time, at(60));
        assert_eq!(path, dir.path().join("js-extern-macro/src/a.rs"));
    }

    #[test]
    fn a_callers_macro_is_checked_against_the_sources_it_names() {
        let dir = TempDir::new("proc-macro-stale");
        fs::create_dir_all(dir.path().join("my-macro/src")).unwrap();
        fs::write(dir.path().join("my-macro/src/lib.rs"), "").unwrap();
        fs::write(dir.path().join("libmy_macro.so"), "").unwrap();
        let toolchain = Toolchain::new(dir.path()).unwrap();
        let proc_macro = ProcMacro::new("my_macro", dir.path().join("libmy_macro.so"))
            .source_dir(dir.path().join("my-macro"));

        touch(&dir.path().join("my-macro/src/lib.rs"), at(0));
        touch(&dir.path().join("libmy_macro.so"), at(60));
        assert!(toolchain.check_proc_macro(&proc_macro).is_ok());

        touch(&dir.path().join("my-macro/src/lib.rs"), at(120));
        let error = toolchain.check_proc_macro(&proc_macro).unwrap_err();
        let BuildError::ProcMacro { crate_name, newer, .. } = &error else {
            panic!("expected a stale macro, got {error}");
        };
        assert_eq!(crate_name, "my_macro");
        assert_eq!(newer.as_deref(), Some(dir.path().join("my-macro/src/lib.rs").as_path()));
        assert!(error.to_string().contains("older than"), "{error}");

        fs::remove_file(dir.path().join("libmy_macro.so")).unwrap();
        let error = toolchain.check_proc_macro(&proc_macro).unwrap_err();
        assert!(error.to_string().contains("is not built at"), "{error}");
    }
}
