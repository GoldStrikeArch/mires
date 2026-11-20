//! mires bug repro (MIRES-019): promoted constant `Option<&u8>` (`Some(&b'.')`) silently decodes
//! to `{TAG:"None"}`, causing `s.as_bytes().last() == Some(&b'.')` to return `false` on `"...dog."`.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

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
fn ends_with_dot(s: &str) -> bool {
    s.as_bytes().last() == Some(&b'.')
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    // On floor's `ascii` case ("...dog."), `ends_with_dot` must be true; the bug makes it false!
    if text.len() > 5 && text.as_bytes()[text.len() - 1] == b'.' && !ends_with_dot(text) {
        return JsValue::erase("MISCOMPILED: Some(&b'.') decoded as None");
    }
    JsValue::erase(text)
}
