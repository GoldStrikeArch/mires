//! (g) closures capturing `&o.h` / `&mut o.h` / whole `o`: identity after reassignment.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use core::hint::black_box;
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

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

fn h(x: u32) -> H {
    H { head: x, tail: x + 1 }
}

#[inline(never)]
fn e_closures() {
    let mut o = black_box(Outer { n: 1, h: h(1) });
    let at = a(&o.h);
    let ato = a(&o);
    {
        let mut set = |v: H| {
            o.h = v;
        };
        set(h(3));
        set(h(5));
    }
    print_bool(a(&o.h) == at);
    print_u32(o.h.head);
    {
        let r = &mut o.h;
        let mut through = move || {
            *r = h(7);
            r.head += 1;
            r as *const H as usize
        };
        let p = through();
        print_bool(p == at);
    }
    print_u32(o.h.head);
    {
        let look = {
            let hr = &o.h;
            move || a(hr)
        };
        print_bool(look() == at);
    }
    {
        let mut whole = || {
            o = Outer { n: 2, h: h(9) };
        };
        whole();
        whole();
    }
    print_bool(a(&o.h) == at);
    print_bool(a(&o) == ato);
    print_u32(o.h.head + o.n);
    // move closure owning a struct, reassigning its field across calls
    let mut owned = black_box(Outer { n: 3, h: h(1) });
    let mut f = move || {
        owned.h = H { head: owned.h.head + 1, tail: 0 };
        owned.h.head + owned.n
    };
    print_u32(f());
    print_u32(f());
    let mut bx: Box<dyn FnMut() -> u32> = Box::new(f);
    print_u32(bx());
    // a move closure that reassigns the whole captured struct
    let mut owned2 = black_box(Outer { n: 4, h: h(1) });
    let mut g = move || {
        let before = a(&owned2.h);
        owned2 = Outer { n: owned2.n + 1, h: h(owned2.h.head + 10) };
        (before == a(&owned2.h), owned2.h.head + owned2.n)
    };
    let (same1, v1) = g();
    let (same2, v2) = g();
    print_bool(same1);
    print_bool(same2);
    print_u32(v1 * 100 + v2);
    // FnOnce returning a captured struct
    let own3 = black_box(h(4));
    let once = move || own3;
    let got = once();
    print_u32(got.head);
    // a closure that takes the struct by &mut parameter and stores whole
    let store = |target: &mut Outer, v: Outer| *target = v;
    let mut o2 = black_box(Outer { n: 1, h: h(1) });
    let at2 = a(&o2.h);
    store(&mut o2, Outer { n: 6, h: h(6) });
    print_bool(a(&o2.h) == at2);
    print_u32(o2.h.head + o2.n);
    let addr_of = |o: &Outer| a(&o.h);
    o2.h = h(2);
    print_bool(addr_of(&o2) == at2);
    print_u32(o2.h.tail);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_closures();
}
