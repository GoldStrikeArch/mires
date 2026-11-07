//! Minification: shorter names, in two passes that run in two different places.
//!
//! `-Cllvm-args=js-minify=on` turns on three things at once. Two of them live here:
//!
//! * [`shorten_locals`] renames a function's **locals** — parameters, `let`s, the `$`-temporaries,
//!   the structurizer's labels — to `a`, `b`, `c`, ... It runs at codegen time, on the JavaScript
//!   AST, once per item.
//! * [`item_renames`] renames the program's **items** — every top level function, static and
//!   vtable whose name is not fixed — to `$a`, `$b`, `$c`, ... It runs at link time, on the
//!   finished program text, once.
//!
//! The third is the compact printer, which is `jsast::Style::Compact` and belongs to the printer.
//!
//! # Why the two passes cannot be one
//!
//! Renaming an item requires seeing every reference to it, which only the link step can: an item is
//! codegenned by whichever crate instantiated it and may be called from any other. But by the time
//! the link step runs, an item is *text* — `link.rs` splices printed declarations, it does not hold
//! ASTs, and there is no JavaScript parser in this backend to give it any. So the item pass is
//! textual, and the local pass, which needs scopes and therefore the AST, has to happen earlier,
//! where the whole program is not visible.
//!
//! That split is fine because the two namespaces are made disjoint by construction: **a generated
//! item name always starts with `$` and a generated local name never does.** A local can therefore
//! never shadow an item, whatever order the two passes run in and whichever crate each name came
//! from. It is also why the item pass may be textual without being a guess — see below.
//!
//! # Why a textual item rename is sound
//!
//! The pass never asks what an identifier *is*. It substitutes an identifier only when the exact
//! string is a name in the link step's own item table, which is the same table `link.rs` intersects
//! `refs` against. Three rules keep that from firing anywhere it should not:
//!
//! * string literals are skipped, so an enum variant's name, a def path or a zombie message that
//!   happens to spell an item name is left alone. A variant tag is the case that matters: it is
//!   compared with `===` against the same string the constructor built, and both sides have to keep
//!   whatever the Rust source called the variant;
//! * `//` comments are skipped, for the same reason;
//! * an identifier directly after a `.` is a property, and properties are a different namespace —
//!   the same structural argument `jsast::visit_idents` already relies on.
//!
//! The scanner needs no more than that because the text is nearly all this backend's own printer
//! output, which has no regular expression literals, no template literals, no single quoted strings
//! and no block comments. The exception is a `js!{}` block, which is verbatim JavaScript and may
//! hold all four. A name inside one is substituted only if it spells an item exactly, and an item
//! name carries a `$h` hash infix or is `$a` from the pass itself, so a block would have to spell
//! one to be touched.
//!
//! # The allocator
//!
//! js_of_ocaml's `js_assign.ml` documents the strategy and the measurements behind it: assign
//! `a`, `b`, ... to parameters positionally first (so that call sites and signatures across the
//! program share a spelling, which is what a compressor rewards), then take the remaining names in
//! order of *occurrence count*, giving each the shortest name still free. Greedy, no interference
//! graph, and measured there within 0.06% of the ILP optimum. The alphabet is jsoo's too: 54
//! characters may start an identifier and 64 may continue one, encoded little-endian so that the
//! common short names stay short — minus `$` in the leading position, which is what reserves that
//! namespace for the item pass.
//!
//! # What is deliberately not renamed
//!
//! * **Item names, at codegen time.** They carry the `$h` hash infix and are the link step's
//!   business; the local pass sees them as free identifiers and merely refuses to reuse them.
//! * **Fixed names** (`#[no_mangle]`, `#[export_name]`, an imported binding), at either pass.
//!   `counter_clicked` is called from `demo/index.html` and `rust_entry` from `scripts/entry.js`;
//!   renaming those would be renaming the program's public interface.
//! * **`__rt`, the imported bindings and the JavaScript globals.** They are free identifiers in
//!   every function that uses one, so the local pass already refuses to reuse their spellings;
//!   [`GLOBALS`] and [`IMPORT_PREFIX`] additionally refuse them in functions that do *not*, which
//!   is belt and braces at the price of a lookup.
//! * **`refs`.** A `JsItem`'s reference set is left exactly as codegen computed it. It is an
//!   over-approximation intersected with the item table, and local renaming cannot change which
//!   *item* names an item mentions; recomputing it would only swap one set of junk entries
//!   (`acc`, `_3`) for another (`a`, `b`), and the junk is what the intersection exists to discard.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsItem, LinkItem};
use crate::jsast::{self, ArrowBody, Expr, Stmt};

