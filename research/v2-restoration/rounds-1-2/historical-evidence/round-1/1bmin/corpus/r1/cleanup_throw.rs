//! (j) a host throw while the new value for a field is being built: the old value must be
//! dropped exactly once, and only during cleanup.
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

struct T(u32);
impl Drop for T {
    fn drop(&mut self) {
        print_u32(self.0)
    }
}
struct Holder {
    t: T,
    v: Vec<T>,
}

#[inline(never)]
fn build(n: u32, at: u32, when: u32) -> T {
    if at == when {
        unsafe { host_throw() }
    }
    black_box(T(n))
}

#[inline(never)]
fn make_vec(at: u32) -> Vec<T> {
    let first = build(31, at, 4);
    let second = build(32, at, 5);
    vec![first, second]
}

#[unsafe(no_mangle)]
pub fn run(at: u32) -> u32 {
    let mut h = Holder { t: T(1), v: vec![T(2)] };
    print_u32(90);
    h.t = build(11, at, 1);
    print_u32(91);
    let tmp = build(12, at, 0);
    if at == 2 {
        unsafe { host_throw() }
    }
    h.t = tmp;
    print_u32(92);
    h.v = make_vec(at);
    print_u32(93);
    if at == 3 {
        h.t = T(13);
        unsafe { host_throw() }
    }
    print_u32(94);
    h.t.0 + h.v.len() as u32
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(run(99));
}
