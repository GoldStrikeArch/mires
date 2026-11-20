//! mires bug repro: `<(String, usize) as Clone>::clone` writes into an uninitialized local.
//!
//! The floor library's identity contract: the text must come back unchanged. The text goes
//! through a cloned `(String, usize)` tuple.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
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
fn duplicate(pair: &(String, usize)) -> (String, usize) {
    pair.clone()
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let pair = (String::from(text), text.len());
    let copy = duplicate(&pair);
    JsValue::erase(copy.0.as_str())
}
