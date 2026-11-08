//! An owning Rust value cannot be fabricated from or erased into an opaque host value.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use mires_js::{js_export, JsValue};

#[js_export]
pub fn rust_entry() {
    let _ = JsValue::erase(String::new());
    let _: String = JsValue::undefined().read();
}
