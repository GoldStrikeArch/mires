//! Canonical product wrappers coexist with private flat result bodies.

#![no_std]
#![no_main]
#![allow(improper_ctypes_definitions)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

unsafe extern "C" {
    fn host_throw();
}

#[derive(Clone, Copy)]
pub struct Pair {
    pub left: u32,
    pub right: u32,
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn exported_pair(seed: u32) -> Pair {
    Pair {
        left: seed.wrapping_add(1),
        right: seed.wrapping_mul(3),
    }
}

#[inline(never)]
fn callback_pair(seed: u32) -> Pair {
    Pair {
        left: seed.wrapping_add(2),
        right: seed.wrapping_add(4),
    }
}

#[unsafe(no_mangle)]
pub static PAIR_CALLBACK: fn(u32) -> Pair = callback_pair;

#[unsafe(no_mangle)]
pub fn indirect_pair(seed: u32) -> Pair {
    PAIR_CALLBACK(seed)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn exported_tuple(seed: u32) -> (u16, u32) {
    (seed.wrapping_add(1) as u16, seed.wrapping_mul(2))
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn fallible_pair(seed: u32, mode: u32) -> Pair {
    if mode == 1 {
        panic!("product panic");
    }
    if mode == 2 {
        unsafe { host_throw() };
    }
    Pair {
        left: seed.wrapping_add(7),
        right: seed.wrapping_mul(5),
    }
}

#[inline(never)]
fn direct_score(seed: u32) -> u32 {
    let pair = exported_pair(seed);
    pair.left.wrapping_add(pair.right)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn direct_fallible_score(seed: u32, mode: u32) -> u32 {
    let pair = fallible_pair(seed, mode);
    pair.left.wrapping_add(pair.right)
}

#[inline(never)]
fn tuple_score(seed: u32) -> u32 {
    let tuple = exported_tuple(seed);
    u32::from(tuple.0).wrapping_add(tuple.1)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(direct_score(5));

    let direct = callback_pair(7);
    print_u32(direct.left + direct.right);

    let indirect = PAIR_CALLBACK(11);
    print_u32(indirect.left + indirect.right);
    print_u32(tuple_score(4));
    print_u32(direct_fallible_score(3, 0));
}
