//! Provenance-free wide slice constants over zero-sized elements (`*const [Z]`, `*mut [Z]`,
//! `&[Z]`, `NonNull<[Z]>`, transparent wrappers, `Option`, `const`, `static`, promoted `&const`,
//! and aggregates holding them).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr::{self, NonNull};

struct Z;

#[repr(align(8))]
struct Align8;

#[repr(align(16))]
struct Align16;

#[repr(transparent)]
struct ZSlice([Z]);

struct ZHolder {
    raw: *const [Z],
    aligned: &'static [Align8],
    opt: Option<NonNull<[Align16]>>,
}

unsafe impl Sync for ZHolder {}

const Z_SLICE_3: *const [Z] = ptr::slice_from_raw_parts(NonNull::<Z>::dangling().as_ptr(), 3);
const Z_SLICE_0: *const [Z] = ptr::slice_from_raw_parts(NonNull::<Z>::dangling().as_ptr(), 0);
const Z_SLICE_MUT: *mut [Z] = ptr::slice_from_raw_parts_mut(NonNull::<Z>::dangling().as_ptr(), 5);
const Z_CUSTOM_100: *const [Z] = ptr::slice_from_raw_parts(0x1000 as *const Z, 100);
const Z_NULL_0: *const [Z] = ptr::slice_from_raw_parts(ptr::null::<Z>(), 0);
const Z_REF_4: &[Z] = unsafe { &*ptr::slice_from_raw_parts(NonNull::<Z>::dangling().as_ptr(), 4) };
const A8_REF_7: &[Align8] =
    unsafe { &*ptr::slice_from_raw_parts(NonNull::<Align8>::dangling().as_ptr(), 7) };
const A16_NN_2: NonNull<[Align16]> =
    NonNull::slice_from_raw_parts(NonNull::<Align16>::dangling(), 2);
const UNIT_SLICE_9: *const [()] = ptr::slice_from_raw_parts(NonNull::<()>::dangling().as_ptr(), 9);
const ZERO_ARR_SLICE_6: *const [[u32; 0]] =
    ptr::slice_from_raw_parts(NonNull::<[u32; 0]>::dangling().as_ptr(), 6);
const Z_WRAP_3: *const ZSlice =
    ptr::slice_from_raw_parts(NonNull::<Z>::dangling().as_ptr(), 3) as *const ZSlice;

const OPT_REF_SOME: Option<&[Z]> = Some(Z_REF_4);
const OPT_REF_NONE: Option<&[Z]> = None;
const OPT_NN_SOME: Option<NonNull<[Align16]>> = Some(A16_NN_2);
const OPT_NN_NONE: Option<NonNull<[Align16]>> = None;
const OPT_RAW_SOME: Option<*const [Z]> = Some(Z_SLICE_3);
const OPT_RAW_NULL: Option<*const [Z]> = Some(Z_NULL_0);
const OPT_RAW_NONE: Option<*const [Z]> = None;

const HOLDER: ZHolder = ZHolder {
    raw: Z_SLICE_3,
    aligned: A8_REF_7,
    opt: OPT_NN_SOME,
};
const PAIR: (*const [Z], NonNull<[Align16]>) = (Z_CUSTOM_100, A16_NN_2);
const TABLE: [*const [Z]; 3] = [Z_SLICE_0, Z_SLICE_3, Z_CUSTOM_100];
const PROMOTED_RAW_REF: &*const [Z] = &Z_SLICE_3;

struct SyncPtr<T: ?Sized>(*const T);
unsafe impl<T: ?Sized> Sync for SyncPtr<T> {}

