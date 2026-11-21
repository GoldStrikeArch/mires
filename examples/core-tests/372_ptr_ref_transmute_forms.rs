//! Pointer/reference/box transmutes across scalar and aggregate pointees (MIRES-009).
//!
//! Covers:
//! - `*const T`, `*mut T`, `&T`, `&mut T`, `Box<T>` -> `usize` for both scalars and aggregates;
//! - `&T` <-> `NonNull<T>` <-> `*const T` (crossing the reference/raw-slot boundary for
//!   indirect aggregates as well as scalars);
//! - `NonNull<u32>` <-> `NonNull<u8>` / `*const u8` pointee casts via `transmute`;
//! - `&Pair` -> `NonNull<()>` -> `&Pair` type-erasure round-trip;
//! - `Option<&T>`, `Option<&mut T>` -> `usize` agreeing with `(&x as *const T) as usize`;
//! - `NonZero<usize>` <-> `Option<NonNull<u8>>` pattern-scalar niche transmutes.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use core::mem::transmute;
use core::num::NonZero;
use core::ptr::NonNull;

#[repr(C)]
struct Pair {
    a: u32,
    b: u32,
}

#[repr(transparent)]
struct RefWrap<'a, T>(&'a T);

static STAT_BYTE: u8 = 55;

#[inline(never)]
fn const_ptr_u8(p: *const u8) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn mut_ptr_u8(p: *mut u8) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn ref_u8(r: &u8) -> usize {
    unsafe { transmute(r) }
}

#[inline(never)]
fn mut_u8(r: &mut u8) -> usize {
    unsafe { transmute(r) }
}

#[inline(never)]
fn box_u8(b: Box<u8>) -> usize {
    unsafe { transmute(b) }
}

#[inline(never)]
fn const_ptr_pair(p: *const Pair) -> usize {
    unsafe { transmute(p) }
}

#[inline(never)]
fn ref_pair(r: &Pair) -> usize {
    unsafe { transmute(r) }
}

#[inline(never)]
fn mut_pair(r: &mut Pair) -> usize {
    unsafe { transmute(r) }
}

#[inline(never)]
fn box_pair(b: Box<Pair>) -> usize {
    unsafe { transmute(b) }
}

