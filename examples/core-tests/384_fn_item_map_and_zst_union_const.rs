//! `Iterator::map` / `Option::map` with tuple-struct & enum-variant constructors under MIR
//! inlining + `-Oz` (MIRES-013), plus zero-sized union constants (`MaybeUninit<FnDef>` and
//! `MaybeUninit<()>`) inside constant allocations.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;
use core::mem::MaybeUninit;

#[derive(Debug, PartialEq, Eq)]
struct Wrapped(u32);

#[derive(Debug, PartialEq, Eq)]
enum E {
    V(u32),
}

#[inline(never)]
fn twice(x: u32) -> u32 {
    x * 2
}

#[inline(never)]
fn use_mu_fn<F: Fn(u32) -> u32>(p: &(MaybeUninit<F>, u32)) -> u32 {
    let f = unsafe { p.0.assume_init_ref() };
    f(p.1)
}

#[inline(never)]
fn use_mu_unit(p: &(MaybeUninit<()>, u32)) -> u32 {
    let () = unsafe { *p.0.assume_init_ref() };
    p.1 + 1
}

static S_MU_UNIT: (MaybeUninit<()>, u32) = (MaybeUninit::new(()), 41);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<Wrapped> = (1u32..6).map(Wrapped).collect();
    let sum: u32 = v.iter().map(|w| w.0).sum();
    print_u32(sum);

    let o = Some(5u32).map(Wrapped);
    print_u32(o.unwrap().0);

    let ev: Vec<E> = (1u32..6).map(E::V).collect();
    let esum: u32 = ev
        .iter()
        .map(|e| match e {
            E::V(x) => *x,
        })
        .sum();
    print_u32(esum);

    let o2 = Some(6u32).map(twice);
    print_u32(o2.unwrap());

    let tv: Vec<u32> = (1u32..5).map(twice).collect();
    let tsum: u32 = tv.iter().copied().sum();
    print_u32(tsum);

    // Zero-sized union constants (`MaybeUninit<FnDef>`, `MaybeUninit<()>`)
    print_u32(use_mu_fn(const { &(MaybeUninit::new(twice), 21) }));
    print_u32(use_mu_unit(const { &(MaybeUninit::new(()), 17) }));
    print_u32(use_mu_unit(&S_MU_UNIT));
}
