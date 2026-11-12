#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};

// The static holding the wide pointer is declared first and sorts first by name; its target is
// declared last and sorts last.
static A_SL: &[u32] = &Z_ARR;
static A_OPT: Option<&[u32]> = Some(Z_ARR.split_at(1).1);
struct Holder { data: &'static [u32], cells: &'static [AtomicU32], one: &'static [u32] }
static A_HOLD: Holder = Holder { data: &Z_ARR, cells: &Z_CELLS, one: core::slice::from_ref(&Z_ONE) };
static A_TABLE: [&[u32]; 2] = [&Z_ARR, core::slice::from_ref(&Z_ONE)];
static A_THIN: &[u32; 3] = &Z_ARR;
static A_PT: &[Pt] = core::slice::from_ref(&Z_PT);
#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
static Z_PT: Pt = Pt { x: 3, y: 4 };
static Z_ONE: u32 = 9;
static Z_CELLS: [AtomicU32; 2] = [AtomicU32::new(1), AtomicU32::new(2)];
static Z_ARR: [u32; 3] = [100, 200, 300];

#[inline(never)] fn sum(s: &[u32]) -> u32 { let mut t = 0; for v in s { t = t * 3 + *v; } t }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(A_SL.len()); print_u32(sum(A_SL));
    print_u32(A_OPT.map_or(0, sum)); print_u32(sum(A_HOLD.data)); print_u32(sum(A_HOLD.one));
    A_HOLD.cells[1].store(7, Ordering::Relaxed); print_u32(Z_CELLS[1].load(Ordering::Relaxed));
    print_u32(sum(A_TABLE[black_box(0)])); print_u32(sum(A_TABLE[black_box(1)]));
    print_u32(A_THIN[2]); print_u32(A_PT[0].x + A_PT[0].y);
    print_bool(core::ptr::eq(A_SL.as_ptr(), Z_ARR.as_ptr())); print_bool(core::ptr::eq(&A_PT[0], &Z_PT)); print_bool(core::ptr::eq(&A_HOLD.one[0], &Z_ONE));
}
