//! Byte strings: the `js-str=bytes` representation of a Rust `str` (experimental).
//!
//! Under `-Cllvm-args=js-str=bytes` a Rust `str` is a JavaScript string whose code unit `i` is
//! byte `i` of its UTF-8 text, so every unit is below 256 and `length` is the Rust length (the
//! way js_of_ocaml holds an OCaml string). The default, `utf16`, is CONTRACT.md's hybrid, and
//! nothing in this module runs under it: every entry point here is reached only behind
//! [`enabled`], which is what keeps the default output byte-identical.
//!
//! The lowering is written once, against the hybrid's runtime helpers (`__rt.str_bytes`,
//! `__rt.bytes_str`, `__rt.str_len`, the native `String` helpers), and so are all the peepholes
//! and purity tables that recognize them by name. A byte string differs from a UTF-16 string in
//! three places only, and this module owns all three:
//!
//! * **what each helper means** — [`rewrite`] swaps every hybrid helper the installed
//!   declaration calls for its byte-string twin in `runtime/shim.js` (a copy of code units where
//!   the hybrid encodes or decodes), turns `__rt.str_len(s)` into `s.length` and drops
//!   `__rt.native_str_wf`, which is the identity on a byte string;
//! * **what a literal is** — [`byte_string`] is the text of a `&'static str` constant as the
//!   JavaScript string of its bytes, and the printer writes its non-ASCII units as `\xNN`;
//! * **where Rust text meets the host** — `__rt.bstr_in` (host text becoming a Rust `str`: UTF-8
//!   encode, lone surrogate to U+FFFD) and `__rt.bstr_out` (the way back: decode), wrapped by
//!   [`host_in`] and [`host_out`] around each crossing the lowering knows by type.
//!
//! [`leftover_helper`] is the backstop: the link step refuses a program in which an item still
//! calls a hybrid helper after the rewrite (a verbatim `js!` fragment, say), rather than mixing
//! the two representations.

use std::collections::BTreeSet;

use crate::jir::types::{TypeId, TypeKind, TypeTable};
use crate::jsast::{self, ArrowBody, Expr, Stmt};
use crate::opts::StrRepr;

use super::js::value;

/// Whether this link step holds a `str` as a byte string.
pub fn enabled() -> bool {
    crate::opts::get().str_repr == StrRepr::Bytes
}

/// The byte string of `text`: one `char` per UTF-8 byte, each below U+0100.
pub fn byte_string(text: &str) -> String {
    text.bytes().map(char::from).collect()
}

/// The Rust length of a `str` literal the lowering printed as `literal`.
pub fn literal_len(literal: &str) -> usize {
    if enabled() {
        literal.chars().count()
    } else {
        literal.len()
    }
}

/// A Rust `str` literal: the text itself, or its byte string.
pub fn literal(text: &str) -> Expr {
    if enabled() {
        jsast::string(byte_string(text))
    } else {
        jsast::string(text)
    }
}

/// The Rust text of one `char` code point: its UTF-16 string, or its UTF-8 byte string.
pub fn char_text(point: Expr) -> Expr {
    if enabled() {
        jsast::rt_call("bstr_char", vec![point])
    } else {
        jsast::method_call(jsast::id("String"), "fromCodePoint", vec![point])
    }
}

/// Whether `ty` is a pointer to `str` (`&str`, `&mut str`, `*const str`): a value the run time
/// holds as the JavaScript string itself.
pub fn is_str_pointer(table: &TypeTable, ty: TypeId) -> bool {
    value::pointee_of(table, ty).is_some_and(|pointee| value::is_str(table, pointee))
}

/// Whether a value of type `ty` holds a `str` pointer in its own JavaScript value: it is one, or
/// a field or element held by value is. What an export signature is checked against; a pointer
/// to anything but a `str` (the panic handler's `&PanicInfo`) is a handle the host does not read.
pub fn holds_str(table: &TypeTable, ty: TypeId) -> bool {
    fn walk(table: &TypeTable, ty: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
        let ty = table.peeled(ty);
        if !seen.insert(ty) {
            return false;
        }
        let Some(descriptor) = table.get(ty) else {
            return false;
        };
        match &descriptor.kind {
            TypeKind::Str => true,
            TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                value::is_str(table, *pointee)
            }
            TypeKind::Tuple(elements) => elements.iter().any(|e| walk(table, *e, seen)),
            TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                walk(table, *element, seen)
            }
            TypeKind::Adt { .. } => table
                .variants(ty)
                .iter()
                .any(|variant| variant.fields.iter().any(|f| walk(table, f.ty, seen))),
            _ => false,
        }
    }
    walk(table, ty, &mut BTreeSet::new())
}

/// `__rt.bstr_in(value)`: host text as the Rust `str` it becomes.
pub fn host_in(value: Expr) -> Expr {
    match value {
        // A literal the lowering spelled itself is already a byte string.
        Expr::Str(_) => value,
        value => jsast::rt_call("bstr_in", vec![value]),
    }
}

/// `__rt.bstr_out(value)`: a Rust `str` as the host text it encodes.
pub fn host_out(value: Expr) -> Expr {
    match value {
        Expr::Str(text) if text.is_ascii() => Expr::Str(text),
        value => jsast::rt_call("bstr_out", vec![value]),
    }
}

/// The byte-string twin of a hybrid helper, or `None` when `member` is not one.
fn twin(member: &str) -> Option<&'static str> {
    Some(match member {
        "str_bytes" => "bstr_bytes",
        "str_bytes_mut" => "bstr_bytes_mut",
        "bytes_str" => "bstr_of",
        "native_str_push" => "bstr_push",
        "native_str_pop" => "bstr_pop",
        "native_str_truncate" => "bstr_truncate",
        "native_str_is_char_boundary" => "bstr_boundary",
        "native_str_utf16_index" | "native_str_byte_index" => "bstr_index",
        "convert_host_sb" => "bstr_host_sb",
        "convert_str_block" => "bstr_block",
        "convert_sb_block" => "bstr_sb_block",
        "_sb_check" => "bstr_sb_check",
        _ => return None,
    })
}

