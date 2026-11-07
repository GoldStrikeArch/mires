//! `js!{}`: a JavaScript expression written in Rust source, with the Rust bindings it names
//! captured.
//!
//! Compiled against the real `core` (`21_js_block.core`) because the expansion names
//! `::core::unreachable!`, and with `js-macro` built for the host (`21_js_block.macro`).
//!
//! What each entry pins:
//!
//! * a block with no captures at all, which needs no arrow and gets none;
//! * a block whose captures are numbers, so the emitted arrow's parameters are what the slots
//!   became and the arguments are what Rust supplied;
//! * the case the macro exists for: an object literal with a key Rust cannot spell as a field
//!   name. `content-type` is not a Rust identifier and `#[repr(C)]` keys an object by FIELD NAME,
//!   so a declared interface cannot build this object and a host function had to stand in for it;
//! * a capture written twice, which is one slot and one argument;
//! * the object literal shorthand, which is a reference and so a capture, written out in full so
//!   the key keeps its own name;
//! * an arrow inside the block, whose parameter is bound by the block and is therefore NOT a
//!   capture -- the case that makes the analysis need scopes rather than a set of names.

#![no_std]
#![no_main]

#[path = "../core-tests/prelude.rs"]
mod prelude;
use prelude::*;

use js_macro::js;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_i32(js! { 1 + 1 });

    let a = 6;
    let b = 7;
    print_i32(js! { a * b });

    // The motivating shape: an init object with a header name Rust cannot spell.
    let kind = "application/topcoat+json";
    let body = "[\"world\"]";
    let init: &str = js! {
        ({ method: "POST", headers: { "content-type": kind }, body }).headers["content-type"]
    };
    print_str(init);
    print_str(js! { ({ method: "POST", headers: { "content-type": kind }, body }).body });

    // One capture, named twice: one slot, one argument.
    print_i32(js! { a + a });

    // The arrow's parameter is bound by the block, so only `kind` crosses.
    print_str(js! { ((s) => s.slice(0, 11))(kind) });

    // A block written verbatim, which is the only way to reach a template literal: Rust's own
    // lexer refuses a backtick before this macro is ever called.
    print_str(js! { r#"`${a} of ${b}`"# });
}