/// Globals the emitted JavaScript reads. A generated name is never one of these.
///
/// The same list as `names.rs::RESERVED`, for the same reason and one step further along: a local
/// named `Math` would shadow the real one. In practice the allocator cannot reach a name this long
/// (three characters need more than 3445 distinct locals in one function), so this is insurance,
/// not a working rule.
///
/// `__rt` is the runtime shim under either module mode: a global the host defines in a script, the
/// imported namespace in an ES module. Shadowing it would break the same calls either way.
const GLOBALS: &[&str] = &[
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
    "console",
    "eval",
    "globalThis",
    "undefined",
];

// -------------------------------------------------------------------------------------------
// The alphabet
// -------------------------------------------------------------------------------------------

/// The prefix a binding imported from the DOM runtime carries: `_$insert`, `_$template`.
///
/// No generated name ever starts with it, so however many locals a function has, none of them can
/// shadow an imported binding. A prefix rather than a list of names, because which bindings exist
/// is the DOM runtime's ABI and not this backend's business.
const IMPORT_PREFIX: &str = "_$";

/// Characters that may start a generated *local* name. js_of_ocaml's 54, minus `$`.
const LOCAL_FIRST: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_";
/// Characters that may continue any generated name. js_of_ocaml's 64.
const REST: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$";

/// The `n`th name over an alphabet, little-endian: the last character varies fastest, so a low `n`
/// is a short name. Handing the low numbers to the frequent names is the whole of the strategy.
///
/// A direct port of js_of_ocaml's `Var_printer.Alphabet.to_string`.
fn nth_name(first: &str, rest: &str, n: usize) -> String {
    let (first, rest) = (first.as_bytes(), rest.as_bytes());
    // How many characters `n` needs: one, plus one per whole `rest.len()` beyond the first block.
    let mut size = 1;
    let mut x = n;
    while x >= first.len() {
        x = (x - first.len()) / rest.len();
        size += 1;
    }

    let mut out = vec![0u8; size];
    let mut x = n;
    for i in (1..size).rev() {
        x -= first.len();
        out[i] = rest[x % rest.len()];
        x /= rest.len();
    }
    out[0] = first[x];
    String::from_utf8(out).expect("the alphabets are ASCII")
}

/// Hands out the shortest name not yet used and not in an exclusion set.
struct Names<'a> {
    first: &'static str,
    rest: &'static str,
    /// Prepended to every name, which is how the item pass claims the `$` namespace.
    prefix: &'static str,
    next: usize,
    excluded: &'a BTreeSet<String>,
}

