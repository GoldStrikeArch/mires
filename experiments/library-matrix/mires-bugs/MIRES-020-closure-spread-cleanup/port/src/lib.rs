//! mires bug repro (MIRES-020): a capturing closure called with a constant argument, in a caller
//! with cleanup paths, was refused at the call terminator (`SpreadNotTuple`).

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}
#[derive(Clone, Copy)]
struct Pair {
    hi: u32,
    lo: u32,
}
#[inline(never)]
fn sum_lanes(words: &[u32], index: usize) -> u32 {
    let unpack = |i: usize| Pair {
        hi: words[i * 2],
        lo: words[i * 2 + 1],
    };
    let a = unpack(index);
    let b = unpack(1);
    let mut sum = 0;
    for value in [a, b].into_iter() {
        sum += value.hi + value.lo;
    }
    sum
}
#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    // A native build returns 10 for index 0 and 14 for index 1; both indices are run-time values.
    // The floor contract is the identity, so the text comes back unchanged exactly when both
    // answers are the native ones.
    let first = text.len() % 2;
    let second = 1 - first;
    let native = [10, 14];
    let words = [1, 2, 3, 4];
    if sum_lanes(&words, first) != native[first] || sum_lanes(&words, second) != native[second] {
        return JsValue::erase("MISCOMPILED: sum_lanes differs from native");
    }
    JsValue::erase(text)
}
