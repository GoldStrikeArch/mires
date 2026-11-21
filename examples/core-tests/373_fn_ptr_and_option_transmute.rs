//! Function-pointer and `Option<fn>` transmutes / address exposure (MIRES-009).
//!
//! Ensures:
//! - `transmute::<fn(u32) -> u32, usize>` and `f as usize` return a non-zero, stable
//!   address agreeing with `(f as *const ()) as usize` and distinguishing distinct functions;
//! - `transmute::<Option<fn(u32) -> u32>, usize>` returns that same address for `Some(f)`
//!   and `0` for `None`;
//! - `transmute::<usize, Option<fn(u32) -> u32>>(0)` decodes to `None`;
//! - `fn(u32) -> u32` <-> `*const ()` and `NonNull<()>` round-trips remain callable.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::mem::transmute;
use core::ptr::NonNull;

fn inc(x: u32) -> u32 {
    x + 1
}

fn dec(x: u32) -> u32 {
    x.wrapping_sub(1)
}

#[inline(never)]
fn fn_to_usize(f: fn(u32) -> u32) -> usize {
    unsafe { transmute(f) }
}

#[inline(never)]
fn fn_as_usize(f: fn(u32) -> u32) -> usize {
    f as usize
}

#[inline(never)]
fn fn_via_raw_usize(f: fn(u32) -> u32) -> usize {
    (f as *const ()) as usize
}

#[inline(never)]
fn opt_fn_to_usize(o: Option<fn(u32) -> u32>) -> usize {
    unsafe { transmute(o) }
}

#[inline(never)]
fn usize_to_opt_fn(x: usize) -> Option<fn(u32) -> u32> {
    unsafe { transmute(x) }
}

#[inline(never)]
fn fn_to_ptr(f: fn(u32) -> u32) -> *const () {
    unsafe { transmute(f) }
}

#[inline(never)]
fn ptr_to_fn(p: *const ()) -> fn(u32) -> u32 {
    unsafe { transmute(p) }
}

#[inline(never)]
fn fn_to_nn(f: fn(u32) -> u32) -> NonNull<()> {
    unsafe { transmute(f) }
}

#[inline(never)]
fn nn_to_fn(p: NonNull<()>) -> fn(u32) -> u32 {
    unsafe { transmute(p) }
}

#[inline(never)]
fn usize_to_opt_ref(x: usize) -> Option<&'static u8> {
    unsafe { transmute(x) }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let f1: fn(u32) -> u32 = inc;
    let f2: fn(u32) -> u32 = dec;

    // Function pointer round-trips remain callable
    let p = fn_to_ptr(f1);
    print_bool(ptr_to_fn(p)(41) == 42);
    let nn = fn_to_nn(f2);
    print_bool(nn_to_fn(nn)(100) == 99);

    // Function pointer address exposure via transmute, `as usize`, and `as *const () as usize`
    let a1 = fn_to_usize(f1);
    let a1_as = fn_as_usize(f1);
    let a1_raw = fn_via_raw_usize(f1);
    let a2 = fn_to_usize(f2);
    print_bool(a1 != 0);
    print_bool(a1 == a1_as && a1 == a1_raw);
    print_bool(a2 != 0 && a2 == fn_as_usize(f2) && a2 == fn_via_raw_usize(f2));
    print_bool(a1 != a2);

    // Option<fn> <-> usize
    print_bool(opt_fn_to_usize(Some(f1)) == a1);
    print_bool(opt_fn_to_usize(Some(f2)) == a2);
    print_bool(opt_fn_to_usize(None) == 0);
    print_bool(usize_to_opt_fn(0).is_none());
    print_bool(usize_to_opt_fn(a1).is_some());

    // Option<&u8> niche decode for 0
    print_bool(usize_to_opt_ref(0).is_none());
}
