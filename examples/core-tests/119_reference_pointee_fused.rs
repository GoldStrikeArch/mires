//! A reference read out of a place, made into a raw pointer inside a small body the fusion planner
//! spells as one expression, is the pointer it holds: the companion of
//! `116_reference_field_fusion` for the bodies that fuse under the release profiles.
//!
//! With `js-cleanup=on` nearly every body has a cleanup edge and is kept out of fusion, which is
//! why `116` cannot tell a broken fusion from a correct one there. A body with no call that can
//! unwind has none: a `#[inline(never)]` accessor (`a.r as *const P`), a reference read through a
//! reference (`*pp`), an element of an array of references, an `Option<&P>` match, a closure
//! returning the reference it captured. Each is spelled inline and turned into a raw pointer, which
//! must be the referent's box, never the slot of the field, element or capture the reference was
//! read from. The real-storage cases beside them (`&mut o.inner.f`, `&mut o.arr[i]`, a field
//! through nested `#[repr(transparent)]` wrappers, `&v[i]` of an aggregate) keep their slots. Every
//! body also runs beside a live `Drop` guard, so its caller takes the cleanup path. Only relations
//! Rust defines are printed: distinct live objects have distinct addresses.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

struct R<'a> {
    n: u32,
    r: &'a P,
}

struct RO<'a> {
    n: u32,
    o: Option<&'a P>,
}

struct Deep<'a> {
    n: u32,
    inner: R<'a>,
}

struct Inner {
    f: u32,
    g: u32,
}

struct Outer<'a> {
    inner: Inner,
    arr: [u32; 4],
    next: &'a P,
}

#[repr(transparent)]
struct W(P);

#[repr(transparent)]
struct W2(W);

/// Prints its tag when dropped, so everything after it in a body has a cleanup edge.
struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 1 })
}

#[inline(never)]
fn address(p: *const P) -> usize {
    p as usize
}

/// Defines `$plain` with `$body`, and `$guarded` with the same body behind a live `Guard($tag)`.
macro_rules! plain_and_guarded {
    ($plain:ident, $guarded:ident, $tag:expr, $body:block) => {
        #[inline(never)]
        fn $plain() $body

        #[inline(never)]
        fn $guarded() {
            let _guard = Guard($tag);
            $body
        }
    };
}

// Call-free accessors: each is one fused expression.

#[inline(never)]
fn field_addr(a: &R) -> usize {
    a.r as *const P as usize
}

#[inline(never)]
fn field_ptr(a: &R) -> *const P {
    a.r
}

#[inline(never)]
fn field_slot_addr(a: &R) -> usize {
    &a.r as *const &P as usize
}

#[inline(never)]
fn elem_addr(refs: &[&P; 2], i: usize) -> usize {
    refs[i] as *const P as usize
}

#[inline(never)]
fn slice_elem(refs: &[&P], i: usize) -> *const P {
    refs[i]
}

#[inline(never)]
fn through(pp: &&P) -> *const P {
    *pp
}

#[inline(never)]
fn through2(ppp: &&&P) -> usize {
    **ppp as *const P as usize
}

#[inline(never)]
fn opt(a: &RO) -> *const P {
    match a.o {
        Some(r) => r,
        None => ptr::null(),
    }
}

#[inline(never)]
fn same(a: &R, h: &P) -> bool {
    a.r as *const P == h as *const P
}

#[inline(never)]
fn nested(d: &Deep) -> *const P {
    d.inner.r
}

#[inline(never)]
fn next_of(o: &Outer) -> *const P {
    &*o.next
}

#[inline(never)]
fn field_slot(o: &mut Outer) -> *mut u32 {
    &mut o.inner.f
}

#[inline(never)]
fn elem_slot(o: &mut Outer, i: usize) -> *mut u32 {
    &mut o.arr[i]
}

#[inline(never)]
fn wrapped(w: &W2) -> *const P {
    &w.0 .0
}

#[inline(never)]
fn wrapped_field(w: &mut W2) -> *mut u32 {
    &mut w.0 .0.y
}

