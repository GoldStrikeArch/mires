//! Function items and closures share the typed host callback boundary.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{callback0, callback1, callback2, js_export, try_host, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[js_export]
pub fn external_zero() -> JsValue {
    callback0(callback_library::zero)
}

#[js_export]
pub fn external_one() -> JsValue {
    callback1(callback_library::one)
}

#[js_export]
pub fn external_two() -> JsValue {
    callback2(callback_library::two)
}

#[js_export]
pub fn external_throwing() -> JsValue {
    callback0(callback_library::throwing)
}

#[js_export]
pub fn caught_external() -> JsValue {
    match try_host(callback_library::throwing) {
        Ok(_) => JsValue::undefined(),
        Err(error) => error,
    }
}

#[js_export]
pub fn successful_external() -> JsValue {
    match try_host(callback_library::zero) {
        Ok(value) => value,
        Err(_) => JsValue::undefined(),
    }
}

#[js_export]
pub fn generic_external() -> JsValue {
    callback0(callback_library::generic::<17>)
}

fn local(value: JsValue) -> JsValue {
    value
}

#[js_export]
pub fn local_item() -> JsValue {
    callback1(local)
}

#[js_export]
pub fn captured(value: JsValue) -> JsValue {
    callback0(move || value)
}
