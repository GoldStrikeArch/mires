//! (b) a struct with BOTH a by-value aggregate field and reference fields, assigned whole
//! through `&mut`, `ptr::write`, `mem::replace`, `mem::swap`; small (field-wise) and large
//! (runtime `_deep`/`_put_deep`) shapes; on the stack and as a heap element.
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
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

struct M<'a> {
    inner: H,
    r: &'a H,
    o: Option<&'a H>,
    n: u32,
}
// more than eight leaves, nested aggregates, arrays of refs and of aggregates
struct Big<'a> {
    inner: H,
    r: &'a H,
    refs: [&'a H; 2],
    hs: [H; 2],
    pair: (H, &'a H),
    o: Option<&'a H>,
    raw: *const H,
    n: u32,
    m: u32,
}

fn code_m(m: &M) -> u32 {
    m.inner.head * 10000 + m.inner.tail * 1000 + m.r.head * 100 + m.o.map_or(0, |h| h.head) * 10 + m.n
}
fn code_big(b: &Big) -> u32 {
    b.inner.head * 1000000
        + b.r.head * 100000
        + b.refs[0].head * 10000
        + b.refs[1].head * 1000
        + b.hs[0].head * 100
        + b.hs[1].tail * 10
        + b.pair.0.head
        + b.pair.1.tail * 10000000
        + b.o.map_or(0, |h| h.tail) * 3
        + unsafe { (*b.raw).head } * 7
        + b.n
        + b.m
}
fn targets(t: &[H; 4]) -> u32 {
    t[0].head * 1000 + t[1].head * 100 + t[2].head * 10 + t[3].head + (t[0].tail + t[1].tail + t[2].tail + t[3].tail) * 10000
}

#[inline(never)]
fn set_m<'a>(m: &mut M<'a>, v: M<'a>) {
    *m = v;
}
#[inline(never)]
fn set_big<'a>(m: &mut Big<'a>, v: Big<'a>) {
    *m = v;
}

#[inline(never)]
fn t_small() {
    print_str("small");
    let t = [mk(1), mk(2), mk(3), mk(4)];
    let mut m = black_box(M { inner: mk(5), r: &t[0], o: Some(&t[1]), n: 6 });
    let at_inner = a(&m.inner);
    let at_m = a(&m);
    let raw_inner = &raw const m.inner;
    {
        let rm = &mut m;
        *rm = M { inner: mk(7), r: &t[2], o: None, n: 8 };
    }
    print_u32(code_m(&m));
    print_u32(targets(&t));
    print_bool(at_inner == a(&m.inner));
    print_bool(at_m == a(&m));
    print_bool(a(m.r) == a(&t[2]));
    print_u32(unsafe { (*raw_inner).head });
    set_m(&mut m, M { inner: mk(9), r: &t[3], o: Some(&t[0]), n: 1 });
    print_u32(code_m(&m));
    print_u32(targets(&t));
    print_bool(at_inner == a(&m.inner));
    print_bool(ptr::eq(m.r, &t[3]) && ptr::eq(m.o.unwrap(), &t[0]));
    let pm: *mut M = &mut m;
    unsafe { ptr::write(pm, M { inner: mk(2), r: &t[1], o: Some(&t[1]), n: 3 }) };
    print_u32(code_m(&m));
    print_u32(targets(&t));
    print_bool(at_inner == a(&m.inner));
    print_bool(ptr::eq(m.r, m.o.unwrap()));
    let old = mem::replace(&mut m, M { inner: mk(4), r: &t[0], o: None, n: 5 });
    print_u32(code_m(&old));
    print_u32(code_m(&m));
    print_bool(at_inner == a(&m.inner));
    print_bool(a(&old.inner) != a(&m.inner));
    let mut m2 = black_box(M { inner: mk(6), r: &t[3], o: Some(&t[2]), n: 7 });
    let at2 = a(&m2.inner);
    mem::swap(&mut m, &mut m2);
    print_u32(code_m(&m));
    print_u32(code_m(&m2));
    print_bool(at_inner == a(&m.inner) && at2 == a(&m2.inner));
    print_bool(ptr::eq(m.r, &t[3]) && ptr::eq(m2.r, &t[0]));
    print_u32(targets(&t));
    // the reference field is rebound by a store of the by-value field next to it
    m.inner = *m.r;
    m.inner.head += 50;
    print_u32(code_m(&m));
    print_u32(targets(&t));
    print_u32(unsafe { (*raw_inner).head });
}

