//! `transmute` of `NonNull<T>` (and pattern-wrapped pointers) to/from pointer-sized
//! integers across both the JIR and baseline (v1) lowering lanes (MIRES-009).
//!
//! `NonNull<T>` unwraps to `ty::Pat(*const T, !null)`. In the baseline lane,
//! `transmute_scalar` previously called `inner_from.builtin_deref(true)` without
//! `value::peel_pattern`, causing `transmute::<NonNull<T>, usize>` (and wrappers or
//! `Option<NonNull<T>>` transmuted to `usize`) to apply `>>> 0` directly to the
//! `{ buf, off }` slot and silently return `0`.
//!
//! Includes a baseline-fallback function (`fallback_nn_to_usize`, triggered cleanly by a
//! guarded `WideGlobalPointerConstant`) so default `js-lane=jir js-opt=on` builds also
//! exercise the baseline lane directly.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use core::hint::black_box;
use core::mem::{ManuallyDrop, MaybeUninit, align_of, transmute};
use core::num::NonZero;
use core::ptr::NonNull;

#[repr(C)]
struct Pair {
    a: u32,
    b: u32,
}

struct Zst;

#[repr(transparent)]
struct Wrap(NonNull<u8>);

#[repr(transparent)]
struct Wrap2(Wrap);

static STAT_U8: u8 = 77;
static STAT_PAIR: Pair = Pair { a: 10, b: 20 };
static STATIC_SLICE_BUF: [u8; 4] = [1, 2, 3, 4];

