//! Scalar-layout constants wrapping transparent newtypes, single-field structs,
//! tuples, single-element arrays, single-variant enums and nested niche enums
//! (MIRES-006).
//!
//! Every constant here fits in a single scalar register in rustc's layout.
//! Previously the JIR link-time scalar decoder failed to peel transparent and
//! pattern types inside recursive field decodes (silently producing `{}`),
//! lacked array and single-variant enum arms, and miscomputed niche bit widths
//! when a niche path crossed a transparent wrapper.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::num::NonZero;
use core::ptr::NonNull;

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
struct MyNz(NonZero<u32>);

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
struct ZstBefore(PhantomData<u8>, NonZero<u32>);

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
struct TransparentBool(bool);

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
struct TransparentOrd(Ordering);

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
struct TransparentChar(char);

#[repr(packed)]
#[derive(Copy, Clone)]
struct PackedNz(NonZero<u32>);

#[derive(Copy, Clone, PartialEq)]
struct WrapNz(NonZero<u32>);

#[derive(Copy, Clone, PartialEq)]
struct WrapWithZst((), NonZero<u32>);

#[derive(Copy, Clone)]
enum Only {
    V(NonZero<u32>),
}

#[derive(Copy, Clone, PartialEq)]
enum MultiOrd {
    A,
    B,
    C(TransparentOrd),
}

#[derive(Copy, Clone, PartialEq)]
enum MultiBool {
    A,
    B,
    C(TransparentBool),
    D,
}

const NZ5: NonZero<u32> = unsafe { NonZero::new_unchecked(5) };
const NZ9: NonZero<u32> = unsafe { NonZero::new_unchecked(9) };

const OPT_MY_NZ: Option<MyNz> = Some(MyNz(NZ5));
const OPT_MY_NZ_NONE: Option<MyNz> = None;
const OPT_ZST_BEFORE: Option<ZstBefore> = Some(ZstBefore(PhantomData, NZ9));
const OPT_ZST_BEFORE_NONE: Option<ZstBefore> = None;

const OPT_PACKED: Option<PackedNz> = Some(PackedNz(NZ5));
const OPT_PACKED_NONE: Option<PackedNz> = None;

const BARE_WRAP: WrapNz = WrapNz(NZ5);
const OPT_WRAP: Option<WrapNz> = Some(WrapNz(NZ9));
const OPT_WRAP_NONE: Option<WrapNz> = None;
const OPT_WRAP_ZST: Option<WrapWithZst> = Some(WrapWithZst((), NZ5));

const TUP_NZ: (NonZero<u32>,) = (NZ5,);
const TUP_ZST_NZ: ((), NonZero<u32>) = ((), NZ9);
const OPT_TUP_NZ: Option<(NonZero<u32>,)> = Some((NZ5,));
const OPT_TUP_NONE: Option<(NonZero<u32>,)> = None;

const ARR1_NZ: [NonZero<u32>; 1] = [NZ5];
const OPT_ARR1_NZ: Option<[NonZero<u32>; 1]> = Some([NZ9]);
const OPT_ARR1_NONE: Option<[NonZero<u32>; 1]> = None;

const ONLY_NZ: Only = Only::V(NZ5);

const OPT_BOOL_F: Option<bool> = Some(false);
const OPT_BOOL_T: Option<bool> = Some(true);
const OPT_BOOL_N: Option<bool> = None;

const OPT_CHAR_A: Option<char> = Some('A');
const OPT_CHAR_N: Option<char> = None;

const OPT_ORD_L: Option<Ordering> = Some(Ordering::Less);
const OPT_ORD_E: Option<Ordering> = Some(Ordering::Equal);
const OPT_ORD_G: Option<Ordering> = Some(Ordering::Greater);
const OPT_ORD_N: Option<Ordering> = None;

const OPT_TBOOL_F: Option<TransparentBool> = Some(TransparentBool(false));
const OPT_TBOOL_T: Option<TransparentBool> = Some(TransparentBool(true));
const OPT_TBOOL_N: Option<TransparentBool> = None;

const OPT_TORD_L: Option<TransparentOrd> = Some(TransparentOrd(Ordering::Less));
const OPT_TORD_E: Option<TransparentOrd> = Some(TransparentOrd(Ordering::Equal));
const OPT_TORD_G: Option<TransparentOrd> = Some(TransparentOrd(Ordering::Greater));
const OPT_TORD_N: Option<TransparentOrd> = None;

const OPT_TCHAR_Z: Option<TransparentChar> = Some(TransparentChar('Z'));
const OPT_TCHAR_N: Option<TransparentChar> = None;

const NESTED_BOOL_NONE: Option<Option<bool>> = None;
const NESTED_BOOL_SOME_NONE: Option<Option<bool>> = Some(None);
const NESTED_BOOL_F: Option<Option<bool>> = Some(Some(false));
const NESTED_BOOL_T: Option<Option<bool>> = Some(Some(true));