impl<'a> Names<'a> {
    /// Names for locals and labels: `a`, `b`, ..., never starting with `$`.
    fn locals(excluded: &'a BTreeSet<String>) -> Names<'a> {
        Names { first: LOCAL_FIRST, rest: REST, prefix: "", next: 0, excluded }
    }

    /// Names for items: `$a`, `$b`, ..., which no local can ever be.
    fn items(excluded: &'a BTreeSet<String>) -> Names<'a> {
        Names { first: REST, rest: REST, prefix: "$", next: 0, excluded }
    }

    fn take(&mut self) -> String {
        loop {
            let name = format!("{}{}", self.prefix, nth_name(self.first, self.rest, self.next));
            self.next += 1;
            if jsast::is_reserved(&name)
                || name.starts_with(IMPORT_PREFIX)
                || self.excluded.contains(&name)
            {
                continue;
            }
            return name;
        }
    }
}

// -------------------------------------------------------------------------------------------
// Pass 1: locals, at codegen time
// -------------------------------------------------------------------------------------------

/// Renames the locals of every item in a codegen unit. A no-op unless `js-minify` is on.
pub(crate) fn shorten_locals(items: &mut [JsItem]) {
    if !crate::opts::get().minify {
        return;
    }
    for item in items {
        shorten_locals_in(&mut item.decl);
    }
}

/// Renames the locals of one declaration.
///
/// The declaration's own name is the item's, which only the link step may touch, so it goes into
/// the exclusion set rather than into the pool. Everything a function body declares — parameters,
/// `let`s, `const`s, the parameters of the arrows in it — is one flat pool, which is both simpler
/// than js_of_ocaml's per-scope bitsets and correct: renaming by name is a bijection over the
/// function's identifiers, so whatever shadowing the input had, the output has exactly the same.
fn shorten_locals_in(decl: &mut Stmt) {
    let mut collect = Collect::default();
    let own: Option<String> = match decl {
        Stmt::FunctionDecl { name, params, body } => {
            for param in params.iter_mut() {
                collect.add_param(param);
            }
            walk_stmts(body, &mut collect);
            Some(name.clone())
        }
        // A module level binding: a `static`, a vtable. Its name is the item's; its initializer may
        // still hold arrows with parameters of their own.
        Stmt::Let(name, Some(init)) | Stmt::Const(name, init) => {
            walk_expr(init, &mut collect);
            Some(name.clone())
        }
        _ => return,
    };

    // Verbatim JavaScript is opaque: it could mention any of these names and the walk would never
    // know. A `js!{}` block is one, so an item holding a block keeps its long local names rather
    // than being miscompiled.
    if collect.has_raw {
        return;
    }

    let mut excluded: BTreeSet<String> = GLOBALS.iter().map(|name| (*name).to_string()).collect();
    excluded.extend(own);
    for (name, _) in &collect.tally {
        if !collect.declared.contains(name) {
            excluded.insert(name.clone());
        }
    }

    let vars = assign(&collect.params, &collect.declared, &collect.tally, &excluded);
    // Labels are a namespace of their own in JavaScript: `a: for (;;) break a;` is legal beside a
    // variable `a`, so the labels start the alphabet again.
    let no_exclusions = BTreeSet::new();
    let labels = assign(&[], &collect.labels, &collect.label_tally, &no_exclusions);

    let mut apply = Apply { vars: &vars, labels: &labels };
    match decl {
        Stmt::FunctionDecl { params, body, .. } => {
            for param in params.iter_mut() {
                apply.param(param);
            }
            walk_stmts(body, &mut apply);
        }
        Stmt::Let(_, Some(init)) | Stmt::Const(_, init) => walk_expr(init, &mut apply),
        _ => {}
    }
}

/// The allocation itself: parameters positionally, then the rest by falling occurrence count.
///
/// The tie-break on the name is not cosmetic. It makes the output a function of the input alone —
/// two runs of the same compilation must agree — and js_of_ocaml measured 1.8% off its compressed
/// size by ordering ties this way rather than leaving them to the hash table.
fn assign(
    params: &[String],
    declared: &BTreeSet<String>,
    tally: &BTreeMap<String, usize>,
    excluded: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut names = Names::locals(excluded);
    let mut out = BTreeMap::new();

    for param in params {
        out.entry(param.clone()).or_insert_with(|| names.take());
    }

    let mut rest: Vec<(&String, usize)> = tally
        .iter()
        .filter(|(name, _)| declared.contains(*name) && !out.contains_key(*name))
        .map(|(name, count)| (name, *count))
        .collect();
    rest.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    for (name, _) in rest {
        out.insert(name.clone(), names.take());
    }
    out
}

/// What a walk does at each kind of name. One walk, two implementations: one reads, one writes.
trait Visit {
    fn declared(&mut self, name: &mut String);
    fn param(&mut self, name: &mut String);
    fn used(&mut self, name: &mut String);
    fn label_declared(&mut self, name: &mut String);
    fn label_used(&mut self, name: &mut String);
    fn raw(&mut self) {}
}

/// Every name a declaration binds, every name it mentions, and how often.
#[derive(Default)]
struct Collect {
    declared: BTreeSet<String>,
    /// Parameters in the order they are declared, function's own first. Deduplicated: two arrows
    /// may spell their parameter the same way.
    params: Vec<String>,
    /// Occurrences of every identifier, bound or free. A name not in `declared` is free.
    tally: BTreeMap<String, usize>,
    labels: BTreeSet<String>,
    label_tally: BTreeMap<String, usize>,
    has_raw: bool,
}

impl Collect {
    fn add_param(&mut self, name: &str) {
        if self.declared.insert(name.to_owned()) {
            self.params.push(name.to_owned());
        }
        *self.tally.entry(name.to_owned()).or_default() += 1;
    }
}

impl Visit for Collect {
    fn declared(&mut self, name: &mut String) {
        self.declared.insert(name.clone());
        *self.tally.entry(name.clone()).or_default() += 1;
    }

    fn param(&mut self, name: &mut String) {
        self.add_param(name);
    }

    fn used(&mut self, name: &mut String) {
        *self.tally.entry(name.clone()).or_default() += 1;
    }

    fn label_declared(&mut self, name: &mut String) {
        self.labels.insert(name.clone());
        *self.label_tally.entry(name.clone()).or_default() += 1;
    }

    fn label_used(&mut self, name: &mut String) {
        *self.label_tally.entry(name.clone()).or_default() += 1;
    }

    fn raw(&mut self) {
        self.has_raw = true;
    }
}

struct Apply<'a> {
    vars: &'a BTreeMap<String, String>,
    labels: &'a BTreeMap<String, String>,
}

impl Apply<'_> {
    fn rename(map: &BTreeMap<String, String>, name: &mut String) {
        if let Some(short) = map.get(name.as_str()) {
            name.clone_from(short);
        }
    }
}

