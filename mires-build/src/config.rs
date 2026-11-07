use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{
    shimcheck::SHIM_MODULE_ARG, BuildError, Chunk, Chunking, ProcMacro, Rerun, Result, Toolchain,
};

/// File name of the default [`output_name`](BuildConfig::output_name) inside
/// `OUT_DIR`.
pub const DEFAULT_OUTPUT_NAME: &str = "app.js";
/// File name of the runtime shim copied into `OUT_DIR`.
pub const SHIM_NAME: &str = "shim.js";
/// Suffix of the import map written beside the program, after its
/// [`output_name`](BuildConfig::output_name) without the extension.
pub const IMPORT_MAP_SUFFIX: &str = ".importmap.json";
/// Directory inside `OUT_DIR` the compiler writes to before publishing.
const STAGING_DIR: &str = ".mires-build";

/// Builder for a client crate compiled to JavaScript from a Cargo build
/// script.
///
/// The default configuration compiles the crate rooted at the given file with
/// the backend and sysroot already built in the mires checkout, and publishes
/// `app.js`, `app.js.map`, and `shim.js` into `$OUT_DIR`:
///
/// ```rust,no_run
/// mires_build::BuildConfig::new("client/app.rs")
///     .render()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct BuildConfig {
    client_root: PathBuf,
    output_name: String,
    source_map: bool,
    shim: bool,
    emit_args: Vec<String>,
    crate_args: Vec<String>,
    cfgs: Vec<String>,
    js_extern: bool,
    proc_macros: Vec<ProcMacro>,
    rlibs: Vec<(String, PathBuf)>,
    chunking: Option<Chunking>,
    root: Option<PathBuf>,
    bootstrap: bool,
    shim_module_source: Option<PathBuf>,
}

impl BuildConfig {
    /// The default configuration for the client crate rooted at
    /// `client_root`, ready to be customized with the builder methods and
    /// executed with [`render`](Self::render).
    ///
    /// `client_root` is the single `.rs` file rustc is pointed at. The crate
    /// under it is `#![no_std]` and `#![no_main]`, carries its own
    /// `#[panic_handler]`, and has `core` and nothing else: there is no
    /// `alloc`. Extra modules are reached with `#[path = "..."] mod`, since
    /// there is no Cargo target to lay them out. Its `#[no_mangle]` functions
    /// are both the exports and the roots dead code elimination keeps, and
    /// their names survive into the JavaScript unchanged.
    #[must_use]
    pub fn new(client_root: impl Into<PathBuf>) -> Self {
        Self {
            client_root: client_root.into(),
            output_name: DEFAULT_OUTPUT_NAME.to_owned(),
            source_map: true,
            shim: true,
            emit_args: Vec::new(),
            crate_args: Vec::new(),
            cfgs: Vec::new(),
            js_extern: false,
            proc_macros: Vec::new(),
            rlibs: Vec::new(),
            chunking: None,
            root: None,
            bootstrap: false,
            shim_module_source: None,
        }
    }

    /// Name of the emitted program inside `OUT_DIR`. Defaults to
    /// [`DEFAULT_OUTPUT_NAME`], which can be loaded from source via
    /// `asset!(concat!(env!("OUT_DIR"), "/app.js"))`.
    #[must_use]
    pub fn output_name(mut self, name: impl Into<String>) -> Self {
        self.output_name = name.into();
        self
    }

    /// Emit a `.js.map` beside the program. Defaults to `true`.
    #[must_use]
    pub fn source_map(mut self, on: bool) -> Self {
        self.source_map = on;
        self
    }

    /// The file backing the module `__rt` is imported from, so that every
    /// member the compiled chunks reach for can be checked against it.
    ///
    /// Only meaningful with `-Cllvm-args=js-shim-module=<specifier>`, which is
    /// how an island build names a module in place of the global-scope
    /// `shim.js`. The specifier is what the emitted `import * as __rt from ..`
    /// says; this is where that specifier's file lives in the source tree.
    ///
    /// # Why it is worth wiring up
    ///
    /// The backend emits `__rt.<name>(...)` for every operation with no short
    /// inline spelling, and a module the application wrote supplies whichever
    /// of them its islands happen to need. **A member it does not supply is not
    /// a compile error; it is a `TypeError` the first time that line runs**, in
    /// a browser, in whichever branch reaches it first. With this set, it is a
    /// build failure naming the member and the chunk that wanted it.
    #[must_use]
    pub fn shim_module_source(mut self, path: impl Into<PathBuf>) -> Self {
        self.shim_module_source = Some(path.into());
        self
    }

