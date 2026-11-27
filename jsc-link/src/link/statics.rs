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
/// object or array, any sub-objects/sub-arrays projected out of cycle members pre-allocated on
/// their parent paths, then every pre-allocated sub-container and member literal copied onto it.
///
/// The result has one item per member, in the members' order, each keeping its member's name,
/// linkage, references and type signature, so the export clause, the `.d.ts` and the rename see
/// the members they always saw. The declarations are the members' own texts with the literal
/// emptied; the pre-allocations and copies follow the last declaration, inside the last member's
/// item, because none of them may run before every member exists.
///
/// A member this module cannot split (see the module docs) is reported, and the cycle is returned
/// as it was, which the reported error keeps from ever being written.
fn initialize(dcx: &Sink, members: &[&LinkItem]) -> Vec<LinkItem> {
    let cycle_names: HashSet<&str> = members.iter().map(|member| member.name.as_str()).collect();
    let mut split = Vec::with_capacity(members.len());
    for member in members {
        match Declaration::of(member) {
            Ok(declaration) => split.push(declaration),
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

    // Collect every sub-field / element path read off a cycle member across all cycle initializers.
    let member_index_by_name: HashMap<&str, usize> = members
        .iter()
        .enumerate()
        .map(|(idx, member)| (member.name.as_str(), idx))
        .collect();
    let mut projected_paths: BTreeMap<usize, BTreeSet<Vec<PathStep>>> = BTreeMap::new();
    let mut bad_reader: Option<&str> = None;
    for (member, declaration) in members.iter().zip(&split) {
        match collect_cycle_projections(declaration.literal, &cycle_names) {
            Ok(paths) => {
                for (target_name, target_paths) in paths {
                    if let Some(&target_idx) = member_index_by_name.get(target_name.as_str()) {
                        projected_paths
                            .entry(target_idx)
                            .or_default()
                            .extend(target_paths);
                    }
                }
            }
            Err(()) => {
                bad_reader = Some(member.debug_path.as_str());
                break;
            }
        }
    }

    // Resolve each projected path to a direct `{...}` or `[...]` sub-literal in its target member.
    let mut resolved: BTreeMap<(usize, Vec<PathStep>), ResolvedSub> = BTreeMap::new();
    if bad_reader.is_none() {
        'resolve: for (&target_idx, paths) in &projected_paths {
            let literal = split[target_idx].literal;
            for path in paths {
                match resolve_sub_container(literal, path) {
                    Ok(sub) => {
                        resolved.insert((target_idx, path.clone()), sub);
                    }
                    Err(()) => {
                        // Report the member that reads this unresolvable projection (or the target).
                        bad_reader = members
                            .iter()
                            .zip(&split)
                            .find_map(|(m, decl)| {
                                collect_cycle_projections(decl.literal, &cycle_names)
                                    .ok()
                                    .and_then(|map| {
                                        map.get(members[target_idx].name.as_str())
                                            .is_some_and(|set| set.contains(path))
                                            .then_some(m.debug_path.as_str())
                                    })
                            })
                            .or(Some(members[target_idx].debug_path.as_str()));
                        break 'resolve;
                    }
                }
            }
        }
    }

    if let Some(reader) = bad_reader {
        let names: Vec<&str> = members
            .iter()
            .map(|member| member.debug_path.as_str())
            .collect();
        dcx.err(format!(
            "rustc_codegen_js: the statics `{}` refer to each other, and `{}` reads a \
             sub-field or element of a cycle member during initialization, so the \
             cycle cannot be initialized in two steps",
            names.join("`, `"),
            reader,
        ));
        return members.iter().map(|member| (*member).clone()).collect();
    }

    let mut out: Vec<LinkItem> = Vec::with_capacity(members.len());
    for (member, declaration) in members.iter().zip(&split) {
        let mut item = (*member).clone();
        item.js = declaration.declare_empty();
        item.map = declaration.declaration_map(&member.map);
        out.push(item);
    }
    let last = out.last_mut().expect("a cycle has a member");
    let append_mapped =
        |last: &mut LinkItem, member: &LinkItem, declaration: &Declaration<'_>, stmt: &str| {
            if let Some(origin) = declaration.origin(&member.map) {
                let first = last.js.matches('\n').count() as u32;
                let lines = stmt.matches('\n').count() as u32;
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
            last.js.push_str(stmt);
        };

    if resolved.is_empty() {
        for (member, declaration) in members.iter().zip(&split) {
            let fill = declaration.fill();
            append_mapped(last, member, declaration, &fill);
        }
        return out;
    }

    // Step 2: pre-allocate every referenced sub-container shallow-to-deep on its parent path.
    let mut prealloc_order: Vec<(usize, &Vec<PathStep>, ResolvedSub)> = resolved
        .iter()
        .map(|((idx, path), &sub)| (*idx, path, sub))
        .collect();
    prealloc_order.sort_by(|a, b| {
        a.1.len()
            .cmp(&b.1.len())
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(b.1))
    });
    for &(idx, path, sub) in &prealloc_order {
        let target_expr = format_path_expr(split[idx].name, path);
        let empty = match sub.kind {
            ContainerKind::Array => "[]",
            ContainerKind::Object => "{}",
        };
        let stmt = if split[idx].compact {
            format!("{target_expr}={empty};\n")
        } else {
            format!("{target_expr} = {empty};\n")
        };
        append_mapped(last, members[idx], &split[idx], &stmt);
    }

    // Step 3a: fill each pre-allocated sub-container (deepest-first), substituting any
    // pre-allocated direct children with their path expressions to preserve identity.
    for &(idx, path, sub) in prealloc_order.iter().rev() {
        let target_expr = format_path_expr(split[idx].name, path);
        let rewritten = rewrite_container_literal(
            split[idx].name,
            split[idx].literal,
            sub.span,
            path,
            idx,
            &resolved,
        );
        let stmt = format_assign(&target_expr, &rewritten, split[idx].compact);
        append_mapped(last, members[idx], &split[idx], &stmt);
    }

    // Step 3b: fill each root cycle member, substituting any pre-allocated top-level children.
    for (idx, (member, declaration)) in members.iter().zip(&split).enumerate() {
        let root_span = Span {
            start: 0,
            end: declaration.literal.len(),
        };
        let rewritten = rewrite_container_literal(
            declaration.name,
            declaration.literal,
            root_span,
            &[],
            idx,
            &resolved,
        );
        let stmt = format_assign(declaration.name, &rewritten, declaration.compact);
        append_mapped(last, member, declaration, &stmt);
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

enum LiteralToken<'a> {
    String(&'a str),
    Ident(&'a str),
}

struct LiteralTokenCtx<'a> {
    token: LiteralToken<'a>,
    before_previous: u8,
    previous: u8,
    next: Option<u8>,
    next_after_one: Option<u8>,
}

impl LiteralTokenCtx<'_> {
    fn is_object_key(&self) -> bool {
        matches!(self.previous, b'{' | b',') && self.next == Some(b':')
    }

    fn is_computed_proto_key(&self) -> bool {
        matches!(self.token, LiteralToken::String("\"__proto__\""))
            && self.previous == b'['
            && matches!(self.before_previous, b'{' | b',')
            && self.next == Some(b']')
            && self.next_after_one == Some(b':')
    }
}

/// Scans double-quoted strings and identifiers in `text` (a literal [`literal_len`] accepted),
/// invoking `predicate` with each token and its immediately surrounding non-whitespace bytes.
/// Returns `true` on the first token for which `predicate` returns `true`.
fn any_literal_token(text: &str, mut predicate: impl FnMut(LiteralTokenCtx<'_>) -> bool) -> bool {
    let bytes = text.as_bytes();
    let next_significant_pair = |mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let first = bytes.get(at).copied();
        let second = if first.is_some() {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            bytes.get(at).copied()
        } else {
            None
        };
        (first, second)
    };
    let mut before_previous = 0u8;
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
                let (next, next_after_one) = next_significant_pair(at);
                if predicate(LiteralTokenCtx {
                    token: LiteralToken::String(&text[start..at]),
                    before_previous,
                    previous,
                    next,
                    next_after_one,
                }) {
                    return true;
                }
                before_previous = previous;
                previous = b'"';
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' => {
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'_' | b'$'))
                {
                    at += 1;
                }
                let (next, next_after_one) = next_significant_pair(at);
                if predicate(LiteralTokenCtx {
                    token: LiteralToken::Ident(&text[start..at]),
                    before_previous,
                    previous,
                    next,
                    next_after_one,
                }) {
                    return true;
                }
                before_previous = previous;
                previous = bytes[at - 1];
            }
            byte if byte.is_ascii_whitespace() => at += 1,
            byte => {
                before_previous = previous;
                previous = byte;
                at += 1;
            }
        }
    }
    false
}

