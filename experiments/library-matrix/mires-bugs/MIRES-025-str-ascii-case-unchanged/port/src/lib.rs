//! mires bug repro (MIRES-025): `str::to_ascii_lowercase` and `String::make_ascii_uppercase` (and
//! the other two `str` ASCII case conversions) return or leave the text unchanged. They change it
//! through `&mut str`; the same conversion on `[u8]`, `Vec<u8>` or `u8` works.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
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
fn ascii_lowercase(text: &str) -> String {
    text.to_ascii_lowercase()
}

#[inline(never)]
fn ascii_uppercase_in_place(text: &str) -> String {
    let mut owned = String::from(text);
    owned.make_ascii_uppercase();
    owned
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    // The expected results, one byte at a time.
    let lowercase: Vec<u8> = text.bytes().map(|byte| byte.to_ascii_lowercase()).collect();
    let uppercase: Vec<u8> = text.bytes().map(|byte| byte.to_ascii_uppercase()).collect();
    if ascii_lowercase(text).as_bytes() != lowercase.as_slice() {
        return JsValue::erase("MISCOMPILED: str::to_ascii_lowercase returned the text unchanged");
    }
    if ascii_uppercase_in_place(text).as_bytes() != uppercase.as_slice() {
        return JsValue::erase("MISCOMPILED: String::make_ascii_uppercase left the text unchanged");
    }
    JsValue::erase(text)
}
