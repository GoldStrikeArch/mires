//! A by-value copy that is written and never read again is a value of its own: the write lands in
//! the copy, never in what the copy was taken from.
//!
//! A copy whose one use is the write is dead after it, and the fusion planner used to spell such a
//! value at that use. The use is the write's target, so the write landed wherever the copy's
//! expression pointed: the clone `{ x: s.x, y: s.y }` of a struct, through `jsast::member`'s fold
//! of a literal's field to that field's initializer, became `s.x = 99` on the source element, and
//! an aggregate built from two parameters became `n = 99`. Each case copies out of a source of its
//! own (a static, a local array, a slice parameter, a constant slice, a struct field, an `Option`
//! payload), writes the copy through a field, a nested field, a reference field, a tuple struct
//! field, a `&mut` taken later, or inside a loop, and prints the source afterwards.
//!
//! The same rule covers a write through a shared view (`212_interior_mutable_copy_writes`) and the
//! remaining sources and shapes (`213_copy_sources_and_variants`, `214_copy_writes_through_borrows`).
//! Every case but `aggregate` is wrong on the base linker under `js-profile=release-balanced`, with
//! and without `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`. The expectations come from this file
//! run natively at `-O0` and `-O3`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

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
struct R {
    r: &'static u32,
    x: u32,
}

#[derive(Clone, Copy)]
struct T(u32, u32);

#[derive(Clone, Copy)]
struct H {
    pad: u32,
    t: P,
}

static PS: [P; 24] = [
    P { x: 0, y: 100 }, P { x: 1, y: 101 }, P { x: 2, y: 102 }, P { x: 3, y: 103 },
    P { x: 4, y: 104 }, P { x: 5, y: 105 }, P { x: 6, y: 106 }, P { x: 7, y: 107 },
    P { x: 8, y: 108 }, P { x: 9, y: 109 }, P { x: 10, y: 110 }, P { x: 11, y: 111 },
    P { x: 12, y: 112 }, P { x: 13, y: 113 }, P { x: 14, y: 114 }, P { x: 15, y: 115 },
    P { x: 16, y: 116 }, P { x: 17, y: 117 }, P { x: 18, y: 118 }, P { x: 19, y: 119 },
    P { x: 20, y: 120 }, P { x: 21, y: 121 }, P { x: 22, y: 122 }, P { x: 23, y: 123 },
];
const CS: &[P] = &[P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
static NS: [N; 2] = [
    N { a: 7, inner: P { x: 8, y: 9 } },
    N { a: 10, inner: P { x: 11, y: 12 } },
];
static TEN: u32 = 10;
static OTHER: u32 = 777;
static RS: [R; 2] = [R { r: &TEN, x: 1 }, R { r: &TEN, x: 2 }];
static TS: [T; 2] = [T(13, 14), T(15, 16)];

#[inline(never)]
fn total(ps: &[P]) -> u32 {
    ps.iter().map(|p| p.x).sum()
}

#[inline(never)]
fn through(s: &[P]) -> u32 {
    let mut p = s[black_box(4)];
    p.x = 99;
    s[4].x
}

#[inline(never)]
fn through_mut(s: &mut [P]) -> u32 {
    let mut p = s[black_box(2)];
    p.y = 77;
    s[2].y
}

#[inline(never)]
fn later_reference(s: &[P]) -> u32 {
    let mut p = s[black_box(1)];
    let r = &mut p.x;
    *r = 99;
    s[1].x
}

#[inline(never)]
fn in_a_loop(s: &[P]) -> u32 {
    for i in 0..s.len() {
        let mut p = s[i];
        p.x = 99;
    }
    total(s)
}

#[inline(never)]
fn built(n: u32, m: u32) -> u32 {
    let mut q = P { x: n, y: m };
    q.x = 99;
    n + m
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("static");
    print_u32(total(&PS));
    let mut p = PS[4];
    p.x = 99;
    print_u32(PS[4].x);
    print_u32(total(&PS));

    print_str("local array");
    let local = black_box([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
    let mut q = local[black_box(1)];
    q.x = 99;
    print_u32(local[1].x);

    print_str("slice parameter");
    let arr = black_box([P { x: 10, y: 20 }; 8]);
    print_u32(through(&arr));
    print_u32(arr[4].x);
    let mut arr2 = black_box([P { x: 30, y: 40 }; 4]);
    print_u32(through_mut(&mut arr2));
    print_u32(arr2[2].y);

    print_str("constant slice");
    let s: &[P] = CS;
    let mut c = s[black_box(1)];
    c.x = 99;
    print_u32(s[1].x);
    print_u32(total(s));

    print_str("nested field");
    let mut n = NS[black_box(1)];
    n.inner.x = 99;
    print_u32(NS[1].inner.x);
    let mut n = NS[black_box(0)];
    n.a = 99;
    print_u32(NS[0].a);

    print_str("reference field");
    let mut r = RS[black_box(1)];
    r.r = &OTHER;
    print_u32(*RS[1].r);

    print_str("tuple struct");
    let mut t = TS[black_box(0)];
    t.1 = 99;
    print_u32(TS[0].1);

    print_str("struct field");
    let h = black_box(H { pad: 5, t: P { x: 17, y: 18 } });
    let mut ht = h.t;
    ht.x = 99;
    print_u32(h.t.x + h.pad);

    print_str("option payload");
    let o = black_box(Some(P { x: 19, y: 20 }));
    if let Some(mut payload) = o {
        payload.x = 99;
    }
    if let Some(payload) = o {
        print_u32(payload.x);
    }

    print_str("reference taken later");
    let borrowed = black_box([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
    print_u32(later_reference(&borrowed));

    print_str("loop");
    let looped = black_box([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
    print_u32(in_a_loop(&looped));

    print_str("aggregate");
    print_u32(built(black_box(5), black_box(7)));
}
