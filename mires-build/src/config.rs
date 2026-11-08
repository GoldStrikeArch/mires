use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use rcgjs_object::artifact::{self, ArtifactManifest, EntryRole};

use crate::{shimcheck::SHIM_MODULE_ARG, BuildError, Chunk, Chunking, Rerun, Result, Toolchain};

/// File name of the default [`output_name`](BuildConfig::output_name) inside
/// `OUT_DIR`.
pub const DEFAULT_OUTPUT_NAME: &str = "app.js";
/// File name of the runtime shim copied into `OUT_DIR`.
pub const SHIM_NAME: &str = "shim.js";
/// Suffix of the import map written beside the program, after its
/// [`output_name`](BuildConfig::output_name) without the extension.
pub const IMPORT_MAP_SUFFIX: &str = ".importmap.json";
/// Suffix of the optimization report written beside the program.
pub const OPTIMIZATION_REPORT_SUFFIX: &str = ".opt.json";
/// Directory inside `OUT_DIR` the compiler writes to before publishing.
const STAGING_DIR: &str = ".mires-build";
/// Space-free report name passed through rustc's whitespace-split backend option channel.
const STAGED_OPTIMIZATION_REPORT_NAME: &str = "optimization-report.json";

/// Whether the backend optimizer is disabled, analyzed only, or applied.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum OptimizerMode {
    /// Preserve the current compiler output.
    #[default]
    Off,
    /// Run analyses and report decisions without changing compiler output.
    Report,
    /// Emit optimizations whose safety proof succeeds.
    On,
}

impl OptimizerMode {
    fn backend_arg(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Report => Some("js-opt=report"),
            Self::On => Some("js-opt=on"),
        }
    }
}

/// One optimizer pass selectable by the developer attribution controls.
///
/// This is an experimental compiler interface and may change without notice.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum OptimizerPass {
    Sroa,
    AstDce,
    FuturePoller,
    NativeRepresentations,
    LinkDedup,
}

impl OptimizerPass {
    /// Every registered pass in compiler execution order. Mirrors the backend's
    /// `OptimizerPass::ORDERED`.
    const ORDERED: [Self; 5] = [
        Self::Sroa,
        Self::AstDce,
        Self::FuturePoller,
        Self::NativeRepresentations,
        Self::LinkDedup,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Sroa => "sroa",
            Self::AstDce => "ast-dce",
            Self::FuturePoller => "future-poller",
            Self::NativeRepresentations => "native-representations",
            Self::LinkDedup => "link-dedup",
        }
    }
}

/// Which lowering the link step prints an item from. Mirrors the backend's `js-lane`.
///
/// Every object carries both v1's printed baseline JavaScript and the JIR-S body it was encoded
/// from, so this is a link step choice between two available spellings rather than a mode the
/// compilation commits to — which is why it is not matched against the sysroot. The lanes
/// themselves land in a later phase; a non-baseline lane compiles today and warns that the
/// baseline is what ran.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum Lane {
    /// The frozen v1 text, item for item.
    #[default]
    Baseline,
    /// Print the baseline, and the JIR lane beside it, for differential comparison.
    Shadow,
    /// Print from JIR-S wherever the item's class has been promoted.
    Jir,
}

impl Lane {
    fn backend_arg(self) -> Option<&'static str> {
        match self {
            Self::Baseline => None,
            Self::Shadow => Some("js-lane=shadow"),
            Self::Jir => Some("js-lane=jir"),
        }
    }
}

/// A named bundle of compiler settings. Mirrors the backend's `js-profile`.
///
/// A profile is an alias for the three or four options a build actually wants together, so that
/// the combination has one spelling that cannot drift. It is expanded here into the individual
/// options for the sysroot key. The client link also receives `js-profile=` as policy identity:
/// size and balanced deliberately share object bytes, but their carrier economics differ.
///
/// A setting spelled out alongside the profile wins, however it was spelled — [`emit_arg`] and
/// the typed setters are applied after the expansion, and the backend reads the last value given
/// for an option.
///
/// [`emit_arg`]: BuildConfig::emit_arg
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Profile {
    /// The fastest compile, and the output a golden was taken from.
    Dev,
    /// Analyses run and are reported; nothing is rewritten.
    Report,
    /// Compiler-side size policy.
    ReleaseSize,
    /// The default release policy, runtime first.
    ReleaseBalanced,
    /// Item names kept, so a CPU profile of the result still reads.
    ReleaseSpeed,
}

impl Profile {
    fn backend_arg(self) -> &'static str {
        match self {
            Self::Dev => "js-profile=dev",
            Self::Report => "js-profile=report",
            Self::ReleaseSize => "js-profile=release-size",
            Self::ReleaseBalanced => "js-profile=release-balanced",
            Self::ReleaseSpeed => "js-profile=release-speed",
        }
    }

    /// The emit-affecting options this profile names, in the order the backend reads them.
    ///
    /// These are what the sysroot is keyed on, so this list and
    /// `scripts/sysroot-cache.sh`'s `jsc_profile_emit_args` have to agree exactly: a build
    /// driven from a build script and the same build driven from `$JS_EXTRA_ARGS` must land on
    /// one cache entry.
    fn emit_args(self) -> &'static [&'static str] {
        match self {
            Self::Dev => &["js-opt=off"],
            Self::Report => &["js-opt=report"],
            Self::ReleaseSize | Self::ReleaseBalanced => &[
                "js-cleanup=on",
                "js-ecma-target=2018",
                "js-minify=on",
                "js-opt=on",
            ],
            Self::ReleaseSpeed => &[
                "js-cleanup=on",
                "js-ecma-target=2018",
                "js-minify=locals",
                "js-opt=on",
            ],
        }
    }

    /// The lane this profile selects. Link-only, so it never reaches the sysroot key.
    fn lane(self) -> Lane {
        match self {
            Self::Dev | Self::Report => Lane::Baseline,
            Self::ReleaseSize | Self::ReleaseBalanced | Self::ReleaseSpeed => Lane::Jir,
        }
    }
}

