//! Devirtualization of unique constant targets (Phase 5, D6): a fn-pointer constant and a
//! one-type `dyn` call become direct calls; a pointer nobody can trace stays a dispatch.
//!
//! `pick` binds a function pointer from a constant, so the call's target is unique and the O
//! group rewrites it to a direct call. `apply` receives the pointer as a parameter — no trace
//! reaches a constant, so its call stays indirect (the negative shape). The `dyn Op` call's
//! receiver comes straight from the unsizing cast of one concrete type, so its vtable slot is a
//! compile-time constant and the dispatch goes direct too. The output is mode-identical either
//! way; the rewrite is a size-and-shape claim, not a semantic one.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn double(x: u32) -> u32 {
    x.wrapping_mul(2)
}

#[inline(never)]
fn triple(x: u32) -> u32 {
    x.wrapping_mul(3)
}

/// The pointer is a parameter: no constant to trace, the call stays indirect.
#[inline(never)]
fn apply(f: fn(u32) -> u32, x: u32) -> u32 {
    f(x)
}

/// The pointer is a constant: the call devirtualizes.
#[inline(never)]
fn pick(x: u32) -> u32 {
    let f: fn(u32) -> u32 = double;
    f(x)
}

trait Op {
    fn run(&self, x: u32) -> u32;
}

struct Add3;

impl Op for Add3 {
    fn run(&self, x: u32) -> u32 {
        x.wrapping_add(3)
    }
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(pick(21));
    print_u32(apply(double, 5));
    print_u32(apply(triple, 5));
    let op: &dyn Op = &Add3;
    print_u32(op.run(39));
}
