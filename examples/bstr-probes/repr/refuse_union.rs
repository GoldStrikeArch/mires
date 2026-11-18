//! Under `js-str=bytes`, a multi-field untagged `union` holding `str` cannot be converted
//! without a discriminant, so the link step must refuse it.

#![no_std]
#![no_main]

use mires_js::js_bindings;

unsafe extern "C-unwind" {
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { js_abort("panic") }
}

#[derive(Clone, Copy)]
pub union U {
    pub a: &'static str,
    pub n: u32,
}

js_bindings! {
    #[js(call = "globalThis.bstrSee")]
    fn see_union(value: U);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    see_union(U { a: "é" });
}
