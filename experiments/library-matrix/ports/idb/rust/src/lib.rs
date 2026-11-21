//! idb 8.0.4 in Rust: `idb` is the library (openDB, deleteDB, wrap, unwrap, transactions with
//! `done`, promisified store, index and cursor methods, the database shortcuts and `iterate`);
//! `session` plays a corpus script through it, as `libraries/idb/reference.mjs` drives npm idb.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod host;
mod idb;
mod script;
mod session;

use mires_js::{future_to_promise, js_export, throw_value, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("idb port panicked"))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    future_to_promise(session::run(input))
}
