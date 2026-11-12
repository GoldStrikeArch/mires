//! (i) closures capturing `&mut o.h` (FnMut called repeatedly, reassigning it), captured by
//! value, stored in a struct, passed to a callee: identity stable across calls, values right.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct N {
    head: u32,
    tail: u32,
}
struct Outer {
    n: u32,
    h: H,
    m: N,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
#[inline(never)]
fn mkn(head: u32) -> N {
    black_box(N { head, tail: head + 1 })
}
fn a<T>(r: &T) -> usize {
    r as *const T as usize
}
#[inline(never)]
fn call3<F: FnMut(u32) -> usize>(mut f: F) -> (usize, usize, usize) {
    (f(1), f(2), f(3))
}
#[inline(never)]
fn call_dyn(f: &mut dyn FnMut(u32) -> usize, i: u32) -> usize {
    f(i)
}
struct Keeper<F: FnMut(u32) -> usize> {
    f: F,
    calls: u32,
}
impl<F: FnMut(u32) -> usize> Keeper<F> {
    fn go(&mut self, i: u32) -> usize {
        self.calls += 1;
        (self.f)(i)
    }
}

#[inline(never)]
fn t_mut_capture() {
    print_str("mut_capture");
    let mut o = black_box(Outer { n: 1, h: mk(1), m: mkn(1) });
    let outside = a(&o.h);
    let outside_m = a(&o.m);
    {
        let h = &mut o.h;
        let mut f = |i: u32| {
            *h = mk(i * 10);
            a(h)
        };
        let (x, y, z) = (f(1), f(2), f(3));
        print_bool(x == outside && y == outside && z == outside);
    }
    print_u32(o.h.head);
    {
        let mut f = |i: u32| {
            o.h = H { head: o.h.tail + i, tail: o.h.head };
            o.m = N { head: o.m.tail + i, tail: o.m.head };
            a(&o.h)
        };
        let (x, y, z) = call3(&mut f);
        print_bool(x == outside && y == outside && z == outside);
        print_bool(call_dyn(&mut f, 4) == outside);
    }
    print_u32(o.h.head * 100 + o.h.tail);
    print_u32(o.m.head * 100 + o.m.tail);
    print_bool(a(&o.m) == outside_m);
    {
        let mut k = Keeper {
            f: |i: u32| {
                let old = mem::replace(&mut o.m, mkn(i));
                o.n += old.head;
                a(&o.m)
            },
            calls: 0,
        };
        let (x, y) = (k.go(5), k.go(6));
        print_bool(x == outside_m && y == outside_m);
        print_u32(k.calls);
    }
    print_u32(o.n * 100 + o.m.head);
    let mut boxed: Box<dyn FnMut(u32) -> usize + '_> = Box::new(|i| {
        o.h = mk(i);
        a(&o.h)
    });
    let (x, y) = (boxed(7), boxed(8));
    drop(boxed);
    print_bool(x == outside && y == outside);
    print_u32(o.h.head);
}

#[inline(never)]
fn t_by_value() {
    print_str("by_value");
    let h = mk(1);
    let mut inside: Vec<usize> = Vec::new();
    let mut f = move |i: u32| {
        let mut h = h;
        h = mk(h.head + i);
        h.head
    };
    print_u32(f(1) * 10 + f(2));
    print_u32(h.head);
    let mut hm = mk(5);
    let mut g = move |i: u32| {
        hm = H { head: hm.tail + i, tail: hm.head };
        (a(&hm), hm.head)
    };
    let (p1, v1) = g(1);
    let (p2, v2) = g(2);
    let (p3, v3) = g(3);
    inside.push(p1);
    print_bool(p1 == p2 && p2 == p3);
    print_u32(v1 * 10000 + v2 * 100 + v3);
    print_u32(hm.head);
    let mut nm = mkn(5);
    let at = a(&nm);
    let mut k = |i: u32| {
        let old = mem::replace(&mut nm, mkn(0));
        nm = mkn(old.head + i);
        (a(&nm), old.head)
    };
    let (q1, w1) = k(1);
    let (q2, w2) = k(2);
    print_bool(q1 == q2 && q1 == at);
    print_u32(w1 * 100 + w2);
    print_u32(nm.head);
    print_bool(inside.len() == 1);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_mut_capture();
    t_by_value();
}
