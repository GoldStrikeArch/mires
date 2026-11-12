//! (l) stores through `&mut` parameters, trait methods, defaults; nested tuples and arrays.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[derive(Clone, Copy, Default, PartialEq)]
struct H {
    head: u32,
    tail: u32,
}
#[derive(Default)]
struct Outer {
    n: u32,
    h: H,
    t: (H, u32),
    arr: [H; 2],
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

trait Reset {
    fn reset(&mut self);
}
impl Reset for Outer {
    #[inline(never)]
    fn reset(&mut self) {
        *self = Outer { n: 100, h: H { head: 1, tail: 1 }, t: (H { head: 2, tail: 2 }, 3), arr: [H::default(); 2] };
    }
}

#[inline(never)]
fn set_tuple(o: &mut Outer, v: (H, u32)) {
    o.t = v;
}

#[inline(never)]
fn set_generic<T>(slot: &mut T, v: T) {
    *slot = v;
}

#[inline(never)]
fn bump(mut o: Outer) -> Outer {
    o.h = H { head: o.h.head + 1, tail: o.h.tail };
    o
}

#[inline(never)]
fn e() {
    let mut o = black_box(Outer { n: 1, h: H { head: 1, tail: 2 }, t: (H { head: 3, tail: 4 }, 5), arr: [H { head: 6, tail: 6 }; 2] });
    let (ao, ah, at, at0, at1, aa, aa1) = (a(&o), a(&o.h), a(&o.t), a(&o.t.0), a(&o.t.1), a(&o.arr), a(&o.arr[1]));
    o.reset();
    print_bool(a(&o) == ao);
    print_bool(a(&o.h) == ah);
    print_bool(a(&o.t) == at);
    print_bool(a(&o.t.0) == at0);
    print_bool(a(&o.t.1) == at1);
    print_bool(a(&o.arr) == aa);
    print_bool(a(&o.arr[1]) == aa1);
    print_u32(o.n + o.h.head + o.t.0.head + o.t.1);
    set_tuple(&mut o, (H { head: 7, tail: 7 }, 8));
    print_bool(a(&o.t.0) == at0);
    print_u32(o.t.0.head + o.t.1);
    set_generic(&mut o.h, H { head: 9, tail: 9 });
    set_generic(&mut o.arr[0], H { head: 10, tail: 10 });
    set_generic(&mut o.t, (H { head: 11, tail: 11 }, 12));
    print_bool(a(&o.h) == ah);
    print_bool(a(&o.t.0) == at0);
    print_u32(o.h.head + o.arr[0].head + o.t.0.head + o.t.1);
    o = Outer::default();
    print_bool(a(&o) == ao);
    print_bool(o.h == H::default());
    o = bump(bump(Outer { n: 1, h: H { head: 1, tail: 1 }, t: (H::default(), 0), arr: [H::default(); 2] }));
    print_bool(a(&o.h) == ah);
    print_u32(o.h.head);
    o.arr = [H { head: 1, tail: 1 }, H { head: 2, tail: 2 }];
    print_bool(a(&o.arr[1]) == aa1);
    print_bool(aa == a(&o.arr[0]));
    print_u32(o.arr[0].head + o.arr[1].head);
    // a tuple field's inner struct and the tuple's second field keep their offsets
    print_bool(a(&o.t.1) - a(&o.t) == mem::offset_of!((H, u32), 1));
    print_bool(a(&o.t.0.tail) - a(&o.t) == mem::offset_of!(H, tail));
    print_bool(a(&o.arr[1]) - a(&o.arr[0]) == mem::size_of::<H>());
    // mem::replace on a generic slot and swap through generics
    let old = mem::replace(&mut o.t.0, H { head: 20, tail: 20 });
    print_u32(old.head + o.t.0.head);
    let mut other = black_box(Outer::default());
    set_generic(&mut other, Outer { n: 50, ..Outer::default() });
    mem::swap(&mut o, &mut other);
    print_bool(a(&o) == ao);
    print_u32(o.n + other.t.0.head);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e();
}
