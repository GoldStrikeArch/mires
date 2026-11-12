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

#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
static mut ONE: Pt = Pt { x: 1, y: 2 };
static mut ONE_SL: &mut [Pt] = unsafe { core::slice::from_mut(&mut *(&raw mut ONE)) };
static mut V: u32 = 5;
static mut V_SL: &mut [u32] = unsafe { core::slice::from_mut(&mut *(&raw mut V)) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        (*(&raw mut ONE_SL))[0].x = 10; print_u32(ONE.x);
        (*(&raw mut ONE_SL))[0] = Pt { x: 7, y: 8 }; print_u32(ONE.x + ONE.y);
        ONE = Pt { x: 20, y: 30 }; print_u32((*(&raw const ONE_SL))[0].x);
        (*(&raw mut V_SL))[0] = 6; print_u32(V);
        V = 9; print_u32((*(&raw const V_SL))[0]);
        print_usize((*(&raw const V_SL)).len());
    }
}
