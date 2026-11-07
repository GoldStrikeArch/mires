//! Local names: the readable half of a function body.
//!
//! `naming.rs` invents the names of *items*; this module invents the names of the JavaScript
//! locals inside one. A MIR body carries `var_debug_info`, the mapping a debugger would use, and
//! for the entries shaped `name => _N` — a bare place, no projection — that mapping is exactly the
//! name the programmer wrote. Using it turns
//!
//! ```js
//! _5 = _5 + _2 | 0;
//! ```
//!
//! into
//!
//! ```js
//! acc = acc + n | 0;
//! ```
//!
//! # Does the debug info survive?
//!
//! Yes, measured rather than assumed. `var_debug_info` is part of the MIR body, not of the debug
//! information rustc hands a code generator, so `-Cdebuginfo=0` (what `scripts/compile.sh` passes)
//! does not touch it, and it round trips through an rlib with the rest of the body. Inlining
//! (`-Zinline-mir`, which `scripts/build_sysroot.sh` uses to build `core`) *appends* the callee's
//! entries to the caller's rather than dropping either. Dumping the final MIR of the test suite
//! with `-Zdump-mir=all` under both flag sets found every entry still there and every one of them
//! of the bare `name => _N` shape.
//!
//! What inlining does introduce is repetition: two inlined callees both name a local `self`. That
//! is what the `$1`, `$2` suffixes below are for.
//!
//! # The rules
//!
//! A name is only ever *added* to a body that would otherwise read `_0`, `_1`, ..., so every rule
//! here is about not breaking something that already works:
//!
//! * `_0` keeps its name. It is the return place, `emit.rs` spells `return _0` by asking for it,
//!   and it is not a variable the programmer declared.
//! * a local name never matches `^_[0-9]`. Rust allows `let _3 = ...;`, and a local named `_3`
//!   would collide with the *fallback* name of MIR local 3.
//! * a local name never starts with `$`. That namespace belongs to the compiler's own temporaries
//!   (`$t0` from `place.rs`, `$s0` from `emit.rs`, `$loc` from `intrinsics.rs`), which are declared
//!   in the same scope.
//! * a local name is never `bb`, never a JavaScript reserved word, and never one of the globals
//!   the emitted code reads ([`RESERVED`]) — `__rt` above all, which every shim call goes through.
//! * two locals never get the same name: collisions are resolved with a `$1`, `$2`, ... suffix,
//!   numbered per function so that a body's names do not depend on anything outside it.
//!
//! An item name cannot collide with a local, because every item name carries the `$h` hash infix
//! (`naming.rs`) and a Rust identifier cannot contain a `$`. The exception is an item that is
//! `#[no_mangle]` or `#[export_name]`, which keeps its exact spelling: a local named `rust_entry`
//! inside a function that calls `rust_entry` would shadow it. Nothing here can see that name, so
//! it is a documented hazard rather than a handled one — `-Cllvm-args=js-names=mangled` turns all
//! of this off and is the way to tell such a bug apart from a lowering bug.

use std::collections::HashSet;

use rustc_middle::mir::{Body, Local, RETURN_PLACE, VarDebugInfoContents};
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

use crate::jsast::{self, Loc, Stmt};
use crate::opts::NameStyle;

/// Globals the emitted JavaScript reads, which a local of the same name would shadow.
///
/// `__rt` is the runtime shim — every `__rt.js_abort(..)` in the body would resolve to the local
/// instead. The rest are the builtins the backend spells out in `jsast.rs`, `value.rs` and
/// `rvalue.rs`.
const RESERVED: &[&str] = &[
    "__rt",
    "Array",
    "ArrayBuffer",
    "BigInt",
    "Boolean",
    "DataView",
    "Error",
    "Infinity",
    "Math",
    "NaN",
    "Number",
    "Object",
    "String",
    "Symbol",
    "arguments",
    "bb",
    "console",
    "eval",
    "globalThis",
    "undefined",
];

/// One function's local names, indexed by MIR local.
pub(crate) struct LocalNames {
    names: Vec<String>,
    /// The Rust name behind a JavaScript one, for every local whose spelling had to change. Feeds
    /// the source map's `names` table, which is how a debugger shows `self` where the output says
    /// `self$1`. A local that kept its Rust spelling exactly is not listed: nothing to say.
    originals: Vec<(String, String)>,
}

impl LocalNames {
    /// The fallback naming: `_0`, `_1`, ... for every local, as the spike emitted them.
    pub(crate) fn numbered(mir: &Body<'_>) -> LocalNames {
        LocalNames {
            names: mir.local_decls.indices().map(|local| fallback(local)).collect(),
            originals: Vec::new(),
        }
    }

