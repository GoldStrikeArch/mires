//! Floor port (Rust/mires): the boundary alone. `text` is read as a host string (`&str`, copied
//! into the Rust heap as UTF-8) and erased back to a host string.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_bindings, js_export, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

// A panic fails fast: it becomes a JavaScript exception naming the panicking file, instead of
// `loop {}`, which would hang the caller (and a check or timing cell until its watchdog).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let file = info.location().map_or("unknown location", |location| location.file());
    mires_js::throw_value(JsValue::erase("rust panic at ") + JsValue::erase(file))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    JsValue::erase(text)
}