static S_Z_SLICE: SyncPtr<[Z]> = SyncPtr(Z_SLICE_3);
static S_A8_REF: &[Align8] = A8_REF_7;
static S_OPT_REF: Option<&[Z]> = Some(Z_REF_4);
static S_HOLDER: ZHolder = HOLDER;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Basic ZST raw slice consts
    let s3: &[Z] = unsafe { &*black_box(Z_SLICE_3) };
    print_usize(s3.len());
    print_usize(s3.as_ptr() as usize);
    print_bool(ptr::eq(s3.as_ptr(), NonNull::<Z>::dangling().as_ptr()));
    print_bool(ptr::addr_eq(
        s3 as *const [Z],
        NonNull::<Z>::dangling().as_ptr(),
    ));
    print_bool(ptr::eq(
        &s3[black_box(2)],
        NonNull::<Z>::dangling().as_ptr(),
    ));
    let sub = &s3[black_box(1)..black_box(3)];
    print_usize(sub.len());
    print_bool(ptr::eq(sub.as_ptr(), NonNull::<Z>::dangling().as_ptr()));
    print_usize(s3.iter().count());

    let s0: &[Z] = unsafe { &*black_box(Z_SLICE_0) };
    print_usize(s0.len());
    print_usize(s0.iter().count());

    let sm = black_box(Z_SLICE_MUT);
    print_usize(sm.len());
    print_usize(sm.cast::<Z>() as usize);

    // Custom address & null ZST raw slices
    let sc = black_box(Z_CUSTOM_100);
    print_usize(sc.len());
    print_usize(sc.cast::<Z>() as usize);
    print_bool(ptr::eq(sc.cast::<Z>(), 0x1000 as *const Z));
    let sc_ref: &[Z] = unsafe { &*sc };
    print_usize(sc_ref.iter().count());
    print_bool(ptr::eq(&sc_ref[black_box(99)], 0x1000 as *const Z));

    let sn = black_box(Z_NULL_0);
    print_usize(sn.len());
    print_usize(sn.cast::<Z>() as usize);
    print_bool(sn.cast::<Z>().is_null());

    // Aligned ZSTs, (), [u32; 0], transparent wrapper
    let a8 = black_box(A8_REF_7);
    print_usize(a8.len());
    print_usize(a8.as_ptr() as usize);
    print_bool(a8.as_ptr().is_aligned());
    print_bool(ptr::eq(
        &a8[black_box(6)],
        NonNull::<Align8>::dangling().as_ptr(),
    ));

    let a16 = black_box(A16_NN_2);
    print_usize(a16.len());
    print_usize(a16.as_ptr().cast::<Align16>() as usize);
    print_bool(a16.as_ptr().cast::<Align16>().is_aligned());

    let su = black_box(UNIT_SLICE_9);
    print_usize(su.len());
    print_usize(su.cast::<()>() as usize);

    let sza = black_box(ZERO_ARR_SLICE_6);
    print_usize(sza.len());
    print_usize(sza.cast::<[u32; 0]>() as usize);
    print_bool(sza.cast::<[u32; 0]>().is_aligned());

    let sw: &ZSlice = unsafe { &*black_box(Z_WRAP_3) };
    print_usize(sw.0.len());
    print_usize(sw.0.as_ptr() as usize);

    // Options (niche and direct tag)
    print_usize(black_box(OPT_REF_SOME).map_or(999, |s| s.len()));
    print_bool(black_box(OPT_REF_NONE).is_none());
    print_usize(black_box(OPT_NN_SOME).map_or(999, |p| p.len()));
    print_bool(black_box(OPT_NN_NONE).is_none());
    print_usize(black_box(OPT_RAW_SOME).map_or(999, |p| p.len()));
    print_bool(black_box(OPT_RAW_NULL).is_some_and(|p| p.cast::<Z>().is_null()));
    print_bool(black_box(OPT_RAW_NONE).is_none());

    // Aggregates, promoted &const, statics
    let h = black_box(HOLDER);
    print_usize(h.raw.len());
    print_usize(h.aligned.len());
    print_usize(h.opt.unwrap().len());

    let (p0, p1) = black_box(PAIR);
    print_usize(p0.len());
    print_usize(p1.len());

    let tbl = black_box(TABLE);
    print_usize(tbl[0].len() + tbl[1].len() + tbl[2].len());

    let pr = *black_box(PROMOTED_RAW_REF);
    print_usize(pr.len());
    print_bool(ptr::eq(pr, Z_SLICE_3));

    print_usize(black_box(S_Z_SLICE.0).len());
    print_usize(black_box(S_A8_REF).len());
    print_usize(black_box(S_OPT_REF).unwrap().len());
    print_usize(black_box(S_HOLDER.raw).len());
}
