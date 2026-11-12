#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A constant `&mut [T]` over ONE aggregate `static mut` (what `slice::from_mut` makes of it).
// The round-2 emission is `{ buf: [ONE], off: 0, len: 1 }`: a fresh JavaScript array holding the
// static's object, so storing a whole element through the slice replaces the array's slot and
// never reaches the static. Native: 15 20. Round 2: 3 7 (silently). Main: aborts at run time
// (the slice had no length, so the bounds check panics).
#[derive(Clone, Copy)]
struct Pt { x: u32, y: u32 }
static mut ONE: Pt = Pt { x: 1, y: 2 };
static mut ONE_SL: &mut [Pt] = unsafe { core::slice::from_mut(&mut *(&raw mut ONE)) };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        (*(&raw mut ONE_SL))[0] = Pt { x: 7, y: 8 };
        print_u32(ONE.x + ONE.y);
        ONE = Pt { x: 20, y: 30 };
        print_u32((*(&raw const ONE_SL))[0].x);
    }
}
