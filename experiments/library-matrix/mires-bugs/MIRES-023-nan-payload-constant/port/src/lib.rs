//! mires bug repro (MIRES-023): an `f32` or `f64` constant that is a NaN with a payload is emitted
//! as the JS literal `NaN`, so `to_bits()` reads the canonical NaN. Under the matrix profile
//! (`JSC_RUST_MIR_INLINE=off`) this happens to a `const`. With `JSC_RUST_MIR_INLINE=on`, MIR
//! inlining folds `f32::from_bits(0x7fc0_0001)` into such a constant too.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

/// Quiet NaNs whose lowest payload bit is set.
const BITS32: u32 = 0x7fc0_0001;
const BITS64: u64 = 0x7ff8_0000_0000_0001;
const NAN32: f32 = f32::from_bits(BITS32);
const NAN64: f64 = f64::from_bits(BITS64);

#[inline(never)]
fn bits32(x: f32) -> u32 {
    x.to_bits()
}

#[inline(never)]
fn bits64(x: f64) -> u64 {
    x.to_bits()
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let mut lost = String::new();
    if bits32(NAN32) != BITS32 {
        lost.push_str(" const-f32");
    }
    if bits64(NAN64) != BITS64 {
        lost.push_str(" const-f64");
    }
    if bits32(f32::from_bits(BITS32)) != BITS32 {
        lost.push_str(" from_bits-f32");
    }
    if bits64(f64::from_bits(BITS64)) != BITS64 {
        lost.push_str(" from_bits-f64");
    }
    if !lost.is_empty() {
        let mut message = String::from("MISCOMPILED: NaN payload lost:");
        message.push_str(&lost);
        return JsValue::erase(message.as_str());
    }
    JsValue::erase(text)
}
