//! Under `js-str=bytes`, a raw coroutine holding `str` captured inside `js!` cannot cross
//! the host boundary safely, so the link step must refuse it.

#![no_std]
#![no_main]
#![feature(coroutines, stmt_expr_attributes)]

use mires_js::JsValue;

unsafe extern "C-unwind" {
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { js_abort("panic") }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let s = "é";
    let g = #[coroutine]
    move || {
        yield s.len();
    };
    let _ = core::mem::size_of_val(&g);
    let _: JsValue = js_macro::js! { globalThis.String(g) };
}
