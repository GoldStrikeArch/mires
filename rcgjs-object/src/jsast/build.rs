//! Convenience constructors, identifier helpers, float bit helpers, and ES2018 lowerings.

use std::collections::BTreeSet;
use std::fmt;

use super::ast::{
    ArrowBody, BinOp, CompoundOp, Expr, ImportSpecifiers, Loc, Stmt, SwitchCase, UnOp,
};
use super::visit::expression_names;

/// Whether `name` can be written without quotes as a member name or an object key.
///
/// Reserved words are rejected even though modern JavaScript allows them in both positions,
/// so that the output stays valid under any parser.
pub fn is_plain_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(c) => c.is_ascii_alphabetic() || c == '_' || c == '$',
        None => false,
    };
    valid && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') && !is_reserved(name)
}

/// Whether `name` is a JavaScript reserved word.
pub fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

/// Turns an arbitrary name into a usable JavaScript identifier.
///
/// Characters outside `[A-Za-z0-9_$]` become `_`, so a Rust path separator `::` turns into
/// `__`. A name that starts with a digit or that collides with a reserved word is prefixed
/// with `_`. The mapping is not injective: append a hash when uniqueness matters.
pub fn sanitize_ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 1);
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) || is_reserved(&out) {
        out.insert(0, '_');
    }
    out
}

/// A number literal. Accepts anything that converts to `f64`, so `num(1)` works.
pub fn num(value: impl Into<f64>) -> Expr {
    Expr::Num(value.into())
}

/// An `f32` constant from its raw IEEE 754 bits.
///
/// Finite values, infinities, signed zeroes, and the canonical quiet `NaN` (`0x7fc0_0000`) spell
/// as ordinary JavaScript number literals. Any other `NaN` (non-zero payload, negative sign bit,
/// or signaling `NaN`) is constructed through a typed-array bit-cast so its payload and sign bit
/// survive into JavaScript without touching `BigInt` (MIRES-023).
pub fn f32_from_bits(bits: u32) -> Expr {
    const CANONICAL_QNAN: u32 = 0x7fc0_0000;
    let value = f32::from_bits(bits);
    if value.is_nan() && bits != CANONICAL_QNAN {
        let source = method_call(id("Uint32Array"), "of", vec![num(f64::from(bits))]);
        let view = new(id("Float32Array"), vec![member(source, "buffer")]);
        return index(view, num(0));
    }
    num(f64::from(value))
}

/// An `f64` constant from its raw IEEE 754 bits.
///
/// Finite values, infinities, signed zeroes, and the canonical quiet `NaN`
/// (`0x7ff8_0000_0000_0000`) spell as ordinary JavaScript number literals. Any other `NaN` is
/// constructed through a little-endian `Uint32Array` → `Float64Array` bit-cast so its payload and
/// sign bit survive without requiring `BigInt` or `BigUint64Array` (ES2018-safe, MIRES-023).
pub fn f64_from_bits(bits: u64) -> Expr {
    const CANONICAL_QNAN: u64 = 0x7ff8_0000_0000_0000;
    let value = f64::from_bits(bits);
    if value.is_nan() && bits != CANONICAL_QNAN {
        let lo = (bits & 0xffff_ffff) as u32;
        let hi = (bits >> 32) as u32;
        let source = method_call(
            id("Uint32Array"),
            "of",
            vec![num(f64::from(lo)), num(f64::from(hi))],
        );
        let view = new(id("Float64Array"), vec![member(source, "buffer")]);
        return index(view, num(0));
    }
    num(value)
}

/// A number literal spelled exactly as given.
pub fn raw_num(text: impl Into<String>) -> Expr {
    Expr::RawNum(text.into())
}

/// A `BigInt` literal, printed with the trailing `n`. Accepts `i128` and `u128` alike.
pub fn bigint(value: impl fmt::Display) -> Expr {
    Expr::BigInt(value.to_string())
}

/// A string literal.
pub fn string(value: impl Into<String>) -> Expr {
    Expr::Str(value.into())
}

