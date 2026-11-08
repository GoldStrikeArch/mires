//! The deliberately small browser boundary shared by the fingerprint sources.

use mires_js::{callback0, invoke, js_bindings, try_host, JsValue};

js_bindings! {
    #[js(call = "Object")]
    fn object_constructor() -> JsValue;

    #[js(call = "Object")]
    fn object_constructor_with(value: JsValue) -> JsValue;

    #[js(call = "Array")]
    fn array_constructor() -> JsValue;

    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;

    #[js(call = "Reflect.set")]
    fn reflect_set(target: JsValue, key: JsValue, value: JsValue) -> bool;

    #[js(call = "Reflect.has")]
    fn reflect_has(target: JsValue, key: JsValue) -> bool;

    #[js(call = "Object.keys")]
    fn object_keys_raw(target: JsValue) -> JsValue;

    #[js(call = "Object.getOwnPropertyNames")]
    fn own_property_names_raw(target: JsValue) -> JsValue;

    #[js(call = "Object.getPrototypeOf")]
    fn object_prototype_raw(target: JsValue) -> JsValue;

    #[js(call = "Object.defineProperty")]
    fn define_property_raw(target: JsValue, key: JsValue, descriptor: JsValue) -> JsValue;

    #[js(call = "Array.isArray")]
    pub fn is_array(value: JsValue) -> bool;

    #[js(call = "JSON.stringify")]
    fn json_stringify_raw(value: JsValue) -> JsValue;

    #[js(call = "JSON.stringify")]
    fn json_stringify_with_raw(value: JsValue, replacer: JsValue, space: JsValue) -> JsValue;

    #[js(call = "Promise.all")]
    fn promise_all_raw(values: JsValue) -> JsValue;

    #[js(call = "String")]
    fn string_raw(value: JsValue) -> JsValue;

    #[js(call = "Number")]
    fn number_raw(value: JsValue) -> f64;

    #[js(call = "parseInt")]
    fn parse_int_raw(value: JsValue) -> f64;

    #[js(call = "parseFloat")]
    fn parse_float_raw(value: JsValue) -> f64;

    #[js(call = "isFinite")]
    pub fn is_finite(value: f64) -> bool;

    #[js(call = "Date.now")]
    pub fn date_now() -> f64;

    #[js(call = "performance.now")]
    pub fn performance_now() -> f64;

    #[js(call = "matchMedia")]
    pub fn match_media(query: &str) -> JsValue;

    #[js(call = "setTimeout")]
    pub fn set_timeout(callback: JsValue, milliseconds: f64) -> f64;

    #[js(call = "clearTimeout")]
    pub fn clear_timeout(identifier: f64);

    #[js(call = "atob")]
    pub fn decode_base64(value: &str) -> JsValue;

    #[js(new = "Error")]
    fn new_error_raw(message: &str) -> JsValue;

    #[js(global, get = "window")]
    pub fn window() -> JsValue;

    #[js(global, get = "document")]
    pub fn document() -> JsValue;

    #[js(global, get = "navigator")]
    pub fn navigator() -> JsValue;

    #[js(global, get = "screen")]
    pub fn screen() -> JsValue;

    #[js(global, get = "globalThis")]
    pub fn global_this() -> JsValue;
}

/// Creates an ordinary JavaScript object.
#[must_use]
pub fn object() -> JsValue {
    object_constructor()
}

/// Creates an ordinary empty JavaScript array.
#[must_use]
pub fn array() -> JsValue {
    array_constructor()
}

/// Reads a property using the host's full JavaScript property semantics.
#[must_use]
pub fn get(target: JsValue, key: &str) -> JsValue {
    reflect_get(target, JsValue::erase(key))
}

/// Reads a computed property using the host's full JavaScript property semantics.
#[must_use]
pub fn get_key(target: JsValue, key: JsValue) -> JsValue {
    reflect_get(target, key)
}

/// Writes an own or inherited JavaScript property.
pub fn set(target: JsValue, key: &str, value: JsValue) {
    let _ = reflect_set(target, JsValue::erase(key), value);
}

/// Writes a computed JavaScript property.
pub fn set_key(target: JsValue, key: JsValue, value: JsValue) {
    let _ = reflect_set(target, key, value);
}

/// Applies JavaScript's `in` operation.
#[must_use]
pub fn has(target: JsValue, key: &str) -> bool {
    reflect_has(target, JsValue::erase(key))
}

