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

static BYTES: [u8; 5] = *b"hello";
static TEXT: &str = match core::str::from_utf8(&BYTES) { Ok(s) => s, Err(_) => "bad" };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(TEXT);
    print_bool(core::ptr::eq(TEXT.as_ptr(), BYTES.as_ptr()));
    print_bool(core::ptr::eq(TEXT.as_bytes().as_ptr(), &BYTES[0]));
    print_bool(core::ptr::eq(TEXT.as_bytes(), &BYTES[..]));
}
