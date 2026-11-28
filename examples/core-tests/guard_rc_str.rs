//! The boundary of `Rc::<str>::from_raw`: `Rc::<str>::from` now works directly over `[u8]` tail
//! blocks (see `433_str_tail_dst.rs`), while `Rc::<str>::from_raw` on a bare `*const str` still
//! refuses loudly at compile time because a JS string carries no enclosing `RcInner` block to step
//! backwards into via `byte_sub(data_offset)`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::rc::Rc;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let raw: *const str = "statue";
    let shared: Rc<str> = unsafe { Rc::from_raw(raw) };
    print_usize(shared.len());
}
