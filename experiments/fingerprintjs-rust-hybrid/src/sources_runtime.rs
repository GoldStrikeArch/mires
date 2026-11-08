//! Direct host implementations of the inexpensive synchronous entropy sources.

use alloc::string::String;
use mires_js::{construct, try_host, JsValue};

use crate::{
    array, call, clear_timeout, date_now, document, get, has, index, is_android, is_chromium,
    is_chromium_86_or_newer, is_desktop_webkit, is_edge_html, is_finite, is_gecko,
    is_gecko_143_or_newer, is_ipad, is_safari_webkit, is_trident, is_webkit,
    is_webkit_616_or_newer, length, navigator, object, parse_float, parse_int, push,
    set, sort, string_value, try_call, window,
};

fn undefined(value: JsValue) -> Option<JsValue> {
    (!value.is_undefined()).then_some(value)
}

fn media(query: &str) -> bool {
    crate::match_media_matches(query)
}

fn bool_or_undefined(yes: &str, no: &str) -> JsValue {
    if media(yes) {
        JsValue::erase(true)
    } else if media(no) {
        JsValue::erase(false)
    } else {
        JsValue::undefined()
    }
}

/// `navigator.oscpu`.
#[must_use]
pub fn source_os_cpu(_: JsValue) -> JsValue {
    crate::navigator_get("oscpu")
}

/// `navigator.language` and `navigator.languages` with Chromium's private-mode stabilization.
#[must_use]
pub fn source_languages(_: JsValue) -> JsValue {
    let output = array();
    let mut language = crate::navigator_get("language");
    if language.is_undefined() {
        language = crate::navigator_get("userLanguage");
    }
    if language.is_undefined() {
        language = crate::navigator_get("browserLanguage");
    }
    if language.is_undefined() {
        language = crate::navigator_get("systemLanguage");
    }
    if !language.is_undefined() {
        let singleton = array();
        push(singleton, language);
        push(output, singleton);
    }
    let languages = crate::navigator_get("languages");
    if crate::is_array(languages) {
        if !(is_chromium() && is_chromium_86_or_newer()) {
            push(output, languages);
        }
    } else if languages.is_string() && languages.is_truthy() {
        push(output, call(languages, "split", &[JsValue::erase(",")]));
    }
    output
}

/// `screen.colorDepth`.
#[must_use]
pub fn source_color_depth(_: JsValue) -> JsValue {
    crate::screen_get("colorDepth")
}

/// Normalized `navigator.deviceMemory`.
#[must_use]
pub fn source_device_memory(_: JsValue) -> JsValue {
    let parsed = parse_float(crate::navigator_get("deviceMemory"));
    if parsed.is_nan() {
        JsValue::undefined()
    } else {
        JsValue::erase(parsed)
    }
}

/// Stabilized screen resolution.
#[must_use]
pub fn source_screen_resolution(_: JsValue) -> JsValue {
    if is_webkit() && is_webkit_616_or_newer() && is_safari_webkit() {
        return JsValue::undefined();
    }
    get_unstable_screen_resolution()
}

/// Unstabilized screen resolution helper.
#[must_use]
pub fn get_unstable_screen_resolution() -> JsValue {
    let dimensions = array();
    for name in ["width", "height"] {
        let value = parse_int(crate::screen_get(name));
        push(
            dimensions,
            if value.is_nan() {
                JsValue::null()
            } else {
                JsValue::erase(value)
            },
        );
    }
    sort(dimensions);
    let _ = call(dimensions, "reverse", &[]);
    dimensions
}

/// Stabilized processor-count signal.
#[must_use]
pub fn source_hardware_concurrency(_: JsValue) -> JsValue {
    let value = get_unstable_hardware_concurrency();
    if value.is_undefined() {
        return value;
    }
    if is_gecko() && is_gecko_143_or_newer() {
        let number: f64 = value.read();
        return JsValue::erase(if number >= 8.0 { 8.0 } else { 4.0 });
    }
    value
}

