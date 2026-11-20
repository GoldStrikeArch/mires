//! mires bug repro (MIRES-021): `vec![0u8; n]` lowers to `__rt.alloc_zeroed`, which builds a
//! boxed JS `Array` with `new Array(n).fill(0)`. Past 2^27 elements V8 refuses that array, so a
//! 129 MiB zeroed buffer throws `RangeError: Invalid array length`. Natively, and as a JS
//! `Uint8Array`, the same allocation succeeds.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

/// 129 MiB: above V8's 2^27-element limit for the fast array that `fill(0)` builds.
const SIZE: usize = 129 << 20;

/// Allocates a zeroed buffer of `n` bytes and reads its last byte (0 on every correct target).
#[inline(never)]
fn zeroed_last(n: usize) -> u8 {
    let buf = vec![0u8; n];
    buf[n - 1]
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    if zeroed_last(SIZE) != 0 {
        return JsValue::erase("MISCOMPILED: a zeroed Vec<u8> read back non-zero");
    }
    JsValue::erase(text)
}
