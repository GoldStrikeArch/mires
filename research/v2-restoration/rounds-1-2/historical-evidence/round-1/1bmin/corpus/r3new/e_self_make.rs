//! (e) `*r = make(*r)`: a place rebuilt from its own old value, by copy and by move, in loops,
//! through references, boxes, vec elements and options; moved-out values keep their contents.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[derive(Clone, Copy)]
struct S {
    a: u32,
    b: u32,
}
struct N {
    a: u32,
    b: u32,
}
struct Outer {
    k: u32,
    n: N,
    s: S,
    opt: Option<N>,
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}
#[inline(never)]
fn step(s: S, i: u32) -> S {
    S { a: s.b + i, b: s.a }
}
#[inline(never)]
fn stepn(s: N, i: u32) -> N {
    N { a: s.b + i, b: s.a }
}
#[inline(never)]
fn make(r: &S) -> S {
    S { a: r.a + r.b, b: r.a }
}
#[inline(never)]
fn mkn(a: u32) -> N {
    black_box(N { a, b: a + 1 })
}
#[inline(never)]
fn rebuild(r: &mut S) {
    *r = make(r);
}
#[inline(never)]
fn rebuild_copy(r: &mut S) {
    let old = *r;
    *r = S { a: old.b * 2, b: old.a + r.a };
}
#[inline(never)]
fn rotate(o: &mut Outer, n: N) -> N {
    mem::replace(&mut o.n, n)
}

#[inline(never)]
fn t_copy() {
    print_str("copy");
    let mut s = black_box(S { a: 1, b: 2 });
    let at = ad(&s);
    let r = &mut s;
    *r = make(r);
    print_u32(r.a * 10 + r.b);
    *r = step(*r, 1);
    print_u32(r.a * 10 + r.b);
    for i in 0..5 {
        *r = step(*r, i);
    }
    print_u32(r.a * 100 + r.b);
    rebuild(r);
    rebuild_copy(r);
    print_u32(r.a * 100 + r.b);
    print_bool(at == ad(&s));
    let mut o = black_box(Outer { k: 1, n: mkn(1), s: S { a: 1, b: 2 }, opt: None });
    let ats = ad(&o.s);
    for i in 0..4 {
        o.s = step(o.s, i);
        o.s = make(&o.s);
    }
    print_u32(o.s.a * 100 + o.s.b);
    print_bool(ats == ad(&o.s));
}

#[inline(never)]
fn t_move() {
    print_str("move");
    let mut o = black_box(Outer { k: 1, n: mkn(1), s: S { a: 1, b: 2 }, opt: None });
    let atn = ad(&o.n);
    let mut seen: Vec<N> = Vec::new();
    for i in 0..4 {
        o.n = stepn(o.n, i);
    }
    print_u32(o.n.a * 100 + o.n.b);
    print_bool(atn == ad(&o.n));
    for i in 0..3 {
        let old = o.n;
        o.n = mkn(10 + i);
        seen.push(old);
    }
    print_u32(seen[0].a * 10000 + seen[1].a * 100 + seen[2].a);
    print_u32(o.n.a);
    print_bool(atn == ad(&o.n));
    let old = rotate(&mut o, mkn(20));
    let old2 = rotate(&mut o, mkn(30));
    print_u32(old.a * 10000 + old2.a * 100 + o.n.a);
    // a borrowed local moved out and assigned again, in a loop
    let mut cur = mkn(0);
    let atc = ad(&cur);
    let mut out: Vec<N> = Vec::new();
    for i in 1..4 {
        out.push(cur);
        cur = mkn(i * 2);
    }
    print_u32(out[0].a * 100 + out[1].a * 10 + out[2].a);
    print_u32(cur.a);
    print_bool(atc == ad(&cur));
    // struct update syntax over the same place
    o = Outer { k: 5, ..o };
    print_u32(o.k * 1000 + o.n.a * 10 + o.s.a);
    let mut w = black_box((Outer { k: 1, n: mkn(1), s: S { a: 1, b: 2 }, opt: None }, 3u32));
    let atw = ad(&w.0.n);
    w.0 = Outer { k: 7, s: S { a: 9, b: 9 }, ..w.0 };
    print_u32(w.0.k * 1000 + w.0.n.a * 100 + w.0.s.a + w.1);
    print_bool(atw == ad(&w.0.n));
}

#[inline(never)]
fn t_heap() {
    print_str("heap");
    let mut b = Box::new(mkn(1));
    for i in 0..3 {
        *b = stepn(*b, i);
    }
    print_u32(b.a * 100 + b.b);
    let mut bs = Box::new(black_box(S { a: 1, b: 2 }));
    *bs = make(&bs);
    *bs = step(*bs, 4);
    print_u32(bs.a * 100 + bs.b);
    let mut v: Vec<S> = vec![S { a: 1, b: 2 }, S { a: 3, b: 4 }];
    let e1 = ad(&v[1]);
    for i in 0..3 {
        v[1] = step(v[1], i);
        v[0] = make(&v[1]);
    }
    print_u32(v[0].a * 1000 + v[0].b * 100 + v[1].a * 10 + v[1].b);
    print_bool(e1 == ad(&v[1]));
    let mut vn: Vec<N> = vec![mkn(1), mkn(5)];
    let last = vn.pop().unwrap();
    vn.push(stepn(last, 1));
    let first = vn.remove(0);
    vn.push(stepn(first, 2));
    print_u32(vn[0].a * 1000 + vn[0].b * 100 + vn[1].a * 10 + vn[1].b);
    let mut o = black_box(Outer { k: 1, n: mkn(1), s: S { a: 1, b: 2 }, opt: Some(mkn(3)) });
    o.opt = o.opt.take().map(|n| stepn(n, 1));
    print_u32(o.opt.as_ref().map_or(0, |n| n.a * 10 + n.b));
    let t = o.opt.take();
    print_bool(o.opt.is_none());
    o.opt = Some(mkn(8));
    print_u32(t.map_or(0, |n| n.a) * 10 + o.opt.as_ref().map_or(0, |n| n.a));
    o.opt = None;
    o.opt = Some(mkn(6));
    let held = o.opt;
    o.opt = Some(mkn(2));
    print_u32(held.map_or(0, |n| n.a) * 10 + o.opt.as_ref().map_or(0, |n| n.a));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_copy();
    t_move();
    t_heap();
}
