//! (e') a store through a `&mut` into an existing variant's payload field, where the borrow is
//! used exactly once (so the lowering may inline it): the payload must keep its storage.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
enum E {
    A(u32),
    B { h: H, k: u32 },
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn payload(e: &E) -> usize {
    match e {
        E::B { h, .. } => a(h),
        E::A(_) => 0,
    }
}

#[inline(never)]
fn e() {
    let mut e = black_box(E::B { h: H { head: 1, tail: 2 }, k: 3 });
    let before = payload(&e);
    if let E::B { h, .. } = &mut e {
        *h = H { head: 10, tail: 20 };
    }
    print_bool(payload(&e) == before);
    if let E::B { h, .. } = &mut e {
        *h = black_box(H { head: 30, tail: 40 });
    }
    print_bool(payload(&e) == before);
    let raw = if let E::B { h, .. } = &mut e { h as *mut H } else { core::ptr::null_mut() };
    if let E::B { h, k } = &mut e {
        *h = H { head: 50, tail: 60 };
        *k += 1;
    }
    print_bool(raw as usize == payload(&e));
    print_u32(unsafe { (*raw).head });
    let total = match &e {
        E::B { h, k } => h.head + h.tail + *k,
        E::A(x) => *x,
    };
    print_u32(total);
    e = E::A(7);
    print_u32(payload(&e) as u32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e();
}
