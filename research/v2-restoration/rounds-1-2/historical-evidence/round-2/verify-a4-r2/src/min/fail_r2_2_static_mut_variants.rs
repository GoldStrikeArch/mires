#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

// Neighbours of fail_r2_1: the same one-element constant slice over an aggregate `static mut`,
// through a raw wide pointer, inside Option, a tuple element type, an enum element type, with
// `swap`/`copy_from_slice`/`fill`/`iter_mut`, and the array control (which is right).
#[derive(Clone, Copy)]
struct Pt { x: u32, y: u32 }
static mut A: Pt = Pt { x: 1, y: 2 };
struct Raw(*mut [Pt]);
unsafe impl Sync for Raw {}
static A_RAW: Raw = Raw(unsafe { core::slice::from_mut(&mut *(&raw mut A)) } as *mut [Pt]);

static mut B: (u32, u8) = (1, 2);
static mut B_SL: Option<&mut [(u32, u8)]> = Some(unsafe { core::slice::from_mut(&mut *(&raw mut B)) });

static mut C: Option<u64> = Some(5);
static mut C_SL: &mut [Option<u64>] = unsafe { core::slice::from_mut(&mut *(&raw mut C)) };

static mut D: Pt = Pt { x: 1, y: 2 };
static mut D_SL: &mut [Pt] = unsafe { core::slice::from_mut(&mut *(&raw mut D)) };

static mut ARR: [Pt; 2] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }];
static mut ARR_SL: &mut [Pt] = unsafe { &mut *(&raw mut ARR) };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        (*A_RAW.0)[0] = Pt { x: 7, y: 8 };
        print_u32(A.x + A.y);
        if let Some(s) = &mut *(&raw mut B_SL) { s[black_box(0)] = (40, 2); }
        print_u32(B.0 + B.1 as u32);
        (*(&raw mut C_SL))[0] = None;
        print_bool(C.is_none());
        C = Some(9);
        print_u64((*(&raw const C_SL))[0].unwrap_or(0));
        (*(&raw mut D_SL)).fill(Pt { x: 50, y: 5 });
        print_u32(D.x + D.y);
        (*(&raw mut D_SL)).copy_from_slice(&[Pt { x: 60, y: 6 }]);
        print_u32(D.x + D.y);
        for p in (*(&raw mut D_SL)).iter_mut() { *p = Pt { x: 70, y: 7 }; }
        print_u32(D.x + D.y);
        (*(&raw mut ARR_SL))[1] = Pt { x: 80, y: 8 };
        print_u32(ARR[1].x + ARR[1].y);
        ARR = [Pt { x: 9, y: 9 }, Pt { x: 90, y: 9 }];
        print_u32((*(&raw const ARR_SL))[1].x + (*(&raw const ARR_SL))[1].y);
        (*(&raw mut ARR_SL)).swap(0, 1);
        print_u32(ARR[0].x);
    }
}
