//! The DOM operations shared by layout-based entropy sources.

use alloc::string::String;
use alloc::{format, rc::Rc};
use core::cell::Cell;

use mires_js::{
    await_promise, callback0, callback1, callback2, construct, future_to_promise, invoke, try_host,
    JsValue,
};

use crate::{
    array, call, document, get, global_this, has, is_function_native, number_value, push, set,
    set_timeout, string_value, window,
};

/// Returns whether a host value is either `null` or `undefined`.
pub fn is_nullish(value: JsValue) -> bool {
    value.is_null() || value.is_undefined()
}

/// Converts a host value using JavaScript's `String` constructor.
pub fn host_string(value: JsValue) -> &'static str {
    string_value(value).read()
}

/// Creates a native resolved Promise without handwritten JavaScript.
pub fn resolved_promise(value: JsValue) -> JsValue {
    call(get(global_this(), "Promise"), "resolve", &[value])
}

/// Waits with the browser's `setTimeout` task queue.
pub(crate) async fn wait_dom(milliseconds: f64) -> Result<(), JsValue> {
    let executor = callback2(move |resolve, _reject| {
        let callback = callback0(move || {
            let _ = invoke(resolve, JsValue::undefined(), &[JsValue::undefined()]);
            JsValue::undefined()
        });
        let _ = set_timeout(callback, milliseconds);
        JsValue::undefined()
    });
    let promise = construct(get(global_this(), "Promise"), &[executor]);
    await_promise(promise).await.map(|_| ())
}

/// Creates an element through a specific document.
pub fn create_element(document: JsValue, tag: &str) -> JsValue {
    call(document, "createElement", &[JsValue::erase(tag)])
}

/// Appends one child and returns the child, matching `appendChild`.
pub fn append_child(parent: JsValue, child: JsValue) -> JsValue {
    call(parent, "appendChild", &[child])
}

/// Removes a node when it still has a parent.
pub fn remove_from_parent(node: JsValue) {
    let parent = get(node, "parentNode");
    if !is_nullish(parent) {
        let _ = call(parent, "removeChild", &[node]);
    }
}

/// Sets one CSS property with the host's `CSSStyleDeclaration.setProperty`.
pub fn set_style_property(style: JsValue, name: &str, value: &str, priority: &str) {
    let _ = call(
        style,
        "setProperty",
        &[
            JsValue::erase(name),
            JsValue::erase(value),
            JsValue::erase(priority),
        ],
    );
}

/// Adds declarations from the limited style strings used by blocker selectors.
pub fn add_style_string(style: JsValue, source: &str) {
    let source_bytes = source.as_bytes();
    let mut declaration_start = 0;
    while declaration_start <= source_bytes.len() {
        let mut declaration_end = declaration_start;
        while declaration_end < source_bytes.len() && source_bytes[declaration_end] != b';' {
            declaration_end += 1;
        }
        let declaration = &source[declaration_start..declaration_end];
        declaration_start = declaration_end + 1;
        let declaration_bytes = declaration.as_bytes();
        let mut colon = 0;
        while colon < declaration_bytes.len() && declaration_bytes[colon] != b':' {
            colon += 1;
        }
        if colon == declaration_bytes.len() {
            continue;
        }
        let name = &declaration[..colon];
        let raw_value = &declaration[colon + 1..];
        let name = name.trim();
        let name_bytes = name.as_bytes();
        let mut valid_name = !name_bytes.is_empty();
        let mut name_index = 0;
        while name_index < name_bytes.len() {
            let byte = name_bytes[name_index];
            if !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-') {
                valid_name = false;
                break;
            }
            name_index += 1;
        }
        if !valid_name {
            continue;
        }
        let raw_value = raw_value.trim();
        if raw_value.is_empty() {
            continue;
        }
        let (value, priority) = raw_value
            .strip_suffix("!important")
            .map_or((raw_value, ""), |value| (value.trim_end(), "important"));
        set_style_property(style, name, value, priority);
    }
}

/// Invokes an arbitrary host action inside a temporary hidden iframe.
pub fn with_iframe(action: JsValue, initial_html: JsValue, poll_interval: JsValue) -> JsValue {
    future_to_promise(async move {
        let milliseconds = if poll_interval.is_undefined() {
            50.0
        } else {
            number_value(poll_interval)
        };
        with_iframe_async(action, initial_html, milliseconds).await
    })
}

/// Implements the asynchronous iframe lifetime for Rust-created and arbitrary actions.
pub async fn with_iframe_async(
    action: JsValue,
    initial_html: JsValue,
    poll_interval: f64,
) -> Result<JsValue, JsValue> {
    let outer_document = document();
    while is_nullish(get(outer_document, "body")) {
        wait_dom(poll_interval).await?;
    }

    let iframe = create_element(outer_document, "iframe");
    let load = iframe_load_promise(iframe, outer_document, initial_html);
    let loaded = await_promise(load).await;
    if let Err(error) = loaded {
        remove_from_parent(iframe);
        return Err(error);
    }

    let iframe_window = loop {
        let value = get(iframe, "contentWindow");
        if !is_nullish(value) {
            let iframe_document = get(value, "document");
            if !is_nullish(iframe_document) && !is_nullish(get(iframe_document, "body")) {
                break value;
            }
        }
        wait_dom(poll_interval).await?;
    };

    let action_result =
        match try_host(move || invoke(action, JsValue::undefined(), &[iframe, iframe_window])) {
            Ok(value) => await_promise(value).await,
            Err(error) => Err(error),
        };
    remove_from_parent(iframe);
    action_result
}