/// Unstabilized processor-count helper.
#[must_use]
pub fn get_unstable_hardware_concurrency() -> JsValue {
    let parsed = parse_int(crate::navigator_get("hardwareConcurrency"));
    if parsed.is_nan() {
        JsValue::undefined()
    } else {
        JsValue::erase(parsed)
    }
}

/// IANA time zone or the non-DST numeric fallback.
#[must_use]
pub fn source_timezone(_: JsValue) -> JsValue {
    let intl = get(window(), "Intl");
    let formatter = get(intl, "DateTimeFormat");
    if formatter.is_function() {
        let instance = mires_js::invoke(formatter, JsValue::undefined(), &[]);
        let zone = get(call(instance, "resolvedOptions", &[]), "timeZone");
        if zone.is_truthy() {
            return zone;
        }
    }
    let date = get(window(), "Date");
    let now = construct(date, &[]);
    let year = call(now, "getFullYear", &[]);
    let january = construct(date, &[year, JsValue::erase(0.0), JsValue::erase(1.0)]);
    let july = construct(date, &[year, JsValue::erase(6.0), JsValue::erase(1.0)]);
    let first = parse_float(call(january, "getTimezoneOffset", &[]));
    let second = parse_float(call(july, "getTimezoneOffset", &[]));
    let offset = -first.max(second);
    let mut value = String::from("UTC");
    if offset >= 0.0 {
        value.push('+');
    }
    value.push_str(string_value(JsValue::erase(offset)).read());
    JsValue::erase(value.as_str())
}

fn storage(name: &'static str) -> JsValue {
    let found = try_host(move || get(window(), name));
    JsValue::erase(match found {
        Ok(value) => value.is_truthy(),
        Err(_) => true,
    })
}

/// Session storage presence, including SecurityError semantics.
#[must_use]
pub fn source_session_storage(_: JsValue) -> JsValue {
    storage("sessionStorage")
}

/// Local storage presence, including SecurityError semantics.
#[must_use]
pub fn source_local_storage(_: JsValue) -> JsValue {
    storage("localStorage")
}

/// IndexedDB presence, excluding Trident and EdgeHTML.
#[must_use]
pub fn source_indexed_db(_: JsValue) -> JsValue {
    if is_trident() || is_edge_html() {
        return JsValue::undefined();
    }
    storage("indexedDB")
}

/// Legacy WebSQL API presence.
#[must_use]
pub fn source_open_database(_: JsValue) -> JsValue {
    JsValue::erase(get(window(), "openDatabase").is_truthy())
}

/// `navigator.cpuClass`.
#[must_use]
pub fn source_cpu_class(_: JsValue) -> JsValue {
    crate::navigator_get("cpuClass")
}

/// Stabilized platform string.
#[must_use]
pub fn source_platform(_: JsValue) -> JsValue {
    let platform = crate::navigator_get("platform");
    if platform.is_string()
        && platform.read::<&str>() == "MacIntel"
        && is_webkit()
        && !is_desktop_webkit()
    {
        JsValue::erase(if is_ipad() { "iPad" } else { "iPhone" })
    } else {
        platform
    }
}

/// Browser plugin metadata.
#[must_use]
pub fn source_plugins(_: JsValue) -> JsValue {
    let raw = crate::navigator_get("plugins");
    if raw.is_undefined() || raw.is_null() {
        return JsValue::undefined();
    }
    let output = array();
    for plugin_index in 0..length(raw) {
        let plugin = index(raw, plugin_index);
        if plugin.is_undefined() || plugin.is_null() {
            continue;
        }
        let mime_types = array();
        for mime_index in 0..length(plugin) {
            let mime = index(plugin, mime_index);
            let value = object();
            set(value, "type", get(mime, "type"));
            set(value, "suffixes", get(mime, "suffixes"));
            push(mime_types, value);
        }
        let value = object();
        set(value, "name", get(plugin, "name"));
        set(value, "description", get(plugin, "description"));
        set(value, "mimeTypes", mime_types);
        push(output, value);
    }
    output
}

