//! Verifier fixture (c3_enum): downcast then field of an enum payload: scalar payload slots, aggregate payloads, reference payloads
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

struct Inner {
    f: u32,
    g: u32,
}

enum E<'a> {
    A(u32),
    B(Inner),
    C(&'a P),
    D { p: P, q: &'a P },
    F(u32, u32),
}

plain_and_guarded!(enum_payload, enum_payload_guarded, 303, {
    let h = mk(1);
    let h2 = mk(2);
    let mut e = black_box(E::B(Inner { f: 3, g: 4 }));
    let re = &mut e;
    if let E::B(inner) = &mut *re {
        let pf: *mut u32 = &mut inner.f;
        unsafe { *pf = 30 };
        print_u32(inner.f);
        print_bool(ptr::eq(pf, &inner.f));
        print_bool(!ptr::eq(pf, &inner.g));
    }
    if let E::B(inner) = &e {
        print_u32(inner.f + inner.g);
    }
    let mut ef = black_box(E::F(1, 2));
    let ref_ef = &mut ef;
    if let E::F(a, b) = ref_ef {
        let pa: *mut u32 = a;
        let pb: *mut u32 = b;
        unsafe {
            *pa += 10;
            *pb += 20;
        }
        print_bool(pa != pb);
    }
    if let E::F(a, b) = &ef {
        print_u32(*a + *b);
    }
    let mut ea = black_box(E::A(9));
    if let E::A(n) = &mut ea {
        let pn: *mut u32 = n;
        unsafe { *pn = 90 };
    }
    match &ea {
        E::A(n) => print_u32(*n),
        _ => print_u32(0),
    }
    let e2 = black_box(E::C(&h));
    let re2 = &e2;
    if let E::C(r) = re2 {
        let pr: *const P = *r;
        print_bool(ptr::eq(pr, &h));
        print_bool(pr as usize == address(&h));
        print_bool(pr as usize != r as *const &P as usize);
        print_bool(ptr::eq(*r, &h));
    }
    let mut e3 = black_box(E::D { p: mk(7), q: &h2 });
    let re3 = &mut e3;
    if let E::D { p, q } = &mut *re3 {
        let pp: *mut P = p;
        unsafe { (*pp).y = 70 };
        print_u32(p.y);
        let pq: *const P = *q;
        print_bool(ptr::eq(pq, &h2));
        print_bool(pq as usize == address(&h2));
        print_bool(!ptr::eq(pq, pp));
        print_bool(pq as usize != q as *const &P as usize);
    }
    if let E::D { p, q } = &e3 {
        print_u32(p.y + q.y);
    }
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    enum_payload();
    enum_payload_guarded();
}