fn iframe_load_promise(iframe: JsValue, outer_document: JsValue, initial_html: JsValue) -> JsValue {
    let executor = callback2(move |resolve, reject| {
        let complete = Rc::new(Cell::new(false));
        let resolve_complete = Rc::clone(&complete);
        let resolve_once = callback0(move || {
            if !resolve_complete.replace(true) {
                let _ = invoke(resolve, JsValue::undefined(), &[JsValue::undefined()]);
            }
            JsValue::undefined()
        });
        let reject_complete = Rc::clone(&complete);
        let reject_once = callback1(move |error| {
            if !reject_complete.replace(true) {
                let _ = invoke(reject, JsValue::undefined(), &[error]);
            }
            JsValue::undefined()
        });

        let setup = try_host(move || {
            set(iframe, "onload", resolve_once);
            set(iframe, "onerror", reject_once);
            let style = get(iframe, "style");
            set_style_property(style, "display", "block", "important");
            set(style, "position", JsValue::erase("absolute"));
            set(style, "top", JsValue::erase("0"));
            set(style, "left", JsValue::erase("0"));
            set(style, "visibility", JsValue::erase("hidden"));
            if initial_html.is_truthy() && has(iframe, "srcdoc") {
                set(iframe, "srcdoc", initial_html);
            } else {
                set(iframe, "src", JsValue::erase("about:blank"));
            }
            let body = get(outer_document, "body");
            let _ = append_child(body, iframe);
            check_iframe_ready(iframe, Rc::clone(&complete), resolve_once);
            JsValue::undefined()
        });
        if let Err(error) = setup {
            let _ = invoke(reject_once, JsValue::undefined(), &[error]);
        }
        JsValue::undefined()
    });
    construct(get(global_this(), "Promise"), &[executor])
}

fn check_iframe_ready(iframe: JsValue, complete: Rc<Cell<bool>>, resolve: JsValue) {
    if complete.get() {
        return;
    }
    let iframe_window = get(iframe, "contentWindow");
    if !is_nullish(iframe_window) {
        let iframe_document = get(iframe_window, "document");
        if !is_nullish(iframe_document)
            && host_string(get(iframe_document, "readyState")) == "complete"
        {
            let _ = invoke(resolve, JsValue::undefined(), &[]);
            return;
        }
    }
    let callback = callback0(move || {
        check_iframe_ready(iframe, complete.clone(), resolve);
        JsValue::undefined()
    });
    let _ = set_timeout(callback, 10.0);
}

