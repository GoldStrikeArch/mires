//! Backend options, parsed out of `-Cllvm-args`.
//!
//! `-Cllvm-args` is the one channel a hot-plugged codegen backend gets for free: rustc collects
//! every occurrence into `sess.opts.cg.llvm_args`, splitting each on whitespace. Every option this
//! backend understands is spelled `js-<name>` or `js-<name>=<value>`:
//!
//! ```text
//! -Cllvm-args=js-names=readable   readable|mangled   how identifiers are spelled (naming.rs)
//! -Cllvm-args=js-root=NAME        repeatable         an extra DCE root, by emitted JS name
//! -Cllvm-args=js-structure=...    regions|trampoline how control flow is rebuilt (stage 2)
//! -Cllvm-args=js-queue=off        on|off             the expression queue (stage 2)
//! -Cllvm-args=js-switch=dtree     dtree|flat         how `SwitchInt` is spelled (stage 2)
//! -Cllvm-args=js-comments=on      on|off             per item header comments
//! -Cllvm-args=js-line-comments    flag               `// file.rs:LINE` location comments
//! -Cllvm-args=js-source-map=off   on|off             emit a source map beside the program
//! -Cllvm-args=js-emit-skip=PREFIX repeatable         hide items under a def path prefix
//! -Cllvm-args=js-scoped-lets      flag               declare a local where it is assigned
//! -Cllvm-args=js-minify=off       on|locals|off      rename and print for size (minify.rs)
//! -Cllvm-args=js-ecma-target=latest 2018|latest       emitted JavaScript syntax ceiling
//! -Cllvm-args=js-hoist-consts=off on|off             share repeated constants (cgu.rs)
//! -Cllvm-args=js-modules=script   esm|script         ES module or plain script (link.rs)
//! -Cllvm-args=js-shim-module=...  a module specifier where `__rt` is imported from
//! -Cllvm-args=js-entry-role=main main|worker|island|procedure|shared deployment role
//! -Cllvm-args=js-chunk-role=NAME:ROLE repeatable     declared role of a chunk root
//! -Cllvm-args=js-dom-module=...   a module specifier where the DOM runtime is imported from
//! -Cllvm-args=js-chunk=NAME:ENTRY repeatable         split the program per entry point (link.rs)
//! -Cllvm-args=js-chunk-shared=... a chunk name       where two chunks' common items go
//! -Cllvm-args=js-chunk-registry=off on|off           register each chunk's entry for a loader
//! -Cllvm-args=js-opt=off          off|report|on      whole-program optimizer mode
//! -Cllvm-args=js-opt-passes=all   a comma list       developer-only optimizer pass mask
//! -Cllvm-args=js-opt-warnings=on  on|off             warn for reachable workspace misses
//! -Cllvm-args=js-opt-report=PATH  a file path        write the deterministic optimizer report
//! -Cllvm-args=js-workspace-root=PATH a directory     source allowed to produce opt warnings
//! -Cllvm-args=js-cleanup=off      on|off             retain host-exception cleanup
//! -Cllvm-args=js-jir=on           on|off             carry JIR-S bodies in the object files
//! -Cllvm-args=js-lane=baseline    baseline|shadow|jir which lane the link step prints from
//! -Cllvm-args=js-lane-report=PATH a file path        write the per item lane availability report
//! -Cllvm-args=js-lane-classes=all a comma list       the ItemClass flags the JIR lane may own
//! -Cllvm-args=js-lane-only=NAME   a comma list       lower only these items on the JIR lane
//! -Cllvm-args=js-jir-flat-abi=on    on|off          plan the JIR lane's flat direct-call ABI
//! -Cllvm-args=js-jir-typed-storage=off on|off       opt in to proven exact numeric Vec storage
//! -Cllvm-args=js-jir-ready-futures=off on|off       opt in to proven nonsuspending Promise drivers
//! -Cllvm-args=js-str=utf16        utf16|bytes        how a Rust `str` is held at run time (release profiles default to `bytes` on the JIR lane)
//! -Cllvm-args=js-opt-weight-alloc=24 an integer     the representation cost model's bytes per
//!                                                   allocation site (Phase 4)
//! -Cllvm-args=js-jir-dump-conflicts=PATH a file path where a dedup conflict autopsy is written
//! -Cllvm-args=js-profile=dev      an alias bundle    dev|report|release-size|release-balanced|
//!                                                    release-speed (see [`Profile`])
//! ```
//!
//! The value shown above is the default. `js-switch` and `js-dom-module` are parsed, validated and
//! stored but not yet consumed; every other option is live. `js-jir`, `js-lane`, `js-lane-report`,
//! `js-lane-classes`, `js-lane-only` and `js-jir-dump-conflicts` are the v2 lane controls. The
//! lane driver (`jsc-link/src/link/lane.rs`) reads them at link time; an item it cannot lower
//! keeps its baseline text, so a lane is a per item choice and never a whole-program mode.
//!
//! `js-profile` is an **alias layer**, not a setting of its own: it is resolved before the
//! individual options are read, so a profile names a starting point and any flag spelled out
//! alongside it wins, wherever the two appear on the command line. See [`Profile`].
//!
//! An option is **emit-affecting** when it can change the text of an item two crates might both
//! codegen, which is what makes it one that every object in a program has to agree on — the
//! sysroot's `core` included. `js-minify`, `js-structure`, `js-queue`, `js-names`, and
//! `js-ecma-target` are; the module options are not, because everything they change is synthesized
//! by the link step or belongs to the leaf crate alone. See CONTRACT.md. The one spelled `=off` is
//! on by default and turns *off* the milestone that introduced it, which is what makes a regression
//! bisectable: `js-queue=off` and `js-structure=trampoline` each reproduce, byte for byte, the
//! output the backend produced before that milestone landed.
//!
//! `js-emit-skip` is the odd one out: it is validated here but *acted on by
//! `scripts/emit-test.sh`*, which filters the text the backend printed. It cannot be a backend
//! rule — an item left out of the program would be a `ReferenceError` at run time rather than a
//! tidier golden file, and every emit fixture is also run under node. Keeping the flag here is
//! what puts the skip list in the compile command a golden was baselined with.
//!
//! The parsed options live in a process global: the backend dylib is loaded once and rustc
//! compiles exactly one crate per process, so there is nothing to key them by. The producer's
//! `init` fills it from `CodegenBackend::init`, and [`get`] hands out the result (defaults if
//! `init` never ran, which only happens in unit tests).

use serde::{Deserialize, Serialize};

use std::path::{Component, Path, PathBuf};

use crate::jir::summary::ItemClass;
use std::sync::OnceLock;

/// Whether optimizer analyses are disabled, reported, or applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OptimizerMode {
    /// Preserve the current lowering and collect no optimization metadata.
    Off,
    /// Run analyses and report their result without changing emitted JavaScript.
    Report,
    /// Apply every optimization whose safety proof succeeds.
    On,
}

impl OptimizerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Report => "report",
            Self::On => "on",
        }
    }
}

/// Which lowering the link step prints an item from.
///
/// v2 keeps v1's printed baseline JavaScript in every object file as a per item fallback, so the
/// lane is a *choice between two available spellings* rather than a mode the whole compilation
/// commits to. It is decided entirely at link time — an object carries both lanes — which is what
/// keeps it out of the sysroot cache key.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Lane {
    /// v1's printed text, item for item. The frozen oracle.
    #[default]
    Baseline,
    /// Print the baseline, and the JIR lane beside it, for differential comparison.
    Shadow,
    /// Print from JIR-S wherever the item's class has been promoted, baseline text elsewhere.
    Jir,
}

impl Lane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Shadow => "shadow",
            Self::Jir => "jir",
        }
    }
}

