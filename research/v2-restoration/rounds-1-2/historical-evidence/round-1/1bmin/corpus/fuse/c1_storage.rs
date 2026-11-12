//! Verifier fixture (c1_storage): clearing: `&(*r).inner.f`, `&(*r).arr[2]` are storage slots (written through and read back)
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

struct Nested<'a> {
    k: u32,
    link: &'a P,
}

struct Outer<'a> {
    inner: Inner,
    arr: [u32; 4],
    next: &'a P,
    parr: [P; 2],
    nested: Nested<'a>,
}

plain_and_guarded!(storage, storage_guarded, 301, {
    let h = mk(1);
    let h3 = mk(3);
    let mut o = black_box(Outer {
        inner: Inner { f: 1, g: 2 },
        arr: [0, 1, 2, 3],
        next: &h,
        parr: [mk(5), mk(6)],
        nested: Nested { k: 4, link: &h3 },
    });
    let r = &mut o;
    let pf: *mut u32 = &mut (*r).inner.f;
    unsafe { *pf = 42 };
    print_u32(r.inner.f);
    print_bool(ptr::eq(pf, &r.inner.f));
    let pg: *const u32 = &(*r).inner.g;
    print_bool(pf as *const u32 != pg);
    print_u32(unsafe { *pg });
    let pe: *mut u32 = &mut (*r).arr[2];
    unsafe { *pe += 10 };
    print_u32(r.arr[2]);
    print_bool(ptr::eq(pe, &r.arr[2]));
    print_bool(!ptr::eq(pe, &r.arr[1]));
    let i = black_box(3);
    let pi: *mut u32 = &mut (*r).arr[i];
    unsafe { *pi += 100 };
    print_u32(r.arr[3]);
    print_bool(ptr::eq(pi, &r.arr[3]));
    let pk: *mut u32 = &mut (*r).nested.k;
    unsafe { *pk += 1 };
    print_u32(r.nested.k);
    print_bool(ptr::eq(pk, &r.nested.k));
    let pp: *mut P = &mut (*r).parr[1];
    unsafe { (*pp).x = 77 };
    print_u32(r.parr[1].x);
    let px: *mut u32 = &mut (*r).parr[0].x;
    unsafe { *px = 55 };
    print_u32(r.parr[0].x);
    let shared = &o;
    let qf: *const u32 = &(*shared).inner.f;
    let qe: *const u32 = &(*shared).arr[2];
    print_u32(unsafe { *qf + *qe });
    print_bool(qf != qe);
    print_bool(ptr::eq(qf, &o.inner.f));
    print_bool(ptr::eq(qe, &o.arr[2]));
    print_u32(o.inner.f + o.arr[2] + o.arr[3] + o.parr[1].x + o.parr[0].x + o.nested.k);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    storage();
    storage_guarded();
}
