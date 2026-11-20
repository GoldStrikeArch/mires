//! mires bug repro (MIRES-018): `str::is_ascii()` / `[u8]::is_ascii()` throws
//! `byte offset 1 cannot form a u32 scalar view` on sliced/unaligned byte strings.

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
fn check_tail_ascii(s: &str) -> bool {
    if s.len() > 1 && s.is_char_boundary(1) {
        s[1..].is_ascii()
    } else {
        true
    }
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let _ = check_tail_ascii(text);
    JsValue::erase(text)
}
