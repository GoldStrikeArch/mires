//! Small dynamic browser boundary shared by the workloads.

use mires_js::{callback0, callback2, construct, invoke, js_bindings, JsValue};

js_bindings! {
    #[js(call = "Object")]
    fn object_new() -> JsValue;

    #[js(call = "Object")]
    fn object_from(value: JsValue) -> JsValue;

    #[js(call = "Array")]
    fn array_new() -> JsValue;

    #[js(call = "Array.isArray")]
    pub fn is_array(value: JsValue) -> bool;

    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;

    #[js(call = "Reflect.set")]
    fn reflect_set(target: JsValue, key: JsValue, value: JsValue) -> bool;

    #[js(call = "parseFloat")]
    fn parse_float_raw(value: JsValue) -> f64;

    #[js(global, get = "globalThis")]
    pub fn global_this() -> JsValue;

    #[js(call = "setTimeout")]
    pub fn set_timeout(callback: JsValue, milliseconds: f64) -> f64;

    #[js(call = "clearTimeout")]
    pub fn clear_timeout(identifier: f64);
}

#[must_use]
pub fn object() -> JsValue {
    object_new()
}

/// Boxes a JavaScript primitive so it can be used with the Reflect-based dynamic boundary.
#[must_use]
pub fn box_value(value: JsValue) -> JsValue {
    object_from(value)
}

#[must_use]
pub fn array() -> JsValue {
    array_new()
}

#[must_use]
pub fn get(target: JsValue, key: &str) -> JsValue {
    reflect_get(target, JsValue::erase(key))
}

#[must_use]
pub fn get_key(target: JsValue, key: JsValue) -> JsValue {
    reflect_get(target, key)
}

pub fn set(target: JsValue, key: &str, value: JsValue) {
    let _ = reflect_set(target, JsValue::erase(key), value);
}

#[must_use]
pub fn index(target: JsValue, at: usize) -> JsValue {
    get_key(target, JsValue::erase(at))
}

#[must_use]
pub fn length(target: JsValue) -> usize {
    parse_float(get(target, "length")) as usize
}

#[must_use]
pub fn parse_float(value: JsValue) -> f64 {
    if value.is_number() {
        value.read()
    } else {
        parse_float_raw(value)
    }
}

#[must_use]
pub fn call(receiver: JsValue, method: &str, arguments: &[JsValue]) -> JsValue {
    invoke(get(receiver, method), receiver, arguments)
}

pub fn push(target: JsValue, value: JsValue) {
    let _ = call(target, "push", &[value]);
}

#[must_use]
pub fn promise<F>(executor: F) -> JsValue
where
    F: Fn(JsValue, JsValue) -> JsValue + 'static,
{
    construct(get(global_this(), "Promise"), &[callback2(executor)])
}

pub fn settle(function: JsValue, value: JsValue) {
    let _ = invoke(function, JsValue::undefined(), &[value]);
}

#[must_use]
pub fn wait(milliseconds: f64) -> JsValue {
    promise(move |resolve, _reject| {
        let callback = callback0(move || {
            settle(resolve, JsValue::undefined());
            JsValue::undefined()
        });
        let _ = set_timeout(callback, milliseconds);
        JsValue::undefined()
    })
}

#[must_use]
pub fn promise_resolve(value: JsValue) -> JsValue {
    let promise = get(global_this(), "Promise");
    invoke(get(promise, "resolve"), promise, &[value])
}
