//! Shared header for the `core` test suite: the print shim, and a panic handler that never
//! formats.
//!
//! Every test in this directory is an ordinary `#![no_std]` crate compiled against the real
//! `core` that `scripts/build_sysroot.sh` built with this backend — no `mini_core`, no `no_core`,
//! no lang items of its own.
//!
//! # The panic handler
//!
//! `#[panic_handler]` is the one item a `no_std` binary owes `core`. This one reads
//! `PanicInfo::location()`, which works: the location is a `&Location` the caller passed through
//! the `#[track_caller]` chain, and its `file`/`line`/`col` accessors are plain field reads.
//!
//! It deliberately does **not** touch `PanicInfo::message()`. That is a `fmt::Arguments`, and
//! formatting one means walking a byte-packed format-string template through raw pointer
//! arithmetic, which has no meaning in a model where a value is a JavaScript object rather than
//! bytes at an address. `fmt::Arguments` is *built* on the panic path — that has to compile, and
//! does — but it is never read.

#![allow(dead_code)]

unsafe extern "C" {
    pub fn js_log_i32(x: i32);
    pub fn js_log_i64(x: i64);
    pub fn js_log_f64(x: f64);
    pub fn js_log_bool(x: bool);
    pub fn js_log_str(s: &str);
    pub fn js_abort(msg: &str) -> !;
}

pub fn print_i32(x: i32) {
    unsafe { js_log_i32(x) }
}

/// An unsigned 32 bit value goes out through the `f64` shim: every `u32` is exact in a double, and
/// `js_log_i32` would print `u32::MAX` as `-1`.
pub fn print_u32(x: u32) {
    unsafe { js_log_f64(x as f64) }
}

pub fn print_usize(x: usize) {
    unsafe { js_log_f64(x as f64) }
}

pub fn print_i64(x: i64) {
    unsafe { js_log_i64(x) }
}

/// A `u64` does *not* fit a double, so it goes out as the `i64` with the same bits: `u64::MAX`
/// prints as `-1`. The native transplant used to validate the expectations does the same cast.
pub fn print_u64(x: u64) {
    unsafe { js_log_i64(x as i64) }
}

pub fn print_f64(x: f64) {
    unsafe { js_log_f64(x) }
}

pub fn print_bool(x: bool) {
    unsafe { js_log_bool(x) }
}

pub fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}

/// `Ordering` has no `Debug` we may use, so it is printed as the number `three_way_compare`
/// produces: `-1`, `0`, `1`.
pub fn print_ordering(o: core::cmp::Ordering) {
    print_i32(o as i32)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        print_str(location.file());
        print_i32(location.line() as i32);
    }
    unsafe { js_abort("panic") }
}

// ---------------------------------------------------------------------------
// The string sink
// ---------------------------------------------------------------------------
//
// A `fmt::Write` sink that builds its string on the JavaScript side rather than
// in the heap. `write!` into one of these spends a host string concatenation per
// piece instead of a `Vec<u8>` push per byte, and what comes back is already the
// JavaScript string every other shim member takes.
//
// The handle is a `usize` index into a table the shim keeps, so the whole
// surface is `extern "C"`-shaped. See `CONTRACT.md`, "Allocation".

unsafe extern "C" {
    pub fn sb_new() -> usize;
    pub fn sb_push(handle: usize, s: &str);
    pub fn sb_push_char(handle: usize, c: char);
    pub fn sb_take(handle: usize) -> &'static str;
}

/// A `core::fmt::Write` sink backed by a host string builder.
pub struct JsStr(usize);

impl JsStr {
    pub fn new() -> JsStr {
        JsStr(unsafe { sb_new() })
    }

    /// The string written so far. The sink is empty again afterwards.
    ///
    /// The lifetime is a convenient fiction: a JavaScript string is a value, so what comes back is
    /// owned by nobody and outlives everything.
    pub fn take(&mut self) -> &'static str {
        unsafe { sb_take(self.0) }
    }
}

impl core::fmt::Write for JsStr {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        unsafe { sb_push(self.0, s) };
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        unsafe { sb_push_char(self.0, c) };
        Ok(())
    }
}
