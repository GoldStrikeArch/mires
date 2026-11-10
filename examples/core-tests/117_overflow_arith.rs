//! Overflow-reporting integer arithmetic at every width, checked against a native oracle.
//!
//! `overflowing_*`, `checked_*`, `saturating_*` and the `Step` behind a `Range` all reach the MIR
//! operators `AddWithOverflow`, `SubWithOverflow` and `MulWithOverflow`, whose result is a
//! `(T, bool)` pair. Most uses take that pair apart at once, so the lowering carries it as the
//! exact JavaScript result and reads the two fields off it; a pair that escapes whole (here through
//! `black_box`, or as a non-inlined function's return value) is still a two-element array. Both
//! shapes are swept here over every width — `i8` to `i128`, `u8` to `u128`, and the 32 bit `isize`
//! and `usize` of this target — at the edges where an answer flips: `MIN`, `MAX`, `±1`, `0`, the
//! half-width carry of a product and the largest square that still fits. A 32 bit product is not
//! exact in a double, which is why the wrapped half must come from `Math.imul`.
//!
//! Each sweep folds every result into a 32 bit FNV-1a hash and prints it, so a single wrong bit
//! anywhere changes a printed line, and prints the `overflowing_*` results of a few boundary pairs
//! outright. The sweeps run several rounds so that the arithmetic is optimized code, not only the
//! interpreter's first pass.
//!
//! `escapes` then lets a pair leave its op every way a program can — a call argument, a return
//! value, an array element, a struct field, a closure capture, an `Option` payload, `black_box`,
//! a `&` and a `&mut` borrow, two pairs into one call, a projected pair that also escapes, a pair
//! read after a recursive call and one carried across a loop back edge — and hashes each shape on
//! its own. It spells the operators as the `*_with_overflow` intrinsics, which rustc lowers to the
//! MIR operators in this crate's own bodies, so every shape reaches the lowering's escape analysis
//! even without MIR inlining.
//!
//! The expectations were validated by transplanting this file onto `std` with `isize`/`usize`
//! spelled `i32`/`u32` (the target's pointer width) and running it at `-O` and `-O0` with
//! `-C overflow-checks=off`, which agree.

#![no_std]
#![no_main]
#![feature(core_intrinsics)]
#![allow(internal_features)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::intrinsics::{add_with_overflow, mul_with_overflow, sub_with_overflow};

/// How many times each sweep runs over its value table.
const ROUNDS: u32 = 6;

/// The word a `checked_*` that answered `None` folds as.
const NONE: u32 = 0x9e37_79b9;

/// A 32 bit FNV-1a accumulator over 32 bit words.
#[derive(Clone, Copy)]
struct Hash(u32);

impl Hash {
    fn new() -> Hash {
        Hash(0x811c_9dc5)
    }

    fn word(&mut self, word: u32) {
        self.0 = (self.0 ^ word).wrapping_mul(0x0100_0193);
    }

    fn flag(&mut self, flag: bool) {
        self.word(if flag { 0x5bd1_e995 } else { 0x1b87_3593 });
    }
}

/// A value that folds into a [`Hash`] and prints itself.
trait Value: Copy {
    fn fold(self, hash: &mut Hash);
    fn print(self);
}

macro_rules! narrow {
    ($($t:ty => $print:ident as $via:ty),*) => {$(
        impl Value for $t {
            fn fold(self, hash: &mut Hash) {
                hash.word(self as u32);
            }
            fn print(self) {
                $print(self as $via);
            }
        }
    )*};
}

narrow!(i8 => print_i32 as i32, i16 => print_i32 as i32, i32 => print_i32 as i32,
    isize => print_i32 as i32, u8 => print_u32 as u32, u16 => print_u32 as u32,
    u32 => print_u32 as u32, usize => print_u32 as u32);

macro_rules! wide {
    ($($t:ty),*) => {$(
        impl Value for $t {
            fn fold(self, hash: &mut Hash) {
                let mut rest = self as u128;
                for _ in 0..(<$t>::BITS / 32) {
                    hash.word(rest as u32);
                    rest >>= 32;
                }
            }
            fn print(self) {
                if <$t>::BITS > 64 {
                    print_i64(((self as u128) >> 64) as i64);
                }
                print_i64(self as i64);
            }
        }
    )*};
}

wide!(i64, u64, i128, u128);

/// An overflowing operation whose pair leaves this function whole.
macro_rules! escaping {
    ($($name:ident: $t:ty),*) => {$(
        #[inline(never)]
        fn $name(op: u32, a: $t, b: $t) -> ($t, bool) {
            match op {
                0 => a.overflowing_add(b),
                1 => a.overflowing_sub(b),
                _ => a.overflowing_mul(b),
            }
        }
    )*};
}

