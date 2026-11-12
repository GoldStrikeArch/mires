//! Verifier round-2 probe k3: by-value copies of pooled data, written; the tables must not change.
#![no_std]
#![no_main]
#![allow(dead_code, unused, static_mut_refs)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::rc::Rc;
use alloc::borrow::ToOwned;
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
fn pick(i: usize) -> P { PS[i] }
#[inline(never)]
fn pick_q(i: usize) -> Q { QS[i] }
#[inline(never)]
fn ident<T>(t: T) -> T { t }
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
    // clone_from_slice
    let mut buf = [P { x: 0, y: 0, tag: "" }; 12]; buf.clone_from_slice(PS); buf[5].x = 99; black_box(buf[5].x);
}
#[inline(never)]
fn shape_1() {
    // Vec::from
    let mut v = Vec::from(PS); v[6].x = 99; black_box(v[6].x);
}
#[inline(never)]
fn shape_2() {
    // extend_from_slice
    let mut v: Vec<P> = Vec::new(); v.extend_from_slice(PS); v[7].x = 99; black_box(v[7].x);
}
#[inline(never)]
fn shape_3() {
    // first_chunk deref copy (window)
    let mut a = *PS.first_chunk::<2>().unwrap(); a[0].x = 99; black_box(a[0].x);
}
#[inline(never)]
fn shape_4() {
    // sub-slice try_into array copy
    let mut a: [P; 2] = PS[3..5].try_into().unwrap(); a[1].x = 99; black_box(a[1].x);
}
#[inline(never)]
fn shape_5() {
    // to_owned
    let mut v = PS.to_owned(); v[8].x = 99; black_box(v[8].x);
}
#[inline(never)]
fn shape_6() {
    // concat
    let mut v = [PS, PS].concat(); v[9].x = 99; v[21].x = 98; black_box(v[9].x);
}
#[inline(never)]
fn shape_7() {
    // repeat
    let mut v = PS.repeat(2); v[10].x = 99; black_box(v[10].x);
}
#[inline(never)]
fn shape_8() {
    // iter map deref collect
    let mut v: Vec<P> = PS.iter().map(|p| *p).collect(); v[11].x = 99; black_box(v[11].x);
}
#[inline(never)]
fn shape_9() {
    // Vec push of elements then write
    let mut v = Vec::new(); for p in PS { v.push(*p); } for p in v.iter_mut() { p.x += 1000; } black_box(v[0].x);
}
#[inline(never)]
fn shape_10() {
    // Vec of Q then nested write
    let mut v = QS.to_vec(); v[0].p.x = 99; v[1].arr[2] = 99; v[2].t.1 = 99; black_box(v[0].p.x);
}
#[inline(never)]
fn shape_11() {
    // 2-D row copy_from_slice
    let mut row = [0u32; 4]; row.copy_from_slice(&AR[5]); row[1] = 99; black_box(row[1]);
}
#[inline(never)]
fn shape_12() {
    // 2-D first_chunk copy
    let mut rows = *AR.first_chunk::<3>().unwrap(); rows[1][1] = 99; black_box(rows[1][1]);
}
#[inline(never)]
fn shape_13() {
    // swap elements of a copy
    let mut v = PS.to_vec(); v.swap(0, 11); v[0].x = 99; v.reverse(); v[3].y = 98; black_box(v[0].x);
}
#[inline(never)]
fn shape_14() {
    // sort copy by key then write
    let mut v = PS.to_vec(); v.sort_by(|a, b| b.x.cmp(&a.x)); v[0].x = 99; black_box(v[0].x);
}
#[inline(never)]
fn shape_15() {
    // retain / dedup on copy
    let mut v = PS.to_vec(); v.retain(|p| p.x % 2 == 0); v[0].y = 99; v.dedup_by_key(|p| p.y); black_box(v.len());
}
#[inline(never)]
fn shape_16() {
    // write through &mut of REF copy fields
    let mut r = *REF; let Q { p, arr, t } = &mut r; p.y = 99; arr[0] = 99; t.1 = 99; black_box(r.p.y);
}
#[inline(never)]
fn shape_17() {
    // closure returning copy
    let get = |i: usize| PS[i]; let mut p = get(5); p.x = 99; black_box(p.x);
}
#[inline(never)]
fn shape_18() {
    // fn returning copy
    let mut p = pick(6); p.x = 99; black_box(p.x);
}
#[inline(never)]
fn shape_19() {
    // fn returning nested copy
    let mut q = pick_q(1); q.p.x = 99; q.arr[1] = 99; black_box(q.p.x);
}
#[inline(never)]
fn shape_20() {
    // copy through generic identity
    let mut p = ident(PS[7]); p.x = 99; black_box(p.x);
}
#[inline(never)]
fn shape_21() {
    // copy stored in Vec<Holder>
    let mut v = Vec::new(); v.push(Holder { p: PS[8], n: 1 }); v[0].p.x = 99; black_box(v[0].p.x);
}
#[inline(never)]
fn shape_22() {
    // min_by_key copied
    let mut p = PS.iter().copied().min_by_key(|p| p.y).unwrap(); p.x = 99; black_box(p.x);
}
#[inline(never)]
fn shape_23() {
    // find copied
    let mut p = PS.iter().find(|p| p.x == 9).copied().unwrap(); p.y = 99; black_box(p.y);
}
#[inline(never)]
fn shape_24() {
    // last cloned
    let mut p = PS.last().cloned().unwrap(); p.y = 99; black_box(p.y);
}
#[inline(never)]
fn shape_25() {
    // enumerate filter map
    let mut v: Vec<P> = PS.iter().enumerate().filter_map(|(i, p)| if i < 3 { Some(*p) } else { None }).collect(); v[2].x = 99; black_box(v[2].x);
}
#[inline(never)]
fn shape_26() {
    // chain two tables
    for mut p in PS.iter().chain(PS.iter()).copied() { p.y = 0; black_box(p.y); }
}
#[inline(never)]
fn shape_27() {
    // unzip
    let (mut xs, mut ps): (Vec<u32>, Vec<P>) = PS.iter().map(|p| (p.x, *p)).unzip(); ps[0].x = 99; xs[0] = 1; black_box(ps[0].x);
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
    print_u32(sum_ps()); print_u32(sum_qs()); print_u32(sum_ar()); print_u32(sum_ref()); print_u32(sum_all_again());
    print_str("done");
}
