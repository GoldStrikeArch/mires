#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
struct N { id: u32 }
static B: N = N { id: 2 };
static ONE: &[N] = core::slice::from_ref(&B);
static P: u32 = 7;
static ONEP: &[u32] = core::slice::from_ref(&P);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(ONE.len() as u32);
    print_u32(ONE[0].id);
    print_bool(core::ptr::eq(&ONE[0], &B));
    print_bool(core::ptr::eq(ONE.as_ptr(), &B));
    print_u32(ONEP.len() as u32);
    print_u32(ONEP[0]);
    print_bool(core::ptr::eq(&ONEP[0], &P));
    print_bool(core::ptr::eq(ONEP.as_ptr(), &P));
    // The same slices built at run time.
    let s = core::slice::from_ref(black_box(&B));
    print_bool(core::ptr::eq(s.as_ptr(), &B));
    let t = core::slice::from_ref(black_box(&P));
    print_bool(core::ptr::eq(t.as_ptr(), &P));
}
