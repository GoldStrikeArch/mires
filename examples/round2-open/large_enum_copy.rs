//! A by-value copy of an enum with MORE THAN EIGHT variants shares its aggregate payload with the
//! source (`clone_expr` falls back to a one-level `Object.assign` above `MAX_CLONED_VARIANTS`:
//! jsc-link/src/lower/js/value.rs:557, backend/src/value.rs:717), so a write into the copy's
//! payload lands in the source. Safe Rust, every lane, main and the A1 commit alike.
//! Native (-O0 and -O3): 12 / 992012 / 12 / 34 / 12 / 12
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
enum Big {
    V0,
    V1(u32),
    V2(u32),
    V3(u32),
    V4(u32),
    V5(u32),
    V6(u32),
    V7(u32),
    V8(P),
    V9([u32; 2]),
}

fn obs(b: &Big) -> u32 {
    match b {
        Big::V8(p) => p.x * 10 + p.y,
        Big::V9(a) => a[0] * 10 + a[1],
        Big::V1(v) => *v,
        _ => 0,
    }
}

#[inline(never)]
fn dead(s: &[Big]) -> u32 {
    let mut c = s[black_box(0)];
    if let Big::V8(p) = &mut c {
        p.x = 99;
    }
    obs(&s[0])
}

#[inline(never)]
fn read_back(s: &[Big]) -> u32 {
    let mut c = s[black_box(0)];
    if let Big::V8(p) = &mut c {
        p.x = 99;
    }
    obs(&c) * 1000 + obs(&s[0])
}

#[inline(never)]
fn by_value(b: Big) -> u32 {
    let mut c = b;
    if let Big::V8(p) = &mut c {
        p.y = 7;
    }
    obs(&b)
}

#[inline(never)]
fn array_payload(s: &[Big]) -> u32 {
    let mut c = s[black_box(1)];
    if let Big::V9(a) = &mut c {
        a[1] = 99;
    }
    obs(&s[1])
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = black_box([Big::V8(P { x: 1, y: 2 }), Big::V9([3, 4]), Big::V1(5)]);
    print_u32(dead(&a));
    let b = black_box([Big::V8(P { x: 1, y: 2 }), Big::V9([3, 4]), Big::V1(5)]);
    print_u32(read_back(&b));
    let c = black_box([Big::V8(P { x: 1, y: 2 }), Big::V9([3, 4]), Big::V1(5)]);
    print_u32(by_value(c[0]));
    print_u32(array_payload(&c));
    print_u32(obs(&c[0]));
    print_u32(obs(&a[0]));
}
