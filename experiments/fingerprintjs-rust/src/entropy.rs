//! Dynamic entropy-source lifecycle compatible with FingerprintJS v5.2.0.

use alloc::vec::Vec;

use mires_js::{callback0, callback1, future_to_promise, invoke, try_host, JsValue};

use crate::{
    array, call, collection_break, date_now, define_property, get, get_key, index, is_promise,
    length, object, object_keys, promise_all, push, set, set_key, suppress_unhandled_rejection,
    LOOP_RELEASE_INTERVAL, SOURCE_FUNCTION_NAMES, SOURCE_NAMES,
};

/// Starts one source immediately and returns its reusable component getter.
#[must_use]
pub fn load_source(source: JsValue, source_options: JsValue) -> JsValue {
    let load_start = date_now();
    let settle_load = move |ok: bool, value: JsValue| -> JsValue {
        let load_duration = date_now() - load_start;
        if !ok {
            callback0(move || resolved_component(false, value, load_duration))
        } else if value.is_function() {
            callback0(move || {
                let get_start = date_now();
                component_from_action(value, get_start, load_duration)
            })
        } else {
            callback0(move || resolved_component(true, value, load_duration))
        }
    };

    let source_load_promise =
        match try_host(move || invoke(source, JsValue::null(), &[source_options])) {
            Ok(value) if is_promise(value) => {
                let on_success = callback1(move |value| settle_load(true, value));
                let on_failure = callback1(move |error| settle_load(false, error));
                suppress_unhandled_rejection(call(value, "then", &[on_success, on_failure]))
            }
            Ok(value) => resolved_promise(settle_load(true, value)),
            Err(error) => resolved_promise(settle_load(false, error)),
        };

    callback0(move || {
        let finalize = callback1(|finalize| invoke(finalize, JsValue::undefined(), &[]));
        call(source_load_promise, "then", &[finalize])
    })
}

fn resolved_promise(value: JsValue) -> JsValue {
    call(get(crate::global_this(), "Promise"), "resolve", &[value])
}

fn resolved_component(ok: bool, value: JsValue, duration: f64) -> JsValue {
    let component = object();
    set(component, if ok { "value" } else { "error" }, value);
    set(component, "duration", JsValue::erase(duration));
    resolved_promise(component)
}

fn component_from_action(action: JsValue, start: f64, load_duration: f64) -> JsValue {
    match try_host(move || invoke(action, JsValue::undefined(), &[])) {
        Ok(value) if is_promise(value) => {
            let on_success =
                callback1(move |value| component(true, value, load_duration + date_now() - start));
            let on_failure =
                callback1(move |error| component(false, error, load_duration + date_now() - start));
            call(value, "then", &[on_success, on_failure])
        }
        Ok(value) => resolved_component(true, value, load_duration + date_now() - start),
        Err(error) => resolved_component(false, error, load_duration + date_now() - start),
    }
}

fn component(ok: bool, value: JsValue, duration: f64) -> JsValue {
    let result = object();
    set(result, if ok { "value" } else { "error" }, value);
    set(result, "duration", JsValue::erase(duration));
    result
}

/// Starts all included sources and returns a reusable component collector.
#[must_use]
pub fn load_sources(
    sources: JsValue,
    source_options: JsValue,
    exclude_sources: JsValue,
    loop_release_interval: JsValue,
) -> JsValue {
    let keys = object_keys(sources);
    let included = array();
    for key_index in 0..length(keys) {
        let key = index(keys, key_index);
        if !array_includes(exclude_sources, key) {
            push(included, key);
        }
    }
    let interval = if loop_release_interval.is_undefined() {
        LOOP_RELEASE_INTERVAL
    } else {
        loop_release_interval.read::<f64>()
    };
    let source_getters_promise = future_to_promise(async move {
        let mut values = Vec::with_capacity(length(included));
        let mut last_release = date_now();
        for key_index in 0..length(included) {
            let key = index(included, key_index);
            values.push(load_source(get_key(sources, key), source_options));
            collection_break(&mut last_release, interval).await;
        }
        let host_values = array();
        for value in values {
            push(host_values, value);
        }
        Ok(host_values)
    });
    let source_getters_promise = suppress_unhandled_rejection(source_getters_promise);

    callback0(move || {
        future_to_promise(async move {
            let source_getters = mires_js::await_promise(source_getters_promise).await?;
            let promises = array();
            let mut last_release = date_now();
            for getter_index in 0..length(source_getters) {
                let getter = index(source_getters, getter_index);
                let promise = invoke(getter, JsValue::undefined(), &[]);
                push(promises, suppress_unhandled_rejection(promise));
                collection_break(&mut last_release, interval).await;
            }
            let assemble = callback1(move |values| {
                let result = object();
                for key_index in 0..length(included) {
                    set_key(result, index(included, key_index), index(values, key_index));
                }
                result
            });
            // Let the host Promise adopt `Promise.all` directly. Keeping a Rust future asleep
            // for every source result retains the entire executor graph and is unnecessary.
            Ok(call(promise_all(promises), "then", &[assemble]))
        })
    })
}