/// A boolean literal.
pub fn boolean(value: bool) -> Expr {
    Expr::Bool(value)
}

/// `undefined`, which is also the representation of unit and of zero sized types.
pub fn undefined() -> Expr {
    Expr::Undefined
}

/// `null`.
pub fn null() -> Expr {
    Expr::Null
}

/// An identifier reference.
pub fn id(name: impl Into<String>) -> Expr {
    Expr::Ident(name.into())
}

/// Verbatim JavaScript in expression position.
pub fn raw_expr(text: impl Into<String>) -> Expr {
    Expr::Raw(text.into())
}

/// `obj.name`, with a property read out of an object literal folded away.
///
/// `{ buf: xs, off: 0 }.buf` is `xs`. The fold exists for the pointer model: a `&mut` to a local
/// is the slot literal `{ buf: x, off: 0 }`, dereferencing it is `p.buf[p.off]`, and putting the
/// two together without this would print `{ buf: x, off: 0 }.buf[{ buf: x, off: 0 }.off]` where
/// `x[0]` is meant. Every entry that is dropped has to be one whose evaluation could not be
/// observed ([`is_droppable`]), and a duplicated key declines the fold rather than guessing which
/// one JavaScript would have kept.
pub fn member(obj: Expr, name: impl Into<String>) -> Expr {
    let name = name.into();
    if let Expr::Object(entries) = &obj {
        if let Some(value) = fold_property(entries, &name) {
            return value;
        }
    }
    Expr::Member(Box::new(obj), name)
}

/// Whether reading `name` out of this object literal folds away to one of its entries.
///
/// The question [`member`] answers by folding, asked ahead of time: `queue.rs` moves an expression
/// to where it is read only when doing so does not duplicate it, and a slot literal substituted
/// into both halves of `p.buf[p.off]` duplicates nothing exactly when both reads fold.
pub fn folds_property(entries: &[(String, Expr)], name: &str) -> bool {
    fold_property(entries, name).is_some()
}

/// The value an object literal gives `name`, when reading it can drop the other entries.
fn fold_property(entries: &[(String, Expr)], name: &str) -> Option<Expr> {
    let mut found: Option<&Expr> = None;
    for (key, value) in entries {
        if key == name {
            // A repeated key is JavaScript's business, not this peephole's.
            if found.is_some() {
                return None;
            }
            found = Some(value);
        } else if !is_droppable(value) {
            return None;
        }
    }
    found.cloned()
}

/// Whether an expression can be dropped without changing what the program does.
///
/// Deliberately narrow: anything that calls, assigns or is verbatim text is kept.
pub fn is_droppable(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Arrow(..) => true,
        Expr::Member(object, _) | Expr::Unary(_, object) => is_droppable(object),
        Expr::Index(lhs, rhs) | Expr::Binary(_, lhs, rhs) => is_droppable(lhs) && is_droppable(rhs),
        Expr::Cond(test, consequent, alternate) => {
            is_droppable(test) && is_droppable(consequent) && is_droppable(alternate)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().all(is_droppable),
        Expr::Object(entries) => entries.iter().all(|(_, value)| is_droppable(value)),
        Expr::Call(..)
        | Expr::New(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::Raw(_) => false,
    }
}

/// `obj[index]`, spelled `obj.name` when the index is a fixed property name.
///
/// A slot whose `off` is a string names a field, so `p.buf[p.off]` on one folds to `x["field"]`;
/// that is the same property access as `x.field`, and the printer only puts the brackets back for
/// a name that is not an identifier.
pub fn index(obj: Expr, index: Expr) -> Expr {
    if let Expr::Str(name) = &index {
        if is_plain_ident(name) {
            return member(obj, name.clone());
        }
    }
    Expr::Index(Box::new(obj), Box::new(index))
}

/// `callee(args...)`.
pub fn call(callee: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(Box::new(callee), args)
}

/// `obj.name(args...)`.
pub fn method_call(obj: Expr, name: impl Into<String>, args: Vec<Expr>) -> Expr {
    call(member(obj, name), args)
}

/// `new callee(args...)`.
pub fn new(callee: Expr, args: Vec<Expr>) -> Expr {
    Expr::New(Box::new(callee), args)
}

/// A prefix operator application.
pub fn unary(op: UnOp, operand: Expr) -> Expr {
    Expr::Unary(op, Box::new(operand))
}

/// `-operand`.
pub fn neg(operand: Expr) -> Expr {
    unary(UnOp::Neg, operand)
}

/// `!operand`.
pub fn not(operand: Expr) -> Expr {
    unary(UnOp::Not, operand)
}

/// `~operand`.
pub fn bit_not(operand: Expr) -> Expr {
    unary(UnOp::BitNot, operand)
}

/// `typeof operand`.
pub fn type_of(operand: Expr) -> Expr {
    unary(UnOp::TypeOf, operand)
}

/// An infix operator application.
pub fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::Binary(op, Box::new(lhs), Box::new(rhs))
}

