//! Round 2: a reinterpreting view reachable in SAFE Rust. `<[[u64; 2]]>::as_flattened` is a safe,
//! stable `core` method; this crate contains no `unsafe` except the host declarations. Native
//! Rust prints `7`, `dropped`, `7` for `total` and `3`, `dropped`, `7` for `ones`.
//!
//! In this backend the flattened view covers the first row only (`__rt.unwindow` steps into one
//! element), so `words[2]` and `words[3]` are `undefined` although their type is `u64`. `total` uses only `+` (a `Binary`, quiet since Stage 3b);
//! `ones` uses `count_ones` (the `ctpop` intrinsic, quiet since Stage 4'a).
#![no_std]
#![no_main]
#![forbid(unsafe_op_in_unsafe_fn)]

unsafe extern "C" {
    safe fn js_log_i32(x: i32);
    safe fn js_log_str(s: &str);
    safe fn probe_flag() -> i32;
}

struct Guard(i32);
impl Drop for Guard {
    fn drop(&mut self) {
        js_log_str("dropped");
        js_log_i32(self.0);
    }
}

#[inline(never)]
fn total(words: &[u64]) -> u64 {
    words[2] + words[3]
}

#[inline(never)]
fn ones(words: &[u64]) -> u32 {
    words[2].count_ones() + words[3].count_ones()
}

#[inline(never)]
fn check(rows: &[[u64; 2]], use_ones: bool) {
    let guard = Guard(7);
    let flat = rows.as_flattened();
    let r = if use_ones { ones(flat) as i32 } else { total(flat) as i32 };
    js_log_i32(r);
    drop(guard);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let rows = [[1u64, 2], [3, 4]];
    check(&rows, probe_flag() == 1);
}
