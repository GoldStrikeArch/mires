//! Under `js-str=bytes`, a `str` behind an unsafe raw `*const`/`*mut` pointer outside
//! `Box`/`Rc`/`Arc`/`RawVec` cannot cross safely, so the link step must refuse it
//! (`scripts/bstr-probes.sh` expects the compile to fail with the reason).

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

js_bindings! {
    #[js(call = "globalThis.bstrRawPtr")]
    fn raw_ptr(value: *const (&'static str, u32));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let pair = ("é", 1u32);
    raw_ptr(&pair as *const _);
}
