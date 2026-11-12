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
static CHUNK: Option<&[u32; 2]> = ARR.first_chunk::<2>();
static LCHUNK: Option<&[u32; 2]> = ARR.last_chunk::<2>();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    match CHUNK { Some(c) => { print_u32(c[0] + c[1]); } None => print_str("none") }
    match LCHUNK { Some(c) => { print_u32(c[0] + c[1]); } None => print_str("none") }
}
