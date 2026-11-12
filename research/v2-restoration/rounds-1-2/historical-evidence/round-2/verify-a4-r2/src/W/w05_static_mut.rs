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

static mut BUF: [u32; 4] = [1, 2, 3, 4];
static mut SLM: &mut [u32] = unsafe { &mut *(&raw mut BUF) };
static mut TAILM: &mut [u32] = unsafe { core::slice::from_raw_parts_mut((&raw mut BUF as *mut u32).add(2), 2) };
struct P(*const [u32]);
unsafe impl Sync for P {}
static RAW: P = P(&raw const BUF as *const [u32; 4] as *const [u32]);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        show(&*RAW.0);
        BUF[1] = 20;
        show(&*RAW.0);
        show(&*(&raw const *SLM));
        (*(&raw mut SLM))[0] = 100;
        print_u32(BUF[0]);
        (*(&raw mut TAILM))[1] = 400;
        print_u32(BUF[3]);
        BUF[2] = 33;
        print_u32((*(&raw const TAILM))[0]);
        BUF = [7, 8, 9, 10];
        show(&*RAW.0);
        print_u32((*(&raw const TAILM))[0]);
        print_bool(core::ptr::eq(RAW.0 as *const u32, &raw const BUF as *const u32));
        print_bool(core::ptr::eq((*(&raw const TAILM)).as_ptr(), (&raw const BUF as *const u32).add(2)));
        print_usize((*RAW.0).len());
    }
}
