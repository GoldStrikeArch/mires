//! The unit of linking: one named, self contained piece of JavaScript.
//!
//! Codegen no longer produces one string per codegen unit. It produces a [`JsItem`] per mono item,
//! carrying enough structure for the whole-program pass in `link.rs` to work out what is reachable
//! and what may therefore be dropped:
//!
//! * `refs` is the set of identifiers the item's JavaScript mentions, computed **mechanically** by
//!   [`crate::jsast::visit_idents`]. It is deliberately over-approximate — locals, `$`-temps, `bb`
//!   and JS builtins all end up in it — because the link step intersects it with the table of
//!   items that actually exist. The naming invariants in `naming.rs` (an item name never starts
//!   with `$`, never matches `^_[0-9]`, is never `bb`) are what make that intersection sound.
//! * `zombies` are the things the backend could not lower. Recording rather than reporting them is
//!   the whole point: a zombie in an item nothing reaches costs nothing, and only a zombie the
//!   program can actually reach is an error. See [`Zombie`].
//! * `linkage` says what the item's name and visibility are: whether the name is fixed, whether
//!   the item is a root, and whether the module exports it. See [`Linkage`].
//!
//! # Object file format
//!
//! A codegen unit's "object file" is the concatenated printed JavaScript of its items — so it
//! stays directly runnable under `node` — followed by one trailing line:
//!
//! ```text
//! //# rcgjs: <base64 of a postcard serialized item table>
//! ```
//!
//! The table stores each item's byte range **into the JavaScript above it** rather than a second
//! copy of the text, which keeps the footer small and makes splicing at link time a slice. Spans
//! do not survive serialization (they are meaningless in another compilation session), so every
//! zombie carries a location string rendered when it was recorded.
//!
//! The payload starts with [`FORMAT_VERSION`]. postcard is not self describing, so a table written
//! in an older shape does not fail to parse, it parses into something else: fields slide, an item
//! quietly loses its name or its export, and the program that comes out is wrong rather than
//! rejected. The version turns that into a diagnostic naming the rebuild that fixes it.

use std::collections::BTreeSet;
use std::fmt;

use base64::Engine;
use rustc_middle::ty::TyCtxt;
use rustc_span::{Span, SpanData};
use serde::{Deserialize, Serialize};

use crate::jsast::{self, Expr, Loc, Mapping, Stmt};

/// The marker introducing an object file's item table.
const FOOTER_PREFIX: &str = "//# rcgjs: ";

/// The item table's format, the first element of the payload.
///
/// Bump it whenever the shape of [`SerItem`] or of anything it holds changes. Every rlib in a
/// program carries a table, so a bump makes every object built by an older backend unreadable —
/// which is the point, and why the message says how to rebuild them.
const FORMAT_VERSION: u16 = 5;

/// A JavaScript identifier the backend owns: the name of a top level item.
///
/// Distinct from a plain `String` so that the reachability pass cannot confuse an item name with,
/// say, a local. Construct one only through `naming.rs`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct JsName(String);

