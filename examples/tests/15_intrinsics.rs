#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// The intrinsics this test exercises, declared the way `mini_core` declares `ptr_metadata`: a
// `#[rustc_intrinsic]` function with no body. rustc checks each signature against its own table,
// so these are exactly the shapes `core` declares — only the trait bounds (which live in `core`)
// are dropped.
//
// A body-less intrinsic is `must_be_overridden`, so every one of these reaches
// `backend/src/intrinsics.rs`. The exception is deliberate: `wrapping_add` and `wrapping_mul` are
// rewritten into a MIR `BinaryOp` by rustc's own `LowerIntrinsics` pass long before codegen, and
// are here to prove that path still ends up wrapping the same way.
mod intrinsics {
    use super::mini_core::Copy;

    #[rustc_intrinsic]
    pub const fn size_of<T>() -> usize;

    #[rustc_intrinsic]
    pub const fn align_of<T>() -> usize;

    #[rustc_intrinsic]
    pub const fn variant_count<T>() -> usize;

    #[rustc_intrinsic]
    pub const fn needs_drop<T>() -> bool;

    #[rustc_intrinsic]
    pub const fn type_name<T>() -> &'static str;

    #[rustc_intrinsic]
    pub const fn ctpop<T: Copy>(x: T) -> u32;

    #[rustc_intrinsic]
    pub const fn ctlz<T: Copy>(x: T) -> u32;

    #[rustc_intrinsic]
    pub const fn cttz<T: Copy>(x: T) -> u32;

    #[rustc_intrinsic]
    pub const fn bswap<T: Copy>(x: T) -> T;

    #[rustc_intrinsic]
    pub const fn bitreverse<T: Copy>(x: T) -> T;

    #[rustc_intrinsic]
    pub const fn rotate_left<T: Copy>(x: T, shift: u32) -> T;

    #[rustc_intrinsic]
    pub const fn rotate_right<T: Copy>(x: T, shift: u32) -> T;

    #[rustc_intrinsic]
    pub const fn wrapping_add<T: Copy>(a: T, b: T) -> T;

    #[rustc_intrinsic]
    pub const fn wrapping_mul<T: Copy>(a: T, b: T) -> T;

    #[rustc_intrinsic]
    pub const fn saturating_add<T: Copy>(a: T, b: T) -> T;

    #[rustc_intrinsic]
    pub const fn saturating_sub<T: Copy>(a: T, b: T) -> T;

    #[rustc_intrinsic]
    pub const unsafe fn exact_div<T: Copy>(x: T, y: T) -> T;

    #[rustc_intrinsic]
    pub fn sqrtf64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub fn powif64(a: f64, x: i32) -> f64;