/// Calls a named method with its receiver intact.
#[must_use]
pub fn call(target: JsValue, method: &str, arguments: &[JsValue]) -> JsValue {
    invoke(
        get(object_constructor_from(target), method),
        target,
        arguments,
    )
}

fn object_constructor_from(value: JsValue) -> JsValue {
    object_constructor_with(value)
}

/// Calls a named method while converting a synchronous throw into `Result`.
pub fn try_call(
    target: JsValue,
    method: &'static str,
    arguments: &[JsValue],
) -> Result<JsValue, JsValue> {
    let arguments = arguments.to_vec();
    try_host(move || call(target, method, &arguments))
}

/// Returns the enumerable own keys of an object as a native array.
#[must_use]
pub fn object_keys(target: JsValue) -> JsValue {
    object_keys_raw(target)
}

/// Returns all own property names of an object as a native array.
#[must_use]
pub fn own_property_names(target: JsValue) -> JsValue {
    own_property_names_raw(target)
}

/// Returns an object's immediate prototype.
#[must_use]
pub fn object_prototype(target: JsValue) -> JsValue {
    object_prototype_raw(target)
}

/// Defines a JavaScript property from an ordinary descriptor object.
pub fn define_property(target: JsValue, key: &str, descriptor: JsValue) {
    let _ = define_property_raw(target, JsValue::erase(key), descriptor);
}

/// Returns the length of an array-like value.
#[must_use]
pub fn length(value: JsValue) -> usize {
    get(value, "length").read()
}

/// Reads an array-like index.
#[must_use]
pub fn index(value: JsValue, index: usize) -> JsValue {
    get_key(value, JsValue::erase(index))
}

/// Writes an array-like index.
pub fn set_index(value: JsValue, index: usize, item: JsValue) {
    set_key(value, JsValue::erase(index), item);
}

/// Appends a value to a native array.
pub fn push(value: JsValue, item: JsValue) {
    let _ = call(value, "push", &[item]);
}

/// Sorts a native array with JavaScript's default UTF-16 ordering.
pub fn sort(value: JsValue) {
    let _ = call(value, "sort", &[]);
}

/// Converts a value with JavaScript's `String` constructor.
#[must_use]
pub fn string_value(value: JsValue) -> JsValue {
    string_raw(value)
}

/// Converts a value with JavaScript's `Number` constructor.
#[must_use]
pub fn number_value(value: JsValue) -> f64 {
    number_raw(value)
}

/// Parses an arbitrary host value with JavaScript `parseInt`.
#[must_use]
pub fn parse_int(value: JsValue) -> f64 {
    parse_int_raw(value)
}

/// Parses an arbitrary host value with JavaScript `parseFloat`.
#[must_use]
pub fn parse_float(value: JsValue) -> f64 {
    parse_float_raw(value)
}

/// Calls the native JSON serializer.
#[must_use]
pub fn json_stringify(value: JsValue) -> JsValue {
    json_stringify_raw(value)
}

/// Calls the native JSON serializer with a replacer and indentation.
#[must_use]
pub fn json_stringify_with(value: JsValue, replacer: JsValue, space: JsValue) -> JsValue {
    json_stringify_with_raw(value, replacer, space)
}

/// Calls native `Promise.all`.
#[must_use]
pub fn promise_all(values: JsValue) -> JsValue {
    promise_all_raw(values)
}

/// Tests the exact thenable shape used by FingerprintJS.
#[must_use]
pub fn is_promise(value: JsValue) -> bool {
    value.is_truthy() && get(object_constructor_from(value), "then").is_function()
}

/// Prevents an otherwise-unobserved Promise rejection warning.
#[must_use]
pub fn suppress_unhandled_rejection(value: JsValue) -> JsValue {
    #[allow(clippy::redundant_closure)]
    let ignored = callback0(|| JsValue::undefined());
    let _ = call(value, "then", &[JsValue::undefined(), ignored]);
    value
}

/// Creates an ordinary JavaScript `Error` with an optional custom name.
#[must_use]
pub fn new_error(name: &str, message: &str) -> JsValue {
    let error = new_error_raw(message);
    set(error, "name", JsValue::erase(name));
    error
}

/// Converts `None` to JavaScript `undefined`.
#[must_use]
pub fn optional(value: Option<JsValue>) -> JsValue {
    value.unwrap_or_else(JsValue::undefined)
}