/// How a Rust `str` is held at run time (`js-str`).
///
/// `Utf16` is CONTRACT.md's hybrid: a `&str` is a JavaScript string holding the UTF-16 text, and
/// every byte-level operation converts. `Bytes` makes it a **byte string**: a JavaScript string
/// whose code unit `i` is byte `i` of the UTF-8 text, so every unit is below 256, its `length` is
/// the Rust length and a byte offset is an index. Host text is UTF-8 encoded where it becomes a
/// Rust `str` and decoded where Rust text reaches the host.
///
/// The three `release-*` profiles select `Bytes` by default when running on the JIR lane; `dev`,
/// `report`, unprofiled invocations, and non-JIR overrides default to `Utf16`.
///
/// Link-only: every `str` operation the lowering emits is chosen at link time on the JIR lane,
/// and nothing an object file carries depends on it, so it never joins the sysroot key. The
/// baseline text in the objects is UTF-16 text, which is why `bytes` refuses any item that
/// falls back to it (`jsc-link/src/link/lane.rs`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum StrRepr {
    #[default]
    Utf16,
    Bytes,
}

impl StrRepr {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf16 => "utf16",
            Self::Bytes => "bytes",
        }
    }
}

/// A named bundle of option settings, applied before the individual options are read.
///
/// A profile exists so that the three or four flags a build actually wants together have one
/// spelling that cannot drift: `js-profile=release-size` is the whole of what a size-tuned
/// release build passes, and the day a phase changes what that means, it changes here rather
/// than in every build script. Resolution order is what makes it an alias rather than an
/// override — the profile is expanded first and any explicitly spelled option then wins, so
/// `js-profile=release-size js-minify=off` is a release-size build without minification.
///
/// Release profiles run the promoted JIR optimizer. The profile is also the policy input for
/// exact physical-ABI and representation-carrier economics; `dev` and `report` retain their
/// non-emitting meanings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Profile {
    /// `js-opt=off js-lane=baseline`: the fastest compile, and the output a golden was taken from.
    Dev,
    /// `js-opt=report js-lane=baseline`: analyses run and are reported, nothing is rewritten.
    Report,
    /// `js-lane=jir js-minify=on js-opt=on js-str=bytes`: compiler-side size policy.
    ReleaseSize,
    /// `js-lane=jir js-minify=on js-opt=on js-str=bytes`: weighted bytes and allocation policy.
    ReleaseBalanced,
    /// `js-lane=jir js-minify=locals js-opt=on js-str=bytes`: item names kept, so a profile still reads.
    ReleaseSpeed,
}

impl Profile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Report => "report",
            Self::ReleaseSize => "release-size",
            Self::ReleaseBalanced => "release-balanced",
            Self::ReleaseSpeed => "release-speed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "dev" => Some(Self::Dev),
            "report" => Some(Self::Report),
            "release-size" => Some(Self::ReleaseSize),
            "release-balanced" => Some(Self::ReleaseBalanced),
            "release-speed" => Some(Self::ReleaseSpeed),
            _ => None,
        }
    }

    /// The emit-affecting half of this alias, in application order.
    ///
    /// Keep this in lockstep with `scripts/sysroot-cache.sh`'s
    /// `jsc_profile_emit_args` and mires-build's `Profile::emit_args`.
    pub(crate) const fn emit_args(self) -> &'static [&'static str] {
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

    /// Writes this profile's settings into `opts`, before any explicit option is read.
    fn apply(self, opts: &mut Opts) {
        opts.profile = Some(self);
        match self {
            Self::Dev => {
                opts.optimizer = OptimizerMode::Off;
                opts.lane = Lane::Baseline;
                opts.str_repr = StrRepr::Utf16;
            }
            Self::Report => {
                opts.optimizer = OptimizerMode::Report;
                opts.lane = Lane::Baseline;
                opts.str_repr = StrRepr::Utf16;
            }
            Self::ReleaseSize | Self::ReleaseBalanced => {
                opts.cleanup = true;
                opts.ecma_target = EcmaTarget::Es2018;
                opts.lane = Lane::Jir;
                opts.minify = true;
                opts.minify_items = true;
                opts.optimizer = OptimizerMode::On;
                opts.str_repr = StrRepr::Bytes;
            }
            Self::ReleaseSpeed => {
                opts.cleanup = true;
                opts.ecma_target = EcmaTarget::Es2018;
                opts.lane = Lane::Jir;
                opts.minify = true;
                opts.minify_items = false;
                opts.optimizer = OptimizerMode::On;
                opts.str_repr = StrRepr::Bytes;
            }
        }
    }
}

/// One independently selectable optimizer pass, in compiler execution order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum OptimizerPass {
    Sroa,
    AstDce,
    FuturePoller,
    NativeRepresentations,
    LinkDedup,
}

impl OptimizerPass {
    pub const ORDERED: [Self; 5] = [
        Self::Sroa,
        Self::AstDce,
        Self::FuturePoller,
        Self::NativeRepresentations,
        Self::LinkDedup,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sroa => "sroa",
            Self::AstDce => "ast-dce",
            Self::FuturePoller => "future-poller",
            Self::NativeRepresentations => "native-representations",
            Self::LinkDedup => "link-dedup",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Sroa => 1 << 0,
            Self::AstDce => 1 << 1,
            Self::FuturePoller => 1 << 2,
            Self::NativeRepresentations => 1 << 3,
            Self::LinkDedup => 1 << 4,
        }
    }
}

/// The optimizer passes selected for one compilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptimizerPassMask(u8);

impl OptimizerPassMask {
    const ALL: Self = Self(
        OptimizerPass::Sroa.bit()
            | OptimizerPass::AstDce.bit()
            | OptimizerPass::FuturePoller.bit()
            | OptimizerPass::NativeRepresentations.bit()
            | OptimizerPass::LinkDedup.bit(),
    );

    pub const fn contains(self, pass: OptimizerPass) -> bool {
        self.0 & pass.bit() != 0
    }

    pub fn ordered(self) -> impl Iterator<Item = OptimizerPass> {
        OptimizerPass::ORDERED
            .into_iter()
            .filter(move |pass| self.contains(*pass))
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => return Some(Self::ALL),
            "none" => return Some(Self(0)),
            "" => return None,
            _ => {}
        }

        let mut mask = Self(0);
        for name in value.split(',') {
            let pass = match name {
                "sroa" => OptimizerPass::Sroa,
                "ast-dce" => OptimizerPass::AstDce,
                "future-poller" => OptimizerPass::FuturePoller,
                "native-representations" => OptimizerPass::NativeRepresentations,
                "link-dedup" => OptimizerPass::LinkDedup,
                _ => return None,
            };
            mask.0 |= pass.bit();
        }
        Some(mask)
    }
}

impl Default for OptimizerPassMask {
    fn default() -> Self {
        Self::ALL
    }
}

/// How the backend spells the identifiers it invents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NameStyle {
    /// A sanitized def path plus a hash suffix: `mini_core$print_i32$h1c0ffee...`.
    Readable,
    /// The raw v0 symbol name, as the spike emitted it. Kept for bisecting.
    Mangled,
}

/// How control flow is rebuilt from the CFG.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Structure {
    /// The structurizer's region tree (stage 2).
    Regions,
    /// The `switch (bb)` trampoline.
    Trampoline,
}

/// The JavaScript syntax ceiling used by the printer.
///
/// `Es2018` constrains syntax only. It may still emit calls to later runtime facilities such as
/// `BigInt`, whose availability is a separate deployment requirement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum EcmaTarget {
    Es2018,
    #[default]
    Latest,
}

impl EcmaTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Es2018 => "2018",
            Self::Latest => "latest",
        }
    }
}

/// What kind of JavaScript file the program is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Modules {
    /// An ES module: the runtime shim arrives through an `import`, and the exported items are
    /// named by a trailing `export` clause.
    Esm,
    /// A plain script: `__rt` is a global the host defines before the program runs, and an
    /// exported item is a global the host calls afterwards.
    Script,
}

