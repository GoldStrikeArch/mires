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

mod convert;
mod opt;
#[cfg(test)]
mod tests;

use crate::jir::types::{TypeId, TypeTable};
use crate::jsast::{self, Expr};
use crate::opts::StrRepr;

use super::js::value;

pub use convert::{
    host_convert, host_convert_carrier, reaches_str, synthesize_converters, Crossing,
};
pub use opt::{leftover_helper, rewrite};

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

/// Whether `opts` holds a `str` as a byte string.
pub fn enabled_for(opts: &crate::opts::Opts) -> bool {
    opts.str_repr == StrRepr::Bytes
}

/// Whether this link step holds a `str` as a byte string.
pub fn enabled() -> bool {
    enabled_for(crate::opts::get())
}

/// The byte string of `text`: one `char` per UTF-8 byte, each below U+0100.
pub fn byte_string(text: &str) -> String {
    text.bytes().map(char::from).collect()
}

/// The Rust length of a `str` literal printed under `bytes` mode (`js-str=bytes`).
pub fn literal_len_for(literal: &str, bytes: bool) -> usize {
    if bytes {
        literal.chars().count()
    } else {
        literal.len()
    }
}

/// The Rust length of a `str` literal the lowering printed as `literal`.
pub fn literal_len(literal: &str) -> usize {
    literal_len_for(literal, enabled())
}

/// A Rust `str` literal under `bytes` mode: the text itself, or its byte string.
pub fn literal_for(text: &str, bytes: bool) -> Expr {
    if bytes {
        jsast::string(byte_string(text))
    } else {
        jsast::string(text)
    }
}

/// A Rust `str` literal: the text itself, or its byte string.
pub fn literal(text: &str) -> Expr {
    literal_for(text, enabled())
}

/// The Rust text of one `char` code point under `bytes` mode: UTF-16 or UTF-8 byte string.
pub fn char_text_for(point: Expr, bytes: bool) -> Expr {
    if bytes {
        jsast::rt_call("bstr_char", vec![point])
    } else {
        jsast::method_call(jsast::id("String"), "fromCodePoint", vec![point])
    }
}

/// Whether `ty` is a shared `&str` reference: a value the run time holds as the JavaScript string
/// itself. Mutable `&mut str` and raw `str` pointers (`*const str`, `*mut str`, `NonNull<str>`,
/// `Unique<str>`, `Box<str>`) are live `{ buf, off, len }` UTF-8 byte slices
/// (`PointerForm::FatSlice`), identical in `utf16` and `bytes` modes.
pub fn is_str_pointer(table: &TypeTable, ty: TypeId) -> bool {
    value::is_const_str_ptr(table, ty)
}
