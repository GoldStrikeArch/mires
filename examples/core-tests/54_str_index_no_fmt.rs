//! Indexing and `contains` on `&str` without `format!` or `Debug`.
//!
//! `&s[a..b]` and `str::contains` reach `str::slice_error_fail`. The sysroot
//! patch makes that panic a constant string, so this program must not pull
//! `fmt::write`, `char::escape_debug`, or Unicode tables. Companion
//! `54_str_index_no_fmt.absent` is the size receipt.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ascii = "hello";
    print_str(&ascii[1..4]);
    print_str(&ascii[..2]);
    print_str(&ascii[3..]);
    print_usize(ascii[1..3].len());
    print_bool(ascii.contains("ell"));
    print_bool(ascii.contains("zz"));

    let accented = "héllo";
    print_str(&accented[0..3]);
    print_bool(accented.contains("é"));
    print_usize(accented.len());
}
