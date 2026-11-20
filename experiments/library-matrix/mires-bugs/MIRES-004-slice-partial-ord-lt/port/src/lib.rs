//! mires bug repro: `<` on slices (`<[T] as PartialOrd>::lt`) is refused: core implements it with
//! a `transmute` of `ControlFlow<bool>` to `u8` (core/src/slice/cmp.rs). `sort_unstable` on a
//! `Vec<&[u16]>` reaches it, which is how the minisearch port met it.
//!
//! The floor library's identity contract; the text is compared with itself as UTF-16.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

#[inline(never)]
fn less(a: &[u16], b: &[u16]) -> bool {
    a < b
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let units: Vec<u16> = text.encode_utf16().collect();
    if less(&units, &units) {
        throw_value(JsValue::erase("a slice is less than itself"));
    }
    JsValue::erase(text)
}
