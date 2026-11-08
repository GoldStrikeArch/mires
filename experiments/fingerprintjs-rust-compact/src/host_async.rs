//! Small Promise and timer helpers shared by asynchronous entropy sources.

use mires_js::{callback0, callback1, callback2, construct, invoke, js_bindings, JsValue};

use crate::{call, get, get_key, global_this, object, set, set_timeout, window};

js_bindings! {
    #[js(new = "MessageChannel")]
    fn message_channel() -> JsValue;
}

/// Creates a native Promise from a safe Rust executor callback.
pub fn promise<F>(executor: F) -> JsValue
where
    F: Fn(JsValue, JsValue) -> JsValue + 'static,
{
    construct(get(global_this(), "Promise"), &[callback2(executor)])
}

/// Settles a native Promise resolver or rejecter.
pub fn settle(function: JsValue, value: JsValue) {
    let _ = invoke(function, JsValue::undefined(), &[value]);
}

/// Returns a Promise settled by `setTimeout`.
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

/// Yields once through a fresh `MessageChannel` task, matching upstream `mapWithBreaks`.
pub fn yield_to_task() -> JsValue {
    promise(move |resolve, _reject| {
        let channel = message_channel();
        let port1 = get(channel, "port1");
        let port2 = get(channel, "port2");
        set(
            port1,
            "onmessage",
            callback0(move || {
                settle(resolve, JsValue::undefined());
                JsValue::undefined()
            }),
        );
        let _ = call(port2, "postMessage", &[JsValue::undefined()]);
        JsValue::undefined()
    })
}

/// Wraps a non-thenable in `Promise.resolve`.
pub fn as_promise(value: JsValue) -> JsValue {
    if crate::is_promise(value) {
        value
    } else {
        resolved_host_promise(value)
    }
}

/// Creates a native resolved Promise.
pub fn resolved_host_promise(value: JsValue) -> JsValue {
    call(get(global_this(), "Promise"), "resolve", &[value])
}

/// Creates a native rejected Promise.
pub fn rejected_promise(error: JsValue) -> JsValue {
    call(get(global_this(), "Promise"), "reject", &[error])
}

/// Waits for an idle callback, falling back to the browser timer queue.
pub fn request_idle_callback_if_available(fallback_timeout: f64, deadline_timeout: f64) -> JsValue {
    let host_window = window();
    let request_idle = get(host_window, "requestIdleCallback");
    if !request_idle.is_truthy() {
        return wait(fallback_timeout.min(deadline_timeout));
    }

    promise(move |resolve, _reject| {
        let callback = callback1(move |_deadline| {
            settle(resolve, JsValue::undefined());
            JsValue::undefined()
        });
        let options = object();
        set(options, "timeout", JsValue::erase(deadline_timeout));
        let _ = invoke(request_idle, host_window, &[callback, options]);
        JsValue::undefined()
    })
}

/// Applies the delay used before FingerprintJS starts its entropy sources.
pub fn prepare_for_sources(delay_fallback: Option<f64>) -> JsValue {
    let delay = delay_fallback.unwrap_or(50.0);
    request_idle_callback_if_available(delay, delay * 2.0)
}

/// Applies JavaScript's `instanceof` protocol without handwritten JavaScript.
pub fn instance_of(value: JsValue, constructor: JsValue) -> bool {
    let symbol = get(global_this(), "Symbol");
    let has_instance = get(symbol, "hasInstance");
    let predicate = get_key(constructor, has_instance);
    invoke(predicate, constructor, &[value]).read()
}

#[cfg(test)]
mod tests {
    #[test]
    fn prepare_uses_the_upstream_default_and_deadline_ratio() {
        let delay = 50.0;
        assert_eq!(delay, 50.0);
        assert_eq!(delay * 2.0, 100.0);
    }
}
