//! Verifier fixture (r7_from_ref): `ptr::from_ref`, `&raw const *r`, `addr_of!`, returned references, nested and tuple fields, a static
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

struct Deep<'a> {
    n: u32,
    inner: R<'a>,
}

struct T<'a>(u32, &'a P);

struct RM<'a> {
    n: u32,
    r: &'a mut P,
}

static G: P = P { x: 40, y: 41 };

#[inline(never)]
fn pick<'a>(a: &R<'a>) -> &'a P {
    a.r
}

#[inline(never)]
fn make<'a>(h: &'a P) -> R<'a> {
    black_box(R { n: 7, r: h })
}

plain_and_guarded!(from_ref, from_ref_guarded, 209, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    print_bool(ptr::from_ref(a.r) == &raw const h);
    print_bool(ptr::eq(ptr::from_ref(a.r), &h));
    print_bool(ptr::from_ref(a.r) as usize == address(&h));
    print_bool(ptr::from_ref(&a.r) as usize != ptr::from_ref(a.r) as usize);
    print_bool(&raw const *a.r == &raw const h);
    print_bool(ptr::eq(&raw const *a.r, &h));
    print_bool(core::ptr::addr_of!(*a.r) as usize == address(&h));
    print_bool(ptr::eq(pick(&a), &h));
    print_bool(pick(&a) as *const P as usize == address(&h));
    print_bool(make(&h2).r as *const P as usize == address(&h2));
    print_bool(ptr::eq(make(&h2).r, &h2));
    let d = black_box(Deep { n: 2, inner: R { n: 3, r: &h2 } });
    print_bool(ptr::eq(d.inner.r, &h2));
    print_bool(d.inner.r as *const P as usize == address(&h2));
    print_bool(&d.inner.r as *const &P as usize != d.inner.r as *const P as usize);
    let rd = black_box(&d);
    print_bool(ptr::eq(rd.inner.r, &h2));
    print_bool((*rd).inner.r as *const P == &h2 as *const P);
    let t = black_box(T(4, &h));
    print_bool(ptr::eq(t.1, &h));
    print_bool(t.1 as *const P as usize == address(&h));
    let tup = black_box((5u32, &h2));
    print_bool(ptr::eq(tup.1, &h2));
    print_bool(tup.1 as *const P as usize == address(&h2));
    let s = black_box(R { n: 6, r: &G });
    print_bool(ptr::eq(s.r, &G));
    print_bool(s.r as *const P as usize == &G as *const P as usize);
    print_bool(s.r as *const P == &raw const G);
    print_u32(a.n + d.n + d.inner.n + t.0 + tup.0 + s.n + s.r.x);
});

plain_and_guarded!(mut_reborrow, mut_reborrow_guarded, 210, {
    let mut h = mk(1);
    let h_at = &h as *const P as usize;
    {
        let a = black_box(RM { n: 1, r: &mut h });
        let p: *mut P = &mut *a.r;
        print_bool(p as usize == h_at);
        print_bool(&raw mut *a.r == p);
        print_bool(ptr::eq(&*a.r, p));
        print_bool(&a.r as *const &mut P as usize != a.r as *const P as usize);
        unsafe { (*p).y = 20 };
        print_u32(a.r.y + a.n);
    }
    print_u32(h.y);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    from_ref();
    from_ref_guarded();
    mut_reborrow();
    mut_reborrow_guarded();
}