/// Returns whether the current browser matches FingerprintJS's Chromium probe.
pub fn dom_is_chromium() -> bool {
    let host_window = window();
    let navigator = get(host_window, "navigator");
    [
        has(navigator, "webkitPersistentStorage"),
        has(navigator, "webkitTemporaryStorage"),
        host_string(get(navigator, "vendor")).starts_with("Google"),
        has(host_window, "webkitResolveLocalFileSystemURL"),
        has(host_window, "BatteryManager"),
        has(host_window, "webkitMediaStream"),
        has(host_window, "webkitSpeechGrammar"),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 5
}

/// Returns whether the current browser matches FingerprintJS's WebKit probe.
pub fn dom_is_webkit() -> bool {
    let host_window = window();
    let navigator = get(host_window, "navigator");
    [
        has(host_window, "ApplePayError"),
        has(host_window, "CSSPrimitiveValue"),
        has(host_window, "Counter"),
        host_string(get(navigator, "vendor")).starts_with("Apple"),
        has(host_window, "RGBColor"),
        has(host_window, "WebKitMediaKeys"),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 4
}

/// Returns whether the current browser matches FingerprintJS's Gecko probe.
pub fn dom_is_gecko() -> bool {
    let host_window = window();
    let navigator = get(host_window, "navigator");
    let root = get(document(), "documentElement");
    let moz_appearance = !is_nullish(root) && has(get(root, "style"), "MozAppearance");
    [
        has(navigator, "buildID"),
        moz_appearance,
        has(host_window, "onmozfullscreenchange"),
        has(host_window, "mozInnerScreenX"),
        has(host_window, "CSSMozDocumentRule"),
        has(host_window, "CanvasCaptureMediaStream"),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 4
}

/// Returns whether Chromium's version probes match version 128 or newer.
pub fn dom_is_chromium_128_or_newer() -> bool {
    let host_window = window();
    let promise = get(host_window, "Promise");
    let audio_context = get(host_window, "AudioContext");
    let css = get(host_window, "CSS");
    [
        !is_nullish(promise) && has(promise, "try"),
        has(document(), "caretPositionFromPoint"),
        !is_nullish(audio_context) && has(get(audio_context, "prototype"), "onerror"),
        call(
            css,
            "supports",
            &[JsValue::erase("ruby-align"), JsValue::erase("space-around")],
        )
        .is_truthy(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 3
}

/// Returns whether WebKit's version probes match Safari 17 or newer.
pub fn dom_is_webkit_616_or_newer() -> bool {
    let host_window = window();
    let navigator = get(host_window, "navigator");
    let button = get(host_window, "HTMLButtonElement");
    let css = get(host_window, "CSS");
    [
        !has(navigator, "getStorageUpdates"),
        !is_nullish(button) && has(get(button, "prototype"), "popover"),
        has(host_window, "CSSCounterStyleRule"),
        call(
            css,
            "supports",
            &[JsValue::erase("font-size-adjust: ex-height 0.5")],
        )
        .is_truthy(),
        call(
            css,
            "supports",
            &[JsValue::erase("text-transform: full-width")],
        )
        .is_truthy(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 4
}

/// Returns whether Gecko's version probes match Firefox 143 or newer.
pub fn dom_is_gecko_143_or_newer() -> bool {
    let host_window = window();
    let css = get(host_window, "CSS");
    let composition_event = get(host_window, "CompositionEvent");
    [
        css_supports(css, "selector(::details-content)"),
        css_supports(css, "selector(::before::marker)"),
        css_supports(css, "selector(::after::marker)"),
        !is_nullish(composition_event) && !has(get(composition_event, "prototype"), "locale"),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        >= 3
}

fn css_supports(css: JsValue, value: &str) -> bool {
    call(css, "supports", &[JsValue::erase(value)]).is_truthy()
}

/// Returns whether the current WebKit browser is Safari 15.4 or newer.
pub fn dom_is_safari_webkit() -> bool {
    let host_window = window();
    let print = get(host_window, "print");
    if !print.is_function() {
        return false;
    }
    let function = get(global_this(), "Function");
    let to_string = get(get(function, "prototype"), "toString");
    let source = invoke(to_string, print, &[]);
    is_function_native(host_string(source))
        && host_string(get(host_window, "browser")) == "[object WebPageNamespace]"
}

/// Returns whether the current browser matches FingerprintJS's Android probe.
pub fn dom_is_android() -> bool {
    let host_window = window();
    let navigator = get(host_window, "navigator");
    if dom_is_chromium() {
        let connection = get(navigator, "connection");
        let audio_constructor = get(host_window, "Audio");
        let audio = construct(audio_constructor, &[]);
        return [
            !has(host_window, "SharedWorker"),
            connection.is_truthy() && has(connection, "ontypechange"),
            !has(audio, "sinkId"),
        ]
        .into_iter()
        .filter(|value| *value)
        .count()
            >= 2;
    }
    if dom_is_gecko() {
        let app_version = host_string(get(navigator, "appVersion")).to_ascii_lowercase();
        return [
            has(host_window, "onorientationchange"),
            has(host_window, "orientation"),
            app_version.contains("android"),
        ]
        .into_iter()
        .filter(|value| *value)
        .count()
            >= 2;
    }
    false
}

/// Returns the current fullscreen element using every upstream vendor fallback.
pub fn fullscreen_element() -> JsValue {
    let host_document = document();
    for name in [
        "fullscreenElement",
        "msFullscreenElement",
        "mozFullScreenElement",
        "webkitFullscreenElement",
    ] {
        let value = get(host_document, name);
        if value.is_truthy() {
            return value;
        }
    }
    JsValue::null()
}

/// Exits fullscreen with the owning document as `this`.
pub(crate) fn exit_fullscreen_dom() -> Result<JsValue, JsValue> {
    let host_document = document();
    let mut function = JsValue::undefined();
    for name in [
        "exitFullscreen",
        "msExitFullscreen",
        "mozCancelFullScreen",
        "webkitExitFullscreen",
    ] {
        let candidate = get(host_document, name);
        if candidate.is_truthy() {
            function = candidate;
            break;
        }
    }
    try_host(move || invoke(function, host_document, &[]))
}

/// Builds the repeated words used to expose natural font sizing.
pub fn natural_font_text(container_width: usize) -> String {
    let count = container_width / 20;
    let mut output = String::new();
    for index in 0..count {
        if index > 0 {
            output.push(' ');
        }
        output.push_str("word");
    }
    output
}

/// Returns a native array containing four optional frame sides.
pub fn frame_array(frame: [Option<f64>; 4]) -> JsValue {
    let output = array();
    for value in frame {
        push(output, value.map_or_else(JsValue::null, JsValue::erase));
    }
    output
}

/// Formats an integer CSS pixel width.
pub fn pixel_width(value: usize) -> String {
    format!("{value}px")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_natural_font_filler_exactly() {
        let text = natural_font_text(4_000);
        assert_eq!(text.split(' ').count(), 200);
        assert_eq!(text.len(), 999);
        assert!(text.starts_with("word word"));
        assert!(text.ends_with("word"));
    }
}
