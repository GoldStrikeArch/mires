//! The initialization order of module level bindings, cycles among `static`s included.
//!
//! # What a binding's initializer reads
//!
//! A `static` is printed as one declaration, `let NAME = <value>;`, by both producers
//! (`backend/src/base.rs::codegen_static` and the JIR lane's `lower_static`). Its value was
//! const-evaluated by rustc, so the initializer is a literal: an object or array literal built out
//! of numbers, strings and the names of other items. A name in it is always a *reference*, never a
//! read of what the named binding holds: a reference to a `static` of an indirect type is the
//! binding itself, a reference to a boxed one is `{ buf: NAME, off: 0 }`, and a raw pointer to one
//! is `__rt.box(NAME)`, which keys on the object's identity. So an initializer needs every binding
//! it names to *exist*, and nothing more.
//!
//! [`crate::link::order`] therefore declares a binding after the bindings it names, which is a
//! depth-first post-order over `refs`. That is enough unless the bindings name each other.
//!
//! # A cycle
//!
//! Rust allows it: `static A: Node = Node { next: &B }; static B: Node = Node { next: &A };`, a
//! `static` that names itself, a ring of any length. No order of plain `let`s initializes one: the
//! first declaration names a binding that is still in its temporal dead zone, and the module throws
//! a `ReferenceError` before any Rust code runs. A native program has no such moment, because its
//! statics are bytes in the image before `main`.
//!
//! The cycle is printed in two steps instead ([`initialize`]):
//!
//! ```js
//! let A = {};
//! let B = {};
//! Object.assign(A, { id: 1, next: B });
//! Object.assign(B, { id: 2, next: A });
//! ```
//!
//! Every member is declared first as an empty object (or an empty array, for an initializer that
//! is an array literal), so that every name in the cycle exists; then each member's own literal is
//! evaluated, now that the names it holds are bound, and copied onto it. The binding keeps the
//! identity it was declared with, which is the identity every other initializer and every function
//! captured, so `ptr::eq` of the two ways to reach a member is still true. `Object.assign` copies
//! the literal's own enumerable properties, in their own order, onto an empty object or array
//! with the literal's prototype, and for every literal this printer emits that gives the same
//! object, with one exception: a `__proto__` key, which an object literal takes as the object's
//! prototype instead of a property, so there is no own property to copy. A Rust field named
//! `__proto__` prints as that key, and a cycle member holding one is refused (below). Where in
//! the literal a reference
//! sits does not matter — a field, an enum payload (`{ TAG: "Some", _0: B }`, a decoded
//! `Option<&'static T>`), an array element, a slice's buffer — because the whole literal is
//! evaluated only once every member exists.
//!
//! The members are kept together where the post-order would have put the cycle as a whole: after
//! every binding any member names outside the cycle, and before every binding that names a member.
//! No Rust code runs while a cycle is half filled, because module evaluation runs initializers
//! only, and a binding outside the cycle that names a member is declared after the whole cycle.
//!
//! A binding with no cycle is printed exactly as before, byte for byte: the walk that finds the
//! cycles ([`layout`]) visits in the order the plain post-order did, and only a cycle is rewritten.
//!
//! # What is refused
//!
//! The two-step form needs the shape this module documents above: a `static` item whose text is
//! one `let NAME = <literal>;` (after its `//` header comments), where the literal is one object or
//! array literal with no `__proto__` key. A cycle with any other member, or through a binding of
//! another kind, is a link error naming the statics, never a program that throws at load or
//! silently holds `undefined` or a wrong prototype. Apart from a field named `__proto__`, no
//! producer emits such a cycle today; the refusal is what keeps a future one from being printed
//! in an order that cannot work.
//!
//! # Chunks
//!
//! A cycle cannot span two chunks: whichever chunk reaches one member reaches them all, so they
//! share one owner (see `write_chunks`). The one way to split one, a `static` named as a chunk's
//! entry point, is refused by [`split_cycles`].
//!
//! Across files the same hazard has a second shape. Inside a cycle of chunk imports, which file
//! evaluates first depends on which one the host loads first, so a module level initializer that
//! reads a binding imported from a file that imports it back reads it uninitialized under some
//! load order. A function is safe to import around such a cycle: function declarations exist
//! before any module evaluates. [`evaluation_cycles`] refuses every other import read by an
//! initializer around a cycle. Only a `static` named as an entry point creates one, because a
//! chunk another file imports from owns nothing but its entry: everything that entry reaches is
//! reached by two chunks, so it is shared.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::diag::Sink;
use crate::item::{ItemKind, ItemMap, JsName, LinkItem};

/// One step of the print order: a binding or item on its own, or a cycle of bindings that has to
/// be initialized as a whole.
pub(crate) enum Placed<'a> {
    Item(&'a LinkItem),
    /// Two or more bindings that name each other, or one that names itself, in the order the walk
    /// discovered them (the first is the one the walk entered the cycle through).
    Cycle(Vec<&'a LinkItem>),
}

/// The bindings among `names` in initialization order: a depth-first post-order over the
/// references between bindings that `members` holds, with each strongly connected component kept
/// together (Tarjan's algorithm).
///
/// On a graph without cycles every component is one binding and is emitted when its walk
/// finishes, which is exactly the post-order the plain walk produced; that is what keeps every
/// program without a cycle byte-identical.
pub(crate) fn layout<'a>(
    table: &'a BTreeMap<JsName, LinkItem>,
    members: &HashSet<&'a JsName>,
    names: &[&'a JsName],
) -> Vec<Placed<'a>> {
    let mut walk = Walk {
        table,
        members,
        next: 0,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: Vec::new(),
        on_stack: HashSet::new(),
        out: Vec::new(),
    };
    for name in names {
        let Some((key, item)) = table.get_key_value(*name) else {
            continue;
        };
        if item.kind.is_binding() && members.contains(key) && !walk.index.contains_key(key) {
            walk.connect(key);
        }
    }
    walk.out
}

