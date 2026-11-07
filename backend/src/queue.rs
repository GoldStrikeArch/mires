//! The expression queue: assignments that move to the place that reads them.
//!
//! A MIR body is three-address code, so the lowering emits one statement per operation and a
//! local to carry each intermediate result:
//!
//! ```js
//! _2 = _1.w;
//! _3 = _1.h;
//! _0 = Math.imul(_2, _3) | 0;
//! return _0;
//! ```
//!
//! Most of those locals are written once and read once. This pass walks the region tree between
//! the structurizer and `emit.rs` and, instead of emitting such an assignment, *queues* it: the
//! expression is held back and substituted at the one place that reads it. What comes out is the
//! expression the source wrote:
//!
//! ```js
//! return Math.imul(_1.w, _1.h) | 0;
//! ```
//!
//! The model is `js_of_ocaml`'s (`compiler/lib/generate.ml`, module `Q`). Two things make it
//! simpler here: MIR tells us exactly which locals are single-assignment (`uses.rs`), where
//! js_of_ocaml has to infer it, and the region tree hands us straight-line runs already delimited.
//!
//! # What may move, and how far
//!
//! Substituting an assignment into a later expression moves its evaluation *later*; flushing a
//! held-back assignment emits it just before the statement that forced the flush, which also only
//! moves it later. Nothing in this pass ever moves an expression earlier, so the whole
//! correctness argument is about what an expression may be moved past. That is the purity ladder:
//!
//! * **Const** — literals, arithmetic over them, and reads of locals `uses.rs` calls immutable.
//!   Depends on nothing that can change and changes nothing, so it may move anywhere.
//! * **Read** — a field or element read, an accessor's `get()`, a clone. May not move past a
//!   write.
//! * **Effect** — a call, an assignment, anything opaque. May not move past a read or another
//!   effect.
//!
//! Two expressions commute when one of them is `Const`, or when both are `Read`. Everything the
//! pass does is one of those two moves, guarded by that test:
//!
//! * before emitting a statement, every queued entry that does not commute with it is flushed, in
//!   the order the entries were queued;
//! * an entry is substituted into an expression only if it commutes with everything already
//!   evaluated in that expression — substituting into the second operand of `a.x + _5` moves
//!   `a.x` in front of whatever `_5` was;
//! * an entry whose name still appears in the statement after substitution is flushed, which is
//!   what makes a declined substitution safe rather than a dangling read;
//! * an entry is flushed if the statement assigns to a variable the entry mentions;
//! * the queue is flushed completely at every region boundary, so an entry never crosses a branch,
//!   a loop header or a jump.
//!
//! A queued entry is substituted only when its name occurs *exactly once* in the statement. One
//! MIR use can lower to several mentions of the local (`&mut xs[i]` names its base twice), and
//! duplicating an expression that has effects — or that is merely expensive — is not a rewrite
//! this pass is allowed to make.
//!
//! # Destination passing
//!
//! `_0` is neither inlinable (a `return` reads it) nor droppable, so the last assignment before a
//! `Region::Return` is handled here directly: `_0 = e; return _0;` becomes `return e;`. A
//! function whose return type is zero sized returns nothing at all — `_0` is dead in the sense
//! `uses.rs` means it, its assignments lose their target, and the `return` loses its value.
//!
//! `Region::Return` cannot carry an expression, so a return is lowered here into a
//! `Region::Block` holding the finished statement; `emit.rs` passes block statements through
//! untouched. Every return in the tree is rewritten this way, so `emit.rs`'s own `Region::Return`
//! arm only ever runs with the queue turned off.
//!
//! # Declarations
//!
//! Nothing here declares anything. `base.rs` builds the prelude *after* this pass has run, from
//! the names the finished body still mentions, so an inlined temporary and an eliminated `_0`
//! simply never appear in it.

use std::collections::{HashMap, HashSet};

use rustc_middle::mir::Local;
use structurizer::{Region, Scrut};

use crate::base::FnCx;
use crate::cfg_adapter::Scrutinee;
use crate::jsast::{self, ArrowBody, BinOp, Expr, Stmt};
use crate::uses::Uses;
use crate::value;

/// How freely an expression may be moved. See the module documentation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Purity {
    /// Depends on nothing that can change, changes nothing.
    Const,
    /// Reads memory that something else could write.
    Read,
    /// Writes, calls, or anything the pass cannot see into.
    Effect,
}

/// Whether evaluating `a` and `b` in either order gives the same result.
fn commutes(a: Purity, b: Purity) -> bool {
    a == Purity::Const || b == Purity::Const || (a == Purity::Read && b == Purity::Read)
}

/// The globals the runtime provides, whose members are constants rather than mutable state.
///
/// A Rust item can never be named one of these: every name the backend invents for an item
/// carries a `$`-separated path and a hash suffix (`naming.rs`).
const PURE_GLOBALS: &[&str] = &["Math", "BigInt", "Number", "Object", "Array", "String", "__rt"];

/// An assignment held back from the output, waiting for the expression that reads it.
struct Entry {
    name: String,
    value: Expr,
    purity: Purity,
}

/// Runs the pass over one function's region tree.
pub(crate) fn run<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    uses: &Uses,
    region: Region<Vec<Stmt>, Scrutinee<'tcx>>,
) -> Region<Vec<Stmt>, Scrutinee<'tcx>> {
    let mut reads = HashMap::new();
    count_reads(&region, &mut reads);
    Queuer::new(fx, uses, reads).region(region, true)
}

