//! Feature-based browser detection against native host objects.

use mires_js::{construct, JsValue};

use crate::{
    call, detect_android, detect_chromium, detect_chromium_122_or_newer,
    detect_chromium_128_or_newer, detect_chromium_86_or_newer, detect_desktop_webkit,
    detect_edge_html, detect_gecko, detect_gecko_120_or_newer, detect_gecko_143_or_newer,
    detect_ipad, detect_safari_webkit, detect_samsung_internet, detect_trident, detect_webkit,
    detect_webkit_606_or_newer, detect_webkit_616_or_newer, document, get, has, is_function_native,
    navigator, screen, string_value, window,
};

fn present(value: JsValue) -> bool {
    !value.is_undefined() && !value.is_null()
}

fn supports(arguments: &[JsValue]) -> bool {
    let css = get(window(), "CSS");
    present(css) && call(css, "supports", arguments).read()
}

fn prototype(global: &str) -> Option<JsValue> {
    let constructor = get(window(), global);
    present(constructor).then(|| get(constructor, "prototype"))
}

fn starts_with(value: JsValue, prefix: &str) -> bool {
    present(value) && value.read::<&str>().starts_with(prefix)
}

fn ascii_contains_ignore_case(haystack: &str, needle: &[u8]) -> bool {
    let hay = haystack.as_bytes();
    if hay.len() < needle.len() {
        return false;
    }
    let mut index = 0;
    while index + needle.len() <= hay.len() {
        let mut matched = 0;
        while matched < needle.len() && hay[index + matched].to_ascii_lowercase() == needle[matched]
        {
            matched += 1;
        }
        if matched == needle.len() {
            return true;
        }
        index += 1;
    }
    false
}

/// Checks for the Trident engine without reading the user agent.
#[must_use]
pub fn is_trident() -> bool {
    let w = window();
    let n = navigator();
    detect_trident([
        has(w, "MSCSSMatrix"),
        has(w, "msSetImmediate"),
        has(w, "msIndexedDB"),
        has(n, "msMaxTouchPoints"),
        has(n, "msPointerEnabled"),
    ])
}

/// Checks for the legacy EdgeHTML engine without reading the user agent.
#[must_use]
pub fn is_edge_html() -> bool {
    let w = window();
    let n = navigator();
    detect_edge_html(
        [
            has(w, "msWriteProfilerMark"),
            has(w, "MSStream"),
            has(n, "msLaunchUri"),
            has(n, "msSaveBlob"),
        ],
        is_trident(),
    )
}

/// Checks for the Chromium engine without reading the user agent.
#[must_use]
pub fn is_chromium() -> bool {
    let w = window();
    let n = navigator();
    detect_chromium([
        has(n, "webkitPersistentStorage"),
        has(n, "webkitTemporaryStorage"),
        starts_with(get(n, "vendor"), "Google"),
        has(w, "webkitResolveLocalFileSystemURL"),
        has(w, "BatteryManager"),
        has(w, "webkitMediaStream"),
        has(w, "webkitSpeechGrammar"),
    ])
}

/// Checks for the WebKit engine without reading the user agent.
#[must_use]
pub fn is_webkit() -> bool {
    let w = window();
    let n = navigator();
    detect_webkit([
        has(w, "ApplePayError"),
        has(w, "CSSPrimitiveValue"),
        has(w, "Counter"),
        starts_with(get(n, "vendor"), "Apple"),
        has(w, "RGBColor"),
        has(w, "WebKitMediaKeys"),
    ])
}

/// Checks whether a WebKit browser is desktop-class.
#[must_use]
pub fn is_desktop_webkit() -> bool {
    let w = window();
    let html_element = prototype("HTMLElement");
    let document_prototype = prototype("Document");
    detect_desktop_webkit([
        has(w, "safari"),
        !has(w, "ongestureend"),
        !has(w, "TouchEvent"),
        !has(w, "orientation"),
        html_element.is_some_and(|value| !has(value, "autocapitalize")),
        document_prototype.is_some_and(|value| has(value, "pointerLockElement")),
    ])
}

/// Checks whether a modern WebKit browser is Safari.
#[must_use]
pub fn is_safari_webkit() -> bool {
    let w = window();
    let print = get(w, "print");
    let native = present(print) && is_function_native(string_value(print).read());
    let namespace = string_value(get(w, "browser")).read::<&str>() == "[object WebPageNamespace]";
    detect_safari_webkit(native, namespace)
}

/// Checks for the Gecko engine without reading the user agent.
#[must_use]
pub fn is_gecko() -> bool {
    let w = window();
    let root = get(document(), "documentElement");
    let style = if present(root) {
        get(root, "style")
    } else {
        JsValue::undefined()
    };
    detect_gecko([
        has(navigator(), "buildID"),
        present(style) && has(style, "MozAppearance"),
        has(w, "onmozfullscreenchange"),
        has(w, "mozInnerScreenX"),
        has(w, "CSSMozDocumentRule"),
        has(w, "CanvasCaptureMediaStream"),
    ])
}

/// Checks Gecko's version-120 feature set.
#[must_use]
pub fn is_gecko_120_or_newer() -> bool {
    detect_gecko_120_or_newer([
        has(navigator(), "userActivation"),
        supports(&[
            JsValue::erase("color"),
            JsValue::erase("light-dark(#000, #fff)"),
        ]),
        supports(&[JsValue::erase("height"), JsValue::erase("1lh")]),
        has(navigator(), "globalPrivacyControl"),
    ])
}

