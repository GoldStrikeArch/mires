//! Event-style host access: a listener closure installed on an opaque host object, which reads
//! properties of the event it is handed and calls methods on it.
//!
//! The pieces are pinned elsewhere one at a time (examples/host_operations.rs for property reads
//! and method calls, examples/extern-tests for `#[js_extern]` shapes, the callback fixtures for
//! closures handed to the host). The combination was pinned only by the DOM suite's
//! `21_event_accessors`, which needs the framework's `view_dom_macro`; this fixture is that
//! combination without the framework. scripts/host-event-access-check.mjs drives it with fake
//! elements and events and asserts what each listener did.
//!
//! The guarded listener holds a value with a `Drop` across a host method that may throw. Under
//! `js-cleanup=on` the cleanup path must drop it exactly once and let the host's exception
//! through unchanged.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

use js_extern_macro::js_extern;
use mires_js::{callback1, js_export, throw_value, JsValue};

#[js_extern]
unsafe extern "C" {
    /// `element.addEventListener(kind, listener)`, the listener a Rust closure.
    #[js(method = "addEventListener")]
    fn add_event_listener(element: JsValue, kind: &str, listener: JsValue);

    /// `event.target`: the element the event came from, not the one the listener sits on.
    #[js(get = "target")]
    fn target(event: JsValue) -> JsValue;

    #[js(get = "value")]
    fn value(element: JsValue) -> JsValue;

    #[js(method = "preventDefault")]
    fn prevent_default(event: JsValue);

    /// The check's log: an array the listeners append to.
    #[js(method = "push")]
    fn push(log: JsValue, entry: JsValue);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("host event access panic"))
}

/// What the user typed: `event.target.value`, or `""` when the element has no string `value` (a
/// `<div>` the event bubbled from), so a reader always gets a string.
fn target_value(event: JsValue) -> JsValue {
    let text = value(target(event));
    if text.is_string() {
        text
    } else {
        JsValue::erase("")
    }
}

/// Appends `name` to `log` when it goes out of scope, however it goes.
struct Guard {
    log: JsValue,
    name: &'static str,
}

impl Drop for Guard {
    fn drop(&mut self) {
        push(self.log, JsValue::erase(self.name));
    }
}

/// An `input` listener: report what was typed, then cancel the default action.
#[js_export]
pub fn listen_input(element: JsValue, log: JsValue) {
    add_event_listener(
        element,
        "input",
        callback1(move |event| {
            push(log, target_value(event));
            prevent_default(event);
            JsValue::undefined()
        }),
    );
}

/// A `submit` listener holding a guard across `preventDefault`, which the check makes throw on
/// some dispatches. The entry after the call is written only when the call returns.
#[js_export]
pub fn listen_guarded(element: JsValue, log: JsValue) {
    add_event_listener(
        element,
        "submit",
        callback1(move |event| {
            let _guard = Guard {
                log,
                name: "guard dropped",
            };
            push(log, JsValue::erase("before preventDefault"));
            prevent_default(event);
            push(log, target_value(event));
            JsValue::undefined()
        }),
    );
}
