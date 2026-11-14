//! Native Box values across explicit return, panic-abort, and a throwing host call.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

unsafe extern "C-unwind" {
    fn host_throw();
}

struct Trace(u32);

impl Drop for Trace {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[allow(dead_code)]
struct Holder {
    first: Trace,
    second: Trace,
}

struct Pair(u32, u32);

#[inline(never)]
fn early_box(seed: u32, early: bool) -> u32 {
    let value = Box::new(Pair(seed, seed + 1));
    if early {
        return value.0 + value.1 + 1;
    }
    let total = value.0 + value.1;
    drop(value);
    total
}

#[unsafe(no_mangle)]
pub fn box_exit(mode: u32) -> u32 {
    let value = Box::new(Holder {
        first: Trace(10),
        second: Trace(20),
    });
    if mode == 1 {
        panic!("box panic");
    }
    if mode == 2 {
        unsafe { host_throw() };
    }
    drop(value);
    7
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(box_exit(0));
    print_u32(early_box(3, true));
    print_u32(early_box(5, false));
}