    /// Copy the runtime shim into `OUT_DIR`. Defaults to `true`.
    ///
    /// The shim is a plain script defining the `__rt` object every compiled
    /// program calls into, so the page needs it whether it is served from here
    /// or written by hand.
    #[must_use]
    pub fn shim(mut self, on: bool) -> Self {
        self.shim = on;
        self
    }

    /// Pass an emit-affecting backend option, such as `js-minify=on`.
    ///
    /// These options change how items are spelled, so the client crate and the
    /// sysroot's `core` have to agree on them: the sysroot records the set it
    /// was built with, and [`render`](Self::render) refuses to compile against
    /// a sysroot that disagrees. Calls accumulate, in order.
    #[must_use]
    pub fn emit_arg(mut self, arg: impl Into<String>) -> Self {
        self.emit_args.push(arg.into());
        self
    }

    /// Pass a backend option that only affects this crate's own output, such
    /// as `js-comments=off`.
    ///
    /// An option here is per item, and an item is emitted by whichever crate
    /// compiled it, so the sysroot does not have to match. Calls accumulate,
    /// in order.
    #[must_use]
    pub fn crate_arg(mut self, arg: impl Into<String>) -> Self {
        self.crate_args.push(arg.into());
        self
    }

    /// Set a `--cfg` flag for the client crate. Calls accumulate, in order.
    ///
    /// A crate compiled in two passes has its cfgs resolved by the expansion;
    /// one compiled in a single pass has them passed to the backend's rustc.
    #[must_use]
    pub fn cfg(mut self, cfg: impl Into<String>) -> Self {
        self.cfgs.push(cfg.into());
        self
    }

    /// Compile a client crate that uses `#[js_extern]`. Defaults to `false`.
    ///
    /// The backend cannot run a proc macro, so such a crate is compiled in two
    /// passes: plain rustc expands the macros and resolves the cfgs, and the
    /// backend compiles what comes out. The macro itself runs inside rustc and
    /// so is built for the host, which [`render`](Self::render) reports rather
    /// than does: it is the [`ExternMacro`](crate::Step::ExternMacro) step, an
    /// ordinary cargo build in the mires checkout.
    #[must_use]
    pub fn js_extern(mut self, on: bool) -> Self {
        self.js_extern = on;
        self
    }

    /// Expand the client crate through a proc macro the caller built for the
    /// host. Calls accumulate.
    ///
    /// Any macro turns the two-pass compile on, the way
    /// [`js_extern`](Self::js_extern) does. The macro's dylib is checked
    /// against the sources it names before the expansion runs, and both are
    /// watched for reruns.
    #[must_use]
    pub fn proc_macro(mut self, proc_macro: ProcMacro) -> Self {
        self.proc_macros.push(proc_macro);
        self
    }

    /// Compile the `no_std` crate rooted at `src` to an rlib with the backend
    /// first, and make it available to the client crate as `crate_name`.
    /// Calls accumulate, in order, so a later crate can link an earlier one.
    ///
    /// The rlib is built by the same backend against the same sysroot, so the
    /// client and the crate agree about `core` and about each other. It is on
    /// the search path of both passes: an expansion that names it resolves,
    /// and the program links it.
    #[must_use]
    pub fn rlib(mut self, crate_name: impl Into<String>, src: impl Into<PathBuf>) -> Self {
        self.rlibs.push((crate_name.into(), src.into()));
        self
    }

    /// Link the template ABI this checkout ships, as `view_abi`. Defaults to
    /// `false`.
    ///
    /// A convenience for [`rlib`](Self::rlib) pointed at the checkout's own
    /// `view-abi`, which is what an expansion of a client component or island
    /// is written against.
    #[must_use]
    pub fn view_abi(mut self, on: bool) -> Self {
        self.rlibs.retain(|(name, _)| name != "view_abi");
        if on {
            self.rlibs.push(("view_abi".to_owned(), PathBuf::new()));
        }
        self
    }

    /// Whether the client crate goes through the expansion pass.
    fn expands(&self) -> bool {
        self.js_extern || !self.proc_macros.is_empty()
    }

