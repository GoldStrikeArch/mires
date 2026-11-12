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

static mut BYTES: [u8; 5] = *b"hello";
static mut TEXT: &str = unsafe { core::str::from_utf8_unchecked(&*(&raw const BYTES)) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe { print_str(*(&raw const TEXT)); BYTES[0] = b'j'; print_str(*(&raw const TEXT)); }
}
