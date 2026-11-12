//! (u2) `*a.r = v` where `a.r: &mut F` and `F` is a coroutine (an `async fn` future): the store
//! is spelled inline (`*a.r = v`) and through a named reference (`let r = &mut *a.r; *r = v`).
//! Both write the POINTEE; the owner of the future must see the new one.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::future::Future;
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
struct A<'a, F> {
    n: u32,
    r: &'a mut F,
}
#[inline(never)]
fn inline_store<F>(a: &mut A<'_, F>, v: F) {
    *a.r = v;
}
#[inline(never)]
fn named_store<F>(a: &mut A<'_, F>, v: F) {
    let r = &mut *a.r;
    *r = v;
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut f = make(1);
    {
        let mut a = A { n: 1, r: &mut f };
        inline_store(&mut a, make(2));
        print_u32(a.n);
    }
    print_u32(poll_now(&mut f));
    let mut g = make(3);
    {
        let mut a = A { n: 2, r: &mut g };
        named_store(&mut a, make(4));
        print_u32(a.n);
    }
    print_u32(poll_now(&mut g));
    let mut h = make(5);
    {
        let a = A { n: 3, r: &mut h };
        *a.r = make(6);
        print_u32(a.n);
    }
    print_u32(poll_now(&mut h));
}