#[inline(never)]
fn big<'a>(t: &'a [H; 4], k: usize, seed: u32) -> Big<'a> {
    black_box(Big {
        inner: mk(seed),
        r: &t[k % 4],
        refs: [&t[(k + 1) % 4], &t[(k + 2) % 4]],
        hs: [mk(seed + 1), mk(seed + 2)],
        pair: (mk(seed + 3), &t[(k + 3) % 4]),
        o: if k % 2 == 0 { Some(&t[k % 4]) } else { None },
        raw: &t[(k + 1) % 4],
        n: seed,
        m: k as u32,
    })
}

#[inline(never)]
fn t_big() {
    print_str("big");
    let t = [mk(1), mk(2), mk(3), mk(4)];
    let mut b = big(&t, 0, 1);
    let at = (a(&b.inner), a(&b.hs[0]), a(&b.hs[1]), a(&b.pair.0), a(&b));
    let raw_h1 = &raw const b.hs[1];
    {
        let rb = &mut b;
        *rb = big(&t, 1, 2);
    }
    print_u32(code_big(&b));
    print_u32(targets(&t));
    print_bool(at == (a(&b.inner), a(&b.hs[0]), a(&b.hs[1]), a(&b.pair.0), a(&b)));
    print_u32(unsafe { (*raw_h1).head });
    set_big(&mut b, big(&t, 2, 3));
    print_u32(code_big(&b));
    print_u32(targets(&t));
    print_bool(at == (a(&b.inner), a(&b.hs[0]), a(&b.hs[1]), a(&b.pair.0), a(&b)));
    let pb: *mut Big = &mut b;
    unsafe { ptr::write(pb, big(&t, 3, 4)) };
    print_u32(code_big(&b));
    print_u32(targets(&t));
    print_bool(at == (a(&b.inner), a(&b.hs[0]), a(&b.hs[1]), a(&b.pair.0), a(&b)));
    let old = mem::replace(&mut b, big(&t, 0, 5));
    print_u32(code_big(&old));
    print_u32(code_big(&b));
    let mut b2 = big(&t, 1, 6);
    let at2 = a(&b2.hs[1]);
    mem::swap(&mut b, &mut b2);
    print_u32(code_big(&b));
    print_u32(code_big(&b2));
    print_bool(at.2 == a(&b.hs[1]) && at2 == a(&b2.hs[1]));
    print_bool(ptr::eq(b.refs[0], &t[2]) && ptr::eq(b2.refs[0], &t[1]));
    print_u32(targets(&t));
    print_u32(unsafe { (*raw_h1).head });
    // no sharing between the two after the swap
    b.hs[0].head += 100;
    b.pair.0.tail += 100;
    print_u32(code_big(&b2));
    print_u32(targets(&t));
}

#[inline(never)]
fn t_heap() {
    print_str("heap");
    let t = [mk(1), mk(2), mk(3), mk(4)];
    let mut v: Vec<M> = vec![
        M { inner: mk(5), r: &t[0], o: Some(&t[1]), n: 1 },
        M { inner: mk(6), r: &t[2], o: None, n: 2 },
    ];
    let e0 = a(&v[0].inner);
    v[0] = M { inner: mk(7), r: &t[3], o: Some(&t[2]), n: 3 };
    print_u32(code_m(&v[0]));
    print_u32(targets(&t));
    print_bool(e0 == a(&v[0].inner));
    v.swap(0, 1);
    print_u32(code_m(&v[0]));
    print_u32(code_m(&v[1]));
    print_u32(targets(&t));
    print_bool(ptr::eq(v[0].r, &t[2]) && ptr::eq(v[1].r, &t[3]));
    v.push(M { inner: mk(8), r: &t[1], o: Some(&t[0]), n: 4 });
    v.reverse();
    v.sort_by_key(|m| m.n);
    print_u32(code_m(&v[0]) + code_m(&v[1]) * 2 + code_m(&v[2]) * 3);
    print_u32(targets(&t));
    let popped = v.remove(0);
    v.insert(1, popped);
    print_u32(code_m(&v[0]) + code_m(&v[1]) * 2 + code_m(&v[2]) * 3);
    let mut bx = Box::new(big(&t, 0, 1));
    *bx = big(&t, 1, 2);
    print_u32(code_big(&bx));
    let mut vb: Vec<Big> = vec![big(&t, 2, 3), big(&t, 3, 4)];
    vb[1] = big(&t, 0, 5);
    vb.swap(0, 1);
    print_u32(code_big(&vb[0]));
    print_u32(code_big(&vb[1]));
    print_u32(targets(&t));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_small();
    t_big();
    t_heap();
}