#[inline(never)]
fn opt_ref_u8(o: Option<&u8>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn opt_mut_u8(o: Option<&mut u8>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn opt_ref_pair(o: Option<&Pair>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn opt_mut_pair(o: Option<&mut Pair>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn refwrap_u8(w: RefWrap<'_, u8>) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn refwrap_pair(w: RefWrap<'_, Pair>) -> usize {
    unsafe { transmute(w) }
}

#[inline(never)]
fn nn_u32_to_ptr_u8(p: NonNull<u32>) -> *const u8 {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u32_to_nn_u8(p: NonNull<u32>) -> NonNull<u8> {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u8_to_nn_u32(p: NonNull<u8>) -> NonNull<u32> {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_u8_to_ref(p: NonNull<u8>) -> &'static u8 {
    unsafe { transmute(p) }
}

#[inline(never)]
fn ref_to_nn_u8(r: &u8) -> NonNull<u8> {
    unsafe { transmute(r) }
}

#[inline(never)]
fn nn_pair_to_ptr_pair(p: NonNull<Pair>) -> *const Pair {
    unsafe { transmute(p) }
}

#[inline(never)]
fn nn_pair_to_ref_pair<'a>(p: NonNull<Pair>) -> &'a Pair {
    unsafe { transmute(p) }
}

#[inline(never)]
fn ref_pair_to_nn_pair(r: &Pair) -> NonNull<Pair> {
    unsafe { transmute(r) }
}

#[inline(never)]
fn ref_pair_to_ptr_pair(r: &Pair) -> *const Pair {
    unsafe { transmute(r) }
}

#[inline(never)]
fn ptr_pair_to_ref_pair<'a>(p: *const Pair) -> &'a Pair {
    unsafe { transmute(p) }
}

#[inline(never)]
fn ref_pair_to_erased(r: &Pair) -> NonNull<()> {
    unsafe { transmute(r) }
}

#[inline(never)]
fn erased_to_ref_pair<'a>(p: NonNull<()>) -> &'a Pair {
    unsafe { transmute(p) }
}

#[inline(never)]
fn arr_ref_to_nn_u8(a: &[u8; 4]) -> NonNull<u8> {
    unsafe { transmute(a) }
}

#[inline(never)]
fn nz_to_opt_nn(n: NonZero<usize>) -> Option<NonNull<u8>> {
    unsafe { transmute(n) }
}

#[inline(never)]
fn opt_nn_to_nz(o: Option<NonNull<u8>>) -> NonZero<usize> {
    unsafe { transmute(o) }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut b = 11u8;
    let p_u8 = NonNull::from(&mut b);
    let a_u8 = p_u8.as_ptr() as usize;

    print_bool(const_ptr_u8(p_u8.as_ptr()) == a_u8);
    print_bool(mut_ptr_u8(p_u8.as_ptr()) == a_u8);
    print_bool(ref_u8(&b) == a_u8);
    print_bool(mut_u8(&mut b) == a_u8);
    print_bool(opt_ref_u8(Some(&b)) == a_u8);
    print_bool(opt_ref_u8(None) == 0);
    print_bool(opt_mut_u8(Some(&mut b)) == a_u8);
    print_bool(opt_mut_u8(None) == 0);
    print_bool(refwrap_u8(RefWrap(&b)) == a_u8);

    let hb = Box::new(22u8);
    let hb_addr = (&*hb as *const u8) as usize;
    print_bool(box_u8(hb) == hb_addr);

    let mut pair = Pair { a: 111, b: 222 };
    let pp = NonNull::from(&pair);
    let a_pair = (&pair as *const Pair) as usize;
    print_bool(const_ptr_pair(&pair as *const Pair) == a_pair);
    print_bool(ref_pair(&pair) == a_pair);
    print_bool(mut_pair(&mut pair) == a_pair);
    print_bool(opt_ref_pair(Some(&pair)) == a_pair);
    print_bool(opt_ref_pair(None) == 0);
    print_bool(opt_mut_pair(Some(&mut pair)) == a_pair);
    print_bool(opt_mut_pair(None) == 0);
    print_bool(refwrap_pair(RefWrap(&pair)) == a_pair);

    let hpair = Box::new(Pair { a: 7, b: 8 });
    let hp_addr = (&*hpair as *const Pair) as usize;
    print_bool(box_pair(hpair) == hp_addr);

    // Pointee casts through transmute
    let mut x = 0x1234_5678u32;
    let px = NonNull::from(&mut x);
    print_bool((nn_u32_to_ptr_u8(px) as usize) == (px.as_ptr() as usize));
    let nn_b = nn_u32_to_nn_u8(px);
    print_bool((nn_b.as_ptr() as usize) == (px.as_ptr() as usize));
    print_bool(unsafe { *nn_u8_to_nn_u32(nn_b).as_ptr() } == 0x1234_5678u32);

    // Scalar ref <-> NonNull
    let ps = NonNull::from(&STAT_BYTE);
    print_bool(*nn_u8_to_ref(ps) == 55);
    print_bool(unsafe { *ref_to_nn_u8(&STAT_BYTE).as_ptr() } == 55);

    // Aggregate ref <-> NonNull <-> raw ptr <-> erased NonNull<()>
    print_bool(unsafe { (*nn_pair_to_ptr_pair(pp)).a } == 111);
    let r_from_nn = nn_pair_to_ref_pair(pp);
    print_bool(r_from_nn.a == 111 && r_from_nn.b == 222);
    let nn_from_r = ref_pair_to_nn_pair(&pair);
    print_bool(unsafe { (*nn_from_r.as_ptr()).a } == 111 && unsafe { (*nn_from_r.as_ptr()).b } == 222);
    print_bool(unsafe { (*ref_pair_to_ptr_pair(&pair)).b } == 222);
    let r_from_ptr = ptr_pair_to_ref_pair(pp.as_ptr());
    print_bool(r_from_ptr.a == 111 && r_from_ptr.b == 222);
    let erased = ref_pair_to_erased(&pair);
    let r_from_erased = erased_to_ref_pair(erased);
    print_bool(r_from_erased.a == 111 && r_from_erased.b == 222);

    // Array ref -> NonNull<u8> (fmt::Arguments::new shape)
    let arr = [4u8, 5, 6, 7];
    let arr_nn = arr_ref_to_nn_u8(&arr);
    print_bool(unsafe { *arr_nn.as_ptr() } == 4 && unsafe { *arr_nn.as_ptr().add(3) } == 7);

    // Pattern-scalar niche transmutes: NonZero<usize> <-> Option<NonNull<u8>>
    let nz_lit = NonZero::new(0x3000usize).unwrap();
    print_bool(nz_to_opt_nn(nz_lit).map(|q| q.as_ptr() as usize) == Some(0x3000));
    print_bool(opt_nn_to_nz(Some(p_u8)).get() == a_u8);
}
