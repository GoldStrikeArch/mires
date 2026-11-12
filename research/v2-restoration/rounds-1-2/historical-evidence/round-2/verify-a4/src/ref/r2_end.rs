#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
const ARR: [u32; 3] = [100, 200, 300];
struct SyncEnd(Option<core::ptr::NonNull<u32>>);
unsafe impl Sync for SyncEnd {}
static END: SyncEnd = SyncEnd(Some(unsafe { core::ptr::NonNull::new_unchecked((&ARR as *const [u32; 3]).cast::<u32>().cast_mut().wrapping_add(3)) }));
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(black_box(&END).0.is_some());
}