impl JsName {
    pub(crate) fn new(name: String) -> JsName {
        JsName(name)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for JsName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an item is, which decides how it is ordered in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ItemKind {
    /// A function declaration. Hoisted by JavaScript, so order does not matter.
    Fn,
    /// A module level `let`, from a Rust `static`. Must precede its users.
    Static,
    /// An interned vtable, emitted as a module level array.
    Vtable,
    /// A piece of the allocator shim.
    Alloc,
    /// A hoisted shared constant — a `#[track_caller]` location, a long string literal — emitted
    /// as a module level `const`. See [`crate::cgu::CguCx::hoist`].
    Const,
    /// An `import` declaration, in ES module output. Printed before everything else.
    ///
    /// One item binds one name, and the item's name *is* that binding: an import is then reachable
    /// exactly when some other item mentions what it binds, and two crates that import the same
    /// name from the same module produce one item with one text, which `link.rs` collapses like
    /// any other duplicate. The item's `debug_path` is the module specifier it imports from.
    Import,
}

impl ItemKind {
    /// Whether the item is a module level binding, which has to be initialized in order.
    pub(crate) fn is_binding(self) -> bool {
        matches!(self, ItemKind::Static | ItemKind::Vtable | ItemKind::Const)
    }

    /// The serialized tag. Append only: a value already written into an object file keeps it.
    fn tag(self) -> u8 {
        match self {
            ItemKind::Fn => 0,
            ItemKind::Static => 1,
            ItemKind::Vtable => 2,
            ItemKind::Alloc => 3,
            ItemKind::Const => 4,
            ItemKind::Import => 5,
        }
    }

    fn from_tag(tag: u8) -> Option<ItemKind> {
        Some(match tag {
            0 => ItemKind::Fn,
            1 => ItemKind::Static,
            2 => ItemKind::Vtable,
            3 => ItemKind::Alloc,
            4 => ItemKind::Const,
            5 => ItemKind::Import,
            _ => return None,
        })
    }
}

/// Where a declaration whose *order* is part of what it means came from.
///
/// A `view!` template cloner is the case, and the only one so far. The reference compiler declares
/// one per distinct template in the order the module first uses them, and a trace numbers them in
/// declaration order, so the order is observable. This backend names a cloner after a hash of its
/// HTML, which orders it arbitrarily, so it carries the source position of the `view!` that built
/// it instead and `link.rs` sorts on that.
///
/// The position is the *expansion's*, not the generated item's, so every template one `view!` built
/// shares it; [`SourceOrder::within`] is what then keeps them in the order the expansion declared
/// them, which is the order the view mentions them in.
///
/// A template several views use takes the *earliest* position of any of them, which is what "first
/// use" means. It cannot be the first one codegenned: items are codegenned in a symbol-hash order
/// that has nothing to do with the source.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct SourceOrder {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) col: u32,
    /// Which declaration of that one expansion this is, for two that share the position above.
    /// See [`crate::abi::FnCx::source_order`] for where the number comes from.
    pub(crate) within: u32,
}

/// How an item joins the program: its name, its rooting and its visibility.
///
/// The three are separate facts and only look like one because a `#[no_mangle]` item in the crate
/// being linked has all three. The same item read back out of a dependency's rlib keeps its exact
/// name, because something outside the program may still call it by that name, but is neither a
/// root nor part of this program's interface: a static archive hands out members that are asked
/// for, it does not push its own exports into the program. See [`crate::link`].
#[derive(Clone, Debug, Default)]
pub(crate) struct Linkage {
    /// The exact name the item must be emitted under. Minification never renames it.
    pub(crate) fixed_name: Option<String>,
    /// Whether the item is a dead-code-elimination root.
    pub(crate) root: bool,
    /// Whether the module's export clause names it. Only ES module output has one.
    pub(crate) exported: bool,
}

impl Linkage {
    /// An item only this program refers to, under a name only this program sees.
    pub(crate) fn internal() -> Linkage {
        Linkage::default()
    }

    /// An item the program exposes to its host under `name`: `#[no_mangle]`, `#[export_name]`.
    pub(crate) fn exported(name: String) -> Linkage {
        Linkage { fixed_name: Some(name), root: true, exported: true }
    }

    /// An item that keeps `name` and is a root, but is no part of the program's interface.
    pub(crate) fn rooted(name: String) -> Linkage {
        Linkage { fixed_name: Some(name), root: true, exported: false }
    }

    /// An item that must be emitted under `name` and is nothing else: not a root, not exported.
    ///
    /// The allocator shim is the case. Its name is fixed because the calls to it are compiled into
    /// `library/alloc` under exactly that symbol, and it is not a root because a program that never
    /// allocates should not carry one.
    pub(crate) fn fixed(name: String) -> Linkage {
        Linkage { fixed_name: Some(name), root: false, exported: false }
    }

    /// Turns this into the linkage of the same item seen from a dependency.
    ///
    /// The name survives and everything else goes. Keeping the name is what stops minification
    /// from renaming a dependency's `#[no_mangle]` item that external JavaScript calls; dropping
    /// the rest is what keeps `compiler_builtins`' few hundred exports from rooting all of `core`.
    pub(crate) fn demote(&mut self) {
        self.root = false;
        self.exported = false;
    }
}

/// Where a zombie came from, for grouping in reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ZombieKind {
    /// A construct the MIR walk cannot lower.
    Unsupported,
    /// A constant the backend cannot turn into a JavaScript value.
    Constant,
    /// A backend invariant that did not hold — a would-be ICE, reported instead of panicking.
    Internal,
}

