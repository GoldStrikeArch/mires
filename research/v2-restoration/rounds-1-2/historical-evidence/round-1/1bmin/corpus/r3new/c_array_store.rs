//! (c) whole-array stores of aggregates with outstanding raw pointers taken before the store;
//! nested arrays; scalar arrays inside structs.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::mem;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct S {
    a: u32,
    b: u32,
}
#[inline(never)]
fn mk(a: u32) -> S {
    black_box(S { a, b: a * 2 })
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}
struct W {
    k: u32,
    arr: [u32; 3],
    ss: [S; 2],
}

#[inline(never)]
fn t_flat() {
    print_str("flat");
    let mut a = black_box([mk(1), mk(2)]);
    let p0 = &raw const a[0];
    let p1 = &raw const a[1];
    let base = a.as_ptr();
    let r1 = ad(&a[1]);
    a = [mk(3), mk(4)];
    print_u32(unsafe { (*p0).a * 10 + (*p1).a });
    print_bool(ptr::eq(p0, &raw const a[0]) && ptr::eq(p1, &raw const a[1]));
    print_bool(ptr::eq(base, a.as_ptr()));
    print_bool(ptr::eq(p1, base.wrapping_add(1)));
    print_bool(r1 == ad(&a[1]));
    print_bool(p1 as usize - p0 as usize == mem::size_of::<S>());
    a = [S { a: 5, b: 6 }, S { a: 7, b: 8 }];
    print_u32(unsafe { (*p0).a * 1000 + (*p0).b * 100 + (*p1).a * 10 + (*p1).b });
    print_bool(r1 == ad(&a[1]));
    a = black_box([mk(9); 2]);
    a[0].a += 1;
    print_u32(unsafe { (*p0).a * 100 + (*p1).a });
    // rotation through a whole store: the right side reads the array being written
    a = [a[1], a[0]];
    print_u32(a[0].a * 100 + a[1].a);
    print_u32(unsafe { (*p0).a * 100 + (*p1).a });
    let mut t = black_box([mk(1), mk(2), mk(3)]);
    let q2 = &raw const t[2];
    t = [t[2], t[0], t[1]];
    print_u32(t[0].a * 100 + t[1].a * 10 + t[2].a);
    t = [t[1], t[2], t[0]];
    print_u32(t[0].a * 100 + t[1].a * 10 + t[2].a);
    print_u32(unsafe { (*q2).a });
    t[0] = t[2];
    t[2].a = 9;
    print_u32(t[0].a * 100 + t[1].a * 10 + t[2].a);
}

#[inline(never)]
fn t_long() {
    print_str("long");
    // ten leaves: past the field-wise limit
    let mut a = black_box([mk(1), mk(2), mk(3), mk(4), mk(5)]);
    let p3 = &raw const a[3];
    let r3 = ad(&a[3]);
    a = black_box([mk(6), mk(7), mk(8), mk(9), mk(10)]);
    print_u32(unsafe { (*p3).a });
    print_bool(r3 == ad(&a[3]));
    print_bool(ptr::eq(p3, a.as_ptr().wrapping_add(3)));
    a = [a[4], a[3], a[2], a[1], a[0]];
    print_u32(a[0].a * 10000 + a[1].a * 1000 + a[2].a * 100 + a[3].a * 10 + a[4].a);
    print_u32(unsafe { (*p3).a });
    let b = a;
    a[0].a = 77;
    print_u32(b[0].a + a[0].a);
}

#[inline(never)]
fn t_nested() {
    print_str("nested");
    let mut g = black_box([[mk(1), mk(2)], [mk(3), mk(4)]]);
    let p10 = &raw const g[1][0];
    let p01 = &raw const g[0][1];
    let row1 = &raw const g[1];
    let r11 = ad(&g[1][1]);
    g = [[mk(5), mk(6)], [mk(7), mk(8)]];
    print_u32(unsafe { (*p10).a * 10 + (*p01).a });
    print_bool(r11 == ad(&g[1][1]));
    print_bool(ptr::eq(p10, &raw const g[1][0]));
    print_u32(unsafe { (*row1)[1].a });
    g[1] = [mk(9), mk(10)];
    print_u32(unsafe { (*p10).a * 100 + (*row1)[1].a });
    print_bool(r11 == ad(&g[1][1]));
    g = [g[1], g[0]];
    print_u32(g[0][0].a * 1000 + g[0][1].a * 100 + g[1][0].a * 10 + g[1][1].a);
    print_u32(unsafe { (*p10).a * 10 + (*p01).a });
    g[0] = g[1];
    g[1][0].a = 0;
    print_u32(g[0][0].a * 1000 + g[0][1].a * 100 + g[1][0].a * 10 + g[1][1].a);
}

#[inline(never)]
fn t_in_struct() {
    print_str("in_struct");
    let mut w = black_box(W { k: 1, arr: [1, 2, 3], ss: [mk(1), mk(2)] });
    let pe = &raw const w.arr[1];
    let ps = &raw const w.ss[1];
    let rs = ad(&w.ss[1]);
    w.arr = [4, 5, 6];
    w.ss = [mk(7), mk(8)];
    print_u32(unsafe { *pe * 100 + (*ps).a });
    print_bool(rs == ad(&w.ss[1]));
    w = black_box(W { k: 2, arr: [7, 8, 9], ss: [mk(3), mk(4)] });
    print_u32(unsafe { *pe * 100 + (*ps).a } + w.k);
    print_bool(rs == ad(&w.ss[1]));
    let wm = &mut w;
    *wm = W { k: 3, arr: [wm.arr[2], wm.arr[1], wm.arr[0]], ss: [wm.ss[1], wm.ss[0]] };
    print_u32(w.arr[0] * 100 + w.arr[1] * 10 + w.arr[2]);
    print_u32(w.ss[0].a * 10 + w.ss[1].a);
    print_u32(unsafe { *pe * 100 + (*ps).a } + w.k);
    let other = black_box(W { k: 4, arr: [1, 1, 1], ss: [mk(5), mk(6)] });
    w.arr = other.arr;
    w.ss = other.ss;
    w.arr[0] = 2;
    w.ss[0].a = 0;
    print_u32(other.arr[0] * 10 + other.ss[0].a + w.arr[0] * 100);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_flat();
    t_long();
    t_nested();
    t_in_struct();
}
