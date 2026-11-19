//! Rust port of the pilot library, compiled to JavaScript by mires.
//!
//! The boundary: `matrix_run` (re-exported as `run` by the builder's facade) receives the corpus input as a host object, reads `text` as a host string
//! (`&str`, converted to the Rust heap's UTF-8) and `top` as an integer, runs the pure Rust
//! analysis in `pilot`, and hands the resulting `String` back as a host string.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod pilot;

use mires_js::{js_bindings, js_export, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

// A panic fails fast: it becomes a JavaScript exception naming the panicking file, instead of
// `loop {}`, which would hang the caller (and a check or timing cell until its watchdog).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let file = info.location().map_or("unknown location", |location| location.file());
    mires_js::throw_value(JsValue::erase("rust panic at ") + JsValue::erase(file))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text = reflect_get(input, JsValue::erase("text"));
    let top = reflect_get(input, JsValue::erase("top"));
    let report = pilot::analyse(text.read::<&str>(), top.read::<usize>());
    JsValue::erase(report.as_str())
}
