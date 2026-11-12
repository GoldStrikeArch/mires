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

struct St { a: u32, arr: [u32; 3], b: u8 }
static ST: St = St { a: 1, arr: [5, 6, 7], b: 2 };
static SL: &[u32] = &ST.arr;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(SL); show(black_box(SL)); print_bool(core::ptr::eq(SL.as_ptr(), ST.arr.as_ptr())); print_u32(ST.a + ST.b as u32); }