    /// The readable naming: `var_debug_info` where it says something, `_N` everywhere else.
    pub(crate) fn new(mir: &Body<'_>, style: NameStyle) -> LocalNames {
        if style == NameStyle::Mangled {
            return LocalNames::numbered(mir);
        }

        // First wins. A local that two debug entries name — which inlining produces — reads as the
        // outermost one, the name the source of *this* function used.
        // The rescued spelling and the Rust name it came from: the second is what a debugger is
        // told the first stands for.
        let mut wanted: Vec<Option<(String, String)>> = vec![None; mir.local_decls.len()];
        for info in &mir.var_debug_info {
            let VarDebugInfoContents::Place(place) = info.value else { continue };
            // A projection means the name is not this local but a piece of it — a closure capture,
            // a field of a scattered aggregate. Naming the whole local after the piece would lie.
            if !place.projection.is_empty() {
                continue;
            }
            let index = place.local.as_usize();
            // `_0` is the return place, not a binding, and `emit.rs` asks for it by that name.
            if place.local == RETURN_PLACE || index >= wanted.len() {
                continue;
            }
            if wanted[index].is_none() {
                let raw = info.name.as_str();
                wanted[index] = rescue(raw).map(|name| (name, raw.to_owned()));
            }
        }

        // Every local that keeps its number holds that number's spelling, so a debug name that
        // happens to read `_7` cannot take it. (`rescue` already refuses that shape; this also
        // covers the collision suffixes.)
        let mut taken: HashSet<String> = HashSet::with_capacity(mir.local_decls.len());
        for local in mir.local_decls.indices() {
            if wanted[local.as_usize()].is_none() {
                taken.insert(fallback(local));
            }
        }
        taken.insert(fallback(RETURN_PLACE));

        let mut names = Vec::with_capacity(mir.local_decls.len());
        let mut originals = Vec::new();
        for local in mir.local_decls.indices() {
            let name = match wanted[local.as_usize()].take() {
                Some((base, raw)) => {
                    let name = unique(base, &mut taken);
                    if name != raw {
                        originals.push((name.clone(), raw));
                    }
                    name
                }
                None => fallback(local),
            };
            names.push(name);
        }
        LocalNames { names, originals }
    }

    /// The JavaScript name to Rust name pairs worth putting in a source map's `names` table.
    pub(crate) fn originals(&self) -> &[(String, String)] {
        &self.originals
    }

    /// The JavaScript name of a MIR local.
    ///
    /// A local outside the body's range is a backend bug rather than a user error, and one that
    /// would otherwise print as a silent `undefined`; the numbered spelling keeps it visible.
    pub(crate) fn get(&self, local: Local) -> String {
        match self.names.get(local.as_usize()) {
            Some(name) => name.clone(),
            None => fallback(local),
        }
    }
}

/// The name a local has when nothing better is known.
fn fallback(local: Local) -> String {
    format!("_{}", local.as_usize())
}

/// Turns a Rust binding name into one that is safe to declare beside the backend's own names.
///
/// Returns `None` for a name that cannot be rescued into anything readable, which leaves the local
/// numbered.
fn rescue(raw: &str) -> Option<String> {
    // A generated binding (`_`, or a `let _x` the user asked not to be warned about) carries no
    // information the number does not.
    if raw.is_empty() || raw == "_" {
        return None;
    }

    // Handles the characters, the leading digit and the reserved words. A Rust identifier is
    // already a JavaScript identifier except for those, so this is normally the identity.
    let mut name = jsast::sanitize_ident(raw);

    // `$` is the compiler's namespace and `_0` is a MIR local's; a Rust identifier can be neither
    // by accident, but `let r#_0 = ..` is legal and `sanitize_ident` would let it through.
    let looks_like_local =
        name.starts_with('_') && name[1..].starts_with(|c: char| c.is_ascii_digit());
    if name.starts_with('$') || looks_like_local {
        name.insert(0, 'x');
    }
    // `bb` and the globals: a suffix rather than a prefix, so `__rt` stays recognizable as `__rt_`.
    if RESERVED.contains(&name.as_str()) {
        name.push('_');
    }

    debug_assert!(jsast::is_plain_ident(&name), "`{name}` is not a JavaScript identifier");
    Some(name)
}

/// The first of `base`, `base$1`, `base$2`, ... that is free, marking it taken.
fn unique(base: String, taken: &mut HashSet<String>) -> String {
    if taken.insert(base.clone()) {
        return base;
    }
    for n in 1.. {
        let candidate = format!("{base}${n}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the suffixes are unbounded")
}

/// The `-Cllvm-args=js-line-comments` emitter: a `// file.rs:LINE` before each run of statements.
///
/// The interim stand in for a source map — enough to find the Rust line a run of JavaScript came
/// from, cheap enough to leave in a golden file. One comment per *run*: consecutive statements
/// lowered from the same line share one, which is what keeps a body readable rather than doubling
/// its line count. The state is per basic block, because that is the granularity at which the
/// backend still has the spans (`base.rs::codegen_block`); a block the structurizer moves takes
/// its comments with it.
///
/// The file name is spelled as rustc would spell it in a diagnostic, so `--remap-path-prefix`
/// applies and a golden file does not depend on where the checkout lives.
pub(crate) struct LineComments<'tcx> {
    tcx: Option<TyCtxt<'tcx>>,
    last: Option<String>,
}