impl Visit for Apply<'_> {
    fn declared(&mut self, name: &mut String) {
        Apply::rename(self.vars, name);
    }

    fn param(&mut self, name: &mut String) {
        Apply::rename(self.vars, name);
    }

    fn used(&mut self, name: &mut String) {
        Apply::rename(self.vars, name);
    }

    fn label_declared(&mut self, name: &mut String) {
        Apply::rename(self.labels, name);
    }

    fn label_used(&mut self, name: &mut String) {
        Apply::rename(self.labels, name);
    }
}

fn walk_stmts<V: Visit>(stmts: &mut [Stmt], v: &mut V) {
    for stmt in stmts {
        walk_stmt(stmt, v);
    }
}

fn walk_stmt<V: Visit>(stmt: &mut Stmt, v: &mut V) {
    match stmt {
        Stmt::ExprStmt(expr) | Stmt::Throw(expr) => walk_expr(expr, v),
        Stmt::Let(name, init) => {
            v.declared(name);
            if let Some(init) = init {
                walk_expr(init, v);
            }
        }
        Stmt::Lets(decls) => {
            for (name, init) in decls {
                v.declared(name);
                if let Some(init) = init {
                    walk_expr(init, v);
                }
            }
        }
        Stmt::Const(name, init) => {
            v.declared(name);
            walk_expr(init, v);
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                walk_expr(value, v);
            }
        }
        Stmt::If(test, then, els) => {
            walk_expr(test, v);
            walk_stmts(then, v);
            if let Some(els) = els {
                walk_stmts(els, v);
            }
        }
        Stmt::Switch(disc, cases) => {
            walk_expr(disc, v);
            for case in cases {
                for test in &mut case.tests {
                    walk_expr(test, v);
                }
                walk_stmts(&mut case.body, v);
            }
        }
        Stmt::Labeled(label, inner) => {
            v.label_declared(label);
            walk_stmt(inner, v);
        }
        Stmt::LoopForever(label, body) => {
            if let Some(label) = label {
                v.label_declared(label);
            }
            walk_stmts(body, v);
        }
        Stmt::While(test, body) => {
            walk_expr(test, v);
            walk_stmts(body, v);
        }
        Stmt::Continue(label) | Stmt::Break(label) => {
            if let Some(label) = label {
                v.label_used(label);
            }
        }
        // Never emitted inside a body today; treated as a nested binding rather than assumed away.
        Stmt::FunctionDecl { name, params, body } => {
            v.declared(name);
            for param in params {
                v.param(param);
            }
            walk_stmts(body, v);
        }
        Stmt::Block(body) => walk_stmts(body, v),
        Stmt::Raw(_) => v.raw(),
        // An import declares names the whole program shares, and `link.rs` keeps them exactly as
        // they are spelled: renaming one here would rewrite the binding inside the `import` and
        // leave every use of it looking for something the module does not export.
        Stmt::Import { .. } | Stmt::Comment(_) | Stmt::Loc(_) => {}
    }
}

fn walk_expr<V: Visit>(expr: &mut Expr, v: &mut V) {
    match expr {
        Expr::Ident(name) => v.used(name),
        Expr::Member(obj, _) => walk_expr(obj, v),
        Expr::Index(obj, index) => {
            walk_expr(obj, v);
            walk_expr(index, v);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            walk_expr(callee, v);
            for arg in args {
                walk_expr(arg, v);
            }
        }
        Expr::Unary(_, operand) => walk_expr(operand, v),
        Expr::Binary(_, lhs, rhs) | Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            walk_expr(lhs, v);
            walk_expr(rhs, v);
        }
        Expr::Cond(test, consequent, alternate) => {
            walk_expr(test, v);
            walk_expr(consequent, v);
            walk_expr(alternate, v);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                walk_expr(value, v);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                walk_expr(item, v);
            }
        }
        Expr::Arrow(params, body) => {
            for param in params {
                v.param(param);
            }
            match body {
                ArrowBody::Expr(body) => walk_expr(body, v),
                ArrowBody::Block(body) => walk_stmts(body, v),
            }
        }
        Expr::Raw(_) => v.raw(),
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => {}
    }
}