/// How often the finished JavaScript reads each name, before anything is moved.
///
/// MIR's use count answers a question about MIR, and one MIR use does not always lower to one
/// mention of the local: `&mut xs[i]` names its base twice, an intrinsic may use an argument
/// twice. Counting the emitted tree closes that gap, and closes it for every lowering rather than
/// for the ones anybody thought of — an expression is only moved to "the place that reads it" when
/// the tree really does read it in exactly one place.
fn count_reads<'tcx>(region: &Region<Vec<Stmt>, Scrutinee<'tcx>>, reads: &mut HashMap<String, usize>) {
    let mut bump = |name: &str| {
        *reads.entry(name.to_string()).or_insert(0) += 1;
    };
    match region {
        Region::Seq(children) => {
            for child in children {
                count_reads(child, reads);
            }
        }
        Region::Block(stmts) => {
            for stmt in stmts {
                each_ident_stmt(stmt, &mut bump);
            }
        }
        Region::LetTmp { value, .. } => each_ident(&value.value, &mut bump),
        Region::If { scrut, then_, else_, .. } => {
            count_scrut(scrut, reads);
            count_reads(then_, reads);
            count_reads(else_, reads);
        }
        Region::Switch { scrut, arms, default, .. } => {
            count_scrut(scrut, reads);
            for (_, body) in arms {
                count_reads(body, reads);
            }
            count_reads(default, reads);
        }
        Region::Labeled { body, .. } | Region::Loop { body, .. } => count_reads(body, reads),
        Region::Dispatch { cases, .. } => {
            for (_, body) in cases {
                count_reads(body, reads);
            }
        }
        Region::SetSel(..)
        | Region::Break(_)
        | Region::Continue(_)
        | Region::Return
        | Region::Unreachable
        | Region::Throw => {}
    }
}

fn count_scrut<'tcx>(scrut: &Scrut<Scrutinee<'tcx>>, reads: &mut HashMap<String, usize>) {
    if let Scrut::Payload(scrutinee) = scrut {
        each_ident(&scrutinee.value, &mut |name| {
            *reads.entry(name.to_string()).or_insert(0) += 1;
        });
    }
}

struct Queuer<'a> {
    uses: &'a Uses,
    /// The local each emitted name stands for, for the names the pass can act on.
    locals: HashMap<String, Local>,
    /// Names whose value cannot change: reading one is `Purity::Const`.
    immutable: HashSet<String>,
    /// The name of the return place.
    ret: String,
    /// How the return place is read: the bare name, or `_0[0]` where it is boxed (`base.rs`).
    ret_read: Expr,
    /// Whether the return place is boxed, which turns destination passing off for it: the box may
    /// still be named by a slot the body handed out, so the store into it has to stay.
    ret_boxed: bool,
    /// Whether the function returns a zero sized value, and so returns nothing.
    unit_return: bool,
    /// How often the emitted tree reads each name. See [`count_reads`].
    reads: HashMap<String, usize>,
    /// Whether `js-scoped-lets` asked for declarations at the assignment rather than the prelude.
    scoped: bool,
}

impl<'a> Queuer<'a> {
    fn new<'tcx>(
        fx: &FnCx<'_, 'tcx>,
        uses: &'a Uses,
        reads: HashMap<String, usize>,
    ) -> Queuer<'a> {
        let mut locals = HashMap::new();
        let mut immutable = HashSet::new();
        for local in fx.mir.local_decls.indices() {
            let name = fx.local_name(local);
            if uses.immutable(local) {
                immutable.insert(name.clone());
            }
            locals.insert(name, local);
        }
        let return_ty = fx.monomorphize(fx.mir.return_ty());
        Queuer {
            uses,
            locals,
            immutable,
            ret: fx.local_name(rustc_middle::mir::RETURN_PLACE),
            ret_read: fx.local_expr(rustc_middle::mir::RETURN_PLACE),
            ret_boxed: fx.is_boxed(rustc_middle::mir::RETURN_PLACE),
            unit_return: value::is_zst(fx.tcx, return_ty),
            reads,
            scoped: crate::opts::get().scoped_lets,
        }
    }

    /// How often the emitted tree reads `name`.
    fn reads(&self, name: &str) -> usize {
        self.reads.get(name).copied().unwrap_or(0)
    }

    /// The local an emitted name stands for, or `None` for a compiler temporary or an item.
    fn local(&self, name: &str) -> Option<Local> {
        self.locals.get(name).copied()
    }
}

// ---------------------------------------------------------------------------
// Purity
// ---------------------------------------------------------------------------

