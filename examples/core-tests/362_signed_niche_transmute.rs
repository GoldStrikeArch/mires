//! Signed niche enum <-> integer transmutes (MIRES-010).
//!
//! Covers `Option<Ordering>`, `Option<char>`, `Option<Option<bool>>`, `Option<NonZero<i8>>`,
//! `Option<NonZero<i32>>`, and `ControlFlow<bool>` in both directions (decode & encode) with
//! signed scalars whose niche has its high bit set, plus `const` decoding and `&`/`&mut`
//! observations. On base `6efb9ef` this compiles and silently miscompiles (`output-mismatch`).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::mem::transmute;
use core::num::NonZero;
use core::ops::ControlFlow;

#[inline(never)]
fn ord_enc_i8(o: Option<Ordering>) -> i8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn ord_dec_i8(b: i8) -> Option<Ordering> {
    unsafe { transmute(b) }
}
#[inline(never)]
fn char_enc_i32(o: Option<char>) -> i32 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn char_dec_i32(v: i32) -> Option<char> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn char_enc_u32(o: Option<char>) -> u32 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn char_dec_u32(v: u32) -> Option<char> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn opt_opt_bool_enc_i8(o: Option<Option<bool>>) -> i8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn opt_opt_bool_dec_i8(b: i8) -> Option<Option<bool>> {
    unsafe { transmute(b) }
}
#[inline(never)]
fn opt_opt_bool_enc_u8(o: Option<Option<bool>>) -> u8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn opt_opt_bool_dec_u8(b: u8) -> Option<Option<bool>> {
    unsafe { transmute(b) }
}
#[inline(never)]
fn nzi8_enc_i8(o: Option<NonZero<i8>>) -> i8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn nzi8_dec_i8(b: i8) -> Option<NonZero<i8>> {
    unsafe { transmute(b) }
}
#[inline(never)]
fn nzi8_enc_u8(o: Option<NonZero<i8>>) -> u8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn nzi8_dec_u8(b: u8) -> Option<NonZero<i8>> {
    unsafe { transmute(b) }
}
#[inline(never)]
fn nzi32_enc_i32(o: Option<NonZero<i32>>) -> i32 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn nzi32_dec_i32(v: i32) -> Option<NonZero<i32>> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn cf_enc_i8(c: ControlFlow<bool>) -> i8 {
    unsafe { transmute(c) }
}
#[inline(never)]
fn cf_dec_i8(b: i8) -> ControlFlow<bool> {
    unsafe { transmute(b) }
}

const CONST_OPT_ORD_NONE: Option<Ordering> = None;
const CONST_OPT_ORD_LESS: Option<Ordering> = Some(Ordering::Less);
const CONST_OPT_ORD_DEC: Option<Ordering> = unsafe { transmute::<i8, Option<Ordering>>(-2) };

#[inline(never)]
fn observe_ref(o: &Option<Ordering>) -> i32 {
    match o {
        None => -99,
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) => 0,
        Some(Ordering::Greater) => 1,
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Option<Ordering> <-> i8
    let ords = [
        None,
        Some(Ordering::Less),
        Some(Ordering::Equal),
        Some(Ordering::Greater),
    ];
    for o in ords {
        let b = ord_enc_i8(o);
        print_i32(b as i32);
        let d = ord_dec_i8(b);
        print_bool(d == o);
        print_i32(observe_ref(&d));
    }

    // Explicit -2 / -1 probes and &mut Option::take
    let mut cell = ord_dec_i8(-2);
    print_bool(cell.is_none());
    print_bool(cell.take().is_none());
    cell = ord_dec_i8(-1);
    print_bool(cell.take() == Some(Ordering::Less));
    print_bool(cell.is_none());

    // Constants
    print_bool(CONST_OPT_ORD_NONE.is_none());
    print_bool(CONST_OPT_ORD_LESS == Some(Ordering::Less));
    print_bool(CONST_OPT_ORD_DEC.is_none());

    // Option<char> <-> i32 / u32
    let chars = [None, Some('A'), Some('\u{10FFFF}')];
    for c in chars {
        let s = char_enc_i32(c);
        print_i32(s);
        print_bool(char_dec_i32(s) == c);
        let u = char_enc_u32(c);
        print_u32(u);
        print_bool(char_dec_u32(u) == c);
    }
    print_bool(char_dec_i32(-1).is_none());

    // Option<Option<bool>> <-> i8 / u8
    let oobs = [None, Some(None), Some(Some(false)), Some(Some(true))];
    for v in oobs {
        let s = opt_opt_bool_enc_i8(v);
        print_i32(s as i32);
        print_bool(opt_opt_bool_dec_i8(s) == v);
        let u = opt_opt_bool_enc_u8(v);
        print_u32(u as u32);
        print_bool(opt_opt_bool_dec_u8(u) == v);
    }
    print_bool(opt_opt_bool_dec_i8(-1).is_none());

    // Option<NonZero<i8>> <-> i8 / u8
    for v in [0i8, -5, i8::MIN, 1, 127] {
        let d = nzi8_dec_i8(v);
        print_i32(nzi8_enc_i8(d) as i32);
        print_bool(d == NonZero::new(v));
        let u = nzi8_enc_u8(d);
        print_u32(u as u32);
        print_bool(nzi8_dec_u8(u) == d);
    }

    // Option<NonZero<i32>> <-> i32
    for v in [0i32, -1, i32::MIN, 42] {
        let d = nzi32_dec_i32(v);
        print_i32(nzi32_enc_i32(d));
        print_bool(d == NonZero::new(v));
    }

    // ControlFlow<bool> <-> i8
    for c in [
        ControlFlow::Continue(()),
        ControlFlow::Break(false),
        ControlFlow::Break(true),
    ] {
        let b = cf_enc_i8(c);
        print_i32(b as i32);
        print_bool(cf_dec_i8(b) == c);
    }
}
