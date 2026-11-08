//! FingerprintJS-compatible public orchestration and host-value helpers.

use alloc::rc::Rc;
use core::cell::RefCell;

use mires_js::{callback0, callback1, construct, invoke, try_host, JsValue};

use crate::{
    as_promise, call, date_now, define_property, get, has, hash_hex, index, is_android,
    is_desktop_webkit, is_safari_webkit, is_webkit, is_webkit_616_or_newer, json_stringify,
    json_stringify_with, length, load_sources, make_sources, number_value, object, object_keys,
    prepare_for_sources, push, set, sort, string_value, x64hash128, ALGORITHM_VERSION,
};

/// Hashes arbitrary host components with the exact native canonicalization contract.
pub fn hash_components_host(components: JsValue) -> JsValue {
    let keys = object_keys(components);
    sort(keys);
    let parts = crate::array();
    for key_index in 0..length(keys) {
        let key = index(keys, key_index);
        let component = crate::get_key(components, key);
        let value = if has(component, "error") {
            JsValue::erase("error")
        } else {
            let serialized = json_stringify(get(component, "value"));
            if serialized.is_undefined() {
                JsValue::erase("undefined")
            } else {
                serialized
            }
        };
        let escaped = escape_component_key(key);
        let part = crate::array();
        push(part, escaped);
        push(part, JsValue::erase(":"));
        push(part, value);
        push(parts, call(part, "join", &[JsValue::erase("")]));
    }
    let canonical = call(parts, "join", &[JsValue::erase("|")]);
    hash_hex(&x64hash128(canonical.read()))
}

fn escape_component_key(key: JsValue) -> JsValue {
    let escaped_backslashes = call(key, "split", &[JsValue::erase("\\")]);
    let escaped_backslashes = call(escaped_backslashes, "join", &[JsValue::erase("\\\\")]);
    let escaped_colons = call(escaped_backslashes, "split", &[JsValue::erase(":")]);
    let escaped_colons = call(escaped_colons, "join", &[JsValue::erase("\\:")]);
    let escaped_pipes = call(escaped_colons, "split", &[JsValue::erase("|")]);
    call(escaped_pipes, "join", &[JsValue::erase("\\|")])
}

/// Produces the upstream indented debug serialization, including non-enumerable Error fields.
pub fn components_to_debug_string(components: JsValue) -> JsValue {
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
    let delay = if delay.is_undefined() {
        None
    } else {
        Some(number_value(delay))
    };
    let prepared = prepare_for_sources(delay);
    let finish = callback0(move || {
        let source_options = object();
        set(source_options, "cache", object());
        set(source_options, "debug", debug);
        let get_components = load_sources(
            make_sources(),
            source_options,
            crate::array(),
            JsValue::undefined(),
        );
        make_agent(get_components, debug)
    });
    call(as_promise(prepared), "then", &[finish])
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
    let comment = crate::join_as_strings(&[
        string_value(JsValue::erase(pro_score)),
        JsValue::erase(
            " if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score",
        ),
    ]);
    let result = object();
    set(result, "score", JsValue::erase(score));
    set(result, "comment", comment);
    result
}

fn print_debug(result: JsValue, time_between_load_and_get: f64) {
    let console = get(crate::global_this(), "console");
    let message = crate::join_as_strings(&[
        JsValue::erase("Copy the text below to get the debug data:\n\n```\nversion: "),
        JsValue::erase(ALGORITHM_VERSION),
        JsValue::erase("\nuserAgent: "),
        string_value(get(crate::navigator(), "userAgent")),
        JsValue::erase("\ntimeBetweenLoadAndGet: "),
        string_value(JsValue::erase(time_between_load_and_get)),
        JsValue::erase("\nvisitorId: "),
        string_value(get(result, "visitorId")),
        JsValue::erase("\ncomponents: "),
        string_value(components_to_debug_string(get(result, "components"))),
        JsValue::erase("\n```"),
    ]);
    let _ = call(console, "log", &[message]);
}