impl Queuer<'_> {
    /// How freely `expr` may be moved.
    ///
    /// The classification is structural, over the JavaScript the lowering produced rather than
    /// over MIR, because that is what gets moved. It errs towards [`Purity::Effect`]: an
    /// expression the pass does not recognize is one it will not reorder.
    fn purity(&self, expr: &Expr) -> Purity {
        match expr {
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null => Purity::Const,
            // A function body is not evaluated where it is written. Whether it *closes over*
            // something that changes does not matter either: it closes over the variable, so
            // building the closure later still reads the same slot when it eventually runs.
            Expr::Arrow(..) => Purity::Const,
            Expr::Ident(name) => self.ident_purity(name),
            // `Math.imul` is a constant; `p.x` is a read of memory something else may write.
            Expr::Member(object, _) => match pure_global(object) {
                true => Purity::Const,
                false => self.purity(object).max(Purity::Read),
            },
            Expr::Index(object, index) => {
                self.purity(object).max(self.purity(index)).max(Purity::Read)
            }
            Expr::Call(callee, args) => {
                let mut purity = self.callee_purity(callee);
                for arg in args {
                    purity = purity.max(self.purity(arg));
                }
                purity
            }
            // `new Array(n)` is a fresh array and nothing else; anything else built with `new` is
            // an `Error` on its way to a `throw`.
            Expr::New(callee, args) => {
                let mut purity = match &**callee {
                    Expr::Ident(name) if name == "Array" => Purity::Read,
                    _ => Purity::Effect,
                };
                for arg in args {
                    purity = purity.max(self.purity(arg));
                }
                purity
            }
            Expr::Unary(_, operand) => self.purity(operand),
            Expr::Binary(_, lhs, rhs) => self.purity(lhs).max(self.purity(rhs)),
            Expr::Assign(..) | Expr::CompoundAssign(..) => Purity::Effect,
            Expr::Cond(test, consequent, alternate) => {
                self.purity(test).max(self.purity(consequent)).max(self.purity(alternate))
            }
            Expr::Object(entries) => {
                entries.iter().fold(Purity::Const, |acc, (_, value)| acc.max(self.purity(value)))
            }
            Expr::Array(items) | Expr::Seq(items) => {
                items.iter().fold(Purity::Const, |acc, item| acc.max(self.purity(item)))
            }
            // Verbatim text the pass cannot read.
            Expr::Raw(_) => Purity::Effect,
        }
    }

    /// Reading a name: constant when the value behind it cannot change.
    ///
    /// A hoisted constant is recognised by its spelling (`naming.rs`): it is a module level
    /// `const` nothing ever assigns to, so reading it is as movable as the literal it replaced —
    /// without this, hoisting would make the queue *less* able to fold a caller location into the
    /// call that takes it than it was when the location was written out inline.
    fn ident_purity(&self, name: &str) -> Purity {
        if self.immutable.contains(name)
            || PURE_GLOBALS.contains(&name)
            || crate::naming::is_hoisted_const(name)
        {
            Purity::Const
        } else {
            // A `static`, a compiler temporary, or a local assigned more than once.
            Purity::Read
        }
    }

    /// What calling `callee` does, ignoring its arguments.
    ///
    /// Only the handful of shapes the backend emits are recognized. A call to a compiled Rust
    /// function is opaque and so is an effect, which is what stops the queue from carrying a
    /// memory read across one.
    fn callee_purity(&self, callee: &Expr) -> Purity {
        match callee {
            Expr::Member(object, method) => match (&**object, method.as_str()) {
                (Expr::Ident(name), _) if name == "Math" => Purity::Const,
                (Expr::Ident(name), _) if name == "BigInt" || name == "Number" => Purity::Const,
                // `Object.assign({}, v)` is how an aggregate is cloned.
                (Expr::Ident(name), "assign") if name == "Object" => Purity::Read,
                // The saturating float casts; every other shim member has effects.
                (Expr::Ident(name), "f2i" | "f2i_big") if name == "__rt" => Purity::Read,
                // The `str` conversions. Each is a function of its arguments and writes nothing a
                // program can read: `str_bytes` fills a memo, but the identity that memo restores
                // is documented as not guaranteed, so nothing may depend on where the call lands.
                // `Read` rather than `Const` because the byte buffer it hands back is a place
                // another statement may write through.
                (Expr::Ident(name), "str_bytes" | "str_len" | "bytes_str")
                    if name == "__rt" =>
                {
                    Purity::Read
                }
                // The pointer helpers that only *read*. Each is a function of the places its
                // arguments name, so moving one changes nothing that another statement between
                // here and there could observe:
                //
                // * `box` and `addr` memoize in a `WeakMap`, which is a write no program can see —
                //   the answer for one buffer is the same wherever the call lands;
                // * `ptr_eq`, `ptr_cmp` and `compare_bytes` read memory and report on it, and
                //   `compare_bytes` is a read of two runs of elements even though its siblings in
                //   the same family write;
                // * `offset`, `unscale`, `unwindow` and `thin` build a new record out of an old one;
                // * `read_array` copies elements out, and `array_ref` names them without copying,
                //   memoizing the view it hands back the way `box` memoizes a slot.
                //
                // `Read` and not `Const` for all of them: what they read is memory another
                // statement may write, so they may not be carried across a write.
                (
                    Expr::Ident(name),
                    "box" | "addr" | "ptr_eq" | "ptr_cmp" | "compare_bytes" | "offset"
                    | "unscale" | "unwindow" | "thin" | "read_array" | "array_ref",
                ) if name == "__rt" => Purity::Read,
                // Everything else in the shim writes: `copy`, `copy_nonoverlapping`,
                // `write_bytes`, `write_array` and `overwrite` all store through a pointer, and a
                // store may not move at all.
                (Expr::Ident(name), _) if name == "__rt" => Purity::Effect,
                // An accessor's getter, an array copy, a fill of a fresh array.
                (object, "get" | "slice" | "fill") => self.purity(object).max(Purity::Read),
                _ => Purity::Effect,
            },
            Expr::Ident(name) if name == "BigInt" || name == "Number" => Purity::Const,
            _ => Purity::Effect,
        }
    }
}