/// Whether `text`, a literal [`literal_len`] accepted, has a `__proto__` key anywhere: the bare
/// name, the string `"__proto__"`, or the computed key `["__proto__"]` directly after `{` or `,`
/// and directly before `:`.
///
/// `Object.assign` copies own properties via `[[Set]]`, which invokes `Object.prototype`'s
/// `__proto__` setter on an empty `{}` target instead of creating an own data property.
fn has_proto_key(text: &str) -> bool {
    any_literal_token(text, |tok| {
        tok.is_computed_proto_key()
            || (tok.is_object_key()
                && matches!(
                    tok.token,
                    LiteralToken::String("\"__proto__\"") | LiteralToken::Ident("__proto__")
                ))
    })
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PathStep {
    Prop(String),
    Index(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerKind {
    Object,
    Array,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedSub {
    span: Span,
    kind: ContainerKind,
}

fn format_assign(target: &str, literal: &str, compact: bool) -> String {
    if compact {
        format!("Object.assign({target},{literal});\n")
    } else {
        format!("Object.assign({target}, {literal});\n")
    }
}

fn format_path_expr(root: &str, path: &[PathStep]) -> String {
    let mut out = String::from(root);
    for step in path {
        match step {
            PathStep::Prop(key) if rcgjs_object::jsast::is_plain_ident(key) => {
                out.push('.');
                out.push_str(key);
            }
            PathStep::Prop(key) => {
                out.push('[');
                write_quoted_js_string(&mut out, key);
                out.push(']');
            }
            PathStep::Index(index) => {
                use std::fmt::Write;
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}

fn write_quoted_js_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$')
}

/// Parses a JS double-quoted string literal starting at `bytes[start] == b'"'`, returning the
/// unescaped string value and the byte index right after the closing quote.
fn parse_js_string(text: &str, start: usize) -> Result<(String, usize), ()> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return Err(());
    }
    let mut out = String::new();
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => return Ok((out, at + 1)),
            b'\\' => {
                at += 1;
                if at >= bytes.len() {
                    return Err(());
                }
                match bytes[at] {
                    b'"' => {
                        out.push('"');
                        at += 1;
                    }
                    b'\\' => {
                        out.push('\\');
                        at += 1;
                    }
                    b'n' => {
                        out.push('\n');
                        at += 1;
                    }
                    b'r' => {
                        out.push('\r');
                        at += 1;
                    }
                    b't' => {
                        out.push('\t');
                        at += 1;
                    }
                    b'0' => {
                        out.push('\0');
                        at += 1;
                    }
                    b'u' => {
                        if at + 5 > bytes.len() {
                            return Err(());
                        }
                        let hex = &text[at + 1..at + 5];
                        let code = u32::from_str_radix(hex, 16).map_err(|_| ())?;
                        let ch = char::from_u32(code).ok_or(())?;
                        out.push(ch);
                        at += 5;
                    }
                    b'x' => {
                        if at + 3 > bytes.len() {
                            return Err(());
                        }
                        let hex = &text[at + 1..at + 3];
                        let code = u32::from_str_radix(hex, 16).map_err(|_| ())?;
                        let ch = char::from_u32(code).ok_or(())?;
                        out.push(ch);
                        at += 3;
                    }
                    _ => return Err(()),
                }
            }
            _ => {
                let ch = text[at..].chars().next().ok_or(())?;
                out.push(ch);
                at += ch.len_utf8();
            }
        }
    }
    Err(())
}

/// Scans `text` for every property/element projection chain read off any identifier in
/// `cycle_names`, collecting every non-empty prefix per target member. Returns `Err(())` if a
/// cycle member is indexed dynamically (e.g., `B[i]`) or with malformed projection syntax.
fn collect_cycle_projections(
    text: &str,
    cycle_names: &HashSet<&str>,
) -> Result<BTreeMap<String, BTreeSet<Vec<PathStep>>>, ()> {
    let bytes = text.as_bytes();
    let mut out: BTreeMap<String, BTreeSet<Vec<PathStep>>> = BTreeMap::new();
    let mut previous = 0u8;
    let mut at = 0;
    while at < bytes.len() {
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
            byte if is_ident_start(byte) => {
                let start = at;
                while at < bytes.len() && is_ident_cont(bytes[at]) {
                    at += 1;
                }
                let ident = &text[start..at];
                let next_idx = skip_ws(bytes, at);
                let next = bytes.get(next_idx).copied();
                let is_key = matches!(previous, b'{' | b',') && next == Some(b':');
                if previous != b'.'
                    && !is_key
                    && cycle_names.contains(ident)
                    && matches!(next, Some(b'.' | b'['))
                {
                    let mut steps = Vec::new();
                    let mut cur = next_idx;
                    while let Some(&ch) = bytes.get(cur) {
                        match ch {
                            b'.' => {
                                cur = skip_ws(bytes, cur + 1);
                                let pstart = cur;
                                if cur >= bytes.len() || !is_ident_start(bytes[cur]) {
                                    return Err(());
                                }
                                while cur < bytes.len() && is_ident_cont(bytes[cur]) {
                                    cur += 1;
                                }
                                steps.push(PathStep::Prop(text[pstart..cur].to_owned()));
                                cur = skip_ws(bytes, cur);
                            }
                            b'[' => {
                                cur = skip_ws(bytes, cur + 1);
                                if cur >= bytes.len() {
                                    return Err(());
                                }
                                if bytes[cur] == b'"' {
                                    let (key, after) = parse_js_string(text, cur)?;
                                    cur = skip_ws(bytes, after);
                                    if bytes.get(cur) != Some(&b']') {
                                        return Err(());
                                    }
                                    cur = skip_ws(bytes, cur + 1);
                                    steps.push(PathStep::Prop(key));
                                } else if bytes[cur].is_ascii_digit() {
                                    let dstart = cur;
                                    while cur < bytes.len() && bytes[cur].is_ascii_digit() {
                                        cur += 1;
                                    }
                                    let index: usize = text[dstart..cur].parse().map_err(|_| ())?;
                                    cur = skip_ws(bytes, cur);
                                    if bytes.get(cur) != Some(&b']') {
                                        return Err(());
                                    }
                                    cur = skip_ws(bytes, cur + 1);
                                    steps.push(PathStep::Index(index));
                                } else {
                                    return Err(());
                                }
                            }
                            _ => break,
                        }
                    }
                    if steps.is_empty() {
                        return Err(());
                    }
                    let entry = out.entry(ident.to_owned()).or_default();
                    for len in 1..=steps.len() {
                        entry.insert(steps[..len].to_vec());
                    }
                    at = cur;
                    previous = b']';
                } else {
                    previous = bytes[at - 1];
                }
            }
            byte if byte.is_ascii_whitespace() => at += 1,
            byte => {
                previous = byte;
                at += 1;
            }
        }
    }
    Ok(out)
}

/// Scans a single top-level expression inside an object or array literal starting at `start` up
/// to the matching delimiter (`','`, `'}'`, or `']'`) at depth 0, returning the trimmed value span
/// and the byte index of the delimiter.
fn scan_value_span(text: &str, start: usize, close: u8) -> Result<(Span, usize), ()> {
    let bytes = text.as_bytes();
    let vstart = skip_ws(bytes, start);
    let mut at = vstart;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while at < bytes.len() {
        let b = bytes[at];
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            at += 1;
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => {
                if depth == 0 {
                    if b == close {
                        break;
                    }
                    return Err(());
                }
                depth -= 1;
            }
            b',' if depth == 0 => break,
            _ => {}
        }
        at += 1;
    }
    if in_string || depth != 0 || at >= bytes.len() {
        return Err(());
    }
    let mut vend = at;
    while vend > vstart && bytes[vend - 1].is_ascii_whitespace() {
        vend -= 1;
    }
    if vend == vstart {
        return Err(());
    }
    Ok((
        Span {
            start: vstart,
            end: vend,
        },
        at,
    ))
}

