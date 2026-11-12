//! (g) enum whole-store: VALUES only (identity of payloads is a documented limit and is probed
//! in g_enum_identity). Every line here must match native.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
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
    C(H, H),
    D,
}
struct W {
    n: u32,
    e: E,
    f: E,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
fn ce(e: &E) -> u32 {
    match e {
        E::A { x } => 1000 + x,
        E::B { h, k } => 2000 + h.head * 100 + h.tail * 10 + k,
        E::C(p, q) => 3000 + p.head * 100 + q.head * 10 + p.tail + q.tail,
        E::D => 4000,
    }
}
#[inline(never)]
fn set(e: &mut E, v: E) {
    *e = v;
}

#[inline(never)]
fn t_values() {
    print_str("values");
    let mut w = black_box(W { n: 1, e: E::B { h: mk(1), k: 2 }, f: E::A { x: 3 } });
    // a pointer into the old payload, read after a same-variant store: reads the old or the
    // new payload natively? The place is the same, so natively it reads the NEW value.
    let hp: *const H = if let E::B { h, .. } = &w.e { h } else { ptr::null() };
    w.e = E::B { h: mk(5), k: 6 };
    print_u32(ce(&w.e));
    print_u32(unsafe { (*hp).head });
    w.e = E::A { x: 7 };
    print_u32(ce(&w.e));
    w.e = E::C(mk(1), mk(3));
    print_u32(ce(&w.e));
    w.e = E::B { h: mk(8), k: 9 };
    print_u32(ce(&w.e));
    w.e = E::D;
    print_u32(ce(&w.e));
    w.e = E::C(mk(2), mk(4));
    w.f = w.e;
    if let E::C(p, _) = &mut w.f {
        p.head += 50;
    }
    print_u32(ce(&w.e));
    print_u32(ce(&w.f));
    set(&mut w.e, E::B { h: mk(1), k: 1 });
    set(&mut w.f, w.e);
    if let E::B { h, k } = &mut w.e {
        *h = mk(9);
        *k += 1;
    }
    print_u32(ce(&w.e));
    print_u32(ce(&w.f));
    mem::swap(&mut w.e, &mut w.f);
    print_u32(ce(&w.e));
    print_u32(ce(&w.f));
    let old = mem::replace(&mut w.e, E::A { x: 1 });
    print_u32(ce(&old) + ce(&w.e));
    let p: *mut E = &mut w.f;
    unsafe { ptr::write(p, E::C(mk(1), mk(2))) };
    print_u32(ce(&w.f));
    unsafe { *p = E::D };
    print_u32(ce(&w.f) + w.n);
    let mut v: Vec<E> = vec![E::A { x: 1 }, E::B { h: mk(2), k: 3 }, E::C(mk(4), mk(5)), E::D];
    v.swap(0, 1);
    v.swap(2, 3);
    v[0] = v[3];
    v[3] = E::A { x: 9 };
    v.reverse();
    print_u32(ce(&v[0]));
    print_u32(ce(&v[1]));
    print_u32(ce(&v[2]));
    print_u32(ce(&v[3]));
    v.sort_by_key(|e| ce(e));
    print_u32(ce(&v[0]) + ce(&v[3]));
    let mut o: Option<H> = black_box(None);
    o = Some(mk(1));
    let r = &mut o;
    *r = Some(mk(2));
    print_u32(r.map_or(0, |h| h.head));
    *r = None;
    print_bool(r.is_none());
    *r = Some(mk(3));
    let t = r.take();
    print_u32(t.map_or(0, |h| h.head) + o.map_or(7, |h| h.head));
    let mut res: Result<H, u32> = black_box(Ok(mk(1)));
    res = Err(5);
    res = match res {
        Err(e) => Ok(mk(e)),
        ok => ok,
    };
    print_u32(res.map_or(0, |h| h.tail));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_values();
}