/// The pointer of a `p.buf[p.off]` slot dereference, when that is what this index is.
///
/// The shape matters twice: it is one read of a place rather than two of a pointer, and the
/// pointer's expression can be substituted into it *because* folding a slot literal's properties
/// away (`jsast::member`) leaves an ordinary assignable `x[0]` behind.
fn slot_deref_name<'a>(object: &'a Expr, index: &'a Expr) -> Option<&'a str> {
    let (Expr::Member(buffer, buf), Expr::Member(offset, off)) = (object, index) else {
        return None;
    };
    if buf != crate::ptr::BUF || off != crate::ptr::OFF {
        return None;
    }
    match (&**buffer, &**offset) {
        (Expr::Ident(pointer), Expr::Ident(same)) if pointer == same => Some(pointer),
        _ => None,
    }
}

/// Whether an expression may be written twice, once in each half of `p.buf[p.off]`.
///
/// Two cases, and both cost nothing:
///
/// * a **slot literal**, because reading a property out of one folds it away entirely
///   ([`jsast::member`]), so `{ buf: x, off: 0 }.buf[{ buf: x, off: 0 }.off]` prints as `x[0]`;
/// * a **place expression** — a name, or a chain of fixed field and element reads over one — of
///   which two adjacent evaluations always agree, because a slot dereference evaluates nothing
///   between them, and which is no longer than the temporary it saves.
///
/// Anything that calls, assigns or allocates is excluded: evaluating it twice would do it twice.
fn duplicable_pointer(value: &Expr) -> bool {
    match value {
        Expr::Object(entries) => {
            jsast::folds_property(entries, crate::ptr::BUF)
                && jsast::folds_property(entries, crate::ptr::OFF)
        }
        Expr::Ident(_) => true,
        Expr::Member(object, _) => duplicable_pointer(object),
        Expr::Index(object, index) => {
            duplicable_pointer(object)
                && matches!(&**index, Expr::Num(_) | Expr::Str(_) | Expr::Ident(_))
        }
        _ => false,
    }
}

/// Whether an expression names one of the runtime's own globals.
fn pure_global(expr: &Expr) -> bool {
    matches!(expr, Expr::Ident(name) if PURE_GLOBALS.contains(&name.as_str()))
}

// ---------------------------------------------------------------------------
// Walking the tree of an emitted statement
// ---------------------------------------------------------------------------

/// Calls `f` with every identifier `stmt` reads or writes.
///
/// Member names and object keys are `String`s rather than expressions in this AST, so `a.length`
/// contributes only `a` — see `jsast::visit_idents`, which this mirrors for the two things the
/// queue needs and that one does not offer: counting, and a version that takes an expression.
fn each_ident_stmt(stmt: &Stmt, f: &mut dyn FnMut(&str)) {
    fn list(stmts: &[Stmt], f: &mut dyn FnMut(&str)) {
        for stmt in stmts {
            each_ident_stmt(stmt, f);
        }
    }
    match stmt {
        Stmt::ExprStmt(expr) | Stmt::Throw(expr) => each_ident(expr, f),
        Stmt::Let(_, init) => {
            if let Some(init) = init {
                each_ident(init, f);
            }
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(init) = init {
                    each_ident(init, f);
                }
            }
        }
        Stmt::Const(_, init) => each_ident(init, f),
        Stmt::Return(value) => {
            if let Some(value) = value {
                each_ident(value, f);
            }
        }
        Stmt::If(test, then_, else_) => {
            each_ident(test, f);
            list(then_, f);
            if let Some(else_) = else_ {
                list(else_, f);
            }
        }
        Stmt::Switch(disc, cases) => {
            each_ident(disc, f);
            for case in cases {
                for test in &case.tests {
                    each_ident(test, f);
                }
                list(&case.body, f);
            }
        }
        Stmt::Labeled(_, inner) => each_ident_stmt(inner, f),
        Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::FunctionDecl { body, .. } => {
            list(body, f)
        }
        Stmt::While(test, body) => {
            each_ident(test, f);
            list(body, f);
        }
        // An import declares its bindings rather than mentioning them; see `jsast::visit_idents`.
        Stmt::Import { .. }
        | Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => {}
    }
}

/// Calls `f` with every identifier `expr` *reads*, function bodies included.
///
/// The target of an assignment to a bare name is a write and is skipped: `_5 = _5 + 1` reads
/// `_5` once. Writes are collected separately, by [`assigned_roots`].
fn each_ident(expr: &Expr, f: &mut dyn FnMut(&str)) {
    match expr {
        Expr::Ident(name) => f(name),
        Expr::Assign(target, value) if matches!(&**target, Expr::Ident(_)) => each_ident(value, f),
        Expr::Member(object, _) => each_ident(object, f),
        // A slot dereference names its pointer twice and reads it *once*: substituting the
        // pointer's expression into both halves collapses them back to one place expression (see
        // [`Subst::expr`]), so counting two mentions here would decline a substitution that costs
        // nothing. The substitution is atomic — both halves or neither — which is what makes this
        // count honest rather than optimistic.
        Expr::Index(object, index) if slot_deref_name(object, index).is_some() => {
            f(slot_deref_name(object, index).unwrap_or_default())
        }
        Expr::Index(object, index) => {
            each_ident(object, f);
            each_ident(index, f);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            each_ident(callee, f);
            for arg in args {
                each_ident(arg, f);
            }
        }
        Expr::Unary(_, operand) => each_ident(operand, f),
        Expr::Binary(_, lhs, rhs)
        | Expr::Assign(lhs, rhs)
        | Expr::CompoundAssign(_, lhs, rhs) => {
            each_ident(lhs, f);
            each_ident(rhs, f);
        }
        Expr::Cond(test, consequent, alternate) => {
            each_ident(test, f);
            each_ident(consequent, f);
            each_ident(alternate, f);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                each_ident(value, f);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                each_ident(item, f);
            }
        }
        Expr::Arrow(_, body) => match body {
            ArrowBody::Expr(body) => each_ident(body, f),
            ArrowBody::Block(body) => {
                for stmt in body {
                    each_ident_stmt(stmt, f);
                }
            }
        },
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Raw(_) => {}
    }
}

