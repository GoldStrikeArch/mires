#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

// Controls for w13: the same whole-element writes with no wide pointer constant.
#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
// (a) a thin `&mut Pt` constant into a `static mut`
static mut ONE: Pt = Pt { x: 1, y: 2 };
static mut R: &mut Pt = unsafe { &mut *(&raw mut ONE) };
// (b) a one-element slice of a `static mut` built at run time
static mut TWO: Pt = Pt { x: 1, y: 2 };
// (c) a slice over a `static mut` array, built at run time
static mut ARR: [Pt; 2] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        (*(&raw mut R)).x = 10; print_u32(ONE.x);
        **(&raw mut R) = Pt { x: 7, y: 8 }; print_u32(ONE.x + ONE.y);
        ONE = Pt { x: 20, y: 30 }; print_u32((*(&raw const R)).x);

        let s: &mut [Pt] = core::slice::from_mut(&mut *(&raw mut TWO));
        s[0].x = 10; print_u32(TWO.x);
        s[black_box(0)] = Pt { x: 7, y: 8 }; print_u32(TWO.x + TWO.y);
        let s2: &mut [Pt] = core::slice::from_mut(&mut *(&raw mut TWO));
        TWO = Pt { x: 20, y: 30 }; print_u32((*(s2 as *mut [Pt]))[0].x);

        let a: &mut [Pt] = &mut *(&raw mut ARR);
        a[black_box(1)] = Pt { x: 7, y: 8 }; print_u32(ARR[1].x + ARR[1].y);
        let a2: *mut [Pt] = &raw mut ARR;
        ARR = [Pt { x: 20, y: 30 }, Pt { x: 40, y: 50 }]; print_u32((*a2)[1].x);
        ARR[0] = Pt { x: 60, y: 70 }; print_u32((*a2)[0].x);

        let mut local = Pt { x: 1, y: 2 };
        let ls: &mut [Pt] = core::slice::from_mut(&mut local);
        ls[black_box(0)] = Pt { x: 7, y: 8 };
        print_u32(local.x + local.y);
    }
}