/// Checks Gecko's version-143 feature set.
#[must_use]
pub fn is_gecko_143_or_newer() -> bool {
    let composition = prototype("CompositionEvent");
    detect_gecko_143_or_newer([
        supports(&[JsValue::erase("selector(::details-content)")]),
        supports(&[JsValue::erase("selector(::before::marker)")]),
        supports(&[JsValue::erase("selector(::after::marker)")]),
        composition.is_some_and(|value| !has(value, "locale")),
    ])
}

/// Checks Chromium's version-86 feature set.
#[must_use]
pub fn is_chromium_86_or_newer() -> bool {
    let w = window();
    detect_chromium_86_or_newer([
        !has(w, "MediaSettingsRange"),
        has(w, "RTCEncodedAudioFrame"),
        string_value(get(w, "Intl")).read::<&str>() == "[object Intl]",
        string_value(get(w, "Reflect")).read::<&str>() == "[object Reflect]",
    ])
}

/// Checks Chromium's version-122 feature set.
#[must_use]
pub fn is_chromium_122_or_newer() -> bool {
    let url_pattern = prototype("URLPattern");
    let webgl = prototype("WebGLRenderingContext");
    let set_prototype = prototype("Set");
    detect_chromium_122_or_newer([
        set_prototype.is_some_and(|value| has(value, "union")),
        has(window(), "Iterator"),
        url_pattern.is_some_and(|value| has(value, "hasRegExpGroups")),
        webgl.is_some_and(|value| has(value, "RGB8")),
    ])
}

/// Checks Chromium's version-128 feature set.
#[must_use]
pub fn is_chromium_128_or_newer() -> bool {
    let promise = get(window(), "Promise");
    let audio = prototype("AudioContext");
    detect_chromium_128_or_newer([
        present(promise) && has(promise, "try"),
        has(document(), "caretPositionFromPoint"),
        audio.is_some_and(|value| has(value, "onerror")),
        supports(&[JsValue::erase("ruby-align"), JsValue::erase("space-around")]),
    ])
}

/// Checks WebKit's version-606 feature set.
#[must_use]
pub fn is_webkit_606_or_newer() -> bool {
    let w = window();
    detect_webkit_606_or_newer([
        has(w, "DOMRectList"),
        has(w, "RTCPeerConnectionIceEvent"),
        has(w, "SVGGeometryElement"),
        has(w, "ontransitioncancel"),
    ])
}

/// Checks WebKit's version-616 feature set.
#[must_use]
pub fn is_webkit_616_or_newer() -> bool {
    let w = window();
    let button = prototype("HTMLButtonElement");
    detect_webkit_616_or_newer([
        !has(navigator(), "getStorageUpdates"),
        button.is_some_and(|value| has(value, "popover")),
        has(w, "CSSCounterStyleRule"),
        supports(&[JsValue::erase("font-size-adjust: ex-height 0.5")]),
        supports(&[JsValue::erase("text-transform: full-width")]),
    ])
}

/// Checks the device features that distinguish an iPad.
#[must_use]
pub fn is_ipad() -> bool {
    let n = navigator();
    let platform_is_ipad = get(n, "platform").read::<&str>() == "iPad";
    let s = screen();
    let ratio = get(s, "width").read::<f64>() / get(s, "height").read::<f64>();
    let element = prototype("Element");
    detect_ipad(
        platform_is_ipad,
        [
            has(window(), "MediaSource"),
            element.is_some_and(|value| get(value, "webkitRequestFullscreen").is_truthy()),
            ratio > 0.65 && ratio < 1.53,
        ],
    )
}

/// Returns the browser's current prefixed or unprefixed fullscreen element.
#[must_use]
pub fn get_fullscreen_element() -> JsValue {
    let d = document();
    for name in [
        "fullscreenElement",
        "msFullscreenElement",
        "mozFullScreenElement",
        "webkitFullscreenElement",
    ] {
        let value = get(d, name);
        if value.is_truthy() {
            return value;
        }
    }
    JsValue::null()
}

/// Exits prefixed or unprefixed fullscreen while retaining the document receiver.
#[must_use]
pub fn exit_fullscreen() -> JsValue {
    let d = document();
    for name in [
        "exitFullscreen",
        "msExitFullscreen",
        "mozCancelFullScreen",
        "webkitExitFullscreen",
    ] {
        let function = get(d, name);
        if function.is_truthy() {
            return mires_js::invoke(function, d, &[]);
        }
    }
    JsValue::undefined()
}

/// Checks for Android without reading the user agent string on Chromium.
#[must_use]
pub fn is_android() -> bool {
    let chromium = is_chromium();
    let gecko = is_gecko();
    let w = window();
    let n = navigator();
    let connection = get(n, "connection");
    let audio_constructor = get(w, "Audio");
    let has_sink = if present(audio_constructor) {
        has(construct(audio_constructor, &[]), "sinkId")
    } else {
        false
    };
    let app_version = get(n, "appVersion");
    let mentions_android = present(app_version)
        && ascii_contains_ignore_case(app_version.read::<&str>(), b"android");
    detect_android(
        chromium,
        gecko,
        [
            !has(w, "SharedWorker"),
            present(connection) && has(connection, "ontypechange"),
            !has_sink,
        ],
        [
            has(w, "onorientationchange"),
            has(w, "orientation"),
            mentions_android,
        ],
    )
}

/// Checks for Samsung Internet after Chromium has been detected.
#[must_use]
pub fn is_samsung_internet() -> bool {
    let audio = prototype("Audio");
    let image = prototype("Image");
    let viewport = get(window(), "visualViewport");
    detect_samsung_internet([
        audio.is_some_and(|value| has(value, "srLatency")),
        audio.is_some_and(|value| has(value, "srChannelCount")),
        has(navigator(), "devicePosture"),
        present(viewport) && has(viewport, "segments"),
        image.is_some_and(|value| has(value, "getTextInformation")),
    ])
}
