//! A written by-value copy stays apart from its source, whatever the source and whatever the
//! write.
//!
//! `210_dead_copy_writes` pins the shape (a copy whose one use is the write) over statics, local
//! arrays and slice parameters. This file walks the rest of the sources: a `const`, a `const` slice,
//! a promoted constant behind a reference (whose corruption outlives the call), a local array, a
//! `Box`, a `Vec`, a field of a boxed struct. It also walks the rest of the writes: a nested field,
//! a whole nested struct, an array inside a tuple struct, an array of structs, a new variant written
//! over a copied enum (directly, through a `&mut`, or into a payload), and the `Option` methods that
//! write in place (`take`, `replace`, `insert`, `get_or_insert`, `as_mut`). Each case prints the
//! source afterwards and, where it is live, the copy; `sources` reprints every static at the end,
//! so a write that leaked into one shows up again.
//!
//! The nested, constant-slice, promoted, local-array and box cases are wrong on the base linker
//! under `js-profile=release-balanced`, and all of them but `promoted` and `box by reference` with
//! `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on` too; the other cases are the neighbours that were
//! already right and must stay so. The expectations come from this file run natively at `-O0` and
//! `-O3`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct N {
    a: u32,
    inner: P,
}

#[derive(Clone, Copy)]
enum E {
    A(u32),
    B(u32, u32),
    C(P),
}

#[derive(Clone, Copy)]
struct T3(P, [u32; 2], u32);

fn obs(e: &E) -> u32 {
    match *e {
        E::A(v) => 1000 + v,
        E::B(a, b) => 2000 + a * 10 + b,
        E::C(p) => 3000 + p.x * 10 + p.y,
    }
}

fn sum(s: &[P]) -> u32 {
    let mut t = 0;
    for p in s {
        t = t * 7 + p.x * 3 + p.y;
    }
    t
}

static PS: [P; 3] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
const KP: P = P { x: 11, y: 12 };
const KS: &[P] = &[P { x: 21, y: 22 }, P { x: 23, y: 24 }];
static ES: [E; 3] = [E::A(1), E::B(2, 3), E::C(P { x: 4, y: 5 })];
static NS: [N; 2] = [
    N {
        a: 7,
        inner: P { x: 8, y: 9 },
    },
    N {
        a: 10,
        inner: P { x: 11, y: 12 },
    },
];
static TS: [T3; 2] = [
    T3(P { x: 1, y: 2 }, [3, 4], 5),
    T3(P { x: 6, y: 7 }, [8, 9], 10),
];
static OP: [Option<P>; 2] = [None, Some(P { x: 31, y: 32 })];

#[inline(never)]
fn constant() -> u32 {
    let mut c = KP;
    c.x = 99;
    KP.x * 1000 + c.x
}

#[inline(never)]
fn constant_slice() -> u32 {
    let s = KS;
    let mut c = s[black_box(1)];
    c.y = 99;
    s[1].y + KS[1].y
}

#[inline(never)]
fn promoted() -> u32 {
    let r: &P = &P { x: 41, y: 42 };
    let mut c = *r;
    c.x = 99;
    r.x
}

#[inline(never)]
fn promoted_array() -> u32 {
    let r: &[P; 2] = &[P { x: 43, y: 44 }, P { x: 45, y: 46 }];
    let mut c = r[black_box(1)];
    c.x = 99;
    r[1].x
}

#[inline(never)]
fn local_array() -> u32 {
    let a = [P { x: 53, y: 54 }, P { x: 55, y: 56 }];
    let mut c = a[black_box(1)];
    c.y = 99;
    a[1].y
}

#[inline(never)]
fn boxed() -> u32 {
    let b = black_box(Box::new(P { x: 61, y: 62 }));
    let mut c = *b;
    c.x = 99;
    b.x
}

#[inline(never)]
fn boxed_by_reference(b: &Box<P>) -> u32 {
    let mut c = **b;
    c.y = 99;
    b.y
}

#[inline(never)]
fn boxed_field() -> u32 {
    let b = black_box(Box::new(N {
        a: 63,
        inner: P { x: 64, y: 65 },
    }));
    let mut c = b.inner;
    c.x = 99;
    b.inner.x
}

#[inline(never)]
fn vector() -> u32 {
    let v: Vec<P> = black_box(alloc::vec![P { x: 71, y: 72 }, P { x: 73, y: 74 }]);
    let mut c = v[black_box(1)];
    c.x = 99;
    v[1].x
}

#[inline(never)]
fn nested_field(s: &[N]) -> u32 {
    let mut c = s[black_box(1)];
    c.inner.x = 99;
    s[1].inner.x
}

#[inline(never)]
fn nested_whole(s: &[N]) -> u32 {
    let mut c = s[black_box(1)];
    c.inner = P { x: 99, y: 98 };
    s[1].inner.x * 100 + s[1].inner.y
}

#[inline(never)]
fn nested_copied_out(s: &[N]) -> u32 {
    let c = s[black_box(1)];
    let mut i = c.inner;
    i.x = 99;
    s[1].inner.x * 100 + c.inner.x
}

#[inline(never)]
fn array_in_tuple_struct(s: &[T3]) -> u32 {
    let mut c = s[black_box(1)];
    c.1[1] = 99;
    c.0.y = 98;
    s[1].1[1] * 100 + s[1].0.y
}

