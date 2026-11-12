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

static M: [[u32; 2]; 3] = [[1, 2], [3, 4], [5, 6]];
static ROWS: &[[u32; 2]] = &M;
static TAILROWS: &[[u32; 2]] = M.split_at(1).1;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(ROWS.len()); print_u32(ROWS[2][1]); show(&ROWS[1]); print_usize(TAILROWS.len()); show(&TAILROWS[0]);
    print_bool(core::ptr::eq(&TAILROWS[0], &M[1])); print_bool(core::ptr::eq(ROWS.as_ptr(), M.as_ptr()));
    print_bool(core::ptr::eq(&TAILROWS[1][1], &M[2][1]));
}