const NESTED_TBOOL_NONE: Option<Option<TransparentBool>> = None;
const NESTED_TBOOL_SOME_NONE: Option<Option<TransparentBool>> = Some(None);
const NESTED_TBOOL_F: Option<Option<TransparentBool>> = Some(Some(TransparentBool(false)));
const NESTED_TBOOL_T: Option<Option<TransparentBool>> = Some(Some(TransparentBool(true)));

const OPT_MD_NZ: Option<ManuallyDrop<NonZero<u32>>> = Some(ManuallyDrop::new(NZ5));
const OPT_MD_NONE: Option<ManuallyDrop<NonZero<u32>>> = None;

const OPT_DANGLING: Option<NonNull<u32>> = Some(NonNull::dangling());
const OPT_DANGLING_NONE: Option<NonNull<u32>> = None;

const MULTI_ORD_A: MultiOrd = MultiOrd::A;
const MULTI_ORD_B: MultiOrd = MultiOrd::B;
const MULTI_ORD_L: MultiOrd = MultiOrd::C(TransparentOrd(Ordering::Less));
const MULTI_ORD_E: MultiOrd = MultiOrd::C(TransparentOrd(Ordering::Equal));
const MULTI_ORD_G: MultiOrd = MultiOrd::C(TransparentOrd(Ordering::Greater));

const MULTI_BOOL_A: MultiBool = MultiBool::A;
const MULTI_BOOL_B: MultiBool = MultiBool::B;
const MULTI_BOOL_F: MultiBool = MultiBool::C(TransparentBool(false));
const MULTI_BOOL_T: MultiBool = MultiBool::C(TransparentBool(true));
const MULTI_BOOL_D: MultiBool = MultiBool::D;

