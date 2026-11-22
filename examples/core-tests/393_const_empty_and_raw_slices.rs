//! Provenance-free wide slice constants over non-ZST elements: empty dangling slices (`meta == 0`),
//! custom-address and null raw slices (`meta == 0` and `meta > 0`), `Option`, `const`, `static`,
//! promoted `&const`, and aggregates holding them.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr::{self, NonNull};

#[repr(transparent)]
struct U32Slice([u32]);

#[repr(C)]
struct Pair(u32, u32);

struct SliceHolder {
    empty_u32: *const [u32],
    raw_5: *const [u32],
    opt: Option<NonNull<[u32]>>,
}

unsafe impl Sync for SliceHolder {}

const EMPTY_U32_RAW: *const [u32] =
    ptr::slice_from_raw_parts(NonNull::<u32>::dangling().as_ptr(), 0);
const EMPTY_U32_MUT: *mut [u32] =
    ptr::slice_from_raw_parts_mut(NonNull::<u32>::dangling().as_ptr(), 0);
const EMPTY_U32_REF: &[u32] =
    unsafe { &*ptr::slice_from_raw_parts(NonNull::<u32>::dangling().as_ptr(), 0) };
const EMPTY_U32_NN: NonNull<[u32]> = NonNull::slice_from_raw_parts(NonNull::<u32>::dangling(), 0);
const EMPTY_U8_RAW: *const [u8] = ptr::slice_from_raw_parts(NonNull::<u8>::dangling().as_ptr(), 0);
const EMPTY_U64_RAW: *const [u64] =
    ptr::slice_from_raw_parts(NonNull::<u64>::dangling().as_ptr(), 0);
const EMPTY_CHUNK_RAW: *const [[u8; 4]] =
    ptr::slice_from_raw_parts(NonNull::<[u8; 4]>::dangling().as_ptr(), 0);
const EMPTY_WRAP: *const U32Slice = EMPTY_U32_RAW as *const U32Slice;

const CUSTOM_U32_0: *const [u32] = ptr::slice_from_raw_parts(0x1000 as *const u32, 0);
const CUSTOM_U32_5: *const [u32] = ptr::slice_from_raw_parts(0x1000 as *const u32, 5);
const CUSTOM_U32_5_MUT: *mut [u32] = ptr::slice_from_raw_parts_mut(0x1000 as *mut u32, 5);
const CUSTOM_UNALIGNED: *const [u32] = ptr::slice_from_raw_parts(0x1002 as *const u32, 3);
const CUSTOM_U8_7: *const [u8] = ptr::slice_from_raw_parts(0x2000 as *const u8, 7);
const CUSTOM_U8_4096: *const [u8] = ptr::slice_from_raw_parts(0x1000 as *const u8, 0);
const CUSTOM_U16_4096: *const [u16] = ptr::slice_from_raw_parts(0x1000 as *const u16, 0);
const CUSTOM_PAIR_5: *const [Pair] = ptr::slice_from_raw_parts(0x1000 as *const Pair, 5);
const NULL_PAIR_0: *const [Pair] = ptr::slice_from_raw_parts(ptr::null::<Pair>(), 0);
const DANGLING_PAIR_0: *const [Pair] =
    ptr::slice_from_raw_parts(NonNull::<Pair>::dangling().as_ptr(), 0);
const HUGE_U8_0: *const [u8] = ptr::slice_from_raw_parts(0xffff_fff0usize as *const u8, 0);
const NULL_U32_0: *const [u32] = ptr::slice_from_raw_parts(ptr::null::<u32>(), 0);
const NULL_U32_5: *const [u32] = ptr::slice_from_raw_parts(ptr::null::<u32>(), 5);

const OPT_REF_SOME: Option<&[u32]> = Some(EMPTY_U32_REF);
const OPT_REF_NONE: Option<&[u32]> = None;
const OPT_NN_SOME: Option<NonNull<[u32]>> = Some(EMPTY_U32_NN);
const OPT_NN_NONE: Option<NonNull<[u32]>> = None;
const OPT_RAW_SOME: Option<*const [u32]> = Some(CUSTOM_U32_5);
const OPT_RAW_NULL: Option<*const [u32]> = Some(NULL_U32_0);
const OPT_RAW_NONE: Option<*const [u32]> = None;

const HOLDER: SliceHolder = SliceHolder {
    empty_u32: EMPTY_U32_RAW,
    raw_5: CUSTOM_U32_5,
    opt: OPT_NN_SOME,
};
const PAIR: (*const [u32], *const [u8]) = (CUSTOM_U32_0, CUSTOM_U8_7);
const TABLE: [*const [u32]; 3] = [EMPTY_U32_RAW, CUSTOM_U32_0, CUSTOM_U32_5];
const PROMOTED_RAW_REF: &*const [u32] = &CUSTOM_U32_5;

