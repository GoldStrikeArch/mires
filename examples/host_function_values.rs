//! Declared host operations retain their behavior when used as Rust function values.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{js_bindings, js_export, try_host, JsValue};

js_bindings! {
    #[js(global, get = "fixtureValue")]
    fn value() -> JsValue;
    #[js(set = "value")]
    fn set_value(receiver: JsValue, value: JsValue);
    #[js(method = "read")]
    fn read(receiver: JsValue, argument: JsValue) -> JsValue;
    #[js(index)]
    fn at(receiver: JsValue, key: JsValue) -> JsValue;
    #[js(new = "FixtureConstructor")]
    fn make(value: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    mires_js::throw_value(JsValue::erase("host binding fixture panic"))
}

#[js_export]
pub fn get_value() -> JsValue {
    try_host(value).unwrap_or_else(|error| error)
}

#[js_export]
pub fn put_value(receiver: JsValue, value: JsValue) -> JsValue {
    let setter: fn(JsValue, JsValue) = set_value;
    try_host(move || {
        setter(receiver, value);
        value
    })
    .unwrap_or_else(|error| error)
}

#[js_export]
pub fn method(receiver: JsValue, argument: JsValue) -> JsValue {
    let method: fn(JsValue, JsValue) -> JsValue = read;
    try_host(move || method(receiver, argument)).unwrap_or_else(|error| error)
}

#[js_export]
pub fn element(receiver: JsValue, key: JsValue) -> JsValue {
    let getter: fn(JsValue, JsValue) -> JsValue = at;
    try_host(move || getter(receiver, key)).unwrap_or_else(|error| error)
}

#[js_export]
pub fn construct(value: JsValue) -> JsValue {
    let factory: fn(JsValue) -> JsValue = make;
    try_host(move || factory(value)).unwrap_or_else(|error| error)
}