#[inline(never)]
fn array_of_structs(a: &[P; 3]) -> u32 {
    let mut c = *a;
    c[1].x = 99;
    c[black_box(2)].y = 98;
    a[1].x * 100 + a[2].y
}

#[inline(never)]
fn new_variant(s: &[E]) -> u32 {
    let mut e = s[black_box(1)];
    e = E::A(77);
    obs(&s[1]) * 10000 + obs(&e)
}

#[inline(never)]
fn new_variant_dead(s: &[E]) -> u32 {
    let mut e = s[black_box(1)];
    e = E::A(77);
    let _ = e;
    obs(&s[1])
}

#[inline(never)]
fn new_variant_through_reference(s: &[E]) -> u32 {
    let mut e = s[black_box(1)];
    {
        let r = &mut e;
        *r = E::C(P { x: 8, y: 9 });
    }
    obs(&s[1]) * 10000 + obs(&e)
}

#[inline(never)]
fn payload(s: &[E]) -> u32 {
    let mut e = s[black_box(2)];
    if let E::C(ref mut p) = e {
        p.x = 9;
    }
    obs(&s[2]) * 10000 + obs(&e)
}

#[inline(never)]
fn payload_in_tuple(s: &[E]) -> u32 {
    let mut c = (s[black_box(2)], 5u32);
    if let E::C(ref mut p) = c.0 {
        p.y = 99;
    }
    obs(&s[2])
}

#[inline(never)]
fn option_take(s: &[Option<P>]) -> u32 {
    let mut o = s[black_box(1)];
    let t = o.take();
    s[1].map_or(0, |p| p.x) * 1000 + t.map_or(0, |p| p.y) + o.is_none() as u32
}

#[inline(never)]
fn option_replace(s: &[Option<P>]) -> u32 {
    let mut o = s[black_box(1)];
    o.replace(P { x: 99, y: 98 });
    s[1].map_or(0, |p| p.x)
}

#[inline(never)]
fn option_insert(s: &[Option<P>]) -> u32 {
    let mut o = s[black_box(0)];
    o.insert(P { x: 99, y: 98 }).x += 1;
    s[0].map_or(5, |p| p.x) * 1000 + o.map_or(0, |p| p.x)
}

#[inline(never)]
fn option_get_or_insert(s: &[Option<P>]) -> u32 {
    let mut o = s[black_box(0)];
    o.get_or_insert(P { x: 99, y: 98 }).y = 7;
    s[0].map_or(5, |p| p.y) * 1000 + o.map_or(0, |p| p.y)
}

#[inline(never)]
fn option_as_mut(s: &[Option<P>]) -> u32 {
    let mut o = s[black_box(1)];
    if let Some(p) = o.as_mut() {
        p.x = 99;
    }
    s[1].map_or(0, |p| p.x)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ps = black_box(PS);
    let es = black_box(ES);
    let ns = black_box(NS);
    let ts = black_box(TS);
    let op = black_box(OP);

    print_str("constant");
    print_u32(constant());
    print_str("constant slice");
    print_u32(constant_slice());
    print_str("promoted");
    print_u32(promoted());
    print_u32(promoted());
    print_str("promoted array");
    print_u32(promoted_array());
    print_u32(promoted_array());
    print_str("local array");
    print_u32(local_array());
    print_str("box");
    print_u32(boxed());
    print_str("box by reference");
    let b = black_box(Box::new(P { x: 66, y: 67 }));
    print_u32(boxed_by_reference(&b));
    print_u32(b.y);
    print_str("box field");
    print_u32(boxed_field());
    print_str("vec");
    print_u32(vector());

    print_str("nested field");
    print_u32(nested_field(&ns));
    print_u32(nested_field(&NS));
    print_str("nested whole");
    print_u32(nested_whole(&ns));
    print_u32(nested_whole(&NS));
    print_str("nested copied out");
    print_u32(nested_copied_out(&ns));
    print_str("array in tuple struct");
    print_u32(array_in_tuple_struct(&ts));
    print_u32(array_in_tuple_struct(&TS));
    print_str("array of structs");
    print_u32(array_of_structs(&ps));
    print_u32(array_of_structs(&PS));

    print_str("new variant");
    print_u32(new_variant(&es));
    print_str("new variant, dead");
    print_u32(new_variant_dead(&es));
    print_u32(new_variant_dead(&ES));
    print_str("new variant through a reference");
    print_u32(new_variant_through_reference(&es));
    print_str("payload");
    print_u32(payload(&es));
    print_u32(payload(&ES));
    print_str("payload in a tuple");
    print_u32(payload_in_tuple(&es));
    print_u32(payload_in_tuple(&ES));

    print_str("option take");
    print_u32(option_take(&op));
    print_u32(option_take(&OP));
    print_str("option replace");
    print_u32(option_replace(&op));
    print_u32(option_replace(&OP));
    print_str("option insert");
    print_u32(option_insert(&op));
    print_str("option get_or_insert");
    print_u32(option_get_or_insert(&op));
    print_str("option as_mut");
    print_u32(option_as_mut(&op));
    print_u32(option_as_mut(&OP));

    print_str("sources");
    print_u32(sum(&ps));
    print_u32(sum(&PS));
    print_u32(obs(&ES[1]) + obs(&ES[2]));
    print_u32(NS[1].inner.x * 100 + TS[1].1[1]);
    print_u32(OP[1].map_or(0, |p| p.x));
}
