//! NaN-payload constants inside statics, promoted references, slices, structs,
//! single-field scalar structs, tuples, arrays, Option, repr(transparent) wrappers,
//! MaybeUninit, closures, and cyclic statics (MIRES-023).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::mem::MaybeUninit;

const BITS32_A: u32 = 0x7fc0_0001;
const BITS32_B: u32 = 0xffc0_0002;
const BITS32_C: u32 = 0x7fc0_0000;
const NAN32_A: f32 = f32::from_bits(BITS32_A);
const NAN32_B: f32 = f32::from_bits(BITS32_B);
const NAN32_C: f32 = f32::from_bits(BITS32_C);

const BITS64_A: u64 = 0x7ff8_0000_0000_0001;
const BITS64_B: u64 = 0xfff8_0000_0000_0002;
const BITS64_C: u64 = 0x7ff8_0000_0000_0000;
const NAN64_A: f64 = f64::from_bits(BITS64_A);
const NAN64_B: f64 = f64::from_bits(BITS64_B);
const NAN64_C: f64 = f64::from_bits(BITS64_C);

#[derive(Copy, Clone)]
struct Pair {
    a: f32,
    b: f64,
}

#[derive(Copy, Clone)]
struct One32(f32);

#[derive(Copy, Clone)]
struct One64(f64);

#[repr(transparent)]
#[derive(Copy, Clone)]
struct Wrap32(f32);

#[repr(transparent)]
#[derive(Copy, Clone)]
struct WrapWrap32(Wrap32);

#[repr(transparent)]
#[derive(Copy, Clone)]
struct Wrap64(f64);

struct CycleNode {
    val: f64,
    next: &'static CycleNode,
}

static S_F32: f32 = NAN32_A;
static S_F64: f64 = NAN64_A;
static S_ARR32: [f32; 3] = [NAN32_A, NAN32_B, NAN32_C];
static S_ARR64: [f64; 3] = [NAN64_A, NAN64_B, NAN64_C];
static S_PAIR: Pair = Pair {
    a: NAN32_B,
    b: NAN64_B,
};

static CYCLE_A: CycleNode = CycleNode {
    val: NAN64_A,
    next: &CYCLE_B,
};
static CYCLE_B: CycleNode = CycleNode {
    val: NAN64_B,
    next: &CYCLE_A,
};

const C_OPT32_SOME: Option<f32> = Some(NAN32_A);
const C_OPT32_NONE: Option<f32> = None;
const C_OPT64_SOME: Option<f64> = Some(NAN64_A);
const C_OPT64_NONE: Option<f64> = None;
const C_PAIR: Pair = Pair {
    a: NAN32_A,
    b: NAN64_A,
};
const C_ONE32: One32 = One32(NAN32_B);
const C_ONE64: One64 = One64(NAN64_B);
const C_TUP32_ZST: (f32, ()) = (NAN32_A, ());
const C_TUP64_ZST: (f64, ()) = (NAN64_A, ());
const C_TUP_BOTH: (f32, f64) = (NAN32_B, NAN64_B);
const C_ARR32_1: [f32; 1] = [NAN32_A];
const C_ARR32_3: [f32; 3] = [NAN32_A, NAN32_B, NAN32_C];
const C_ARR64_3: [f64; 3] = [NAN64_A, NAN64_B, NAN64_C];
const C_WRAP32: WrapWrap32 = WrapWrap32(Wrap32(NAN32_A));
const C_WRAP64: Wrap64 = Wrap64(NAN64_A);
const C_MU32: MaybeUninit<f32> = MaybeUninit::new(NAN32_B);
const C_MU64: MaybeUninit<f64> = MaybeUninit::new(NAN64_B);

#[inline(never)]
fn b32_s(x: f32) -> u32 {
    x.to_bits()
}

#[inline(never)]
fn b32_c(x: f32) -> u32 {
    x.to_bits()
}

#[inline(never)]
fn b32_p(x: f32) -> u32 {
    x.to_bits()
}