/// `target = value`.
pub fn assign(target: Expr, value: Expr) -> Expr {
    Expr::Assign(Box::new(target), Box::new(value))
}

/// `target op= value`.
pub fn compound_assign(op: CompoundOp, target: Expr, value: Expr) -> Expr {
    Expr::CompoundAssign(op, Box::new(target), Box::new(value))
}

/// Nullish assignment with ES2018 syntax and one evaluation of the target reference and value.
///
/// This AST deliberately has no `??=` node. Lowering at construction time keeps a later-syntax
/// capability out of every target while preserving the distinction between nullish and falsy.
pub fn nullish_assign(target: Expr, value: Expr) -> Expr {
    let mut used = expression_names([&target, &value]);
    match target {
        Expr::Ident(name) => {
            let current = fresh_identifier(&mut used, "value");
            call(
                arrow(
                    vec![current.clone()],
                    nullish_choice(&current, id(name.clone()), value),
                ),
                vec![id(name)],
            )
        }
        Expr::Member(object, property) => {
            let receiver = fresh_identifier(&mut used, "object");
            let current = fresh_identifier(&mut used, "value");
            let slot = || member(id(receiver.clone()), property.clone());
            let read_once = call(
                arrow(
                    vec![current.clone()],
                    nullish_choice(&current, slot(), value),
                ),
                vec![slot()],
            );
            call(arrow(vec![receiver], read_once), vec![*object])
        }
        Expr::Index(object, key) => {
            let receiver = fresh_identifier(&mut used, "object");
            let index_name = fresh_identifier(&mut used, "key");
            let current = fresh_identifier(&mut used, "value");
            let slot = || index(id(receiver.clone()), id(index_name.clone()));
            let read_once = call(
                arrow(
                    vec![current.clone()],
                    nullish_choice(&current, slot(), value),
                ),
                vec![slot()],
            );
            call(
                arrow(vec![receiver, index_name], read_once),
                vec![*object, *key],
            )
        }
        other => {
            let current = fresh_identifier(&mut used, "value");
            call(
                arrow(
                    vec![current.clone()],
                    nullish_choice(&current, other.clone(), value),
                ),
                vec![other],
            )
        }
    }
}

fn nullish_choice(current: &str, target: Expr, value: Expr) -> Expr {
    let current_value = || id(current);
    let is_nullish = binary(
        BinOp::Or,
        binary(BinOp::StrictEq, current_value(), null()),
        binary(BinOp::StrictEq, current_value(), unary(UnOp::Void, num(0))),
    );
    cond(is_nullish, assign(target, value), current_value())
}