/// A rustc optimization level applied consistently to the client and its JavaScript sysroot.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RustOptLevel {
    O0,
    O1,
    O2,
    O3,
    Size,
    SizeMin,
}

impl RustOptLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::O0 => "0",
            Self::O1 => "1",
            Self::O2 => "2",
            Self::O3 => "3",
            Self::Size => "s",
            Self::SizeMin => "z",
        }
    }
}

/// Builder for a client crate compiled to JavaScript from a Cargo build
/// script.
///
/// The default configuration compiles the crate rooted at the given file with
/// the backend and sysroot already built in the spike checkout, and publishes
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
    minimal_shim: bool,
    emit_args: Vec<String>,
    crate_args: Vec<String>,
    cfgs: Vec<String>,
    views: bool,
    keyed_views: bool,
    procedures: bool,
    chunking: Option<Chunking>,
    spike_root: Option<PathBuf>,
    bootstrap: bool,
    shim_module_source: Option<PathBuf>,
    optimizer: OptimizerMode,
    optimizer_passes: Option<Vec<OptimizerPass>>,
    optimization_warnings: bool,
    optimization_report: bool,
    rust_opt_level: Option<RustOptLevel>,
    workspace_root: Option<PathBuf>,
    profile: Option<Profile>,
    lane: Option<Lane>,
    jir: bool,
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
            minimal_shim: false,
            emit_args: Vec::new(),
            crate_args: Vec::new(),
            cfgs: Vec::new(),
            views: false,
            keyed_views: false,
            procedures: false,
            chunking: None,
            spike_root: None,
            bootstrap: false,
            shim_module_source: None,
            optimizer: OptimizerMode::Off,
            optimizer_passes: None,
            optimization_warnings: true,
            optimization_report: false,
            rust_opt_level: None,
            workspace_root: env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from),
            profile: None,
            lane: None,
            jir: true,
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

    /// Publish the runtime shim beside the entries that import it. Defaults to `true`.
    ///
    /// The manifest determines whether the provider is a script or an ES module. Disable this
    /// when the application supplies an external provider through its own import map.
    #[must_use]
    pub fn shim(mut self, on: bool) -> Self {
        self.shim = on;
        self
    }

    /// Publish only runtime members transitively reached by the emitted program.
    ///
    /// Defaults to `false`, preserving the complete development shim. When enabled, the build
    /// runs `scripts/minimal-shim.mjs` after linking and publishes its closed subset instead.
    #[must_use]
    pub fn minimal_shim(mut self, on: bool) -> Self {
        self.minimal_shim = on;
        self
    }

    /// Select a named bundle of compiler settings. See [`Profile`].
    ///
    /// The profile is expanded before every other setting on this builder, so anything spelled
    /// out as well — [`optimizer`](Self::optimizer), [`lane`](Self::lane), an
    /// [`emit_arg`](Self::emit_arg) — overrides the profile's choice for that one option and
    /// leaves the rest of it in place.
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Select which lane the link step prints from. See [`Lane`].
    ///
    /// Link-only: the sysroot is keyed on what an object *contains*, and both lanes travel in
    /// every object, so changing this never rebuilds one.
    #[must_use]
    pub fn lane(mut self, lane: Lane) -> Self {
        self.lane = Some(lane);
        self
    }

    /// Declare whether the compiled entry runs as a main module, worker, island, or procedure.
    #[must_use]
    pub fn entry_role(self, role: EntryRole) -> Self {
        self.crate_arg(format!("js-entry-role={}", role.as_str()))
    }

    /// Carry the JIR-S body of each item in the emitted objects. Defaults to `true`.
    ///
    /// Emit-affecting, and the one lane option that is: it decides what an object file holds, so
    /// the sysroot's `core` and the crate linking it have to agree. Turning it off produces
    /// v1-shaped objects, which is how a question about the encoder or about object size is
    /// bisected.
    #[must_use]
    pub fn jir(mut self, on: bool) -> Self {
        self.jir = on;
        self
    }

    /// Select the backend optimizer mode. Defaults to [`OptimizerMode::Off`].
    #[must_use]
    pub fn optimizer(mut self, mode: OptimizerMode) -> Self {
        self.optimizer = mode;
        self
    }

    /// Restrict optimizer passes for compiler attribution runs.
    ///
    /// The order supplied here is ignored. The compiler always uses its fixed pass order.
    #[doc(hidden)]
    #[must_use]
    pub fn optimizer_passes(mut self, passes: impl IntoIterator<Item = OptimizerPass>) -> Self {
        let mut passes: Vec<_> = passes.into_iter().collect();
        passes.sort_unstable();
        passes.dedup();
        self.optimizer_passes = Some(passes);
        self
    }

    /// Warn for reachable optimization misses in workspace source. Defaults to `true`.
    #[must_use]
    pub fn optimization_warnings(mut self, on: bool) -> Self {
        self.optimization_warnings = on;
        self
    }

    /// Publish a deterministic size and optimization report beside the program.
    #[must_use]
    pub fn optimization_report(mut self, on: bool) -> Self {
        self.optimization_report = on;
        self
    }

    /// Set rustc's optimization level for the client and its JavaScript sysroot.
    ///
    /// The default leaves the current build profiles unchanged.
    #[must_use]
    pub fn rust_opt_level(mut self, level: RustOptLevel) -> Self {
        self.rust_opt_level = Some(level);
        self
    }

    /// Set the source root whose rejected optimizer candidates may produce warnings.
    #[must_use]
    pub fn workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
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
    #[must_use]
    pub fn cfg(mut self, cfg: impl Into<String>) -> Self {
        self.cfgs.push(cfg.into());
        self
    }

    /// Compile a client crate that uses the view macros. Defaults to `false`.
    ///
    /// The host proc macros run in the same rustc invocation as the backend,
    /// preserving the original crate's spans for diagnostics and source maps.
    /// The template ABI their client expansion names is built as an rlib by
    /// the same backend first, so the crate has one `core` and one ABI rather
    /// than two of either.
    ///
    /// The macros themselves run inside rustc and so are built for the host,
    /// which [`render`](Self::render) reports rather than does: it is an
    /// ordinary cargo build in the spike checkout.
    #[must_use]
    pub fn views(mut self, on: bool) -> Self {
        self.views = on;
        self
    }

    /// Use the transitional keyed-list lowering for keyed `view!` loops.
    ///
    /// Defaults to `false`, so [`views`](Self::views) keeps expanding through
    /// the legacy `push_keyed` compatibility lane. Enabling this also enables
    /// views and selects a separately built proc-macro artifact; it cannot
    /// change the expansion used by another build in the same checkout.
    #[must_use]
    pub fn keyed_views(mut self, on: bool) -> Self {
        self.keyed_views = on;
        self.views |= on;
        self
    }

    /// Compile a client crate that calls functions declared with
    /// `#[topcoat_runtime_macro::procedure]` and may spawn their futures.
    /// Defaults to `false`.
    ///
    /// Procedure clients compile the host macro and target crate in one rustc
    /// invocation, plus the packaged client-side `topcoat-dom` seam and async
    /// executor. The framework proc macro is run unchanged; this option only
    /// supplies the target crates its client expansion names.
    #[must_use]
    pub fn procedures(mut self, on: bool) -> Self {
        self.procedures = on;
        self
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

    /// Override the spike checkout the backend, sysroot, and scripts come
    /// from. Defaults to the checkout this crate itself lives in, and is
    /// overridden in turn by [`SPIKE_ROOT_ENV`](crate::SPIKE_ROOT_ENV).
    #[must_use]
    pub fn spike_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.spike_root = Some(root.into());
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
        let toolchain = Toolchain::resolve(self.spike_root.clone())?;
        self.check_report_shape()?;

        // Before anything that can fail: a build that ends in an error still
        // has to know when to try again.
        let emit_args = self.emit_arguments();
        let rerun = Rerun::new(&self.client_root, &toolchain);
        let rerun = if self.views {
            rerun.views_with_keyed(&toolchain, self.keyed_views)
        } else {
            rerun
        };
        let rerun = if self.procedures {
            rerun.procedures(&toolchain)
        } else {
            rerun
        };
        rerun.emit();

        if !self.client_root.is_file() {
            return Err(BuildError::ClientMissing {
                path: self.client_root,
            });
        }
        if let Some(chunking) = &self.chunking {
            chunking.check()?;
        }
        if self.bootstrap {
            toolchain.bootstrap_with_opt_level(&emit_args, self.rust_opt_level)?;
        }
        toolchain.check_with_opt_level(&emit_args, self.rust_opt_level)?;
        Rerun::selected_sysroot(&toolchain, &emit_args, self.rust_opt_level)?.emit_paths();

        // The backend names the map, and the `sourceMappingURL` line pointing
        // at it, after the file it is told to write. Staging under the final
        // name is what makes both come out right.
        let staging = out_dir.join(STAGING_DIR);
        fs::create_dir_all(&staging).map_err(|source| BuildError::Io {
            path: staging.clone(),
            source,
        })?;
        let staged = staging.join(&self.output_name);
        let staged_report = self
            .optimization_report
            .then(|| staging.join(STAGED_OPTIMIZATION_REPORT_NAME));
        let (source, script_args) = self.prepare(&toolchain, &staging, &emit_args)?;
        let workspace_root = self.backend_workspace_root(&toolchain);
        toolchain.compile_with_settings(
            &source,
            &staged,
            &self.extra_args_for(
                staged_report
                    .as_ref()
                    .map(|_| Path::new(STAGED_OPTIMIZATION_REPORT_NAME)),
                workspace_root.as_deref(),
            ),
            &emit_args,
            &script_args,
            self.rust_opt_level,
        )?;

        let (chunks, shared, artifact_manifest) =
            self.publish_chunks(&staging, &staged, &out_dir)?;
        self.check_shim_members(&chunks, shared.as_ref())?;
        let import_map = self.publish_import_map(&out_dir, &chunks, shared.as_ref())?;

        let shim = self.publish_shim(&toolchain, &artifact_manifest)?;
        let optimization_report = staged_report
            .map(|staged| {
                let published =
                    out_dir.join(format!("{}{OPTIMIZATION_REPORT_SUFFIX}", self.output_name));
                publish(&staged, &published).map(|()| published)
            })
            .transpose()?;

        Ok(Artifacts {
            chunks,
            shared,
            shim,
            import_map,
            optimization_report,
            artifact_manifest,
        })
    }

    /// Refuses a report whose byte totals would describe the discarded unsplit staging program.
    fn check_report_shape(&self) -> Result {
        if self.optimization_report && self.chunking.is_some() {
            return Err(BuildError::ChunkedOptimizationReport);
        }
        Ok(())
    }

    /// Generate the runtime at the location imported by the published entry graph, then
    /// bind that provider's bytes into the manifest.
    fn publish_shim(&self, toolchain: &Toolchain, manifest_path: &Path) -> Result<Option<PathBuf>> {
        if !self.shim {
            return Ok(None);
        }
        let io = |source| BuildError::Io {
            path: manifest_path.to_path_buf(),
            source,
        };
        let manifest = ArtifactManifest::read(manifest_path).map_err(io)?;
        let directory = manifest_path.parent().unwrap();
        manifest.verify(directory).map_err(io)?;
        let requirements = manifest
            .artifacts
            .values()
            .filter_map(|artifact| artifact.runtime.as_ref())
            .collect::<Vec<_>>();
        if requirements.is_empty() {
            return Ok(None);
        }
        let modules = requirements
            .iter()
            .map(|runtime| runtime.module.clone())
            .collect::<BTreeSet<_>>();
        if modules.len() != 1 {
            return Err(io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "one runtime publication cannot supply different entry providers",
            )));
        }
        let module = modules.first().unwrap();
        let name = match module {
            None => SHIM_NAME,
            Some(module) => module.strip_prefix("./").filter(|name| {
                !name.is_empty() && !name.contains('\\')
                    && Path::new(name).components().all(|part| matches!(part, std::path::Component::Normal(_)))
            }).ok_or_else(|| io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "generated runtime needs a local module specifier; use shim(false) for an external provider")))?,
        };
        let published = directory.join(name);
        fs::create_dir_all(published.parent().unwrap()).map_err(io)?;
        if manifest.artifacts.contains_key(name) {
            return Err(io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "runtime provider collides with a compiler artifact",
            )));
        }
        toolchain.runtime_from_manifest(
            manifest_path,
            &published,
            module.is_some(),
            self.minimal_shim,
        )?;
        let manifest = ArtifactManifest::read(manifest_path).map_err(io)?;
        manifest.verify(directory).map_err(io)?;
        Ok(Some(published))
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

    /// Publish exactly the entry graph the linker recorded, after verifying its bytes.
    fn publish_chunks(
        &self,
        staging: &Path,
        staged: &Path,
        out_dir: &Path,
    ) -> Result<(Vec<Chunk>, Option<Chunk>, PathBuf)> {
        let manifest_path = artifact::manifest_path(staged);
        let io = |source| BuildError::Io {
            path: manifest_path.clone(),
            source,
        };
        let mut manifest = ArtifactManifest::read(&manifest_path).map_err(io)?;
        manifest.verify(staging).map_err(io)?;
        let chunking = self
            .chunking
            .as_ref()
            .filter(|chunking| !chunking.is_empty());
        let directory = chunking.map_or_else(
            || out_dir.to_path_buf(),
            |chunking| out_dir.join(chunking.directory()),
        );
        fs::create_dir_all(&directory).map_err(|source| BuildError::Io {
            path: directory.clone(),
            source,
        })?;
        let requested = chunking.map_or_else(
            || vec!["program".to_owned()],
            |chunking| chunking.names().map(str::to_owned).collect::<Vec<_>>(),
        );
        let mut selected = requested.iter().cloned().collect::<BTreeSet<_>>();
        let shared_name = chunking.and_then(|chunking| {
            manifest
                .entries
                .get(chunking.shared_name())
                .filter(|entry| entry.role == EntryRole::Shared)
                .map(|_| chunking.shared_name().to_owned())
        });
        selected.extend(shared_name.iter().cloned());
        if manifest
            .entries
            .keys()
            .any(|entry| !selected.contains(entry))
        {
            return Err(io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "linker manifest contains an unrequested entry",
            )));
        }
        let mut needed = BTreeSet::new();
        for entry in &selected {
            if !manifest.entries.contains_key(entry) {
                return Err(BuildError::MissingOutput {
                    path: staging.join(format!("{entry}.js")),
                });
            }
            needed.extend(manifest.reachable(entry).map_err(io)?);
        }
        // The compiler also retains an unsplit inspection program when it emits chunks.
        // That auxiliary program is not in the deployed entry closure.
        manifest.artifacts.retain(|name, _| needed.contains(name));
        for name in &needed {
            publish(&staging.join(name), &directory.join(name))?;
        }
        let chunk = |entry_name: &str, public_name: String| {
            let entry = &manifest.entries[entry_name];
            let program = &manifest.artifacts[&entry.program];
            Chunk {
                name: public_name,
                path: directory.join(&entry.program),
                map: program.source_map.as_ref().map(|name| directory.join(name)),
            }
        };
        let chunks = requested
            .iter()
            .map(|name| {
                let public_name = if chunking.is_none() {
                    self.output_stem()
                } else {
                    name.clone()
                };
                chunk(name, public_name)
            })
            .collect();
        let shared = shared_name.as_ref().map(|name| chunk(name, name.clone()));
        let published_manifest = directory.join(manifest_path.file_name().unwrap());
        manifest.write(&published_manifest).map_err(io)?;
        manifest.verify(&directory).map_err(io)?;
        Ok((chunks, shared, published_manifest))
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

    /// The source the backend compiles and the arguments the compile script
    /// takes with it.
    ///
    /// View and procedure support crates are built first, then the original
    /// client source is compiled with their host proc macros and target rlibs
    /// in one rustc invocation. Keeping the original source as rustc's input is
    /// what lets source maps reach the author's files instead of a staging
    /// expansion.
    fn prepare(
        &self,
        toolchain: &Toolchain,
        staging: &Path,
        emit_args: &str,
    ) -> Result<(PathBuf, Vec<String>)> {
        if !self.views && !self.procedures {
            return Ok((self.client_root.clone(), Vec::new()));
        }
        if self.views {
            toolchain.check_view_macros_with_keyed(self.keyed_views)?;
        }
        if self.procedures {
            toolchain.check_procedure_support()?;
        }

        // The ABI is a crate of its own, compiled by the same backend against
        // the same sysroot, so the proc-macro output and the program agree
        // about it.
        let view_abi = staging.join("libview_abi.rlib");
        toolchain.compile_with_opt_level(
            &toolchain.view_abi(),
            &view_abi,
            emit_args,
            &[
                "--crate-type".to_owned(),
                "rlib".to_owned(),
                "--crate-name".to_owned(),
                "view_abi".to_owned(),
            ],
            self.rust_opt_level,
        )?;

        let mut compiled_externs = vec![
            "--extern".to_owned(),
            format!("view_abi={}", view_abi.display()),
        ];
        if self.views {
            compiled_externs.extend([
                "--extern".to_owned(),
                format!(
                    "view_dom_macro={}",
                    toolchain.view_macros_with_keyed(self.keyed_views).display()
                ),
                "--extern".to_owned(),
                format!(
                    "js_extern_macro={}",
                    toolchain
                        .js_extern_macro_with_keyed(self.keyed_views)
                        .display()
                ),
            ]);
        }
        if self.procedures {
            let view_async = staging.join("libview_async.rlib");
            toolchain.compile_with_opt_level(
                &toolchain.view_async(),
                &view_async,
                emit_args,
                &[
                    "--crate-type".to_owned(),
                    "rlib".to_owned(),
                    "--crate-name".to_owned(),
                    "view_async".to_owned(),
                    "--extern".to_owned(),
                    format!("view_abi={}", view_abi.display()),
                ],
                self.rust_opt_level,
            )?;

            // Keep the proc-macro search path free of host rlibs. rustc may
            // resolve the transitive `js_macro` dependency by file name while
            // loading `topcoat_dom` metadata, and a host `view_abi` beside it
            // would be offered for the wasm target.
            let macro_dir = staging.join("procedure-macros");
            fs::create_dir_all(&macro_dir).map_err(|source| BuildError::Io {
                path: macro_dir.clone(),
                source,
            })?;
            let js_macro = toolchain.js_macro();
            let js_macro_copy = macro_dir.join(
                js_macro
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("js-macro")),
            );
            publish(&js_macro, &js_macro_copy)?;
            let topcoat_dom = staging.join("libtopcoat_dom.rlib");
            toolchain.compile_with_opt_level(
                &toolchain.topcoat_dom(),
                &topcoat_dom,
                emit_args,
                &[
                    "--crate-type".to_owned(),
                    "rlib".to_owned(),
                    "--crate-name".to_owned(),
                    "topcoat_dom".to_owned(),
                    "--search".to_owned(),
                    macro_dir.display().to_string(),
                    "--extern".to_owned(),
                    format!("view_abi={}", view_abi.display()),
                    "--extern".to_owned(),
                    format!("view_async={}", view_async.display()),
                    "--extern".to_owned(),
                    format!("js_macro={}", js_macro.display()),
                ],
                self.rust_opt_level,
            )?;

            compiled_externs.extend([
                "--search".to_owned(),
                staging.display().to_string(),
                "--search".to_owned(),
                macro_dir.display().to_string(),
                "--extern".to_owned(),
                format!("view_async={}", view_async.display()),
                "--extern".to_owned(),
                format!("topcoat_dom={}", topcoat_dom.display()),
                "--extern".to_owned(),
                format!("js_macro={}", js_macro.display()),
                "--extern".to_owned(),
                format!(
                    "topcoat_runtime_macro={}",
                    toolchain.procedure_macro().display()
                ),
            ]);
        }
        for cfg in &self.cfgs {
            compiled_externs.extend(["--cfg".to_owned(), cfg.clone()]);
        }

        Ok((self.client_root.clone(), compiled_externs))
    }

    /// The `JS_EXTRA_ARGS` value for this configuration: the emit-affecting
    /// options, the per crate ones, the chunk request, and the source map
    /// switch.
    ///
    /// The chunk request sits with the per crate options because splitting is a
    /// link step decision, like `js-modules`: it changes no item's own text, so
    /// the sysroot does not have to be built for it.
    #[cfg(test)]
    fn extra_args(&self) -> String {
        self.extra_args_for(None, None)
    }

    fn emit_arguments(&self) -> String {
        let mut args = self.profile_emit_args();
        if !self.jir {
            args.push("js-jir=off".to_owned());
        }
        args.extend(self.emit_args.iter().cloned());
        if let Some(arg) = self.optimizer.backend_arg() {
            args.push(arg.to_owned());
        }
        if let Some(arg) = self.optimizer_passes_arg() {
            args.push(arg);
        }
        args.join(" ")
    }

    /// The profile's emit-affecting options, first, so that everything else overrides them.
    fn profile_emit_args(&self) -> Vec<String> {
        self.profile
            .map(Profile::emit_args)
            .unwrap_or_default()
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect()
    }

    /// The lane that actually runs: the explicit one, else the profile's, else the baseline.
    fn selected_lane(&self) -> Lane {
        self.lane
            .or_else(|| self.profile.map(Profile::lane))
            .unwrap_or_default()
    }

    fn optimizer_passes_arg(&self) -> Option<String> {
        let passes = self.optimizer_passes.as_ref()?;
        let value = match passes.as_slice() {
            [] => "none".to_owned(),
            selected if selected == OptimizerPass::ORDERED => "all".to_owned(),
            _ => passes
                .iter()
                .map(|pass| pass.as_str())
                .collect::<Vec<_>>()
                .join(","),
        };
        Some(format!("js-opt-passes={value}"))
    }

    fn extra_args_for(&self, report: Option<&Path>, workspace_root: Option<&Path>) -> String {
        let source_map = self.source_map.then_some("js-source-map=on");
        let warnings = (!self.optimization_warnings).then_some("js-opt-warnings=off");
        let report = report.map(|path| format!("js-opt-report={}", path.display()));
        let workspace_root = (self.optimizer != OptimizerMode::Off)
            .then(|| workspace_root.map(|path| format!("js-workspace-root={}", path.display())))
            .flatten();
        let optimizer_passes = self.optimizer_passes_arg();
        let chunks = self
            .chunking
            .as_ref()
            .map(Chunking::args)
            .unwrap_or_default();
        let profile_args = self.profile_emit_args();
        let profile = self.profile.map(Profile::backend_arg);
        let jir = (!self.jir).then_some("js-jir=off");
        let lane = self.selected_lane().backend_arg();
        let args = profile_args
            .iter()
            .map(String::as_str)
            .chain(profile)
            .chain(jir)
            .chain(self.emit_args.iter().map(String::as_str))
            .chain(self.optimizer.backend_arg())
            .chain(optimizer_passes.as_deref())
            .chain(self.crate_args.iter().map(String::as_str))
            .chain(lane)
            .chain(chunks.iter().map(String::as_str))
            .chain(warnings)
            .chain(report.as_deref())
            .chain(workspace_root.as_deref())
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

    /// The workspace path after compile_core's checkout-root remapping.
    fn backend_workspace_root(&self, toolchain: &Toolchain) -> Option<PathBuf> {
        let root = self.workspace_root.as_ref()?;
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let toolchain_root = toolchain
            .root()
            .canonicalize()
            .unwrap_or_else(|_| toolchain.root().to_path_buf());
        match root.strip_prefix(&toolchain_root) {
            Ok(relative) if !relative.as_os_str().is_empty() => Some(relative.to_path_buf()),
            _ => Some(root),
        }
    }
}