#[inline(never)]
fn b64(x: f64) -> u64 {
    x.to_bits()
}

#[inline(never)]
fn read_ref32(r: &f32) -> u32 {
    (*r).to_bits()
}

#[inline(never)]
fn read_ref64(r: &f64) -> u64 {
    (*r).to_bits()
}

#[inline(never)]
fn read_slice32(s: &[f32]) -> (u32, u32) {
    (s[0].to_bits(), s[1].to_bits())
}

#[inline(never)]
fn read_slice64(s: &[f64]) -> (u64, u64) {
    (s[0].to_bits(), s[1].to_bits())
}

#[inline(never)]
fn call_closure<F: FnOnce() -> (f32, f64)>(f: F) -> (u32, u64) {
    let (a, b) = f();
    (a.to_bits(), b.to_bits())
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Statics (direct, array, struct, cyclic).
    print_u32(b32_s(S_F32));
    print_u64(b64(S_F64));
    print_u32(b32_s(S_ARR32[0]));
    print_u32(b32_s(S_ARR32[1]));
    print_u32(b32_s(S_ARR32[2]));
    print_u64(b64(S_ARR64[0]));
    print_u64(b64(S_ARR64[1]));
    print_u64(b64(S_ARR64[2]));
    print_u32(b32_s(S_PAIR.a));
    print_u64(b64(S_PAIR.b));
    print_u64(b64(CYCLE_A.val));
    print_u64(b64(CYCLE_A.next.val));
    print_u64(b64(CYCLE_B.next.val));

    // 2. Promoted references & slices.
    let r32: &'static f32 = &NAN32_A;
    let r64: &'static f64 = &NAN64_A;
    print_u32(read_ref32(black_box(r32)));
    print_u64(read_ref64(black_box(r64)));
    let (sl32_0, sl32_1) = read_slice32(black_box(&[NAN32_A, NAN32_B]));
    print_u32(sl32_0);
    print_u32(sl32_1);
    let (sl64_0, sl64_1) = read_slice64(black_box(&[NAN64_A, NAN64_B]));
    print_u64(sl64_0);
    print_u64(sl64_1);

    // 3. Option, structs, single-field structs, tuples.
    print_bool(C_OPT32_NONE.is_none());
    print_bool(C_OPT64_NONE.is_none());
    print_u32(b32_c(black_box(C_OPT32_SOME).unwrap()));
    print_u64(b64(black_box(C_OPT64_SOME).unwrap()));
    let pair = black_box(C_PAIR);
    print_u32(b32_c(pair.a));
    print_u64(b64(pair.b));
    print_u32(b32_c(black_box(C_ONE32).0));
    print_u64(b64(black_box(C_ONE64).0));
    print_u32(b32_c(black_box(C_TUP32_ZST).0));
    print_u64(b64(black_box(C_TUP64_ZST).0));
    let tup = black_box(C_TUP_BOTH);
    print_u32(b32_c(tup.0));
    print_u64(b64(tup.1));

    // 4. Arrays, repr(transparent), MaybeUninit, closure capture.
    print_u32(b32_p(black_box(C_ARR32_1)[0]));
    let a32 = black_box(C_ARR32_3);
    print_u32(b32_p(a32[0]));
    print_u32(b32_p(a32[1]));
    print_u32(b32_p(a32[2]));
    let a64 = black_box(C_ARR64_3);
    print_u64(b64(a64[0]));
    print_u64(b64(a64[1]));
    print_u64(b64(a64[2]));
    print_u32(b32_p(black_box(C_WRAP32).0 .0));
    print_u64(b64(black_box(C_WRAP64).0));
    unsafe {
        print_u32(b32_p(black_box(C_MU32).assume_init()));
        print_u64(b64(black_box(C_MU64).assume_init()));
    }
    let cap32 = NAN32_A;
    let cap64 = NAN64_B;
    let (cl32, cl64) = call_closure(move || (cap32, cap64));
    print_u32(cl32);
    print_u64(cl64);
}
