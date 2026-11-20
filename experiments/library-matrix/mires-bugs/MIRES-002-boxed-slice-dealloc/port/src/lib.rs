//! mires bug repro: dropping a `Box<[u16]>` aborts with "this pointer does not name a live heap
//! block, so it cannot be deallocated".
//!
//! The floor library's identity contract (the text comes back unchanged). `input.case` picks the
//! construction (absent in the floor corpus, so the default `0`); see run.sh for all three.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
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
fn boxed(units: &[u16], how: u32) -> Box<[u16]> {
    match how {
        // `Box::<[u16]>::from(&[u16])`, as `slice.into()` spells it.
        0 => units.into(),
        // A sub-slice of it.
        1 => units[units.len().min(1)..].into(),
        // `Vec::into_boxed_slice`.
        _ => units.to_vec().into_boxed_slice(),
    }
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    let how = reflect_get(input, JsValue::erase("case"));
    let how = if how.is_undefined() { 0 } else { how.read::<u32>() };
    let units: Vec<u16> = text.encode_utf16().collect();
    let label = boxed(&units, how);
    let length = label.len();
    drop(label);
    let _ = length;
    JsValue::erase(text)
}
