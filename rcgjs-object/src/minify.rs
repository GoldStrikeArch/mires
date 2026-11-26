//! Minification: shorter names, in two passes that run in two different places.
//!
//! `-Cllvm-args=js-minify=on` turns on three things at once. Two of them live here:
//!
//! * [`shorten_locals`] renames a function's **locals** — parameters, `let`s, the `$`-temporaries,
//!   the structurizer's labels — to `a`, `b`, `c`, ... It runs at codegen time, on the JavaScript
//!   AST, once per item.
//! * [`item_renames`] renames the program's **items** — every top level function, static and vtable
//!   whose name is not fixed — to `$a`, `$b`, `$c`, ... It runs at link time, on the finished
//!   program text, once.
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
//! item name always starts with `$` and a codegen-minified local name never does.** Link-time
//! lowerers can also add `$`-prefixed compiler locals after the local pass; the item allocator
//! reserves every such identifier already present in the final corpus. A local can therefore never
//! shadow an item, whatever order the two passes run in and whichever crate each name came from.
//! It is also why the item pass may be textual without being a guess — see below.
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
//!   `GLOBALS` and `IMPORT_PREFIX` additionally refuse them in functions that do *not*, which is
//!   belt and braces at the price of a lookup.
//! * **`refs`.** A `JsItem`'s reference set is left exactly as codegen computed it. It is an
//!   over-approximation intersected with the item table, and local renaming cannot change which
//!   *item* names an item mentions; recomputing it would only swap one set of junk entries (`acc`,
//!   `_3`) for another (`a`, `b`), and the junk is what the intersection exists to discard.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{ItemMap, JsItem, LinkItem};
use crate::jsast::{self, Expr, Stmt};

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
    "CompressionStream",
    "DataView",
    "DecompressionStream",
    "Error",
    "Infinity",
    "Math",
    "NaN",
    "Number",
    "Object",
    "String",
    "Symbol",
    "Uint8Array",
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
        Names {
            first: LOCAL_FIRST,
            rest: REST,
            prefix: "",
            next: 0,
            excluded,
        }
    }

    /// Names for items: `$a`, `$b`, ..., which no local can ever be.
    fn items(excluded: &'a BTreeSet<String>) -> Names<'a> {
        Names {
            first: REST,
            rest: REST,
            prefix: "$",
            next: 0,
            excluded,
        }
    }

    fn take(&mut self) -> String {
        loop {
            let name = format!(
                "{}{}",
                self.prefix,
                nth_name(self.first, self.rest, self.next)
            );
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
pub fn shorten_locals(items: &mut [JsItem]) {
    if !crate::opts::get().minify.is_enabled() {
        return;
    }
    for item in items {
        let renames = shorten_locals_in(&mut item.decl);
        rename_source_names(&mut item.names, &renames);
    }
}

fn rename_source_names(names: &mut [(String, String)], renames: &BTreeMap<String, String>) {
    for (emitted, _) in names {
        if let Some(short) = renames.get(emitted) {
            *emitted = short.clone();
        }
    }
}

/// Renames the locals of one declaration whatever the build's options say.
///
/// [`shorten_locals`] is a no-op without `js-minify`, so a caller that wants to know what ONE item
/// would cost minified cannot use it: it would measure the lowering's own local names. This is the
/// same renaming with the option check left out, for a measurement made on a clone. It is never
/// what a build prints — a program whose locals were shortened here and nowhere else would be
/// half-minified.
pub fn shorten_locals_for_measurement(decl: &mut Stmt) {
    let _ = shorten_locals_in(decl);
}

/// Renames the locals of one declaration.
///
/// The declaration's own name is the item's, which only the link step may touch, so it goes into
/// the exclusion set rather than into the pool. Everything a function body declares — parameters,
/// `let`s, `const`s, the parameters of the arrows in it — is one flat pool, which is both simpler
/// than js_of_ocaml's per-scope bitsets and correct: renaming by name is a bijection over the
/// function's identifiers, so whatever shadowing the input had, the output has exactly the same.
fn shorten_locals_in(decl: &mut Stmt) -> BTreeMap<String, String> {
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
        _ => return BTreeMap::new(),
    };

    // Verbatim JavaScript is opaque: it could mention any of these names and the walk would never
    // know. A `js!{}` block is one, so an item holding a block keeps its long local names rather
    // than being miscompiled.
    if collect.has_raw {
        return BTreeMap::new();
    }

    let mut excluded: BTreeSet<String> = GLOBALS.iter().map(|name| (*name).to_string()).collect();
    excluded.extend(own);
    excluded.extend(collect.catch_bindings.iter().cloned());
    for name in collect.tally.keys() {
        if !collect.declared.contains(name) {
            excluded.insert(name.clone());
        }
    }

    let mut vars = assign(
        &collect.params,
        &collect.declared,
        &collect.tally,
        &excluded,
    );
    for binding in &collect.catch_bindings {
        vars.remove(binding);
    }
    // Labels are a namespace of their own in JavaScript: `a: for (;;) break a;` is legal beside a
    // variable `a`, so the labels start the alphabet again.
    let no_exclusions = BTreeSet::new();
    let labels = assign(&[], &collect.labels, &collect.label_tally, &no_exclusions);

    let mut apply = Apply {
        vars: &vars,
        labels: &labels,
    };
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
    vars
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
    fn catch_binding(&mut self, _name: &mut String) {}
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
    catch_bindings: BTreeSet<String>,
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
    fn catch_binding(&mut self, name: &mut String) {
        // The flat renamer cannot distinguish a catch binding from a free identifier outside
        // that catch. Reserve its spelling throughout the function, including nested catches.
        self.catch_bindings.insert(name.clone());
        self.used(name);
    }

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

struct NameWalker<'a, V>(&'a mut V);

impl<V: Visit> jsast::VisitMut for NameWalker<'_, V> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Let(name, init) => {
                self.0.declared(name);
                if let Some(init) = init {
                    self.visit_expr_mut(init);
                }
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    self.0.declared(name);
                    if let Some(init) = init {
                        self.visit_expr_mut(init);
                    }
                }
            }
            Stmt::Const(name, init) => {
                self.0.declared(name);
                self.visit_expr_mut(init);
            }
            Stmt::Labeled(label, inner) => {
                self.0.label_declared(label);
                self.visit_stmt_mut(inner);
            }
            Stmt::LoopForever(label, body) => {
                if let Some(label) = label {
                    self.0.label_declared(label);
                }
                self.visit_stmts_mut(body);
            }
            Stmt::Continue(label) | Stmt::Break(label) => {
                if let Some(label) = label {
                    self.0.label_used(label);
                }
            }
            // Never emitted inside a body today; treated as a nested binding rather than assumed away.
            Stmt::FunctionDecl { name, params, body } => {
                self.0.declared(name);
                for param in params {
                    self.0.param(param);
                }
                self.visit_stmts_mut(body);
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                self.visit_stmts_mut(body);
                self.0.catch_binding(binding);
                self.visit_stmts_mut(handler);
            }
            Stmt::Raw(_) => self.0.raw(),
            // An import declares names the whole program shares, and `link.rs` keeps them exactly as
            // they are spelled: renaming one here would rewrite the binding inside the `import` and
            // leave every use of it looking for something the module does not export.
            Stmt::Import { .. } => {}
            _ => jsast::walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Ident(name) => self.0.used(name),
            Expr::Arrow(params, _) => {
                for param in params {
                    self.0.param(param);
                }
                jsast::walk_expr_mut(self, expr);
            }
            Expr::Raw(_) => self.0.raw(),
            _ => jsast::walk_expr_mut(self, expr),
        }
    }
}

