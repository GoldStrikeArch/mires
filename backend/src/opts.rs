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
//! -Cllvm-args=js-hoist-consts=off on|off             share repeated constants (cgu.rs)
//! -Cllvm-args=js-modules=script   esm|script         ES module or plain script (link.rs)
//! -Cllvm-args=js-shim-module=...  a module specifier where `__rt` is imported from
//! -Cllvm-args=js-dom-module=...   a module specifier where the DOM runtime is imported from
//! -Cllvm-args=js-chunk=NAME:ENTRY repeatable         split the program per entry point (link.rs)
//! -Cllvm-args=js-chunk-shared=... a chunk name       where two chunks' common items go
//! ```
//!
//! The value shown above is the default. `js-switch` and `js-dom-module` are parsed, validated and
//! stored but not yet consumed; every other option is live.
//!
//! An option is **emit-affecting** when it can change the text of an item two crates might both
//! codegen, which is what makes it one that every object in a program has to agree on — the
//! sysroot's `core` included. `js-minify`, `js-structure`, `js-queue` and `js-names` are; the
//! module options are not, because everything they change is synthesized by the link step or
//! belongs to the leaf crate alone. See CONTRACT.md. The one spelled `=off` is on by default and turns *off*
//! the milestone that introduced it, which is what makes a regression bisectable: `js-queue=off`
//! and `js-structure=trampoline` each reproduce, byte for byte, the output the backend produced
//! before that milestone landed.
//!
//! `js-emit-skip` is the odd one out: it is validated here but *acted on by
//! `scripts/emit-test.sh`*, which filters the text the backend printed. It cannot be a backend
//! rule — an item left out of the program would be a `ReferenceError` at run time rather than a
//! tidier golden file, and every emit fixture is also run under node. Keeping the flag here is
//! what puts the skip list in the compile command a golden was baselined with.
//!
//! The parsed options live in a process global: the backend dylib is loaded once and rustc
//! compiles exactly one crate per process, so there is nothing to key them by. [`init`] fills it
//! from `CodegenBackend::init`, and [`get`] hands out the result (defaults if `init` never ran,
//! which only happens in unit tests).

use std::sync::OnceLock;

use rustc_session::Session;

/// How the backend spells the identifiers it invents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NameStyle {
    /// A sanitized def path plus a hash suffix: `mini_core$print_i32$h1c0ffee...`.
    Readable,
    /// The raw v0 symbol name, as the spike emitted it. Kept for bisecting.
    Mangled,
}

/// How control flow is rebuilt from the CFG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Structure {
    /// The structurizer's region tree (stage 2).
    Regions,
    /// The `switch (bb)` trampoline.
    Trampoline,
}

/// What kind of JavaScript file the program is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Modules {
    /// An ES module: the runtime shim arrives through an `import`, and the exported items are
    /// named by a trailing `export` clause.
    Esm,
    /// A plain script: `__rt` is a global the host defines before the program runs, and an
    /// exported item is a global the host calls afterwards.
    Script,
}

/// How a `SwitchInt` is spelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SwitchStyle {
    /// A decision tree over the case values (stage 2).
    DTree,
    /// One flat JS `switch` with a case per value.
    Flat,
}