// -------------------------------------------------------------------------------------------
// Pass 2: item names, at link time
// -------------------------------------------------------------------------------------------

/// The short name every item whose name is not fixed is renamed to: `$a`, `$b`, ...
///
/// `items` is the printed set, which is where both the names to rename and the names that must
/// survive come from, and `corpus` is the text those items were printed as. The two are separate
/// arguments because a split program is several files printed out of one item set, and one map over
/// the concatenation is what gives a name that crosses a file boundary the same spelling on both
/// sides. [`apply_renames`] does the rewriting.
pub(crate) fn item_renames(corpus: &str, items: &[&LinkItem]) -> BTreeMap<String, String> {
    let mut renamable: BTreeSet<String> = BTreeSet::new();
    let mut kept: BTreeSet<String> = BTreeSet::new();
    for item in items {
        let name = item.name.as_str().to_owned();
        // A fixed name is one something outside the program spells out: `rust_entry`,
        // `counter_clicked`, an imported binding. It is fixed whichever crate the item came from,
        // because a dependency's `#[no_mangle]` item is called by that name from outside too.
        if item.linkage.fixed_name.is_some() {
            kept.insert(name);
        } else {
            renamable.insert(name);
        }
    }
    if renamable.is_empty() {
        return BTreeMap::new();
    }

    // An item whose name is also a **property** name anywhere in the program keeps its own name.
    //
    // The two live in different JavaScript namespaces, but this pass rewrites text, and the
    // scanner below can tell a property read (`d.exp`) from a name in code position while an
    // object literal's *key* (`{ exp: e }`) reads exactly like one. Renaming the item would
    // rewrite the key with it and leave the reads looking for a field that is no longer there.
    //
    // It is a real collision rather than a hypothetical one: `compiler_builtins` exports `exp`,
    // and `core`'s float formatter passes a `flt2dec::Decoded` whose field is also `exp`, so any
    // program that formats an `f64` has both. Keeping one long name costs a few bytes; the
    // alternative is a silently wrong program.
    let mut properties: BTreeSet<&str> = BTreeSet::new();
    for_each_ident_kind(corpus, |_, ident, kind| {
        if kind == IdentKind::Property {
            properties.insert(ident);
        }
    });
    renamable.retain(|name| !properties.contains(name.as_str()));
    if renamable.is_empty() {
        return BTreeMap::new();
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for_each_ident(corpus, |_, ident| {
        if renamable.contains(ident) {
            *counts.entry(ident).or_default() += 1;
        }
    });

    let mut order: Vec<(&str, usize)> = counts.into_iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let mut names = Names::items(&kept);
    order.into_iter().map(|(name, _)| (name.to_owned(), names.take())).collect()
}

/// Rewrites every identifier [`item_renames`] gave a short name.
///
/// An empty map leaves the text alone, byte for byte, which is what a program with nothing to
/// rename gets.
pub(crate) fn apply_renames(program: &str, renames: &BTreeMap<String, String>) -> String {
    if renames.is_empty() {
        return program.to_owned();
    }
    let mut out = String::with_capacity(program.len());
    let mut cursor = 0;
    for_each_ident(program, |at, ident| {
        if let Some(short) = renames.get(ident) {
            out.push_str(&program[cursor..at]);
            out.push_str(short);
            cursor = at + ident.len();
        }
    });
    out.push_str(&program[cursor..]);
    out
}

/// Calls `f` with the byte offset and text of every identifier in *code* position.
///
/// Not a JavaScript lexer: a scanner for the subset this backend's printer emits, which has no
/// regular expression literals, no template literals, no single quoted strings and no block
/// comments. It skips double quoted strings and `//` comments, consumes numeric literals whole (so
/// that the `x` of `0x1f` and the `n` of `1n` are not read as names), and passes over an identifier
/// that directly follows a `.`, which is a property and so a different namespace.
fn for_each_ident<'a>(text: &'a str, mut f: impl FnMut(usize, &'a str)) {
    for_each_ident_kind(text, |at, ident, kind| {
        if kind == IdentKind::Code {
            f(at, ident);
        }
    });
}

/// Where an identifier the scanner found sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IdentKind {
    /// A name that denotes a binding: an item, a local, a parameter.
    Code,
    /// A member name, which is its object's namespace and not the program's: the `x` of `p.x`, and
    /// the key of an object literal entry.
    Property,
}

