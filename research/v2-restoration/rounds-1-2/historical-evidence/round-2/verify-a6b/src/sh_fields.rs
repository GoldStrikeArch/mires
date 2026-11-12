#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy, PartialEq)]
struct Inner { x: u32, y: i64 }
#[derive(Clone, Copy)]
struct Big {
    a: u64, b: i64, c: f64, d: bool, e: char, f: u8, g: i16,
    bytes: [u8; 5], words: [u16; 3], longs: [u64; 2],
    s: &'static str, inner: Inner, arr: [Inner; 2], grid: [[u8; 2]; 2],
    refs: [&'static Big; 2], sl: &'static [&'static Big], bs: &'static [u8],
    unit: (), tup: (u8, &'static Big),
    next: &'static Big,
}
static B1: Big = Big {
    a: 0xFFFF_FFFF_FFFF_FFF0, b: -5, c: 1.5, d: true, e: 'x', f: 200, g: -300,
    bytes: [1, 2, 3, 4, 5], words: [1000, 2000, 3000], longs: [1 << 40, 7],
    s: "one", inner: Inner { x: 11, y: -12 }, arr: [Inner { x: 1, y: 2 }, Inner { x: 3, y: 4 }], grid: [[1, 2], [3, 4]],
    refs: [&B2, &B1], sl: &[&B2, &B1, &B2], bs: b"hello",
    unit: (), tup: (9, &B2),
    next: &B2,
};
static B2: Big = Big {
    a: 2, b: i64::MIN, c: -0.25, d: false, e: 'y', f: 0, g: 77,
    bytes: [9, 8, 7, 6, 5], words: [1, 2, 3], longs: [u64::MAX, 0],
    s: "a-long-string-that-is-used-twice-so-it-may-be-hoisted-out-of-line", inner: Inner { x: 21, y: 22 }, arr: [Inner { x: 5, y: 6 }, Inner { x: 7, y: 8 }], grid: [[5, 6], [7, 8]],
    refs: [&B1, &B1], sl: &[], bs: b"",
    unit: (), tup: (3, &B1),
    next: &B1,
};
static OUT: &'static str = "a-long-string-that-is-used-twice-so-it-may-be-hoisted-out-of-line";
fn sum(b: &Big) -> u64 {
    let mut t = b.a.wrapping_add(b.b as u64).wrapping_add(b.c as i64 as u64).wrapping_add(b.d as u64).wrapping_add(b.e as u64).wrapping_add(b.f as u64).wrapping_add(b.g as i64 as u64);
    for x in b.bytes.iter() { t = t.wrapping_mul(3).wrapping_add(*x as u64); }
    for x in b.words { t = t.wrapping_mul(3).wrapping_add(x as u64); }
    for x in b.longs.iter() { t = t.wrapping_mul(3).wrapping_add(*x); }
    t = t.wrapping_add(b.s.len() as u64).wrapping_add(b.inner.x as u64).wrapping_add(b.inner.y as u64);
    for i in b.arr.iter() { t = t.wrapping_mul(5).wrapping_add(i.x as u64 + i.y as u64); }
    for r in b.grid.iter() { for c in r.iter() { t = t.wrapping_mul(7).wrapping_add(*c as u64); } }
    for r in b.refs.iter() { t = t.wrapping_mul(11).wrapping_add(r.f as u64); }
    for r in b.sl.iter() { t = t.wrapping_mul(13).wrapping_add(r.g as i64 as u64); }
    for r in b.bs.iter() { t = t.wrapping_mul(17).wrapping_add(*r as u64); }
    t.wrapping_add(b.tup.0 as u64).wrapping_add(b.tup.1.f as u64)
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u64(sum(&B1));
    print_u64(sum(&B2));
    print_u64(sum(B1.next.next));
    print_usize(B1.bytes.len() + B1.sl.len() * 10 + B2.sl.len() * 100 + B1.refs.len() * 1000 + B1.bs.len() * 10000);
    print_bool(core::ptr::eq(B1.next.next, &B1));
    print_bool(core::ptr::eq(B1.refs[1], &B1));
    print_bool(core::ptr::eq(B1.sl[0], B1.sl[2]));
    print_bool(core::ptr::eq(B2.tup.1, &B1));
    print_str(B1.next.s);
    print_bool(B2.s == OUT);
    // copies of a cycle member by value: the copy is detached, the fields are the same
    let mut c = B1;
    c.inner.x = 99; c.bytes[0] = 42; c.arr[1].y = -1; c.longs[1] = 12345;
    print_u32(c.inner.x + B1.inner.x);
    print_u32(c.bytes[0] as u32 + B1.bytes[0] as u32);
    print_i64(c.arr[1].y + B1.arr[1].y);
    print_u64(c.longs[1] + B1.longs[1]);
    print_bool(core::ptr::eq(c.next, &B2));
    print_bool(c.inner == B1.inner);
    print_bool(B1.arr[0] == Inner { x: 1, y: 2 });
    let d = *B2.next;
    print_u64(sum(&d));
    let arr = B1.refs;
    print_bool(core::ptr::eq(arr[0], &B2));
    let inner_copy = B2.arr;
    print_u32(inner_copy[1].x);
    print_f64(B1.c + B2.c);
    print_bool(B1.d && !B2.d);
    print_u32(B1.e as u32 + B2.e as u32);
}