/// The state of [`layout`]'s walk.
struct Walk<'a, 'm> {
    table: &'a BTreeMap<JsName, LinkItem>,
    members: &'m HashSet<&'a JsName>,
    next: usize,
    /// The order each binding was entered in.
    index: HashMap<&'a JsName, usize>,
    /// The smallest entry order reachable from each binding through the bindings still on the
    /// stack.
    low: HashMap<&'a JsName, usize>,
    stack: Vec<&'a JsName>,
    on_stack: HashSet<&'a JsName>,
    out: Vec<Placed<'a>>,
}

impl<'a> Walk<'a, '_> {
    fn connect(&mut self, name: &'a JsName) {
        let entered = self.next;
        self.next += 1;
        self.index.insert(name, entered);
        self.low.insert(name, entered);
        self.stack.push(name);
        self.on_stack.insert(name);

        let item = &self.table[name];
        let mut names_itself = names_itself(item);
        for referenced in &item.refs {
            let Some((key, target)) = self.table.get_key_value(referenced) else {
                continue;
            };
            if !target.kind.is_binding() || !self.members.contains(key) {
                continue;
            }
            if key == name {
                names_itself = true;
                continue;
            }
            match self.index.get(key) {
                None => {
                    self.connect(key);
                    let reached = self.low[key];
                    self.lower(name, reached);
                }
                Some(&seen) if self.on_stack.contains(key) => self.lower(name, seen),
                Some(_) => {}
            }
        }

        if self.low[name] != entered {
            return;
        }
        let mut component = Vec::new();
        while let Some(member) = self.stack.pop() {
            self.on_stack.remove(member);
            component.push(&self.table[member]);
            if member == name {
                break;
            }
        }
        if component.len() == 1 && !names_itself {
            self.out.push(Placed::Item(component[0]));
        } else {
            // Popped last-entered first; the cycle is listed from the binding the walk entered it
            // through.
            component.reverse();
            self.out.push(Placed::Cycle(component));
        }
    }

    fn lower(&mut self, name: &'a JsName, to: usize) {
        let low = self
            .low
            .get_mut(name)
            .expect("an entered binding has a low link");
        *low = (*low).min(to);
    }
}

/// Whether a `static`'s initializer names the static itself: `static S: Node = Node { me: &S }`.
///
/// `refs` cannot say: an item's own name is left out of its references (`JsItem::new`), because a
/// function that calls itself is no edge of the call graph. For a binding it is the one name that
/// matters, so it is looked for in the text, by the scanner the item rename trusts
/// ([`crate::minify::for_each_ident`]): strings, comments and property names are skipped, and the
/// declaration `let NAME` is the one occurrence every static has.
fn names_itself(item: &LinkItem) -> bool {
    if item.kind != ItemKind::Static {
        return false;
    }
    let name = item.name.as_str();
    // Nearly every static mentions its name once; the scan is only needed past a second mention.
    if item.js.matches(name).nth(1).is_none() {
        return false;
    }
    let mut mentions = 0;
    crate::minify::for_each_ident(&item.js, |_, ident| {
        if ident == name {
            mentions += 1;
        }
    });
    mentions > 1
}

/// The two-step initialization of every cycle in `placed`, one list of items per cycle in the
/// order the cycles appear. See [`initialize`].
pub(crate) fn initialize_cycles(dcx: &Sink, placed: &[Placed<'_>]) -> Vec<Vec<LinkItem>> {
    placed
        .iter()
        .filter_map(|step| match step {
            Placed::Item(_) => None,
            Placed::Cycle(members) => Some(initialize(dcx, members)),
        })
        .collect()
}

/// `placed` as the items to print, each cycle replaced by its two-step form in `cycles` (from
/// [`initialize_cycles`] over the same `placed`).
pub(crate) fn print_order<'a>(
    placed: &[Placed<'a>],
    cycles: &'a [Vec<LinkItem>],
) -> Vec<&'a LinkItem> {
    let mut cycles = cycles.iter();
    let mut out = Vec::with_capacity(placed.len());
    for step in placed {
        match step {
            Placed::Item(item) => out.push(*item),
            Placed::Cycle(_) => out.extend(
                cycles
                    .next()
                    .expect("one initialized cycle per cycle placed")
                    .iter(),
            ),
        }
    }
    out
}

