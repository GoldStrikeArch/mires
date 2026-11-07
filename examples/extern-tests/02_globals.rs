//! `#[js_extern]` rooted at the global scope: what the host already has, imported from nothing.
//!
//! `01_chart.rs` covers the two roots a module gives, an argument and an imported binding. This
//! covers the third. The shapes are the dashboard's: a browser global constructed with `new`, a
//! method on what it returned, and properties read and written through a global path.
//!
//! # Which declarations need the flag, and why only those
//!
//! A `call` and a `new` have no receiver, so with no module they are already global and say
//! nothing extra. Every member shape does have one: a `get` with no module reads a property of
//! argument zero, which is right for `chart.data` and wrong for `dash.status`. `#[js(global)]` is
//! how the second says so, and it is what makes the third root reachable at all.
//!
//! `scripts/globals-check.mjs` installs a recorder as the globals below and asserts the operations
//! the emission performed on it, the way `scripts/extern-check.mjs` does through a module.

#![no_std]
#![no_main]

use js_extern_macro::js_extern;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// An opaque JavaScript value, for the reasons `01_chart.rs` gives at length: one machine word so
/// MIR keeps it, and `repr(transparent)` so it is the value rather than an object holding one.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsValue {
    /// Never read by Rust code; see the module docs.
    #[allow(dead_code)]
    handle: u32,
}

// No module: everything here is a name the host already has.
#[js_extern]
unsafe extern "C" {
    /// `new EventSource(url)`, the dashboard's own line. No flag: a construction has no argument
    /// to be rooted at instead.
    #[js(new = "EventSource")]
    fn event_source(url: &str) -> JsValue;

    /// `dashLog(message)`: a bare global function.
    #[js(call = "dashLog")]
    fn log(message: &str);

    /// `Math.max(a, b)`: a global reached through a path, whose last step keeps its receiver.
    #[js(call = "Math.max")]
    fn max(a: f64, b: f64) -> f64;

    /// `source.close()`: a method on what the constructor returned. Rooted at its argument, with
    /// no flag, inside the same block as the globals above.
    #[js(method = "close")]
    fn close(source: JsValue);

    /// `dash.status`: the read the flag exists for.
    #[js(global, get = "dash.status")]
    fn status() -> JsValue;

    /// `dash.status = value`.
    #[js(global, set = "dash.status")]
    fn set_status(value: JsValue);

    /// `dash.ticks[index]`: an index rooted at a global rather than at a receiver.
    #[js(global, index = "dash.ticks")]
    fn tick(index: usize) -> JsValue;

    /// `dash.missing`, which the recorder answers `undefined` for: `undefined` is a `None` as
    /// much as `null` is, which is the half of the nullable rule a global path can check.
    #[js(global, get = "dash.missing", nullable)]
    fn missing() -> Option<JsValue>;
}

/// `new EventSource(url)` and nothing else, so the trace shows the construction alone.
#[unsafe(no_mangle)]
pub extern "C" fn v_new_global(url: &str) -> JsValue {
    event_source(url)
}

/// A construction followed by a method on what it returned: the two roots in one expression.
#[unsafe(no_mangle)]
pub extern "C" fn v_new_then_close(url: &str) {
    let source = event_source(url);
    close(source);
}

/// A bare global call.
#[unsafe(no_mangle)]
pub extern "C" fn v_call_global(message: &str) {
    log(message)
}

/// A global call through a path, which is `Math.max` and not `max`.
#[unsafe(no_mangle)]
pub extern "C" fn v_call_global_path(a: f64, b: f64) -> f64 {
    max(a, b)
}

/// A property read off a global path.
#[unsafe(no_mangle)]
pub extern "C" fn v_get_global() -> JsValue {
    status()
}

/// A property write to a global path.
#[unsafe(no_mangle)]
pub extern "C" fn v_set_global(value: JsValue) {
    set_status(value)
}

/// An index off a global path.
#[unsafe(no_mangle)]
pub extern "C" fn v_index_global(index: usize) -> JsValue {
    tick(index)
}

/// A nullable read off a global path, answered `undefined`.
#[unsafe(no_mangle)]
pub extern "C" fn v_nullable_global() -> bool {
    missing().is_none()
}
