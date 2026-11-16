//! Verifies that `#[track_caller]` callees containing surviving non-optional asserts (`div`,
//! `BoundsCheck`, and forwarded `#[track_caller]`) decline inlining cleanly so `emit.rs` preserves
//! the `#[panic_handler]` invocation and exact caller `file`, `line`, and `column`.

#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_log_i32(x: i32);
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(loc) = info.location() {
        unsafe {
            js_log_str(loc.file());
            js_log_i32(loc.line() as i32);
            js_log_i32(loc.column() as i32);
        }
    }
    unsafe { js_abort("panic") }
}

#[track_caller]
fn div_tc(a: u32, b: u32) -> u32 {
    a / b
}

#[track_caller]
fn idx_tc(s: &[u32], i: usize) -> u32 {
    s[i]
}

#[track_caller]
fn outer_tc(b: u32) -> u32 {
    div_tc(100, b)
}

#[inline(never)]
fn caller_div() -> u32 {
    div_tc(100, 0)
}

#[inline(never)]
fn caller_idx() -> u32 {
    let arr = [10u32, 20, 30];
    idx_tc(&arr, 9)
}

#[inline(never)]
fn caller_outer() -> u32 {
    outer_tc(0)
}

#[unsafe(no_mangle)]
pub fn rust_entry(mode: i32) {
    let v = match mode {
        0 => caller_div(),
        1 => caller_idx(),
        _ => caller_outer(),
    };
    unsafe { js_log_i32(v as i32) };
}