    #[rustc_intrinsic]
    pub const fn floorf64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn ceilf64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn truncf64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn roundf64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn round_ties_even_f64(x: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn fabs<T: Copy>(x: T) -> T;

    #[rustc_intrinsic]
    pub const fn copysignf64(x: f64, y: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn minimumf64(x: f64, y: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn maximumf64(x: f64, y: f64) -> f64;

    #[rustc_intrinsic]
    pub const fn black_box<T>(dummy: T) -> T;

    // Returns `&'static <the panic_location lang item>`, which here is
    // `mini_core::PanicLocation`.
    #[rustc_intrinsic]
    pub fn caller_location() -> &'static super::mini_core::PanicLocation;
}

struct Big {
    a: i32,
    b: i32,
    c: i32,
}

enum Three {
    A,
    B,
    C,
}

fn layout() {
    print_i32(intrinsics::size_of::<i32>() as i32);
    print_i32(intrinsics::size_of::<u8>() as i32);
    print_i32(intrinsics::size_of::<Big>() as i32);
    print_i32(intrinsics::align_of::<Big>() as i32);
    print_i32(intrinsics::variant_count::<Three>() as i32);
    print_bool(intrinsics::needs_drop::<i32>());
    print_str(intrinsics::type_name::<i32>());
    print_str(intrinsics::type_name::<bool>());
}

fn bits() {
    print_i32(intrinsics::ctpop(182u32) as i32);
    print_i32(intrinsics::ctpop(255u8) as i32);
    print_i32(intrinsics::ctlz(1u32) as i32);
    print_i32(intrinsics::ctlz(0u32) as i32);
    print_i32(intrinsics::ctlz(1u8) as i32);
    print_i32(intrinsics::cttz(8u32) as i32);
    print_i32(intrinsics::cttz(0u8) as i32);
    print_i32(intrinsics::bswap(305419896u32) as i32);
    print_i32(intrinsics::bswap(4660u16) as i32);
    print_i32(intrinsics::bitreverse(129u8) as i32);
    print_i32(intrinsics::bitreverse(2u8) as i32);
    print_i32(intrinsics::rotate_left(305419896u32, 8) as i32);
    print_i32(intrinsics::rotate_right(305419896u32, 8) as i32);
    print_i32(intrinsics::rotate_left(305419896u32, 0) as i32);
    print_i32(intrinsics::rotate_left(129u8, 1) as i32);
}

fn arithmetic() {
    print_i32(intrinsics::wrapping_add(2147483647i32, 1));
    print_i32(intrinsics::wrapping_mul(65536i32, 65536));
    print_i32(intrinsics::saturating_add(120i8, 30) as i32);
    print_i32(intrinsics::saturating_sub(-120i8, 30) as i32);
    print_i32(intrinsics::saturating_add(250u8, 10) as i32);
    print_i32(intrinsics::saturating_sub(5u8, 10) as i32);
    print_i32(unsafe { intrinsics::exact_div(-12i32, 4) });
}

// The same operations at 64 bits, where a value is a JS `BigInt` and the backend spells the bit
// twiddling as a loop rather than as the 32 bit folklore.
fn wide() {
    print_i32(intrinsics::ctpop(255u64) as i32);
    print_i32(intrinsics::ctlz(1u64) as i32);
    print_i32(intrinsics::ctlz(0u64) as i32);
    print_i32(intrinsics::cttz(0u64) as i32);
    print_i32(intrinsics::cttz(256u64) as i32);
    print_i64(intrinsics::bswap(72623859790382856u64) as i64);
    print_i64(intrinsics::bitreverse(1u64) as i64);
    print_i64(intrinsics::rotate_left(81985529216486895u64, 8) as i64);
    print_i64(intrinsics::rotate_right(81985529216486895u64, 8) as i64);
    print_i64(intrinsics::saturating_add(9223372036854775807i64, 1));
    print_i64(intrinsics::saturating_sub(-9223372036854775808i64, 1));
    print_i64(intrinsics::saturating_sub(5u64, 10) as i64);
    print_i64(intrinsics::wrapping_add(9223372036854775807i64, 1));
    print_i64(unsafe { intrinsics::exact_div(-12i64, 4) });
}

fn floats() {
    print_f64(intrinsics::sqrtf64(2.0));
    print_f64(intrinsics::powif64(2.0, 10));
    print_f64(intrinsics::floorf64(-1.5));
    print_f64(intrinsics::ceilf64(-1.5));
    print_f64(intrinsics::truncf64(-1.7));
    print_f64(intrinsics::roundf64(-2.5));
    print_f64(intrinsics::roundf64(2.5));
    print_f64(intrinsics::round_ties_even_f64(-2.5));
    print_f64(intrinsics::round_ties_even_f64(2.5));
    print_f64(intrinsics::round_ties_even_f64(3.5));
    print_f64(intrinsics::fabs(-3.25f64));
    print_f64(intrinsics::copysignf64(3.25, -1.0));
    print_f64(intrinsics::minimumf64(1.5, 2.5));
    print_f64(intrinsics::maximumf64(1.5, 2.5));
}

// `#[track_caller]`: the callee takes the caller's `Location` as a hidden trailing parameter, and
// `caller_location()` reads it back. A `#[track_caller]` function passes its *own* location on,
// which is why `through` reports the line that called `through` rather than the line inside it.
#[track_caller]
fn line_of_caller() -> u32 {
    intrinsics::caller_location().line
}

#[track_caller]
fn through() -> u32 {
    line_of_caller()
}

// Not `#[track_caller]`, so `caller_location()` means "here": the line inside this function.
fn line_of_here() -> u32 {
    intrinsics::caller_location().line
}

fn track_caller() {
    print_i32(line_of_caller() as i32);
    print_i32(line_of_caller() as i32);
    print_i32(through() as i32);
    print_i32(line_of_here() as i32);
}

#[no_mangle]
fn rust_entry() {
    layout();
    bits();
    arithmetic();
    wide();
    floats();
    print_i32(intrinsics::black_box(42i32));
    track_caller();
}