/// How a `SwitchInt` is spelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SwitchStyle {
    /// A decision tree over the case values (stage 2).
    DTree,
    /// One flat JS `switch` with a case per value.
    Flat,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Opts {
    pub names: NameStyle,
    /// Extra dead-code-elimination roots, named by their emitted JS name.
    pub roots: Vec<String>,
    pub structure: Structure,
    pub queue: bool,
    pub switch: SwitchStyle,
    /// Whether each item is preceded by a `// <def path>` comment.
    pub comments: bool,
    pub line_comments: bool,
    /// Whether the finished program gets a `.map` and a `sourceMappingURL` comment.
    pub source_map: bool,
    /// Def path prefixes whose items are left out of `--emit`ted JS fixtures.
    pub emit_skip: Vec<String>,
    pub scoped_lets: bool,
    /// Whether names are shortened and the program printed without the whitespace. See
    /// `minify.rs`.
    pub minify: bool,
    /// Whether minification also renames *item* names at link time.
    ///
    /// `js-minify=locals` leaves them alone, which keeps a minified program readable enough to
    /// grep for a def path and is how the two halves of the win were measured apart. Meaningless
    /// unless [`Opts::minify`] is on.
    pub minify_items: bool,
    /// The syntax ceiling the item printer must satisfy.
    ///
    /// **Emit-affecting**: BigInt constants and logical assignments have different spellings, so
    /// every object in a program, including the sysroot, must agree on this value.
    #[serde(default)]
    pub ecma_target: EcmaTarget,
    /// Whether repeated constants — `#[track_caller]` locations, long string literals — are
    /// emitted once as a module level `const` instead of being written out at every use. See
    /// the codegen unit's constant pool.
    pub hoist_consts: bool,
    /// Whether the program is an ES module or a plain script. See [`Modules`].
    pub modules: Modules,
    /// The module the runtime shim's `__rt` namespace is imported from, in ES module output.
    pub shim_module: String,
    /// Declared deployment role of the unsplit program. Does not affect object emission.
    pub entry_role: crate::artifact::EntryRole,
    /// Explicit deployment roles for named chunks; unspecified chunks are main entries.
    pub chunk_roles: Vec<(String, crate::artifact::EntryRole)>,
    /// The module the DOM runtime's bindings are imported from, in ES module output.
    pub dom_module: String,
    /// The chunks to split the program into, as `(name, entry)` pairs in request order.
    ///
    /// Empty means one file, which is every compilation that does not ask to be split. See
    /// the link step.
    pub chunks: Vec<(String, String)>,
    /// The name of the chunk holding what two or more chunks both reach.
    pub shared_chunk: String,
    /// Whether each named chunk registers its entry in `globalThis.__rcgjsChunks`.
    #[serde(default)]
    pub chunk_registry: bool,
    /// Whether the link step writes a `.d.ts` beside each output file. See `dts.rs`.
    ///
    /// NOT emit affecting: it is synthesized at the link step from what the object files already
    /// carry, and nothing about the JavaScript changes, so it does not have to agree with the
    /// flags the sysroot was built with.
    pub dts: bool,
    /// Whether optimization analyses run and whether their proven rewrites are emitted.
    pub optimizer: OptimizerMode,
    /// Developer-only mask used to attribute optimizer passes independently.
    pub optimizer_passes: OptimizerPassMask,
    /// Whether reachable misses attributed to workspace source produce diagnostics.
    pub optimization_warnings: bool,
    /// Where the link step writes its deterministic optimization and size report.
    pub optimization_report: Option<String>,
    /// The source tree whose rejected candidates may produce diagnostics.
    pub workspace_root: Option<PathBuf>,
    /// Whether each emitted object carries the JIR-S body of the items it defines.
    ///
    /// **Emit-affecting**, and the only one of the lane options that is: it decides what an
    /// object file *contains*, so the sysroot's `core` and the crate linking it have to agree
    /// or the link step finds items with a JIR body on one side and none on the other. On by
    /// default; `js-jir=off` produces v1-shaped objects, which is the bisecting step for a
    /// regression in the encoder or a size question about the objects themselves.
    pub jir: bool,
    /// Carry host-exception cleanup bodies through codegen and dependency metadata.
    pub cleanup: bool,
    /// Which lane the link step prints from. See [`Lane`].
    ///
    /// NOT emit-affecting: both lanes travel in the object, and choosing between them is a link
    /// step decision, exactly as `js-chunk` is.
    pub lane: Lane,
    /// Where the link step writes the per item lane availability report.
    ///
    /// A link-only path, like [`Opts::optimization_report`]: it describes what the linker chose,
    /// so it cannot change what a crate emits.
    pub lane_report: Option<String>,
    /// The [`crate::jir::summary::ItemClass`] flags the JIR lane is allowed to own.
    ///
    /// An item is offered to the lowerer only when **every** flag of its class is in this set, so
    /// `js-lane-classes=scalar-leaf` promotes the items that are nothing but scalar leaves and
    /// leaves an item that is also an aggregate on the baseline. That is the promotion order of
    /// `checkpoints/phase-2-briefs.md` spelled as an option: one class at a time, shadow-green
    /// before the flip. The default is every flag, which is what the exit gate measures.
    ///
    /// An item whose class is **empty** — the classifier found nothing in it that this option can
    /// name — is a subset of every set and so is offered whenever a lane runs at all, `none`
    /// included. `js-lane=baseline` is how a build promotes nothing.
    ///
    /// Link-only, like [`Opts::lane`] itself.
    pub lane_classes: ItemClass,
    /// Whether the JIR lane plans the flat direct-call ABI.
    ///
    /// On by default. `js-jir-flat-abi=off` leaves every call in the baseline shape — one
    /// JavaScript argument per Rust argument, an aggregate as an object — which is the bisecting
    /// step for a miscompile the flat ABI could be behind. Link-only, like [`Opts::lane`]: the
    /// plan is computed over the whole program's JIR at link time.
    pub flat_abi: bool,
    /// Whether the linker may emit the narrow exact-size numeric `Vec` storage tranche.
    ///
    /// Off by default until the Phase 12D complete-workload, multi-engine promotion gate passes.
    /// Link-only: the choice is made over whole-program JIR after representation planning.
    #[serde(default)]
    pub typed_storage: bool,
    /// Link-only opt-in for proven nonsuspending Promise-boundary drivers.
    #[serde(default)]
    pub ready_futures: bool,
    /// What the representation planner's cost model charges per allocation site, in
    /// minified-byte equivalents: a site inside `depth` nested loops costs
    /// `weight_alloc · 4^min(depth, 3)` (Phase 4, D9).
    ///
    /// Link-only, like [`Opts::flat_abi`]: the cost model runs over the whole program's JIR at
    /// link time and changes nothing an object file contains, so this is NOT emit-affecting and
    /// never joins [`crate::item::normalized_emit_args`] or the sysroot key.
    pub weight_alloc: u64,
    /// When non-empty, the only item names the JIR lane may lower: a bisection aid.
    ///
    /// `js-lane-only=a,b` answers "which item does the jir lane get wrong" in log2 builds rather
    /// than by reading a diff of the whole program. Names are the emitted JavaScript identifiers,
    /// the same spelling the lane report and the reachability walk use. Link-only.
    pub lane_only: Vec<String>,
    /// Where the link step writes the autopsy of the items two objects defined differently.
    pub jir_dump_conflicts: Option<String>,
    /// How a Rust `str` is held at run time. See [`StrRepr`]. Link-only.
    #[serde(default)]
    pub str_repr: StrRepr,
    /// The alias bundle this compilation named, if any. See [`Profile`].
    ///
    /// Recorded rather than expanded away so that a report can say which profile a build ran
    /// under; every setting it implies is already resolved into the fields above.
    pub profile: Option<Profile>,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            names: NameStyle::Readable,
            roots: Vec::new(),
            structure: Structure::Regions,
            queue: true,
            switch: SwitchStyle::Flat,
            comments: true,
            line_comments: false,
            source_map: false,
            emit_skip: Vec::new(),
            scoped_lets: false,
            minify: false,
            minify_items: true,
            ecma_target: EcmaTarget::Latest,
            // Off by default on measured evidence (see CONTRACT.md "Hoisted constants"): the
            // 16-hex hash names are incompressible, so hoisting loses 1-5% gzipped even where
            // it wins raw minified bytes. The mechanism stays for a future per-item reuse-count
            // gate; `--remap-path-prefix` (the actual win) is unconditional in the scripts.
            hoist_consts: false,
            modules: Modules::Script,
            shim_module: "./shim.js".to_owned(),
            entry_role: crate::artifact::EntryRole::Main,
            chunk_roles: Vec::new(),
            dom_module: "topcoat-dom".to_owned(),
            chunks: Vec::new(),
            dts: false,
            shared_chunk: "shared".to_owned(),
            chunk_registry: false,
            optimizer: OptimizerMode::Off,
            optimizer_passes: OptimizerPassMask::default(),
            optimization_warnings: true,
            optimization_report: None,
            workspace_root: None,
            // On by default: an object without a JIR body cannot be linked on the JIR lane at
            // all, and the whole point of Phase 1 is that every object already carries one by
            // the time a lane can consume it.
            jir: true,
            cleanup: false,
            // The frozen v1 text is what every golden, budget and differential is baselined
            // against, so it stays the default until a lane has been promoted past it.
            lane: Lane::default(),
            lane_report: None,
            // Every class: the lane driver offers every reachable item with a body to the
            // lowerer and counts what comes back. Narrowing this is how a promotion round is run.
            lane_classes: ItemClass::all(),
            flat_abi: true,
            typed_storage: false,
            ready_futures: false,
            weight_alloc: 24,
            lane_only: Vec::new(),
            jir_dump_conflicts: None,
            str_repr: StrRepr::Utf16,
            profile: None,
        }
    }
}

