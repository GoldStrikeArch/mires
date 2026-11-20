//! fflate 0.8.3 in Rust, compiled to JavaScript by mires: a faithful port of the synchronous
//! compression, decompression, framing, streaming, and text-codec subset defined in
//! `libraries/fflate/CONTRACT.md`.
//!
//! `matrix_run` is re-exported as `run` by the builder's facade.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod boundary;
mod codec;
mod deflate;
mod framing;
mod huffman;
mod inflate;
mod stream;
mod tables;

use mires_js::{js_export, throw_value, JsValue};

/// Panic handler fails fast as a JS exception instead of hanging the caller.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("fflate (Rust port): panic"))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    boundary::run(input)
}
