//! Real JS host throw across a live `Drop` guard must still run the destructor and preserve exact
//! thrown host value identity in both unwind and abort modes, even when the same crate also has
//! panicking code and a `#[panic_handler]` calling `throw_value`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::rc::Rc;
use core::cell::Cell;

use mires_js::{construct, js_bindings, throw_value, try_host, JsValue};

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
    throw_value(JsValue::erase("panic-boom"))
}

js_bindings! {
    #[js(global, get = "Error")]
    fn error_constructor() -> JsValue;
}

struct DropFlag(Rc<Cell<bool>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[inline(never)]
fn maybe_panic(flag: bool, slice: &[u32]) -> u32 {
    if flag {
        slice[99]
    } else {
        slice[0]
    }
}

#[inline(never)]
fn throw_across_local_guard(value: JsValue, local_dropped: Rc<Cell<bool>>) -> JsValue {
    let _guard = DropFlag(local_dropped);
    throw_value(value)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ok = maybe_panic(false, &[42, 7]) == 42;
    print_str("maybe-panic-ok");
    print_bool(ok);

    // 1. Throwing an Error object across both a closure-captured guard and a stack-local guard.
    let sentinel = construct(error_constructor(), &[JsValue::erase("host-sentinel")]);
    let capture_dropped = Rc::new(Cell::new(false));
    let local_dropped = Rc::new(Cell::new(false));
    let capture_guard = DropFlag(Rc::clone(&capture_dropped));
    let local_ref = Rc::clone(&local_dropped);
    let obj_identity = match try_host(move || {
        let _ = &capture_guard;
        throw_across_local_guard(sentinel, Rc::clone(&local_ref))
    }) {
        Ok(_) => false,
        Err(err) => err.strictly_equals(sentinel),
    };
    let local_ok = if cfg!(panic = "unwind") {
        local_dropped.get()
    } else {
        true
    };
    print_str("obj-throw-ok");
    print_bool(obj_identity && capture_dropped.get() && local_ok);

    // 2. Throwing a primitive string via `throw_value` outside `#[panic_handler]` must preserve
    // exact primitive string identity (NOT wrapped into an AbortError).
    let raw_str = JsValue::erase("host-raw-str");
    let capture_dropped2 = Rc::new(Cell::new(false));
    let local_dropped2 = Rc::new(Cell::new(false));
    let capture_guard2 = DropFlag(Rc::clone(&capture_dropped2));
    let local_ref2 = Rc::clone(&local_dropped2);
    let str_identity = match try_host(move || {
        let _ = &capture_guard2;
        throw_across_local_guard(raw_str, Rc::clone(&local_ref2))
    }) {
        Ok(_) => false,
        Err(err) => err.strictly_equals(raw_str),
    };
    let local_ok2 = if cfg!(panic = "unwind") {
        local_dropped2.get()
    } else {
        true
    };
    print_str("str-throw-ok");
    print_bool(str_identity && capture_dropped2.get() && local_ok2);
}
