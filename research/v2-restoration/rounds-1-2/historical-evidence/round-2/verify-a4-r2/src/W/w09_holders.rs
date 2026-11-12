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
static TAIL: &[u32] = ARR.split_at(2).1;
enum En { Empty, Sl(&'static [u32]), Pair(&'static [u32], u32) }
static ENS: [En; 3] = [En::Sl(&ARR), En::Empty, En::Pair(TAIL, 5)];
static TUP: (&[u32], Option<&[u32]>, u8) = (TAIL, Some(&ARR), 3);
static NEST: &[&[u32]] = &[&ARR, TAIL, &[]];
static ONEST: Option<&[Option<&[u32]>]> = Some(&[Some(&ARR), None, Some(TAIL)]);
static RES: Result<&[u32], &str> = Ok(TAIL);
static RSL: &&[u32] = &TAIL;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for e in &ENS { match e { En::Empty => print_str("empty"), En::Sl(s) => show(s), En::Pair(s, n) => { show(s); print_u32(*n); } } }
    show(TUP.0); showo(TUP.1); print_u32(TUP.2 as u32);
    for s in NEST { show(s); }
    for o in ONEST.unwrap() { showo(*o); }
    match RES { Ok(s) => show(s), Err(e) => print_str(e) }
    show(*RSL); show(black_box(*RSL));
    print_bool(core::ptr::eq(NEST[1], TAIL)); print_bool(core::ptr::eq(NEST[0].as_ptr(), ARR.as_ptr()));
}
