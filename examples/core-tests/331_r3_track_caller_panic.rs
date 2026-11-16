//! Verifies that inlining `#[track_caller]` `slice::chunks_exact(0)` preserves the exact
//! caller `file`, `line`, and `column` on panic rather than reporting `core`.

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

#[inline(never)]
fn trigger_chunks_zero(data: &[u8]) -> usize {
    let it = data.chunks_exact(0);
    it.len()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let data = [1u8, 2, 3, 4];
    let _ = trigger_chunks_zero(&data);
}