impl ZombieKind {
    fn tag(self) -> u8 {
        match self {
            ZombieKind::Unsupported => 0,
            ZombieKind::Constant => 1,
            ZombieKind::Internal => 2,
        }
    }

    fn from_tag(tag: u8) -> Option<ZombieKind> {
        Some(match tag {
            0 => ZombieKind::Unsupported,
            1 => ZombieKind::Constant,
            2 => ZombieKind::Internal,
            _ => return None,
        })
    }
}

/// Something the backend could not lower, standing in the emitted JavaScript as a runtime abort.
///
/// Nothing is reported when a zombie is recorded. `link.rs` reports the ones the program can
/// actually reach, which is why `loc` is rendered eagerly: by the time a zombie from another crate
/// is read back out of an rlib there is no source map that could render its span.
#[derive(Clone, Debug)]
pub(crate) struct Zombie {
    /// `file.rs:LINE:COL`, rendered when the zombie was recorded.
    pub(crate) loc: String,
    pub(crate) message: String,
    /// The span, kept while the item stays in this compilation session. Dropped on serialization,
    /// which is why nothing reads it yet: today every zombie is reported through `loc`, and a
    /// zombie only ever reaches a report after a round trip through an object file.
    #[allow(dead_code)]
    pub(crate) span: Option<SpanData>,
    pub(crate) kind: ZombieKind,
}

/// A place to record zombies from code that has no [`crate::base::FnCx`] — constants, statics.
#[derive(Default)]
pub(crate) struct ZombieLog {
    list: std::cell::RefCell<Vec<Zombie>>,
}

impl ZombieLog {
    /// Records a zombie and returns the poison expression that stands in for the value.
    pub(crate) fn record(
        &self,
        tcx: TyCtxt<'_>,
        span: Span,
        kind: ZombieKind,
        message: String,
    ) -> Expr {
        let loc = tcx.sess.source_map().span_to_diagnostic_string(span);
        self.list.borrow_mut().push(Zombie {
            loc,
            message: message.clone(),
            span: Some(span.data()),
            kind,
        });
        jsast::rt_call("js_abort", vec![jsast::string(message)])
    }

    pub(crate) fn take(&self) -> Vec<Zombie> {
        std::mem::take(&mut self.list.borrow_mut())
    }

    /// Moves everything recorded here into another log.
    ///
    /// For lowering that builds an item out of several sub-lowerings — a `static` initializer, a
    /// vtable — where the zombies all belong to the one item that comes out.
    #[allow(dead_code)]
    pub(crate) fn drain_into(&self, other: &ZombieLog) {
        other.list.borrow_mut().extend(self.take());
    }
}

/// One named top level declaration, plus everything the link step needs to know about it.
pub(crate) struct JsItem {
    pub(crate) name: JsName,
    pub(crate) kind: ItemKind,
    pub(crate) decl: Stmt,
    /// Every identifier the declaration mentions. Over-approximate; see the module docs.
    pub(crate) refs: BTreeSet<JsName>,
    pub(crate) zombies: Vec<Zombie>,
    pub(crate) linkage: Linkage,
    /// The Rust def path, for comments and for `required by` chains.
    pub(crate) debug_path: String,
    /// Where this item has to be declared, for the kinds whose order is observable. `None` for
    /// everything else, which is ordered by dependency and then by name. See [`SourceOrder`].
    pub(crate) order: Option<SourceOrder>,
    /// Where the item was declared, which is the location its header line maps to. `None` for a
    /// synthesized item (a vtable, an allocator shim) and whenever source maps are off.
    pub(crate) loc: Option<Loc>,
    /// Emitted JavaScript name to the Rust name behind it, for the map's `names` table.
    pub(crate) names: Vec<(String, String)>,
    /// The item's TypeScript signature, for `js-dts=on`. `Some` only for a function whose Rust
    /// signature and emitted parameter list agree; see [`crate::dts::signature`].
    pub(crate) dts: Option<crate::dts::Dts>,
}