/// The hybrid helpers no byte-string program may call once [`rewrite`] has run.
const HYBRID_HELPERS: &[&str] = &[
    "str_bytes",
    "str_bytes_mut",
    "bytes_str",
    "str_len",
    "native_str_wf",
    "native_str_push",
    "native_str_pop",
    "native_str_truncate",
    "native_str_is_char_boundary",
    "native_str_utf16_index",
    "native_str_byte_index",
    "convert_host_sb",
    "convert_str_block",
    "convert_sb_block",
    "_sb_check",
];

/// The first hybrid helper `js` still names, if any.
pub fn leftover_helper(js: &str) -> Option<&'static str> {
    HYBRID_HELPERS.iter().copied().find(|helper| {
        let needle = format!("__rt.{helper}");
        js.match_indices(&needle).any(|(at, _)| {
            let next = js[at + needle.len()..].chars().next();
            !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        })
    })
}

/// The runtime member a call's callee names: `__rt.<member>`.
fn rt_member(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Member(object, member) if matches!(&**object, Expr::Ident(rt) if rt == "__rt") => {
            Some(member)
        }
        _ => None,
    }
}

/// Rewrites one installed declaration from the hybrid helpers to the byte-string ones.
pub fn rewrite(stmt: &mut Stmt) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => rewrite_expr(e),
        Stmt::Let(_, init) | Stmt::Return(init) => {
            if let Some(e) = init {
                rewrite_expr(e);
            }
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    rewrite_expr(e);
                }
            }
        }
        Stmt::If(test, then, otherwise) => {
            rewrite_expr(test);
            rewrite_all(then);
            if let Some(otherwise) = otherwise {
                rewrite_all(otherwise);
            }
        }
        Stmt::Switch(disc, cases) => {
            rewrite_expr(disc);
            for case in cases {
                for test in &mut case.tests {
                    rewrite_expr(test);
                }
                rewrite_all(&mut case.body);
            }
        }
        Stmt::Labeled(_, inner) => rewrite(inner),
        Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::FunctionDecl { body, .. } => {
            rewrite_all(body)
        }
        Stmt::While(test, body) => {
            rewrite_expr(test);
            rewrite_all(body);
        }
        Stmt::TryCatch { body, handler, .. } => {
            rewrite_all(body);
            rewrite_all(handler);
        }
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => {}
    }
}

fn rewrite_all(stmts: &mut [Stmt]) {
    for stmt in stmts {
        rewrite(stmt);
    }
}

fn rewrite_expr(expr: &mut Expr) {
    match expr {
        Expr::Member(object, _) | Expr::Unary(_, object) => rewrite_expr(object),
        Expr::Index(object, index) => {
            rewrite_expr(object);
            rewrite_expr(index);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            rewrite_expr(callee);
            for arg in args.iter_mut() {
                rewrite_expr(arg);
            }
        }
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            rewrite_expr(l);
            rewrite_expr(r);
        }
        Expr::Cond(t, c, a) => {
            rewrite_expr(t);
            rewrite_expr(c);
            rewrite_expr(a);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                rewrite_expr(value);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                rewrite_expr(item);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(body)) => rewrite_expr(body),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => rewrite_all(stmts),
        _ => {}
    }
    // Children first, so a helper's argument is already a byte-string expression.
    let Expr::Call(callee, args) = expr else {
        return;
    };
    let Some(member) = rt_member(callee) else {
        return;
    };
    match (member, args.len()) {
        ("str_len", 1) => {
            let string = args.pop().expect("one argument");
            *expr = match string {
                Expr::Str(text) => jsast::num(text.chars().count() as f64),
                string => jsast::member(string, "length"),
            };
        }
        ("native_str_wf", 1) => *expr = args.pop().expect("one argument"),
        (member, _) => {
            if let Some(twin) = twin(member) {
                **callee = jsast::member(jsast::id("__rt"), twin);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(member: &str, args: Vec<Expr>) -> Expr {
        jsast::rt_call(member, args)
    }

    #[test]
    fn a_literal_is_the_string_of_its_bytes() {
        assert_eq!(byte_string("aé😀"), "a\u{c3}\u{a9}\u{f0}\u{9f}\u{98}\u{80}");
    }

    #[test]
    fn the_rewrite_swaps_helpers_and_folds_lengths() {
        let mut stmt = jsast::ret(jsast::binary(
            jsast::BinOp::Add,
            rt(
                "str_len",
                vec![rt(
                    "bytes_str",
                    vec![jsast::id("b"), jsast::num(0), jsast::num(2)],
                )],
            ),
            rt("str_len", vec![rt("native_str_wf", vec![jsast::id("s")])]),
        ));
        rewrite(&mut stmt);
        let printed = jsast::program_to_string(std::slice::from_ref(&stmt));
        assert!(
            printed.contains("__rt.bstr_of(b, 0, 2).length"),
            "{printed}"
        );
        assert!(printed.contains("s.length"), "{printed}");
        assert_eq!(leftover_helper(&printed), None, "{printed}");
    }

    #[test]
    fn leftovers_are_found_by_whole_name() {
        assert_eq!(leftover_helper("__rt.str_bytes(s)"), Some("str_bytes"));
        assert_eq!(
            leftover_helper("__rt.str_bytes_mut(s)"),
            Some("str_bytes_mut")
        );
        assert_eq!(leftover_helper("__rt.bstr_bytes(s); __rt.str_lenx"), None);
    }
}
