//! The minimal format cluster the wave-1 registry lowers natively (Phase 4, D10).
//!
//! Every default-Display shape `FormatArgumentNewDisplay` proves — fixed-width integers, `bool`,
//! `char`, `&str`, `String`, and shortest-`f64` — flowing through `format!` and `ToString`, which
//! is exactly the `FormatArgsNew`/`FormatArgsToString`/`IntToString`/`BoolToString`/
//! `CharToString`/`F64SpecToString` operation set. Stdout is mode-independent; the v1 receipts
//! this cluster must eventually reproduce on the lane are `native-format-args-test.sh`
//! (275,291 → 3,474 B) and `native-host-string-test.sh` (24,964 → 5,992 B).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use alloc::string::{String, ToString};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // `ToString` for the primitives.
    print_str(&42i32.to_string());
    print_str(&(-7i64).to_string());
    print_str(&255u8.to_string());
    print_str(&true.to_string());
    print_str(&false.to_string());
    print_str(&'Z'.to_string());
    print_str(&'π'.to_string());

    // Exact shortest-`f64` text.
    print_str(&1.5f64.to_string());
    print_str(&0.1f64.to_string());
    print_str(&(-0.0f64).to_string());

    // `format!` over each display shape, with static pieces between the holes.
    let owned = String::from("owned");
    let n = 6;
    let f = 2.5f64;
    print_str(&format!("n={n} f={f} b={} c={} s={} o={owned}", false, 'y', "str"));
    print_str(&format!("{}", 123u16));
    print_str(&format!("no holes"));

    // A formatted result is an ordinary String: length and append still hold.
    let mut text = format!("{}+{}", 6, 7);
    text.push('!');
    print_str(&text);
    print_usize(text.len());
}
