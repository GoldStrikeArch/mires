//! `Clone` of an async closure that owns its captures: the other closure kind whose clone rustc
//! writes as a shim that assembles the copy field by field (`341_clone_shim_aggregates`).
//!
//! An async closure's environment is a JavaScript array like a closure's, so its clone needs the
//! same empty container before its first capture lands (`CONTRACT.md`, "Aggregates assembled in
//! place"). Without it the clone threw `Cannot set properties of undefined (setting '0')`.
//!
//! The original is dropped before the copy runs, and a captured `Drop` value counts its drops, so
//! a copy that shared its captures with the original would show. The baseline lane cannot take a
//! field of an async closure at all and refuses this fixture at compile time; the JIR lane runs it.
//! The captures are `String`s: dropping an async closure that captured a `Vec<T>` fails on its own,
//! clone or no clone (`__rt.native_vec_set_len` on a value that is not a native vector), a separate
//! gap this fixture does not test.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use core::cell::Cell;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

#[derive(Clone)]
struct Token<'a> {
    value: u32,
    drops: &'a Cell<u32>,
}

impl Token<'_> {
    fn get(&self) -> u32 {
        self.value
    }
}

impl Drop for Token<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

#[inline(never)]
fn clone_value<T: Clone>(value: &T) -> T {
    value.clone()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let name = String::from("async");
    let more = String::from("abc");
    let measure = async move || name.len() + more.len();
    let copy = clone_value(&measure);
    print_usize(block_on(measure()));
    drop(measure);
    print_usize(block_on(copy()));
    print_usize(block_on(copy()));

    let drops = Cell::new(0);
    let token = Token {
        value: 7,
        drops: &drops,
    };
    // A method call captures the whole token, not just the `Copy` field a field read would.
    let read = async move || token.get() + 1;
    let again = clone_value(&read);
    print_u32(block_on(again()));
    drop(read);
    print_u32(drops.get());
    drop(again);
    print_u32(drops.get());
}