/// Calls `f` with the byte offset, the text and the [kind](IdentKind) of every identifier.
///
/// An object literal's key is reported as a property, which is what it is. It is recognised by the
/// shape the printer emits — a name directly after `{` or `,` and directly before `:` — and a
/// *label* (`outer: for (;;)`) can wear that shape too when it opens a block. Confusing the two
/// only ever costs a name its rename, never its meaning, so the ambiguity is resolved towards
/// calling it a property.
fn for_each_ident_kind<'a>(text: &'a str, mut f: impl FnMut(usize, &'a str, IdentKind)) {
    let bytes = text.as_bytes();
    let start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
    let part = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    // The last character that was not whitespace, which is what says whether a name before a `:`
    // opens an object entry.
    let mut previous = 0u8;

    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                previous = b'"';
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            c if start(c) => {
                let mut end = i;
                while end < bytes.len() && part(bytes[end]) {
                    end += 1;
                }
                // The next character that is not a space, which tells a key from a plain name.
                let mut after = end;
                while after < bytes.len() && (bytes[after] == b' ' || bytes[after] == b'\n') {
                    after += 1;
                }
                let is_read = i > 0 && bytes[i - 1] == b'.';
                let is_key = bytes.get(after) == Some(&b':')
                    && bytes.get(after + 1) != Some(&b':')
                    && matches!(previous, b'{' | b',');
                let kind =
                    if is_read || is_key { IdentKind::Property } else { IdentKind::Code };
                f(i, &text[i..end], kind);
                previous = bytes[end - 1];
                i = end;
            }
            // A numeric literal, taken whole: its tail can hold letters (`0x1f`, `1n`, `1e3`).
            c if c.is_ascii_digit() => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                    i += 1;
                }
                previous = bytes[i - 1];
            }
            b' ' | b'\n' | b'\t' | b'\r' => i += 1,
            other => {
                previous = other;
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| nth_name(LOCAL_FIRST, REST, i)).collect()
    }

    #[test]
    fn the_alphabet_is_little_endian_and_shortest_first() {
        let generated = names(60);
        assert_eq!(generated[0], "a");
        assert_eq!(generated[25], "z");
        assert_eq!(generated[26], "A");
        assert_eq!(generated[51], "Z");
        assert_eq!(generated[52], "_");
        // 53 single character names, then the two character ones, last character fastest.
        assert_eq!(generated[53], "aa");
        assert_eq!(generated[54], "ab");
        assert!(generated.iter().collect::<BTreeSet<_>>().len() == 60, "names repeat");
        assert!(generated.iter().all(|name| jsast::is_plain_ident(name)));
    }

    #[test]
    fn item_names_never_collide_with_local_ones() {
        let none = BTreeSet::new();
        let mut items = Names::items(&none);
        let mut locals = Names::locals(&none);
        let item_names: BTreeSet<String> = (0..200).map(|_| items.take()).collect();
        let local_names: BTreeSet<String> = (0..200).map(|_| locals.take()).collect();
        assert!(item_names.iter().all(|name| name.starts_with('$')));
        assert!(local_names.iter().all(|name| !name.starts_with('$')));
        assert!(item_names.is_disjoint(&local_names));
        assert!(local_names.iter().all(|name| !jsast::is_reserved(name)));
    }

    #[test]
    fn reserved_words_and_exclusions_are_skipped() {
        let excluded: BTreeSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let mut names = Names::locals(&excluded);
        assert_eq!(names.take(), "c");
        assert_eq!(names.take(), "d");
        // `do`, `if`, `in` are reserved and cannot be handed out.
        let none = BTreeSet::new();
        let mut all = Names::locals(&none);
        let generated: BTreeSet<String> = (0..4000).map(|_| all.take()).collect();
        for word in ["do", "if", "in", "let", "new", "var", "for", "try"] {
            assert!(!generated.contains(word), "handed out `{word}`");
        }
    }

    #[test]
    fn the_scanner_skips_strings_comments_and_properties() {
        let mut seen = Vec::new();
        for_each_ident(
            r#"// alpha
let beta = obj.gamma + "delta" + 0x1f + eps;
"#,
            |_, ident| seen.push(ident.to_owned()),
        );
        assert_eq!(seen, ["let", "beta", "obj", "eps"]);
    }

    #[test]
    fn the_scanner_reads_an_object_key_as_a_property() {
        let mut code = Vec::new();
        let mut properties = Vec::new();
        for_each_ident_kind("f({ exp: exp, mant: d.mant }, exp);", |_, ident, kind| {
            match kind {
                IdentKind::Code => code.push(ident.to_owned()),
                IdentKind::Property => properties.push(ident.to_owned()),
            }
        });
        // The two `exp`s in value position are names; the one before the `:` is a key.
        assert_eq!(code, ["f", "exp", "d", "exp"]);
        assert_eq!(properties, ["exp", "mant", "mant"]);
    }

    #[test]
    fn an_item_sharing_a_name_with_a_field_keeps_it() {
        // `compiler_builtins` exports `exp`, and `core`'s float formatter reads a `Decoded::exp`.
        // Renaming the item would rewrite the field with it, and every read would answer
        // `undefined` — so the item keeps its name and only the other one shortens.
        let item = |name: &str, js: &str| LinkItem {
            name: crate::item::JsName::new(name.to_owned()),
            kind: crate::item::ItemKind::Fn,
            js: js.to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: crate::item::Linkage::internal(),
            debug_path: String::new(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
        };
        let items = [
            item("exp", "function exp(x) { return x; }\n"),
            item("helper", "function helper(d) { return { exp: d.exp }; }\n"),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let program: String = items.iter().map(|item| item.js.clone()).collect();
        let out = apply_renames(&program, &item_renames(&program, &refs));
        assert!(out.contains("function exp(x)"), "`exp` was renamed:\n{out}");
        assert!(out.contains("{ exp: d.exp }"), "the field was rewritten:\n{out}");
        assert!(!out.contains("function helper"), "`helper` was not renamed:\n{out}");
    }

    /// A split program is several files printed out of one item set, so the map is computed over the
    /// concatenation and applied to each file. What must hold is that a name defined in one file and
    /// imported by another comes out spelled the same way in both, which is what the import clause
    /// being ordinary text buys.
    #[test]
    fn one_map_spells_a_shared_name_the_same_way_in_every_file() {
        let item = |name: &str, js: &str| LinkItem {
            name: crate::item::JsName::new(name.to_owned()),
            kind: crate::item::ItemKind::Fn,
            js: js.to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: crate::item::Linkage::internal(),
            debug_path: String::new(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
        };
        let items = [
            item("shared$h0", "function shared$h0(x) { return x + 1; }\n"),
            item("island_a$h0", "function island_a$h0() { return shared$h0(1); }\n"),
            item("island_b$h0", "function island_b$h0() { return shared$h0(2); }\n"),
            item("$chunk$shared", "import { shared$h0 } from \"./shared.js\";\n"),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let shared_file = items[0].js.clone();
        let island_file = format!("{}{}", items[3].js, items[1].js);
        let corpus = format!("{shared_file}{island_file}");

        let renames = item_renames(&corpus, &refs);
        let short = renames.get("shared$h0").expect("the shared item was renamed").clone();
        let shared_out = apply_renames(&shared_file, &renames);
        let island_out = apply_renames(&island_file, &renames);

        assert!(shared_out.contains(&format!("function {short}(")), "the definition:\n{shared_out}");
        assert!(
            island_out.contains(&format!("import {{ {short} }} from \"./shared.js\"")),
            "the import:\n{island_out}"
        );
        assert!(island_out.contains(&format!("return {short}(1)")), "the call:\n{island_out}");
    }

    /// An enum variant's name is a string literal in the emitted program (`value::EnumRepr`), and a
    /// variant may be called anything a Rust identifier may. Both renaming passes have to leave it
    /// alone, or a `switch` would compare a tag against a name the constructor no longer builds.
    #[test]
    fn a_variant_tag_string_is_never_renamed() {
        let item = |name: &str, js: &str| LinkItem {
            name: crate::item::JsName::new(name.to_owned()),
            kind: crate::item::ItemKind::Fn,
            js: js.to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: crate::item::Linkage::internal(),
            debug_path: String::new(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
        };
        // An item named exactly like a variant, so that a pass that did not skip strings would
        // rewrite the tag with it.
        let items = [
            item("Some", "function Some(x) { return x; }\n"),
            item(
                "read",
                "function read(o) { return o.TAG === \"Some\" ? o._0 : Some(0); }\n",
            ),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let program: String = items.iter().map(|item| item.js.clone()).collect();
        let out = apply_renames(&program, &item_renames(&program, &refs));
        assert!(out.contains(r#"o.TAG === "Some""#), "a tag was rewritten:\n{out}");

        // The local pass renames the AST, where a tag is a string and `TAG` an object key.
        let mut decl = jsast::function(
            "build$h0",
            vec!["payload".into()],
            vec![jsast::ret(jsast::object(vec![
                ("TAG".to_owned(), jsast::string("Some")),
                ("_0".to_owned(), jsast::id("payload")),
            ]))],
        );
        shorten_locals_in(&mut decl);
        let text = jsast::stmt_to_string(&decl);
        assert!(text.contains(r#"TAG: "Some""#), "the tag moved:\n{text}");
        assert!(!text.contains("payload"), "the parameter was not renamed:\n{text}");
    }

    #[test]
    fn the_scanner_survives_escapes_and_slashes() {
        let mut seen = Vec::new();
        for_each_ident(r#"a = "x\"b" / c; d = "e"; // f"#, |_, ident| seen.push(ident.to_owned()));
        assert_eq!(seen, ["a", "c", "d"]);
    }

    #[test]
    fn locals_are_renamed_and_free_names_are_not() {
        // function f$h0(acc, n) { let t = acc; return __rt.add(t, n); }
        let mut decl = jsast::function(
            "f$h0",
            vec!["acc".into(), "n".into()],
            vec![
                jsast::let_("t", jsast::id("acc")),
                jsast::ret(jsast::rt_call("add", vec![jsast::id("t"), jsast::id("n")])),
            ],
        );
        shorten_locals_in(&mut decl);
        assert_eq!(
            jsast::stmt_to_string(&decl),
            "function f$h0(a, b) {\n  let c = a;\n  return __rt.add(c, b);\n}\n"
        );
    }

    #[test]
    fn a_local_never_takes_a_name_the_body_already_mentions() {
        // `a` is a free identifier here (an item this function calls), so no local may become `a`.
        let mut decl = jsast::function(
            "f$h0",
            vec!["x".into()],
            vec![jsast::ret(jsast::call(jsast::id("a"), vec![jsast::id("x")]))],
        );
        shorten_locals_in(&mut decl);
        assert_eq!(jsast::stmt_to_string(&decl), "function f$h0(b) {\n  return a(b);\n}\n");
    }

    #[test]
    fn labels_get_their_own_namespace() {
        let mut decl = jsast::function(
            "f$h0",
            vec!["x".into()],
            vec![jsast::loop_forever(
                "bb0",
                vec![jsast::Stmt::Break(Some("bb0".into()))],
            )],
        );
        shorten_locals_in(&mut decl);
        // The parameter took `a`; the label starts the alphabet again and takes `a` too.
        assert_eq!(
            jsast::stmt_to_string(&decl),
            "function f$h0(a) {\n  a: for (;;) {\n    break a;\n  }\n}\n"
        );
    }

    #[test]
    fn a_raw_statement_stops_the_renaming_of_its_item() {
        let mut decl = jsast::function(
            "f$h0",
            vec!["x".into()],
            vec![jsast::raw_stmt("return x;")],
        );
        let before = jsast::stmt_to_string(&decl);
        shorten_locals_in(&mut decl);
        assert_eq!(jsast::stmt_to_string(&decl), before);
    }

    #[test]
    fn the_hottest_name_gets_the_shortest() {
        // Three locals, used 1, 5 and 2 times. Parameters come first regardless.
        let body = vec![
            jsast::let_("cold", jsast::num(0)),
            jsast::let_("hot", jsast::num(0)),
            jsast::let_("warm", jsast::num(0)),
            jsast::expr_stmt(jsast::assign(
                jsast::id("hot"),
                jsast::binary(
                    jsast::BinOp::Add,
                    jsast::binary(jsast::BinOp::Add, jsast::id("hot"), jsast::id("hot")),
                    jsast::binary(jsast::BinOp::Add, jsast::id("hot"), jsast::id("warm")),
                ),
            )),
        ];
        let mut decl = jsast::function("f$h0", vec!["p".into()], body);
        shorten_locals_in(&mut decl);
        let text = jsast::stmt_to_string(&decl);
        assert!(text.starts_with("function f$h0(a) {"), "{text}");
        assert!(text.contains("let d = 0;\n  let b = 0;\n  let c = 0;"), "{text}");
    }
}
