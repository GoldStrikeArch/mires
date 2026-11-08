//! Locals, parameters and generated temporaries keep declared host globals in scope.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{JsValue, js_bindings, js_export, try_host};
js_bindings! {
    #[js(global, get = "fixture_global")]
    fn read_global() -> JsValue;
    #[js(global, get = "_0")]
    fn read_numbered() -> JsValue;
    #[js(global, get = "a")]
    fn read_short() -> JsValue;
    #[js(call = "$t0")]
    fn call_temporary(value: JsValue) -> JsValue;
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    mires_js::throw_value(JsValue::erase("global shadow fixture panic"))
}
#[js_export]
pub fn matching_local() -> JsValue {
    let fixture_global = read_global();
    if fixture_global.is_truthy() {
        fixture_global
    } else {
        JsValue::null()
    }
}
#[js_export]
pub fn matching_parameter(fixture_global: JsValue) -> bool {
    read_global().strictly_equals(fixture_global)
}
#[js_export]
pub fn numbered() -> JsValue {
    read_numbered()
}
#[js_export]
pub fn short() -> JsValue {
    let a = read_short();
    if a.is_truthy() { a } else { JsValue::null() }
}
#[js_export]
pub fn temporary(value: JsValue) -> JsValue {
    call_temporary(value)
}
#[inline(never)]
fn helper() -> JsValue {
    read_global()
}
#[js_export]
pub fn through_helper(fixture_global: JsValue) -> JsValue {
    let result = helper();
    if fixture_global.is_null() {
        fixture_global
    } else {
        result
    }
}
#[js_export]
pub fn unused() -> f64 {
    let _fixture_global = read_global();
    17.0
}
#[js_export]
pub fn caught() -> JsValue {
    try_host(|| {
        let fixture_global = read_global();
        if fixture_global.is_truthy() {
            fixture_global
        } else {
            JsValue::null()
        }
    })
    .unwrap_or_else(|error| error)
}
