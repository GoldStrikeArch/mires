//! marked 18.1.1 in Rust, compiled to JavaScript by mires: a faithful port of the CommonMark,
//! GFM, and pedantic Markdown-to-HTML compiler subset defined in `libraries/marked/CONTRACT.md`.
//!
//! `matrix_run` is re-exported as `run` by the builder's facade.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod block_rules;
mod boundary;
mod helpers;
mod inline_rules;
mod lexer;
mod renderer;
mod tokens;
mod unicode;

use mires_js::{js_export, throw_value, JsValue};

/// Panic handler fails fast as a JS exception instead of hanging the caller.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("marked (Rust port): panic"))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    boundary::run(input)
}