/// How many times `stmt` mentions `name`.
fn count_in_stmt(stmt: &Stmt, name: &str) -> usize {
    let mut count = 0;
    each_ident_stmt(stmt, &mut |ident| {
        if ident == name {
            count += 1;
        }
    });
    count
}

/// How many times `expr` mentions `name`.
fn count_in_expr(expr: &Expr, name: &str) -> usize {
    let mut count = 0;
    each_ident(expr, &mut |ident| {
        if ident == name {
            count += 1;
        }
    });
    count
}

/// The variable an assignment target names, for the targets that name one.
///
/// `_5 = ..` and `_5.x = ..` both write to `_5`; a queued entry that mentions it has to be
/// flushed before either.
fn assigned_root(target: &Expr) -> Option<&str> {
    match target {
        Expr::Ident(name) => Some(name),
        Expr::Member(object, _) => assigned_root(object),
        Expr::Index(object, _) => assigned_root(object),
        _ => None,
    }
}

/// Every variable a statement assigns to, however deeply the assignment is buried.
fn assigned_roots(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::ExprStmt(expr) | Stmt::Throw(expr) | Stmt::Const(_, expr) => {
            collect_assigned(expr, out)
        }
        Stmt::Let(_, Some(expr)) | Stmt::Return(Some(expr)) => collect_assigned(expr, out),
        Stmt::If(test, then_, else_) => {
            collect_assigned(test, out);
            for stmt in then_.iter().chain(else_.iter().flatten()) {
                assigned_roots(stmt, out);
            }
        }
        Stmt::While(test, body) => {
            collect_assigned(test, out);
            for stmt in body {
                assigned_roots(stmt, out);
            }
        }
        Stmt::Switch(disc, cases) => {
            collect_assigned(disc, out);
            for stmt in cases.iter().flat_map(|case| &case.body) {
                assigned_roots(stmt, out);
            }
        }
        Stmt::Labeled(_, inner) => assigned_roots(inner, out),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => {
            for stmt in body {
                assigned_roots(stmt, out);
            }
        }
        _ => {}
    }
}

