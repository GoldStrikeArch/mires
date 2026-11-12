//! (f) a host throw while the new value of an aggregate place is being built, with cleanup on:
//! the old value is intact and dropped exactly once, later, during cleanup.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use prelude::*;

unsafe extern "C" {
    fn host_throw();
}

struct D(u32);
impl Drop for D {
    fn drop(&mut self) {
        print_u32(self.0)
    }
}
struct Two {
    a: D,
    b: D,
}
struct Holder {
    two: Two,
    arr: [D; 2],
    o: Option<Two>,
    v: Vec<Two>,
}

#[inline(never)]
fn build(n: u32, at: u32, when: u32) -> D {
    if at == when {
        unsafe { host_throw() }
    }
    black_box(D(n))
}
#[inline(never)]
fn set(r: &mut Two, at: u32) {
    *r = Two { a: build(31, at, 0), b: build(32, at, 3) };
}

#[unsafe(no_mangle)]
pub fn run(at: u32) -> u32 {
    let mut h = Holder {
        two: Two { a: D(1), b: D(2) },
        arr: [D(3), D(4)],
        o: Some(Two { a: D(5), b: D(6) }),
        v: vec![Two { a: D(7), b: D(8) }],
    };
    print_u32(90);
    h.two = Two { a: build(11, at, 0), b: build(12, at, 1) };
    print_u32(91);
    h.arr = [build(13, at, 0), build(14, at, 2)];
    print_u32(92);
    set(&mut h.two, at);
    print_u32(93);
    h.o = Some(Two { a: build(15, at, 0), b: build(16, at, 4) });
    print_u32(94);
    h.v[0] = Two { a: build(17, at, 0), b: build(18, at, 5) };
    print_u32(95);
    h.two.a.0 + h.two.b.0 + h.arr[1].0 + h.v[0].b.0 + h.o.as_ref().map_or(0, |t| t.a.0)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(run(99));
}
