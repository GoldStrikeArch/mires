//! (g) enum whole-store then payload field identity. CONTRACT.md "Known limits" documents that a
//! whole-enum store replaces payload objects: identity lines may deviate, values may not.
//! Lines: label, then alternating relations; see the report for which are identity lines.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
#[derive(Clone, Copy)]
enum E {
    A { x: u32 },
    B { h: H, k: u32 },
}
struct W {
    n: u32,
    e: E,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}
fn payload(e: &E) -> usize {
    if let E::B { h, .. } = e { ad(h) } else { 0 }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut w = black_box(W { n: 1, e: E::B { h: mk(1), k: 2 } });
    let at_e = ad(&w.e);
    let at_h = payload(&w.e);
    let hp: *const H = if let E::B { h, .. } = &w.e { h } else { ptr::null() };
    // 1: the enum's own object keeps its identity (documented to hold)
    w.e = E::B { h: mk(5), k: 6 };
    print_bool(at_e == ad(&w.e));
    // 2: IDENTITY of the payload after a same-variant whole store (documented limit)
    print_bool(at_h == payload(&w.e));
    // 3: VALUE through the old payload pointer (native: new value 5)
    print_u32(unsafe { (*hp).head });
    // 4: store into the payload through a reference keeps identity (documented to hold)
    let before = payload(&w.e);
    if let E::B { h, .. } = &mut w.e {
        *h = mk(7);
    }
    print_bool(before == payload(&w.e));
    // 5: value
    print_u32(if let E::B { h, k } = &w.e { h.head * 10 + k } else { 0 });
    // 6: other variant and back: enum object identity
    w.e = E::A { x: 1 };
    w.e = E::B { h: mk(8), k: 9 };
    print_bool(at_e == ad(&w.e));
    // 7: IDENTITY of payload after variant round trip (documented limit)
    print_bool(at_h == payload(&w.e));
    // 8: value
    print_u32(if let E::B { h, k } = &w.e { h.head * 10 + k } else { 0 } + w.n);
}
