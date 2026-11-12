//! (b) aliasing through `&mut`: stores through a reborrow, swaps, replaces keep one place.
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
struct Pair {
    a: H,
    b: H,
}
struct Big {
    n: u32,
    h: H,
    v: Vec<u32>,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn sw(x: &mut H, y: &mut H) {
    mem::swap(x, y)
}

#[inline(never)]
fn set_whole(b: &mut Big, v: Big) {
    *b = v;
}

#[inline(never)]
fn e_reborrow() {
    let mut o = black_box(Pair { a: H { head: 1, tail: 2 }, b: H { head: 3, tail: 4 } });
    let at_a = a(&o.a);
    let at_b = a(&o.b);
    let r = &mut o.a;
    *r = H { head: 10, tail: 20 };
    let ra = r as *const H as usize;
    r.head += 1;
    print_bool(ra == at_a);
    print_bool(ra == a(&o.a));
    print_u32(o.a.head);
    mem::swap(&mut o.a, &mut o.b);
    print_bool(a(&o.a) == at_a);
    print_bool(a(&o.b) == at_b);
    print_u32(o.a.head * 100 + o.b.head);
    sw(&mut o.a, &mut o.b);
    print_u32(o.a.head * 100 + o.b.head);
    print_bool(a(&o.a) == at_a);
    unsafe { ptr::swap(&raw mut o.a, &raw mut o.b) };
    print_u32(o.a.head * 100 + o.b.head);
    let old = unsafe { ptr::replace(&raw mut o.a, H { head: 7, tail: 8 }) };
    print_u32(old.head * 100 + o.a.head);
    print_bool(a(&o.a) == at_a);
    print_u32(o.a.tail + o.b.tail);
}

#[inline(never)]
fn e_heap() {
    let mut x = black_box(Big { n: 1, h: H { head: 1, tail: 1 }, v: vec![1, 2, 3] });
    let mut y = black_box(Big { n: 2, h: H { head: 2, tail: 2 }, v: vec![9] });
    let (ax, ay) = (a(&x), a(&y));
    let (ahx, ahy) = (a(&x.h), a(&y.h));
    mem::swap(&mut x, &mut y);
    print_u32(x.n * 10 + y.n);
    print_usize(x.v.len() * 10 + y.v.len());
    print_u32(x.v[0] + y.v[2]);
    print_bool(ax == a(&x));
    print_bool(ay == a(&y));
    print_bool(ahx == a(&x.h));
    print_bool(ahy == a(&y.h));
    x.v.push(4);
    y.v.push(5);
    print_usize(x.v.len() + y.v.len());
    print_u32(x.h.head * 10 + y.h.head);
    set_whole(&mut x, Big { n: 5, h: H { head: 5, tail: 5 }, v: vec![7, 7] });
    print_bool(ax == a(&x));
    print_bool(ahx == a(&x.h));
    print_u32(x.n + x.v[1] + x.h.tail);
    let mut vs: Vec<Pair> = vec![
        Pair { a: H { head: 1, tail: 1 }, b: H { head: 2, tail: 2 } },
        Pair { a: H { head: 3, tail: 3 }, b: H { head: 4, tail: 4 } },
    ];
    let r = &mut vs[1].b;
    *r = H { head: 42, tail: 0 };
    let rr = r as *const H as usize;
    print_bool(rr == a(&vs[1].b));
    print_u32(vs[1].b.head);
    let e = &mut vs[0];
    *e = Pair { a: H { head: 8, tail: 8 }, b: H { head: 9, tail: 9 } };
    let ee = e as *const Pair as usize;
    print_bool(ee == a(&vs[0]));
    print_u32(vs[0].a.head + vs[0].b.head);
    // swap two Vec elements' fields through split_at_mut
    let (lo, hi) = vs.split_at_mut(1);
    mem::swap(&mut lo[0].a, &mut hi[0].b);
    print_u32(vs[0].a.head * 100 + vs[1].b.head);
    print_bool(a(&vs[1].b) == rr);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_reborrow();
    e_heap();
}
