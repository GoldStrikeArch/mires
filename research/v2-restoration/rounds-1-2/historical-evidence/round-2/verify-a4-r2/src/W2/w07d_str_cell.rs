#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::UnsafeCell;
use core::hint::black_box;
struct Shared(UnsafeCell<[u8; 5]>);
unsafe impl Sync for Shared {}
static BYTES: Shared = Shared(UnsafeCell::new(*b"hello"));
struct Text(*const str);
unsafe impl Sync for Text {}
static TEXT: Text = Text(unsafe { core::str::from_utf8_unchecked(&*BYTES.0.get()) } as *const str);
static SL: Text2 = Text2(unsafe { &*BYTES.0.get() } as *const [u8; 5] as *const [u8]);
struct Text2(*const [u8]);
unsafe impl Sync for Text2 {}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        print_str(&*TEXT.0);
        print_usize((&*SL.0).len());
        print_u32((&*SL.0)[0] as u32);
        (*BYTES.0.get())[0] = b'j';
        print_str(&*TEXT.0);
        print_u32((&*SL.0)[0] as u32);
    }
}
