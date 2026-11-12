//! Verifier round-2 probe k2 (copies never read after the write): by-value copies of pooled data, written; the tables must not change.
#![no_std]
#![no_main]
#![allow(dead_code, unused, static_mut_refs)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::Cell;
use core::hint::black_box;

#[derive(Clone, Copy, PartialEq)]
pub struct P { x: u32, y: u32, tag: &'static str }
#[derive(Clone, Copy)]
pub struct Q { p: P, arr: [u32; 3], t: (u32, u32) }
pub struct Holder { p: P, n: u32 }
static mut SLOT: P = P { x: 0, y: 0, tag: "" };

const PS: &[P] = &[
    P { x: 0, y: 1, tag: "k-point-0" },
    P { x: 1, y: 4, tag: "k-point-1" },
    P { x: 2, y: 7, tag: "k-point-2" },
    P { x: 3, y: 10, tag: "k-point-3" },
    P { x: 4, y: 13, tag: "k-point-4" },
    P { x: 5, y: 16, tag: "k-point-5" },
    P { x: 6, y: 19, tag: "k-point-6" },
    P { x: 7, y: 22, tag: "k-point-7" },
    P { x: 8, y: 25, tag: "k-point-8" },
    P { x: 9, y: 28, tag: "k-point-9" },
    P { x: 10, y: 31, tag: "k-point-10" },
    P { x: 11, y: 34, tag: "k-point-11" }
];
const QS: &[Q] = &[
    Q { p: P { x: 20, y: 40, tag: "k-inner-0" }, arr: [0, 1, 2], t: (5, 6) },
    Q { p: P { x: 21, y: 41, tag: "k-inner-1" }, arr: [1, 2, 3], t: (6, 7) },
    Q { p: P { x: 22, y: 42, tag: "k-inner-2" }, arr: [2, 3, 4], t: (7, 8) },
    Q { p: P { x: 23, y: 43, tag: "k-inner-3" }, arr: [3, 4, 5], t: (8, 9) }
];
const AR: &[[u32; 4]] = &[
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [8, 9, 10, 11],
    [12, 13, 14, 15],
    [16, 17, 18, 19],
    [20, 21, 22, 23],
    [24, 25, 26, 27],
    [28, 29, 30, 31],
    [32, 33, 34, 35],
    [36, 37, 38, 39],
    [40, 41, 42, 43],
    [44, 45, 46, 47],
    [48, 49, 50, 51],
    [52, 53, 54, 55],
    [56, 57, 58, 59],
    [60, 61, 62, 63],
    [64, 65, 66, 67],
    [68, 69, 70, 71],
    [72, 73, 74, 75],
    [76, 77, 78, 79],
    [80, 81, 82, 83],
    [84, 85, 86, 87],
    [88, 89, 90, 91],
    [92, 93, 94, 95],
    [96, 97, 98, 99],
    [100, 101, 102, 103],
    [104, 105, 106, 107],
    [108, 109, 110, 111],
    [112, 113, 114, 115],
    [116, 117, 118, 119]
];
const REF: &Q = &Q { p: P { x: 500, y: 501, tag: "k-ref-inner-with-a-long-enough-name-to-pass-the-threshold-of-256-bytes-000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000" }, arr: [502, 503, 504], t: (505, 506) };

#[inline(never)]
fn bump(mut p: P) -> u32 { p.x += 1000; p.y += 1; p.x }

#[inline(never)]
fn sum_ps() -> u32 { PS.iter().map(|p| p.x * 3 + p.y + p.tag.len() as u32).sum() }
#[inline(never)]
fn sum_qs() -> u32 {
    let mut t = 0;
    for q in QS { t += q.p.x + q.p.y * 2 + q.arr[0] + q.arr[1] * 3 + q.arr[2] * 5 + q.t.0 * 7 + q.t.1; }
    t
}
#[inline(never)]
fn sum_ar() -> u32 { AR.iter().map(|row| row[0] + row[1] * 2 + row[2] * 3 + row[3] * 4).sum() }
#[inline(never)]
fn sum_ref() -> u32 { REF.p.x + REF.p.y + REF.arr[0] + REF.arr[1] * 2 + REF.arr[2] + REF.t.0 + REF.t.1 }
#[inline(never)]
fn sum_all_again() -> u32 {
    let mut t = 0;
    for p in PS.iter() { t += p.x + p.y; }
    for q in QS.iter() { t += q.p.x + q.arr[0] + q.t.0; }
    for row in AR.iter() { t += row[0] + row[3]; }
    t + REF.p.x + REF.arr[1]
}

#[inline(never)]
fn check(shape: u32) {
    let (a, b, c, d, e) = (sum_ps(), sum_qs(), sum_ar(), sum_ref(), sum_all_again());
    if a != 518 || b != 736 || c != 18000 || d != 4024 || e != 4967 {
        print_str("CHANGED after shape");
        print_u32(shape);
        print_u32(a); print_u32(b); print_u32(c); print_u32(d); print_u32(e);
    }
}

#[inline(never)]
fn shape_0() {
    // index copy (known shape)
    let mut p = PS[4]; p.x = 99;
}
#[inline(never)]
fn shape_1() {
    // get+deref
    let mut p = *PS.get(4).unwrap(); p.x = 99;
}
#[inline(never)]
fn shape_2() {
    // iter nth deref
    let mut p = *PS.iter().nth(4).unwrap(); p.x = 99;
}
#[inline(never)]
fn shape_3() {
    // iter copied nth
    let mut p = PS.iter().copied().nth(4).unwrap(); p.x = 99;
}
#[inline(never)]
fn shape_4() {
    // clone
    let mut p = PS[4].clone(); p.x = 99;
}
#[inline(never)]
fn shape_5() {
    // nested struct copy, write nested fields
    let mut q = QS[1]; q.p.x = 99; q.arr[0] = 99; q.t.0 = 99;
}
#[inline(never)]
fn shape_6() {
    // array field copy
    let mut a = QS[1].arr; a[0] = 99;
}
#[inline(never)]
fn shape_7() {
    // struct field copy
    let mut inner = QS[1].p; inner.x = 99;
}
#[inline(never)]
fn shape_8() {
    // tuple field copy
    let mut t = QS[1].t; t.0 = 99;
}
#[inline(never)]
fn shape_9() {
    // struct update syntax
    let mut q2 = Q { arr: [1, 2, 3], ..QS[1] }; q2.p.x = 99;
}
#[inline(never)]
fn shape_10() {
    // deref of &Q const
    let mut r = *REF; r.p.x = 99; r.arr[1] = 99;
}
#[inline(never)]
fn shape_11() {
    // row of 2-D table
    let mut row = AR[2]; row[0] = 99;
}
#[inline(never)]
fn shape_12() {
    // to_vec
    let mut v = PS.to_vec(); v[0].x = 99; v[4].x = 98;
}
#[inline(never)]
fn shape_13() {
    // iter cloned collect
    let mut v: Vec<P> = PS.iter().cloned().collect(); v[1].x = 99;
}
#[inline(never)]
fn shape_14() {
    // copy_from_slice
    let mut buf = [P { x: 0, y: 0, tag: "" }; 12]; buf.copy_from_slice(PS); buf[2].x = 99;
}
#[inline(never)]
fn shape_15() {
    // array repeat of element
    let mut buf = [PS[0]; 4]; buf[0].x = 99; print_u32(buf[1].x);
}
#[inline(never)]
fn shape_16() {
    // box of element
    let mut b = Box::new(PS[3]); b.x = 99;
}
#[inline(never)]
fn shape_17() {
    // ptr::read
    let mut p = unsafe { core::ptr::read(&PS[5]) }; p.x = 99;
}
#[inline(never)]
fn shape_18() {
    // mem::swap
    let mut p = PS[6]; let mut other = PS[7]; core::mem::swap(&mut p, &mut other); other.x = 99; p.x = 98;
}
#[inline(never)]
fn shape_19() {
    // option wrap
    let mut o = Some(PS[7]); if let Some(p) = o.as_mut() { p.x = 99; }
}
#[inline(never)]
fn shape_20() {
    // tuple wrap
    let mut pair = (PS[8], PS[9]); pair.0.x = 99; pair.1.y = 99;
}
#[inline(never)]
fn shape_21() {
    // by-value argument
    print_u32(bump(PS[10]));
}
#[inline(never)]
fn shape_22() {
    // match binding
    match PS[11] { mut p => { p.x = 99; } }
}
#[inline(never)]
fn shape_23() {
    // for over copied
    for mut p in PS.iter().copied() { p.x = 0; }
}
#[inline(never)]
fn shape_24() {
    // first copied
    let mut c = PS.first().copied().unwrap(); c.y = 99;
}
#[inline(never)]
fn shape_25() {
    // try_into array copy + sort
    let mut arr: [P; 12] = PS.try_into().unwrap(); arr.sort_by_key(|p| 100 - p.y); arr[0].x = 99;
}
#[inline(never)]
fn shape_26() {
    // deref of array-ref view
    let mut arr2 = *<&[P; 12]>::try_from(PS).unwrap(); arr2[0].x = 99;
}
#[inline(never)]
fn shape_27() {
    // reborrow field of copy
    let mut q = QS[0]; let inner = &mut q.p; inner.y = 99;
}
#[inline(never)]
fn shape_28() {
    // cell get/set
    let c = Cell::new(PS[1]); let mut p = c.get(); p.x = 99; c.set(p); let mut again = c.replace(PS[2]); again.y = 98;
}
#[inline(never)]
fn shape_29() {
    // array literal of elements
    let mut ps2 = [PS[0], PS[1]]; ps2[1].x = 99;
}
#[inline(never)]
fn shape_30() {
    // move closure copy
    let mut p = PS[2]; let mut f = move || { p.x += 1; p.x }; print_u32(f());
}
#[inline(never)]
fn shape_31() {
    // 2-D row via iter
    let mut total = 0; for row in AR.iter() { let mut r = *row; r[3] = 7; total += r[3]; } print_u32(total);
}
#[inline(never)]
fn shape_32() {
    // split_first copy
    let (first, _) = PS.split_first().unwrap(); let mut p = *first; p.x = 99;
}
#[inline(never)]
fn shape_33() {
    // last copy via pattern
    if let [.., last] = PS { let mut p = *last; p.x = 99; }
}
#[inline(never)]
fn shape_34() {
    // nested copy then copy again
    let q = QS[2]; let mut p = q.p; p.x = 99; let mut a = q.arr; a[2] = 99;
}
#[inline(never)]
fn shape_35() {
    // copy into static mut
    unsafe { SLOT = PS[3]; SLOT.x = 99; }
}
#[inline(never)]
fn shape_36() {
    // copy into struct field then write
    let mut h = Holder { p: PS[5], n: 0 }; h.p.x = 99; h.n = 1;
}
#[inline(never)]
fn shape_37() {
    // mem::replace
    let mut p = PS[6]; let mut old = core::mem::replace(&mut p, PS[7]); old.x = 99; p.x = 98;
}
#[inline(never)]
fn shape_38() {
    // write through &mut of copied array elem
    let mut arr = [PS[8], PS[9]]; for p in arr.iter_mut() { p.x = 99; }
}
#[inline(never)]
fn shape_39() {
    // copy via map
    let mut p = PS.get(1).map(|p| *p).unwrap(); p.x = 99;
}
#[inline(never)]
fn shape_40() {
    // rev next copied
    let mut p = PS.iter().rev().next().copied().unwrap(); p.x = 99;
}
#[inline(never)]
fn shape_41() {
    // zip copied
    for (mut a, mut b) in PS.iter().copied().zip(QS.iter().copied()) { a.x = 99; b.p.x = 99; b.arr[0] = 99; }
}
#[inline(never)]
fn shape_42() {
    // chunks copy
    for chunk in PS.chunks(5) { let mut p = chunk[0]; p.x = 99; }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(sum_ps()); print_u32(sum_qs()); print_u32(sum_ar()); print_u32(sum_ref()); print_u32(sum_all_again());
    shape_0(); check(0);
    shape_1(); check(1);
    shape_2(); check(2);
    shape_3(); check(3);
    shape_4(); check(4);
    shape_5(); check(5);
    shape_6(); check(6);
    shape_7(); check(7);
    shape_8(); check(8);
    shape_9(); check(9);
    shape_10(); check(10);
    shape_11(); check(11);
    shape_12(); check(12);
    shape_13(); check(13);
    shape_14(); check(14);
    shape_15(); check(15);
    shape_16(); check(16);
    shape_17(); check(17);
    shape_18(); check(18);
    shape_19(); check(19);
    shape_20(); check(20);
    shape_21(); check(21);
    shape_22(); check(22);
    shape_23(); check(23);
    shape_24(); check(24);
    shape_25(); check(25);
    shape_26(); check(26);
    shape_27(); check(27);
    shape_28(); check(28);
    shape_29(); check(29);
    shape_30(); check(30);
    shape_31(); check(31);
    shape_32(); check(32);
    shape_33(); check(33);
    shape_34(); check(34);
    shape_35(); check(35);
    shape_36(); check(36);
    shape_37(); check(37);
    shape_38(); check(38);
    shape_39(); check(39);
    shape_40(); check(40);
    shape_41(); check(41);
    shape_42(); check(42);
    print_u32(sum_ps()); print_u32(sum_qs()); print_u32(sum_ar()); print_u32(sum_ref()); print_u32(sum_all_again());
    print_str("done");
}
