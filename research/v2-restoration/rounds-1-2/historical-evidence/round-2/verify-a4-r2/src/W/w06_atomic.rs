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

static CELLS: [AtomicU32; 3] = [AtomicU32::new(1), AtomicU32::new(2), AtomicU32::new(3)];
static SL: &[AtomicU32] = &CELLS;
static TAIL: &[AtomicU32] = CELLS.split_at(1).1;
static OPT: Option<&[AtomicU32]> = Some(&CELLS);
static ONE: AtomicU32 = AtomicU32::new(50);
static ONE_SL: &[AtomicU32] = core::slice::from_ref(&ONE);
struct H { cells: &'static [AtomicU32], tail: Option<&'static [AtomicU32]> }
static HH: H = H { cells: &CELLS, tail: Some(TAIL) };
#[inline(never)] fn total(s: &[AtomicU32]) -> u32 { let mut t = 0; for c in s { t = t * 10 + c.load(Ordering::Relaxed); } t }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(total(SL)); print_u32(total(TAIL));
    SL[0].store(7, Ordering::Relaxed);
    print_u32(CELLS[0].load(Ordering::Relaxed));
    CELLS[2].store(9, Ordering::Relaxed);
    print_u32(total(TAIL)); print_u32(total(black_box(SL)));
    TAIL[0].fetch_add(1, Ordering::Relaxed);
    print_u32(CELLS[1].load(Ordering::Relaxed));
    print_u32(OPT.map_or(0, total));
    black_box(OPT).unwrap()[1].store(4, Ordering::Relaxed);
    print_u32(total(&CELLS)); print_u32(total(HH.cells)); print_u32(HH.tail.map_or(0, total));
    HH.tail.unwrap()[1].store(6, Ordering::Relaxed);
    print_u32(total(&CELLS));
    ONE_SL[0].store(51, Ordering::Relaxed); print_u32(ONE.load(Ordering::Relaxed));
    ONE.store(52, Ordering::Relaxed); print_u32(total(ONE_SL));
    print_bool(core::ptr::eq(&SL[1], &CELLS[1])); print_bool(core::ptr::eq(TAIL.as_ptr(), &CELLS[1])); print_bool(core::ptr::eq(&ONE_SL[0], &ONE));
}