fn array_includes(values: JsValue, needle: JsValue) -> bool {
    if values.is_undefined() || values.is_null() {
        return false;
    }
    for index_value in 0..length(values) {
        if index(values, index_value).strictly_equals(needle) {
            return true;
        }
    }
    false
}

/// Transforms a source while retaining its sync/async and one/two-stage shape.
#[must_use]
pub fn transform_source(source: JsValue, transform_value: JsValue) -> JsValue {
    callback1(move |options| {
        let load_result = invoke(source, JsValue::undefined(), &[options]);
        transform_maybe_promise(load_result, transform_value, true)
    })
}

fn transform_maybe_promise(value: JsValue, transform: JsValue, allow_stage: bool) -> JsValue {
    if is_promise(value) {
        let callback = callback1(move |value| transform_loaded(value, transform, allow_stage));
        return call(value, "then", &[callback]);
    }
    transform_loaded(value, transform, allow_stage)
}

fn transform_loaded(value: JsValue, transform: JsValue, allow_stage: bool) -> JsValue {
    if allow_stage && value.is_function() {
        return callback0(move || {
            let result = invoke(value, JsValue::undefined(), &[]);
            transform_maybe_promise(result, transform, false)
        });
    }
    invoke(transform, JsValue::undefined(), &[value])
}

/// Builds the exact 42-key source object in declared order.
#[must_use]
pub fn make_sources() -> JsValue {
    let sources = object();
    let screen_state = crate::ScreenFrameState::new();
    for (index, name) in SOURCE_NAMES.iter().enumerate() {
        let source = source_callback(index, screen_state.clone());
        let descriptor = object();
        set(
            descriptor,
            "value",
            JsValue::erase(SOURCE_FUNCTION_NAMES[index]),
        );
        set(descriptor, "configurable", JsValue::erase(true));
        define_property(source, "name", descriptor);
        let length_descriptor = object();
        let source_length = if *name == "webGlBasics" || *name == "webGlExtensions" {
            1_f64
        } else {
            0_f64
        };
        set(length_descriptor, "value", JsValue::erase(source_length));
        set(length_descriptor, "configurable", JsValue::erase(true));
        define_property(source, "length", length_descriptor);
        set(sources, name, source);
    }
    sources
}

fn source_callback(index: usize, screen_state: crate::ScreenFrameState) -> JsValue {
    macro_rules! source {
        ($function:path) => {{
            callback1(move |options| $function(options))
        }};
    }
    match index {
        0 => source!(crate::source_user_agent_data),
        1 => source!(crate::source_fonts),
        2 => source!(crate::source_dom_blockers),
        3 => source!(crate::source_font_preferences),
        4 => source!(crate::source_audio),
        5 => callback1(move |options| {
            crate::source_screen_frame_with_state(options, screen_state.clone())
        }),
        6 => source!(crate::source_canvas_for_collection),
        7 => source!(crate::source_os_cpu),
        8 => source!(crate::source_languages),
        9 => source!(crate::source_color_depth),
        10 => source!(crate::source_device_memory),
        11 => source!(crate::source_screen_resolution),
        12 => source!(crate::source_hardware_concurrency),
        13 => source!(crate::source_timezone),
        14 => source!(crate::source_session_storage),
        15 => source!(crate::source_local_storage),
        16 => source!(crate::source_indexed_db),
        17 => source!(crate::source_open_database),
        18 => source!(crate::source_cpu_class),
        19 => source!(crate::source_platform),
        20 => source!(crate::source_plugins),
        21 => source!(crate::source_touch_support),
        22 => source!(crate::source_vendor),
        23 => source!(crate::source_vendor_flavors),
        24 => source!(crate::source_cookies_enabled),
        25 => source!(crate::source_color_gamut),
        26 => source!(crate::source_inverted_colors),
        27 => source!(crate::source_forced_colors),
        28 => source!(crate::source_monochrome),
        29 => source!(crate::source_contrast),
        30 => source!(crate::source_reduced_motion),
        31 => source!(crate::source_reduced_transparency),
        32 => source!(crate::source_hdr),
        33 => source!(crate::source_math),
        34 => source!(crate::source_pdf_viewer_enabled),
        35 => source!(crate::source_architecture),
        36 => source!(crate::source_apple_pay),
        37 => source!(crate::source_private_click_measurement),
        38 => source!(crate::source_audio_base_latency),
        39 => source!(crate::source_date_time_locale),
        40 => callback1(crate::source_web_gl_basics),
        41 => callback1(crate::source_web_gl_extensions),
        _ => JsValue::undefined(),
    }
}
