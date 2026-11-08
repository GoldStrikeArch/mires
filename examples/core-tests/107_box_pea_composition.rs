//! Native Box ownership and structural drop alongside loop-carried local products.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

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

#[derive(Clone, Copy)]
pub struct Pair {
    x: u32,
    y: u32,
}

#[unsafe(no_mangle)]
pub fn box_pea(seed: u32, rounds: u32, early: bool, observe: fn(Pair) -> u32) -> u32 {
    let boxed = Box::new(Holder {
        first: Trace(seed),
        second: Trace(seed + 1),
    });
    let mut moved = boxed;
    let mut pair = Pair {
        x: seed,
        y: seed + 2,
    };
    let copy = pair;
    #[cfg(box_pea_replace)]
    let saved = core::mem::replace(&mut pair.x, seed + 3);
    #[cfg(not(box_pea_replace))]
    let saved = {
        let saved = pair.x;
        pair.x = seed + 3;
        saved
    };
    let mut index = 0;
    while index < rounds {
        pair.x += pair.y;
        pair.y += 1;
        let first = &mut moved.first.0;
        *first += index;
        index += 1;
    }
    moved.second.0 += pair.y;
    print_u32(saved);
    print_u32(copy.x);
    print_u32(copy.y);
    let observed = observe(if early { copy } else { pair });
    if early {
        return pair.x + observed;
    }
    let result = pair.x + pair.y + observed;
    drop(moved);
    result
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    fn observe(pair: Pair) -> u32 {
        pair.x + pair.y
    }
    print_u32(box_pea(3, 0, false, observe));
    print_u32(box_pea(3, 2, false, observe));
    print_u32(box_pea(3, 2, true, observe));
}