impl Opts {
    /// Whether `pass` runs its analysis in this compilation.
    pub fn analyzes(&self, pass: OptimizerPass) -> bool {
        self.optimizer != OptimizerMode::Off && self.optimizer_passes.contains(pass)
    }

    /// Whether `pass` may change emitted JavaScript in this compilation.
    pub fn applies(&self, pass: OptimizerPass) -> bool {
        self.optimizer == OptimizerMode::On && self.optimizer_passes.contains(pass)
    }

    pub fn requested_passes(&self) -> impl Iterator<Item = OptimizerPass> {
        self.optimizer_passes.ordered()
    }

    pub fn analyzed_passes(&self) -> impl Iterator<Item = OptimizerPass> {
        let enabled = self.optimizer != OptimizerMode::Off;
        self.optimizer_passes.ordered().filter(move |_| enabled)
    }

    /// Passes allowed to change output in this compilation.
    ///
    /// This is not the set that actually changed a reachable item. The final linker derives that
    /// set from emitted optimization evidence for the report.
    pub fn emission_enabled_passes(&self) -> impl Iterator<Item = OptimizerPass> {
        let enabled = self.optimizer == OptimizerMode::On;
        self.optimizer_passes.ordered().filter(move |_| enabled)
    }
}

static OPTS: OnceLock<Opts> = OnceLock::new();

/// Installs the options this compilation runs with. The first call wins; later ones are ignored.
///
/// The backend parses them out of `-Cllvm-args`; `jsc-link` reads them back off the manifest the
/// backend wrote, which is what keeps one parse per compilation rather than two.
pub fn set(opts: Opts) {
    let _ = OPTS.set(opts);
}

/// The options this compilation runs with.
pub fn get() -> &'static Opts {
    OPTS.get_or_init(Opts::default)
}

/// Whether a rendered source file is inside the configured workspace.
#[allow(dead_code)]
pub fn is_workspace_source(file: &str) -> bool {
    let Some(root) = &get().workspace_root else {
        return false;
    };
    is_source_within(root, Path::new(file))
}

fn is_source_within(root: &Path, file: &Path) -> bool {
    let root = lexical_path(root);
    let file = lexical_path(file);
    if root.as_os_str().is_empty() {
        return !matches!(
            file.components().next(),
            Some(Component::ParentDir | Component::RootDir | Component::Prefix(_))
        );
    }
    file.starts_with(root)
}

/// Resolves `.` and `..` without consulting the filesystem.
///
/// rustc preserves a `#[path = "../dependency.rs"]` spelling in diagnostics. `Path::starts_with`
/// compares that spelling component by component, so an outside file could otherwise look like a
/// child of the workspace. Canonicalizing is not suitable here: reports must not depend on whether
/// a source path still exists or which symlinks the build machine happens to expose.
fn lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let last = normalized.components().next_back();
                if matches!(last, Some(Component::Normal(_))) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Where an option diagnostic goes.
///
/// A rustc `Session` in a real compilation, wrapped by the backend; a plain stderr sink in
/// `jsc-link`. The indirection is what makes the parser testable: a `Session` cannot be built
/// outside a running compiler, and every rule below is a rule about the *arguments* rather than
/// about rustc, so the tests hand it a collector instead.
pub trait Diagnostics {
    fn err(&self, message: &str);
    fn warn(&self, message: &str);
}