#[derive(Clone, Debug)]
pub(crate) struct Opts {
    pub(crate) names: NameStyle,
    /// Extra dead-code-elimination roots, named by their emitted JS name.
    pub(crate) roots: Vec<String>,
    pub(crate) structure: Structure,
    pub(crate) queue: bool,
    pub(crate) switch: SwitchStyle,
    /// Whether each item is preceded by a `// <def path>` comment.
    pub(crate) comments: bool,
    pub(crate) line_comments: bool,
    /// Whether the finished program gets a `.map` and a `sourceMappingURL` comment.
    pub(crate) source_map: bool,
    /// Def path prefixes whose items are left out of `--emit`ted JS fixtures.
    pub(crate) emit_skip: Vec<String>,
    pub(crate) scoped_lets: bool,
    /// Whether names are shortened and the program printed without the whitespace. See `minify.rs`.
    pub(crate) minify: bool,
    /// Whether minification also renames *item* names at link time.
    ///
    /// `js-minify=locals` leaves them alone, which keeps a minified program readable enough to
    /// grep for a def path and is how the two halves of the win were measured apart. Meaningless
    /// unless [`Opts::minify`] is on.
    pub(crate) minify_items: bool,
    /// Whether repeated constants — `#[track_caller]` locations, long string literals — are
    /// emitted once as a module level `const` instead of being written out at every use. See
    /// [`crate::cgu::CguCx::hoist`].
    pub(crate) hoist_consts: bool,
    /// Whether the program is an ES module or a plain script. See [`Modules`].
    pub(crate) modules: Modules,
    /// The module the runtime shim's `__rt` namespace is imported from, in ES module output.
    pub(crate) shim_module: String,
    /// The module the DOM runtime's bindings are imported from, in ES module output.
    pub(crate) dom_module: String,
    /// The chunks to split the program into, as `(name, entry)` pairs in request order.
    ///
    /// Empty means one file, which is every compilation that does not ask to be split. See
    /// [`crate::link`].
    pub(crate) chunks: Vec<(String, String)>,
    /// The name of the chunk holding what two or more chunks both reach.
    pub(crate) shared_chunk: String,
    /// Whether the link step writes a `.d.ts` beside each output file. See [`crate::dts`].
    ///
    /// NOT emit affecting: it is synthesized at the link step from what the object files already
    /// carry, and nothing about the JavaScript changes, so it does not have to agree with the
    /// flags the sysroot was built with.
    pub(crate) dts: bool,
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
            // Off by default on measured evidence (see CONTRACT.md "Hoisted constants"): the
            // 16-hex hash names are incompressible, so hoisting loses 1-5% gzipped even where
            // it wins raw minified bytes. The mechanism stays for a future per-item reuse-count
            // gate; `--remap-path-prefix` (the actual win) is unconditional in the scripts.
            hoist_consts: false,
            modules: Modules::Script,
            shim_module: "./shim.js".to_owned(),
            dom_module: "mires-dom".to_owned(),
            chunks: Vec::new(),
            dts: false,
            shared_chunk: "shared".to_owned(),
        }
    }
}

static OPTS: OnceLock<Opts> = OnceLock::new();

/// Parses `-Cllvm-args` and installs the result. Reports bad options on `sess`.
pub(crate) fn init(sess: &Session) {
    let _ = OPTS.set(parse(sess));
}

/// The options this compilation runs with.
pub(crate) fn get() -> &'static Opts {
    OPTS.get_or_init(Opts::default)
}

