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

const C: &[u32] = &[100, 200, 300];
static S: &[u32] = &[1, 2, 3];
const CS: &str = "hello";
const CO: Option<&[u32]> = Some(&[7, 8]);
static TABLE: [&[u32]; 2] = [&[1], &[2, 3]];
#[unsafe(no_mangle)]
pub fn rust_entry() { show(C); show(S); print_str(CS); showo(CO); show(TABLE[black_box(1)]); print_bool(core::ptr::eq(S, S)); }
