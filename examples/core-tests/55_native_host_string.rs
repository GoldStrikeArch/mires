//! A native `String` passed to `js_bindings!` as `&str` is a host sink, not a pin.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use mires_js::{js_bindings, js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "console.log")]
    fn log(label: &str);

    #[js(call = "console.log")]
    fn log_value(label: &str, value: JsValue);
}

#[js_export]
pub fn native_host_string() -> usize {
    let mut value = String::new();
    value.push_str("hello");
    log(&value);
    value.len()
}

#[js_export]
pub fn mixed_host_value() -> usize {
    let value = String::new();
    log_value("mixed", JsValue::erase(true));
    value.len()
}

#[js_export]
pub fn rust_entry() {}
