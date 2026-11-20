//! mires bug repro: `Vec::into_iter().map(..).collect::<Vec<_>>()` is refused when the standard
//! library's in-place collection applies (the source allocation is reused for the result).
//!
//! The floor library's identity contract; the text passes through `(u32, usize)` pairs collected
//! back into `usize` positions, the pattern `text::property_order` used.

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
fn second_halves(pairs: Vec<(u32, usize)>) -> Vec<usize> {
    pairs.into_iter().map(|(_, position)| position).collect()
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let pairs: Vec<(u32, usize)> = text.char_indices().map(|(at, character)| (character as u32, at)).collect();
    let positions = second_halves(pairs);
    let _ = positions.len();
    JsValue::erase(text)
}