struct SliceHolder(&'static [u8]);
const STATIC_SLICE_HOLDER: SliceHolder = SliceHolder(&STATIC_SLICE_BUF);

#[inline(never)]
fn nn_u8_usize(p: NonNull<u8>) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u8_isize(p: NonNull<u8>) -> isize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u32_usize(p: NonNull<u32>) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u64_usize(p: NonNull<u64>) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_pair_usize(p: NonNull<Pair>) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_zst_usize(p: NonNull<Zst>) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_to_nz(p: NonNull<u8>) -> NonZero<usize> {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nz_to_nn(n: NonZero<usize>) -> NonNull<u8> {
    unsafe { transmute(n) }
}

#[inline(never)]
fn usize_to_nn(x: usize) -> NonNull<u8> {
    unsafe { transmute(x) }
}

#[inline(never)]
fn isize_to_nn(x: isize) -> NonNull<u8> {
    unsafe { transmute(x) }
}

#[inline(never)]
fn opt_nn_usize(o: Option<NonNull<u8>>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn usize_to_opt_nn(x: usize) -> Option<NonNull<u8>> {
    unsafe { transmute(x) }
}

#[inline(never)]
fn opt_nn_pair_usize(o: Option<NonNull<Pair>>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn wrap_to_usize(w: Wrap) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn wrap2_to_usize(w: Wrap2) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn mu_nn_to_usize(w: MaybeUninit<NonNull<u8>>) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn md_nn_to_usize(w: ManuallyDrop<NonNull<u8>>) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn tup_nn_to_usize(w: (NonNull<u8>,)) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn arr_nn_to_usize(w: [NonNull<u8>; 1]) -> usize {
    unsafe { transmute(w) }
}

// Forces JIR encode decline (`WideGlobalPointerConstant`) so this function stays on the
// baseline lane even in default `js-lane=jir js-opt=on` builds.
#[inline(never)]
fn fallback_nn_to_usize(p: NonNull<u8>, take_const: bool) -> usize {
    if take_const {
        return STATIC_SLICE_HOLDER.0.as_ptr() as usize;
    }
    unsafe { transmute(p) }
}

#[inline(never)]
fn fallback_opt_nn_to_usize(o: Option<NonNull<u8>>, take_const: bool) -> usize {
    if take_const {
        return STATIC_SLICE_HOLDER.0.as_ptr() as usize;
    }
    unsafe { transmute(o) }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut b = 3u8;
    let p_u8 = NonNull::from(&mut b);
    let a_u8 = p_u8.as_ptr() as usize;
    print_bool(nn_u8_usize(p_u8) != 0 && nn_u8_usize(p_u8) == a_u8);
    print_bool(nn_u8_isize(p_u8) == (a_u8 as isize));

    let mut w32 = 42u32;
    let p_u32 = NonNull::from(&mut w32);
    print_bool(nn_u32_usize(p_u32) != 0 && nn_u32_usize(p_u32) == (p_u32.as_ptr() as usize));

    let mut w64 = 99u64;
    let p_u64 = NonNull::from(&mut w64);
    print_bool(nn_u64_usize(p_u64) != 0 && nn_u64_usize(p_u64) == (p_u64.as_ptr() as usize));

    let mut pair = Pair { a: 1, b: 2 };
    let p_pair = NonNull::from(&mut pair);
    let a_pair = p_pair.as_ptr() as usize;
    print_bool(nn_pair_usize(p_pair) != 0 && nn_pair_usize(p_pair) == a_pair);

    let mut z = Zst;
    let p_zst = NonNull::from(&mut z);
    print_bool(nn_zst_usize(p_zst) != 0 && nn_zst_usize(p_zst) == (p_zst.as_ptr() as usize));

    let h_u8 = Box::new(5u8);
    let ph_u8 = NonNull::from(&*h_u8);
    print_bool(nn_u8_usize(ph_u8) != 0 && nn_u8_usize(ph_u8) == (ph_u8.as_ptr() as usize));

    let h_pair = Box::new(Pair { a: 3, b: 4 });
    let ph_pair = NonNull::from(&*h_pair);
    print_bool(nn_pair_usize(ph_pair) != 0 && nn_pair_usize(ph_pair) == (ph_pair.as_ptr() as usize));

    let ps_u8 = NonNull::from(&STAT_U8);
    print_bool(nn_u8_usize(ps_u8) != 0 && nn_u8_usize(ps_u8) == (ps_u8.as_ptr() as usize));

    let ps_pair = NonNull::from(&STAT_PAIR);
    print_bool(nn_pair_usize(ps_pair) != 0 && nn_pair_usize(ps_pair) == (ps_pair.as_ptr() as usize));

    let dang_u8 = NonNull::<u8>::dangling();
    print_bool(nn_u8_usize(dang_u8) == align_of::<u8>() && nn_u8_usize(dang_u8) == (dang_u8.as_ptr() as usize));

    let dang_u32 = NonNull::<u32>::dangling();
    print_bool(nn_u32_usize(dang_u32) == align_of::<u32>() && nn_u32_usize(dang_u32) == (dang_u32.as_ptr() as usize));

    let wp = NonNull::<u8>::without_provenance(NonZero::new(0x1234).unwrap());
    print_bool(nn_u8_usize(wp) == 0x1234 && nn_u8_usize(wp) == (wp.as_ptr() as usize));

    // NonZero<usize> and usize/isize round-trips
    let nz = nn_to_nz(p_u8);
    print_bool(nz.get() == a_u8);
    print_bool((nz_to_nn(nz).as_ptr() as usize) == a_u8);
    print_bool(nn_u8_usize(usize_to_nn(a_u8)) == a_u8);
    print_bool(nn_u8_usize(usize_to_nn(0x1000)) == 0x1000);
    let neg_nn = isize_to_nn(black_box(-4isize));
    print_bool((neg_nn.as_ptr() as usize) == ((-4isize) as usize));
    print_bool(nn_u8_usize(neg_nn) == ((-4isize) as usize));

    // Option<NonNull<T>>
    print_bool(opt_nn_usize(Some(p_u8)) == a_u8);
    print_bool(opt_nn_usize(None) == 0);
    print_bool(usize_to_opt_nn(0).is_none());
    print_bool(usize_to_opt_nn(0x2000).map(|q| q.as_ptr() as usize) == Some(0x2000));
    print_bool(opt_nn_pair_usize(Some(p_pair)) == a_pair);
    print_bool(opt_nn_pair_usize(None) == 0);

    // Transparent and single-field wrappers
    print_bool(wrap_to_usize(Wrap(p_u8)) == a_u8);
    print_bool(wrap2_to_usize(Wrap2(Wrap(p_u8))) == a_u8);
    print_bool(mu_nn_to_usize(MaybeUninit::new(p_u8)) == a_u8);
    print_bool(md_nn_to_usize(ManuallyDrop::new(p_u8)) == a_u8);
    print_bool(tup_nn_to_usize((p_u8,)) == a_u8);
    print_bool(arr_nn_to_usize([p_u8]) == a_u8);

    // Baseline-fallback path under default JIR builds
    let take_const = black_box(false);
    print_bool(fallback_nn_to_usize(p_u8, take_const) == a_u8);
    print_bool(fallback_nn_to_usize(ph_u8, take_const) == (ph_u8.as_ptr() as usize));
    print_bool(fallback_opt_nn_to_usize(Some(p_u8), take_const) == a_u8);
    print_bool(fallback_opt_nn_to_usize(None, take_const) == 0);
}