pub fn parse_args(args: &[String], dcx: &dyn Diagnostics) -> Opts {
    let mut opts = Opts::default();
    let mut explicit_str = false;

    // The alias layer, resolved first so that an explicitly spelled option overrides whatever the
    // profile asked for regardless of which side of it the option was written on. A profile is a
    // starting point, and a command line reads as one thing whichever order it was assembled in.
    for arg in args {
        let (key, value) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (arg.as_str(), None),
        };
        if key != "js-profile" {
            continue;
        }
        match value.and_then(Profile::parse) {
            Some(profile) => profile.apply(&mut opts),
            None => bad_value(
                dcx,
                key,
                value,
                "dev`, `report`, `release-size`, `release-balanced`, `release-speed",
            ),
        }
    }

    for arg in args {
        let (key, value) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (arg.as_str(), None),
        };

        // A typo here silently changes what the backend emits, so every unknown key and every
        // unusable value gets a diagnostic rather than being ignored.
        match key {
            // Validated against the actual binary by the producer. Rustc tracks this argument;
            // it does not select a linker policy or belong in normalized emit options.
            "js-producer" => match value {
                Some(value)
                    if value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) => {
                }
                other => bad_value(dcx, key, other, "a lowercase SHA-256 digest"),
            },
            "js-names" => match value {
                Some("readable") => opts.names = NameStyle::Readable,
                Some("mangled") => opts.names = NameStyle::Mangled,
                other => bad_value(dcx, key, other, "readable`, `mangled"),
            },
            "js-root" => match value {
                Some(name) if !name.is_empty() => opts.roots.push(name.to_owned()),
                other => bad_value(dcx, key, other, "a JavaScript identifier"),
            },
            "js-structure" => match value {
                Some("regions") => opts.structure = Structure::Regions,
                Some("trampoline") => opts.structure = Structure::Trampoline,
                other => bad_value(dcx, key, other, "regions`, `trampoline"),
            },
            "js-queue" => match on_off(value) {
                Some(on) => opts.queue = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-switch" => match value {
                Some("dtree") => opts.switch = SwitchStyle::DTree,
                Some("flat") => opts.switch = SwitchStyle::Flat,
                other => bad_value(dcx, key, other, "dtree`, `flat"),
            },
            "js-comments" => match on_off(value) {
                Some(on) => opts.comments = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-line-comments" => match on_off_flag(value) {
                Some(on) => opts.line_comments = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-source-map" => match on_off_flag(value) {
                Some(on) => opts.source_map = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-emit-skip" => match value {
                Some(prefix) if !prefix.is_empty() => opts.emit_skip.push(prefix.to_owned()),
                other => bad_value(dcx, key, other, "a def path prefix"),
            },
            "js-scoped-lets" => match on_off_flag(value) {
                Some(on) => opts.scoped_lets = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-minify" => match value {
                // Everything but the link-time item rename, so that the two halves of the win can
                // be told apart and a minified program can still be grepped for a def path.
                Some("locals") => {
                    opts.minify = true;
                    opts.minify_items = false;
                }
                other => match on_off_flag(other) {
                    Some(on) => opts.minify = on,
                    None => bad_value(dcx, key, other, "on`, `locals`, `off"),
                },
            },
            "js-ecma-target" => match value {
                Some("2018") => opts.ecma_target = EcmaTarget::Es2018,
                Some("latest") => opts.ecma_target = EcmaTarget::Latest,
                other => bad_value(dcx, key, other, "2018`, `latest"),
            },
            "js-dts" => match on_off(value) {
                Some(on) => opts.dts = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-hoist-consts" => match on_off(value) {
                Some(on) => opts.hoist_consts = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-modules" => match value {
                Some("esm") => opts.modules = Modules::Esm,
                Some("script") => opts.modules = Modules::Script,
                other => bad_value(dcx, key, other, "esm`, `script"),
            },
            "js-shim-module" => match value {
                Some(spec) if !spec.is_empty() => opts.shim_module = spec.to_owned(),
                other => bad_value(dcx, key, other, "a module specifier"),
            },
            "js-entry-role" => match value.and_then(crate::artifact::EntryRole::parse) {
                Some(role) => opts.entry_role = role,
                None => bad_value(
                    dcx,
                    key,
                    value,
                    "main`, `worker`, `island`, `procedure`, `shared",
                ),
            },
            "js-chunk-role" => match value.and_then(|value| value.split_once(':')) {
                Some((name, role)) if is_chunk_name(name) => {
                    match crate::artifact::EntryRole::parse(role) {
                        Some(role) => opts.chunk_roles.push((name.to_owned(), role)),
                        None => bad_value(
                            dcx,
                            key,
                            value,
                            "<chunk name>:<main|worker|island|procedure|shared>",
                        ),
                    }
                }
                _ => bad_value(
                    dcx,
                    key,
                    value,
                    "<chunk name>:<main|worker|island|procedure|shared>",
                ),
            },
            "js-dom-module" => match value {
                Some(spec) if !spec.is_empty() => opts.dom_module = spec.to_owned(),
                other => bad_value(dcx, key, other, "a module specifier"),
            },
            "js-chunk" => match value.and_then(|value| value.split_once(':')) {
                Some((name, entry)) if is_chunk_name(name) && !entry.is_empty() => {
                    opts.chunks.push((name.to_owned(), entry.to_owned()));
                }
                _ => bad_value(dcx, key, value, "<chunk name>:<entry point>"),
            },
            "js-chunk-shared" => match value {
                Some(name) if is_chunk_name(name) => opts.shared_chunk = name.to_owned(),
                other => bad_value(dcx, key, other, "a chunk name"),
            },
            "js-chunk-registry" => match on_off(value) {
                Some(on) => opts.chunk_registry = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-opt" => match value {
                Some("off") => opts.optimizer = OptimizerMode::Off,
                Some("report") => opts.optimizer = OptimizerMode::Report,
                Some("on") => opts.optimizer = OptimizerMode::On,
                other => bad_value(dcx, key, other, "off`, `report`, `on"),
            },
            "js-opt-passes" => match value.and_then(OptimizerPassMask::parse) {
                Some(passes) => opts.optimizer_passes = passes,
                None => bad_value(
                    dcx,
                    key,
                    value,
                    "all`, `none`, or a comma list of `sroa`, `ast-dce`, `future-poller`, \
                     `native-representations`, and `link-dedup`",
                ),
            },
            "js-opt-warnings" => match on_off(value) {
                Some(on) => opts.optimization_warnings = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-opt-report" => match value {
                Some(path) if !path.is_empty() => opts.optimization_report = Some(path.to_owned()),
                other => bad_value(dcx, key, other, "a file path"),
            },
            "js-workspace-root" => match value {
                Some(path) if !path.is_empty() => opts.workspace_root = Some(PathBuf::from(path)),
                other => bad_value(dcx, key, other, "a directory path"),
            },
            "js-cleanup" => match on_off(value) {
                Some(on) => opts.cleanup = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-jir" => match on_off(value) {
                Some(on) => opts.jir = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-lane" => match value {
                Some("baseline") => opts.lane = Lane::Baseline,
                Some("shadow") => opts.lane = Lane::Shadow,
                Some("jir") => opts.lane = Lane::Jir,
                other => bad_value(dcx, key, other, "baseline`, `shadow`, `jir"),
            },
            "js-lane-report" => match value {
                Some(path) if !path.is_empty() => opts.lane_report = Some(path.to_owned()),
                other => bad_value(dcx, key, other, "a file path"),
            },
            "js-lane-classes" => match value.and_then(ItemClass::parse) {
                Some(classes) => opts.lane_classes = classes,
                None => bad_value(
                    dcx,
                    key,
                    value,
                    "all`, `none`, or a comma list of `scalar-leaf`, `aggregate`, `refs`, \
                     `pointers`, `drops`, `unwind`, `host`, `coroutine`, `static`, `vtable",
                ),
            },
            "js-jir-flat-abi" => match on_off(value) {
                Some(on) => opts.flat_abi = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-jir-ready-futures" => match on_off(value) {
                Some(on) => opts.ready_futures = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-jir-typed-storage" => match on_off(value) {
                Some(on) => opts.typed_storage = on,
                None => bad_value(dcx, key, value, "on`, `off"),
            },
            "js-opt-weight-alloc" => match value.and_then(|value| value.parse().ok()) {
                Some(weight) => opts.weight_alloc = weight,
                None => bad_value(dcx, key, value, "a non-negative integer"),
            },
            "js-lane-only" => match value {
                Some(names) if !names.is_empty() => {
                    for name in names.split(',').filter(|name| !name.is_empty()) {
                        opts.lane_only.push(name.to_owned());
                    }
                    opts.lane_only.sort();
                    opts.lane_only.dedup();
                }
                other => bad_value(dcx, key, other, "a comma list of emitted item names"),
            },
            "js-jir-dump-conflicts" => match value {
                Some(path) if !path.is_empty() => opts.jir_dump_conflicts = Some(path.to_owned()),
                other => bad_value(dcx, key, other, "a file path"),
            },
            "js-str" => {
                explicit_str = true;
                match value {
                    Some("utf16") => opts.str_repr = StrRepr::Utf16,
                    Some("bytes") => opts.str_repr = StrRepr::Bytes,
                    other => bad_value(dcx, key, other, "utf16`, `bytes"),
                }
            }
            // Already resolved, ahead of this loop, so that an explicit option wins.
            "js-profile" => {}
            _ => {
                dcx.warn(&format!(
                    "`-Cllvm-args={arg}` is not understood by rustc_codegen_js"
                ));
            }
        }
    }

    // A split program's pieces reach each other by `import`, so there is no split without modules,
    // and a name collision between two chunks would have them overwrite each other's file.
    if !opts.chunks.is_empty() {
        if opts.modules != Modules::Esm {
            dcx.err(
                "`-Cllvm-args=js-chunk` needs `-Cllvm-args=js-modules=esm`: a chunk imports what \
                 the other chunks hold, and a plain script cannot",
            );
        }
        for (index, (name, _)) in opts.chunks.iter().enumerate() {
            if name == &opts.shared_chunk {
                dcx.err(&format!(
                    "the chunk `{name}` has the same name as the shared chunk; rename it or pass \
                     `-Cllvm-args=js-chunk-shared=<other name>`"
                ));
            }
            if opts.chunks[..index]
                .iter()
                .any(|(earlier, _)| earlier == name)
            {
                dcx.err(&format!("`-Cllvm-args=js-chunk={name}:..` was given twice"));
            }
        }
    }

    for (index, (name, _)) in opts.chunk_roles.iter().enumerate() {
        if !opts.chunks.iter().any(|(chunk, _)| chunk == name) {
            dcx.err(&format!(
                "`js-chunk-role` names an undeclared chunk: {name}"
            ));
        }
        if opts.chunk_roles[..index]
            .iter()
            .any(|(earlier, _)| earlier == name)
        {
            dcx.err(&format!("`js-chunk-role` was given twice for {name}"));
        }
    }

    // Minification turns off header comments: they are the larger half of a small item and say
    // nothing a minified program can use. Source maps remain independent. The local renamer moves
    // their emitted-name keys with the AST and the link-time item renamer rebases generated
    // columns, so compact output retains the same Rust locations and names.
    // Phase 3 note: `js-lane=jir` and `js-opt=on` were refused together through Phase 2, because
    // the v1 optimizer rewrites PRINTED baseline JavaScript that the JIR lane does not print, and
    // two rewriters may not own one item. The rule lifted when the rewrites were re-landed as JIR
    // passes (`jsc-link/src/opt/`): on the jir lane the frozen text optimizer never runs — the
    // link step gates `representation_plan`/`late_lower` on `Lane::Baseline` — and the shadow
    // lane's baseline half still gets the v1 rewrites, its JIR half the new passes.

    // The shadow lane writes a second program beside the first, and a chunked build's file names
    // are fixed by the chunk names rather than derived from the output path, so the two lanes
    // would write each other's chunks. Refused rather than raced.
    if opts.lane == Lane::Shadow && !opts.chunks.is_empty() {
        dcx.err(
            "`-Cllvm-args=js-lane=shadow` cannot be combined with `-Cllvm-args=js-chunk`: both \
             lanes would write the same chunk files. Compare the lanes unsplit, or run the two \
             lanes as two builds",
        );
    }

    // A lane with no bodies to lower is a build that measures the baseline and calls it the lane.
    if opts.lane != Lane::Baseline && !opts.jir {
        dcx.warn(
            "`-Cllvm-args=js-lane` with `js-jir=off` falls back to the baseline for every item: \
             the objects carry no JIR bodies to lower",
        );
    }

    // A lane report describes a choice only the link step makes, so asking for one without a lane
    // to report on is a build script that will read an empty file and believe it.
    if opts.lane_report.is_some() && !opts.jir {
        dcx.warn(
            "`-Cllvm-args=js-lane-report` with `js-jir=off` reports every item as unavailable: \
             the objects carry no JIR bodies to choose from",
        );
    }

    // Release profiles default to byte strings on the JIR lane. If an explicit `js-lane` or
    // `js-jir=off` takes compilation off that lane without spelling `js-str`, revert to UTF-16
    // rather than failing validation.
    if !explicit_str && (opts.lane != Lane::Jir || !opts.jir) {
        opts.str_repr = StrRepr::Utf16;
    }

    // Byte strings exist only in the JIR lowering: the baseline text every object carries holds a
    // `str` as UTF-16 text, and a program that printed some items from it would mix the two.
    if opts.str_repr == StrRepr::Bytes && opts.lane != Lane::Jir {
        dcx.err(
            "`-Cllvm-args=js-str=bytes` needs `-Cllvm-args=js-lane=jir`: byte strings are a \
             JIR-lane lowering, and the baseline lane prints a `str` as UTF-16 text",
        );
    }

    if opts.minify {
        opts.comments = false;
    }

    opts
}