    /// Split the program into one file per island instead of emitting it as
    /// one. Defaults to not splitting.
    ///
    /// [`Chunking`] holds the chunks asked for and how they are named,
    /// published, and served. The chunks land in a directory of their own
    /// inside `OUT_DIR` rather than beside it, and
    /// [`Artifacts::chunks`](Artifacts) names each one:
    ///
    /// ```rust,no_run
    /// use mires_build::{BuildConfig, Chunking};
    ///
    /// BuildConfig::new("client-island/island.rs")
    ///     .output_name("islands.js")
    ///     .chunks(Chunking::new().island("counter").url_base("/demo/chunks/"))
    ///     .render()
    ///     .unwrap();
    /// ```
    ///
    /// A backend that cannot split answers with one file, which every
    /// requested chunk then names: the page still asks for an island by name
    /// and the import map still resolves, so the split is a property of the
    /// backend rather than of the code that serves the result.
    #[must_use]
    pub fn chunks(mut self, chunking: Chunking) -> Self {
        self.chunking = Some(chunking);
        self
    }

    /// Override the mires checkout the backend, sysroot, and scripts come
    /// from. Defaults to the checkout this crate itself lives in, and is
    /// overridden in turn by [`ROOT_ENV`](crate::ROOT_ENV).
    #[must_use]
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Build the backend and the sysroot when they are missing or built with
    /// other options, instead of reporting it. Defaults to `false`.
    ///
    /// Both take minutes, which is why the default is to explain the one time
    /// setup rather than run it. Turn this on for a machine that has no
    /// developer to explain it to.
    #[must_use]
    pub fn bootstrap(mut self, on: bool) -> Self {
        self.bootstrap = on;
        self
    }

    /// Compile the client crate and publish the results into `OUT_DIR`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `OUT_DIR` is unset, if the client crate root, the
    /// backend, or a matching sysroot is missing, if a chunk name cannot be
    /// spelled, if rustc rejects the client crate, or if a file cannot be read
    /// or written.
    pub fn render(self) -> Result<Artifacts> {
        let out_dir = env::var_os("OUT_DIR")
            .map(PathBuf::from)
            .ok_or(BuildError::NoOutDir)?;
        let toolchain = Toolchain::resolve(self.root.clone())?;

        // Before anything that can fail: a build that ends in an error still
        // has to know when to try again.
        let mut rerun = Rerun::new(&self.client_root, &toolchain);
        if self.js_extern {
            rerun = rerun.extern_macro(&toolchain);
        }
        for proc_macro in &self.proc_macros {
            rerun = rerun.proc_macro(proc_macro);
        }
        for (_, src) in self.rlib_sources(&toolchain) {
            rerun = rerun.rlib(&src);
        }
        rerun.emit();

        if !self.client_root.is_file() {
            return Err(BuildError::ClientMissing {
                path: self.client_root,
            });
        }
        if let Some(chunking) = &self.chunking {
            chunking.check()?;
        }
        let emit_args = self.emit_args.join(" ");
        if self.bootstrap {
            toolchain.bootstrap(&emit_args)?;
        }
        toolchain.check(&emit_args)?;

        // The backend names the map, and the `sourceMappingURL` line pointing
        // at it, after the file it is told to write. Staging under the final
        // name is what makes both come out right.
        let staging = out_dir.join(STAGING_DIR);
        fs::create_dir_all(&staging).map_err(|source| BuildError::Io {
            path: staging.clone(),
            source,
        })?;
        let staged = staging.join(&self.output_name);
        let (source, script_args) = self.prepare(&toolchain, &staging, &emit_args)?;
        toolchain.compile(&source, &staged, &self.extra_args(), &script_args)?;

        let (chunks, shared) = self.publish_chunks(&staging, &staged, &out_dir)?;
        self.check_shim_members(&chunks, shared.as_ref())?;
        let import_map = self.publish_import_map(&out_dir, &chunks, shared.as_ref())?;

        let shim = self
            .shim
            .then(|| {
                let published = out_dir.join(SHIM_NAME);
                publish(&toolchain.shim(), &published).map(|()| published)
            })
            .transpose()?;

        Ok(Artifacts {
            chunks,
            shared,
            shim,
            import_map,
        })
    }

