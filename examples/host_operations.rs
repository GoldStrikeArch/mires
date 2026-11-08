//! Dynamic host markers preserve ordered arguments and indirect Rust callers.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{construct, invoke, js_bindings, js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "hostRead")]
    fn read(index: u32) -> JsValue;

    #[js(get = "method")]
    fn method(target: JsValue) -> JsValue;

    #[js(method = "method")]
    fn method_call(target: JsValue, argument: JsValue) -> JsValue;

    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;

    #[js(call = "Reflect.set")]
    fn reflect_set(target: JsValue, key: JsValue, value: JsValue) -> bool;

    #[js(new = "hostConstructors.Constructor")]
    fn native_constructor(value: JsValue) -> JsValue;

    #[js(global, get = "Uint8Array")]
    fn byte_constructor() -> JsValue;

    #[js(get = "prototype")]
    fn prototype(value: JsValue) -> JsValue;

    #[js(call = "Object.getPrototypeOf")]
    fn prototype_of(value: JsValue) -> JsValue;

    #[js(get = "left")]
    fn left(value: JsValue) -> JsValue;

    #[js(get = "right")]
    fn right(value: JsValue) -> JsValue;

    #[js(call = "hostRecord")]
    fn record(left: JsValue, right: JsValue) -> JsValue;
}

#[js_export]
pub fn global_argument() -> JsValue {
    prototype_of(prototype(byte_constructor()))
}

#[js_export]
pub fn discard_global() {
    let _ = byte_constructor();
}

#[js_export]
pub fn ordered_fields(value: JsValue) -> JsValue {
    record(left(value), right(value))
}

#[js_export]
pub fn discard_field(value: JsValue) {
    let _ = left(value);
}

#[js_export]
pub fn object() -> bool {
    read(0).is_object()
}

#[js_export]
pub fn call() -> JsValue {
    invoke(read(0), read(1), &[read(2), read(3)])
}

#[js_export]
pub fn create() -> JsValue {
    construct(read(0), &[read(1), read(2)])
}

#[js_export]
pub fn empty_call() -> JsValue {
    invoke(read(0), read(1), &[])
}

#[js_export]
pub fn empty_create() -> JsValue {
    construct(read(0), &[])
}

#[js_export]
pub fn dynamic(function: JsValue, this: JsValue, arguments: &[JsValue]) -> JsValue {
    invoke(function, this, arguments)
}

#[js_export]
pub fn retrieved() -> JsValue {
    invoke(method(read(0)), read(1), &[read(2)])
}

#[js_export]
pub fn member() -> JsValue {
    method_call(read(0), read(1))
}

#[js_export]
pub fn get() -> JsValue {
    reflect_get(read(0), read(1))
}

#[js_export]
pub fn set() -> bool {
    reflect_set(read(0), read(1), read(2))
}

#[js_export]
pub fn native_create() -> JsValue {
    native_constructor(read(0))
}

#[js_export]
pub fn indirect(value: JsValue, object: bool) -> bool {
    let predicate: fn(JsValue) -> bool = if object {
        JsValue::is_object
    } else {
        JsValue::is_function
    };
    predicate(value)
}

#[js_export]
pub fn predicates(value: JsValue) -> u32 {
    u32::from(value.is_undefined())
        | (u32::from(value.is_null()) << 1)
        | (u32::from(value.is_truthy()) << 2)
        | (u32::from(value.is_boolean()) << 3)
        | (u32::from(value.is_number()) << 4)
        | (u32::from(value.is_string()) << 5)
        | (u32::from(value.is_function()) << 6)
        | (u32::from(value.is_object()) << 7)
        | (u32::from(value.strictly_equals(value)) << 8)
}
