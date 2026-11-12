//! (i) drop order when a field holding a Drop type is reassigned: old dropped once, after the
//! new value is built.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use prelude::*;

static mut LOG: u64 = 0;
fn note(d: u32) {
    unsafe { LOG = LOG * 10 + d as u64 }
}
fn flush() {
    unsafe {
        print_u64(LOG);
        LOG = 0;
    }
}

struct T(u32);
impl Drop for T {
    fn drop(&mut self) {
        note(self.0)
    }
}
#[inline(never)]
fn build(n: u32) -> T {
    note(8);
    black_box(T(n))
}
struct Two {
    a: T,
    b: T,
}
struct Holder {
    k: u32,
    t: T,
    two: Two,
    bx: Box<T>,
    v: Vec<T>,
}
fn holder(seed: u32) -> Holder {
    Holder {
        k: seed,
        t: build(1),
        two: Two { a: build(2), b: build(3) },
        bx: Box::new(build(4)),
        v: vec![build(5), build(6)],
    }
}

#[inline(never)]
fn e() {
    let mut h = black_box(holder(0));
    flush();
    h.t = build(7);
    flush();
    h.two = Two { a: build(1), b: build(2) };
    flush();
    h.two.b = build(9);
    flush();
    h.bx = Box::new(build(9));
    flush();
    h.v = vec![build(1)];
    flush();
    let moved = h.t;
    h.t = build(5);
    flush();
    drop(moved);
    flush();
    mem::replace(&mut h.t, build(6));
    flush();
    let _ = mem::replace(&mut h.t, build(4));
    flush();
    let _keep = mem::replace(&mut h.t, build(3));
    flush();
    for i in 0..2 {
        h.t = build(i);
    }
    flush();
    h = holder(1);
    flush();
    print_u32(h.k + h.t.0 + h.two.a.0 + h.bx.0 + h.v[1].0);
    {
        let r = &mut h.two;
        *r = Two { a: build(7), b: build(7) };
    }
    flush();
    let mut arr = [build(1), build(2)];
    flush();
    arr[1] = build(3);
    flush();
    arr = [build(4), build(5)];
    flush();
    print_u32(arr[0].0 + arr[1].0 + _keep.0);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e();
    flush();
}