#[inline(never)]
fn get_my_nz(v: Option<MyNz>) -> u32 {
    match v {
        Some(x) => x.0.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_zst_before(v: Option<ZstBefore>) -> u32 {
    match v {
        Some(x) => x.1.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_packed(v: Option<PackedNz>) -> u32 {
    match v {
        Some(x) => {
            let n = x.0;
            n.get()
        }
        None => 0,
    }
}

#[inline(never)]
fn get_bare_wrap(v: WrapNz) -> u32 {
    v.0.get()
}

#[inline(never)]
fn get_opt_wrap(v: Option<WrapNz>) -> u32 {
    match v {
        Some(x) => x.0.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_opt_wrap_zst(v: Option<WrapWithZst>) -> u32 {
    match v {
        Some(x) => x.1.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_tup(v: (NonZero<u32>,)) -> u32 {
    v.0.get()
}

#[inline(never)]
fn get_tup_zst(v: ((), NonZero<u32>)) -> u32 {
    v.1.get()
}

#[inline(never)]
fn get_opt_tup(v: Option<(NonZero<u32>,)>) -> u32 {
    match v {
        Some((n,)) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_arr1(v: [NonZero<u32>; 1]) -> u32 {
    v[0].get()
}

#[inline(never)]
fn get_opt_arr1(v: Option<[NonZero<u32>; 1]>) -> u32 {
    match v {
        Some([n]) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_only(v: Only) -> u32 {
    match v {
        Only::V(n) => n.get(),
    }
}

#[inline(never)]
fn get_opt_bool(v: Option<bool>) -> i32 {
    match v {
        None => -1,
        Some(false) => 0,
        Some(true) => 1,
    }
}

#[inline(never)]
fn get_opt_char(v: Option<char>) -> u32 {
    match v {
        Some(c) => c as u32,
        None => 0,
    }
}

#[inline(never)]
fn get_opt_ord(v: Option<Ordering>) -> i32 {
    match v {
        None => -9,
        Some(o) => o as i32,
    }
}

#[inline(never)]
fn get_opt_tbool(v: Option<TransparentBool>) -> i32 {
    match v {
        None => -1,
        Some(TransparentBool(false)) => 0,
        Some(TransparentBool(true)) => 1,
    }
}

#[inline(never)]
fn get_opt_tord(v: Option<TransparentOrd>) -> i32 {
    match v {
        None => -9,
        Some(TransparentOrd(o)) => o as i32,
    }
}

#[inline(never)]
fn get_opt_tchar(v: Option<TransparentChar>) -> u32 {
    match v {
        Some(TransparentChar(c)) => c as u32,
        None => 0,
    }
}

#[inline(never)]
fn get_nested_bool(v: Option<Option<bool>>) -> i32 {
    match v {
        None => -2,
        Some(None) => -1,
        Some(Some(false)) => 0,
        Some(Some(true)) => 1,
    }
}

#[inline(never)]
fn get_nested_tbool(v: Option<Option<TransparentBool>>) -> i32 {
    match v {
        None => -2,
        Some(None) => -1,
        Some(Some(TransparentBool(false))) => 0,
        Some(Some(TransparentBool(true))) => 1,
    }
}

#[inline(never)]
fn get_opt_md(v: Option<ManuallyDrop<NonZero<u32>>>) -> u32 {
    match v {
        Some(md) => md.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_opt_dangling(v: Option<NonNull<u32>>) -> usize {
    match v {
        Some(p) => p.as_ptr() as usize,
        None => 0,
    }
}

#[inline(never)]
fn get_multi_ord(v: MultiOrd) -> i32 {
    match v {
        MultiOrd::A => 10,
        MultiOrd::B => 20,
        MultiOrd::C(TransparentOrd(o)) => o as i32,
    }
}

#[inline(never)]
fn get_multi_bool(v: MultiBool) -> i32 {
    match v {
        MultiBool::A => 10,
        MultiBool::B => 20,
        MultiBool::C(TransparentBool(false)) => 0,
        MultiBool::C(TransparentBool(true)) => 1,
        MultiBool::D => 30,
    }
}

#[inline(never)]
fn test_arr1_and_only() {
    print_u32(get_arr1(ARR1_NZ));
    print_u32(get_opt_arr1(OPT_ARR1_NZ));
    print_u32(get_opt_arr1(OPT_ARR1_NONE));
    print_u32(get_only(ONLY_NZ));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("transparent-and-packed");
    print_u32(get_my_nz(OPT_MY_NZ));
    print_u32(get_my_nz(OPT_MY_NZ_NONE));
    print_u32(get_zst_before(OPT_ZST_BEFORE));
    print_u32(get_zst_before(OPT_ZST_BEFORE_NONE));
    print_u32(get_packed(OPT_PACKED));
    print_u32(get_packed(OPT_PACKED_NONE));

    print_str("wrap-tup-arr-only");
    print_u32(get_bare_wrap(BARE_WRAP));
    print_u32(get_opt_wrap(OPT_WRAP));
    print_u32(get_opt_wrap(OPT_WRAP_NONE));
    print_u32(get_opt_wrap_zst(OPT_WRAP_ZST));
    print_u32(get_tup(TUP_NZ));
    print_u32(get_tup_zst(TUP_ZST_NZ));
    print_u32(get_opt_tup(OPT_TUP_NZ));
    print_u32(get_opt_tup(OPT_TUP_NONE));
    test_arr1_and_only();

    print_str("bool-char-ord");
    print_i32(get_opt_bool(OPT_BOOL_F));
    print_i32(get_opt_bool(OPT_BOOL_T));
    print_i32(get_opt_bool(OPT_BOOL_N));
    print_u32(get_opt_char(OPT_CHAR_A));
    print_u32(get_opt_char(OPT_CHAR_N));
    print_i32(get_opt_ord(OPT_ORD_L));
    print_i32(get_opt_ord(OPT_ORD_E));
    print_i32(get_opt_ord(OPT_ORD_G));
    print_i32(get_opt_ord(OPT_ORD_N));

    print_str("transparent-bool-char-ord");
    print_i32(get_opt_tbool(OPT_TBOOL_F));
    print_i32(get_opt_tbool(OPT_TBOOL_T));
    print_i32(get_opt_tbool(OPT_TBOOL_N));
    print_i32(get_opt_tord(OPT_TORD_L));
    print_i32(get_opt_tord(OPT_TORD_E));
    print_i32(get_opt_tord(OPT_TORD_G));
    print_i32(get_opt_tord(OPT_TORD_N));
    print_u32(get_opt_tchar(OPT_TCHAR_Z));
    print_u32(get_opt_tchar(OPT_TCHAR_N));

    print_str("nested-and-md-and-nonnull");
    print_i32(get_nested_bool(NESTED_BOOL_NONE));
    print_i32(get_nested_bool(NESTED_BOOL_SOME_NONE));
    print_i32(get_nested_bool(NESTED_BOOL_F));
    print_i32(get_nested_bool(NESTED_BOOL_T));
    print_i32(get_nested_tbool(NESTED_TBOOL_NONE));
    print_i32(get_nested_tbool(NESTED_TBOOL_SOME_NONE));
    print_i32(get_nested_tbool(NESTED_TBOOL_F));
    print_i32(get_nested_tbool(NESTED_TBOOL_T));
    print_u32(get_opt_md(OPT_MD_NZ));
    print_u32(get_opt_md(OPT_MD_NONE));
    print_usize(get_opt_dangling(OPT_DANGLING));
    print_usize(get_opt_dangling(OPT_DANGLING_NONE));

    print_str("multi-niche");
    print_i32(get_multi_ord(MULTI_ORD_A));
    print_i32(get_multi_ord(MULTI_ORD_B));
    print_i32(get_multi_ord(MULTI_ORD_L));
    print_i32(get_multi_ord(MULTI_ORD_E));
    print_i32(get_multi_ord(MULTI_ORD_G));
    print_i32(get_multi_bool(MULTI_BOOL_A));
    print_i32(get_multi_bool(MULTI_BOOL_B));
    print_i32(get_multi_bool(MULTI_BOOL_F));
    print_i32(get_multi_bool(MULTI_BOOL_T));
    print_i32(get_multi_bool(MULTI_BOOL_D));

    print_str("eq");
    print_bool(OPT_MY_NZ == black_box(Some(MyNz(NZ5))));
    print_bool(OPT_WRAP == black_box(Some(WrapNz(NZ9))));
    print_bool(NESTED_TBOOL_T == black_box(Some(Some(TransparentBool(true)))));
    print_bool(MULTI_ORD_L == black_box(MultiOrd::C(TransparentOrd(Ordering::Less))));
}
