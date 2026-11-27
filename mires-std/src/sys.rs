//! Bridge to the JavaScript runtime shim (`runtime/shim.js`).
//!
//! All foreign declarations use the `"C-unwind"` ABI so under `-Cpanic=unwind`
//! host exceptions thrown by `js_panic_error` or inside `js_catch_unwind`
//! unwind cleanly through Rust `Drop` frames without triggering unwind-check
//! errors.

/// Return object produced by `__rt.js_catch_unwind(fn, arg)`.
///
/// Two non-ZST fields guarantee `mires` represents this as a JS object
/// `{ ok, value }` rather than peeling it as a single-field transparent wrapper.
#[repr(C)]
pub(crate) struct JsCatchResult {
    pub(crate) ok: bool,
    pub(crate) value: i32,
}

#[allow(improper_ctypes)]
unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_eprint_str(s: &str);
    fn js_abort(msg: &str) -> !;
    fn js_panic_error(msg: &str) -> !;
    fn js_now_ms() -> f64;
    fn js_epoch_ms() -> f64;
    fn js_catch_unwind(f: extern "C-unwind" fn(*mut u8) -> i32, arg: *mut u8) -> JsCatchResult;
}

#[inline]
pub(crate) fn log_str(s: &str) {
    // SAFETY: `__rt.js_log_str` accepts a JS string and prints it via `console.log`.
    unsafe { js_log_str(s) }
}

#[inline]
pub(crate) fn eprint_str(s: &str) {
    // SAFETY: `__rt.js_eprint_str` accepts a JS string and prints it via `console.error`.
    unsafe { js_eprint_str(s) }
}

#[inline]
pub(crate) fn abort(msg: &str) -> ! {
    // SAFETY: `__rt.js_abort` throws a non-catchable abort error and never returns.
    unsafe { js_abort(msg) }
}

#[inline]
pub(crate) fn panic_error(msg: &str) -> ! {
    // SAFETY: `__rt.js_panic_error` throws a catchable JS Error and never returns.
    unsafe { js_panic_error(msg) }
}

#[inline]
pub(crate) fn now_ms() -> f64 {
    // SAFETY: `__rt.js_now_ms` returns a monotonic JS number in milliseconds.
    unsafe { js_now_ms() }
}

#[inline]
pub(crate) fn epoch_ms() -> f64 {
    // SAFETY: `__rt.js_epoch_ms` returns Unix epoch milliseconds as a JS number.
    unsafe { js_epoch_ms() }
}

#[inline]
pub(crate) fn catch_unwind_raw(
    f: extern "C-unwind" fn(*mut u8) -> i32,
    arg: *mut u8,
) -> JsCatchResult {
    // SAFETY: `__rt.js_catch_unwind` invokes `f(arg)` inside JS `try/catch`,
    // rethrowing abort errors and returning `{ ok: true, value }` or `{ ok: false, error }`.
    unsafe { js_catch_unwind(f, arg) }
}
