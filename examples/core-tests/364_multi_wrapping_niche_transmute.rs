//! Multi-variant niche enums, wrapping niche ranges (across both signed and unsigned boundaries),
//! mid-positioned untagged variants, uninhabited niche-range variants, and transparent wrappers.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::convert::Infallible;
use core::mem::transmute;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Three {
    A(bool),
    B,
    C,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MidBool {
    A,
    B(bool),
    C,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MidDead {
    A,
    Dead(Infallible),
    B(bool),
    C,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeadDead {
    Dead(Infallible),
    A,
    B(bool),
    C,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i8)]
enum SignedCrossI8 {
    A = -127,
    B = 0,
    C = 125,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MultiSignedCross {
    Payload(SignedCrossI8),
    N0,
    N1,
    N2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i128)]
enum HiI128 {
    A = -3,
    B = -2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WrapHiI128 {
    P(HiI128),
    N0,
    N1,
    N2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MultiOrd {
    N0,
    Payload(Ordering),
    N1,
    N2,
}

macro_rules! mk_almost_full {
    ($name:ident; $($v:ident = $d:expr),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        #[allow(dead_code)]
        enum $name { $($v = $d),* }
    };
}

// Step-3 discriminants 1..=253: max interior gap is 2, wrapping gap {254, 255, 0} has size 3!
mk_almost_full!(AlmostFullU8;
    V00=1, V01=4, V02=7, V03=10, V04=13, V05=16, V06=19, V07=22, V08=25, V09=28,
    V10=31, V11=34, V12=37, V13=40, V14=43, V15=46, V16=49, V17=52, V18=55, V19=58,
    V20=61, V21=64, V22=67, V23=70, V24=73, V25=76, V26=79, V27=82, V28=85, V29=88,
    V30=91, V31=94, V32=97, V33=100, V34=103, V35=106, V36=109, V37=112, V38=115, V39=118,
    V40=121, V41=124, V42=127, V43=130, V44=133, V45=136, V46=139, V47=142, V48=145, V49=148,
    V50=151, V51=154, V52=157, V53=160, V54=163, V55=166, V56=169, V57=172, V58=175, V59=178,
    V60=181, V61=184, V62=187, V63=190, V64=193, V65=196, V66=199, V67=202, V68=205, V69=208,
    V70=211, V71=214, V72=217, V73=220, V74=223, V75=226, V76=229, V77=232, V78=235, V79=238,
    V80=241, V81=244, V82=247, V83=250, V84=253
);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct WrapAlmostFull(AlmostFullU8);

// Genuine unsigned-wrapping niche range: N0 = 254 (-2), N1 = 255 (-1), N2 = 0!
#[derive(Clone, Copy, PartialEq, Eq)]
enum TrueWrapU8 {
    P(AlmostFullU8),
    N0,
    N1,
    N2,
}

// Same wrapping niche range behind a `#[repr(transparent)]` wrapper (tests `niche_width` walking
// unpeeled field chains `[0, 0]` for both runtime transmutes and niche-variant const decode).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TrueWrapTrans {
    P(WrapAlmostFull),
    N0,
    N1,
    N2,
}

const CONST_TRUE_WRAP_N2: TrueWrapU8 = TrueWrapU8::N2;
const CONST_TRUE_WRAP_N0: TrueWrapU8 = TrueWrapU8::N0;
const CONST_TRUE_WRAP_P: TrueWrapU8 = TrueWrapU8::P(AlmostFullU8::V84);
const CONST_TRANS_WRAP_N2: TrueWrapTrans = TrueWrapTrans::N2;
const CONST_TRANS_WRAP_N0: TrueWrapTrans = TrueWrapTrans::N0;

#[inline(never)]
fn three_enc(v: Three) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn three_dec(b: u8) -> Three {
    unsafe { transmute(b) }
}

#[inline(never)]
fn mid_enc(v: MidBool) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn mid_dec(b: u8) -> MidBool {
    unsafe { transmute(b) }
}

#[inline(never)]
fn mid_dead_enc(v: MidDead) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn mid_dead_dec(b: u8) -> MidDead {
    unsafe { transmute(b) }
}

#[inline(never)]
fn lead_dead_enc(v: LeadDead) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn lead_dead_dec(b: u8) -> LeadDead {
    unsafe { transmute(b) }
}

#[inline(never)]
fn cross_enc_i8(v: MultiSignedCross) -> i8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn cross_dec_i8(b: i8) -> MultiSignedCross {
    unsafe { transmute(b) }
}
#[inline(never)]
fn cross_enc_u8(v: MultiSignedCross) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn cross_dec_u8(b: u8) -> MultiSignedCross {
    unsafe { transmute(b) }
}

#[inline(never)]
fn w128_enc_i128(v: WrapHiI128) -> i128 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn w128_dec_i128(b: i128) -> WrapHiI128 {
    unsafe { transmute(b) }
}
#[inline(never)]
fn w128_enc_u128(v: WrapHiI128) -> u128 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn w128_dec_u128(b: u128) -> WrapHiI128 {
    unsafe { transmute(b) }
}

#[inline(never)]
fn mord_enc_i8(v: MultiOrd) -> i8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn mord_dec_i8(b: i8) -> MultiOrd {
    unsafe { transmute(b) }
}

#[inline(never)]
fn twrap_enc_u8(v: TrueWrapU8) -> u8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn twrap_dec_u8(b: u8) -> TrueWrapU8 {
    unsafe { transmute(b) }
}
#[inline(never)]
fn twrap_enc_i8(v: TrueWrapU8) -> i8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn twrap_dec_i8(b: i8) -> TrueWrapU8 {
    unsafe { transmute(b) }
}
#[inline(never)]
fn ttrans_enc_i8(v: TrueWrapTrans) -> i8 {
    unsafe { transmute(v) }
}
#[inline(never)]
fn ttrans_dec_i8(b: i8) -> TrueWrapTrans {
    unsafe { transmute(b) }
}
#[inline(never)]
fn mk_wrap_payload(v: AlmostFullU8) -> TrueWrapTrans {
    TrueWrapTrans::P(WrapAlmostFull(v))
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for v in [Three::A(false), Three::A(true), Three::B, Three::C] {
        let b = three_enc(v);
        print_u32(b as u32);
        print_bool(three_dec(b) == v);
    }

    for v in [MidBool::A, MidBool::B(false), MidBool::B(true), MidBool::C] {
        let b = mid_enc(v);
        print_u32(b as u32);
        print_bool(mid_dec(b) == v);
    }

    for v in [MidDead::A, MidDead::B(false), MidDead::B(true), MidDead::C] {
        let b = mid_dead_enc(v);
        print_u32(b as u32);
        print_bool(mid_dead_dec(b) == v);
    }

    for v in [
        LeadDead::A,
        LeadDead::B(false),
        LeadDead::B(true),
        LeadDead::C,
    ] {
        let b = lead_dead_enc(v);
        print_u32(b as u32);
        print_bool(lead_dead_dec(b) == v);
    }

    // Signed boundary crossing: N0 = 126, N1 = 127, N2 = -128 (u8: 126, 127, 128)
    for v in [
        MultiSignedCross::Payload(SignedCrossI8::A),
        MultiSignedCross::Payload(SignedCrossI8::B),
        MultiSignedCross::Payload(SignedCrossI8::C),
        MultiSignedCross::N0,
        MultiSignedCross::N1,
        MultiSignedCross::N2,
    ] {
        let s = cross_enc_i8(v);
        print_i32(s as i32);
        print_bool(cross_dec_i8(s) == v);
        let u = cross_enc_u8(v);
        print_u32(u as u32);
        print_bool(cross_dec_u8(u) == v);
    }

    // 128-bit multi-niche with high bit set (-6, -5, -4 / u128::MAX-5..=u128::MAX-3)
    for v in [
        WrapHiI128::P(HiI128::A),
        WrapHiI128::P(HiI128::B),
        WrapHiI128::N0,
        WrapHiI128::N1,
        WrapHiI128::N2,
    ] {
        let s = w128_enc_i128(v);
        print_i32(s as i32);
        print_bool(w128_dec_i128(s) == v);
        let u = w128_enc_u128(v);
        print_bool(u == (s as u128));
        print_bool(w128_dec_u128(u) == v);
    }

    // MultiOrd: untagged Ordering in middle
    for v in [
        MultiOrd::N0,
        MultiOrd::Payload(Ordering::Less),
        MultiOrd::Payload(Ordering::Equal),
        MultiOrd::Payload(Ordering::Greater),
        MultiOrd::N1,
        MultiOrd::N2,
    ] {
        let s = mord_enc_i8(v);
        print_i32(s as i32);
        print_bool(mord_dec_i8(s) == v);
    }

    // True unsigned-wrapping niche range: 254, 255, 0 (-2, -1, 0)
    for v in [
        TrueWrapU8::P(AlmostFullU8::V00),
        TrueWrapU8::P(AlmostFullU8::V84),
        TrueWrapU8::N0,
        TrueWrapU8::N1,
        TrueWrapU8::N2,
    ] {
        let u = twrap_enc_u8(v);
        print_u32(u as u32);
        print_bool(twrap_dec_u8(u) == v);
        let s = twrap_enc_i8(v);
        print_i32(s as i32);
        print_bool(twrap_dec_i8(s) == v);
    }

    // Same wrapping niche range through a transparent wrapper (tests niche_width!)
    for v in [
        mk_wrap_payload(AlmostFullU8::V00),
        mk_wrap_payload(AlmostFullU8::V84),
        TrueWrapTrans::N0,
        TrueWrapTrans::N1,
        TrueWrapTrans::N2,
    ] {
        let s = ttrans_enc_i8(v);
        print_i32(s as i32);
        print_bool(ttrans_dec_i8(s) == v);
    }

    // Const decoding of wrapping niches (direct and through transparent wrapper)
    print_bool(CONST_TRUE_WRAP_N2 == TrueWrapU8::N2);
    print_bool(CONST_TRUE_WRAP_N0 == TrueWrapU8::N0);
    print_bool(CONST_TRUE_WRAP_P == TrueWrapU8::P(AlmostFullU8::V84));
    print_bool(CONST_TRANS_WRAP_N2 == TrueWrapTrans::N2);
    print_bool(CONST_TRANS_WRAP_N0 == TrueWrapTrans::N0);
}