#[inline(never)]
fn elem_of(v: &[P; 2], i: usize) -> *const P {
    &v[i]
}

#[inline(never)]
fn call_dyn(f: &dyn Fn() -> usize) -> usize {
    f()
}

#[inline(never)]
fn call_with<F: Fn(&R) -> *const P>(f: F, a: &R) -> *const P {
    f(a)
}

plain_and_guarded!(leaves, leaves_guarded, 404, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    print_bool(field_addr(&a) == address(&h));
    print_bool(field_ptr(&a) == &h as *const P);
    print_bool(ptr::eq(field_ptr(&a), &h));
    print_bool(field_slot_addr(&a) != field_addr(&a));
    print_bool(same(&a, &h));
    print_bool(!same(&a, &h2));
    let refs: [&P; 2] = black_box([&h, &h2]);
    print_bool(elem_addr(&refs, 1) == address(&h2));
    print_bool(elem_addr(&refs, 0) != elem_addr(&refs, 1));
    print_bool(ptr::eq(slice_elem(&refs, 0), &h));
    print_bool(ptr::eq(slice_elem(&refs[1..], 0), &h2));
    let pp: &&P = &a.r;
    print_bool(ptr::eq(through(pp), &h));
    print_bool(through2(&pp) == address(&h));
    let o = black_box(RO { n: 2, o: Some(&h2) });
    print_bool(ptr::eq(opt(&o), &h2));
    let d = black_box(Deep { n: 3, inner: R { n: 4, r: &h2 } });
    print_bool(ptr::eq(nested(&d), &h2));
    let mut out = black_box(Outer { inner: Inner { f: 1, g: 2 }, arr: [0, 1, 2, 3], next: &h });
    print_bool(ptr::eq(next_of(&out), &h));
    print_bool(next_of(&out) as usize == address(&h));
    let pf = field_slot(&mut out);
    unsafe { *pf = 42 };
    let pe = elem_slot(&mut out, 2);
    unsafe { *pe += 10 };
    print_u32(out.inner.f + out.inner.g + out.arr[2]);
    let mut w2 = black_box(W2(W(mk(5))));
    let w2_at = &w2 as *const W2 as usize;
    print_bool(wrapped(&w2) as usize == w2_at);
    print_bool(ptr::eq(wrapped(&w2), &w2.0 .0));
    let py = wrapped_field(&mut w2);
    unsafe { *py = 60 };
    print_u32(w2.0 .0.y);
    let v = black_box([mk(7), mk(8)]);
    print_bool(ptr::eq(elem_of(&v, 1), &v[1]));
    print_bool(elem_of(&v, 0) as usize == &v[0] as *const P as usize);
    print_u32(a.n + o.n + d.n + d.inner.n);
});

// Closures over a reference field: each closure body is a call-free leaf.
plain_and_guarded!(closure_capture, closure_capture_guarded, 208, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    let f = || a.r as *const P as usize;
    print_bool(f() == address(&h));
    let r = a.r;
    let g = move || r as *const P;
    print_bool(ptr::eq(g(), &h));
    print_bool(g() == a.r as *const P);
    let hh = || &a.r as *const &P as usize;
    print_bool(hh() != f());
    let k = |q: &R| q.r as *const P as usize;
    print_bool(k(&a) == address(&h));
    print_bool(k(&black_box(R { n: 2, r: &h2 })) == address(&h2));
    let m = |q: &R| ptr::eq(q.r, &h);
    print_bool(m(&a));
    print_bool(!m(&R { n: 3, r: &h2 }));
    print_bool(call_dyn(&f) == address(&h));
    print_bool(call_dyn(&|| a.r as *const P as usize) == f());
    print_bool(ptr::eq(call_with(|q| q.r as *const P, &a), &h));
    print_bool(call_with(|q| ptr::from_ref(q.r), &a) == g());
    let b = black_box(R { n: 5, r: &h2 });
    let pick = move || if b.n == 5 { b.r as *const P } else { ptr::null() };
    print_bool(ptr::eq(pick(), &h2));
    print_u32(a.n);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    leaves();
    leaves_guarded();
    closure_capture();
    closure_capture_guarded();
}