fn walk_stmts<V: Visit>(stmts: &mut [Stmt], v: &mut V) {
    jsast::VisitMut::visit_stmts_mut(&mut NameWalker(v), stmts);
}

fn walk_expr<V: Visit>(expr: &mut Expr, v: &mut V) {
    jsast::VisitMut::visit_expr_mut(&mut NameWalker(v), expr);
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
pub fn item_renames(corpus: &str, items: &[&LinkItem]) -> BTreeMap<String, String> {
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

    // Link-time lowerers run after [`shorten_locals`] and use `$`-prefixed compiler locals. Keep
    // those exact names out of the item allocator's `$` namespace so a shortened callee cannot be
    // shadowed by one of them. Names of actual items remain available for renaming.
    for_each_ident(corpus, |_, ident| {
        if ident.starts_with('$') && !renamable.contains(ident) {
            kept.insert(ident.to_owned());
        }
    });

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for_each_ident(corpus, |_, ident| {
        if renamable.contains(ident) {
            *counts.entry(ident).or_default() += 1;
        }
    });

    let mut order: Vec<(&str, usize)> = counts.into_iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let mut names = Names::items(&kept);
    order
        .into_iter()
        .map(|(name, _)| (name.to_owned(), names.take()))
        .collect()
}

/// Rewrites every identifier [`item_renames`] gave a short name.
///
/// An empty map leaves the text alone, byte for byte, which is what a program with nothing to
/// rename gets.
pub fn apply_renames(program: &str, renames: &BTreeMap<String, String>) -> String {
    rewrite(program, renames).0
}

/// Rewrites item identifiers and rebases the item's generated source-map columns with them.
///
/// Item minification runs after an object has already recorded its mappings. Every replacement is
/// an ASCII identifier and cannot add or remove a line, so rebasing is local to the replacement's
/// generated line. Name-table entries already carry Rust names and do not change.
pub fn apply_renames_mapped(
    program: &str,
    map: &ItemMap,
    renames: &BTreeMap<String, String>,
) -> (String, ItemMap) {
    let (program, replacements) = rewrite(program, renames);
    if replacements.is_empty() || map.segments.is_empty() {
        return (program, map.clone());
    }
    let mut map = map.clone();
    for (line, col, ..) in &mut map.segments {
        *col = renamed_column(*line, *col, &replacements);
    }
    (program, map)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Replacement {
    line: u32,
    start: u32,
    end: u32,
    replacement_len: u32,
}

fn rewrite(program: &str, renames: &BTreeMap<String, String>) -> (String, Vec<Replacement>) {
    if renames.is_empty() {
        return (program.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(program.len());
    let mut output_cursor = 0;
    let mut scan_cursor = 0;
    let mut line = 0;
    let mut col = 0;
    let mut replacements = Vec::new();
    for_each_ident(program, |at, ident| {
        advance_position(&program[scan_cursor..at], &mut line, &mut col);
        let start = col;
        advance_position(ident, &mut line, &mut col);
        scan_cursor = at + ident.len();
        if let Some(short) = renames.get(ident) {
            out.push_str(&program[output_cursor..at]);
            out.push_str(short);
            output_cursor = at + ident.len();
            replacements.push(Replacement {
                line,
                start,
                end: col,
                replacement_len: short.encode_utf16().count() as u32,
            });
        }
    });
    out.push_str(&program[output_cursor..]);
    (out, replacements)
}

fn advance_position(text: &str, line: &mut u32, col: &mut u32) {
    for character in text.chars() {
        if character == '\n' {
            *line += 1;
            *col = 0;
        } else {
            *col += character.len_utf16() as u32;
        }
    }
}

fn renamed_column(line: u32, original: u32, replacements: &[Replacement]) -> u32 {
    let mut delta = 0i64;
    for replacement in replacements
        .iter()
        .filter(|replacement| replacement.line == line)
    {
        let start = i64::from(replacement.start);
        let end = i64::from(replacement.end);
        let replacement_len = i64::from(replacement.replacement_len);
        let original = i64::from(original);
        if original < start {
            break;
        }
        if original >= end {
            delta += replacement_len - (end - start);
            continue;
        }
        let within = (original - start).min(replacement_len);
        return u32::try_from(start + delta + within)
            .expect("a generated column stays nonnegative");
    }
    u32::try_from(i64::from(original) + delta).expect("a generated column stays nonnegative")
}

/// Calls `f` with the byte offset and text of every identifier in *code* position.
///
/// Not a JavaScript lexer: a scanner for the subset this backend's printer emits, which has no
/// regular expression literals, no template literals, no single quoted strings and no block
/// comments. It skips double quoted strings and `//` comments, consumes numeric literals whole (so
/// that the `x` of `0x1f` and the `n` of `1n` are not read as names), and passes over an identifier
/// that directly follows a `.`, which is a property and so a different namespace.
pub fn for_each_ident<'a>(text: &'a str, mut f: impl FnMut(usize, &'a str)) {
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
                let kind = if is_read || is_key {
                    IdentKind::Property
                } else {
                    IdentKind::Code
                };
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
        assert!(
            generated.iter().collect::<BTreeSet<_>>().len() == 60,
            "names repeat"
        );
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
    fn item_names_skip_link_generated_dollar_locals() {
        let item = |name: &str, js: &str| LinkItem {
            name: crate::item::JsName::new(name.to_owned()),
            kind: crate::item::ItemKind::Fn,
            js: js.to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: crate::item::Linkage::internal(),
            debug_path: String::new(),
            crate_name: "test".to_owned(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        let items = [
            item("hot$h0", "function hot$h0() { return 1; }\n"),
            item("callee$h0", "function callee$h0(x) { return x; }\n"),
            item(
                "caller$h0",
                "function caller$h0(x) { let $b = 0; $l: for (;;) { return callee$h0(x) + hot$h0() + hot$h0(); } }\n",
            ),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let program: String = items.iter().map(|item| item.js.clone()).collect();
        let renames = item_renames(&program, &refs);

        assert_eq!(renames["hot$h0"], "$a");
        assert_eq!(renames["callee$h0"], "$c");
        let out = apply_renames(&program, &renames);
        assert!(out.contains("let $b = 0;"), "the local moved:\n{out}");
        assert!(
            out.contains("return $c(x) + $a() + $a();"),
            "the call was shadowed:\n{out}"
        );
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
        for_each_ident_kind(
            "f({ exp: exp, mant: d.mant }, exp);",
            |_, ident, kind| match kind {
                IdentKind::Code => code.push(ident.to_owned()),
                IdentKind::Property => properties.push(ident.to_owned()),
            },
        );
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
            crate_name: "test".to_owned(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        let items = [
            item("exp", "function exp(x) { return x; }\n"),
            item("helper", "function helper(d) { return { exp: d.exp }; }\n"),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let program: String = items.iter().map(|item| item.js.clone()).collect();
        let out = apply_renames(&program, &item_renames(&program, &refs));
        assert!(out.contains("function exp(x)"), "`exp` was renamed:\n{out}");
        assert!(
            out.contains("{ exp: d.exp }"),
            "the field was rewritten:\n{out}"
        );
        assert!(
            !out.contains("function helper"),
            "`helper` was not renamed:\n{out}"
        );
    }

    /// A split program is several files printed out of one item set, so the map is computed over
    /// the concatenation and applied to each file. What must hold is that a name defined in one
    /// file and imported by another comes out spelled the same way in both, which is what the
    /// import clause being ordinary text buys.
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
            crate_name: "test".to_owned(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        let items = [
            item("shared$h0", "function shared$h0(x) { return x + 1; }\n"),
            item(
                "island_a$h0",
                "function island_a$h0() { return shared$h0(1); }\n",
            ),
            item(
                "island_b$h0",
                "function island_b$h0() { return shared$h0(2); }\n",
            ),
            item(
                "$chunk$shared",
                "import { shared$h0 } from \"./shared.js\";\n",
            ),
        ];
        let refs: Vec<&LinkItem> = items.iter().collect();
        let shared_file = items[0].js.clone();
        let island_file = format!("{}{}", items[3].js, items[1].js);
        let corpus = format!("{shared_file}{island_file}");

        let renames = item_renames(&corpus, &refs);
        let short = renames
            .get("shared$h0")
            .expect("the shared item was renamed")
            .clone();
        let shared_out = apply_renames(&shared_file, &renames);
        let island_out = apply_renames(&island_file, &renames);

        assert!(
            shared_out.contains(&format!("function {short}(")),
            "the definition:\n{shared_out}"
        );
        assert!(
            island_out.contains(&format!("import {{ {short} }} from \"./shared.js\"")),
            "the import:\n{island_out}"
        );
        assert!(
            island_out.contains(&format!("return {short}(1)")),
            "the call:\n{island_out}"
        );
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
            crate_name: "test".to_owned(),
            order: None,
            map: crate::item::ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
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
        assert!(
            out.contains(r#"o.TAG === "Some""#),
            "a tag was rewritten:\n{out}"
        );

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
        assert!(
            !text.contains("payload"),
            "the parameter was not renamed:\n{text}"
        );
    }

    #[test]
    fn the_scanner_survives_escapes_and_slashes() {
        let mut seen = Vec::new();
        for_each_ident(r#"a = "x\"b" / c; d = "e"; // f"#, |_, ident| {
            seen.push(ident.to_owned())
        });
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
    fn item_renaming_rebases_source_map_columns() {
        let program = "function long$h0(){let x=\"😀\";return long$h0()}\n";
        let second = program.rfind("long$h0").unwrap();
        let second_start = program[..second].encode_utf16().count() as u32;
        let second_end = second_start + "long$h0".encode_utf16().count() as u32;
        let map = ItemMap {
            sources: vec!["source.rs".to_owned()],
            names: vec!["crate::long".to_owned()],
            segments: vec![(0, second_start, 0, 1, 0, 0), (0, second_end, 0, 1, 0, -1)],
        };
        let renames = BTreeMap::from([("long$h0".to_owned(), "$a".to_owned())]);

        let (renamed, map) = apply_renames_mapped(program, &map, &renames);
        let renamed_second = renamed.rfind("$a").unwrap();
        let expected_start = renamed[..renamed_second].encode_utf16().count() as u32;

        assert_eq!(renamed, "function $a(){let x=\"😀\";return $a()}\n");
        assert_eq!(map.segments[0].1, expected_start);
        assert_eq!(map.segments[1].1, expected_start + 2);
        assert_eq!(map.names, ["crate::long"]);
    }

    #[test]
    fn local_renaming_moves_source_map_name_keys() {
        let mut decl = jsast::function(
            "f$h0",
            vec!["payload".into()],
            vec![jsast::ret(jsast::id("payload"))],
        );
        let renames = shorten_locals_in(&mut decl);
        let mut names = vec![("payload".to_owned(), "payload".to_owned())];

        rename_source_names(&mut names, &renames);

        assert_eq!(names, [("a".to_owned(), "payload".to_owned())]);
    }

    #[test]
    fn a_local_never_takes_a_name_the_body_already_mentions() {
        // `a` is a free identifier here (an item this function calls), so no local may become `a`.
        let mut decl = jsast::function(
            "f$h0",
            vec!["x".into()],
            vec![jsast::ret(jsast::call(
                jsast::id("a"),
                vec![jsast::id("x")],
            ))],
        );
        shorten_locals_in(&mut decl);
        assert_eq!(
            jsast::stmt_to_string(&decl),
            "function f$h0(b) {\n  return a(b);\n}\n"
        );
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
        let mut decl =
            jsast::function("f$h0", vec!["x".into()], vec![jsast::raw_stmt("return x;")]);
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
        assert!(
            text.contains("let d = 0;\n  let b = 0;\n  let c = 0;"),
            "{text}"
        );
    }
}
