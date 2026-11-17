//! Under `js-str=bytes` a `str` the host could write back through a mutable pointer cannot be
//! converted exactly, so the link step must refuse it (`scripts/bstr-probes.sh` expects the
//! compile to fail with the reason).

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
    #[js(call = "globalThis.bstrRewrite")]
    fn rewrite(value: &mut (&'static str, u32));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut pair = ("é", 1);
    rewrite(&mut pair);
}
