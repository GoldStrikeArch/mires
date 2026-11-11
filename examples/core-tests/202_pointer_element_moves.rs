//! Moving a pointer-valued element rebinds the place and never writes into the pointee.
//!
//! A reference to an aggregate is the aggregate's own JavaScript object, and a raw pointer or a
//! `Box` is a slot record, so an element of an array or a `Vec` of `&P`, `*const P`,
//! `Option<&P>` or `Box<P>` holds an object that names *something else*. `ptr::swap`,
//! `slice::swap`, `reverse` and the sorts move such elements through `ptr::copy` and
//! `ptr::copy_nonoverlapping`, and a move that wrote the new element into the object already in
//! the place would overwrite the pointee: `[&h, &h2].swap(0, 1)` used to copy `h2` over `h`.
//! `mem::swap`, `mem::replace`, `ptr::write` and `ptr::replace` are exercised beside them.
//!
//! The controls are the elements that ARE written through: aggregates held by value, moved the
//! same ways. A raw pointer to a nested field of an element, taken before the element is moved,
//! still names that element's field afterwards, and a future held by value and written through a
//! raw pointer is seen by the local it lives in. Every pointee is printed after the moves, so a
//! move that touched one shows up as a changed value rather than only as a changed address.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::hint::black_box;
use core::mem;
use core::pin::Pin;
use core::ptr;
use core::task::{Context, Poll, Waker};

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 100 })
}

/// Every pointee's two fields, one digit each, so that an overwritten `P` changes the number.
fn vals(a: &P, b: &P, c: &P, d: &P) -> u32 {
    let ys = a.y + b.y + c.y + d.y - 400 - a.x - b.x - c.x - d.x;
    a.x * 1000 + b.x * 100 + c.x * 10 + d.x + ys * 100000
}

/// The pointees of a run of references, in order.
fn refs(ps: &[&P]) -> u32 {
    let mut s = 0;
    for p in ps {
        s = s * 10 + p.x;
    }
    s
}

/// The pointees of a run of raw pointers, in order.
fn raws(ps: &[*const P]) -> u32 {
    let mut s = 0;
    for p in ps {
        s = s * 10 + unsafe { (**p).x };
    }
    s
}

/// The pointees of a run of optional references, in order; `9` for a `None`.
fn opts(ps: &[Option<&P>]) -> u32 {
    let mut s = 0;
    for p in ps {
        s = s * 10 + p.map_or(9, |p| p.x);
    }
    s
}

