//! FingerprintJS-compatible public orchestration and host-value helpers.

use alloc::rc::Rc;
use core::cell::RefCell;

use mires_js::{
    await_promise, callback0, callback1, construct, future_to_promise, invoke, try_host, JsValue,
};

use crate::{
    call, date_now, define_property, get, has, index, is_android, is_desktop_webkit,
    is_safari_webkit, is_webkit, is_webkit_616_or_newer, json_stringify_with,
    length, load_sources, make_sources, number_value, object, object_keys, prepare_for_sources,
    set, string_value, ALGORITHM_VERSION,
};

/// Hashes arbitrary host components with the exact native canonicalization contract.
#[must_use]
pub fn hash_components_host(components: JsValue) -> JsValue {
    crate::kernel_hash_components(components)
}

/// Produces the upstream indented debug serialization, including non-enumerable Error fields.
#[must_use]
pub fn components_to_debug_string(components: JsValue) -> JsValue {
    // JSON replacers receive `(key, value)`, so use the arity-2 callback for exact semantics.
    let replacer = mires_js::callback2(move |_key, value| {
        if !value.is_error() {
            return value;
        }
        error_to_object(value)
    });
    json_stringify_with(components, replacer, JsValue::erase(2_u32))
}

fn error_to_object(error: JsValue) -> JsValue {
    let result = object();
    set(result, "name", get(error, "name"));
    set(result, "message", get(error, "message"));
    let stack = get(error, "stack");
    if stack.is_undefined() {
        set(result, "stack", JsValue::undefined());
    } else {
        set(
            result,
            "stack",
            call(stack, "split", &[JsValue::erase("\n")]),
        );
    }
    let keys = object_keys(error);
    for key_index in 0..length(keys) {
        let key = index(keys, key_index);
        crate::set_key(result, key, crate::get_key(error, key));
    }
    result
}

const NPM_MONITORING_URL: &str = "https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring";

fn monitor() {
    if get(crate::window(), "__fpjs_d_m").is_truthy() {
        return;
    }
    let random = call(get(crate::global_this(), "Math"), "random", &[]);
    if number_value(random) >= 0.001 {
        return;
    }
    if let Err(error) = try_host(|| {
        let request = construct(get(crate::global_this(), "XMLHttpRequest"), &[]);
        let _ = call(
            request,
            "open",
            &[
                JsValue::erase("get"),
                JsValue::erase(NPM_MONITORING_URL),
                JsValue::erase(true),
            ],
        );
        let _ = call(request, "send", &[]);
        JsValue::undefined()
    }) {
        let _ = call(get(crate::global_this(), "console"), "error", &[error]);
    }
}

/// Loads built-in sources after the upstream idle preparation delay.
#[must_use]
pub fn load(options: JsValue) -> JsValue {
    let options = if options.is_undefined() {
        object()
    } else {
        options
    };
    let delay = get(options, "delayFallback");
    let debug = get(options, "debug");
    let monitoring = get(options, "monitoring");
    if monitoring.is_undefined() || monitoring.is_truthy() {
        monitor();
    }
    future_to_promise(async move {
        let delay = if delay.is_undefined() {
            None
        } else {
            Some(number_value(delay))
        };
        await_promise(prepare_for_sources(delay)).await?;
        let source_options = object();
        set(source_options, "cache", object());
        set(source_options, "debug", debug);
        let get_components = load_sources(
            make_sources(),
            source_options,
            crate::array(),
            JsValue::undefined(),
        );
        Ok(make_agent(get_components, debug))
    })
}

fn make_agent(get_components: JsValue, debug: JsValue) -> JsValue {
    let creation_time = date_now();
    let agent = object();
    let get_callback = callback1(move |get_options| {
        let start_time = date_now();
        let finish = callback1(move |components| {
            let result = make_lazy_get_result(components);
            if debug.is_truthy()
                || (!get_options.is_null()
                    && !get_options.is_undefined()
                    && get(get_options, "debug").is_truthy())
            {
                print_debug(result, start_time - creation_time);
            }
            result
        });
        call(
            invoke(get_components, JsValue::undefined(), &[]),
            "then",
            &[finish],
        )
    });
    set(agent, "get", get_callback);
    agent
}

fn make_lazy_get_result(components: JsValue) -> JsValue {
    let result = object();
    let cache = Rc::new(RefCell::new(JsValue::undefined()));
    let getter_cache = Rc::clone(&cache);
    let getter = callback0(move || {
        let mut value = *getter_cache.borrow();
        if value.is_undefined() {
            value = hash_components_host(components);
            *getter_cache.borrow_mut() = value;
        }
        value
    });
    let setter = callback1(move |value| {
        *cache.borrow_mut() = value;
        JsValue::undefined()
    });
    set_function_name(getter, "get visitorId");
    set_function_name(setter, "set visitorId");
    let descriptor = object();
    set(descriptor, "get", getter);
    set(descriptor, "set", setter);
    set(descriptor, "enumerable", JsValue::erase(true));
    set(descriptor, "configurable", JsValue::erase(true));
    define_property(result, "visitorId", descriptor);
    set(result, "confidence", confidence(components));
    set(result, "components", components);
    set(result, "version", JsValue::erase(ALGORITHM_VERSION));
    result
}

fn set_function_name(function: JsValue, name: &str) {
    let descriptor = object();
    set(descriptor, "value", JsValue::erase(name));
    set(descriptor, "configurable", JsValue::erase(true));
    define_property(function, "name", descriptor);
}

fn confidence(components: JsValue) -> JsValue {
    let score = if is_android() {
        0.4
    } else if is_webkit() {
        if is_desktop_webkit() && !(is_webkit_616_or_newer() && is_safari_webkit()) {
            0.5
        } else {
            0.3
        }
    } else {
        let platform = get(components, "platform");
        let platform = if has(platform, "value") {
            string_value(get(platform, "value"))
        } else {
            JsValue::erase("")
        };
        let platform: &str = platform.read();
        if platform.starts_with("Win") {
            0.6
        } else if platform.starts_with("Mac") {
            0.5
        } else {
            0.7
        }
    };
    let pro_score = crate::round(0.99 + 0.01 * score, 0.0001);
    let pro_score_string = string_value(JsValue::erase(pro_score));
    let comment = alloc::format!(
        "{} if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score",
        pro_score_string.read::<&str>(),
    );
    let result = object();
    set(result, "score", JsValue::erase(score));
    set(result, "comment", JsValue::erase(comment.as_str()));
    result
}

fn print_debug(result: JsValue, time_between_load_and_get: f64) {
    let console = get(crate::global_this(), "console");
    let message = alloc::format!(
        "Copy the text below to get the debug data:\n\n```\nversion: {}\nuserAgent: {}\ntimeBetweenLoadAndGet: {}\nvisitorId: {}\ncomponents: {}\n```",
        ALGORITHM_VERSION,
        string_value(get(crate::navigator(), "userAgent")).read::<&str>(),
        time_between_load_and_get,
        string_value(get(result, "visitorId")).read::<&str>(),
        string_value(components_to_debug_string(get(result, "components"))).read::<&str>(),
    );
    let _ = call(console, "log", &[JsValue::erase(message.as_str())]);
}
