#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};

// Control for w06/w08: `slice.as_ptr()` against `&array[i]` for aggregate elements, with no
// constant wide pointer anywhere: a local array, and a static reached only at run time.
#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
static CELLS: [AtomicU32; 3] = [AtomicU32::new(1), AtomicU32::new(2), AtomicU32::new(3)];
static PTS: [Pt; 3] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }, Pt { x: 5, y: 6 }];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let local = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }, Pt { x: 5, y: 6 }];
    let s: &[Pt] = &local[black_box(1)..];
    print_bool(core::ptr::eq(s.as_ptr(), &local[1]));
    print_bool(core::ptr::eq(&s[0], &local[1]));
    print_bool(core::ptr::eq(local.as_ptr(), &local[0]));
    let t: &[AtomicU32] = &CELLS[black_box(1)..];
    print_bool(core::ptr::eq(t.as_ptr(), &CELLS[1]));
    print_bool(core::ptr::eq(&t[0], &CELLS[1]));
    print_bool(core::ptr::eq(CELLS.as_ptr(), &CELLS[0]));
    let u: &[Pt] = &PTS[black_box(1)..];
    print_bool(core::ptr::eq(u.as_ptr(), &PTS[1]));
    print_bool(core::ptr::eq(&u[0], &PTS[1]));
    let whole: &[Pt] = &PTS;
    print_bool(core::ptr::eq(whole.as_ptr(), PTS.as_ptr()));
    print_bool(core::ptr::eq(whole[1..].as_ptr(), u.as_ptr()));
}