#[inline(never)]
fn reference_elements() {
    print_str("references");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut a: [&P; 2] = black_box([&p1, &p2]);
    a.swap(0, 1);
    print_u32(p1.x * 10 + p2.x);
    print_u32(a[0].x * 10 + a[1].x);
    print_bool(ptr::eq(a[0], &p2) && ptr::eq(a[1], &p1));
    let mut v: Vec<&P> = vec![&p1, &p2, &p3, &p4];
    v.swap(0, 3);
    print_u32(refs(&v));
    v.reverse();
    print_u32(refs(&v));
    v.sort_by_key(|r| 10 - r.x);
    print_u32(refs(&v));
    print_bool(ptr::eq(v[0], &p4) && ptr::eq(v[3], &p1));
    print_u32(vals(&p1, &p2, &p3, &p4));
    let old = mem::replace(&mut v[0], &p1);
    print_u32(old.x * 10 + v[0].x);
    let (left, right) = v.split_at_mut(2);
    mem::swap(&mut left[1], &mut right[0]);
    print_u32(refs(&v));
    unsafe { ptr::swap(&mut v[0], &mut v[3]) };
    print_u32(refs(&v));
    unsafe { ptr::write(&mut v[1], &p4) };
    print_u32(refs(&v));
    v.insert(1, &p3);
    let removed = v.remove(4);
    print_u32(removed.x * 100000 + refs(&v));
    let pv = v.as_mut_ptr();
    unsafe { ptr::copy(pv, pv.add(1), 3) };
    print_u32(refs(&v));
    let mut w: [&P; 4] = [&p4, &p3, &p2, &p1];
    unsafe { ptr::copy_nonoverlapping(w.as_ptr(), v.as_mut_ptr(), 4) };
    print_u32(refs(&v));
    w.swap(1, 2);
    print_u32(refs(&w) * 10000 + refs(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
}

#[inline(never)]
fn raw_elements() {
    print_str("raw pointers");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut v: Vec<*const P> = vec![&p1, &p2, &p3, &p4];
    v.swap(0, 2);
    print_u32(raws(&v));
    v.reverse();
    print_u32(raws(&v));
    v.swap(0, 1);
    v.swap(1, 2);
    v.swap(2, 3);
    print_u32(raws(&v));
    let old = mem::replace(&mut v[1], &p4);
    print_u32(unsafe { (*old).x } * 10000 + raws(&v));
    print_bool(ptr::eq(v[1], &p4) && ptr::eq(old, &p2));
    let mut a: [*const P; 2] = [&p1, &p2];
    a.swap(0, 1);
    unsafe { ptr::swap(&mut a[0], &mut v[0]) };
    print_u32(raws(&a) * 10000 + raws(&v));
    let mut r: *const P = &p3;
    let mut s: *const P = &p4;
    mem::swap(&mut r, &mut s);
    unsafe { ptr::write(&mut v[3], r) };
    print_u32(unsafe { (*r).x * 10 + (*s).x } * 10000 + raws(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
}

#[inline(never)]
fn option_elements() {
    print_str("options");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut v: Vec<Option<&P>> = vec![Some(&p1), None, Some(&p3), Some(&p4)];
    v.swap(0, 1);
    v.swap(2, 3);
    print_u32(opts(&v));
    v.reverse();
    print_u32(opts(&v));
    let taken = v[0].take();
    print_u32(taken.map_or(0, |p| p.x) * 10000 + opts(&v));
    let old = mem::replace(&mut v[1], Some(&p2));
    print_u32(old.map_or(0, |p| p.x) * 10000 + opts(&v));
    let mut a: [Option<&P>; 2] = [None, Some(&p2)];
    a.swap(0, 1);
    unsafe { ptr::swap(&mut a[1], &mut v[2]) };
    print_u32(opts(&a) * 10000 + opts(&v));
    print_bool(ptr::eq(a[1].unwrap(), &p1) && ptr::eq(v[1].unwrap(), &p2) && v[2].is_none());
    print_u32(vals(&p1, &p2, &p3, &p4));
}

#[inline(never)]
fn box_elements() {
    print_str("boxes");
    let mut v: Vec<Box<P>> = vec![Box::new(mk(1)), Box::new(mk(2)), Box::new(mk(3))];
    let first = &*v[0] as *const P;
    let third = &*v[2] as *const P;
    v.swap(0, 2);
    print_bool(ptr::eq(&*v[2], first) && ptr::eq(&*v[0], third));
    v.reverse();
    print_bool(ptr::eq(&*v[0], first));
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x);
    let old = mem::replace(&mut v[2], Box::new(mk(7)));
    print_u32(old.x * 10 + v[2].x);
    print_bool(ptr::eq(&*old, third));
    let mut b = Box::new(mk(8));
    mem::swap(&mut b, &mut v[0]);
    print_bool(ptr::eq(&*b, first));
    unsafe { ptr::swap(&mut v[0], &mut v[1]) };
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x + b.x * 1000);
    print_u32(unsafe { (*first).x * 10 + (*third).x });
    *v[0] = mk(9);
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x + b.x * 1000 + old.y);
}

#[inline(never)]
fn reference_locals() {
    print_str("locals");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut r: &P = &p1;
    let mut s: &P = &p2;
    mem::swap(&mut r, &mut s);
    print_u32(r.x * 10 + s.x);
    unsafe { ptr::swap(&mut r, &mut s) };
    print_u32(r.x * 10 + s.x);
    let was = mem::replace(&mut r, &p3);
    print_u32(was.x * 10 + r.x);
    unsafe { ptr::write(&mut s, &p4) };
    print_u32(r.x * 10 + s.x);
    // `r` and `s` live in boxed cells (their addresses were taken), so each is read as `cell[0]`.
    // The raw pointer a reference becomes is its pointee's, never the cell's slot.
    print_bool(ptr::eq(r, &p3) && ptr::eq(s, &p4));
    let slot: &mut &P = black_box(&mut r);
    *slot = &p2;
    print_bool(ptr::eq(r, &p2) && r as *const P == &p2 as *const P);
    print_u32(vals(&p1, &p2, &p3, &p4));
}

/// By value: the elements themselves move, and every one of them is written through.
#[inline(never)]
fn value_elements() {
    print_str("values");
    let mut a = [mk(1), mk(2), mk(3), mk(4)];
    let pa = &raw const a[1];
    a.swap(0, 1);
    print_u32(unsafe { (*pa).x });
    a.reverse();
    print_u32(a[0].x * 1000 + a[1].x * 100 + a[2].x * 10 + a[3].x);
    let mut v: Vec<P> = vec![mk(5), mk(6), mk(7)];
    v.swap(0, 2);
    let old = mem::replace(&mut v[1], mk(8));
    mem::swap(&mut a[0], &mut v[0]);
    unsafe { ptr::swap(&mut a[3], &mut v[2]) };
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x + old.x * 1000);
    print_u32(a[0].x * 1000 + a[1].x * 100 + a[2].x * 10 + a[3].x);
    v.reverse();
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x);
    print_u32(v[0].y + v[1].y + v[2].y - 300);
}

