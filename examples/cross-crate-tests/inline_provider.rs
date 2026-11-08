#![no_std]
#![deny(unsafe_code)]

use mires_js::JsValue;

#[inline(never)]
pub fn offset(value: u32) -> u32 {
    value.wrapping_add(7)
}

#[inline(never)]
pub fn alternative(value: u32) -> u32 {
    value.wrapping_mul(3)
}

#[inline(never)]
pub fn label() -> &'static str {
    "provider constant"
}

#[inline(never)]
pub fn copied_pair(value: u32) -> u32 {
    let pair = (value, value.wrapping_add(1));
    let copy = pair;
    pair.0.wrapping_add(copy.1)
}

#[inline(never)]
pub fn unequal(pair: (u32, u32)) -> bool {
    pair.0 != pair.1
}

#[inline(never)]
pub fn host_null(value: JsValue) -> bool {
    value.is_null()
}

#[inline(never)]
pub fn host_call(function: JsValue, receiver: JsValue, argument: JsValue) -> JsValue {
    mires_js::invoke(function, receiver, &[argument])
}
