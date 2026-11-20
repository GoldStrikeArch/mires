//! earcut 3.2.4 in Rust, compiled to JavaScript by mires: a faithful port of the complete
//! public API (`earcut`, `deviation`, `flatten`, `refine`) and contract pipelines (`earcutRefine`,
//! `batch`) defined in `libraries/earcut/CONTRACT.md`.
//!
//! `matrix_run` is re-exported as `run` by the builder's facade.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod boundary;
mod earcut;
mod refine;
mod util;

use mires_js::{js_export, throw_value, JsValue};

/// Panic handler fails fast as a JS exception instead of hanging the caller.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("earcut (Rust port): panic"))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    boundary::run(input)
}