impl JsItem {
    /// Builds an item, computing `refs` from the declaration.
    pub(crate) fn new(
        name: JsName,
        kind: ItemKind,
        decl: Stmt,
        zombies: Vec<Zombie>,
        linkage: Linkage,
        debug_path: String,
    ) -> JsItem {
        let mut refs = BTreeSet::new();
        jsast::visit_idents(&decl, &mut |ident| {
            if ident != name.as_str() {
                refs.insert(JsName::new(ident.to_owned()));
            }
        });
        JsItem {
            name,
            kind,
            decl,
            refs,
            zombies,
            linkage,
            debug_path,
            order: None,
            loc: None,
            names: Vec::new(),
            dts: None,
        }
    }

    /// Attaches the TypeScript signature `js-dts=on` writes into the `.d.ts`.
    pub(crate) fn with_dts(mut self, dts: Option<crate::dts::Dts>) -> JsItem {
        self.dts = dts;
        self
    }

    /// Fixes where this item is declared relative to the others that carry one. See [`SourceOrder`].
    pub(crate) fn with_order(mut self, order: SourceOrder) -> JsItem {
        self.order = Some(order);
        self
    }

    /// Attaches what the source map needs: where the item was declared and what its names mean.
    pub(crate) fn with_source(mut self, loc: Option<Loc>, names: Vec<(String, String)>) -> JsItem {
        if crate::opts::get().source_map {
            self.loc = loc;
            self.names = names;
        }
        self
    }

    /// The item's JavaScript and the source map entries for it, relative to its own first line.
    ///
    /// The mappings are always computed the same way; with source maps off the item carries no
    /// [`Stmt::Loc`] markers and no names, so the list comes out empty and the text is exactly what
    /// [`jsast::program_to_string`] would have printed.
    ///
    /// A hoisted constant gets no header comment even with comments on: its debug path *is* its
    /// value spelled again (`caller location file.rs:19:15` above `{ file: "file.rs", line: 19,
    /// column: 15 }`), so the comment would be pure duplication of the one thing this milestone
    /// exists to stop repeating. `emit-test.sh`'s filter already handles an item with no comment.
    pub(crate) fn to_js_mapped(&self, comments: bool) -> (String, Vec<Mapping>) {
        let comments = comments && self.kind != ItemKind::Const;
        let names: std::collections::BTreeMap<String, String> = self.names.iter().cloned().collect();
        let style = match crate::opts::get().minify {
            true => jsast::Style::Compact,
            false => jsast::Style::Readable,
        };
        if comments {
            let program = [jsast::comment(self.debug_path.clone()), self.decl.clone()];
            jsast::program_to_string_mapped_styled(&program, names, self.loc.clone(), style)
        } else {
            let program = std::slice::from_ref(&self.decl);
            jsast::program_to_string_mapped_styled(program, names, self.loc.clone(), style)
        }
    }
}

/// The source map entries for one item, with generated lines relative to its own first line.
///
/// Relative, because an item's place in the finished program is not known until the link step has
/// decided what is reachable, and an item from an rlib was printed in another compilation entirely.
/// An item's JavaScript always starts at the beginning of a line, so only the line has to be
/// rebased; the columns are already right.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ItemMap {
    pub(crate) sources: Vec<String>,
    pub(crate) names: Vec<String>,
    /// `(gen_line, gen_col, source, src_line, src_col, name)`. `name` is `-1` for "no name", and
    /// `source`/`name` index the two lists above.
    pub(crate) segments: Vec<(u32, u32, u32, u32, u32, i32)>,
}

impl ItemMap {
    /// Interns the sources and names of `mappings` into the compact form the object file stores.
    fn of(mappings: Vec<Mapping>) -> ItemMap {
        let mut map = ItemMap::default();
        for mapping in mappings {
            let source = intern(&mut map.sources, &mapping.loc.file);
            let name = match &mapping.name {
                Some(name) => intern(&mut map.names, name) as i32,
                None => -1,
            };
            map.segments.push((
                mapping.gen_line,
                mapping.gen_col,
                source,
                mapping.loc.line,
                mapping.loc.col,
                name,
            ));
        }
        map
    }
}

