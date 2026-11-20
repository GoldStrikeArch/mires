#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_export, throw_value, JsValue};
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
    let i = input.read::<u32>();
    JsValue::erase(sum_lanes(&[1, 2, 3, 4], (i as usize) % 2))
}
