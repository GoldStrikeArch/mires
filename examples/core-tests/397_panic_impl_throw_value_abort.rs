//! `#[panic_handler]` (`LangItem::PanicImpl`) calling `mires_js::throw_value(JsValue::erase("boom"))`
//! must abort WITHOUT running live Rust `Drop` guards (`CONTRACT.md`), wrap primitive throws as a
//! registered `AbortError`, and allow `prune_invoke` to prune caller `Invoke` edges under
//! `RCGJS_WITNESS_CHECK=1`. Also exercises `mires_js::abort`.

#![no_std]
#![no_main]

use mires_js::{abort, js_bindings, throw_value, try_host, JsValue};

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_log_bool(b: bool);
}

fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}

fn print_bool(b: bool) {
    unsafe { js_log_bool(b) }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("boom"))
}

js_bindings! {
    #[js(get = "message")]
    fn error_message(error: JsValue) -> JsValue;
    #[js(call = "__rt.is_abort")]
    fn rt_is_abort(error: JsValue) -> bool;
}

struct Guard(&'static str);

impl Drop for Guard {
    fn drop(&mut self) {
        print_str(self.0);
    }
}

#[inline(never)]
fn panic_with_live_guard(slice: &[u32]) -> u32 {
    let _guard = Guard("must-not-drop-panic");
    slice[10]
}

#[inline(never)]
fn abort_with_live_guard() -> ! {
    let _guard = Guard("must-not-drop-abort");
    abort("direct-abort")
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let panic_res = try_host(|| {
        let _ = panic_with_live_guard(&[1, 2]);
        JsValue::undefined()
    });
    let panic_ok = match panic_res {
        Ok(_) => false,
        Err(err) => {
            err.is_error()
                && error_message(err).strictly_equals(JsValue::erase("boom"))
                && rt_is_abort(err)
        }
    };
    print_str("panic-ok");
    print_bool(panic_ok);

    let abort_res = try_host(|| abort_with_live_guard());
    let abort_ok = match abort_res {
        Ok(_) => false,
        Err(err) => {
            err.is_error()
                && error_message(err).strictly_equals(JsValue::erase("rust abort: direct-abort"))
                && rt_is_abort(err)
        }
    };
    print_str("abort-ok");
    print_bool(abort_ok);

    print_str("final-abort");
    let _ = panic_with_live_guard(&[1, 2]);
}