#[derive(Clone, Copy)]
struct D {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct A {
    k: u32,
    d: D,
}

#[inline(never)]
fn mka(s: u32) -> A {
    black_box(A { k: s, d: D { x: s * 10, y: s * 10 + 1 } })
}

/// A pointer to an element's nested field, taken before the element is moved, still names that
/// element's field: the move writes into the element rather than replacing it.
#[inline(never)]
fn nested(base: *mut A) {
    unsafe {
        let e1 = base.add(1);
        let raw_d = &raw const (*e1).d;
        print_u32((*raw_d).x);
        ptr::swap(base, e1);
        print_u32((*e1).k * 1000 + (*raw_d).x);
        ptr::copy(base.add(2), e1, 1);
        print_u32((*e1).k * 1000 + (*raw_d).x);
        ptr::copy_nonoverlapping(base, e1, 1);
        print_u32((*raw_d).x);
        e1.write(mka(9));
        print_u32((*raw_d).x);
        let old = ptr::replace(e1, mka(8));
        print_u32((*raw_d).x * 1000 + old.d.x);
        print_bool(ptr::eq(raw_d, &raw const (*e1).d));
    }
}

#[inline(never)]
fn nested_elements() {
    print_str("nested");
    let mut arr = [mka(1), mka(2), mka(3)];
    nested(arr.as_mut_ptr());
    print_u32(arr[0].d.x * 10000 + arr[1].d.x * 100 + arr[2].d.x);
    let mut v: Vec<A> = vec![mka(1), mka(2), mka(3)];
    nested(v.as_mut_ptr());
    print_u32(v[0].d.x * 10000 + v[1].d.x * 100 + v[2].d.x);
}

async fn val(n: u32) -> u32 {
    n
}

#[inline(never)]
fn make(n: u32) -> impl Future<Output = u32> {
    val(n + 100)
}

fn poll_now<F: Future<Output = u32>>(f: &mut F) -> u32 {
    let mut cx = Context::from_waker(Waker::noop());
    match unsafe { Pin::new_unchecked(f) }.poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => 0,
    }
}

/// A future is an aggregate held by value: a store through a raw pointer to the local reaches it,
/// and so does an element move into it, although a future has no clone for the move to carry.
#[inline(never)]
fn future_local() {
    print_str("future");
    let mut f = make(1);
    let p = &raw mut f;
    unsafe { p.write(make(3)) };
    print_u32(poll_now(&mut f));
    let mut g = make(4);
    let h = make(5);
    unsafe { ptr::copy_nonoverlapping(&h, &mut g, 1) };
    mem::forget(h);
    print_u32(poll_now(&mut g));
}

/// A byte copy of an aggregate through `*mut u8` writes into the element it lands on: the bytes
/// are copied, so the source stays a separate value.
#[inline(never)]
fn byte_copy() {
    print_str("bytes");
    let mut src = mk(5);
    let mut arr = [mk(1), mk(2)];
    unsafe {
        ptr::copy_nonoverlapping(
            &src as *const P as *const u8,
            arr.as_mut_ptr() as *mut u8,
            mem::size_of::<P>(),
        )
    };
    src.x = 9;
    print_u32(arr[0].x * 10 + arr[1].x + src.x * 100);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    reference_elements();
    raw_elements();
    option_elements();
    box_elements();
    reference_locals();
    value_elements();
    nested_elements();
    future_local();
    byte_copy();
}
