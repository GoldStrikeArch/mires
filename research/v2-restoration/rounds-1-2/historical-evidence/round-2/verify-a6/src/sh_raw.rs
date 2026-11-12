#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct R { id: u32, p: *const R, q: *const u32 }
unsafe impl Sync for R {}
static R1: R = R { id: 1, p: &R2 as *const R, q: &VAL as *const u32 };
static R2: R = R { id: 2, p: &R1 as *const R, q: &VAL as *const u32 };
static RS: R = R { id: 3, p: &RS as *const R, q: core::ptr::null() };
static VAL: u32 = 55;
struct SP(*const R);
unsafe impl Sync for SP {}
static SPTR: SP = SP(&R1 as *const R);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        print_u32((*R1.p).id * 10 + (*(*R1.p).p).id);
        print_bool((*R1.p).p == &R1 as *const R);
        print_bool(R1.p == &R2 as *const R);
        print_bool(R1.p == R2.p);
        print_bool(RS.p == &RS as *const R);
        print_bool((*RS.p).p == RS.p);
        print_u32((*(*RS.p).p).id + *R1.q + *(*R2.p).q);
        print_bool(R1.q == R2.q);
        print_bool(RS.q.is_null());
        print_bool(SPTR.0 == R2.p);
        print_u32((*(*SPTR.0).p).id);
    }
}