    /// Check every `__rt` member the chunks reach for against the module that
    /// supplies them, when the build named one.
    ///
    /// A no-op otherwise, and deliberately: a build using the global-scope
    /// `shim.js` has its members guaranteed by construction, because
    /// `scripts/make-esm-shim.mjs` derives the export clause from the object
    /// the script installs.
    fn check_shim_members(&self, chunks: &[Chunk], shared: Option<&Chunk>) -> Result {
        let Some(module) = &self.shim_module_source else {
            return Ok(());
        };
        let prefix = format!("{SHIM_MODULE_ARG}=");
        let Some(specifier) = self
            .crate_args
            .iter()
            .find_map(|arg| arg.strip_prefix(&prefix))
        else {
            return Ok(());
        };
        crate::shimcheck::check(chunks, shared, specifier, module)
    }

    /// Publish what the compiler wrote in `staging` into `out_dir`, as one
    /// chunk per file emitted.
    ///
    /// A program that was not asked to split is published beside `out_dir` as
    /// it always was, and reads back as a single chunk named after its output
    /// file. A program that was asked to split is published into a directory of
    /// its own, whether or not the backend could split it: a backend that could
    /// not wrote one file, and every requested chunk names it.
    fn publish_chunks(
        &self,
        staging: &Path,
        staged: &Path,
        out_dir: &Path,
    ) -> Result<(Vec<Chunk>, Option<Chunk>)> {
        let Some(chunking) = &self.chunking else {
            let (path, map) = self.publish_file(staged, &out_dir.join(&self.output_name))?;
            let chunk = Chunk {
                name: self.output_stem(),
                path,
                map,
            };
            return Ok((vec![chunk], None));
        };

        let dir = out_dir.join(chunking.directory());
        fs::create_dir_all(&dir).map_err(|source| BuildError::Io {
            path: dir.clone(),
            source,
        })?;

        let split = chunking
            .names()
            .any(|name| chunking.emitted(staging, name).is_file());
        if !split {
            let (path, map) = self.publish_file(staged, &dir.join(&self.output_name))?;
            let mut names: Vec<String> = chunking.names().map(str::to_owned).collect();
            if names.is_empty() {
                names.push(self.output_stem());
            }
            let chunks = names
                .into_iter()
                .map(|name| Chunk {
                    name,
                    path: path.clone(),
                    map: map.clone(),
                })
                .collect();
            return Ok((chunks, None));
        }

        let mut chunks = Vec::with_capacity(chunking.names().count());
        for name in chunking.names() {
            let from = chunking.emitted(staging, name);
            let (path, map) = self.publish_file(&from, &dir.join(format!("{name}.js")))?;
            chunks.push(Chunk {
                name: name.to_owned(),
                path,
                map,
            });
        }

        // The shared chunk exists only when two or more chunks reached a common
        // item, so its absence is an answer rather than a missing output.
        let shared_name = chunking.shared_name();
        let from = chunking.emitted(staging, shared_name);
        let shared = from
            .is_file()
            .then(|| {
                self.publish_file(&from, &dir.join(format!("{shared_name}.js")))
                    .map(|(path, map)| Chunk {
                        name: shared_name.to_owned(),
                        path,
                        map,
                    })
            })
            .transpose()?;

        Ok((chunks, shared))
    }

    /// Publish one emitted file and the source map beside it.
    fn publish_file(&self, from: &Path, to: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
        publish(from, to)?;
        let map = self
            .source_map
            .then(|| {
                let staged = with_map_extension(from);
                let published = with_map_extension(to);
                publish(&staged, &published).map(|()| published)
            })
            .transpose()?;
        Ok((to.to_path_buf(), map))
    }

    /// Write the import map the page resolves the chunks' specifiers with,
    /// when the configuration says where they will be served from.
    fn publish_import_map(
        &self,
        out_dir: &Path,
        chunks: &[Chunk],
        shared: Option<&Chunk>,
    ) -> Result<Option<PathBuf>> {
        let Some(chunking) = &self.chunking else {
            return Ok(None);
        };
        let mut served = chunks.to_vec();
        served.extend(shared.cloned());
        let Some(json) = chunking.import_map(&served) else {
            return Ok(None);
        };

        let path = out_dir.join(format!("{}{IMPORT_MAP_SUFFIX}", self.output_stem()));
        write_if_changed(&path, json.as_bytes())?;
        Ok(Some(path))
    }