/// One cycle of bindings, printed so that it initializes: every member declared as an empty
/// object or array, then every member's literal copied onto it.
///
/// The result has one item per member, in the members' order, each keeping its member's name,
/// linkage, references and type signature, so the export clause, the `.d.ts` and the rename see
/// the members they always saw. The declarations are the members' own texts with the literal
/// emptied; the copies follow the last declaration, inside the last member's item, because none of
/// them may run before every member exists.
///
/// A member this module cannot split (see the module docs) is reported, and the cycle is returned
/// as it was, which the reported error keeps from ever being written.
fn initialize(dcx: &Sink, members: &[&LinkItem]) -> Vec<LinkItem> {
    let cycle_names: HashSet<&str> = members.iter().map(|member| member.name.as_str()).collect();
    let mut split = Vec::with_capacity(members.len());
    for member in members {
        match Declaration::of(member) {
            Ok(declaration) => {
                if reads_cycle_member_property(declaration.literal, &cycle_names) {
                    let names: Vec<&str> = members
                        .iter()
                        .map(|member| member.debug_path.as_str())
                        .collect();
                    dcx.err(format!(
                        "rustc_codegen_js: the statics `{}` refer to each other, and `{}` reads a \
                         sub-field or element of a cycle member during initialization, so the \
                         cycle cannot be initialized in two steps",
                        names.join("`, `"),
                        member.debug_path,
                    ));
                    return members.iter().map(|member| (*member).clone()).collect();
                }
                split.push(declaration);
            }
            Err(reason) => {
                let names: Vec<&str> = members
                    .iter()
                    .map(|member| member.debug_path.as_str())
                    .collect();
                dcx.err(format!(
                    "rustc_codegen_js: the statics `{}` refer to each other, and `{}` {reason}, so \
                     the cycle cannot be initialized in two steps; this is a compiler bug, please \
                     report it",
                    names.join("`, `"),
                    member.debug_path,
                ));
                return members.iter().map(|member| (*member).clone()).collect();
            }
        }
    }

    let mut out: Vec<LinkItem> = Vec::with_capacity(members.len());
    for (member, declaration) in members.iter().zip(&split) {
        let mut item = (*member).clone();
        item.js = declaration.declare_empty();
        item.map = declaration.declaration_map(&member.map);
        out.push(item);
    }
    let last = out.last_mut().expect("a cycle has a member");
    for (member, declaration) in members.iter().zip(&split) {
        let fill = declaration.fill();
        // Each line of a member's fill maps where its declaration did, so a source-mapped program
        // still maps every line it prints, and a debugger stepping through module initialization
        // lands on the `static` being filled.
        if let Some(origin) = declaration.origin(&member.map) {
            let first = last.js.matches('\n').count() as u32;
            let lines = fill.matches('\n').count() as u32;
            let source = intern(&mut last.map.sources, origin.source);
            let name = origin
                .name
                .map_or(-1, |name| intern(&mut last.map.names, name) as i32);
            for line in first..first + lines {
                last.map
                    .segments
                    .push((line, 0, source, origin.line, origin.column, name));
            }
        }
        last.js.push_str(&fill);
    }
    out
}

/// The index of `value` in `list`, appending it if it is not there yet (an [`ItemMap`]'s sources
/// and names are interned per item).
fn intern(list: &mut Vec<String>, value: &str) -> u32 {
    let index = match list.iter().position(|entry| entry == value) {
        Some(index) => index,
        None => {
            list.push(value.to_owned());
            list.len() - 1
        }
    };
    index as u32
}

/// Where in the Rust source a member's declaration starts.
struct Origin<'m> {
    source: &'m str,
    line: u32,
    column: u32,
    name: Option<&'m str>,
}

/// A `static`'s text taken apart: `<lead>let NAME = <literal>;\n`.
struct Declaration<'a> {
    /// The `//` header lines before the declaration, newlines included.
    lead: &'a str,
    /// `let NAME = ` or, compact, `let NAME=`.
    head: &'a str,
    name: &'a str,
    literal: &'a str,
    compact: bool,
}

impl<'a> Declaration<'a> {
    fn of(item: &'a LinkItem) -> Result<Self, &'static str> {
        if item.kind != ItemKind::Static {
            return Err("is not a `static` binding");
        }
        let text = item.js.as_str();
        let mut rest = text;
        while rest.starts_with("//") {
            let Some(end) = rest.find('\n') else {
                return Err("has no declaration");
            };
            rest = &rest[end + 1..];
        }
        let lead = &text[..text.len() - rest.len()];
        let name = item.name.as_str();
        let readable = format!("let {name} = ");
        let compact_head = format!("let {name}=");
        let (head_len, compact) = if rest.starts_with(&readable) {
            (readable.len(), false)
        } else if rest.starts_with(&compact_head) {
            (compact_head.len(), true)
        } else {
            return Err("is not printed as one `let` of its own name");
        };
        let (head, after) = rest.split_at(head_len);
        let Some(literal) = after.strip_suffix(";\n") else {
            return Err("is not printed as one `let` statement");
        };
        match literal.as_bytes().first() {
            Some(b'{') | Some(b'[') => {}
            _ => return Err("is not initialized with an object or array literal"),
        }
        if literal_len(literal) != Some(literal.len()) {
            return Err("is not initialized with exactly one object or array literal");
        }
        if has_proto_key(literal) {
            return Err(
                "has a field named `__proto__`, which an object literal takes as its \
                        prototype rather than as a property and `Object.assign` would not copy",
            );
        }
        Ok(Declaration {
            lead,
            head,
            name,
            literal,
            compact,
        })
    }

    /// The declaration with the literal replaced by an empty one of the same kind, so the
    /// binding exists, with the literal's prototype, before any member's literal is evaluated.
    fn declare_empty(&self) -> String {
        let empty = match self.literal.as_bytes()[0] {
            b'[' => "[]",
            _ => "{}",
        };
        format!("{}{}{empty};\n", self.lead, self.head)
    }

    /// The statement that gives the binding its value, once every member exists.
    fn fill(&self) -> String {
        match self.compact {
            true => format!("Object.assign({},{});\n", self.name, self.literal),
            false => format!("Object.assign({}, {});\n", self.name, self.literal),
        }
    }

    /// The member's source map entries that still point at text: those in its header lines and
    /// at the start of its declaration. The literal moved into a `fill`, which [`initialize`] maps
    /// line by line to the declaration's own [`origin`](Self::origin).
    fn declaration_map(&self, map: &ItemMap) -> ItemMap {
        let head = self.head.len() as u32;
        let lead_lines = self.lead_lines();
        let mut kept = map.clone();
        kept.segments.retain(|&(line, column, ..)| {
            line < lead_lines || (line == lead_lines && column < head)
        });
        kept
    }

    /// The original position of the first map entry on the declaration's line, if there is one.
    fn origin<'m>(&self, map: &'m ItemMap) -> Option<Origin<'m>> {
        let lead_lines = self.lead_lines();
        let &(_, _, source, line, column, name) = map
            .segments
            .iter()
            .filter(|segment| segment.0 == lead_lines)
            .min_by_key(|segment| segment.1)?;
        Some(Origin {
            source: map.sources.get(source as usize)?,
            line,
            column,
            name: usize::try_from(name)
                .ok()
                .and_then(|name| map.names.get(name))
                .map(String::as_str),
        })
    }

    fn lead_lines(&self) -> u32 {
        self.lead.matches('\n').count() as u32
    }
}

