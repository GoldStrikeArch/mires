//! Format shapes the native `format_args!` lowering must not mistake for its own.
//!
//! `format_chain` feeds a real producer call into a template: the `to_string` call must keep
//! its emission. `format_facade` passes a proven `Arguments` value to a user function: the call
//! must not be rewritten as `alloc::fmt::format`. `format_write` and `format_write_twice` are
//! `write!` sinks; two sites in one function must not collide on a shared helper binding.
//! Every function must produce identical results in `off`, `report`, and `on`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use core::fmt::Write;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn format_chain(x: u32) -> String {
    alloc::format!("<{}>", x.to_string())
}

#[no_mangle]
pub fn format_facade(x: u32) -> u32 {
    tally(core::format_args!("{x}"))
}

/// A unary `Arguments` consumer that is not `alloc::fmt::format`.
#[inline(never)]
fn tally(args: core::fmt::Arguments) -> u32 {
    if args.as_str().is_some() { 1 } else { 2 }
}

#[no_mangle]
pub fn format_write(x: u32) -> String {
    let mut out = String::new();
    let _ = write!(out, "x={x}");
    out
}

#[no_mangle]
pub fn format_write_twice(x: u32, y: f64) -> String {
    let mut out = String::new();
    let _ = write!(out, "x={x}");
    let _ = write!(out, ",y={y}");
    out
}

#[no_mangle]
pub fn format_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn rust_entry() {}
