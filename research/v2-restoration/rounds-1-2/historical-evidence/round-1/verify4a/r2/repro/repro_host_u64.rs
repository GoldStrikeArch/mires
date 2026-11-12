//! Round 2: a host value in a number position, in a crate with no `unsafe` at all.
//!
//! `count` is a typed `js_bindings!` getter (the form `examples/host_loop_callbacks.rs` uses in a
//! `#![deny(unsafe_code)]` crate). Its result is not checked, so when the host object's `count`
//! is not a Number (a BigInt, a Symbol, an object with `valueOf`), the `u32` holds that value.
//! `bits` is `count_ones` + `rotate_left` (audited intrinsics, quiet since Stage 4'a);
//! `sum` is `+` and `>>` (Binary, quiet since Stage 3b). The guard pushes onto a host array.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(get = "count")]
    fn count(holder: JsValue) -> u64;
    #[js(method = "push")]
    fn push(array: JsValue, value: JsValue);
}

struct Guard(JsValue);
impl Drop for Guard {
    fn drop(&mut self) {
        push(self.0, JsValue::erase("dropped"));
    }
}

#[inline(never)]
fn bits(n: u64) -> u32 {
    n.count_ones() ^ n.rotate_left(3) as u32
}

#[inline(never)]
fn sum(n: u64) -> u32 {
    (n.wrapping_add(1) ^ (n >> 1)) as u32
}

#[js_export]
pub fn run(holder: JsValue, trace: JsValue, use_bits: bool) -> u32 {
    let n = count(holder);
    let guard = Guard(trace);
    let r = if use_bits { bits(n) } else { sum(n) };
    drop(guard);
    r
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}
