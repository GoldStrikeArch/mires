#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// EVERY expectation in this file assumes a 32-bit pointer width: `usize` is `u32` and
// `isize` is `i32`. The target is wasm32-unknown-unknown. The wrapping helpers are
// `#[inline(never)]` because rustc's `arithmetic_overflow` lint rejects const-foldable
// overflow outright once the target makes the overflow visible.
#[inline(never)]
fn add_usize(a: usize, b: usize) -> usize {
    a + b
}

#[inline(never)]
fn sub_usize(a: usize, b: usize) -> usize {
    a - b
}

#[inline(never)]
fn mul_usize(a: usize, b: usize) -> usize {
    a * b
}

#[inline(never)]
fn add_isize(a: isize, b: isize) -> isize {
    a + b
}

#[inline(never)]
fn mul_isize(a: isize, b: isize) -> isize {
    a * b
}

#[inline(never)]
fn shl_usize(a: usize, n: usize) -> usize {
    a << n
}

fn pick(arr: &[i32; 4], i: usize) -> i32 {
    arr[i]
}

#[no_mangle]
fn rust_entry() {
    let umax: usize = 4294967295;

    // Wrapping at 2^32 -- the whole point of the 32-bit decision.
    print_bool(add_usize(umax, 1) == 0);
    print_i32(add_usize(umax, 1) as i32);
    print_i32(add_usize(umax, 3) as i32);
    print_bool(sub_usize(0, 1) == umax);
    print_bool(mul_usize(65536, 65536) == 0);
    print_i32(mul_usize(65536, 65537) as i32);

    // Unsigned comparison and division stay unsigned at 32 bits.
    print_bool(umax > 2147483648);
    print_i32((umax / 2) as i32);
    print_i32((umax % 10) as i32);
    print_i32((umax / 65536) as i32);

    // Shifts.
    print_i32(shl_usize(1, 31) as i32);
    print_bool(shl_usize(1, 31) > 0);
    print_bool(shl_usize(1, 32) == 1);
    print_i32((umax >> 16) as i32);

    // usize as an array index, constant and computed.
    let arr = [5i32, 6, 7, 8];
    let i: usize = 2;
    print_i32(arr[i]);
    print_i32(arr[i + 1]);
    print_i32(arr[0]);
    print_i32(pick(&arr, 3));

    // Index-driven loop with a manual counter.
    let mut m = [0i32; 3];
    let mut k: usize = 0;
    while k < 3 {
        m[k] = (k as i32) * 10 + 1;
        k += 1;
    }
    print_i32(m[0]);
    print_i32(m[1]);
    print_i32(m[2]);
    print_i32(k as i32);

    // isize: negative arithmetic and wrapping at the signed 32-bit boundary.
    let imin: isize = -2147483648;
    let imax: isize = 2147483647;
    print_i32(mul_isize(-5, 3) as i32);
    print_i32(add_isize(-5, 2) as i32);
    print_bool(add_isize(-5, 2) < 0);
    print_i32(add_isize(imin, -1) as i32);
    print_i32(add_isize(imax, 1) as i32);
    print_i32(mul_isize(imax, 2) as i32);
    print_bool(imin < 0);
    print_bool(imax > 0);

    // usize <-> signed casts.
    print_i32(7usize as i32);
    print_i32(umax as i32);
    print_i32(2147483648usize as i32);
    print_i32(-1i32 as usize as i32);
    print_bool(-1i32 as usize == umax);
    print_bool(-1isize as usize == umax);
    print_i32(imin as usize as i32);
    print_bool(imin as usize > 0);
    print_i32(300usize as u8 as i32);
    print_i32(umax as u16 as i32);

    print_str("13_usize ok");
}
