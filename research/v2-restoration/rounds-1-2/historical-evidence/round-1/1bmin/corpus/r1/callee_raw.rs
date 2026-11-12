//! Analysis-limit probe: the raw pointer that reinitializes a moved-out place is made in a
//! *callee*, so the caller body has neither a store to the place nor a raw borrow of it.
//! (Stacked Borrows accepts this program; Tree Borrows may not.)
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use prelude::*;

struct Inner {
    v: u32,
    w: u32,
}
struct Outer {
    n: u32,
    inner: Inner,
}

#[inline(never)]
fn leak(o: &mut Outer) -> *mut Inner {
    &raw mut o.inner
}

#[inline(never)]
fn mk(v: u32) -> Inner {
    black_box(Inner { v, w: v * 10 })
}

#[inline(never)]
fn e() {
    let mut o = black_box(Outer { n: 1, inner: mk(1) });
    let p = leak(&mut o);
    let mut x = o.inner;
    unsafe { p.write(mk(9)) };
    x.v += 1;
    print_u32(x.v);
    print_u32(unsafe { (*p).v });
    print_u32(o.n);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e();
}