/// Whether `text`, a literal [`literal_len`] accepted, has a `__proto__` key anywhere: the bare
/// name or the string `"__proto__"` directly after `{` or `,` and directly before `:`.
///
/// In an object literal that one key sets the object's prototype instead of creating a property,
/// so `Object.assign` (which copies own properties) would drop it. A Rust field named `__proto__`
/// prints as exactly that key. The scan is the printer's token shape: double quoted strings are
/// skipped as tokens (the only quotes [`literal_len`] lets through), and a name after `.` is a
/// property read, not a key.
fn has_proto_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    let next_significant = |mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        bytes.get(at).copied()
    };
    let is_key = |previous: u8, after: usize| {
        matches!(previous, b'{' | b',') && next_significant(after) == Some(b':')
    };
    let mut previous = 0u8;
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b'"' => {
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                let at = at.min(bytes.len());
                if &text[start..at] == "\"__proto__\"" && is_key(previous, at) {
                    return true;
                }
                previous = b'"';
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' => {
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'_' | b'$'))
                {
                    at += 1;
                }
                let read = start > 0 && bytes[start - 1] == b'.';
                if &text[start..at] == "__proto__" && !read && is_key(previous, at) {
                    return true;
                }
                previous = bytes[at - 1];
            }
            byte if byte.is_ascii_whitespace() => at += 1,
            byte => {
                previous = byte;
                at += 1;
            }
        }
    }
    false
}

/// Whether `text`, a literal [`literal_len`] accepted, reads a property or element (`.` or `[`)
/// off any identifier in `cycle_names`.
///
/// During two-step cycle initialization (`let A = {}; let B = {}; Object.assign(A, <litA>);`),
/// every cycle member starts as an empty object or array. Capturing a cycle member by reference
/// (`next: B`, `{ buf: B, off: "x" }`, `{ buf: B, off: 0, len: 3 }`) is sound because no property
/// of `B` is read yet; reading a nested property or element (`B.inner`, `S.inner`, `S.arr`,
/// `B[0]`) would evaluate to `undefined` before `B`/`S` is populated.
fn reads_cycle_member_property(text: &str, cycle_names: &HashSet<&str>) -> bool {
    let bytes = text.as_bytes();
    let next_significant = |mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        bytes.get(at).copied()
    };
    let mut previous = 0u8;
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b'"' => {
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                at = at.min(bytes.len());
                previous = b'"';
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' => {
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'_' | b'$'))
                {
                    at += 1;
                }
                let ident = &text[start..at];
                let after_dot = previous == b'.';
                let next = next_significant(at);
                let is_key = matches!(previous, b'{' | b',') && next == Some(b':');
                if !after_dot
                    && !is_key
                    && cycle_names.contains(ident)
                    && matches!(next, Some(b'.' | b'['))
                {
                    return true;
                }
                previous = bytes[at - 1];
            }
            byte if byte.is_ascii_whitespace() => at += 1,
            byte => {
                previous = byte;
                at += 1;
            }
        }
    }
    false
}

