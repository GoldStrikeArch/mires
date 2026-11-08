//! Three-argument host callbacks preserve captures and opaque values.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{callback3, js_bindings, js_export, throw_value, JsValue};
js_bindings! {
    #[js(new = "Array")]
    fn array() -> JsValue;
    #[js(method = "push")]
    fn push(array: JsValue, value: JsValue);
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("callback fixture panic"))
}
#[js_export]
pub fn make(seed: JsValue) -> JsValue {
    callback3(move |first, second, third| {
        let values = array();
        for value in [seed, first, second, third] { push(values, value); }
        values
    })
}
#[js_export]
pub fn make_many() -> JsValue {
    let values = array();
    for index in 0..4_u32 { push(values, make(JsValue::erase(index))); }
    values
}