/// The files [`BuildConfig::render`] published into `OUT_DIR`.
#[derive(Debug, Clone)]
pub struct Artifacts {
    /// The verified linker deployment manifest, relative to the published artifacts.
    pub artifact_manifest: PathBuf,
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
    /// The size and optimization report, when requested.
    pub optimization_report: Option<PathBuf>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TempDir;

    #[test]
    fn the_default_configuration_asks_for_a_source_map() {
        let config = BuildConfig::new("client/app.rs");
        assert_eq!(config.output_name, DEFAULT_OUTPUT_NAME);
        assert_eq!(config.extra_args(), "js-source-map=on");
        assert!(!config.minimal_shim);
        assert!(!config.keyed_views);
    }

    #[test]
    fn keyed_view_lowering_is_an_explicit_view_mode() {
        let config = BuildConfig::new("client/app.rs").keyed_views(true);

        assert!(config.views);
        assert!(config.keyed_views);
    }

    #[test]
    fn the_minimal_runtime_is_opt_in() {
        assert!(
            BuildConfig::new("client/app.rs")
                .minimal_shim(true)
                .minimal_shim
        );
    }

    #[test]
    fn typed_optimizer_settings_feed_the_backend_in_stable_order() {
        let report = Path::new("/out/app.opt.json");
        let config = BuildConfig::new("client/app.rs")
            .source_map(false)
            .optimizer(OptimizerMode::Report)
            .optimizer_passes([
                OptimizerPass::FuturePoller,
                OptimizerPass::AstDce,
                OptimizerPass::FuturePoller,
                OptimizerPass::NativeRepresentations,
                OptimizerPass::LinkDedup,
                OptimizerPass::Sroa,
            ])
            .optimization_warnings(false)
            .optimization_report(true)
            .workspace_root("/workspace");

        assert_eq!(config.emit_arguments(), "js-opt=report js-opt-passes=all");
        assert_eq!(
            config.extra_args_for(Some(report), Some(Path::new("/workspace"))),
            concat!(
                "js-opt=report js-opt-passes=all ",
                "js-opt-warnings=off js-opt-report=/out/app.opt.json ",
                "js-workspace-root=/workspace",
            )
        );
        assert!(config.optimization_report);
    }