/// Whether `name` can be both a chunk's file name and part of a module specifier.
///
/// The same rule mires-build checks the request against before it is ever passed here, so a name that
/// gets this far is one both sides agree on.
fn is_chunk_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
}

/// `on`/`off`, with no value at all meaning "not given".
fn on_off(value: Option<&str>) -> Option<bool> {
    match value {
        Some("on" | "true" | "yes") => Some(true),
        Some("off" | "false" | "no") => Some(false),
        _ => None,
    }
}

/// Like [`on_off`], but a bare `js-flag` with no `=value` means `on`.
fn on_off_flag(value: Option<&str>) -> Option<bool> {
    match value {
        None => Some(true),
        other => on_off(other),
    }
}

fn bad_value(dcx: &dyn Diagnostics, key: &str, value: Option<&str>, expected: &str) {
    match value {
        Some(value) => dcx.err(&format!(
            "`{value}` is not a valid `{key}` (expected `{expected}`)"
        )),
        None => dcx.err(&format!(
            "`-Cllvm-args={key}` needs a value (expected `{expected}`)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A [`Diagnostics`] that keeps what was said, so a rule can be asserted on rather than
    /// watched for.
    #[derive(Default)]
    struct Collected {
        errors: RefCell<Vec<String>>,
        warnings: RefCell<Vec<String>>,
    }

    impl Diagnostics for Collected {
        fn err(&self, message: &str) {
            self.errors.borrow_mut().push(message.to_owned());
        }

        fn warn(&self, message: &str) {
            self.warnings.borrow_mut().push(message.to_owned());
        }
    }

    /// Parses a whitespace separated command line, as rustc splits `-Cllvm-args`.
    fn parse_line(line: &str) -> (Opts, Collected) {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        let collected = Collected::default();
        let opts = parse_args(&args, &collected);
        (opts, collected)
    }

    fn opts_of(line: &str) -> Opts {
        let (opts, collected) = parse_line(line);
        assert_eq!(collected.errors.borrow().as_slice(), [] as [String; 0]);
        opts
    }

    #[test]
    fn producer_identity_is_validated_without_selecting_an_emission_policy() {
        let digest = "0123456789abcdef".repeat(4);
        let args = vec!["js-opt=on".to_owned(), format!("js-producer={digest}")];
        let diagnostics = Collected::default();
        let opts = parse_args(&args, &diagnostics);
        assert!(diagnostics.errors.borrow().is_empty());
        assert!(diagnostics.warnings.borrow().is_empty());
        assert_eq!(format!("{opts:?}"), format!("{:?}", opts_of("js-opt=on")));
        assert_eq!(crate::item::normalized_emit_args(&args), "js-opt=on");
        for value in ["", "00", &"A".repeat(64), &"g".repeat(64)] {
            let (_, diagnostics) = parse_line(&format!("js-producer={value}"));
            assert_eq!(diagnostics.errors.borrow().len(), 1, "{value}");
        }
    }

    #[test]
    fn the_lane_options_default_to_carrying_jir_and_printing_the_baseline() {
        let opts = opts_of("");

        assert!(opts.jir);
        assert!(!opts.cleanup);
        assert_eq!(opts.ecma_target, EcmaTarget::Latest);
        assert_eq!(opts.lane, Lane::Baseline);
        assert_eq!(opts.lane_report, None);
        assert_eq!(opts.jir_dump_conflicts, None);
        assert_eq!(opts.profile, None);
    }

    #[test]
    fn every_lane_option_is_parsed_and_validated() {
        let opts = opts_of(
            "js-jir=off js-lane=shadow js-lane-report=/out/lane.json \
             js-jir-dump-conflicts=/out/conflicts.json",
        );
        assert!(!opts.jir);
        assert_eq!(opts.lane, Lane::Shadow);
        assert_eq!(opts.lane_report.as_deref(), Some("/out/lane.json"));
        assert_eq!(
            opts.jir_dump_conflicts.as_deref(),
            Some("/out/conflicts.json")
        );

        assert!(opts_of("js-jir=on").jir);
        assert!(opts_of("js-cleanup=on").cleanup);
        assert!(!opts_of("js-cleanup=off").cleanup);
        assert!(!opts_of("js-cleanup=on js-cleanup=off").cleanup);
        assert_eq!(opts_of("js-lane=baseline").lane, Lane::Baseline);
        assert_eq!(opts_of("js-lane=jir").lane, Lane::Jir);

        // The promoted set defaults to every class and narrows to what a promotion round names.
        assert_eq!(Opts::default().lane_classes, ItemClass::all());
        assert!(Opts::default().lane_classes.contains(ItemClass::VTABLE));
        assert_eq!(
            opts_of("js-lane-classes=scalar-leaf,aggregate").lane_classes,
            ItemClass::SCALAR_LEAF.union(ItemClass::AGGREGATE)
        );
        assert_eq!(
            opts_of("js-lane-classes=none").lane_classes,
            ItemClass::empty()
        );
        assert_eq!(
            opts_of("js-lane-classes=all").lane_classes,
            ItemClass::all()
        );

        // Names are sorted and deduplicated, whether they arrive in one option or several.
        assert_eq!(
            opts_of("js-lane-only=b$h2,a$h1 js-lane-only=a$h1").lane_only,
            vec!["a$h1".to_owned(), "b$h2".to_owned()]
        );
        assert!(Opts::default().lane_only.is_empty());

        for (line, expected) in [
            ("js-lane=fast", "`fast` is not a valid `js-lane`"),
            ("js-jir=maybe", "`maybe` is not a valid `js-jir`"),
            ("js-cleanup=maybe", "`maybe` is not a valid `js-cleanup`"),
            (
                "js-lane-report",
                "`-Cllvm-args=js-lane-report` needs a value",
            ),
            (
                "js-jir-dump-conflicts",
                "`-Cllvm-args=js-jir-dump-conflicts` needs a value",
            ),
            (
                "js-lane-classes=scalars",
                "`scalars` is not a valid `js-lane-classes`",
            ),
            ("js-lane-only", "`-Cllvm-args=js-lane-only` needs a value"),
        ] {
            let (_, collected) = parse_line(line);
            let errors = collected.errors.borrow();
            assert_eq!(errors.len(), 1, "{line}: {errors:?}");
            assert!(errors[0].starts_with(expected), "{line}: {errors:?}");
        }
    }

    #[test]
    fn release_cleanup_is_default_and_an_explicit_opt_out_wins() {
        for profile in ["release-size", "release-balanced", "release-speed"] {
            assert!(opts_of(&format!("js-profile={profile}")).cleanup);
            for line in [
                format!("js-profile={profile} js-cleanup=off"),
                format!("js-cleanup=off js-profile={profile}"),
            ] {
                assert!(!opts_of(&line).cleanup, "{line}");
            }
        }
    }

    #[test]
    fn a_profile_expands_to_the_options_it_names() {
        let dev = opts_of("js-profile=dev");
        assert_eq!(dev.profile, Some(Profile::Dev));
        assert_eq!(dev.optimizer, OptimizerMode::Off);
        assert_eq!(dev.lane, Lane::Baseline);
        assert_eq!(dev.str_repr, StrRepr::Utf16);
        assert!(!dev.minify);

        let report = opts_of("js-profile=report");
        assert_eq!(report.optimizer, OptimizerMode::Report);
        assert_eq!(report.lane, Lane::Baseline);
        assert_eq!(report.str_repr, StrRepr::Utf16);

        for name in ["release-size", "release-balanced"] {
            let opts = opts_of(&format!("js-profile={name}"));
            assert_eq!(opts.ecma_target, EcmaTarget::Es2018, "{name}");
            assert_eq!(opts.lane, Lane::Jir, "{name}");
            assert_eq!(opts.str_repr, StrRepr::Bytes, "{name}");
            assert!(opts.minify, "{name}");
            assert!(opts.minify_items, "{name}");
            assert_eq!(opts.optimizer, OptimizerMode::On, "{name}");
            // Minification decides the header comments, and it does so after the profile.
            assert!(!opts.comments, "{name}");
        }

        let speed = opts_of("js-profile=release-speed");
        assert_eq!(speed.ecma_target, EcmaTarget::Es2018);
        assert_eq!(speed.lane, Lane::Jir);
        assert_eq!(speed.str_repr, StrRepr::Bytes);
        assert!(speed.minify);
        assert!(!speed.minify_items);
        assert_eq!(speed.optimizer, OptimizerMode::On);

        let (_, collected) = parse_line("js-profile=release");
        assert_eq!(collected.errors.borrow().len(), 1);
        let (_, collected) = parse_line("js-profile");
        assert_eq!(collected.errors.borrow().len(), 1);
    }

    #[test]
    fn release_profiles_default_to_byte_strings_while_overrides_and_non_jir_keep_utf16() {
        assert_eq!(opts_of("").str_repr, StrRepr::Utf16);
        assert_eq!(opts_of("js-lane=jir").str_repr, StrRepr::Utf16);

        for profile in ["release-size", "release-balanced", "release-speed"] {
            let default_opts = opts_of(&format!("js-profile={profile}"));
            assert_eq!(default_opts.lane, Lane::Jir, "{profile}");
            assert_eq!(default_opts.str_repr, StrRepr::Bytes, "{profile}");

            for line in [
                format!("js-profile={profile} js-str=utf16"),
                format!("js-str=utf16 js-profile={profile}"),
            ] {
                let overridden = opts_of(&line);
                assert_eq!(overridden.lane, Lane::Jir, "{line}");
                assert_eq!(overridden.str_repr, StrRepr::Utf16, "{line}");
            }

            for line in [
                format!("js-profile={profile} js-lane=baseline"),
                format!("js-lane=baseline js-profile={profile}"),
                format!("js-profile={profile} js-lane=shadow"),
            ] {
                let non_jir = opts_of(&line);
                assert_ne!(non_jir.lane, Lane::Jir, "{line}");
                assert_eq!(non_jir.str_repr, StrRepr::Utf16, "{line}");
            }

            let (no_jir, collected) = parse_line(&format!("js-profile={profile} js-jir=off"));
            assert!(collected.errors.borrow().is_empty(), "{profile}");
            assert_eq!(no_jir.str_repr, StrRepr::Utf16, "{profile}");

            let (_, explicit_err) = parse_line(&format!("js-profile={profile} js-str=bytes js-lane=baseline"));
            assert_eq!(explicit_err.errors.borrow().len(), 1, "{profile}");
        }

        let (_, collected) = parse_line("js-str=bytes js-lane=baseline");
        assert_eq!(collected.errors.borrow().len(), 1);

        // `js-str` is link-only and never joins normalized emit args or sysroot keys.
        let with_bytes = ["js-profile=release-size".to_owned(), "js-str=bytes".to_owned()];
        let with_utf16 = ["js-profile=release-size".to_owned(), "js-str=utf16".to_owned()];
        assert_eq!(
            crate::item::normalized_emit_args(&with_bytes),
            crate::item::normalized_emit_args(&with_utf16)
        );
    }

    #[test]
    fn minification_preserves_source_maps() {
        let (opts, collected) = parse_line("js-minify=on js-source-map=on");

        assert!(opts.minify);
        assert!(opts.source_map);
        assert!(collected.errors.borrow().is_empty());
        assert!(collected.warnings.borrow().is_empty());
    }

    #[test]
    fn an_explicit_option_beats_the_profile_on_either_side_of_it() {
        for line in [
            "js-profile=release-size js-minify=off js-lane=baseline js-ecma-target=latest",
            "js-minify=off js-ecma-target=latest js-lane=baseline js-profile=release-size",
        ] {
            let opts = opts_of(line);
            assert_eq!(opts.profile, Some(Profile::ReleaseSize), "{line}");
            assert!(!opts.minify, "{line}");
            assert_eq!(opts.ecma_target, EcmaTarget::Latest, "{line}");
            assert_eq!(opts.lane, Lane::Baseline, "{line}");
            // Nothing the profile decided and the command line did not is disturbed.
            assert_eq!(opts.optimizer, OptimizerMode::On, "{line}");
        }

        let opts = opts_of("js-opt=report js-profile=dev");
        assert_eq!(opts.optimizer, OptimizerMode::Report);
        assert_eq!(opts.profile, Some(Profile::Dev));

        // Profiles are overlaid in order: a later one wins for every option it names, and leaves
        // the options it does not name where the earlier one put them.
        let opts = opts_of("js-profile=dev js-profile=report");
        assert_eq!(opts.profile, Some(Profile::Report));
        assert_eq!(opts.optimizer, OptimizerMode::Report);
        let opts = opts_of("js-profile=release-size js-profile=report");
        assert_eq!(opts.optimizer, OptimizerMode::Report);
        assert!(opts.minify);
    }

    #[test]
    fn the_ecma_target_is_validated() {
        assert_eq!(
            opts_of("js-ecma-target=2018").ecma_target,
            EcmaTarget::Es2018
        );
        assert_eq!(
            opts_of("js-ecma-target=latest").ecma_target,
            EcmaTarget::Latest
        );
        for line in ["js-ecma-target=2021", "js-ecma-target"] {
            let (_, collected) = parse_line(line);
            assert_eq!(collected.errors.borrow().len(), 1, "{line}");
        }
    }

    #[test]
    fn chunk_entry_registration_is_explicit() {
        assert!(!opts_of("").chunk_registry);
        assert!(opts_of("js-chunk-registry=on").chunk_registry);
        assert!(!opts_of("js-chunk-registry=off").chunk_registry);
        let (_, collected) = parse_line("js-chunk-registry=maybe");
        assert_eq!(collected.errors.borrow().len(), 1);
    }

    /// Phase 2 refused this combination — one item, two rewriters. Phase 3 re-landed the rewrites
    /// as JIR passes, so the lane runs them itself and the refusal is gone. The test stays,
    /// inverted: re-introducing the error would be a silent loss of every pass on the lane.
    #[test]
    fn the_jir_lane_runs_the_optimizer_itself() {
        for line in [
            "js-lane=jir js-opt=on",
            // The shadow lane's two halves each get their own: the baseline the frozen v1 text
            // rewrites, the JIR half the passes in `jsc-link/src/opt/`.
            "js-lane=shadow js-opt=on",
            "js-profile=release-size js-opt=on",
            "js-lane=jir js-opt=report",
            "js-opt=on js-profile=release-size js-opt=off",
        ] {
            let (_, collected) = parse_line(line);
            assert_eq!(
                collected.errors.borrow().as_slice(),
                [] as [String; 0],
                "{line}"
            );
        }
    }

    #[test]
    fn the_flat_call_abi_is_on_by_default_and_can_be_turned_off() {
        assert!(opts_of("").flat_abi);
        assert!(!opts_of("js-jir-flat-abi=off").flat_abi);
        let (_, collected) = parse_line("js-jir-flat-abi=maybe");
        assert_eq!(collected.errors.borrow().len(), 1);
    }

    #[test]
    fn exact_numeric_storage_is_an_explicit_link_only_opt_in() {
        assert!(!opts_of("").typed_storage);
        assert!(opts_of("js-jir-typed-storage=on").typed_storage);
        assert!(!opts_of("js-jir-typed-storage=off").typed_storage);
        let (_, collected) = parse_line("js-jir-typed-storage=maybe");
        assert_eq!(collected.errors.borrow().len(), 1);
        let args = ["js-jir-typed-storage=on".to_owned(), "js-opt=on".to_owned()];
        assert_eq!(crate::item::normalized_emit_args(&args), "js-opt=on");
    }

    #[test]
    fn ready_future_drivers_are_an_explicit_link_only_opt_in() {
        assert!(!opts_of("").ready_futures);
        assert!(opts_of("js-jir-ready-futures=on").ready_futures);
        assert!(!opts_of("js-jir-ready-futures=off").ready_futures);
        let (_, collected) = parse_line("js-jir-ready-futures=maybe");
        assert_eq!(collected.errors.borrow().len(), 1);
        for profile in ["dev", "release-size", "release-balanced", "release-speed"] {
            assert!(!opts_of(&format!("js-profile={profile}")).ready_futures);
        }
        let args = ["js-jir-ready-futures=on".to_owned(), "js-opt=on".to_owned()];
        assert_eq!(crate::item::normalized_emit_args(&args), "js-opt=on");
    }

    #[test]
    fn the_alloc_weight_defaults_to_24_and_parses_an_integer() {
        assert_eq!(opts_of("").weight_alloc, 24);
        assert_eq!(opts_of("js-opt-weight-alloc=40").weight_alloc, 40);
        assert_eq!(opts_of("js-opt-weight-alloc=0").weight_alloc, 0);
        let (_, collected) = parse_line("js-opt-weight-alloc=heavy");
        assert_eq!(collected.errors.borrow().len(), 1);
        // Link-only: the cost model runs at link time, so the option must never join the
        // emit-args key two objects are held to agree on.
        let args = ["js-opt-weight-alloc=40".to_owned(), "js-opt=on".to_owned()];
        assert_eq!(crate::item::normalized_emit_args(&args), "js-opt=on");
    }

    #[test]
    fn the_shadow_lane_and_chunks_cannot_both_name_the_output_files() {
        let (_, collected) = parse_line("js-lane=shadow js-modules=esm js-chunk=main:rust_entry");
        let errors = collected.errors.borrow();
        assert!(
            errors.iter().any(|error| error.contains("js-lane=shadow")
                && error.contains("same chunk files")),
            "{errors:?}"
        );

        // Either lane on its own is fine: only the pair would write one file twice.
        let (_, collected) = parse_line("js-lane=jir js-modules=esm js-chunk=main:rust_entry");
        assert!(collected.errors.borrow().is_empty());
    }

    #[test]
    fn a_lane_without_jir_bodies_is_warned_about() {
        let (_, collected) = parse_line("js-jir=off js-lane=jir");
        let warnings = collected.warnings.borrow();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("falls back to the baseline for every item")),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_lane_report_without_jir_bodies_is_warned_about() {
        let (_, collected) = parse_line("js-jir=off js-lane-report=/out/lane.json");
        let warnings = collected.warnings.borrow();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("no JIR bodies"), "{warnings:?}");

        let (_, collected) = parse_line("js-lane-report=/out/lane.json");
        assert_eq!(collected.warnings.borrow().as_slice(), [] as [String; 0]);
    }

    #[test]
    fn an_unknown_key_still_warns_rather_than_being_ignored() {
        let (_, collected) = parse_line("js-jir-lane=on");
        let warnings = collected.warnings.borrow();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("is not understood"), "{warnings:?}");
    }

    #[test]
    fn workspace_sources_are_compared_after_lexical_normalization() {
        let root = Path::new("examples/optimizer-tests/workspace");

        assert!(is_source_within(
            root,
            Path::new("./examples/optimizer-tests/workspace/diagnostics.rs")
        ));
        assert!(is_source_within(
            root,
            Path::new("examples/optimizer-tests/workspace/nested/../diagnostics.rs")
        ));
        assert!(!is_source_within(
            root,
            Path::new("examples/optimizer-tests/workspace/../dependency.rs")
        ));
        assert!(!is_source_within(
            root,
            Path::new("examples/optimizer-tests/workspace-other/source.rs")
        ));
    }

    #[test]
    fn current_directory_root_does_not_admit_a_parent_source() {
        assert!(is_source_within(Path::new("."), Path::new("src/lib.rs")));
        assert!(!is_source_within(
            Path::new("."),
            Path::new("../dependency/src/lib.rs")
        ));
        assert!(!is_source_within(
            Path::new("."),
            Path::new("/absolute/src/lib.rs")
        ));
    }

    #[test]
    fn optimizer_pass_masks_are_canonical_and_deduplicated() {
        let mask = OptimizerPassMask::parse(
            "link-dedup,native-representations,future-poller,ast-dce,sroa,future-poller",
        )
        .unwrap();
        let names: Vec<_> = mask.ordered().map(OptimizerPass::as_str).collect();

        assert_eq!(
            names,
            [
                "sroa",
                "ast-dce",
                "future-poller",
                "native-representations",
                "link-dedup"
            ]
        );
        assert_eq!(OptimizerPassMask::parse("all"), Some(mask));
        assert_eq!(OptimizerPassMask::parse("none"), Some(OptimizerPassMask(0)));
        assert_eq!(OptimizerPassMask::parse("unknown"), None);
    }

    #[test]
    fn optimizer_modes_control_analysis_and_application_separately() {
        let mut opts = Opts {
            optimizer_passes: OptimizerPassMask::parse("ast-dce").unwrap(),
            ..Opts::default()
        };
        assert!(!opts.analyzes(OptimizerPass::AstDce));
        assert!(!opts.applies(OptimizerPass::AstDce));

        opts.optimizer = OptimizerMode::Report;
        assert!(opts.analyzes(OptimizerPass::AstDce));
        assert!(!opts.applies(OptimizerPass::AstDce));
        assert!(!opts.analyzes(OptimizerPass::FuturePoller));

        opts.optimizer = OptimizerMode::On;
        assert!(opts.analyzes(OptimizerPass::AstDce));
        assert!(opts.applies(OptimizerPass::AstDce));
        assert!(!opts.applies(OptimizerPass::FuturePoller));
    }
}
