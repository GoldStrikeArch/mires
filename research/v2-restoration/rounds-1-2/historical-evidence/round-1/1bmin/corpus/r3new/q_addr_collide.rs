//! (q) integer addresses of distinct live places never collide: three structs larger than 16
//! bytes, each first addressed through its FIRST scalar field, then through a later field.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[repr(C)]
struct Five {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
}
#[inline(never)]
fn mk(s: u32) -> Five {
    black_box(Five { a: s, b: s + 1, c: s + 2, d: s + 3, e: s + 4 })
}
fn au(r: &u32) -> usize {
    r as *const u32 as usize
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let o = mk(1);
    let p = mk(10);
    let q = mk(20);
    let (oa, pa, qa) = (au(&o.a), au(&p.a), au(&q.a));
    let (oe, pe, qe) = (au(&o.e), au(&p.e), au(&q.e));
    print_bool(oa != pa && pa != qa && oa != qa);
    print_bool(oe != pa && oe != qa);
    print_bool(pe != oa && pe != qa);
    print_bool(qe != oa && qe != pa);
    print_bool(oe - oa == mem::offset_of!(Five, e));
    let all = [
        au(&o.a), au(&o.b), au(&o.c), au(&o.d), au(&o.e),
        au(&p.a), au(&p.b), au(&p.c), au(&p.d), au(&p.e),
        au(&q.a), au(&q.b), au(&q.c), au(&q.d), au(&q.e),
    ];
    let mut clashes = 0u32;
    for i in 0..all.len() {
        for j in 0..i {
            if all[i] == all[j] {
                clashes += 1;
            }
        }
    }
    print_u32(clashes);
    print_bool(&o as *const Five as usize == oa);
    print_u32(o.e + p.a + q.c);
}
