//! Host callbacks created directly inside `for` loop bodies keep their own turn's captures.
//!
//! Every local is declared once, in the function prelude, so a closure environment built inside a
//! loop is assigned to the same binding on every turn. A thunk that closed over that binding would
//! read whatever the last turn left there. CONTRACT.md, "A thunk built inside a loop snapshots its
//! environment", is the rule; this fixture pins it for `mires_js` callbacks, which share that
//! lowering with the DOM markers. Each closure is created in the loop body itself (not behind a
//! helper function, which would give every turn a fresh call frame and hide the defect), stored in
//! a `Vec`, and handed to JavaScript only after the loop has finished, so
//! scripts/host-loop-callbacks-check.mjs can call each one and require its own turn's values.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use mires_js::{callback0, callback1, js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(new = "Array")]
    fn array() -> JsValue;
    #[js(method = "push")]
    fn push(array: JsValue, value: JsValue);
    #[js(get = "count")]
    fn count(counter: JsValue) -> u32;
    #[js(set = "count")]
    fn set_count(counter: JsValue, value: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("host loop callbacks panic"))
}

static ROWS: [u32; 4] = [10, 20, 30, 40];

/// The callbacks, in creation order, as a JavaScript array. Called after every loop has ended.
#[inline(never)]
fn to_array(callbacks: Vec<JsValue>) -> JsValue {
    let out = array();
    for callback in callbacks {
        push(out, callback);
    }
    out
}

/// The loop variable captured by value.
#[js_export]
pub fn by_value(turns: u32) -> JsValue {
    let mut callbacks = Vec::new();
    for index in 0..turns {
        callbacks.push(callback0(move || JsValue::erase(index)));
    }
    to_array(callbacks)
}

/// The loop variable is a `&'static u32`, a reference to a `Copy` value, and the closure captures
/// the reference. A one-argument callback, so the thunk forwards an argument as well.
#[js_export]
pub fn by_reference() -> JsValue {
    let mut callbacks = Vec::new();
    for row in ROWS.iter() {
        callbacks.push(callback1(move |offset| JsValue::erase(*row + offset.read::<u32>())));
    }
    to_array(callbacks)
}

/// Both loop variables of a nested loop, captured by the inner body.
#[js_export]
pub fn nested(rows: u32, columns: u32) -> JsValue {
    let mut callbacks = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            callbacks.push(callback0(move || JsValue::erase(row * 100 + column)));
        }
    }
    to_array(callbacks)
}

/// The loop variable and an outer counter: every call adds its own turn's index to the counter
/// the caller supplied and answers the index. The counter is a host object because a callback's
/// captures belong to the host (see `mires_js::callback0`), so the environment holds two values.
#[js_export]
pub fn counting(counter: JsValue, turns: u32) -> JsValue {
    let mut callbacks = Vec::new();
    for index in 0..turns {
        callbacks.push(callback0(move || {
            set_count(counter, count(counter) + index);
            JsValue::erase(index)
        }));
    }
    to_array(callbacks)
}

/// A local the loop body itself mutates, captured after the mutation: each closure owes the value
/// the running total had on its own turn, not the final one.
#[js_export]
pub fn running(turns: u32) -> JsValue {
    let mut callbacks = Vec::new();
    let mut total = 0_u32;
    for index in 1..=turns {
        total += index;
        callbacks.push(callback0(move || JsValue::erase(total)));
    }
    to_array(callbacks)
}
