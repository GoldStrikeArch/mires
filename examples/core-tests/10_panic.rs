//! A panic that reaches the `#[panic_handler]` through real `core`.
//!
//! `Option::unwrap` on a `None` calls `core::panicking::panic("called `Option::unwrap()` on a
//! `None` value")`, which builds a `fmt::Arguments` out of that one static string and hands a
//! `PanicInfo` to this crate's handler. Two things are being checked at once:
//!
//! * the `#[track_caller]` chain — `unwrap` is `#[track_caller]`, so the `Location` the handler
//!   prints is *this* file and the line of the `unwrap()` call, not a line inside `core`;
//! * that the program then aborts, which the runner sees as a non-zero exit status.
//!
//! `fmt::Arguments` is *built* here and never read: `PanicInfo::message()` is deliberately not
//! touched, because formatting it would need the whole of `core::fmt`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

fn first(values: &[i32]) -> i32 {
    let found: Option<&i32> = values.first();
    *found.unwrap()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Reached: the slice is not empty.
    print_i32(first(&[7, 8]));

    // Not reached: `unwrap` panics, the handler prints the location of the call inside `first`,
    // and `js_abort` ends the program.
    print_i32(first(&[]));
    print_i32(999);
}