    #[test]
    fn a_profile_expands_into_the_options_it_names() {
        let size = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::ReleaseSize);
        assert_eq!(
            size.emit_arguments(),
            "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on"
        );
        assert_eq!(
            size.extra_args(),
            "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on js-profile=release-size js-lane=jir"
        );

        let speed = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::ReleaseSpeed);
        assert_eq!(
            speed.emit_arguments(),
            "js-cleanup=on js-ecma-target=2018 js-minify=locals js-opt=on"
        );
        assert_eq!(
            speed.extra_args(),
            "js-cleanup=on js-ecma-target=2018 js-minify=locals js-opt=on js-profile=release-speed js-lane=jir"
        );

        let balanced = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::ReleaseBalanced);
        assert_eq!(balanced.emit_arguments(), size.emit_arguments());
        assert_eq!(
            balanced.extra_args(),
            "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on js-profile=release-balanced js-lane=jir"
        );

        let dev = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::Dev);
        assert_eq!(dev.emit_arguments(), "js-opt=off");
        // The baseline lane is the default, so it costs no argument at all.
        assert_eq!(dev.extra_args(), "js-opt=off js-profile=dev");

        let report = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::Report);
        assert_eq!(report.emit_arguments(), "js-opt=report");
        assert_eq!(report.extra_args(), "js-opt=report js-profile=report");
    }

    #[test]
    fn a_spelled_out_option_overrides_the_profile_that_implied_it() {
        // The profile is expanded first, and the backend reads the last value given for an
        // option, so the explicit one is what runs.
        let config = BuildConfig::new("client/app.rs")
            .source_map(false)
            .profile(Profile::ReleaseSize)
            .optimizer(OptimizerMode::Report)
            .emit_arg("js-minify=off")
            .lane(Lane::Baseline);

        assert_eq!(
            config.emit_arguments(),
            "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on js-minify=off js-opt=report"
        );
        assert_eq!(
            config.extra_args(),
            concat!(
                "js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on js-profile=release-size ",
                "js-minify=off js-opt=report",
            )
        );
    }

    #[test]
    fn the_lane_is_a_link_step_choice_and_the_jir_body_is_an_emit_one() {
        let shadow = BuildConfig::new("client/app.rs")
            .source_map(false)
            .lane(Lane::Shadow);
        assert_eq!(shadow.emit_arguments(), "");
        assert_eq!(shadow.extra_args(), "js-lane=shadow");

        let without_jir = BuildConfig::new("client/app.rs")
            .source_map(false)
            .jir(false);
        assert_eq!(without_jir.emit_arguments(), "js-jir=off");
        assert_eq!(without_jir.extra_args(), "js-jir=off");
        let cleanup = BuildConfig::new("client/app.rs")
            .source_map(false)
            .emit_arg("js-cleanup=on");
        assert_eq!(cleanup.emit_arguments(), "js-cleanup=on");
        assert_eq!(cleanup.extra_args(), "js-cleanup=on");

        // The default carries JIR bodies and prints the baseline, which is no argument either way.
        let default = BuildConfig::new("client/app.rs").source_map(false);
        assert_eq!(default.emit_arguments(), "");
        assert_eq!(default.extra_args(), "");
    }

    #[test]
    fn an_empty_optimizer_pass_mask_is_explicit() {
        let config = BuildConfig::new("client/app.rs")
            .source_map(false)
            .optimizer(OptimizerMode::On)
            .optimizer_passes([]);

        assert_eq!(config.emit_arguments(), "js-opt=on js-opt-passes=none");
    }

    #[test]
    fn an_optimizer_report_cannot_claim_to_describe_split_chunks() {
        let config = BuildConfig::new("client/app.rs")
            .chunks(Chunking::new().island("counter"))
            .optimization_report(true);

        assert!(matches!(
            config.check_report_shape(),
            Err(BuildError::ChunkedOptimizationReport)
        ));
    }

    #[test]
    fn rust_optimization_levels_use_rustcs_spellings() {
        assert_eq!(RustOptLevel::O0.as_str(), "0");
        assert_eq!(RustOptLevel::O1.as_str(), "1");
        assert_eq!(RustOptLevel::O2.as_str(), "2");
        assert_eq!(RustOptLevel::O3.as_str(), "3");
        assert_eq!(RustOptLevel::Size.as_str(), "s");
        assert_eq!(RustOptLevel::SizeMin.as_str(), "z");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn publishing_a_minimal_module_shim_closes_over_the_program() {
        let dir = TempDir::new("minimal-shim");
        let out = dir.path();
        let program = staged(out, &["islands.js"]);
        let source = "import * as __rt from './shim.js'; __rt.host_task(() => {});\n";
        fs::write(&program, source).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let toolchain = Toolchain::new(root).unwrap();
        let manifest_path = artifact::manifest_path(&program);
        let mut manifest = ArtifactManifest::read(&manifest_path).unwrap();
        let program_artifact = manifest.artifacts.get_mut("islands.js").unwrap();
        program_artifact.bytes = source.len() as u64;
        program_artifact.sha256 = artifact::sha256(source.as_bytes());
        program_artifact
            .imports
            .insert("./shim.js".to_owned(), None);
        program_artifact.runtime = Some(artifact::RuntimeRequirement {
            module: Some("./shim.js".to_owned()),
            source_sha256: artifact::sha256(&fs::read(toolchain.shim()).unwrap()),
            direct: BTreeSet::from(["host_task".to_owned()]),
            closure: BTreeSet::from(["host_task".to_owned()]),
            external: BTreeSet::new(),
            artifact: None,
        });
        manifest.write(&manifest_path).unwrap();
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .crate_arg("js-modules=esm")
            .minimal_shim(true);
        let (_, _, published_manifest) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        let published = config
            .publish_shim(&toolchain, &published_manifest)
            .unwrap()
            .unwrap();
        let source = fs::read_to_string(published).unwrap();

        assert!(source.contains("globalThis.__rt.host_task ="), "{source}");
        assert!(source.contains("  host_task,"), "{source}");
        assert!(!source.contains("globalThis.__rt.host_try ="), "{source}");
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
            "js-minify=on js-modules=esm js-chunk=counter:__island_counter js-chunk-shared=shared js-chunk-role=counter:island"
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
        use rcgjs_object::artifact::{Artifact, ArtifactKind, Entry, ARTIFACT_SCHEMA};
        use std::collections::BTreeMap;
        let staging = dir.join(".mires-build");
        fs::create_dir_all(&staging).unwrap();
        let mut manifest = ArtifactManifest {
            schema: ARTIFACT_SCHEMA,
            object_format: rcgjs_object::object::FORMAT_VERSION,
            jir_schema: rcgjs_object::jir::JIR_SCHEMA,
            semantic_policy: "faithful".to_owned(),
            syntax_ceiling: "2018".to_owned(),
            entries: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        };
        let split = files
            .iter()
            .any(|name| name.ends_with(".js") && *name != "islands.js");
        for name in files {
            let source = format!("// {name}\n");
            fs::write(staging.join(name), &source).unwrap();
            let kind = if name.ends_with(".map") {
                ArtifactKind::SourceMap
            } else if name.ends_with(".d.ts") {
                ArtifactKind::Declarations
            } else {
                ArtifactKind::Program
            };
            let mut artifact = Artifact::new(kind, source.as_bytes());
            if kind == ArtifactKind::Program {
                artifact.items.insert(format!("export:{name}"));
                let map = format!("{name}.map");
                if files.contains(&map.as_str()) {
                    artifact.source_map = Some(map);
                }
                let declarations = format!("{}.d.ts", name.trim_end_matches(".js"));
                if files.contains(&declarations.as_str()) {
                    artifact.declarations = Some(declarations);
                }
                if *name != "islands.js" || !split {
                    let entry_name = if split {
                        name.trim_end_matches(".js")
                    } else {
                        "program"
                    };
                    manifest.entries.insert(
                        entry_name.to_owned(),
                        Entry {
                            role: if entry_name == "shared" {
                                EntryRole::Shared
                            } else {
                                EntryRole::Main
                            },
                            program: (*name).to_owned(),
                            roots: BTreeMap::from([((*name).to_owned(), format!("export:{name}"))]),
                        },
                    );
                }
            }
            manifest.artifacts.insert((*name).to_owned(), artifact);
        }
        let program = staging.join("islands.js");
        manifest.write(&artifact::manifest_path(&program)).unwrap();
        program
    }

    #[test]
    fn a_program_no_chunk_was_asked_for_is_published_beside_out_dir() {
        let dir = TempDir::new("chunks-none");
        let out = dir.path();
        let program = staged(out, &["islands.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .source_map(false);

        let (chunks, shared, _) = config
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
    fn empty_chunking_keeps_the_unsplit_program() {
        let dir = TempDir::new("chunks-empty");
        let out = dir.path();
        let program = staged(out, &["islands.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .chunks(Chunking::new());
        let (chunks, shared, manifest) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();
        assert_eq!(chunks[0].name, "islands");
        assert_eq!(chunks[0].path, out.join("islands.js"));
        assert_eq!(shared, None);
        assert_eq!(manifest, out.join("islands.js.artifacts.json"));
    }

    #[test]
    fn a_source_map_is_published_beside_its_chunk() {
        let dir = TempDir::new("chunks-map");
        let out = dir.path();
        let program = staged(out, &["islands.js", "islands.js.map"]);
        let config = BuildConfig::new("client/app.rs").output_name("islands.js");

        let (chunks, _, _) = config
            .publish_chunks(program.parent().unwrap(), &program, out)
            .unwrap();

        assert_eq!(
            chunks[0].map.as_deref(),
            Some(out.join("islands.js.map")).as_deref()
        );
    }

    #[test]
    fn publishing_refuses_changed_program_map_and_declaration_bytes() {
        for changed in ["islands.js", "islands.js.map", "islands.d.ts"] {
            let dir = TempDir::new("artifact-drift");
            let program = staged(
                dir.path(),
                &["islands.js", "islands.js.map", "islands.d.ts"],
            );
            fs::write(
                program.parent().unwrap().join(changed),
                b"changed after linking",
            )
            .unwrap();
            let error = BuildConfig::new("client/app.rs")
                .output_name("islands.js")
                .publish_chunks(program.parent().unwrap(), &program, dir.path())
                .unwrap_err();
            assert!(
                error.to_string().contains("disagree with linker manifest"),
                "{error}"
            );
            assert!(!dir.path().join("islands.js").exists());
        }
    }

    #[test]
    fn an_unsplit_manifest_cannot_substitute_for_requested_chunks() {
        let dir = TempDir::new("chunks-unsplit");
        let program = staged(dir.path(), &["islands.js"]);
        let config = BuildConfig::new("client/app.rs")
            .output_name("islands.js")
            .chunks(Chunking::new().island("counter").island("search"));
        let error = config
            .publish_chunks(program.parent().unwrap(), &program, dir.path())
            .unwrap_err();
        assert!(error.to_string().contains("unrequested entry"), "{error}");
        assert!(!dir.path().join("chunks/islands.js").exists());
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

        let (chunks, shared, _) = config
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

        let (_, shared, _) = config
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
                    .import("topcoat-dom", "/demo/topcoat-dom.js"),
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
                r#"{"imports":{"topcoat-dom":"/demo/topcoat-dom.js","#,
                r#""topcoat-island/counter":"/demo/chunks/counter.js"}}"#,
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
            .contains(r#""topcoat-island/shared":"/demo/chunks/shared.js""#));
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
            artifact_manifest: PathBuf::from("/out/islands.js.artifacts.json"),
            chunks: vec![one.clone()],
            shared: None,
            shim: None,
            import_map: None,
            optimization_report: None,
        };

        assert_eq!(artifacts.program(), Some(&one));
        assert_eq!(artifacts.chunk("islands"), Some(&one));
        assert_eq!(artifacts.chunk("counter"), None);

        let split = Artifacts {
            artifact_manifest: PathBuf::from("/out/islands.js.artifacts.json"),
            chunks: vec![one.clone()],
            shared: Some(one),
            shim: None,
            import_map: None,
            optimization_report: None,
        };

        assert_eq!(split.program(), None);
        assert_eq!(
            split.chunk("islands").map(|chunk| &chunk.name),
            Some(&"islands".to_owned())
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