pub(super) fn lower_logical_assignment(op: BinOp, target: &Expr, value: &Expr) -> Expr {
    let mut used = expression_names([target, value]);
    match target {
        Expr::Ident(name) => binary(
            op,
            id(name.clone()),
            assign(id(name.clone()), value.clone()),
        ),
        Expr::Member(object, property) => {
            let receiver = fresh_identifier(&mut used, "object");
            let slot = || member(id(receiver.clone()), property.clone());
            let body = binary(op, slot(), assign(slot(), value.clone()));
            call(arrow(vec![receiver], body), vec![(**object).clone()])
        }
        Expr::Index(object, key) => {
            let receiver = fresh_identifier(&mut used, "object");
            let index_name = fresh_identifier(&mut used, "key");
            let slot = || index(id(receiver.clone()), id(index_name.clone()));
            let body = binary(op, slot(), assign(slot(), value.clone()));
            call(
                arrow(vec![receiver, index_name], body),
                vec![(**object).clone(), (**key).clone()],
            )
        }
        // Every valid assignment target expressible by this AST is handled above. Preserve an
        // ES2018 spelling for malformed hand-built nodes so the printer never leaks later syntax.
        other => binary(op, other.clone(), assign(other.clone(), value.clone())),
    }
}

fn fresh_identifier(used: &mut BTreeSet<String>, role: &str) -> String {
    for suffix in 0u32.. {
        let candidate = format!("__rcgjs_es2018_{role}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the identifier space is finite")
}

/// `test ? consequent : alternate`.
pub fn cond(test: Expr, consequent: Expr, alternate: Expr) -> Expr {
    Expr::Cond(Box::new(test), Box::new(consequent), Box::new(alternate))
}

/// An object literal.
pub fn object(entries: Vec<(String, Expr)>) -> Expr {
    Expr::Object(entries)
}

/// An object literal from string slices, to keep call sites short.
pub fn object_of(entries: Vec<(&str, Expr)>) -> Expr {
    Expr::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// An array literal, which is also how tuples and arrays are represented.
pub fn array(items: Vec<Expr>) -> Expr {
    Expr::Array(items)
}

/// An arrow function with an expression body.
pub fn arrow(params: Vec<String>, body: Expr) -> Expr {
    Expr::Arrow(params, ArrowBody::Expr(Box::new(body)))
}

/// An arrow function with a block body.
pub fn arrow_block(params: Vec<String>, body: Vec<Stmt>) -> Expr {
    Expr::Arrow(params, ArrowBody::Block(body))
}

/// A comma expression.
pub fn seq(items: Vec<Expr>) -> Expr {
    Expr::Seq(items)
}

/// `expr | 0`, the i32 wrap.
pub fn i32_wrap(expr: Expr) -> Expr {
    binary(BinOp::BitOr, expr, num(0))
}

/// `expr >>> 0`, the u32 wrap.
pub fn u32_wrap(expr: Expr) -> Expr {
    binary(BinOp::UShr, expr, num(0))
}

/// `expr << shift >> shift`, the sign extension for a value of `bits` bits.
pub fn sign_extend(bits: u32, expr: Expr) -> Expr {
    let shift = num(32u32.saturating_sub(bits));
    binary(BinOp::Shr, binary(BinOp::Shl, expr, shift.clone()), shift)
}

/// `expr & mask`, the unsigned truncation to `bits` bits.
pub fn zero_extend(bits: u32, expr: Expr) -> Expr {
    let mask = if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    binary(BinOp::BitAnd, expr, num(mask))
}

/// `Math.imul(lhs, rhs)`, the 32 bit multiply.
pub fn imul(lhs: Expr, rhs: Expr) -> Expr {
    call(member(id("Math"), "imul"), vec![lhs, rhs])
}

/// `Math.fround(expr)`, the rounding to `f32` precision.
pub fn fround(expr: Expr) -> Expr {
    call(member(id("Math"), "fround"), vec![expr])
}

/// `BigInt.asIntN(bits, expr)` or `BigInt.asUintN(bits, expr)`, the truncation of a `BigInt`.
pub fn bigint_mask(signed: bool, bits: u32, expr: Expr) -> Expr {
    let name = if signed { "asIntN" } else { "asUintN" };
    call(member(id("BigInt"), name), vec![num(bits), expr])
}

/// `BigInt(expr)`, the widening of a JS number to a `BigInt`. The argument has to be an integer
/// valued number; every masked integer this backend produces is one.
pub fn to_bigint(expr: Expr) -> Expr {
    call(id("BigInt"), vec![expr])
}

/// `Number(expr)`, the narrowing of a `BigInt` to a JS number. Truncate to the destination width
/// with [`bigint_mask`] first: past 2^53 the conversion rounds, and rounding before truncating
/// gives the wrong low bits.
pub fn to_number(expr: Expr) -> Expr {
    call(id("Number"), vec![expr])
}

/// `{ v: value }`, the cell that stands in for an address taken local.
pub fn cell(value: Expr) -> Expr {
    object_of(vec![("v", value)])
}

/// `place.v`, the read through a cell reference.
pub fn cell_get(place: Expr) -> Expr {
    member(place, "v")
}

/// `{ get: () => place, set: (v) => (place = v) }`, the reference to a place projection.
///
/// `param` names the setter parameter; pick something that cannot shadow a captured local.
pub fn accessor(place: Expr, param: &str) -> Expr {
    object_of(vec![
        ("get", arrow(vec![], place.clone())),
        (
            "set",
            arrow(vec![param.to_string()], assign(place, id(param))),
        ),
    ])
}

/// `__rt.name(args...)`, a call into the runtime shim.
pub fn rt_call(name: impl Into<String>, args: Vec<Expr>) -> Expr {
    call(member(id("__rt"), name), args)
}

/// An expression evaluated for its effect.
pub fn expr_stmt(expr: Expr) -> Stmt {
    Stmt::ExprStmt(expr)
}

/// `target = value;`.
pub fn assign_stmt(target: Expr, value: Expr) -> Stmt {
    Stmt::ExprStmt(assign(target, value))
}

/// `let name = init;`.
pub fn let_(name: impl Into<String>, init: Expr) -> Stmt {
    Stmt::Let(name.into(), Some(init))
}

/// `let name;`.
pub fn let_uninit(name: impl Into<String>) -> Stmt {
    Stmt::Let(name.into(), None)
}

/// `let a = 1, b, c = 3;`.
pub fn lets(decls: Vec<(String, Option<Expr>)>) -> Stmt {
    Stmt::Lets(decls)
}

/// `const name = init;`.
pub fn const_(name: impl Into<String>, init: Expr) -> Stmt {
    Stmt::Const(name.into(), init)
}

/// `return value;`.
pub fn ret(value: Expr) -> Stmt {
    Stmt::Return(Some(value))
}

/// `return;`.
pub fn ret_void() -> Stmt {
    Stmt::Return(None)
}

/// `if (test) { .. }`.
pub fn if_(test: Expr, then: Vec<Stmt>) -> Stmt {
    Stmt::If(test, then, None)
}

/// `if (test) { .. } else { .. }`.
pub fn if_else(test: Expr, then: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
    Stmt::If(test, then, Some(els))
}

/// `switch (disc) { .. }`.
pub fn switch(disc: Expr, cases: Vec<SwitchCase>) -> Stmt {
    Stmt::Switch(disc, cases)
}

/// One `case` group. Several tests share one body, which is how fallthrough labels are built.
pub fn case(tests: Vec<Expr>, body: Vec<Stmt>) -> SwitchCase {
    SwitchCase { tests, body }
}

/// The `default` group.
pub fn default_case(body: Vec<Stmt>) -> SwitchCase {
    SwitchCase {
        tests: Vec::new(),
        body,
    }
}

/// `label: for (;;) { .. }`.
pub fn loop_forever(label: impl Into<String>, body: Vec<Stmt>) -> Stmt {
    Stmt::LoopForever(Some(label.into()), body)
}

/// `while (test) { .. }`.
pub fn while_(test: Expr, body: Vec<Stmt>) -> Stmt {
    Stmt::While(test, body)
}

/// `label: stmt`.
pub fn labeled(label: impl Into<String>, stmt: Stmt) -> Stmt {
    Stmt::Labeled(label.into(), Box::new(stmt))
}

/// `continue label;`.
pub fn continue_(label: impl Into<String>) -> Stmt {
    Stmt::Continue(Some(label.into()))
}

/// `break label;`.
pub fn break_(label: impl Into<String>) -> Stmt {
    Stmt::Break(Some(label.into()))
}

/// `throw value;`.
pub fn throw(value: Expr) -> Stmt {
    Stmt::Throw(value)
}

/// `throw new Error("message");`.
pub fn throw_error(message: impl Into<String>) -> Stmt {
    throw(new(id("Error"), vec![string(message)]))
}

/// An ordinary Error recorded as a compiler abort before it is thrown.
pub fn throw_abort(message: impl Into<String>) -> Stmt {
    throw(rt_call(
        "abort_error",
        vec![new(id("Error"), vec![string(message)])],
    ))
}

/// Wraps a `#[panic_handler]` (`LangItem::PanicImpl`) body so any exception escaping it is
/// registered via `__rt.abort_error` and rethrown as a non-unwinding compiler abort
/// (`CONTRACT.md`: "A Rust panic is still an abort").
///
/// Bodies whose statements are already unconditional literal aborts (`__rt.js_abort("...")` or
/// `throw __rt.abort_error(new Error("..."))`) are left unwrapped.
pub fn wrap_panic_impl_body(body: Vec<Stmt>) -> Vec<Stmt> {
    fn is_unconditional_abort(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::ExprStmt(Expr::Call(callee, args)) => {
                matches!(
                    callee.as_ref(),
                    Expr::Member(object, field)
                        if matches!(object.as_ref(), Expr::Ident(ns) if ns == "__rt")
                            && field == "js_abort"
                ) && args.len() == 1
                    && matches!(args[0], Expr::Str(_))
            }
            Stmt::Throw(Expr::Call(callee, args)) => {
                matches!(
                    callee.as_ref(),
                    Expr::Member(object, field)
                        if matches!(object.as_ref(), Expr::Ident(ns) if ns == "__rt")
                            && field == "abort_error"
                ) && args.len() == 1
                    && matches!(
                        &args[0],
                        Expr::New(callee, args)
                            if matches!(callee.as_ref(), Expr::Ident(name) if name == "Error")
                                && args.len() == 1
                                && matches!(args[0], Expr::Str(_))
                    )
            }
            Stmt::LoopForever(_, body) if body.is_empty() => true,
            _ => false,
        }
    }

    let meaningful: Vec<&Stmt> = body
        .iter()
        .filter(|stmt| !matches!(stmt, Stmt::Loc(_) | Stmt::Comment(_)))
        .collect();
    if !meaningful.is_empty() && meaningful.iter().all(|stmt| is_unconditional_abort(stmt)) {
        return body;
    }

    vec![Stmt::TryCatch {
        body,
        binding: "$e".to_owned(),
        handler: vec![throw(rt_call("abort_error", vec![id("$e")]))],
    }]
}

/// A top level function declaration.
pub fn function(name: impl Into<String>, params: Vec<String>, body: Vec<Stmt>) -> Stmt {
    Stmt::FunctionDecl {
        name: name.into(),
        params,
        body,
    }
}

/// `import * as name from "source";`.
pub fn import_namespace(name: impl Into<String>, source: impl Into<String>) -> Stmt {
    Stmt::Import {
        specifiers: ImportSpecifiers::Namespace(name.into()),
        source: source.into(),
    }
}

/// `import { exported as local, .. } from "source";`.
pub fn import_named(names: Vec<(String, String)>, source: impl Into<String>) -> Stmt {
    Stmt::Import {
        specifiers: ImportSpecifiers::Named(names),
        source: source.into(),
    }
}

/// A `//` line comment.
pub fn comment(text: impl Into<String>) -> Stmt {
    Stmt::Comment(text.into())
}

/// Verbatim JavaScript in statement position.
pub fn raw_stmt(text: impl Into<String>) -> Stmt {
    Stmt::Raw(text.into())
}

/// A source location marker, which prints nothing. See [`Stmt::Loc`].
pub fn loc(file: impl Into<String>, line: u32, col: u32) -> Stmt {
    Stmt::Loc(Loc {
        file: file.into(),
        line,
        col,
    })
}
