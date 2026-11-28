//! Regression test for explicit `callee_tracks_caller` encoding (`JIR_SCHEMA = 25`):
//! `#[track_caller]` bodies calling non-`#[track_caller]` functions, closures (`unwrap_or_else`),
//! and trait-object methods must compile and preserve exact caller locations without link-time
//! ABI guessing.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::panic::Location;

trait Compute {
    fn compute(&self, x: i32) -> i32;
}

struct Adder(i32);
impl Compute for Adder {
    fn compute(&self, x: i32) -> i32 {
        self.0 + x
    }
}

#[inline(never)]
fn plain_helper(x: i32) -> i32 {
    x * 2 + 1
}

#[track_caller]
#[inline(never)]
fn tracked_caller_line() -> u32 {
    Location::caller().line()
}

#[track_caller]
#[inline(never)]
fn tracked_calling_mixed(opt: Option<i32>, dyn_c: &dyn Compute, f: impl FnOnce() -> i32) -> (i32, u32) {
    let a = opt.unwrap_or_else(f);
    let b = plain_helper(a);
    let c = dyn_c.compute(b);
    let line = tracked_caller_line();
    (c, line)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let adder = Adder(10);
    let expected_line = line!() + 1;
    let (v1, l1) = tracked_calling_mixed(None, &adder, || 5);
    print_i32(v1);
    print_bool(l1 == expected_line);

    let expected_line2 = line!() + 1;
    let (v2, l2) = tracked_calling_mixed(Some(7), &adder, || 99);
    print_i32(v2);
    print_bool(l2 == expected_line2);
}
