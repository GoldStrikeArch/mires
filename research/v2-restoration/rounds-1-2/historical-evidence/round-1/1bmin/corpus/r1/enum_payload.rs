//! (e) enums: payload structs reassigned, references into payloads, whole-enum stores.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct Inner {
    v: u32,
    w: u32,
}
enum E {
    A(Inner),
    B { h: H, k: u32 },
    C,
}
struct W {
    n: u32,
    e: E,
    opt: Option<Inner>,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

fn sum_e(e: &E) -> u32 {
    match e {
        E::A(i) => i.v + i.w,
        E::B { h, k } => h.head + h.tail + *k,
        E::C => 0,
    }
}

#[inline(never)]
fn e_enum() {
    let mut w = black_box(W {
        n: 1,
        e: E::B { h: H { head: 1, tail: 2 }, k: 3 },
        opt: Some(Inner { v: 1, w: 2 }),
    });
    let ate = a(&w.e);
    let ato = a(&w.opt);
    let mut ph = 0;
    if let E::B { h, .. } = &mut w.e {
        *h = H { head: 10, tail: 20 };
        ph = h as *const H as usize;
    }
    let ph2 = if let E::B { h, .. } = &w.e { a(h) } else { 0 };
    print_bool(ph == ph2);
    print_bool(ph != 0);
    print_u32(sum_e(&w.e));
    // whole-enum reassign, same variant
    w.e = E::B { h: H { head: 30, tail: 40 }, k: 5 };
    let ph3 = if let E::B { h, .. } = &w.e { a(h) } else { 0 };
    print_u32(sum_e(&w.e));
    print_bool(a(&w.e) == ate);
    print_str(if ph3 == ph { "payload-stable" } else { "payload-moved" });
    // variant change and back
    w.e = E::A(Inner { v: 7, w: 8 });
    print_bool(a(&w.e) == ate);
    print_u32(sum_e(&w.e));
    w.e = E::C;
    print_u32(sum_e(&w.e));
    w.e = E::B { h: H { head: 1, tail: 1 }, k: 1 };
    print_u32(sum_e(&w.e));
    if let E::B { k, .. } = &mut w.e {
        *k += 5;
    }
    print_u32(sum_e(&w.e));
    // Option payload
    let p1 = w.opt.as_ref().map(a).unwrap_or(0);
    w.opt = Some(Inner { v: 3, w: 4 });
    let p2 = w.opt.as_ref().map(a).unwrap_or(0);
    print_bool(a(&w.opt) == ato);
    print_str(if p1 == p2 { "payload-stable" } else { "payload-moved" });
    if let Some(i) = &mut w.opt {
        *i = Inner { v: 5, w: 6 };
    }
    print_u32(w.opt.as_ref().unwrap().v);
    let p3 = w.opt.as_ref().map(a).unwrap_or(0);
    print_bool(p3 == p2);
    let t = w.opt.take();
    print_bool(w.opt.is_none());
    w.opt = t.map(|mut i| {
        i.v += 10;
        i
    });
    print_u32(w.opt.as_ref().unwrap().v);
    let got = w.opt.insert(Inner { v: 1, w: 1 });
    got.v += 1;
    print_u32(w.opt.as_ref().unwrap().v);
    let r = w.opt.get_or_insert(Inner { v: 100, w: 100 });
    r.w += 1;
    print_u32(w.opt.as_ref().unwrap().w);
    w.opt = None;
    let r = w.opt.get_or_insert(Inner { v: 100, w: 100 });
    r.w += 1;
    print_u32(w.opt.as_ref().unwrap().w);
    // Vec<E> element reassign
    let mut ve: Vec<E> = vec![E::C, E::A(Inner { v: 1, w: 1 })];
    let av = a(&ve[1]);
    ve[1] = E::B { h: H { head: 2, tail: 2 }, k: 2 };
    print_bool(a(&ve[1]) == av);
    ve[0] = E::A(Inner { v: 9, w: 9 });
    print_u32(ve.iter().map(sum_e).sum::<u32>());
    ve.push(E::C);
    ve.swap(0, 2);
    print_u32(sum_e(&ve[0]) * 100 + sum_e(&ve[2]));
    // a Result<Inner, H> local reassigned across variants with a pointer held
    let mut res: Result<Inner, H> = black_box(Ok(Inner { v: 1, w: 1 }));
    let ar = a(&res);
    res = Err(H { head: 5, tail: 5 });
    print_bool(a(&res) == ar);
    res = Ok(Inner { v: 6, w: 7 });
    print_bool(a(&res) == ar);
    print_u32(match &res {
        Ok(i) => i.v * 10 + i.w,
        Err(h) => h.head,
    });
    print_u32(w.n);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_enum();
}
