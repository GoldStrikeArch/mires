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
static WHOLE: &[u32] = &ARR;
static PRE: &[u32] = ARR.split_at(2).0;
static SUF: &[u32] = ARR.split_at(1).1;
static MID: &[u32] = ARR.split_at(1).1.split_at(2).0;
static E0: &[u32] = ARR.split_at(0).0;
static E2: &[u32] = ARR.split_at(2).1.split_at(0).0;
static E4: &[u32] = ARR.split_at(4).1;
static ONE: &[u32] = core::slice::from_ref(&ARR[2]);
static VIA: &[u32; 4] = &ARR;
static VIA_SL: &[u32] = VIA;
static AS_SL: &[u32] = ARR.as_slice();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    show(WHOLE); show(PRE); show(SUF); show(MID); show(E0); show(E2); show(E4); show(ONE);
    show(black_box(WHOLE)); show(black_box(MID)); show(black_box(E4));
    show(VIA_SL); show(AS_SL); print_u32(VIA[3]);
    print_bool(core::ptr::eq(WHOLE.as_ptr(), ARR.as_ptr()));
    print_bool(core::ptr::eq(WHOLE.as_ptr(), &ARR[0]));
    print_bool(core::ptr::eq(SUF.as_ptr(), &ARR[1]));
    print_bool(core::ptr::eq(MID.as_ptr(), SUF.as_ptr()));
    print_bool(core::ptr::eq(MID, &ARR[1..3]));
    print_bool(core::ptr::eq(MID, SUF));
    print_bool(core::ptr::eq(WHOLE, &ARR[..]));
    print_bool(core::ptr::eq(WHOLE, AS_SL));
    print_bool(core::ptr::eq(&WHOLE[2], &ARR[2]));
    print_bool(core::ptr::eq(&WHOLE[2], &MID[1]));
    print_bool(core::ptr::eq(ONE.as_ptr(), &ARR[2]));
    print_bool(core::ptr::eq(E4.as_ptr(), ARR.as_ptr_range().end));
    print_bool(core::ptr::eq(E0.as_ptr(), ARR.as_ptr()));
    print_bool(core::ptr::eq(WHOLE[1..].as_ptr(), SUF.as_ptr()));
    print_bool(core::ptr::eq(VIA, &ARR));
    print_usize(unsafe { SUF.as_ptr().offset_from(ARR.as_ptr()) } as usize);
    print_usize(unsafe { E4.as_ptr().offset_from(WHOLE.as_ptr()) } as usize);
    let rt: &[u32] = &ARR[black_box(1)..];
    print_bool(core::ptr::eq(rt, SUF)); print_bool(rt == SUF); print_bool(WHOLE == &ARR[..]);
    let mut it = SUF.iter(); print_u32(*it.next().unwrap()); print_usize(it.len());
    print_u32(*SUF.last().unwrap()); print_u32(SUF.iter().rev().fold(0, |a, b| a * 2 + *b));
    print_bool(SUF.contains(&40)); print_bool(PRE.contains(&40));
    let (a, b) = WHOLE.split_at(black_box(2)); show(a); show(b);
    print_bool(E0.is_empty()); print_bool(E4.first().is_none()); print_bool(WHOLE.get(4).is_none()); print_u32(*WHOLE.get(3).unwrap());
}
