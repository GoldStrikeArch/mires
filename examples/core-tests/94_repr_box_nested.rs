//! Nested boxes, vectors of boxes, move-out, and early return under compiler-owned Box drop.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::vec::Vec;

struct Pair(u32, u32);

struct Trace(u32);

impl Drop for Trace {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

struct Holder {
    first: Trace,
    second: Trace,
}

#[inline(never)]
fn nested(seed: u32, early: bool) -> u32 {
    let outer = Box::new(Box::new(Pair(seed, seed + 1)));
    let mut values = Vec::new();
    values.push(Box::new(Pair(seed + 2, seed + 3)));
    values.push(Box::new(Pair(seed + 4, seed + 5)));
    let guarded = Box::new(Holder {
        first: Trace(seed + 10),
        second: Trace(seed + 20),
    });
    let total = outer.0 + outer.1 + values[0].0 + values[0].1 + values[1].0 + values[1].1;
    if early {
        return total + 1;
    }
    drop(guarded);
    total
}

#[inline(never)]
fn move_out() {
    let boxed = Box::new(Holder {
        first: Trace(31),
        second: Trace(41),
    });
    let Holder { first, second } = *boxed;
    print_u32(first.0 + second.0);
    drop(second);
    drop(first);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(nested(1, false));
    print_u32(nested(2, true));
    move_out();
}