/// The byte length of the object or array literal `text` starts with, or `None` if it is not
/// closed or holds a token this printer never puts in a `static` (a single quoted string or a
/// template). Strings are skipped, so a bracket inside one is not counted.
fn literal_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (at, byte) in text.bytes().enumerate() {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'\'' | b'`' => return None,
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Reports every cycle among the bindings `owner` places that would be split across two chunks.
///
/// Whichever chunk reaches one member of a cycle reaches all of them, so the ownership rules put a
/// cycle in one chunk, except when a member is itself a chunk's entry point. That chunk would then
/// import the rest from the shared chunk while the shared chunk imports the entry back, and the
/// first of the two modules to run would read the other's bindings before they exist.
pub(crate) fn split_cycles(
    dcx: &Sink,
    table: &BTreeMap<JsName, LinkItem>,
    owner: &BTreeMap<&JsName, usize>,
    label: impl Fn(usize) -> String,
) {
    let members: HashSet<&JsName> = owner.keys().copied().collect();
    let names: Vec<&JsName> = owner.keys().copied().collect();
    for step in layout(table, &members, &names) {
        let Placed::Cycle(cycle) = step else {
            continue;
        };
        let owners: BTreeSet<usize> = cycle
            .iter()
            .filter_map(|member| owner.get(&member.name).copied())
            .collect();
        if owners.len() > 1 {
            let names: Vec<&str> = cycle
                .iter()
                .map(|member| member.debug_path.as_str())
                .collect();
            let chunks: Vec<String> = owners.into_iter().map(&label).collect();
            dcx.err(format!(
                "the statics `{}` refer to each other but would be split across the chunks `{}`; \
                 a cycle of statics has to be in one chunk, so none of them can be a chunk's \
                 entry point",
                names.join("`, `"),
                chunks.join("`, `"),
            ));
        }
    }
}

/// Reports every module level initializer that reads, around a cycle of chunk imports, a binding
/// another file declares.
///
/// `holds[at]` is what unit `at` prints (the names it owns and the imports it replicates),
/// `owner` who owns each name, and `imported[at]` the names unit `at` imports, by the unit that
/// exports them. The rule: if unit `at` imports a binding from unit `from` and `from` imports
/// (directly or through other units) from `at`, no binding of `at` may name that binding. When a
/// host's import path enters that cycle at `from`, `from` first evaluates what it imports, `at`
/// among them; `at` finds `from` already in progress and evaluates first, so the read finds the
/// binding in its temporal dead zone. Reads from function bodies do not count: they run after
/// every file has been evaluated, and an imported function exists before any file evaluates.
///
/// Reported once per cycle of files, naming each read. Runs after [`split_cycles`], which
/// reports the narrower case of a cycle of statics split by an entry point in its own words.
pub(crate) fn evaluation_cycles(
    dcx: &Sink,
    table: &BTreeMap<JsName, LinkItem>,
    holds: &[Vec<JsName>],
    owner: &BTreeMap<&JsName, usize>,
    imported: &[BTreeMap<usize, BTreeSet<&str>>],
    label: impl Fn(usize) -> String,
) {
    // Which units each unit reaches through its imports (the graph has a handful of nodes).
    let reaches: Vec<BTreeSet<usize>> = (0..imported.len())
        .map(|start| {
            let mut seen = BTreeSet::new();
            let mut work = vec![start];
            while let Some(at) = work.pop() {
                for &from in imported[at].keys() {
                    if seen.insert(from) {
                        work.push(from);
                    }
                }
            }
            seen
        })
        .collect();

    // The reads, grouped by the cycle of files they run around (its members, sorted).
    let mut cycles: BTreeMap<BTreeSet<usize>, Vec<String>> = BTreeMap::new();
    for (at, held) in holds.iter().enumerate() {
        for name in held {
            let Some(item) = table.get(name) else {
                continue;
            };
            if owner.get(name) != Some(&at) {
                continue;
            }
            // Only a module level binding's initializer runs while its file is evaluated.
            let read_at_load = item.kind.is_binding();
            if !read_at_load {
                continue;
            }
            for referenced in &item.refs {
                let Some(&from) = owner.get(referenced) else {
                    continue;
                };
                if from == at || !reaches[from].contains(&at) {
                    continue;
                }
                let Some(target) = table
                    .get(referenced)
                    .filter(|target| target.kind.is_binding())
                else {
                    continue;
                };
                let members: BTreeSet<usize> = (0..imported.len())
                    .filter(|&unit| {
                        unit == at || (reaches[at].contains(&unit) && reaches[unit].contains(&at))
                    })
                    .collect();
                cycles.entry(members).or_default().push(format!(
                    "`{}` in `{}` reads `{}` from `{}`",
                    item.debug_path,
                    label(at),
                    target.debug_path,
                    label(from),
                ));
            }
        }
    }

    for (members, reads) in cycles {
        let files: Vec<String> = members.into_iter().map(&label).collect();
        dcx.err(format!(
            "the chunks `{}` import each other, and module level initializers read bindings \
             around that cycle ({}); depending on which file a host loads first, one of these \
             reads would find its binding uninitialized. Only functions may be imported around \
             a cycle of chunks, and a `static` named as a chunk's entry point is what makes one: \
             name a function as the entry point instead",
            files.join("`, `"),
            reads.join("; "),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Linkage;

    fn binding(name: &str, js: &str, refs: &[&str]) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Static,
            js: js.to_owned(),
            refs: refs.iter().map(|r| JsName::new((*r).to_owned())).collect(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        }
    }

    fn table(items: Vec<LinkItem>) -> BTreeMap<JsName, LinkItem> {
        items
            .into_iter()
            .map(|item| (item.name.clone(), item))
            .collect()
    }

    /// The order and the cycles of every binding in `table`.
    fn placed(table: &BTreeMap<JsName, LinkItem>) -> Vec<Vec<&str>> {
        let members: HashSet<&JsName> = table.keys().collect();
        let names: Vec<&JsName> = table.keys().collect();
        layout(table, &members, &names)
            .iter()
            .map(|step| match step {
                Placed::Item(item) => vec![item.name.as_str()],
                Placed::Cycle(cycle) => cycle.iter().map(|item| item.name.as_str()).collect(),
            })
            .collect()
    }

    fn printed(table: &BTreeMap<JsName, LinkItem>) -> (String, usize) {
        let members: HashSet<&JsName> = table.keys().collect();
        let names: Vec<&JsName> = table.keys().collect();
        let steps = layout(table, &members, &names);
        let dcx = Sink::new();
        let cycles = initialize_cycles(&dcx, &steps);
        let text = print_order(&steps, &cycles)
            .iter()
            .map(|item| item.js.as_str())
            .collect();
        (text, dcx.errors() as usize)
    }

    #[test]
    fn a_graph_without_cycles_keeps_the_post_order() {
        let table = table(vec![
            binding("a", "let a = [b, c];\n", &["b", "c"]),
            binding("b", "let b = [d];\n", &["d"]),
            binding("c", "let c = [d];\n", &["d"]),
            binding("d", "let d = [];\n", &[]),
            binding("e", "let e = [a];\n", &["a"]),
        ]);
        assert_eq!(
            placed(&table),
            [vec!["d"], vec!["b"], vec!["c"], vec!["a"], vec!["e"]]
        );
        let (text, errors) = printed(&table);
        assert_eq!(errors, 0);
        assert_eq!(
            text,
            "let d = [];\nlet b = [d];\nlet c = [d];\nlet a = [b, c];\nlet e = [a];\n"
        );
    }

    /// A two-member cycle with a dependency and a dependent outside it: the dependency first, the
    /// cycle declared and then filled, the dependent after the whole cycle.
    #[test]
    fn a_cycle_is_declared_then_filled_between_its_dependencies_and_its_dependents() {
        let table = table(vec![
            binding(
                "a",
                "// a\nlet a = { id: 1, next: b, leaf: l };\n",
                &["b", "l"],
            ),
            binding("b", "// b\nlet b = { id: 2, next: a };\n", &["a"]),
            binding("l", "// l\nlet l = { id: 3 };\n", &[]),
            binding("z", "// z\nlet z = [a];\n", &["a"]),
        ]);
        assert_eq!(placed(&table), [vec!["l"], vec!["a", "b"], vec!["z"]]);
        let (text, errors) = printed(&table);
        assert_eq!(errors, 0);
        assert_eq!(
            text,
            "// l\nlet l = { id: 3 };\n\
             // a\nlet a = {};\n\
             // b\nlet b = {};\n\
             Object.assign(a, { id: 1, next: b, leaf: l });\n\
             Object.assign(b, { id: 2, next: a });\n\
             // z\nlet z = [a];\n"
        );
    }

    /// An item's own name is never among its `refs` (as the producers build them), so a static
    /// naming itself is found in its text; a string, a property or a key spelling the name is not
    /// a reference.
    #[test]
    fn a_static_naming_itself_is_a_cycle_of_one() {
        let table = table(vec![
            binding("s$h1", "let s$h1={me:s$h1};\n", &[]),
            binding(
                "r$h2",
                "// r$h2\nlet r$h2 = [{ buf: r$h2, off: 0 }];\n",
                &[],
            ),
            binding("t$h3", "let t$h3={t$h3:1,u:\"t$h3\",v:x.t$h3};\n", &["x"]),
            binding("x", "let x={};\n", &[]),
        ]);
        assert_eq!(
            placed(&table),
            [vec!["r$h2"], vec!["s$h1"], vec!["x"], vec!["t$h3"]]
        );
        let (text, errors) = printed(&table);
        assert_eq!(errors, 0);
        assert_eq!(
            text,
            "// r$h2\nlet r$h2 = [];\nObject.assign(r$h2, [{ buf: r$h2, off: 0 }]);\n\
             let s$h1={};\nObject.assign(s$h1,{me:s$h1});\n\
             let x={};\n\
             let t$h3={t$h3:1,u:\"t$h3\",v:x.t$h3};\n"
        );
    }

    /// Where a reference sits inside the literal does not matter: in an enum payload (the shape a
    /// niche-decoded `Option<&'static T>` takes, `{ TAG: "Some", _0: B }`), in a nested struct or
    /// in an array, the whole literal is evaluated only once every member exists.
    #[test]
    fn a_reference_inside_an_enum_payload_is_filled_like_any_other() {
        let table =
            table(vec![
            binding(
                "a$h1",
                "let a$h1 = { id: 1, next: { TAG: \"Some\", _0: b$h2 } };\n",
                &["b$h2"],
            ),
            binding(
                "b$h2",
                "let b$h2 = { id: 2, next: { TAG: \"Some\", _0: a$h1 }, all: [{ x: a$h1 }] };\n",
                &["a$h1"],
            ),
            binding("s$h3", "let s$h3={id:7,next:{TAG:\"Some\",_0:s$h3}};\n", &[]),
        ]);
        assert_eq!(placed(&table), [vec!["a$h1", "b$h2"], vec!["s$h3"]]);
        let (text, errors) = printed(&table);
        assert_eq!(errors, 0);
        assert_eq!(
            text,
            "let a$h1 = {};\nlet b$h2 = {};\n\
             Object.assign(a$h1, { id: 1, next: { TAG: \"Some\", _0: b$h2 } });\n\
             Object.assign(b$h2, { id: 2, next: { TAG: \"Some\", _0: a$h1 }, all: [{ x: a$h1 }] });\n\
             let s$h3={};\nObject.assign(s$h3,{id:7,next:{TAG:\"Some\",_0:s$h3}});\n"
        );
    }

    /// Two cycles joined by one edge are two components, the named one first.
    #[test]
    fn a_ring_and_a_cycle_it_names_are_two_components() {
        let table = table(vec![
            binding("x", "let x = { n: y, o: p };\n", &["y", "p"]),
            binding("y", "let y = { n: z };\n", &["z"]),
            binding("z", "let z = { n: x };\n", &["x"]),
            binding("p", "let p = { n: q };\n", &["q"]),
            binding("q", "let q = { n: p, s: \"}]{\" };\n", &["p"]),
        ]);
        assert_eq!(placed(&table), [vec!["p", "q"], vec!["x", "y", "z"]]);
        let (text, errors) = printed(&table);
        assert_eq!(errors, 0);
        assert!(text.starts_with("let p = {};\nlet q = {};\nObject.assign(p, { n: q });\n"));
        assert!(text.contains("Object.assign(q, { n: p, s: \"}]{\" });\n"));
        assert!(text.ends_with("Object.assign(z, { n: x });\n"));
    }

    /// A cycle the two-step form cannot express is a link error, never a reordering that throws at
    /// load.
    #[test]
    fn a_cycle_through_anything_but_a_literal_is_refused() {
        for (js, kind) in [
            ("let a = __rt.box(b);\n", ItemKind::Static),
            ("let a = { n: b }.n;\n", ItemKind::Static),
            ("let a = [b].concat([]);\n", ItemKind::Static),
            ("var a = { n: b };\n", ItemKind::Static),
            ("let a = { n: b }\n", ItemKind::Static),
            ("const a = [b];\n", ItemKind::Vtable),
        ] {
            let mut a = binding("a", js, &["b"]);
            a.kind = kind;
            let table = table(vec![a, binding("b", "let b = { n: a };\n", &["a"])]);
            let (text, errors) = printed(&table);
            assert_eq!(errors, 1, "{js}");
            assert!(text.contains(js), "{js}");
        }
    }

    #[test]
    fn a_literal_is_measured_past_the_brackets_in_its_strings() {
        assert_eq!(literal_len("{ a: \"}\", b: [1] }"), Some(18));
        assert_eq!(literal_len("[\"\\\"]\", 2]"), Some(10));
        assert_eq!(literal_len("{ a: 1 }.a"), Some(8));
        assert_eq!(literal_len("{ a: 'x' }"), None);
        assert_eq!(literal_len("{ a: [1 }"), None);
    }

    /// A declaration keeps the map entries that still point at it and drops those that pointed
    /// into the literal; every line of every fill maps where its member's declaration did, through
    /// the sources and names of the item that now holds it.
    #[test]
    fn every_line_of_a_cycle_stays_mapped() {
        let mut a = binding("a", "// a\nlet a = { next: b };\n", &["b"]);
        a.map.sources = vec!["src/a.rs".to_owned()];
        a.map.names = vec!["A".to_owned()];
        a.map.segments = vec![
            (0, 0, 0, 1, 0, -1),
            (1, 0, 0, 3, 0, 0),
            (1, 12, 0, 3, 4, -1),
        ];
        let mut b = binding("b", "// b\nlet b = { next: a };\n", &["a"]);
        b.map.sources = vec!["src/b.rs".to_owned()];
        b.map.segments = vec![(0, 0, 0, 7, 0, -1), (1, 0, 0, 8, 0, -1)];
        let table = table(vec![a, b]);
        let members: HashSet<&JsName> = table.keys().collect();
        let names: Vec<&JsName> = table.keys().collect();
        let steps = layout(&table, &members, &names);
        let cycles = initialize_cycles(&Sink::new(), &steps);
        assert_eq!(
            cycles[0][0].map.segments,
            [(0, 0, 0, 1, 0, -1), (1, 0, 0, 3, 0, 0)]
        );
        let last = &cycles[0][1];
        assert_eq!(
            last.js,
            "// b\nlet b = {};\nObject.assign(a, { next: b });\nObject.assign(b, { next: a });\n"
        );
        assert_eq!(last.map.sources, ["src/b.rs", "src/a.rs"]);
        assert_eq!(last.map.names, ["A"]);
        assert_eq!(
            last.map.segments,
            [
                (0, 0, 0, 7, 0, -1),
                (1, 0, 0, 8, 0, -1),
                (2, 0, 1, 3, 0, 0),
                (3, 0, 0, 8, 0, -1)
            ]
        );
    }

    /// A `__proto__` key, bare or quoted, at any depth, is found; a read, a string value or an
    /// element spelling it is not a key.
    #[test]
    fn a_proto_key_is_found_where_a_literal_would_take_it_as_the_prototype() {
        for text in [
            "{__proto__:a}",
            "{ id: 1, __proto__: b }",
            "{\"__proto__\": b}",
            "{ a: [{ x: 1 }, { __proto__ : 2 }] }",
        ] {
            assert!(has_proto_key(text), "{text}");
        }
        for text in [
            "{ a: x.__proto__ }",
            "{ a: \"__proto__\" }",
            "[__proto__, 1]",
            "{ a: \"{__proto__:1}\" }",
            "{ __proto: 1, proto__: 2 }",
        ] {
            assert!(!has_proto_key(text), "{text}");
        }
    }

    /// A cycle member holding a `__proto__` key is refused, since `Object.assign` would not copy
    /// what that key sets.
    #[test]
    fn a_cycle_member_with_a_proto_key_is_refused() {
        let table = table(vec![
            binding("a", "let a = { id: 1, __proto__: b };\n", &["b"]),
            binding("b", "let b = { id: 2, __proto__: a };\n", &["a"]),
        ]);
        let (text, errors) = printed(&table);
        assert_eq!(errors, 1);
        assert!(text.contains("let a = { id: 1, __proto__: b };\n"));
    }

    /// Units 0 (a chunk whose entry point is the static `entry`) and 1 (shared).
    fn two_units(
        entry_refs: &[&str],
        shared_refs: &[&str],
    ) -> (BTreeMap<JsName, LinkItem>, Vec<Vec<JsName>>) {
        let mut function = binding("f", "function f() {}\n", &[]);
        function.kind = ItemKind::Fn;
        let mut reader = binding("g", "function g() { return entry; }\n", &["entry"]);
        reader.kind = ItemKind::Fn;
        let table = table(vec![
            binding("entry", "let entry = {};\n", entry_refs),
            binding("leaf", "let leaf = {};\n", &[]),
            binding("hold", "let hold = {};\n", shared_refs),
            function,
            reader,
        ]);
        let holds = vec![
            vec![JsName::new("entry".to_owned())],
            ["leaf", "hold", "f", "g"]
                .map(|name| JsName::new(name.to_owned()))
                .to_vec(),
        ];
        (table, holds)
    }

    /// How many errors `evaluation_cycles` reports for `two_units`, given what each unit imports.
    fn evaluation_errors(
        entry_refs: &[&str],
        shared_refs: &[&str],
        imported: &[BTreeMap<usize, BTreeSet<&str>>],
    ) -> u32 {
        let (table, holds) = two_units(entry_refs, shared_refs);
        let owner: BTreeMap<&JsName, usize> = holds
            .iter()
            .enumerate()
            .flat_map(|(at, held)| held.iter().map(move |name| (name, at)))
            .collect();
        let dcx = Sink::new();
        evaluation_cycles(&dcx, &table, &holds, &owner, imported, |at| {
            format!("unit{at}")
        });
        dcx.errors()
    }

    /// A binding read around a cycle of imports is refused whichever side reads it and however
    /// the other side uses its import; a function import, a read from a function body and a
    /// one-way import are fine.
    #[test]
    fn a_binding_read_around_a_cycle_of_chunk_imports_is_refused() {
        let imports = |up: &[&'static str], down: &[&'static str]| {
            let mut imported = vec![BTreeMap::new(), BTreeMap::new()];
            if !up.is_empty() {
                imported[0].insert(1, up.iter().copied().collect());
            }
            if !down.is_empty() {
                imported[1].insert(0, down.iter().copied().collect());
            }
            imported
        };
        // The entry reads the shared `leaf`; the shared `hold` reads the entry.
        assert_eq!(
            evaluation_errors(&["leaf"], &["entry"], &imports(&["leaf"], &["entry"])),
            1
        );
        // The entry reads `leaf`; shared imports the entry back only for the function `g`.
        assert_eq!(
            evaluation_errors(&["leaf"], &[], &imports(&["leaf"], &["entry"])),
            1
        );
        // The entry imports only the function `f`; the shared `hold` reads the entry.
        assert_eq!(
            evaluation_errors(&["f"], &["entry"], &imports(&["f"], &["entry"])),
            1
        );
        // Around the cycle only a function is read, and the entry is named from a function body.
        assert_eq!(
            evaluation_errors(&["f"], &[], &imports(&["f"], &["entry"])),
            0
        );
        // One way only: the entry reads `leaf`, nothing imports the entry.
        assert_eq!(
            evaluation_errors(&["leaf"], &[], &imports(&["leaf"], &[])),
            0
        );
        // One way only: the shared `hold` reads the entry, which imports nothing.
        assert_eq!(
            evaluation_errors(&[], &["entry"], &imports(&[], &["entry"])),
            0
        );
    }

    /// A cycle whose members two chunks would own is refused; one owner is fine.
    #[test]
    fn a_cycle_split_across_chunks_is_refused() {
        let table = table(vec![
            binding("a", "let a = { n: b };\n", &["b"]),
            binding("b", "let b = { n: a };\n", &["a"]),
            binding("c", "let c = [];\n", &[]),
        ]);
        let [a, b, c] = ["a", "b", "c"].map(|name| JsName::new(name.to_owned()));
        let label = |at: usize| format!("chunk{at}");
        let together: BTreeMap<&JsName, usize> = [(&a, 2), (&b, 2), (&c, 0)].into_iter().collect();
        let dcx = Sink::new();
        split_cycles(&dcx, &table, &together, label);
        assert_eq!(dcx.errors(), 0);
        let apart: BTreeMap<&JsName, usize> = [(&a, 0), (&b, 2), (&c, 0)].into_iter().collect();
        let dcx = Sink::new();
        split_cycles(&dcx, &table, &apart, label);
        assert_eq!(dcx.errors(), 1);
    }

    /// Only members are walked: a binding another chunk owns is neither placed nor followed.
    #[test]
    fn a_cycle_is_only_found_among_the_members() {
        let table = table(vec![
            binding("a", "let a = { n: b };\n", &["b"]),
            binding("b", "let b = { n: a };\n", &["a"]),
        ]);
        let only_a = JsName::new("a".to_owned());
        let members: HashSet<&JsName> = [&only_a].into_iter().collect();
        let names = vec![&only_a];
        let steps = layout(&table, &members, &names);
        assert!(matches!(steps.as_slice(), [Placed::Item(item)] if item.name.as_str() == "a"));
    }

    /// A cycle whose initializer reads a property or element off any cycle member is refused,
    /// while 1-step primitive slots (`{ buf: b, off: "x" }`) and non-member property accesses
    /// stay allowed.
    #[test]
    fn a_cycle_reading_member_subfields_is_refused_while_slot_refs_succeed() {
        let check_errors = |items: Vec<LinkItem>| -> u32 {
            let table = table(items);
            let names: Vec<&JsName> = table.keys().collect();
            let members: HashSet<&JsName> = names.iter().copied().collect();
            let placed = layout(&table, &members, &names);
            let dcx = Sink::new();
            let _ = initialize_cycles(&dcx, &placed);
            dcx.errors()
        };

        // Allowed: 1-step primitive field cycle (`{ buf: b, off: "x" }`), string/key/non-cycle dot.
        assert_eq!(
            check_errors(vec![
                binding(
                    "a",
                    "let a = { x: 11, next: { buf: b, off: \"x\" }, b: 1, tag: \"b.inner\", ext: x.b.val };\n",
                    &["b"],
                ),
                binding(
                    "b",
                    "let b = { x: 22, next: { buf: a, off: \"x\" } };\n",
                    &["a"],
                ),
            ]),
            0
        );

        // Refused: mutual aggregate sub-field cycle (`other: b.inner`).
        assert_eq!(
            check_errors(vec![
                binding(
                    "a",
                    "let a = { inner: { v: 1 }, other: b.inner };\n",
                    &["b"]
                ),
                binding(
                    "b",
                    "let b = { inner: { v: 2 }, other: a.inner };\n",
                    &["a"]
                ),
            ]),
            1
        );

        // Refused: self-referential aggregate sub-field (`ptr: s.inner`).
        assert_eq!(
            check_errors(vec![binding(
                "s",
                "let s = { inner: { v: 42 }, ptr: s.inner };\n",
                &[],
            )]),
            1
        );

        // Refused: self-referential nested primitive slot (`{ buf: s.inner, off: "val" }`).
        assert_eq!(
            check_errors(vec![binding(
                "s",
                "let s = { inner: { val: 42 }, ptr: { buf: s.inner, off: \"val\" } };\n",
                &[],
            )]),
            1
        );

        // Refused: self-referential nested slice (`{ buf: s.arr, off: 0, len: 3 }`).
        assert_eq!(
            check_errors(vec![binding(
                "s",
                "let s = { arr: [10, 20, 30], sl: { buf: s.arr, off: 0, len: 3 } };\n",
                &[],
            )]),
            1
        );

        // Refused: bracket element read (`other: b[0]`).
        assert_eq!(
            check_errors(vec![
                binding("a", "let a = [{ v: 1 }, b[0]];\n", &["b"]),
                binding("b", "let b = [{ v: 2 }, a[0]];\n", &["a"]),
            ]),
            1
        );
    }
}