fn collect_assigned(expr: &Expr, out: &mut Vec<String>) {
    if let Expr::Assign(target, _) | Expr::CompoundAssign(_, target, _) = expr {
        if let Some(root) = assigned_root(target) {
            out.push(root.to_string());
        }
    }
    // The assignment may be nested: a hoisted temporary rides in a comma expression.
    match expr {
        Expr::Member(object, _) | Expr::Unary(_, object) => collect_assigned(object, out),
        Expr::Index(lhs, rhs)
        | Expr::Binary(_, lhs, rhs)
        | Expr::Assign(lhs, rhs)
        | Expr::CompoundAssign(_, lhs, rhs) => {
            collect_assigned(lhs, out);
            collect_assigned(rhs, out);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            collect_assigned(callee, out);
            for arg in args {
                collect_assigned(arg, out);
            }
        }
        Expr::Cond(test, consequent, alternate) => {
            collect_assigned(test, out);
            collect_assigned(consequent, out);
            collect_assigned(alternate, out);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                collect_assigned(value, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                collect_assigned(item, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Substitution
// ---------------------------------------------------------------------------

/// One pass over an expression, replacing reads of queued names with the expressions behind them.
///
/// The walk follows JavaScript's evaluation order, accumulating in `seen` how impure everything
/// evaluated so far is. A substitution is only made when the entry commutes with that — putting
/// an entry into the second operand of `a.x + _5` runs `a.x` *before* whatever `_5` held, and
/// that reordering has to be one the ladder allows.
///
/// Positions that are evaluated conditionally — the arms of a `?:`, the right operand of `&&`,
/// the body of an arrow — are walked for their purity but never substituted into: an expression
/// moved there might not run at all, or might run more than once.
struct Subst<'a, 'b> {
    queuer: &'a Queuer<'b>,
    queue: &'a mut Vec<Entry>,
    /// The names that occur exactly once in the statement, and so may be substituted.
    once: &'a HashSet<String>,
    /// How impure everything already evaluated in this expression is.
    seen: Purity,
}

impl Subst<'_, '_> {
    fn expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Ident(name) => {
                if let Some(value) = self.take(name) {
                    *expr = value;
                } else {
                    self.saw(self.queuer.ident_purity(name));
                }
            }
            Expr::Member(object, name) => {
                if pure_global(object) {
                    return;
                }
                self.expr(object);
                self.saw(Purity::Read);
                // A substitution may have put a slot literal where the pointer's name was, and
                // reading a property straight out of a literal is the value it was given.
                if matches!(&**object, Expr::Object(_)) {
                    let name = name.clone();
                    let object = std::mem::replace(&mut **object, Expr::Undefined);
                    *expr = jsast::member(object, name);
                }
            }
            // `p.buf[p.off]`: one read, and one substitution site. Both halves are replaced
            // together or neither is, because the counting rule in [`each_ident`] treats them as
            // a single mention and a half substituted pair would leave a dangling name behind.
            Expr::Index(object, index) if slot_deref_name(object, index).is_some() => {
                let name = slot_deref_name(object, index).unwrap_or_default().to_string();
                self.saw(Purity::Read);
                if !self.foldable(&name) {
                    self.saw(self.queuer.ident_purity(&name));
                    return;
                }
                let Some(pointer) = self.take(&name) else {
                    self.saw(self.queuer.ident_purity(&name));
                    return;
                };
                *expr = jsast::index(
                    jsast::member(pointer.clone(), crate::ptr::BUF),
                    jsast::member(pointer, crate::ptr::OFF),
                );
            }
            Expr::Index(object, index) => {
                self.expr(object);
                self.expr(index);
                self.saw(Purity::Read);
            }
            Expr::Call(callee, args) => {
                let purity = self.queuer.callee_purity(callee);
                // The callee is a name or a member of one; only its object can hold a read.
                match &mut **callee {
                    Expr::Member(object, _) if !pure_global(object) => self.expr(object),
                    Expr::Ident(_) | Expr::Member(..) => {}
                    other => self.opaque(other),
                }
                for arg in args {
                    self.expr(arg);
                }
                // The call happens after its arguments, so an argument may still be substituted
                // even when the call itself is an effect.
                self.saw(purity);
            }
            Expr::New(callee, args) => {
                let purity = match &**callee {
                    Expr::Ident(name) if name == "Array" => Purity::Read,
                    _ => Purity::Effect,
                };
                for arg in args {
                    self.expr(arg);
                }
                self.saw(purity);
            }
            Expr::Unary(_, operand) => self.expr(operand),
            // `&&` and `||` only evaluate their right operand sometimes.
            Expr::Binary(BinOp::And | BinOp::Or, lhs, rhs) => {
                self.expr(lhs);
                self.opaque(rhs);
            }
            Expr::Binary(_, lhs, rhs) => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Assign(target, value) | Expr::CompoundAssign(_, target, value) => {
                self.assign_target(target);
                self.expr(value);
                self.saw(Purity::Effect);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.opaque(consequent);
                self.opaque(alternate);
            }
            Expr::Object(entries) => {
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(..) | Expr::Raw(_) => self.opaque(expr),
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null => {}
        }
    }

    /// The left hand side of an assignment.
    ///
    /// Nothing is substituted into an assignment target — it has to stay a place — with one
    /// exception: the pointer of a `p.buf[p.off] = v` may move in, because a slot literal whose
    /// properties fold away leaves `x[0]`, which is as assignable as what it replaced.
    fn assign_target(&mut self, target: &mut Expr) {
        if let Expr::Index(object, index) = target {
            if slot_deref_name(object, index).is_some() {
                self.expr(target);
                return;
            }
        }
        self.opaque(target);
    }

    /// Whether the queued expression for `name` may be substituted into *both* halves of a slot
    /// dereference.
    fn foldable(&self, name: &str) -> bool {
        self.queue
            .iter()
            .find(|entry| entry.name == name)
            .is_some_and(|entry| duplicable_pointer(&entry.value))
    }

    /// The queued expression for `name`, if it may be substituted here.
    fn take(&mut self, name: &str) -> Option<Expr> {
        if !self.once.contains(name) {
            return None;
        }
        let index = self.queue.iter().position(|entry| entry.name == name)?;
        if !commutes(self.seen, self.queue[index].purity) {
            return None;
        }
        let entry = self.queue.remove(index);
        self.seen = self.seen.max(entry.purity);
        Some(entry.value)
    }

    /// Accounts for a subexpression without substituting into it.
    fn opaque(&mut self, expr: &Expr) {
        self.saw(self.queuer.purity(expr));
    }

    fn saw(&mut self, purity: Purity) {
        self.seen = self.seen.max(purity);
    }
}

// ---------------------------------------------------------------------------
// The pass
// ---------------------------------------------------------------------------

/// A region tree with the queue's statement lists in it.
type Tree<'tcx> = Region<Vec<Stmt>, Scrutinee<'tcx>>;

impl Queuer<'_> {
    /// Rewrites one region and everything under it.
    ///
    /// A `Seq` is the only place a queued entry can outlive the statement that produced it: its
    /// children run one after another with nothing between them, so the region tree's order *is*
    /// textual order and an assignment at the end of one block can reach a read in the next. Every
    /// other region is a boundary, and the queue is emptied at it.
    fn region<'tcx>(&self, region: Tree<'tcx>, top: bool) -> Tree<'tcx> {
        let mut children = Vec::new();
        flatten(region, &mut children);

        let mut queue: Vec<Entry> = Vec::new();
        let mut stmts: Vec<Stmt> = Vec::new();
        let mut out: Vec<Tree<'tcx>> = Vec::new();

        for child in children {
            match child {
                Region::Seq(_) => unreachable!("sequences are flattened"),
                Region::Block(block) => {
                    for stmt in block {
                        self.statement(stmt, top, &mut queue, &mut stmts);
                    }
                }
                Region::LetTmp { tmp, mut value } => {
                    let name = crate::emit::tmp_name(tmp);
                    self.assignment(&mut value.value, &name, &mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::LetTmp { tmp, value });
                }
                Region::SetSel(tmp, value) => {
                    let name = crate::emit::tmp_name(tmp);
                    self.flush(&mut queue, &mut stmts, Purity::Const, &[], &[name]);
                    close(&mut out, &mut stmts);
                    out.push(Region::SetSel(tmp, value));
                }
                Region::If { scrut, test, then_, else_ } => {
                    let scrut = self.scrutinee(scrut, &mut queue);
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::If {
                        scrut,
                        test,
                        then_: Box::new(self.region(*then_, false)),
                        else_: Box::new(self.region(*else_, false)),
                    });
                }
                Region::Switch { scrut, arms, default, label } => {
                    let scrut = self.scrutinee(scrut, &mut queue);
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::Switch {
                        scrut,
                        arms: arms
                            .into_iter()
                            .map(|(values, body)| (values, self.region(body, false)))
                            .collect(),
                        default: Box::new(self.region(*default, false)),
                        label,
                    });
                }
                // Destination passing: the assignment to the return place, if it is still the last
                // thing emitted, becomes the value of the `return` instead.
                Region::Return => {
                    flush_all(&mut queue, &mut stmts);
                    let value = if self.unit_return {
                        None
                    } else if self.ret_boxed {
                        Some(self.ret_read.clone())
                    } else {
                        Some(take_assignment(&mut stmts, &self.ret).unwrap_or_else(|| {
                            jsast::id(self.ret.clone())
                        }))
                    };
                    stmts.push(Stmt::Return(value));
                }
                Region::Labeled { label, body } => {
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::Labeled { label, body: Box::new(self.region(*body, false)) });
                }
                Region::Loop { label, body } => {
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::Loop { label, body: Box::new(self.region(*body, false)) });
                }
                Region::Dispatch { sel, label, cases, trailing_break } => {
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(Region::Dispatch {
                        sel,
                        label,
                        cases: cases
                            .into_iter()
                            .map(|(value, body)| (value, self.region(body, false)))
                            .collect(),
                        trailing_break,
                    });
                }
                jump @ (Region::Break(_)
                | Region::Continue(_)
                | Region::Unreachable
                | Region::Throw) => {
                    flush_all(&mut queue, &mut stmts);
                    close(&mut out, &mut stmts);
                    out.push(jump);
                }
            }
        }

        flush_all(&mut queue, &mut stmts);
        close(&mut out, &mut stmts);
        if out.len() == 1 { out.pop().unwrap_or(Region::Seq(Vec::new())) } else { Region::Seq(out) }
    }

    /// One statement of a block payload.
    fn statement(&self, stmt: Stmt, top: bool, queue: &mut Vec<Entry>, out: &mut Vec<Stmt>) {
        // A location comment and a source map marker name nothing and do nothing; letting either
        // flush the queue would make `js-line-comments` and `js-source-map` change the code around
        // the locations they record.
        if matches!(stmt, Stmt::Comment(_) | Stmt::Loc(_)) {
            out.push(stmt);
            return;
        }

        // An assignment to a bare name is the shape everything here is about; it is also the only
        // one whose target must not be read as a use.
        let mut stmt = match stmt {
            Stmt::ExprStmt(Expr::Assign(target, value)) => match *target {
                Expr::Ident(name) => {
                    let mut value = *value;
                    let purity = self.assignment(&mut value, &name, queue, out);
                    self.place(name, value, purity, top, queue, out);
                    return;
                }
                target => Stmt::ExprStmt(Expr::Assign(Box::new(target), value)),
            },
            stmt => stmt,
        };
        let once = self.substitutable(queue, |entry| count_in_stmt(&stmt, entry));
        let purity = match &mut stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) | Stmt::Return(Some(expr)) => {
                self.substitute(expr, queue, &once)
            }
            // The branches of an `Assert`'s guard run conditionally, so only its test is open to
            // substitution and the statement as a whole counts as an effect.
            Stmt::If(test, ..) => {
                self.substitute(test, queue, &once);
                Purity::Effect
            }
            _ => Purity::Effect,
        };

        let mut writes = Vec::new();
        assigned_roots(&stmt, &mut writes);
        let still = self.mentioned(queue, |entry| count_in_stmt(&stmt, entry) > 0);
        self.flush(queue, out, purity, &still, &writes);
        out.push(stmt);
    }

    /// Substitutes into the right hand side of an assignment and flushes what it displaces.
    ///
    /// Returns how impure the right hand side turned out to be, which is also the entry's own
    /// purity if it ends up queued.
    fn assignment(
        &self,
        value: &mut Expr,
        target: &str,
        queue: &mut Vec<Entry>,
        out: &mut Vec<Stmt>,
    ) -> Purity {
        let once = self.substitutable(queue, |entry| count_in_expr(value, entry));
        let purity = self.substitute(value, queue, &once);
        let still = self.mentioned(queue, |entry| count_in_expr(value, entry) > 0);
        let writes = [target.to_string()];
        self.flush(queue, out, purity, &still, &writes);
        purity
    }

    /// Queues, drops or emits a finished assignment.
    fn place(
        &self,
        name: String,
        value: Expr,
        purity: Purity,
        top: bool,
        queue: &mut Vec<Entry>,
        out: &mut Vec<Stmt>,
    ) {
        if let Some(local) = self.local(&name) {
            if self.uses.inlinable(local) && self.reads(&name) == 1 {
                queue.push(Entry { name, value, purity });
                return;
            }
            // Nothing reads the target, so only what the right hand side *does* is left.
            if self.uses.dead(local, self.unit_return) && self.reads(&name) == 0 {
                if purity == Purity::Effect {
                    out.push(jsast::expr_stmt(value));
                }
                return;
            }
            // `js-scoped-lets`: a name assigned once, at the outermost level of the body, can be
            // declared where it is assigned instead of in the prelude. Only the outermost level:
            // everything below is a branch, a loop body or a `switch` case, and the cases of one
            // `switch` share a block scope, so a declaration in one would collide with the same
            // declaration in another. Reading it before the declaration would be reading it before
            // its only assignment, which MIR does not produce.
            if self.scoped && top && self.uses.constant(local) {
                out.push(jsast::const_(name, value));
                return;
            }
        }
        out.push(jsast::assign_stmt(jsast::id(name), value));
    }

    /// The value an `If` or `Switch` branches on, with the queue folded into it.
    /// The caller flushes what is left immediately afterwards, so nothing has to be flushed here:
    /// every entry that stays behind is emitted before the branch either way.
    fn scrutinee<'tcx>(
        &self,
        scrut: Scrut<Scrutinee<'tcx>>,
        queue: &mut Vec<Entry>,
    ) -> Scrut<Scrutinee<'tcx>> {
        match scrut {
            Scrut::Payload(mut scrutinee) => {
                let once =
                    self.substitutable(queue, |entry| count_in_expr(&scrutinee.value, entry));
                self.substitute(&mut scrutinee.value, queue, &once);
                Scrut::Payload(scrutinee)
            }
            Scrut::Tmp(tmp) => Scrut::Tmp(tmp),
        }
    }

    fn substitute(&self, expr: &mut Expr, queue: &mut Vec<Entry>, once: &HashSet<String>) -> Purity {
        let mut subst = Subst { queuer: self, queue, once, seen: Purity::Const };
        subst.expr(expr);
        subst.seen
    }

    /// The queued names that occur exactly once, and so may move to where they are read.
    fn substitutable(
        &self,
        queue: &[Entry],
        count: impl Fn(&str) -> usize,
    ) -> HashSet<String> {
        queue
            .iter()
            .filter(|entry| count(&entry.name) == 1)
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// The queued names a predicate still finds.
    fn mentioned(&self, queue: &[Entry], keep: impl Fn(&str) -> bool) -> Vec<String> {
        queue.iter().filter(|entry| keep(&entry.name)).map(|entry| entry.name.clone()).collect()
    }

    /// Emits every queued entry that cannot stay behind, in the order the entries were queued.
    ///
    /// An entry stays only when it commutes with what is about to be emitted, is not named by it,
    /// and does not read a variable it assigns.
    fn flush(
        &self,
        queue: &mut Vec<Entry>,
        out: &mut Vec<Stmt>,
        purity: Purity,
        still: &[String],
        writes: &[String],
    ) {
        let mut kept = Vec::with_capacity(queue.len());
        for entry in queue.drain(..) {
            let stays = commutes(purity, entry.purity)
                && !still.iter().any(|name| *name == entry.name)
                && !writes.iter().any(|name| count_in_expr(&entry.value, name) > 0);
            if stays {
                kept.push(entry);
            } else {
                out.push(jsast::assign_stmt(jsast::id(entry.name), entry.value));
            }
        }
        *queue = kept;
    }
}

