//! Typed native host values and direct computed and fallible bindings.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_bindings, js_export, JsArray, JsString, JsUint8Array, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "console.log")]
    fn log(label: &str, value: JsValue);

    #[js(call = "Object")]
    fn object() -> JsValue;

    #[js(call = "Object.defineProperty")]
    fn define_property(target: JsValue, key: &str, descriptor: JsValue) -> JsValue;

    #[js(call = "Object.getOwnPropertyDescriptor")]
    fn own_descriptor(target: JsValue, key: &str) -> JsValue;

    #[js(set = "get")]
    fn set_get(target: JsValue, value: JsValue);

    #[js(set = "enumerable")]
    fn set_enumerable(target: JsValue, value: bool);

    #[js(set = "configurable")]
    fn set_configurable(target: JsValue, value: bool);

    #[js(get = "lazy")]
    fn get_lazy(target: JsValue) -> JsValue;

    #[js(get = "get")]
    fn get_descriptor_get(target: JsValue) -> JsValue;

    #[js(get = "enumerable")]
    fn get_enumerable(target: JsValue) -> bool;

    #[js(get = "configurable")]
    fn get_configurable(target: JsValue) -> bool;

    #[js(call = "Number")]
    fn number_as_f32(value: f64) -> f32;

    #[js(has)]
    fn has(value: JsValue, key: &str) -> bool;

    #[js(computed_method)]
    fn call0(value: JsValue, key: &str) -> JsValue;

    #[js(computed_method)]
    fn call1(value: JsValue, key: &str, first: JsValue) -> usize;

    #[js(computed_method)]
    fn call2(value: JsValue, key: &str, first: usize, second: usize) -> JsValue;

    #[js(global, set = "globalThis.hostBoundaryThrown")]
    fn set_thrown(value: JsValue);

    #[js(call = "eval", fallible)]
    fn try_eval(source: &str) -> Result<JsValue, JsValue>;

    #[js(get = "missing", fallible)]
    fn try_get(value: JsValue) -> Result<JsValue, JsValue>;

    #[js(method = "slice", fallible)]
    fn try_method(value: JsValue, start: usize) -> Result<JsValue, JsValue>;

    #[js(new = "URL", fallible)]
    fn try_new_url(value: &str) -> Result<JsValue, JsValue>;

    #[js(new = "Error")]
    fn error(message: &str) -> JsValue;
}

fn report(label: &str, value: bool) {
    log(label, JsValue::erase(value));
}

#[js_export]
pub fn rust_entry() {
    let text = JsString::new("A\u{1f600}B");
    report(
        "string",
        text.len() == 4
            && text.code_unit_at(0) == Some(0x41)
            && text.code_unit_at(1) == Some(0xd83d)
            && text.code_unit_at(2) == Some(0xde00)
            && text.code_unit_at(4).is_none()
            && text
                .slice(1, 3)
                .as_value()
                .strictly_equals(JsString::new("\u{1f600}").as_value())
            && text
                .concat(JsString::new("!"))
                .as_value()
                .strictly_equals(JsValue::erase("A\u{1f600}B!")),
    );
    report(
        "string-cast",
        JsString::try_from(text.as_value()).is_ok()
            && JsString::try_from(JsValue::erase(1u32)).is_err(),
    );

    let mut array = JsArray::<JsValue>::new();
    report("array-empty", array.is_empty());
    report("array-push", array.push(JsValue::erase("first")) == 1);
    array.set(1, JsValue::erase("second"));
    report(
        "array-index",
        array.len() == 2
            && array.get(0).strictly_equals(JsValue::erase("first"))
            && array.get(1).strictly_equals(JsValue::erase("second")),
    );
    report(
        "array-cast",
        JsArray::<JsValue>::try_from(array.as_value()).is_ok()
            && JsArray::<JsValue>::try_from(text.as_value()).is_err(),
    );

    let host = array.as_value();
    report("has", has(host, "length") && !has(host, "missing"));
    report(
        "computed-0",
        call0(host, "pop").strictly_equals(JsValue::erase("second")),
    );
    report(
        "computed-1",
        call1(host, "push", JsValue::erase("third")) == 2,
    );
    report("computed-2", call2(host, "splice", 0, 1).is_object());

    let thrown = error("same object");
    set_thrown(thrown);
    report(
        "fallible-call",
        matches!(
            try_eval("throw globalThis.hostBoundaryThrown"),
            Err(value) if value.strictly_equals(thrown)
        ),
    );
    report(
        "fallible-get",
        matches!(try_get(host), Ok(value) if value.is_undefined()),
    );
    report(
        "fallible-method",
        matches!(try_method(JsValue::null(), 0), Err(value) if value.is_error()),
    );
    report(
        "fallible-new",
        matches!(try_new_url("::"), Err(value) if value.is_error()),
    );

    let mut bytes = JsUint8Array::new(1);
    report("fallible-library-ok", bytes.copy_from(bytes, 0).is_ok());
    report(
        "fallible-library-error",
        matches!(bytes.copy_from(bytes, 1), Err(value) if value.is_error()),
    );

    let getter = mires_js::callback0(|| {
        report("lazy-getter", true);
        JsValue::erase("lazy-value")
    });
    let descriptor = object();
    set_get(descriptor, getter);
    set_enumerable(descriptor, true);
    set_configurable(descriptor, true);
    let target = object();
    let _ = define_property(target, "lazy", descriptor);
    let actual_descriptor = own_descriptor(target, "lazy");
    report(
        "lazy-accessor",
        get_lazy(target).strictly_equals(JsValue::erase("lazy-value")),
    );
    report(
        "lazy-descriptor",
        get_descriptor_get(actual_descriptor).strictly_equals(getter)
            && get_enumerable(actual_descriptor)
            && get_configurable(actual_descriptor),
    );

    report(
        "f32-read",
        JsValue::erase(16_777_217f64).read::<f32>() == 16_777_216f32,
    );
    report("f32-binding", number_as_f32(16_777_217f64) == 16_777_216f32);
}