escaping!(esc_i8: i8, esc_i16: i16, esc_i32: i32, esc_i64: i64, esc_i128: i128,
    esc_isize: isize, esc_u8: u8, esc_u16: u16, esc_u32: u32, esc_u64: u64, esc_u128: u128,
    esc_usize: usize);

/// Folds both halves of a pair.
fn fold_pair<T: Value>(hash: &mut Hash, pair: (T, bool)) {
    pair.0.fold(hash);
    hash.flag(pair.1);
}

/// A pair in a struct field.
#[derive(Clone, Copy)]
struct Holder<T> {
    pair: (T, bool),
    tag: u32,
}

#[inline(never)]
fn sink<T>(pair: (T, bool)) -> (T, bool) {
    pair
}

#[inline(never)]
fn sink2<T: Value>(hash: &mut Hash, p: (T, bool), q: (T, bool)) -> bool {
    p.0.fold(hash);
    q.0.fold(hash);
    p.1 ^ q.1
}

#[inline(never)]
fn by_ref<T: Copy>(pair: &(T, bool)) -> (T, bool) {
    *pair
}

#[inline(never)]
fn by_mut<T: Copy>(pair: &mut (T, bool)) {
    pair.1 = !pair.1;
}

#[inline(never)]
fn call_it<T, F: Fn() -> (T, bool)>(f: F) -> (T, bool) {
    f()
}

/// Defines `escapes(values: &[$t])`: every way a pair can leave the op that made it, each shape
/// hashed on its own, for every pair of `values`. A macro rather than a generic function, because
/// the `*_with_overflow` intrinsics are only defined at a concrete integer type.
macro_rules! escapes {
    ($t:ty) => {
        /// A pair returned whole.
        #[inline(never)]
        fn made(a: $t, b: $t) -> ($t, bool) {
            add_with_overflow(a, b)
        }

        #[inline(never)]
        fn made_product(a: $t, b: $t) -> ($t, bool) {
            mul_with_overflow(a, b)
        }

        /// `p` and `s` are computed before the recursive call; both halves of `p` are read after
        /// it.
        #[inline(never)]
        fn deep(depth: u32, a: $t, b: $t) -> ($t, bool) {
            let p = mul_with_overflow(a, b);
            let s = add_with_overflow(a, b);
            if depth == 0 {
                return (s.0, s.1 ^ p.1);
            }
            let q = deep(depth - 1, b, s.0);
            let r = add_with_overflow(p.0, q.0);
            (r.0, r.1 ^ p.1 ^ q.1)
        }

        #[inline(never)]
        fn escapes(values: &[$t]) {
            let mut h = [Hash::new(); 13];
            for (i, &a) in values.iter().enumerate() {
                for (j, &b) in values.iter().enumerate() {
                    let k = i ^ j;
                    fold_pair(&mut h[0], sink(add_with_overflow(a, b)));
                    fold_pair(&mut h[0], sink(mul_with_overflow(a, b)));
                    fold_pair(&mut h[1], made(a, b));
                    fold_pair(&mut h[1], made_product(a, b));
                    let mut slots = [(a, false), (b, true)];
                    slots[k & 1] = add_with_overflow(a, b);
                    slots[(k >> 1) & 1] = mul_with_overflow(b, a);
                    fold_pair(&mut h[2], slots[black_box(k) & 1]);
                    fold_pair(&mut h[2], slots[black_box(k + 1) & 1]);
                    let held = black_box(Holder {
                        pair: mul_with_overflow(a, b),
                        tag: 7,
                    });
                    fold_pair(&mut h[3], held.pair);
                    h[3].flag(held.tag == 7);
                    let captured = sub_with_overflow(a, b);
                    fold_pair(&mut h[4], call_it(move || captured));
                    let wrapped = Some(add_with_overflow(a, b));
                    let unwrapped = match black_box(wrapped) {
                        Some(pair) => pair,
                        None => (a, false),
                    };
                    fold_pair(&mut h[5], unwrapped);
                    fold_pair(&mut h[6], black_box(add_with_overflow(a, b)));
                    fold_pair(&mut h[6], black_box(mul_with_overflow(a, b)));
                    let borrowed = add_with_overflow(a, b);
                    fold_pair(&mut h[7], by_ref(&borrowed));
                    let mut mutated = mul_with_overflow(a, b);
                    by_mut(&mut mutated);
                    fold_pair(&mut h[8], mutated);
                    let both = sink2(&mut h[9], add_with_overflow(a, b), sub_with_overflow(b, a));
                    h[9].flag(both);
                    let p = add_with_overflow(a, b);
                    let q = sink(p);
                    fold_pair(&mut h[10], (p.0, p.1 ^ q.1));
                    q.0.fold(&mut h[10]);
                    fold_pair(&mut h[11], deep(3, a, b));
                }
            }
            let mut acc = values[0];
            let mut prev = (values[1], false);
            for &x in values {
                let cur = add_with_overflow(acc, x);
                fold_pair(&mut h[12], prev);
                prev = cur;
                let m = mul_with_overflow(cur.0, x);
                acc = m.0;
                h[12].flag(m.1);
            }
            fold_pair(&mut h[12], prev);
            acc.fold(&mut h[12]);
            print_str("escapes");
            for hash in h {
                print_u32(hash.0);
            }
        }
    };
}