fn parse(sess: &Session) -> Opts {
    let mut opts = Opts::default();
    let dcx = sess.dcx();

    for arg in &sess.opts.cg.llvm_args {
        let (key, value) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (arg.as_str(), None),
        };

        // A typo here silently changes what the backend emits, so every unknown key and every
        // unusable value gets a diagnostic rather than being ignored.
        match key {
            "js-names" => match value {
                Some("readable") => opts.names = NameStyle::Readable,
                Some("mangled") => opts.names = NameStyle::Mangled,
                other => bad_value(sess, key, other, "readable`, `mangled"),
            },
            "js-root" => match value {
                Some(name) if !name.is_empty() => opts.roots.push(name.to_owned()),
                other => bad_value(sess, key, other, "a JavaScript identifier"),
            },
            "js-structure" => match value {
                Some("regions") => opts.structure = Structure::Regions,
                Some("trampoline") => opts.structure = Structure::Trampoline,
                other => bad_value(sess, key, other, "regions`, `trampoline"),
            },
            "js-queue" => match on_off(value) {
                Some(on) => opts.queue = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-switch" => match value {
                Some("dtree") => opts.switch = SwitchStyle::DTree,
                Some("flat") => opts.switch = SwitchStyle::Flat,
                other => bad_value(sess, key, other, "dtree`, `flat"),
            },
            "js-comments" => match on_off(value) {
                Some(on) => opts.comments = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-line-comments" => match on_off_flag(value) {
                Some(on) => opts.line_comments = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-source-map" => match on_off_flag(value) {
                Some(on) => opts.source_map = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-emit-skip" => match value {
                Some(prefix) if !prefix.is_empty() => opts.emit_skip.push(prefix.to_owned()),
                other => bad_value(sess, key, other, "a def path prefix"),
            },
            "js-scoped-lets" => match on_off_flag(value) {
                Some(on) => opts.scoped_lets = on,
                None => bad_value(sess, key, value, "on`, `off"),
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
                    None => bad_value(sess, key, other, "on`, `locals`, `off"),
                },
            },
            "js-dts" => match on_off(value) {
                Some(on) => opts.dts = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-hoist-consts" => match on_off(value) {
                Some(on) => opts.hoist_consts = on,
                None => bad_value(sess, key, value, "on`, `off"),
            },
            "js-modules" => match value {
                Some("esm") => opts.modules = Modules::Esm,
                Some("script") => opts.modules = Modules::Script,
                other => bad_value(sess, key, other, "esm`, `script"),
            },
            "js-shim-module" => match value {
                Some(spec) if !spec.is_empty() => opts.shim_module = spec.to_owned(),
                other => bad_value(sess, key, other, "a module specifier"),
            },
            "js-dom-module" => match value {
                Some(spec) if !spec.is_empty() => opts.dom_module = spec.to_owned(),
                other => bad_value(sess, key, other, "a module specifier"),
            },
            "js-chunk" => match value.and_then(|value| value.split_once(':')) {
                Some((name, entry)) if is_chunk_name(name) && !entry.is_empty() => {
                    opts.chunks.push((name.to_owned(), entry.to_owned()));
                }
                _ => bad_value(sess, key, value, "<chunk name>:<entry point>"),
            },
            "js-chunk-shared" => match value {
                Some(name) if is_chunk_name(name) => opts.shared_chunk = name.to_owned(),
                other => bad_value(sess, key, other, "a chunk name"),
            },
            _ => {
                dcx.warn(format!("`-Cllvm-args={arg}` is not understood by rustc_codegen_mires"));
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
                dcx.err(format!(
                    "the chunk `{name}` has the same name as the shared chunk; rename it or pass \
                     `-Cllvm-args=js-chunk-shared=<other name>`"
                ));
            }
            if opts.chunks[..index].iter().any(|(earlier, _)| earlier == name) {
                dcx.err(format!("`-Cllvm-args=js-chunk={name}:..` was given twice"));
            }
        }
    }

    // Minification decides two options that would otherwise contradict it.
    //
    // Header comments are the larger half of a small item and say nothing a minified program can
    // use, so `js-minify` turns them off rather than leaving the caller to. A source map is a
    // harder conflict: the map is built from positions the *printer* records, and the local
    // renamer rewrites the AST before the printer ever sees it, so a map made under `js-minify`
    // would faithfully describe names that no longer exist. Renaming through the map's `names`
    // table is the right fix and is a v2 job; until then the combination is refused loudly rather
    // than served wrong.
    if opts.minify {
        opts.comments = false;
        if opts.source_map {
            dcx.warn(
                "`-Cllvm-args=js-minify=on` disables `js-source-map`: the map would name \
                 variables the minified program no longer has",
            );
            opts.source_map = false;
        }
    }

    opts
}

/// Whether `name` can be both a chunk's file name and part of a module specifier.
///
/// The same rule mires-build checks the request against before it is ever passed here, so a name that
/// gets this far is one both sides agree on.
fn is_chunk_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
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

fn bad_value(sess: &Session, key: &str, value: Option<&str>, expected: &str) {
    match value {
        Some(value) => {
            sess.dcx().err(format!("`{value}` is not a valid `{key}` (expected `{expected}`)"))
        }
        None => sess.dcx().err(format!("`-Cllvm-args={key}` needs a value (expected `{expected}`)")),
    };
}
