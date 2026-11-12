//! Verifier repro: a by-value copy of an element of a pooled table is written; the table must not change.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

const PS: &[P] = &[P { x: 0, y: 5000 }, P { x: 1, y: 5001 }, P { x: 2, y: 5002 }, P { x: 3, y: 5003 }, P { x: 4, y: 5004 }, P { x: 5, y: 5005 }, P { x: 6, y: 5006 }, P { x: 7, y: 5007 }, P { x: 8, y: 5008 }, P { x: 9, y: 5009 }, P { x: 10, y: 5010 }, P { x: 11, y: 5011 }, P { x: 12, y: 5012 }, P { x: 13, y: 5013 }, P { x: 14, y: 5014 }, P { x: 15, y: 5015 }, P { x: 16, y: 5016 }, P { x: 17, y: 5017 }, P { x: 18, y: 5018 }, P { x: 19, y: 5019 }, P { x: 20, y: 5020 }, P { x: 21, y: 5021 }, P { x: 22, y: 5022 }, P { x: 23, y: 5023 }];

#[inline(never)]
fn total() -> u32 {
    PS.iter().map(|p| p.x).sum()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(total());
    let mut p = PS[4];
    p.x = 99;
    print_u32(p.x);
    print_u32(PS[4].x);
    print_u32(total());
}
