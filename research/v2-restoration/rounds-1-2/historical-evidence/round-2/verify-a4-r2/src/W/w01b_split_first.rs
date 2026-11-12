#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
#[inline(never)] fn sum(s: &[u32]) -> u32 { let mut t = 0; for v in s { t = t * 3 + *v; } t }
#[inline(never)] fn show(s: &[u32]) { print_usize(s.len()); print_u32(sum(s)); }
#[inline(never)] fn showo(o: Option<&[u32]>) { match o { Some(s) => show(s), None => print_str("none") } }

static ARR: [u32; 4] = [10, 20, 30, 40];
static FIRST: Option<(&u32, &[u32])> = ARR.split_first();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    match FIRST { Some((a, r)) => { print_u32(*a); show(r); print_bool(core::ptr::eq(a, &ARR[0])); print_bool(core::ptr::eq(r.as_ptr(), &ARR[1])); } None => print_str("none") }
}
