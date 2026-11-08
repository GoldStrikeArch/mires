//! Ordinary external functions whose callers do not need their MIR.

#![no_std]
#![deny(unsafe_code)]

use mires_js::{js_bindings, JsValue};

js_bindings! {
    #[js(call = "callbackProbe")]
    fn probe(label: &str, first: JsValue, second: JsValue) -> JsValue;

    #[js(global, get = "callbackFailure")]
    fn failure() -> JsValue;
}

#[inline(never)]
pub fn zero() -> JsValue {
    probe("zero", JsValue::erase(7u32), JsValue::undefined())
}

#[inline(never)]
pub fn one(value: JsValue) -> JsValue {
    probe("one", value, JsValue::undefined())
}

#[inline(never)]
pub fn two(first: JsValue, second: JsValue) -> JsValue {
    probe("two", first, second)
}

#[inline(never)]
pub fn throwing() -> JsValue {
    mires_js::throw_value(failure())
}

pub fn generic<const N: u32>() -> JsValue {
    JsValue::erase(N)
}
