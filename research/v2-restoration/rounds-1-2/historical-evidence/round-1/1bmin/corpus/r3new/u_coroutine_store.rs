//! (u) whole-value stores of a COROUTINE (an `async fn` future) held by value: through a raw
//! pointer, a generic `&mut T`, `mem::swap`, `mem::replace`, a struct field and a `Vec` element.
//! A future is an aggregate held by value, not a pointer: every alias of the place must see the
//! value stored.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use prelude::*;

async fn val(n: u32) -> u32 {
    n
}
#[inline(never)]
fn make(n: u32) -> impl Future<Output = u32> {
    val(n + 100)
}
fn poll_now<F: Future<Output = u32>>(f: &mut F) -> u32 {
    let mut cx = Context::from_waker(Waker::noop());
    match unsafe { Pin::new_unchecked(f) }.poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => 0,
    }
}
#[inline(never)]
fn set<T>(slot: &mut T, v: T) {
    *slot = v;
}
struct Holder<F> {
    n: u32,
    fut: F,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut f = make(1);
    let p = &raw mut f;
    unsafe { p.write(make(3)) };
    print_u32(poll_now(&mut f));
    let mut g = make(4);
    set(&mut g, make(5));
    print_u32(poll_now(&mut g));
    let mut h = make(6);
    let mut k = make(7);
    mem::swap(&mut h, &mut k);
    print_u32(poll_now(&mut h) * 1000 + poll_now(&mut k));
    let mut r = make(8);
    let old = mem::replace(&mut r, make(9));
    let mut old = old;
    print_u32(poll_now(&mut r) * 1000 + poll_now(&mut old));
    let mut hd = Holder { n: 1, fut: make(10) };
    let pf = &raw mut hd.fut;
    unsafe { pf.write(make(11)) };
    print_u32(poll_now(&mut hd.fut) + hd.n);
    let mut hd2 = Holder { n: 2, fut: make(12) };
    hd2.fut = make(13);
    set(&mut hd2.fut, make(14));
    print_u32(poll_now(&mut hd2.fut) + hd2.n);
    let mut v: Vec<_> = vec![make(15), make(16)];
    v.swap(0, 1);
    v[1] = make(17);
    let pv = v.as_mut_ptr();
    unsafe { pv.write(make(18)) };
    print_u32(poll_now(&mut v[0]) * 1000 + poll_now(&mut v[1]));
    let mut x = make(19);
    let px: *mut _ = &mut x;
    unsafe { *px = make(20) };
    print_u32(poll_now(&mut x));
}
