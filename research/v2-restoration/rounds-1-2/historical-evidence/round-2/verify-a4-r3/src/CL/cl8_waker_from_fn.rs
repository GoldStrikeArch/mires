//! `Waker::from_fn_ptr` (safe, unstable) with a closure in a constant: core casts the fn pointer
//! to the waker's data pointer inside const evaluation.
#![no_std]
#![no_main]
#![feature(waker_from_fn_ptr)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::task::Waker;

fn named_wake() { print_u32(2); }
static W_NAMED: Waker = Waker::from_fn_ptr(named_wake);
static W_CLOSURE: Waker = Waker::from_fn_ptr(|| print_u32(1));

#[unsafe(no_mangle)]
pub fn rust_entry() {
    W_NAMED.wake_by_ref();
    W_CLOSURE.wake_by_ref();
    let w = W_CLOSURE.clone();
    w.wake();
    let rt = Waker::from_fn_ptr(|| print_u32(3));
    rt.wake_by_ref();
    rt.wake();
}