/// Sweeps every pair of `values` through every operation, prints the hashes, then prints the
/// `overflowing_*` results of `edges` outright.
macro_rules! sweep {
    ($name:ident, $label:expr, $t:ty, $escape:ident, $values:expr, $edges:expr) => {
        #[inline(never)]
        fn $name() {
            let values: &[$t] = black_box(&$values);
            let mut overflowing = Hash::new();
            let mut checked = Hash::new();
            let mut wrapping = Hash::new();
            let mut saturating = Hash::new();
            let mut escaped = Hash::new();
            let mut round = 0;
            while round < black_box(ROUNDS) {
                for &a in values {
                    for &b in values {
                        let (r, o) = a.overflowing_add(b);
                        r.fold(&mut overflowing);
                        overflowing.flag(o);
                        let (r, o) = a.overflowing_sub(b);
                        r.fold(&mut overflowing);
                        overflowing.flag(o);
                        let (r, o) = a.overflowing_mul(b);
                        r.fold(&mut overflowing);
                        overflowing.flag(o);

                        for result in [a.checked_add(b), a.checked_sub(b), a.checked_mul(b)] {
                            match result {
                                Some(r) => r.fold(&mut checked),
                                None => checked.word(NONE),
                            }
                        }
                        checked.flag(a.checked_add(b).is_none());
                        checked.flag(a.checked_mul(b).is_some());

                        a.wrapping_add(b).fold(&mut wrapping);
                        a.wrapping_sub(b).fold(&mut wrapping);
                        a.wrapping_mul(b).fold(&mut wrapping);

                        a.saturating_add(b).fold(&mut saturating);
                        a.saturating_sub(b).fold(&mut saturating);
                        a.saturating_mul(b).fold(&mut saturating);

                        for op in 0..3 {
                            let pair = black_box($escape(op, a, b));
                            pair.0.fold(&mut escaped);
                            escaped.flag(pair.1);
                        }
                        let pair = black_box(a.overflowing_mul(b));
                        pair.0.fold(&mut escaped);
                        escaped.flag(pair.1);
                    }
                }
                round += 1;
            }
            print_str($label);
            print_u32(overflowing.0);
            print_u32(checked.0);
            print_u32(wrapping.0);
            print_u32(saturating.0);
            print_u32(escaped.0);
            let mut flags = 0u32;
            for (i, &(a, b)) in black_box(&$edges).iter().enumerate() {
                let a: $t = a;
                let b: $t = b;
                let (r, o) = a.overflowing_add(b);
                r.print();
                flags |= (o as u32) << (3 * i);
                let (r, o) = a.overflowing_sub(b);
                r.print();
                flags |= (o as u32) << (3 * i + 1);
                let (r, o) = a.overflowing_mul(b);
                r.print();
                flags |= (o as u32) << (3 * i + 2);
            }
            print_u32(flags);
            escapes!($t);
            escapes(values);
        }
    };
}

sweep!(sweep_i8, "i8", i8, esc_i8,
    [i8::MIN, i8::MIN + 1, -16, -2, -1, 0, 1, 2, 11, 12, 16, i8::MAX - 1, i8::MAX],
    [(i8::MAX, 1), (i8::MIN, -1), (i8::MIN, i8::MIN), (12, 11)]);
sweep!(sweep_i16, "i16", i16, esc_i16,
    [i16::MIN, i16::MIN + 1, -256, -2, -1, 0, 1, 2, 181, 182, 256, i16::MAX - 1, i16::MAX],
    [(i16::MAX, 1), (i16::MIN, -1), (i16::MIN, i16::MIN), (182, 181)]);
sweep!(sweep_i32, "i32", i32, esc_i32,
    [i32::MIN, i32::MIN + 1, -65536, -2, -1, 0, 1, 2, 46340, 46341, 65536, i32::MAX - 1,
        i32::MAX],
    [(i32::MAX, 1), (i32::MIN, -1), (i32::MIN, i32::MIN), (46341, 46340)]);
