//! Verifier round-2 probe o1: an `Option` with a pointer niche inside constant data.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
struct Opt { name: &'static str, scale: Option<&'static u32> }
static ONE: Opt = Opt { name: "one", scale: Some(&7) };
const TWO: &[Opt] = &[Opt { name: "two", scale: Some(&9) }, Opt { name: "three", scale: None }];
#[derive(Clone, Copy)] struct P { x: u32, tag: &'static str }
const THREE: &[Option<P>] = &[Some(P { x: 5, tag: "p" }), None];
#[inline(never)] fn idx(i: usize) -> usize { core::hint::black_box(i) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(ONE.scale.map_or(0, |s| *s));
    print_u32(TWO[idx(0)].scale.map_or(0, |s| *s));
    print_u32(TWO[idx(1)].scale.map_or(100, |s| *s));
    print_u32(THREE[idx(0)].map_or(0, |p| p.x));
    print_bool(THREE[idx(1)].is_none());
    let local = Opt { name: "local", scale: Some(&11) };
    print_u32(core::hint::black_box(&local).scale.map_or(0, |s| *s));
}