/// The index of `value` in `list`, appending it if it is not there yet.
fn intern(list: &mut Vec<String>, value: &str) -> u32 {
    match list.iter().position(|entry| entry == value) {
        Some(index) => index as u32,
        None => {
            list.push(value.to_owned());
            (list.len() - 1) as u32
        }
    }
}

// -------------------------------------------------------------------------------------------
// Object files
// -------------------------------------------------------------------------------------------

/// An item read back out of an object file: the same shape as a [`JsItem`], with the declaration
/// as already-printed text and the zombie spans flattened to strings.
#[derive(Clone, Debug)]
pub(crate) struct LinkItem {
    pub(crate) name: JsName,
    pub(crate) kind: ItemKind,
    /// The printed declaration, spliced verbatim into the output. Never re-parsed.
    pub(crate) js: String,
    pub(crate) refs: BTreeSet<JsName>,
    pub(crate) zombies: Vec<FlatZombie>,
    pub(crate) linkage: Linkage,
    pub(crate) debug_path: String,
    /// Where this item has to be declared, for the kinds whose order is observable. See
    /// [`SourceOrder`].
    pub(crate) order: Option<SourceOrder>,
    /// The source map entries for `js`, relative to its first line. Empty unless source maps were
    /// on when the object was written.
    pub(crate) map: ItemMap,
    /// The item's TypeScript signature, or `None` for one that has none.
    pub(crate) dts: Option<crate::dts::Dts>,
}

impl LinkItem {
    /// The item's JavaScript with its header comment stripped.
    ///
    /// This is the form two definitions of the same instantiation are compared in. The comment
    /// holds a def path, and a def path is spelled *relative to the crate that printed it*: `core`
    /// codegens `<u32 as ops::bit::Shl>::shl` and a downstream crate codegens the very same body as
    /// `<u32 as core::ops::Shl>::shl`. That disagreement is about naming, not about code, and it
    /// would otherwise be reported as a miscompilation on every link against real `core`.
    pub(crate) fn code(&self) -> &str {
        let mut rest = self.js.as_str();
        loop {
            let trimmed = rest.trim_start_matches(['\n', '\r']);
            if !trimmed.starts_with("//") {
                return trimmed;
            }
            rest = match trimmed.find('\n') {
                Some(end) => &trimmed[end..],
                None => return "",
            };
        }
    }
}

/// A zombie without its span.
#[derive(Clone, Debug)]
pub(crate) struct FlatZombie {
    pub(crate) loc: String,
    pub(crate) message: String,
    /// Round trips through the object file so that a report can group by cause, which the
    /// zombie-report golden test in a later milestone needs.
    #[allow(dead_code)]
    pub(crate) kind: ZombieKind,
}

#[derive(Serialize, Deserialize)]
struct SerItem {
    name: String,
    kind: u8,
    /// Byte offset of the item's JavaScript within the object file's body.
    start: u32,
    len: u32,
    refs: Vec<String>,
    zombies: Vec<SerZombie>,
    fixed_name: Option<String>,
    root: bool,
    exported: bool,
    debug_path: String,
    order: Option<SourceOrder>,
    map: ItemMap,
    dts: Option<crate::dts::Dts>,
}

#[derive(Serialize, Deserialize)]
struct SerZombie {
    loc: String,
    message: String,
    kind: u8,
}

/// Renders a codegen unit's items as an object file: runnable JavaScript plus the item table.
pub(crate) fn write_object(prologue: &str, items: &[JsItem], comments: bool) -> String {
    let mut body = String::new();
    body.push_str(prologue);

    let mut table = Vec::with_capacity(items.len());
    for item in items {
        let (js, mappings) = item.to_js_mapped(comments);
        let start = body.len();
        body.push_str(&js);
        table.push(SerItem {
            name: item.name.as_str().to_owned(),
            kind: item.kind.tag(),
            start: start as u32,
            len: js.len() as u32,
            refs: item.refs.iter().map(|r| r.as_str().to_owned()).collect(),
            zombies: item
                .zombies
                .iter()
                .map(|z| SerZombie {
                    loc: z.loc.clone(),
                    message: z.message.clone(),
                    kind: z.kind.tag(),
                })
                .collect(),
            fixed_name: item.linkage.fixed_name.clone(),
            root: item.linkage.root,
            exported: item.linkage.exported,
            debug_path: item.debug_path.clone(),
            order: item.order.clone(),
            map: ItemMap::of(mappings),
            dts: item.dts.clone(),
        });
    }

    let mut encoded =
        postcard::to_allocvec(&FORMAT_VERSION).expect("a `u16` is always serializable");
    encoded.extend(postcard::to_allocvec(&table).expect("the item table is always serializable"));
    let payload = base64::engine::general_purpose::STANDARD.encode(encoded);
    format!("{body}{FOOTER_PREFIX}{payload}\n")
}