/// Crude touch support signal.
#[must_use]
pub fn source_touch_support(_: JsValue) -> JsValue {
    let n = navigator();
    let points = crate::navigator_get("maxTouchPoints");
    let maximum = if points.is_undefined() {
        undefined(get(n, "msMaxTouchPoints")).unwrap_or_else(|| JsValue::erase(0.0))
    } else {
        JsValue::erase(parse_int(points))
    };
    let touch_event = try_call(document(), "createEvent", &[JsValue::erase("TouchEvent")]).is_ok();
    let value = object();
    set(value, "maxTouchPoints", maximum);
    set(value, "touchEvent", JsValue::erase(touch_event));
    set(
        value,
        "touchStart",
        JsValue::erase(has(window(), "ontouchstart")),
    );
    value
}

/// `navigator.vendor`, normalized to an empty string.
#[must_use]
pub fn source_vendor(_: JsValue) -> JsValue {
    let value = crate::navigator_get("vendor");
    if value.is_truthy() {
        value
    } else {
        JsValue::erase("")
    }
}

/// Browser-specific object globals, sorted by JavaScript string order.
#[must_use]
pub fn source_vendor_flavors(_: JsValue) -> JsValue {
    let output = array();
    for key in [
        "chrome",
        "safari",
        "__crWeb",
        "__gCrWeb",
        "yandex",
        "__yb",
        "__ybro",
        "__firefox__",
        "__edgeTrackingPreventionStatistics",
        "webkit",
        "oprt",
        "samsungAr",
        "ucweb",
        "UCShellJava",
        "puffinDevice",
    ] {
        let value = get(window(), key);
        if value.is_truthy() && value.is_object() {
            push(output, JsValue::erase(key));
        }
    }
    sort(output);
    output
}

/// Cookie availability, including throwing document accessors.
#[must_use]
pub fn source_cookies_enabled(_: JsValue) -> JsValue {
    let result = try_host(|| {
        let d = document();
        set(
            d,
            "cookie",
            JsValue::erase("cookietest=1; SameSite=Strict;"),
        );
        let found = get(d, "cookie").read::<&str>().contains("cookietest=");
        set(
            d,
            "cookie",
            JsValue::erase("cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT"),
        );
        JsValue::erase(found)
    });
    result.unwrap_or_else(|_| JsValue::erase(false))
}

/// Widest matching display color gamut.
#[must_use]
pub fn source_color_gamut(_: JsValue) -> JsValue {
    for gamut in ["rec2020", "p3", "srgb"] {
        if media(&alloc::format!("(color-gamut: {gamut})")) {
            return JsValue::erase(gamut);
        }
    }
    JsValue::undefined()
}

/// Inverted-colors media preference.
#[must_use]
pub fn source_inverted_colors(_: JsValue) -> JsValue {
    bool_or_undefined("(inverted-colors: inverted)", "(inverted-colors: none)")
}

/// Forced-colors media preference.
#[must_use]
pub fn source_forced_colors(_: JsValue) -> JsValue {
    bool_or_undefined("(forced-colors: active)", "(forced-colors: none)")
}

/// Monochrome display depth.
#[must_use]
pub fn source_monochrome(_: JsValue) -> JsValue {
    if !media("(min-monochrome: 0)") {
        return JsValue::undefined();
    }
    for depth in 0..=100 {
        if media(&alloc::format!("(max-monochrome: {depth})")) {
            return JsValue::erase(depth);
        }
    }
    mires_js::throw_value(crate::new_error("Error", "Too high value"))
}

/// Contrast media preference.
#[must_use]
pub fn source_contrast(_: JsValue) -> JsValue {
    if media("(prefers-contrast: no-preference)") {
        JsValue::erase(0_i32)
    } else if media("(prefers-contrast: high)") || media("(prefers-contrast: more)") {
        JsValue::erase(1_i32)
    } else if media("(prefers-contrast: low)") || media("(prefers-contrast: less)") {
        JsValue::erase(-1_i32)
    } else if media("(prefers-contrast: forced)") {
        JsValue::erase(10_i32)
    } else {
        JsValue::undefined()
    }
}