    /// The output file's name without its extension, which names the program
    /// wherever a chunk name is wanted and none was asked for.
    fn output_stem(&self) -> String {
        Path::new(&self.output_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&self.output_name)
            .to_owned()
    }

    /// The rlibs to build, with the empty path [`view_abi`](Self::view_abi)
    /// leaves resolved against the checkout.
    fn rlib_sources(&self, toolchain: &Toolchain) -> Vec<(String, PathBuf)> {
        self.rlibs
            .iter()
            .map(|(name, src)| {
                let src = if name == "view_abi" && src.as_os_str().is_empty() {
                    toolchain.view_abi()
                } else {
                    src.clone()
                };
                (name.clone(), src)
            })
            .collect()
    }

    /// The source the backend compiles and the arguments the compile script
    /// takes with it.
    ///
    /// The rlibs the client links are built first, in order. A crate without
    /// macros is then compiled as it is written, with its cfgs. One with
    /// macros is expanded first, against those rlibs and the macro dylibs,
    /// and the expansion is what the backend compiles.
    fn prepare(
        &self,
        toolchain: &Toolchain,
        staging: &Path,
        emit_args: &str,
    ) -> Result<(PathBuf, Vec<String>)> {
        // Each rlib is a crate of its own, compiled by the same backend against
        // the same sysroot, so the expansion and the program agree about it.
        let mut rlibs: Vec<(String, PathBuf)> = Vec::new();
        for (name, src) in self.rlib_sources(toolchain) {
            let rlib = staging.join(format!("lib{name}.rlib"));
            let mut args = vec![
                "--crate-type".to_owned(),
                "rlib".to_owned(),
                "--crate-name".to_owned(),
                name.clone(),
            ];
            args.extend(extern_args(&rlibs));
            toolchain.compile(&src, &rlib, emit_args, &args)?;
            rlibs.push((name, rlib));
        }
        let mut script_args = extern_args(&rlibs);

        if !self.expands() {
            for cfg in &self.cfgs {
                script_args.push("--cfg".to_owned());
                script_args.push(cfg.clone());
            }
            return Ok((self.client_root.clone(), script_args));
        }

        let mut externs = rlibs.clone();
        if self.js_extern {
            toolchain.check_extern_macro()?;
            externs.push(("js_extern_macro".to_owned(), toolchain.js_extern_macro()));
        }
        for proc_macro in &self.proc_macros {
            toolchain.check_proc_macro(proc_macro)?;
            externs.push((
                proc_macro.crate_name().to_owned(),
                proc_macro.dylib().to_path_buf(),
            ));
        }

        // rustc takes the crate name from the file name, so the expansion is
        // named the way a crate can be.
        let stem: String = self
            .output_name
            .chars()
            .map(|character| {
                if character.is_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let expanded = staging.join(format!("{stem}_expanded.rs"));
        toolchain.expand(&self.client_root, &expanded, &self.cfgs, &externs)?;
        Ok((expanded, script_args))
    }

    /// The `JS_EXTRA_ARGS` value for this configuration: the emit-affecting
    /// options, the per crate ones, the chunk request, and the source map
    /// switch.
    ///
    /// The chunk request sits with the per crate options because splitting is a
    /// link step decision, like `js-modules`: it changes no item's own text, so
    /// the sysroot does not have to be built for it.
    fn extra_args(&self) -> String {
        let source_map = self.source_map.then_some("js-source-map=on");
        let chunks = self
            .chunking
            .as_ref()
            .map(Chunking::args)
            .unwrap_or_default();
        let args = self
            .emit_args
            .iter()
            .chain(&self.crate_args)
            .chain(&chunks)
            .map(String::as_str)
            .chain(source_map);

        let mut joined = String::new();
        for arg in args {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(arg);
        }
        joined
    }
}

/// The files [`BuildConfig::render`] published into `OUT_DIR`.
#[derive(Debug, Clone)]
pub struct Artifacts {
    /// Every emitted chunk, in the order it was asked for. A program that was
    /// not split is one chunk named after its output file.
    pub chunks: Vec<Chunk>,
    /// What two or more chunks both reach, emitted once, when the program was
    /// split and there was anything to share.
    pub shared: Option<Chunk>,
    /// The runtime shim, when [`shim`](BuildConfig::shim) is on.
    pub shim: Option<PathBuf>,
    /// The import map resolving every chunk's specifier to its served URL, when
    /// [`Chunking::url_base`] said where that is.
    pub import_map: Option<PathBuf>,
}

impl Artifacts {
    /// The one emitted file, for a program that was not split.
    ///
    /// `None` once there is more than one file, where there is no single
    /// program to name and [`chunks`](Self::chunks) is the answer.
    #[must_use]
    pub fn program(&self) -> Option<&Chunk> {
        match self.chunks.as_slice() {
            [only] if self.shared.is_none() => Some(only),
            _ => None,
        }
    }

    /// The chunk called `name`, which may be the shared one.
    #[must_use]
    pub fn chunk(&self, name: &str) -> Option<&Chunk> {
        self.chunks
            .iter()
            .chain(&self.shared)
            .find(|chunk| chunk.name == name)
    }
}

/// `path` with `.map` appended, the name the backend gives a program's source
/// map.
fn with_map_extension(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".map");
    PathBuf::from(name)
}

/// Copy `from` to `to`, leaving `to` alone when it already holds those bytes.
fn publish(from: &Path, to: &Path) -> Result {
    if !from.is_file() {
        return Err(BuildError::MissingOutput {
            path: from.to_path_buf(),
        });
    }
    let bytes = fs::read(from).map_err(|source| BuildError::Io {
        path: from.to_path_buf(),
        source,
    })?;
    write_if_changed(to, &bytes)
}

/// Write `bytes` to `path` unless they are already there.
///
/// A recompile that produces the same JavaScript leaves the published file's
/// mtime alone, so a watching dev server does not restart the app over output
/// that did not change.
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result {
    let needs_write = match fs::read(path) {
        Ok(existing) => existing != bytes,
        Err(_) => true,
    };
    if needs_write {
        fs::write(path, bytes).map_err(|source| BuildError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// `--extern NAME=PATH` for each rlib, as the compile script takes them.
fn extern_args(rlibs: &[(String, PathBuf)]) -> Vec<String> {
    rlibs
        .iter()
        .flat_map(|(name, path)| {
            [
                "--extern".to_owned(),
                format!("{name}={}", path.display()),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TempDir;

    #[test]
    fn the_default_configuration_asks_for_a_source_map() {
        let config = BuildConfig::new("client/app.rs");
        assert_eq!(config.output_name, DEFAULT_OUTPUT_NAME);
        assert_eq!(config.extra_args(), "js-source-map=on");
    }

    #[test]
    fn extra_args_are_emit_then_crate_then_source_map() {
        let config = BuildConfig::new("client/app.rs")
            .emit_arg("js-minify=on")
            .emit_arg("js-queue=off")
            .crate_arg("js-comments=off");
        assert_eq!(
            config.extra_args(),
            "js-minify=on js-queue=off js-comments=off js-source-map=on"
        );
    }

    #[test]
    fn extra_args_are_empty_without_options_or_a_source_map() {
        let config = BuildConfig::new("client/app.rs").source_map(false);
        assert_eq!(config.extra_args(), "");
        assert_eq!(
            config.crate_arg("js-comments=off").extra_args(),
            "js-comments=off"
        );
    }

    #[test]
    fn only_emit_args_are_matched_against_the_sysroot() {
        let config = BuildConfig::new("client/app.rs")
            .emit_arg("js-minify=on")
            .crate_arg("js-comments=off");
        assert_eq!(config.emit_args.join(" "), "js-minify=on");
    }

    #[test]
    fn the_chunk_request_travels_with_the_per_crate_options() {
        let config = BuildConfig::new("client/app.rs")
            .source_map(false)
            .emit_arg("js-minify=on")
            .crate_arg("js-modules=esm")
            .chunks(Chunking::new().island("counter"));

        assert_eq!(
            config.extra_args(),
            "js-minify=on js-modules=esm js-chunk=counter:__island_counter js-chunk-shared=shared"
        );
        // Splitting is a link step decision, so the sysroot is unaffected.
        assert_eq!(config.emit_args.join(" "), "js-minify=on");
    }

    #[test]
    fn asking_for_no_chunks_compiles_the_options_it_always_did() {
        let config = BuildConfig::new("client/app.rs").chunks(Chunking::new());

        assert_eq!(config.extra_args(), "js-source-map=on");
    }

    #[test]
    fn the_output_stem_names_a_program_no_chunk_was_asked_for() {
        assert_eq!(BuildConfig::new("client/app.rs").output_stem(), "app");
        assert_eq!(
            BuildConfig::new("client/app.rs")
                .output_name("islands.js")
                .output_stem(),
            "islands"
        );
        assert_eq!(
            BuildConfig::new("client/app.rs")
                .output_name("islands")
                .output_stem(),
            "islands"
        );
    }

    /// A staging directory holding a compiled program and, optionally, the
    /// chunks a splitting backend wrote beside it.
    fn staged(dir: &Path, files: &[&str]) -> PathBuf {
        let staging = dir.join(".mires-build");
        fs::create_dir_all(&staging).unwrap();
        for name in files {
            fs::write(staging.join(name), format!("// {name}\n")).unwrap();
        }
        staging.join("islands.js")
    }

    #[test]
    fn a_program_no_chunk_was_asked_for_is_published_beside_out_dir() {
        let dir = TempDir::new("chunks-none");
        let out = dir.path();
        let program = staged(out, &["islands.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false);

        let (chunks, shared) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        assert_eq!(
            chunks,
            [Chunk {
                name: "islands".to_owned(),
                path: out.join("islands.js"),
                map: None,
            }]
        );
        assert_eq!(shared, None);
    }

    #[test]
    fn a_source_map_is_published_beside_its_chunk() {
        let dir = TempDir::new("chunks-map");
        let out = dir.path();
        let program = staged(out, &["islands.js", "islands.js.map"]);
        let config = BuildConfig::new("client/app.rs").output_name("islands.js");

        let (chunks, _) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        assert_eq!(
            chunks[0].map.as_deref(),
            Some(out.join("islands.js.map")).as_deref()
        );
    }

    #[test]
    fn a_backend_that_cannot_split_answers_with_one_file_for_every_chunk() {
        let dir = TempDir::new("chunks-unsplit");
        let out = dir.path();
        let program = staged(out, &["islands.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false)
            .chunks(
                Chunking::new()
                    .island("counter")
                    .island("search")
                    .url_base("/demo/chunks/"),
            );

        let (chunks, shared) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        // One file, in the chunk directory, named by both chunks: the layout and
        // the URLs are the ones a split program has.
        let path = out.join("chunks").join("islands.js");
        assert!(path.is_file());
        assert_eq!(
            chunks,
            [
                Chunk {
                    name: "counter".to_owned(),
                    path: path.clone(),
                    map: None,
                },
                Chunk {
                    name: "search".to_owned(),
                    path,
                    map: None,
                },
            ]
        );
        assert_eq!(shared, None);
    }

    #[test]
    fn each_chunk_the_backend_wrote_is_published_under_its_own_name() {
        let dir = TempDir::new("chunks-split");
        let out = dir.path();
        let program = staged(out, &["islands.js", "counter.js", "search.js", "shared.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false)
            .chunks(Chunking::new().island("counter").island("search"));

        let (chunks, shared) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        let dir = out.join("chunks");
        assert_eq!(
            chunks,
            [
                Chunk {
                    name: "counter".to_owned(),
                    path: dir.join("counter.js"),
                    map: None,
                },
                Chunk {
                    name: "search".to_owned(),
                    path: dir.join("search.js"),
                    map: None,
                },
            ]
        );
        assert_eq!(
            shared,
            Some(Chunk {
                name: "shared".to_owned(),
                path: dir.join("shared.js"),
                map: None,
            })
        );
        assert_eq!(
            fs::read(dir.join("counter.js")).unwrap(),
            b"// counter.js\n"
        );
    }

    #[test]
    fn nothing_two_chunks_share_means_no_shared_chunk() {
        let dir = TempDir::new("chunks-unshared");
        let out = dir.path();
        let program = staged(out, &["islands.js", "counter.js", "search.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false)
            .chunks(Chunking::new().island("counter").island("search"));

        let (_, shared) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        assert_eq!(shared, None);
    }

    #[test]
    fn a_chunk_the_backend_left_out_is_a_missing_output() {
        let dir = TempDir::new("chunks-missing");
        let out = dir.path();
        let program = staged(out, &["islands.js", "counter.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false)
            .chunks(Chunking::new().island("counter").island("search"));

        let error = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap_err();

        assert!(matches!(error, BuildError::MissingOutput { .. }), "{error}");
    }

    #[test]
    fn the_import_map_is_written_beside_the_program() {
        let dir = TempDir::new("chunks-import-map");
        let out = dir.path();
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .chunks(
                Chunking::new()
                    .island("counter")
                    .url_base("/demo/chunks/")
                    .import("mires-dom", "/demo/mires-dom.js"),
            );
        let chunks = [Chunk {
            name: "counter".to_owned(),
            path: out.join("chunks").join("counter.js"),
            map: None,
        }];

        let path = config
            .publish_import_map(out, &chunks, None)
            .unwrap()
            .unwrap();

        assert_eq!(path, out.join("islands.importmap.json"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            concat!(
                r#"{"imports":{"mires-dom":"/demo/mires-dom.js","#,
                r#""mires-island/counter":"/demo/chunks/counter.js"}}"#,
            )
        );
    }

    #[test]
    fn the_shared_chunk_is_in_the_import_map_too() {
        let dir = TempDir::new("chunks-import-map-shared");
        let out = dir.path();
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .chunks(Chunking::new().island("counter").url_base("/demo/chunks/"));
        let chunks = [Chunk {
            name: "counter".to_owned(),
            path: out.join("chunks").join("counter.js"),
            map: None,
        }];
        let shared = Chunk {
            name: "shared".to_owned(),
            path: out.join("chunks").join("shared.js"),
            map: None,
        };

        let path = config
            .publish_import_map(out, &chunks, Some(&shared))
            .unwrap()
            .unwrap();

        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains(r#""mires-island/shared":"/demo/chunks/shared.js""#));
    }

    #[test]
    fn there_is_no_import_map_without_a_chunk_request() {
        let dir = TempDir::new("chunks-no-import-map");
        let out = dir.path();
        let config = BuildConfig::new("client/app.rs").output_name("islands.js");

        assert_eq!(config.publish_import_map(out, &[], None).unwrap(), None);
    }

    #[test]
    fn a_single_chunk_is_the_program_and_several_are_not() {
        let one = Chunk {
            name: "islands".to_owned(),
            path: PathBuf::from("/out/islands.js"),
            map: None,
        };
        let artifacts = Artifacts {
            chunks: vec![one.clone()],
            shared: None,
            shim: None,
            import_map: None,
        };

        assert_eq!(artifacts.program(), Some(&one));
        assert_eq!(artifacts.chunk("islands"), Some(&one));
        assert_eq!(artifacts.chunk("counter"), None);

        let split = Artifacts {
            chunks: vec![one.clone()],
            shared: Some(one),
            shim: None,
            import_map: None,
        };

        assert_eq!(split.program(), None);
        assert_eq!(
            split.chunk("islands").map(|chunk| &chunk.name),
            Some(&"islands".to_owned())
        );
    }

    #[test]
    fn a_source_map_is_named_after_its_program() {
        assert_eq!(
            with_map_extension(Path::new("/out/app.js")),
            Path::new("/out/app.js.map")
        );
    }

    #[test]
    fn writing_creates_a_missing_file() {
        let dir = TempDir::new("write-create");
        let path = dir.path().join("app.js");

        write_if_changed(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
    }

    #[test]
    fn writing_the_same_bytes_leaves_the_file_alone() {
        let dir = TempDir::new("write-unchanged");
        let path = dir.path().join("app.js");
        write_if_changed(&path, b"one").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        write_if_changed(&path, b"one").unwrap();

        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn writing_other_bytes_replaces_the_file() {
        let dir = TempDir::new("write-changed");
        let path = dir.path().join("app.js");
        write_if_changed(&path, b"one").unwrap();

        write_if_changed(&path, b"two").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn publishing_reports_a_missing_source() {
        let dir = TempDir::new("publish-missing");
        let from = dir.path().join("app.js.map");

        let error = publish(&from, &dir.path().join("out.js.map")).unwrap_err();

        assert!(matches!(error, BuildError::MissingOutput { path } if path == from));
    }

    #[test]
    fn publishing_copies_the_source() {
        let dir = TempDir::new("publish-copy");
        let from = dir.path().join("staged.js");
        let to = dir.path().join("app.js");
        fs::write(&from, b"program").unwrap();

        publish(&from, &to).unwrap();

        assert_eq!(fs::read(&to).unwrap(), b"program");
    }
}