/// Renders items as a finished program: runnable JavaScript, no item table.
///
/// Returns the text and the line each item starts on, which is what rebases the item-relative
/// source map entries onto the program that came out.
pub(crate) fn write_program(prologue: &str, items: &[&LinkItem]) -> (String, Vec<u32>) {
    let mut out = String::from(prologue);
    let mut line = prologue.matches('\n').count() as u32;
    let mut starts = Vec::with_capacity(items.len());
    for item in items {
        starts.push(line);
        out.push_str(&item.js);
        line += item.js.matches('\n').count() as u32;
    }
    (out, starts)
}

/// Reads an object file's item table back, slicing each declaration out of the body.
///
/// Returns `Ok(None)` for text with no footer — an object some other backend wrote, or one of
/// ours that was already pruned down to a finished program. A footer this backend cannot read is
/// an error rather than a `None`: dropping every item of a stale rlib on the floor would report
/// itself as a program missing half of `core`.
pub(crate) fn read_object(text: &str) -> Result<Option<Vec<LinkItem>>, String> {
    let Some(marker) = text.rfind(FOOTER_PREFIX) else {
        return Ok(None);
    };
    // The marker has to start a line, or it is JavaScript that merely looks like a footer.
    if marker != 0 && !text[..marker].ends_with('\n') {
        return Ok(None);
    }
    let body = &text[..marker];
    let payload = text[marker + FOOTER_PREFIX.len()..].trim_end();

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|err| format!("corrupt rcgjs item table (base64: {err})"))?;
    let (version, rest) = postcard::take_from_bytes::<u16>(&bytes)
        .map_err(|err| format!("corrupt rcgjs item table (postcard: {err})"))?;
    if version != FORMAT_VERSION {
        return Err(format!(
            "this object holds an rcgjs item table in format {version}, and this backend reads \
             format {FORMAT_VERSION}: it was written by another build of the backend. Rebuild \
             everything this program links against, the sysroot included \
             (`scripts/build_sysroot.sh --clean`)"
        ));
    }
    let table: Vec<SerItem> = postcard::from_bytes(rest)
        .map_err(|err| format!("corrupt rcgjs item table (postcard: {err})"))?;

    let mut items = Vec::with_capacity(table.len());
    for entry in table {
        let start = entry.start as usize;
        let end = start + entry.len as usize;
        let js = body
            .get(start..end)
            .ok_or_else(|| format!("rcgjs item `{}` points outside the object body", entry.name))?;
        let kind = ItemKind::from_tag(entry.kind)
            .ok_or_else(|| format!("rcgjs item `{}` has an unknown kind", entry.name))?;
        let mut zombies = Vec::with_capacity(entry.zombies.len());
        for z in entry.zombies {
            let kind = ZombieKind::from_tag(z.kind)
                .ok_or_else(|| format!("rcgjs item `{}` has an unknown zombie kind", entry.name))?;
            zombies.push(FlatZombie { loc: z.loc, message: z.message, kind });
        }
        items.push(LinkItem {
            name: JsName::new(entry.name),
            kind,
            js: js.to_owned(),
            refs: entry.refs.into_iter().map(JsName::new).collect(),
            zombies,
            linkage: Linkage {
                fixed_name: entry.fixed_name,
                root: entry.root,
                exported: entry.exported,
            },
            debug_path: entry.debug_path,
            order: entry.order,
            map: entry.map,
            dts: entry.dts,
        });
    }
    Ok(Some(items))
}
