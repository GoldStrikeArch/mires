//! Wide signed and unsigned niche transmutes across i16/u16, i32/u32, isize/usize, i64/u64,
//! and i128/u128 (including BigInt 64-bit pair boundaries and u128 high-bit-set niches).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::mem::transmute;
use core::num::NonZero;

#[inline(never)]
fn ord_enc_u8(o: Option<Ordering>) -> u8 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn ord_dec_u8(b: u8) -> Option<Ordering> {
    unsafe { transmute(b) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
enum OrdI16 {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum OrdI32 {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
enum OrdIsize {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
enum OrdI64 {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i128)]
enum OrdI128 {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u128)]
enum HighU128 {
    A = 0,
    B = 1,
}

#[inline(never)]
fn enc_i16(o: Option<OrdI16>) -> i16 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_i16(v: i16) -> Option<OrdI16> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_u16(o: Option<OrdI16>) -> u16 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_u16(v: u16) -> Option<OrdI16> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_i32(o: Option<OrdI32>) -> i32 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_i32(v: i32) -> Option<OrdI32> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_u32(o: Option<OrdI32>) -> u32 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_u32(v: u32) -> Option<OrdI32> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_isize(o: Option<OrdIsize>) -> isize {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_isize(v: isize) -> Option<OrdIsize> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_usize(o: Option<OrdIsize>) -> usize {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_usize(v: usize) -> Option<OrdIsize> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_i64(o: Option<OrdI64>) -> i64 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_i64(v: i64) -> Option<OrdI64> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_u64(o: Option<OrdI64>) -> u64 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_u64(v: u64) -> Option<OrdI64> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_i128(o: Option<OrdI128>) -> i128 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_i128(v: i128) -> Option<OrdI128> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_u128(o: Option<OrdI128>) -> u128 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_u128(v: u128) -> Option<OrdI128> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_hi_u128(o: Option<Option<HighU128>>) -> u128 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_hi_u128(v: u128) -> Option<Option<HighU128>> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn enc_hi_i128(o: Option<Option<HighU128>>) -> i128 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_hi_i128(v: i128) -> Option<Option<HighU128>> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_nzi64(o: Option<NonZero<i64>>) -> i64 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_nzi64(v: i64) -> Option<NonZero<i64>> {
    unsafe { transmute(v) }
}
#[inline(never)]
fn dec_nzi64_u64(v: u64) -> Option<NonZero<i64>> {
    unsafe { transmute(v) }
}

#[inline(never)]
fn enc_nzi128(o: Option<NonZero<i128>>) -> i128 {
    unsafe { transmute(o) }
}
#[inline(never)]
fn dec_nzi128(v: i128) -> Option<NonZero<i128>> {
    unsafe { transmute(v) }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Cross-signedness Option<Ordering> <-> u8
    for o in [
        None,
        Some(Ordering::Less),
        Some(Ordering::Equal),
        Some(Ordering::Greater),
    ] {
        let u = ord_enc_u8(o);
        print_u32(u as u32);
        print_bool(ord_dec_u8(u) == o);
    }
    print_bool(ord_dec_u8(254).is_none());
    print_bool(ord_dec_u8(255) == Some(Ordering::Less));

    // i16 / u16
    for o in [
        None,
        Some(OrdI16::Less),
        Some(OrdI16::Equal),
        Some(OrdI16::Greater),
    ] {
        let s = enc_i16(o);
        print_i32(s as i32);
        print_bool(dec_i16(s) == o);
        let u = enc_u16(o);
        print_u32(u as u32);
        print_bool(dec_u16(u) == o);
    }

    // i32 / u32
    for o in [
        None,
        Some(OrdI32::Less),
        Some(OrdI32::Equal),
        Some(OrdI32::Greater),
    ] {
        let s = enc_i32(o);
        print_i32(s);
        print_bool(dec_i32(s) == o);
        let u = enc_u32(o);
        print_u32(u);
        print_bool(dec_u32(u) == o);
    }

    // isize / usize
    for o in [
        None,
        Some(OrdIsize::Less),
        Some(OrdIsize::Equal),
        Some(OrdIsize::Greater),
    ] {
        let s = enc_isize(o);
        print_i32(s as i32);
        print_bool(dec_isize(s) == o);
        let u = enc_usize(o);
        print_bool(dec_usize(u) == o);
        print_bool(u == (s as usize));
    }

    // i64 / u64 (BigInt / i64_pair boundary)
    for o in [
        None,
        Some(OrdI64::Less),
        Some(OrdI64::Equal),
        Some(OrdI64::Greater),
    ] {
        let s = enc_i64(o);
        print_i64(s);
        print_bool(dec_i64(s) == o);
        let u = enc_u64(o);
        print_bool(u == (s as u64));
        print_bool(dec_u64(u) == o);
    }
    print_bool(enc_u64(None) == u64::MAX - 1);
    print_bool(enc_u64(Some(OrdI64::Less)) == u64::MAX);

    // i128 / u128 (signed high-bit niche and unsigned high-bit niche >= 2^127)
    for o in [
        None,
        Some(OrdI128::Less),
        Some(OrdI128::Equal),
        Some(OrdI128::Greater),
    ] {
        let s = enc_i128(o);
        print_bool(dec_i128(s) == o);
        let u = enc_u128(o);
        print_bool(u == (s as u128));
        print_bool(dec_u128(u) == o);
    }
    print_bool(enc_i128(None) == -2i128);
    print_bool(enc_u128(None) == u128::MAX - 1);
    print_bool(dec_i128(-2i128).is_none());
    print_bool(dec_u128(u128::MAX - 1).is_none());

    // Option<Option<HighU128>>: None has raw tag u128::MAX (high bit set!)
    for o in [
        None,
        Some(None),
        Some(Some(HighU128::A)),
        Some(Some(HighU128::B)),
    ] {
        let u = enc_hi_u128(o);
        print_bool(dec_hi_u128(u) == o);
        let s = enc_hi_i128(o);
        print_bool(dec_hi_i128(s) == o);
        print_bool(u == (s as u128));
    }
    print_bool(enc_hi_u128(None) == u128::MAX);
    print_bool(enc_hi_i128(None) == -1i128);
    print_bool(dec_hi_u128(u128::MAX).is_none());
    print_bool(dec_hi_i128(-1i128).is_none());

    // Option<NonZero<i64>> and Option<NonZero<i128>>
    for v in [0i64, -5, i64::MIN, 1, i64::MAX] {
        let d = dec_nzi64(v);
        print_bool(d == NonZero::new(v));
        print_i64(enc_nzi64(d));
        print_bool(dec_nzi64_u64(v as u64) == d);
    }
    for v in [0i128, -5, i128::MIN, 1, i128::MAX] {
        let d = dec_nzi128(v);
        print_bool(d == NonZero::new(v));
        print_bool(enc_nzi128(d) == v);
    }
}
