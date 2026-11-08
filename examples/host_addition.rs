//! Native JavaScript addition through the opaque host value type.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{JsValue, invoke, js_export, try_host};
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    mires_js::throw_value(JsValue::erase("host addition fixture panic"))
}
#[js_export]
pub fn add(left: JsValue, right: JsValue) -> JsValue {
    left + right
}
#[js_export]
pub fn catch_add(left: JsValue, right: JsValue) -> JsValue {
    try_host(|| left + right).unwrap_or_else(|error| error)
}
#[js_export]
pub fn unused(left: JsValue, right: JsValue) -> JsValue {
    let _ = left + right;
    JsValue::undefined()
}
#[js_export]
pub fn evaluated(left: JsValue, right: JsValue) -> JsValue {
    let first = invoke(left, JsValue::undefined(), &[]);
    let second = invoke(right, JsValue::undefined(), &[]);
    first + second
}
#[js_export]
pub fn first_class(left: JsValue, right: JsValue) -> JsValue {
    let add: fn(JsValue, JsValue) -> JsValue = <JsValue as core::ops::Add>::add;
    add(left, right)
}