sweep!(sweep_isize, "isize", isize, esc_isize,
    [isize::MIN, isize::MIN + 1, -65536, -2, -1, 0, 1, 2, 46340, 46341, 65536, isize::MAX - 1,
        isize::MAX],
    [(isize::MAX, 1), (isize::MIN, -1), (isize::MIN, isize::MIN), (46341, 46340)]);
sweep!(sweep_i64, "i64", i64, esc_i64,
    [i64::MIN, i64::MIN + 1, -(1 << 32), -2, -1, 0, 1, 2, 3_037_000_499, 3_037_000_500, 1 << 32,
        i64::MAX - 1, i64::MAX],
    [(i64::MAX, 1), (i64::MIN, -1), (i64::MIN, i64::MIN), (3_037_000_500, 3_037_000_499)]);
sweep!(sweep_i128, "i128", i128, esc_i128,
    [i128::MIN, i128::MIN + 1, -(1 << 64), -2, -1, 0, 1, 2, 13_043_817_825_332_782_212,
        13_043_817_825_332_782_213, 1 << 64, i128::MAX - 1, i128::MAX],
    [(i128::MAX, 1), (i128::MIN, -1), (i128::MIN, i128::MIN),
        (13_043_817_825_332_782_213, 13_043_817_825_332_782_212)]);
sweep!(sweep_u8, "u8", u8, esc_u8,
    [0, 1, 2, 3, 15, 16, 17, 127, 128, u8::MAX - 1, u8::MAX],
    [(u8::MAX, 1), (0, 1), (u8::MAX, u8::MAX), (16, 16)]);
sweep!(sweep_u16, "u16", u16, esc_u16,
    [0, 1, 2, 3, 255, 256, 257, 32767, 32768, u16::MAX - 1, u16::MAX],
    [(u16::MAX, 1), (0, 1), (u16::MAX, u16::MAX), (256, 256)]);
sweep!(sweep_u32, "u32", u32, esc_u32,
    [0, 1, 2, 3, 65535, 65536, 65537, 0x7fff_ffff, 0x8000_0000, u32::MAX - 1, u32::MAX],
    [(u32::MAX, 1), (0, 1), (u32::MAX, u32::MAX), (65536, 65536)]);
sweep!(sweep_usize, "usize", usize, esc_usize,
    [0, 1, 2, 3, 65535, 65536, 65537, 0x7fff_ffff, 0x8000_0000, usize::MAX - 1, usize::MAX],
    [(usize::MAX, 1), (0, 1), (usize::MAX, usize::MAX), (65536, 65536)]);
sweep!(sweep_u64, "u64", u64, esc_u64,
    [0, 1, 2, 3, u32::MAX as u64, 1 << 32, (1 << 32) + 1, i64::MAX as u64, 1 << 63, u64::MAX - 1,
        u64::MAX],
    [(u64::MAX, 1), (0, 1), (u64::MAX, u64::MAX), (1 << 32, 1 << 32)]);
sweep!(sweep_u128, "u128", u128, esc_u128,
    [0, 1, 2, 3, u64::MAX as u128, 1 << 64, (1 << 64) + 1, i128::MAX as u128, 1 << 127,
        u128::MAX - 1, u128::MAX],
    [(u128::MAX, 1), (0, 1), (u128::MAX, u128::MAX), (1 << 64, 1 << 64)]);

/// `Range::next` steps with `Step::forward_unchecked`, the overflowing add `fflate`'s byte loop
/// spends its time in; the ranges end at the top of their type and start at the bottom.
#[inline(never)]
fn ranges() {
    let mut hash = Hash::new();
    let mut round = 0;
    while round < black_box(ROUNDS) {
        for i in black_box(i32::MAX - 300)..black_box(i32::MAX) {
            i.fold(&mut hash);
        }
        for i in black_box(i32::MIN)..black_box(i32::MIN + 300) {
            i.fold(&mut hash);
        }
        for i in black_box(u8::MAX - 40)..=black_box(u8::MAX) {
            i.fold(&mut hash);
        }
        for i in black_box(i64::MAX - 40)..black_box(i64::MAX) {
            i.fold(&mut hash);
        }
        for i in (black_box(usize::MAX - 90)..black_box(usize::MAX)).rev() {
            i.fold(&mut hash);
        }
        let mut sum = 0i16;
        for i in black_box(-300i16)..black_box(300) {
            sum = sum.wrapping_add(i.checked_add_unsigned(black_box(7)).unwrap_or(0));
        }
        sum.fold(&mut hash);
        round += 1;
    }
    print_str("ranges");
    print_u32(hash.0);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    sweep_i8();
    sweep_i16();
    sweep_i32();
    sweep_isize();
    sweep_i64();
    sweep_i128();
    sweep_u8();
    sweep_u16();
    sweep_u32();
    sweep_usize();
    sweep_u64();
    sweep_u128();
    ranges();
}