/// Reduced-motion media preference.
#[must_use]
pub fn source_reduced_motion(_: JsValue) -> JsValue {
    bool_or_undefined(
        "(prefers-reduced-motion: reduce)",
        "(prefers-reduced-motion: no-preference)",
    )
}

/// Reduced-transparency media preference.
#[must_use]
pub fn source_reduced_transparency(_: JsValue) -> JsValue {
    bool_or_undefined(
        "(prefers-reduced-transparency: reduce)",
        "(prefers-reduced-transparency: no-preference)",
    )
}

/// High-dynamic-range media preference.
#[must_use]
pub fn source_hdr(_: JsValue) -> JsValue {
    bool_or_undefined("(dynamic-range: high)", "(dynamic-range: standard)")
}

/// Native PDF-viewer flag.
#[must_use]
pub fn source_pdf_viewer_enabled(_: JsValue) -> JsValue {
    crate::navigator_get("pdfViewerEnabled")
}

/// Typed-array architecture probe.
#[must_use]
pub fn source_architecture(_: JsValue) -> JsValue {
    let float_constructor = get(window(), "Float32Array");
    let byte_constructor = get(window(), "Uint8Array");
    let floats = construct(float_constructor, &[JsValue::erase(1_usize)]);
    let bytes = construct(byte_constructor, &[get(floats, "buffer")]);
    crate::set_index(floats, 0, JsValue::erase(f64::INFINITY));
    let infinity: f64 = index(floats, 0).read();
    let another_infinity: f64 = index(floats, 0).read();
    crate::set_index(floats, 0, JsValue::erase(infinity - another_infinity));
    index(bytes, 3)
}

/// Private click measurement attribute string.
#[must_use]
pub fn source_private_click_measurement(_: JsValue) -> JsValue {
    let link = call(document(), "createElement", &[JsValue::erase("a")]);
    let modern = get(link, "attributionSourceId");
    let value = if modern.is_undefined() {
        get(link, "attributionsourceid")
    } else {
        modern
    };
    if value.is_undefined() {
        value
    } else {
        string_value(value)
    }
}

/// AudioContext base latency with platform warning exclusions.
#[must_use]
pub fn source_audio_base_latency(_: JsValue) -> JsValue {
    if !is_android() && !is_webkit() {
        return JsValue::erase(-2_i32);
    }
    let constructor = get(window(), "AudioContext");
    if !constructor.is_function() {
        return JsValue::erase(-1_i32);
    }
    let latency = get(construct(constructor, &[]), "baseLatency");
    if latency.is_null() || latency.is_undefined() {
        return JsValue::erase(-1_i32);
    }
    let number: f64 = latency.read();
    if is_finite(number) {
        latency
    } else {
        JsValue::erase(-3_i32)
    }
}

/// Intl date-time locale and its three status sentinels.
#[must_use]
pub fn source_date_time_locale(_: JsValue) -> JsValue {
    let intl = get(window(), "Intl");
    if intl.is_undefined() || intl.is_null() {
        return JsValue::erase(-1_i32);
    }
    let formatter = get(intl, "DateTimeFormat");
    if !formatter.is_function() {
        return JsValue::erase(-2_i32);
    }
    let instance = mires_js::invoke(formatter, JsValue::undefined(), &[]);
    let locale = get(call(instance, "resolvedOptions", &[]), "locale");
    if locale.is_null() || locale.is_undefined() {
        JsValue::erase(-3_i32)
    } else {
        locale
    }
}

/// Returns the source-start timestamp used by diagnostics.
#[must_use]
pub fn source_clock() -> f64 {
    date_now()
}

/// Cancels a timeout by its numeric identifier.
pub fn cancel_source_timeout(identifier: f64) {
    clear_timeout(identifier);
}
