//! Promoted `&fn_item` and zero-sized closure/constructor constants in allocations (MIRES-024).
//!
//! `apply_ref(&twice)` promotes `&twice` to a constant reference to a zero-sized `ty::FnDef`.
//! Covers `Fn`/`FnMut`/`FnOnce`, arrays and slices of fn items, `Option<&F>` and `Option<F>`,
//! tuples, generic fn items, inherent methods, tuple-struct and enum-variant constructors,
//! non-capturing closures, raw pointers / `NonNull`, transparent and packed wrappers, and
//! `const`/`static` function pointers.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use core::hint::black_box;
use core::ptr::NonNull;

#[inline(never)]
fn twice(x: u32) -> u32 {
    x * 2
}

#[inline(never)]
fn plus1(x: u32) -> u32 {
    x + 1
}

#[inline(never)]
fn id<T>(x: T) -> T {
    x
}

struct Wrapped(u32);

enum E {
    V(u32),
}

#[repr(packed)]
struct PackedPair<F>(F, u32);

#[repr(transparent)]
struct TransZst<F>(F);

#[inline(never)]
fn apply_ref<F: Fn(u32) -> u32>(f: &F) -> u32 {
    f(7) + f(8)
}

#[inline(never)]
fn call_once<F: FnOnce(u32) -> u32 + Copy>(f: &F, x: u32) -> u32 {
    (*f)(x)
}

#[inline(never)]
fn call_mut<F: FnMut(u32) -> u32 + Copy>(f: &F, x: u32) -> u32 {
    let mut g = *f;
    g(x)
}

#[inline(never)]
fn call_arr<F: Fn(u32) -> u32>(fs: &[F; 3], x: u32) -> u32 {
    fs[0](x) + fs[1](x + 1) + fs[2](x + 2)
}

#[inline(never)]
fn call_slice<F: Fn(u32) -> u32>(fs: &[F], x: u32) -> u32 {
    let mut sum = 0;
    for f in fs {
        sum += f(x);
    }
    sum
}

#[inline(never)]
fn call_opt_ref_by_val<F: Fn(u32) -> u32>(o: Option<&F>, x: u32) -> u32 {
    match o {
        Some(f) => f(x),
        None => 99,
    }
}

#[inline(never)]
fn call_opt_ref_by_ref<F: Fn(u32) -> u32>(o: &Option<&F>, x: u32) -> u32 {
    match o {
        Some(f) => f(x),
        None => 99,
    }
}

#[inline(never)]
fn call_opt_ref_none<F: Fn(u32) -> u32>(_: &F, x: u32) -> u32 {
    call_opt_ref_by_ref(&None::<&F>, x) + call_opt_ref_by_val(None::<&F>, x)
}

#[inline(never)]
fn call_opt_val<F: Fn(u32) -> u32>(o: &Option<F>, x: u32) -> u32 {
    match o {
        Some(f) => f(x),
        None => 88,
    }
}

#[inline(never)]
fn call_opt_val_none<F: Fn(u32) -> u32>(_: &F, x: u32) -> u32 {
    call_opt_val(&None::<F>, x)
}

#[inline(never)]
fn call_pair<F: Fn(u32) -> u32>(p: &(F, u32)) -> u32 {
    (p.0)(p.1)
}

#[inline(never)]
fn call_str_len<F: Fn(&String) -> usize>(f: &F, s: &String) -> usize {
    f(s)
}

#[inline(never)]
fn call_bin<F: Fn(u32, u32) -> u32>(f: &F, a: u32, b: u32) -> u32 {
    f(a, b)
}

#[inline(never)]
fn call_ctor<T, F: Fn(u32) -> T>(f: &F, x: u32) -> T {
    f(x)
}

#[inline(never)]
fn call_raw<F: Fn(u32) -> u32>(p: *const F, x: u32) -> u32 {
    unsafe { (&*p)(x) }
}

#[inline(never)]
fn check_non_null<F: Fn(u32) -> u32>(f: &F) -> bool {
    let p: *const F = f;
    let nn = NonNull::from(f);
    !p.is_null() && (p as usize) != 0 && (nn.as_ptr() as usize) != 0
}

#[inline(never)]
fn call_packed<F: Fn(u32) -> u32 + Copy>(p: &PackedPair<F>) -> u32 {
    let f = p.0;
    let x = p.1;
    f(x)
}

#[inline(never)]
fn call_trans_zst<F: Fn(u32) -> u32>(p: &TransZst<F>, x: u32) -> u32 {
    (p.0)(x)
}

const C_FP: fn(u32) -> u32 = twice;
const C_OPT_FP_SOME: Option<fn(u32) -> u32> = Some(twice);
const C_OPT_FP_NONE: Option<fn(u32) -> u32> = None;
const C_ARR_FP: &[fn(u32) -> u32] = &[twice, plus1];
static S_FP: fn(u32) -> u32 = twice;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(apply_ref(&twice));
    print_u32(call_once(&twice, 5));
    print_u32(call_mut(&twice, 6));
    print_u32(call_arr(&[twice, twice, twice], 1));
    print_u32(call_slice(&[twice, twice, twice][..], 3));
    print_u32(call_opt_ref_by_val(Some(&twice), 10));
    print_u32(call_opt_ref_by_val(black_box(Some(&twice)), 11));
    print_u32(call_opt_ref_by_ref(&Some(&twice), 12));
    print_u32(call_opt_ref_none(&twice, 12));
    print_u32(call_opt_val(&Some(twice), 13));
    print_u32(call_opt_val_none(&twice, 13));
    print_u32(call_pair(&(twice, 14)));
    print_u32(call_once(&id::<u32>, 15));
    let s = String::from("hello");
    print_u32(call_str_len(&String::len, &s) as u32);
    print_u32(call_bin(&u32::wrapping_add, 20, 22));
    print_u32(call_ctor(&Wrapped, 33).0);
    let E::V(v) = call_ctor(&E::V, 44);
    print_u32(v);
    print_u32(call_raw(&twice as *const _, 7));
    print_bool(check_non_null(&twice));
    print_u32(call_packed(&PackedPair(twice, 8)));
    print_u32(call_trans_zst(&TransZst(twice), 9));

    // Non-capturing closures promoted to constants
    print_u32(apply_ref(&|x| x * 3));
    print_u32(call_slice(&[|x: u32| x + 2][..], 4));
    let cl = |x: u32| x * 4;
    print_u32(call_opt_ref_by_ref(&Some(&cl), 3));
    print_u32(call_opt_val(&Some(cl), 3));
    print_bool(check_non_null(&|x: u32| x + 6));

    // Constant & static fn pointers
    print_u32(black_box(C_FP)(10));
    print_u32(match black_box(C_OPT_FP_SOME) {
        Some(f) => f(11),
        None => 0,
    });
    print_u32(match black_box(C_OPT_FP_NONE) {
        Some(f) => f(11),
        None => 77,
    });
    print_u32(C_ARR_FP[0](4) + C_ARR_FP[1](4));
    print_u32(S_FP(12));
}