fn parse_object_entries(text: &str, span: Span) -> Result<Vec<(String, Span)>, ()> {
    let bytes = text.as_bytes();
    if bytes.get(span.start) != Some(&b'{')
        || bytes.get(span.end.checked_sub(1).ok_or(())?) != Some(&b'}')
    {
        return Err(());
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut at = skip_ws(bytes, span.start + 1);
    let end = span.end - 1;
    while at < end {
        let (key, after_key) = match bytes[at] {
            b'"' => parse_js_string(text, at)?,
            b'[' => {
                let inner = skip_ws(bytes, at + 1);
                let (key, after_str) = parse_js_string(text, inner)?;
                let close = skip_ws(bytes, after_str);
                if bytes.get(close) != Some(&b']') {
                    return Err(());
                }
                (key, close + 1)
            }
            b if is_ident_start(b) => {
                let kstart = at;
                let mut kend = at + 1;
                while kend < end && is_ident_cont(bytes[kend]) {
                    kend += 1;
                }
                (text[kstart..kend].to_owned(), kend)
            }
            b if b.is_ascii_digit() => {
                let kstart = at;
                let mut kend = at + 1;
                while kend < end && bytes[kend].is_ascii_digit() {
                    kend += 1;
                }
                (text[kstart..kend].to_owned(), kend)
            }
            _ => return Err(()),
        };
        if !seen.insert(key.clone()) {
            return Err(());
        }
        let colon = skip_ws(bytes, after_key);
        if bytes.get(colon) != Some(&b':') {
            return Err(());
        }
        let (val_span, delim) = scan_value_span(text, colon + 1, b'}')?;
        if delim > end {
            return Err(());
        }
        entries.push((key, val_span));
        if bytes[delim] == b',' {
            at = skip_ws(bytes, delim + 1);
        } else {
            at = delim;
        }
    }
    Ok(entries)
}

fn parse_array_elements(text: &str, span: Span) -> Result<Vec<Span>, ()> {
    let bytes = text.as_bytes();
    if bytes.get(span.start) != Some(&b'[')
        || bytes.get(span.end.checked_sub(1).ok_or(())?) != Some(&b']')
    {
        return Err(());
    }
    let mut elems = Vec::new();
    let mut at = skip_ws(bytes, span.start + 1);
    let end = span.end - 1;
    while at < end {
        let (val_span, delim) = scan_value_span(text, at, b']')?;
        if delim > end || text[val_span.start..val_span.end].starts_with("...") {
            return Err(());
        }
        elems.push(val_span);
        if bytes[delim] == b',' {
            at = skip_ws(bytes, delim + 1);
        } else {
            at = delim;
        }
    }
    Ok(elems)
}

fn resolve_sub_container(literal: &str, path: &[PathStep]) -> Result<ResolvedSub, ()> {
    let mut span = Span {
        start: 0,
        end: literal.len(),
    };
    for step in path {
        match step {
            PathStep::Prop(key) => {
                let entries = parse_object_entries(literal, span)?;
                let Some((_, next)) = entries.into_iter().find(|(k, _)| k == key) else {
                    return Err(());
                };
                span = next;
            }
            PathStep::Index(index) => {
                let elems = parse_array_elements(literal, span)?;
                span = *elems.get(*index).ok_or(())?;
            }
        }
    }
    let slice = &literal[span.start..span.end];
    let kind = match slice.as_bytes().first() {
        Some(b'{') => ContainerKind::Object,
        Some(b'[') => ContainerKind::Array,
        _ => return Err(()),
    };
    if literal_len(slice) != Some(slice.len()) {
        return Err(());
    }
    Ok(ResolvedSub { span, kind })
}

fn rewrite_container_literal(
    root_name: &str,
    literal: &str,
    span: Span,
    parent_path: &[PathStep],
    member_idx: usize,
    resolved: &BTreeMap<(usize, Vec<PathStep>), ResolvedSub>,
) -> String {
    let mut replacements: Vec<(Span, String)> = resolved
        .iter()
        .filter_map(|((idx, path), sub)| {
            if *idx == member_idx
                && path.len() == parent_path.len() + 1
                && path[..parent_path.len()] == *parent_path
            {
                Some((sub.span, format_path_expr(root_name, path)))
            } else {
                None
            }
        })
        .collect();
    if replacements.is_empty() {
        return literal[span.start..span.end].to_owned();
    }
    replacements.sort_by_key(|(child_span, _)| child_span.start);
    let mut out = String::with_capacity(span.end - span.start);
    let mut cursor = span.start;
    for (child_span, expr) in replacements {
        out.push_str(&literal[cursor..child_span.start]);
        out.push_str(&expr);
        cursor = child_span.end;
    }
    out.push_str(&literal[cursor..span.end]);
    out
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
            "{[\"__proto__\"]:a}",
            "{ id: 1, [\"__proto__\"]: b }",
            "{ a: [{ x: 1 }, { __proto__ : 2 }] }",
            "{ a: [{ x: 1 }, { [\"__proto__\"]: 2 }] }",
        ] {
            assert!(has_proto_key(text), "{text}");
        }
        for text in [
            "{ a: x.__proto__ }",
            "{ a: \"__proto__\" }",
            "{ a: [\"__proto__\"] }",
            "[[\"__proto__\"]]",
            "{ a: x[\"__proto__\"] }",
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

    /// A cycle reading nested object/array sub-containers off cycle members initializes via
    /// three-step skeleton pre-allocation, while reading a scalar or unresolvable property off
    /// a cycle member stays refused.
    #[test]
    fn a_cycle_reading_member_subfields_preallocates_containers_and_refuses_scalars() {
        let check = |items: Vec<LinkItem>| -> (String, usize) { printed(&table(items)) };

        // 1-step primitive field cycle (`{ buf: b, off: "x" }`), string/key/non-cycle dot.
        let (_, errs) = check(vec![
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
        ]);
        assert_eq!(errs, 0);

        // Mutual aggregate sub-field cycle (`other: b.inner`).
        let (text, errs) = check(vec![
            binding(
                "a",
                "let a = { inner: { v: 1 }, other: b.inner };\n",
                &["b"],
            ),
            binding(
                "b",
                "let b = { inner: { v: 2 }, other: a.inner };\n",
                &["a"],
            ),
        ]);
        assert_eq!(errs, 0);
        assert_eq!(
            text,
            "let a = {};\n\
             let b = {};\n\
             a.inner = {};\n\
             b.inner = {};\n\
             Object.assign(b.inner, { v: 2 });\n\
             Object.assign(a.inner, { v: 1 });\n\
             Object.assign(a, { inner: a.inner, other: b.inner });\n\
             Object.assign(b, { inner: b.inner, other: a.inner });\n"
        );

        // Self-referential aggregate sub-field (`ptr: s.inner`).
        let (text, errs) = check(vec![binding(
            "s",
            "let s = { inner: { v: 42 }, ptr: s.inner };\n",
            &[],
        )]);
        assert_eq!(errs, 0);
        assert_eq!(
            text,
            "let s = {};\n\
             s.inner = {};\n\
             Object.assign(s.inner, { v: 42 });\n\
             Object.assign(s, { inner: s.inner, ptr: s.inner });\n"
        );

        // Self-referential nested primitive slot (`{ buf: s.inner, off: "val" }`).
        let (text, errs) = check(vec![binding(
            "s",
            "let s = { inner: { val: 42 }, ptr: { buf: s.inner, off: \"val\" } };\n",
            &[],
        )]);
        assert_eq!(errs, 0);
        assert!(text.contains("s.inner = {};\n"));
        assert!(text.contains("Object.assign(s.inner, { val: 42 });\n"));

        // Self-referential nested slice (`{ buf: s.arr, off: 0, len: 3 }`).
        let (text, errs) = check(vec![binding(
            "s",
            "let s = { arr: [10, 20, 30], sl: { buf: s.arr, off: 0, len: 3 } };\n",
            &[],
        )]);
        assert_eq!(errs, 0);
        assert!(text.contains("s.arr = [];\n"));
        assert!(text.contains("Object.assign(s.arr, [10, 20, 30]);\n"));

        // Bracket element read (`other: b[0]`).
        let (text, errs) = check(vec![
            binding("a", "let a = [{ v: 1 }, b[0]];\n", &["b"]),
            binding("b", "let b = [{ v: 2 }, a[0]];\n", &["a"]),
        ]);
        assert_eq!(errs, 0);
        assert!(text.contains("a[0] = {};\n"));
        assert!(text.contains("b[0] = {};\n"));
        assert!(text.contains("Object.assign(a, [a[0], b[0]]);\n"));

        // Refused: reading a scalar property off a cycle member (`other: b.scalar`).
        let (_, errs) = check(vec![
            binding("a", "let a = { scalar: 1, other: b.scalar };\n", &["b"]),
            binding("b", "let b = { scalar: 2, other: a.scalar };\n", &["a"]),
        ]);
        assert_eq!(errs, 1);

        // Refused: reading a missing property off a cycle member.
        let (_, errs) = check(vec![
            binding("a", "let a = { x: 1, other: b.missing };\n", &["b"]),
            binding("b", "let b = { x: 2, other: a };\n", &["a"]),
        ]);
        assert_eq!(errs, 1);
    }
}
