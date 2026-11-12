//! (k) a local whose *field* is borrowed (never the local itself), assigned again whole.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct Outer {
    n: u32,
    h: H,
}
#[repr(C)]
struct Pair(H, u32);

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn e() {
    let mut o = black_box(Outer { n: 1, h: H { head: 1, tail: 2 } });
    let p = a(&o.h);
    o = black_box(Outer { n: 2, h: H { head: 3, tail: 4 } });
    print_bool(p == a(&o.h));
    print_u32(o.h.head + o.n);
    let mut arr = black_box([H { head: 1, tail: 1 }, H { head: 2, tail: 2 }]);
    let q = a(&arr[1]);
    arr = black_box([H { head: 3, tail: 3 }, H { head: 4, tail: 4 }]);
    print_bool(q == a(&arr[1]));
    print_u32(arr[1].head);
    let mut t = black_box(Pair(H { head: 1, tail: 1 }, 5u32));
    let tq = a(&t.0);
    let t1 = a(&t.1);
    t = black_box(Pair(H { head: 2, tail: 2 }, 6));
    print_bool(tq == a(&t.0));
    print_bool(t1 == a(&t.1));
    print_bool(a(&t.1) - a(&t.0) == mem::offset_of!(Pair, 1));
    print_bool(a(&t.0.tail) - a(&t.0) == mem::offset_of!(H, tail));
    print_bool(a(&t.0) < a(&t.1));
    print_u32(t.0.head + t.1);
    // a raw pointer to a field of a local that is reassigned, read afterwards
    let mut o2 = black_box(Outer { n: 1, h: H { head: 1, tail: 1 } });
    let rp = &raw const o2.h;
    o2 = black_box(Outer { n: 9, h: H { head: 8, tail: 8 } });
    print_u32(unsafe { (*rp).head } + o2.n);
    // reassigned in a loop with a pointer taken once
    let mut acc = black_box(Outer { n: 0, h: H { head: 0, tail: 0 } });
    let first = a(&acc.h);
    let mut same = 0;
    for i in 0..3u32 {
        acc = black_box(Outer { n: i, h: H { head: i, tail: i } });
        if a(&acc.h) == first {
            same += 1;
        }
    }
    print_u32(same);
    print_u32(acc.h.head);
    // a tuple local reassigned with the (H, u32) field pointer taken
    let mut tup = black_box((H { head: 1, tail: 1 }, 1u32));
    let th = a(&tup.0);
    tup = black_box((H { head: 2, tail: 2 }, 2u32));
    print_bool(th == a(&tup.0));
    print_u32(tup.0.head + tup.1);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e();
}
