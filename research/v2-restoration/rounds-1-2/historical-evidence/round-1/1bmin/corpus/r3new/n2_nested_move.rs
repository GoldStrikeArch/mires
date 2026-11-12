//! (n2) element moves (`ptr::swap`, `ptr::copy`, `slice::swap`) of aggregates with NESTED owned
//! aggregates, accessed through raw pointers only: a pointer to a nested field taken before the
//! move still names that element's nested field afterwards.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct D {
    x: u32,
    y: u32,
}
#[derive(Clone, Copy)]
struct A {
    k: u32,
    d: D,
}
#[inline(never)]
fn mk(s: u32) -> A {
    black_box(A { k: s, d: D { x: s * 10, y: s * 10 + 1 } })
}

#[inline(never)]
fn run(base: *mut A) {
    unsafe {
        let e1 = base.add(1);
        let raw_d = &raw const (*e1).d;
        let raw_x = &raw const (*e1).d.x;
        print_u32((*raw_d).x + *raw_x);
        (*e1).d = D { x: 5, y: 6 };
        print_u32((*raw_d).x * 10 + *raw_x);
        print_bool(ptr::eq(raw_d, &raw const (*e1).d));
        ptr::swap(base, e1);
        print_u32((*e1).k * 1000 + (*e1).d.x);
        print_u32((*raw_d).x);
        print_u32(*raw_x);
        print_bool(ptr::eq(raw_d, &raw const (*e1).d));
        ptr::copy(base.add(2), e1, 1);
        print_u32((*e1).k * 1000 + (*e1).d.x);
        print_u32((*raw_d).x);
        print_bool(ptr::eq(raw_d, &raw const (*e1).d));
        ptr::copy_nonoverlapping(base, e1, 1);
        print_u32((*raw_d).x);
        e1.write(mk(9));
        print_u32((*raw_d).x);
        print_bool(ptr::eq(raw_d, &raw const (*e1).d));
        let old = ptr::replace(e1, mk(8));
        print_u32((*raw_d).x * 1000 + old.d.x);
        // a fresh pointer taken now, then one more move: is the staleness per move?
        let fresh_d = &raw const (*e1).d;
        ptr::swap(base, e1);
        print_u32((*fresh_d).x);
        print_bool(ptr::eq(fresh_d, &raw const (*e1).d));
        (*(&raw mut (*e1).d)).x = 77;
        print_u32((*e1).d.x * 100 + (*fresh_d).x);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("stack");
    let mut arr = [mk(1), mk(2), mk(3)];
    run(arr.as_mut_ptr());
    print_u32(arr[0].d.x * 10000 + arr[1].d.x * 100 + arr[2].d.x);
    print_str("heap");
    let mut v: Vec<A> = vec![mk(1), mk(2), mk(3)];
    run(v.as_mut_ptr());
    print_u32(v[0].d.x * 10000 + v[1].d.x * 100 + v[2].d.x);
}
