//! Regression suite for safe-Rust soundness fixes:
//! 1. Slot metadata key hygiene (`FullCollision` struct fields)
//! 2. `as_flattened()` / `as_flattened_mut()` on nested arrays, subslices, and `Vec<[T; N]>`
//! 3. >8-variant `Copy` enum deep cloning (including `__proto__` field variant)
//! 4. `__proto__` fields in statics, locals, overwrites, and mutable borrows
//! 5. Static projection cycles through struct fields and array elements
//! 6. Aggregate place address identity across overwrites and non-`Copy` move-out isolation

#![no_std]
#![no_main]
#![allow(dead_code, non_snake_case)]

extern crate alloc;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct FullCollision {
    buf: u32,
    off: u32,
    len: u32,
    pr: u32,
    ps: u32,
    sc: u32,
    es: u32,
    w: u32,
    ptr: u32,
    meta: u32,
}

#[derive(Clone, Copy)]
struct ProtoPair {
    __proto__: u32,
    b: u32,
}

#[derive(Clone, Copy)]
enum BigE {
    V0,
    V1(u32),
    V2(u32),
    V3(u32),
    V4(u32),
    V5(u32),
    V6(u32),
    V7(u32),
    V8(ProtoPair),
    V9([u32; 2]),
}

static SP: ProtoPair = ProtoPair { __proto__: 5, b: 6 };

struct Item {
    v: u32,
}
struct CycA {
    items: [Item; 2],
    b: &'static CycB,
}
struct CycB {
    item: &'static Item,
    a: &'static CycA,
}
static CA: CycA = CycA {
    items: [Item { v: 10 }, Item { v: 20 }],
    b: &CB,
};
static CB: CycB = CycB {
    item: &CA.items[1],
    a: &CA,
};

struct NonCopy {
    head: u32,
}
struct Wrap {
    tag: u32,
    nc: NonCopy,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. FullCollision struct
    let fc = black_box(FullCollision {
        buf: 1,
        off: 2,
        len: 3,
        pr: 4,
        ps: 5,
        sc: 6,
        es: 7,
        w: 8,
        ptr: 9,
        meta: 10,
    });
    let p = &fc as *const FullCollision;
    print_bool(p as usize != 0);
    print_bool(ptr::eq(p, &fc));
    print_bool(p == &fc as *const FullCollision);
    print_u32(
        fc.buf + fc.off + fc.len + fc.pr + fc.ps + fc.sc + fc.es + fc.w + fc.ptr + fc.meta,
    );

    // 2. as_flattened / as_flattened_mut on nested arrays, subslices, and Vec
    let mut mat = black_box([[1u32, 2], [3, 4], [5, 6]]);
    mat.as_flattened_mut()[3] = 40;
    print_u32(mat[1][1]);
    let sub = mat[1..].as_flattened();
    print_u32(sub[0] + sub[1] + sub[2] + sub[3]);
    let nested = black_box([[[1u32, 2], [3, 4]], [[5, 6], [7, 8]]]);
    let f3 = nested.as_flattened().as_flattened();
    print_u32(f3[0] + f3[3] + f3[5] + f3[7]);
    let mut v: Vec<[u32; 2]> = Vec::new();
    v.push([10, 20]);
    v.push([30, 40]);
    v.as_flattened_mut()[2] = 300;
    print_u32(v[1][0] + v.as_flattened()[3]);

    // 3. >8-variant Copy enum deep cloning
    let e1 = black_box(BigE::V8(ProtoPair { __proto__: 11, b: 22 }));
    let mut e2 = e1;
    if let BigE::V8(pp) = &mut e2 {
        pp.__proto__ = 99;
        pp.b = 88;
    }
    print_u32(match e1 {
        BigE::V8(pp) => pp.__proto__ * 100 + pp.b,
        _ => 0,
    });
    print_u32(match e2 {
        BigE::V8(pp) => pp.__proto__ * 100 + pp.b,
        _ => 0,
    });
    let e3 = black_box(BigE::V9([7, 8]));
    let mut e4 = e3;
    if let BigE::V9(a) = &mut e4 {
        a[0] = 77;
    }
    print_u32(match e3 {
        BigE::V9(a) => a[0] * 10 + a[1],
        _ => 0,
    });

    // 4. __proto__ in static, local, overwrite, and &mut field
    print_u32(black_box(&SP).__proto__ * 10 + SP.b);
    let mut lp = black_box(ProtoPair { __proto__: 1, b: 2 });
    let r = black_box(&mut lp);
    *r = black_box(ProtoPair { __proto__: 3, b: 4 });
    let fp = &mut lp.__proto__;
    *fp += 10;
    print_u32(lp.__proto__ * 10 + lp.b);

    // 5. Static projection cycle into array element
    print_u32(CB.item.v + CA.b.item.v);
    print_bool(ptr::eq(CB.item, &CA.items[1]));

    // 6. Aggregate place address identity + non-Copy move-out isolation
    let mut w = black_box(Wrap {
        tag: 1,
        nc: NonCopy { head: 42 },
    });
    let a0 = &w.nc as *const NonCopy as usize;
    let moved = w.nc;
    w.nc = black_box(NonCopy { head: 99 });
    let a1 = &w.nc as *const NonCopy as usize;
    let h1 = &w.nc.head as *const u32 as usize;
    print_bool(a0 == a1);
    print_bool(h1 == a1);
    print_u32(moved.head * 100 + w.nc.head);

    let mut bh = black_box(NonCopy { head: 7 });
    let ba0 = &bh as *const NonCopy as usize;
    let bm = bh;
    bh = black_box(NonCopy { head: 8 });
    print_bool(ba0 == &bh as *const NonCopy as usize);
    print_bool(&bh.head as *const u32 as usize == ba0);
    print_u32(bm.head * 10 + bh.head);
}
