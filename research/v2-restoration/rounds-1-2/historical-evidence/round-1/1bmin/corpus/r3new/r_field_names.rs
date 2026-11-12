//! (r) structs whose FIELD NAMES are the keys the runtime's pointer records use
//! (`pr`, `fo`, `an`, `ob`, `sc`, `es`, `buf`, `off`): pointer equality and addresses of
//! references to them.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::ptr;
use prelude::*;

struct Pr {
    pr: u32,
    k: u32,
}
struct Fo {
    k: u32,
    fo: u32,
}
struct An {
    an: u32,
    k: u32,
}
struct Ob {
    ob: u32,
    sc: u32,
    es: u32,
}
struct BufOff {
    buf: u32,
    off: u32,
}
fn a<T>(r: &T) -> usize {
    r as *const T as usize
}
#[inline(never)]
fn same<T>(x: &T, y: &T) -> bool {
    ptr::eq(x, y)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("pr");
    let (p1, p2) = (black_box(Pr { pr: 5, k: 1 }), black_box(Pr { pr: 5, k: 2 }));
    print_bool(!same(&p1, &p2));
    print_bool(same(&p1, &p1));
    print_bool(!ptr::eq(&raw const p1, &raw const p2));
    print_bool(a(&p1) != a(&p2));
    print_u32(p1.pr + p1.k + p2.k);
    print_str("fo");
    let (f1, f2) = (black_box(Fo { k: 1, fo: 4 }), black_box(Fo { k: 1, fo: 4 }));
    print_bool(!same(&f1, &f2));
    print_bool(a(&f1) != a(&f2));
    print_bool(a(&f1.fo) != a(&f1.k));
    print_str("an");
    let (a1, a2) = (black_box(An { an: 7, k: 1 }), black_box(An { an: 7, k: 1 }));
    print_bool(!same(&a1, &a2));
    print_bool(a(&a1) != a(&a2));
    print_bool(a(&a1.k) != a(&a1));
    print_str("ob");
    let (o1, o2) = (black_box(Ob { ob: 1, sc: 2, es: 3 }), black_box(Ob { ob: 1, sc: 2, es: 3 }));
    print_bool(!same(&o1, &o2));
    print_bool(a(&o1) != a(&o2));
    print_bool(a(&o1.es) != a(&o1));
    print_str("buf");
    let (b1, b2) = (black_box(BufOff { buf: 1, off: 2 }), black_box(BufOff { buf: 1, off: 2 }));
    print_bool(!same(&b1, &b2));
    print_bool(same(&b1, &b1));
    print_bool(a(&b1) != a(&b2));
    print_u32(b1.buf + b2.off);
}
