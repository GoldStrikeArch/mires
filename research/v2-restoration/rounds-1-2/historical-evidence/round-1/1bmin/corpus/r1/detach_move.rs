//! (a) detached moves: a struct moved out of a place that is later reinitialized must be a copy.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use core::hint::black_box;
use core::mem;
use prelude::*;

struct Inner {
    v: u32,
    w: u32,
}
impl Default for Inner {
    fn default() -> Self {
        Inner { v: 0, w: 0 }
    }
}
struct Mid {
    inner: Inner,
    k: u32,
}
struct Outer {
    n: u32,
    mid: Mid,
    inner: Inner,
    opt: Option<Inner>,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn mk(v: u32) -> Inner {
    black_box(Inner { v, w: v * 10 })
}

fn fresh(seed: u32) -> Outer {
    black_box(Outer {
        n: seed,
        mid: Mid { inner: mk(seed + 1), k: 5 },
        inner: mk(seed + 2),
        opt: Some(mk(seed + 3)),
    })
}

#[inline(never)]
fn e_field() {
    let mut o = fresh(1);
    let at = a(&o.inner);
    let mut x = o.inner;
    o.inner = mk(20);
    x.v += 1;
    print_u32(x.v);
    print_u32(x.w);
    print_u32(o.inner.v);
    print_u32(o.inner.w);
    print_bool(a(&o.inner) == at);
    print_bool(a(&x) != a(&o.inner));
    // nested: move the inner of mid, reassign mid whole
    let mut y = o.mid.inner;
    o.mid = Mid { inner: mk(30), k: 6 };
    y.v += 1;
    print_u32(y.v);
    print_u32(o.mid.inner.v + o.mid.k);
    // move mid whole, reassign, mutate nested
    let mut m = o.mid;
    o.mid = Mid { inner: mk(40), k: 7 };
    m.inner.v += 1;
    m.k += 1;
    print_u32(m.inner.v);
    print_u32(m.k);
    print_u32(o.mid.inner.v);
    print_u32(o.mid.k);
    // Option::take then reassign
    let mut t = o.opt.take().unwrap();
    o.opt = Some(mk(50));
    t.v += 1;
    print_u32(t.v);
    print_u32(o.opt.as_ref().unwrap().v);
    // match-move out of the option, then reassign
    let mut u = match o.opt {
        Some(i) => i,
        None => mk(0),
    };
    o.opt = Some(mk(55));
    u.v += 1;
    print_u32(u.v);
    print_u32(o.opt.as_ref().unwrap().v);
    // mem::replace
    let mut r = mem::replace(&mut o.inner, mk(60));
    r.v += 1;
    print_u32(r.v);
    print_u32(o.inner.v);
    // mem::take
    let mut k = mem::take(&mut o.inner);
    k.v += 1;
    print_u32(k.v);
    print_u32(o.inner.v);
    o.inner = mk(70);
    print_bool(a(&o.inner) == at);
    print_u32(o.inner.v + o.n);
}

#[inline(never)]
fn e_loop() {
    let mut o = fresh(2);
    let mut sum = 0;
    for i in 0..4u32 {
        let mut x = o.inner;
        o.inner = mk(x.v + i);
        x.v += 100;
        sum += x.v + o.inner.v;
    }
    print_u32(sum);
}

#[inline(never)]
fn e_local() {
    let mut l = mk(1);
    let at = a(&l);
    let mut b = l;
    l = mk(2);
    b.v += 100;
    l.v += 1;
    print_u32(b.v);
    print_u32(l.v);
    print_bool(at == a(&l));
    let c = l;
    l = mk(3);
    print_u32(c.v + l.v);
    print_bool(at == a(&l));
}

#[inline(never)]
fn e_box() {
    let mut bx = Box::new(mk(5));
    let at = a(&*bx);
    let mut x = *bx;
    *bx = mk(6);
    x.v += 1;
    print_u32(x.v);
    print_u32(bx.v);
    print_bool(a(&*bx) == at);
}

#[inline(never)]
fn e_raw_before() {
    let mut o = fresh(3);
    let at = a(&o.inner);
    let p = &raw mut o.inner;
    let mut x = o.inner;
    unsafe { p.write(mk(9)) };
    x.v += 1;
    print_u32(x.v);
    // borrowck still thinks `o.inner` is moved out, so the place is read through the pointer
    print_u32(unsafe { (*p).v });
    print_bool(p as usize == at);
    print_u32(o.n);
}

#[inline(never)]
fn e_closure_body() {
    let mut o = fresh(4);
    // Moving out of a captured place makes the closure `FnOnce`; the loop inside it repeats the
    // move and the reinitialization within one body.
    let f = move || {
        let mut acc = 0;
        for i in 0..3u32 {
            let mut x = o.inner;
            o.inner = mk(x.v + i);
            x.v += 50;
            acc += x.v * 1000 + o.inner.v;
        }
        acc
    };
    print_u32(f());
}

#[inline(never)]
fn eat(i: Inner) -> u32 {
    i.v
}

#[inline(never)]
fn e_move_to_callee() {
    let mut o = fresh(5);
    let s = eat(o.inner);
    o.inner = mk(77);
    print_u32(s + o.inner.v);
    // the moved field passed to a callee that keeps mutating its own copy
    let mut keep = Inner { v: 0, w: 0 };
    stash(o.inner, &mut keep);
    o.inner = mk(88);
    keep.v += 1;
    print_u32(keep.v);
    print_u32(o.inner.v);
}

#[inline(never)]
fn stash(i: Inner, out: &mut Inner) {
    *out = i;
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_field();
    e_loop();
    e_local();
    e_box();
    e_raw_before();
    e_closure_body();
    e_move_to_callee();
}
