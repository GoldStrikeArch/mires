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

static BYTES: [u8; 11] = *b"hello world";
static TEXT: &str = match core::str::from_utf8(&BYTES) { Ok(s) => s, Err(_) => "bad" };
static MIDT: &str = match core::str::from_utf8(BYTES.split_at(6).1) { Ok(s) => s, Err(_) => "bad" };
static EMPT: &str = match core::str::from_utf8(BYTES.split_at(11).1) { Ok(s) => s, Err(_) => "bad" };
static OPT: Option<&str> = match core::str::from_utf8(BYTES.split_at(5).0) { Ok(s) => Some(s), Err(_) => None };
static UNI: [u8; 7] = [0xC3, 0xA9, b'x', 0xE2, 0x82, 0xAC, b'!'];
static UTEXT: &str = match core::str::from_utf8(&UNI) { Ok(s) => s, Err(_) => "bad" };
static UTAIL: &str = match core::str::from_utf8(UNI.split_at(3).1) { Ok(s) => s, Err(_) => "bad" };
static BSL: &[u8] = &BYTES;
static BTAIL: &[u8] = BYTES.split_at(6).1;
struct N { name: &'static str, raw: &'static [u8] }
static NN: [N; 2] = [N { name: TEXT, raw: BSL }, N { name: MIDT, raw: BTAIL }];
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(TEXT); print_usize(TEXT.len()); print_str(MIDT); print_usize(MIDT.len()); print_str(EMPT); print_usize(EMPT.len());
    print_str(OPT.unwrap_or("none")); print_str(black_box(OPT).unwrap_or("none"));
    print_str(UTEXT); print_usize(UTEXT.len()); print_usize(UTEXT.chars().count()); print_str(UTAIL); print_usize(UTAIL.len());
    print_u32(UTEXT.chars().nth(2).unwrap() as u32);
    print_usize(BSL.len()); print_u32(BSL[4] as u32); print_usize(BTAIL.len()); print_u32(BTAIL[0] as u32);
    print_bool(TEXT.as_bytes() == BSL); print_bool(MIDT.as_bytes() == BTAIL); print_bool(MIDT == "world"); print_bool(black_box(TEXT) == "hello world");
    for n in &NN { print_str(n.name); print_usize(n.raw.len()); print_u32(n.raw[1] as u32); }
    print_str(&TEXT[black_box(2)..5]); print_bool(TEXT.starts_with("hell")); print_usize(TEXT.find('w').unwrap_or(99));
    print_bool(core::ptr::eq(BSL.as_ptr(), BYTES.as_ptr())); print_bool(core::ptr::eq(BTAIL.as_ptr(), &BYTES[6]));
}