struct SyncVal<T>(T);
unsafe impl<T> Sync for SyncVal<T> {}

static S_EMPTY_U32: SyncVal<*const [u32]> = SyncVal(EMPTY_U32_RAW);
static S_CUSTOM_5: SyncVal<*const [u32]> = SyncVal(CUSTOM_U32_5);
static S_EMPTY_REF: &[u32] = EMPTY_U32_REF;
static S_OPT_NN: SyncVal<Option<NonNull<[u32]>>> = SyncVal(OPT_NN_SOME);
static S_HOLDER: SliceHolder = HOLDER;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Dangling empty non-ZST slices
    let e: &[u32] = unsafe { &*black_box(EMPTY_U32_RAW) };
    print_usize(e.len());
    print_usize(e.as_ptr() as usize);
    print_bool(e.as_ptr().is_aligned());
    print_bool(ptr::eq(e.as_ptr(), NonNull::<u32>::dangling().as_ptr()));
    print_bool(ptr::addr_eq(
        e as *const [u32],
        NonNull::<u32>::dangling().as_ptr(),
    ));
    let mut count = 0usize;
    for _ in e {
        count += 1;
    }
    print_usize(count);
    let (l, r) = e.split_at(0);
    print_usize(l.len() + r.len());
    print_bool(e.first().is_none());
    print_usize(e[0..0].len());

    let em = black_box(EMPTY_U32_MUT);
    print_usize(em.len());
    print_usize(em.cast::<u32>() as usize);
    print_bool(ptr::eq(
        em.cast::<u32>(),
        NonNull::<u32>::dangling().as_ptr(),
    ));

    let eu8: &[u8] = unsafe { &*black_box(EMPTY_U8_RAW) };
    print_usize(eu8.len());
    print_usize(eu8.as_ptr() as usize);
    print_bool(ptr::eq(eu8.as_ptr(), NonNull::<u8>::dangling().as_ptr()));

    let eu64: &[u64] = unsafe { &*black_box(EMPTY_U64_RAW) };
    print_usize(eu64.len());
    print_usize(eu64.as_ptr() as usize);
    print_bool(ptr::eq(eu64.as_ptr(), NonNull::<u64>::dangling().as_ptr()));

    let echunk: &[[u8; 4]] = unsafe { &*black_box(EMPTY_CHUNK_RAW) };
    print_usize(echunk.len());
    print_usize(echunk.as_ptr() as usize);
    print_usize(echunk.iter().count());

    let ew: &U32Slice = unsafe { &*black_box(EMPTY_WRAP) };
    print_usize(ew.0.len());
    print_usize(ew.0.as_ptr() as usize);

    // Custom-address slices (meta == 0 and meta > 0)
    let c0: &[u32] = unsafe { &*black_box(CUSTOM_U32_0) };
    print_usize(c0.len());
    print_usize(c0.as_ptr() as usize);
    print_bool(ptr::eq(c0.as_ptr(), 0x1000 as *const u32));
    print_usize(c0.iter().count());

    let c5 = black_box(CUSTOM_U32_5);
    print_usize(c5.len());
    print_usize(c5.cast::<u32>() as usize);
    print_bool(c5.cast::<u32>().is_aligned());
    print_bool(!c5.cast::<u32>().is_null());
    print_bool(ptr::eq(c5.cast::<u32>(), 0x1000 as *const u32));
    print_bool(ptr::eq(
        c5,
        ptr::slice_from_raw_parts(0x1000 as *const u32, 5),
    ));
    print_bool(!ptr::eq(c5, CUSTOM_U32_0));
    print_usize(c5.cast::<u32>().wrapping_add(2) as usize);

    let c5m = black_box(CUSTOM_U32_5_MUT);
    print_usize(c5m.len());
    print_usize(c5m.cast::<u32>() as usize);

    let cu = black_box(CUSTOM_UNALIGNED);
    print_usize(cu.len());
    print_usize(cu.cast::<u32>() as usize);
    print_bool(cu.cast::<u32>().is_aligned());

    let cu8 = black_box(CUSTOM_U8_7);
    print_usize(cu8.len());
    print_usize(cu8.cast::<u8>() as usize);
    print_bool(ptr::eq(cu8.cast::<u8>(), 0x2000 as *const u8));

    // Null raw slices
    let n0 = black_box(NULL_U32_0);
    print_usize(n0.len());
    print_usize(n0.cast::<u32>() as usize);
    print_bool(n0.cast::<u32>().is_null());
    print_bool(ptr::eq(n0.cast::<u32>(), ptr::null()));

    let n5 = black_box(NULL_U32_5);
    print_usize(n5.len());
    print_usize(n5.cast::<u32>() as usize);
    print_bool(n5.cast::<u32>().is_null());
    print_bool(!ptr::eq(n0, n5));

    // Options, aggregates, promoted &const, statics
    print_usize(black_box(OPT_REF_SOME).map_or(999, |s| s.len()));
    print_bool(black_box(OPT_REF_NONE).is_none());
    print_usize(black_box(OPT_NN_SOME).map_or(999, |p| p.len()));
    print_bool(black_box(OPT_NN_NONE).is_none());
    print_usize(black_box(OPT_RAW_SOME).map_or(999, |p| p.len()));
    print_bool(black_box(OPT_RAW_NULL).is_some_and(|p| p.cast::<u32>().is_null()));
    print_bool(black_box(OPT_RAW_NONE).is_none());

    let h = black_box(HOLDER);
    print_usize(h.empty_u32.len());
    print_usize(h.raw_5.len());
    print_usize(h.opt.unwrap().len());

    let (p0, p1) = black_box(PAIR);
    print_usize(p0.len());
    print_usize(p1.len());

    let tbl = black_box(TABLE);
    print_usize(tbl[0].len() + tbl[1].len() + tbl[2].len());

    let pr = *black_box(PROMOTED_RAW_REF);
    print_usize(pr.len());
    print_bool(ptr::eq(pr, CUSTOM_U32_5));

    print_usize(black_box(S_EMPTY_U32.0).len());
    print_usize(black_box(S_CUSTOM_5.0).len());
    print_usize(black_box(S_EMPTY_REF).len());
    print_usize(black_box(S_OPT_NN.0).unwrap().len());
    print_usize(black_box(S_HOLDER.raw_5).len());

    // Non-scalar bare-address/dangling slices (`scale`, `thin`, `is_null`, `cast::<u8>()`)
    let cp = black_box(CUSTOM_PAIR_5);
    print_usize(cp.len());
    print_bool(cp.cast::<Pair>().is_null());
    print_usize(cp.cast::<u8>() as usize);
    print_bool(ptr::eq(cp.cast::<u8>(), 0x1000 as *const u8));
    print_bool(ptr::eq(cp.cast::<()>().cast::<u8>(), 0x1000 as *const u8));
    print_bool(ptr::eq(c5.cast::<()>().cast::<u8>(), 0x1000 as *const u8));
    print_usize(cu8.cast::<u32>() as usize);

    let np = black_box(NULL_PAIR_0);
    print_bool(np.cast::<Pair>().is_null());
    print_bool(np.cast::<u8>().is_null());

    let dp = black_box(DANGLING_PAIR_0);
    print_usize(dp.cast::<Pair>() as usize);
    print_bool(ptr::eq(
        dp.cast::<u8>(),
        NonNull::<Pair>::dangling().as_ptr().cast::<u8>(),
    ));

    // Cross-width `cast::<u8>()`, offset `wrapping_add` equality & ordering, huge addresses
    let s1 = black_box(CUSTOM_U8_4096);
    let s2 = black_box(CUSTOM_U16_4096);
    print_bool(ptr::eq(s1.cast::<u8>(), s2.cast::<u8>()));
    print_bool(ptr::addr_eq(s2, s2.cast::<u8>()));
    print_bool(ptr::eq(
        c5.cast::<u32>().wrapping_add(2),
        0x1008 as *const u32,
    ));
    print_bool(ptr::eq(
        c5.cast::<u32>().wrapping_add(2),
        ptr::slice_from_raw_parts(0x1008 as *const u32, 3).cast::<u32>(),
    ));
    print_bool(ptr::eq(
        c5.cast::<u8>().wrapping_add(8),
        0x1008 as *const u8,
    ));
    print_bool(ptr::addr_eq(
        c5.cast::<u32>().wrapping_add(2),
        c5.cast::<u8>().wrapping_add(8),
    ));
    print_ordering(
        c5.cast::<u32>()
            .wrapping_add(1)
            .cast::<()>()
            .cmp(&c5.cast::<u8>().wrapping_add(2).cast::<()>()),
    );
    let huge = black_box(HUGE_U8_0);
    print_usize(huge.cast::<u8>() as usize);
    print_bool(ptr::eq(
        huge.cast::<u8>().wrapping_add(8),
        0xffff_fff8usize as *const u8,
    ));
}
