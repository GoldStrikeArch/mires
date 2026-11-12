//! (l) a field of a local aggregate saved, overwritten, and restored: the restoring store
//! writes back a value READ from that same field earlier. It is not a no-op.
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
struct N {
    head: u32,
    tail: u32,
}
struct S<'a> {
    a: u32,
    b: u32,
    r: &'a H,
    h: H,
    n: N,
    t: (u32, u32),
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
#[inline(never)]
fn mkn(head: u32) -> N {
    black_box(N { head, tail: head + 1 })
}
#[inline(never)]
fn look(s: &S) -> u32 {
    s.a * 1000 + s.b * 100 + s.r.head * 10 + s.t.0
}
#[inline(never)]
fn bump(s: &mut S) {
    s.a += 1;
}

#[inline(never)]
fn t_scalar() {
    print_str("scalar");
    let (h1, h2) = (mk(1), mk(2));
    let mut o = black_box(S { a: 1, b: 2, r: &h1, h: mk(3), n: mkn(4), t: (5, 6) });
    let saved = o.a;
    o.a = 7;
    print_u32(look(&o));
    o.a = saved;
    print_u32(look(&o));
    let saved_r = o.r;
    o.r = &h2;
    print_u32(look(&o));
    o.r = saved_r;
    print_u32(look(&o));
    let saved_t = o.t.0;
    o.t.0 = 9;
    print_u32(look(&o));
    o.t.0 = saved_t;
    print_u32(look(&o));
    // saved, mutated through a reference in a callee, restored
    let saved = o.a;
    bump(&mut o);
    bump(&mut o);
    print_u32(o.a);
    o.a = saved;
    print_u32(o.a);
    // in a loop
    let mut acc = 0;
    for i in 0..3 {
        let keep = o.b;
        o.b = i * 10;
        acc += look(&o);
        o.b = keep;
    }
    print_u32(acc);
    print_u32(look(&o));
    // conditional overwrite then restore
    let keep = o.a;
    if black_box(true) {
        o.a = 8;
    }
    let mid = o.a;
    o.a = keep;
    print_u32(mid * 10 + o.a);
}

#[inline(never)]
fn t_aggregate() {
    print_str("aggregate");
    let h1 = mk(1);
    let mut o = black_box(S { a: 1, b: 2, r: &h1, h: mk(3), n: mkn(4), t: (5, 6) });
    let saved = o.h;
    o.h = mk(7);
    print_u32(o.h.head);
    o.h = saved;
    print_u32(o.h.head * 10 + o.h.tail);
    let saved_n = o.n;
    o.n = mkn(8);
    print_u32(o.n.head);
    o.n = saved_n;
    print_u32(o.n.head * 10 + o.n.tail);
    let saved_t = o.t;
    o.t = (0, 0);
    print_u32(o.t.0 + o.t.1);
    o.t = saved_t;
    print_u32(o.t.0 * 10 + o.t.1);
    let saved_head = o.h.head;
    o.h.head = 0;
    o.h.tail = 0;
    print_u32(o.h.head);
    o.h.head = saved_head;
    print_u32(o.h.head * 10 + o.h.tail);
}

#[inline(never)]
fn t_tuple_local() {
    print_str("tuple");
    let mut t = black_box((1u32, 2u32, mk(3)));
    let s0 = t.0;
    t.0 = t.1;
    t.1 = s0;
    print_u32(t.0 * 10 + t.1);
    let s0 = t.0;
    t.0 = 9;
    let mid = t.0;
    t.0 = s0;
    print_u32(mid * 10 + t.0);
    let mut arr = black_box([mk(1), mk(2)]);
    let s = arr[0].head;
    arr[0].head = 5;
    let mid = arr[0].head;
    arr[0].head = s;
    print_u32(mid * 10 + arr[0].head);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_scalar();
    t_aggregate();
    t_tuple_local();
}
