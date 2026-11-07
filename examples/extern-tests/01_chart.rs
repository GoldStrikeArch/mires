//! `#[js_extern]` against the contract's fake chart library, one entry point per vector.
//!
//! `contract/fixtures/js-extern/chart-lib.mjs` is a RECORDER shaped like an npm chart library, and
//! `vectors.json` holds, per call shape, the trace a correct emission leaves behind. This crate
//! declares that library and drives it; `scripts/extern-check.mjs` runs each entry point below
//! against the fake and compares the trace with the vector of the same id. The emitted JavaScript
//! does not have to match the reference emission's text, and it must be indistinguishable from it
//! at the trace.
//!
//! # What crosses the boundary
//!
//! Numbers and strings cross as themselves: `resize(800)` really passes the number 800, which is
//! why the vector can assert `argc` and `args`. Everything the library hands back, and the two
//! composite arguments the reference emission builds in JavaScript, cross as [`JsValue`]: a
//! `repr(transparent)` one machine word, which is what a chart instance, a config object and a
//! plugin are from Rust's side. Both properties are load bearing, for the reasons `view_abi::Node`
//! gives at length: a zero sized value is folded away by MIR, and an ordinary one field struct is
//! rebuilt field by field, so a JavaScript object passed through one would come back as
//! `{ handle: undefined }`.
//!
//! The composite arguments come FROM the check rather than being built here, and that is the point
//! rather than a shortcut: the vectors pin that the object the caller supplied is the object the
//! library received, unmarshalled.

#![no_std]
#![no_main]

use js_extern_macro::js_extern;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// An opaque JavaScript value: a chart instance, a config, a DOM element, a data point.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsValue {
    /// Never read by Rust code; see the module docs.
    #[allow(dead_code)]
    handle: u32,
}

// The library's module scope: the two bindings it exports, and the statics on them. A shape that
// names a module is rooted at that module's export rather than at an argument.
#[js_extern(module = "./chart-lib.mjs")]
unsafe extern "C" {
    /// `new Chart(target, config)`, through the NAMED export.
    #[js(new = "Chart")]
    fn chart_new(target: JsValue, config: JsValue) -> JsValue;

    /// The same through the DEFAULT export, which is a different function object.
    #[js(new = "default")]
    fn chart_new_default(target: JsValue, config: JsValue) -> JsValue;

    /// `Chart(target, config)` with no `new`, which the library refuses. Declared so that the
    /// `new` shape's negative arm is exercised rather than assumed.
    #[js(call = "Chart")]
    fn chart_call(target: JsValue, config: JsValue) -> JsValue;

    /// `Chart.version`: a property rooted at the module binding and walked one step.
    #[js(get = "Chart.version")]
    fn chart_version() -> JsValue;

    /// `Chart.register(plugin)`: a function reached through a scope path, so the last step is
    /// called as a member and keeps its receiver.
    #[js(call = "Chart.register")]
    fn chart_register(plugin: JsValue);
}

// The instance's own surface. These name no module, so each is rooted at its first argument.
#[js_extern]
unsafe extern "C" {
    #[js(method = "update")]
    fn update(chart: JsValue, data: JsValue);

    #[js(method = "destroy")]
    fn destroy(chart: JsValue);

    /// `chart.resize(width)`: the trailing argument omitted, which the library sees as `argc == 1`.
    #[js(method = "resize")]
    fn resize(chart: JsValue, width: f64);

    /// `chart.resize(width, height)`, where the check passes `undefined` as the height. The two
    /// declarations differ only in arity, and the library reports which one ran.
    #[js(method = "resize")]
    fn resize_to(chart: JsValue, width: f64, height: JsValue);

    /// `chart.getPoint(index)`, which answers `null` out of range. `#[js(nullable)]` is what turns
    /// that into `None` instead of into a value nothing in Rust can hold.
    #[js(method = "getPoint", nullable)]
    fn get_point(chart: JsValue, index: f64) -> Option<JsValue>;

    #[js(get = "data")]
    fn data(chart: JsValue) -> JsValue;

    /// A walk: two property reads, which the library records as two operations.
    #[js(get = "data.datasets")]
    fn datasets(chart: JsValue) -> JsValue;

    #[js(get = "data.datasets.length")]
    fn datasets_len(chart: JsValue) -> f64;

    #[js(set = "title")]
    fn set_title(chart: JsValue, title: JsValue);

    /// `chart.data.datasets[index]`: a walk and then an index, in one declaration.
    #[js(index = "data.datasets")]
    fn dataset_at(chart: JsValue, index: f64) -> JsValue;

    /// Read off a plain data object, which the library does not record: it is here so the check
    /// can prove a `Some` carries the value and not just the tag.
    #[js(get = "x")]
    fn point_x(point: JsValue) -> f64;
}

// ---------------------------------------------------------------------------------------------
// One entry point per vector. Named `v_<vector id with dashes as underscores>`.
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub fn v_new_named(target: JsValue, config: JsValue) -> JsValue {
    chart_new(target, config)
}

#[unsafe(no_mangle)]
pub fn v_new_default(target: JsValue, config: JsValue) -> JsValue {
    chart_new_default(target, config)
}

#[unsafe(no_mangle)]
pub fn v_new_without_new(target: JsValue, config: JsValue) -> JsValue {
    chart_call(target, config)
}

#[unsafe(no_mangle)]
pub fn v_send_one_arg(chart: JsValue, data: JsValue) {
    update(chart, data);
}

#[unsafe(no_mangle)]
pub fn v_send_zero_args(chart: JsValue) {
    destroy(chart);
}

#[unsafe(no_mangle)]
pub fn v_send_trailing_omitted(chart: JsValue) {
    resize(chart, 800.0);
}

#[unsafe(no_mangle)]
pub fn v_send_trailing_undefined(chart: JsValue, height: JsValue) {
    resize_to(chart, 800.0, height);
}

/// The `Some` arm, and the value inside it: `-1` would mean the point never arrived.
#[unsafe(no_mangle)]
pub fn v_nullable_some(chart: JsValue) -> f64 {
    match get_point(chart, 0.0) {
        Some(point) => point_x(point),
        None => -1.0,
    }
}

/// The `None` arm. `true` means a JavaScript `null` became an absent `Option`.
#[unsafe(no_mangle)]
pub fn v_nullable_none(chart: JsValue) -> bool {
    get_point(chart, 99.0).is_none()
}

#[unsafe(no_mangle)]
pub fn v_get_property(chart: JsValue) -> JsValue {
    data(chart)
}

#[unsafe(no_mangle)]
pub fn v_get_scoped(chart: JsValue) -> JsValue {
    datasets(chart)
}

#[unsafe(no_mangle)]
pub fn v_get_static() -> JsValue {
    chart_version()
}

#[unsafe(no_mangle)]
pub fn v_set_property(chart: JsValue, title: JsValue) {
    set_title(chart, title);
}

#[unsafe(no_mangle)]
pub fn v_index_in_range(chart: JsValue) -> JsValue {
    dataset_at(chart, 0.0)
}

#[unsafe(no_mangle)]
pub fn v_index_length(chart: JsValue) -> f64 {
    datasets_len(chart)
}

#[unsafe(no_mangle)]
pub fn v_index_out_of_range(chart: JsValue) -> JsValue {
    dataset_at(chart, 9.0)
}

#[unsafe(no_mangle)]
pub fn v_call_static(plugin: JsValue) {
    chart_register(plugin);
}

/// The suite's entry point. Nothing drives the library from Rust: every vector is one exported
/// function the check calls with the arguments the reference emission used.
#[unsafe(no_mangle)]
pub fn rust_entry() {}