/// Empties the queue into the output, in order.
fn flush_all(queue: &mut Vec<Entry>, out: &mut Vec<Stmt>) {
    for entry in queue.drain(..) {
        out.push(jsast::assign_stmt(jsast::id(entry.name), entry.value));
    }
}

/// Unnests sequences, so that one walk sees every region that runs in textual order.
fn flatten<'tcx>(region: Tree<'tcx>, out: &mut Vec<Tree<'tcx>>) {
    match region {
        Region::Seq(children) => {
            for child in children {
                flatten(child, out);
            }
        }
        other => out.push(other),
    }
}

/// Moves the statements collected so far into a block region.
fn close<'tcx>(out: &mut Vec<Tree<'tcx>>, stmts: &mut Vec<Stmt>) {
    if !stmts.is_empty() {
        out.push(Region::Block(std::mem::take(stmts)));
    }
}

/// Takes the value of a trailing `name = value;`, removing the statement.
///
/// A source map marker prints nothing, so one sitting after the assignment does not make it any
/// less trailing: looking through it is what keeps `js-source-map=on` from silently turning
/// destination passing off and emitting different JavaScript.
fn take_assignment(stmts: &mut Vec<Stmt>, name: &str) -> Option<Expr> {
    let last = stmts.iter().rposition(|stmt| !matches!(stmt, Stmt::Loc(_)))?;
    let matches = match &stmts[last] {
        Stmt::ExprStmt(Expr::Assign(target, _)) => {
            matches!(&**target, Expr::Ident(ident) if ident == name)
        }
        _ => false,
    };
    if !matches {
        return None;
    }
    match stmts.remove(last) {
        Stmt::ExprStmt(Expr::Assign(_, value)) => Some(*value),
        _ => None,
    }
}
