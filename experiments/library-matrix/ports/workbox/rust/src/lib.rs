//! workbox-precaching 7.4.1, with the workbox-core, workbox-routing and workbox-strategies parts
//! it uses, in Rust (CONTRACT.md "API subset"), and the adapter that drives it.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod host;
mod precaching;
mod routing;
mod scenario;
mod strategies;
mod workbox_core;

use mires_js::{JsValue, js_export, throw_value};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("Workbox port panicked"))
}

/// Runs one corpus script and resolves with its output.
#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    scenario::run(input)
}
