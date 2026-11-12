//! Copying a union whose fields are two different aggregates throws a TypeError at run time (the
//! copy clones every field, and the inactive one is undefined). Needs `unsafe` to observe; every
//! lane, main and the A1 commit alike. Native (-O0 and -O3): 99001
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
union UP {
    p: P,
    q: [u32; 2],
}

#[inline(never)]
fn copy_out(us: &[UP]) -> u32 {
    let mut u = us[black_box(0)];
    unsafe {
        u.p.x = 99;
        u.p.x * 1000 + us[0].p.x
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let up = black_box([UP { p: P { x: 1, y: 2 } }, UP { p: P { x: 3, y: 4 } }]);
    print_u32(copy_out(&up));
}
