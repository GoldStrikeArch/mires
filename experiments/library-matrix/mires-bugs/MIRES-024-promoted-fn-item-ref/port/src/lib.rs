//! mires bug repro (MIRES-024): `apply_ref(&twice)` passes a promoted reference to the zero-sized
//! fn item `twice` to a generic `F: Fn(u32) -> u32`. The link step refuses it: "constants of type
//! `fn(u32) -> u32 {twice}` cannot be read out of an allocation". Natively it returns 30.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_export, throw_value, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

#[inline(never)]
fn twice(x: u32) -> u32 {
    x * 2
}

#[inline(never)]
fn apply_ref<F: Fn(u32) -> u32>(f: &F) -> u32 {
    f(7) + f(8)
}

#[js_export]
pub fn matrix_run(_input: JsValue) -> JsValue {
    JsValue::erase(apply_ref(&twice))
}