impl<'tcx> LineComments<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>) -> LineComments<'tcx> {
        let on = crate::opts::get().line_comments;
        LineComments { tcx: on.then_some(tcx), last: None }
    }

    /// Appends the comment for `span`, unless it would repeat the last one or the option is off.
    pub(crate) fn at(&mut self, out: &mut Vec<Stmt>, span: Span) {
        let Some(tcx) = self.tcx else { return };
        if span.is_dummy() {
            return;
        }
        let location = tcx.sess.source_map().lookup_char_pos(span.lo());
        let file = location.file.name.prefer_remapped_unconditionally().to_string();
        let text = format!("{file}:{}", location.line);
        if self.last.as_deref() == Some(text.as_str()) {
            return;
        }
        out.push(jsast::comment(text.clone()));
        self.last = Some(text);
    }
}

/// The `-Cllvm-args=js-source-map=on` emitter: a [`Stmt::Loc`] marker before each run of
/// statements.
///
/// The same seam and the same rule as [`LineComments`] — one marker per run of statements from one
/// Rust position, emitted where the span is still in hand — with the comment replaced by something
/// the printer turns into a source map entry instead of into text. Both can be on at once; the
/// marker prints nothing, so the only difference in the JavaScript is the comments.
///
/// A marker travels with its statements when the structurizer moves the block they are in, which
/// is what makes statement granularity survive the region tree. It does *not* travel when the
/// expression queue moves a single statement past it: that statement then reads as part of its
/// neighbour's line, which costs a stepping stop and nothing else.
pub(crate) struct Locs<'tcx> {
    tcx: Option<TyCtxt<'tcx>>,
    last: Option<Loc>,
}

impl<'tcx> Locs<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>) -> Locs<'tcx> {
        let on = crate::opts::get().source_map;
        Locs { tcx: on.then_some(tcx), last: None }
    }

    /// Appends the marker for `span`, unless it would repeat the last one or the option is off.
    pub(crate) fn at(&mut self, out: &mut Vec<Stmt>, span: Span) {
        let Some(tcx) = self.tcx else { return };
        let Some(loc) = location_of(tcx, span) else { return };
        if self.last.as_ref() == Some(&loc) {
            return;
        }
        out.push(Stmt::Loc(loc.clone()));
        self.last = Some(loc);
    }
}

/// Where a span starts, spelled as rustc would spell it in a diagnostic.
///
/// `None` for a dummy span and for anything that is not in a real file, which is what keeps
/// `<anon>` and the expansion of a macro with no location out of a map's `sources`.
pub(crate) fn location_of(tcx: TyCtxt<'_>, span: Span) -> Option<Loc> {
    if span.is_dummy() {
        return None;
    }
    let position = tcx.sess.source_map().lookup_char_pos(span.lo());
    let file = position.file.name.prefer_remapped_unconditionally().to_string();
    if file.is_empty() {
        return None;
    }
    Some(Loc { file, line: position.line as u32, col: position.col.0 as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescued_names_stay_out_of_the_backend_namespaces() {
        assert_eq!(rescue("count").as_deref(), Some("count"));
        assert_eq!(rescue("_").as_deref(), None);
        assert_eq!(rescue("").as_deref(), None);
        // A raw identifier can spell a MIR local or a reserved word.
        assert_eq!(rescue("_0").as_deref(), Some("x_0"));
        assert_eq!(rescue("_12abc").as_deref(), Some("x_12abc"));
        assert_eq!(rescue("bb").as_deref(), Some("bb_"));
        assert_eq!(rescue("__rt").as_deref(), Some("__rt_"));
        assert_eq!(rescue("class").as_deref(), Some("_class"));
        assert_eq!(rescue("$weird").as_deref(), Some("x$weird"));
        // `_x` is a name the programmer chose; only a bare `_` is anonymous.
        assert_eq!(rescue("_unused").as_deref(), Some("_unused"));
    }

    #[test]
    fn collisions_get_numbered_suffixes() {
        let mut taken: HashSet<String> = HashSet::new();
        assert_eq!(unique("self".to_string(), &mut taken), "self");
        assert_eq!(unique("self".to_string(), &mut taken), "self$1");
        assert_eq!(unique("self".to_string(), &mut taken), "self$2");
        assert_eq!(unique("other".to_string(), &mut taken), "other");
    }

    #[test]
    fn a_debug_name_cannot_take_a_numbered_locals_spelling() {
        let mut taken: HashSet<String> = HashSet::from(["_3".to_string()]);
        // `rescue` never produces `_3`, so the reservation is belt and braces; what it really
        // guards is the suffixed form.
        assert_eq!(unique("x_3".to_string(), &mut taken), "x_3");
        assert!(taken.contains("_3"));
    }
}
