//! JavaScript export roots for the FingerprintJS-compatible package.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate fingerprintjs_rust_hybrid_experiment as fingerprintjs;

use mires_js::{JsValue, js_export};

// Read by tools/build.mjs. The const is deliberately unreachable in the product bundle.
#[allow(dead_code)]
const PACKAGE_EXPORTS: &[(&str, &str, u8, i8, &str)] = &[
    (
        "componentsToDebugString",
        "fp_components_to_debug_string",
        1,
        -1,
        "componentsToDebugString",
    ),
    (
        "getFullscreenElement",
        "fp_get_fullscreen_element",
        0,
        -1,
        "getFullscreenElement",
    ),
    (
        "getUnstableAudioFingerprint",
        "fp_get_unstable_audio_fingerprint",
        0,
        -1,
        "getUnstableAudioFingerprint",
    ),
    (
        "getUnstableCanvasFingerprint",
        "fp_get_unstable_canvas_fingerprint",
        1,
        -1,
        "getUnstableCanvasFingerprint",
    ),
    (
        "getUnstableHardwareConcurrency",
        "fp_get_unstable_hardware_concurrency",
        0,
        -1,
        "getUnstableHardwareConcurrency",
    ),
    (
        "getUnstableScreenFrame",
        "fp_get_unstable_screen_frame",
        0,
        -1,
        "getUnstableScreenFrame",
    ),
    (
        "getUnstableScreenResolution",
        "fp_get_unstable_screen_resolution",
        0,
        -1,
        "getUnstableScreenResolution",
    ),
    (
        "getWebGLContext",
        "fp_get_webgl_context",
        1,
        -1,
        "getWebGLContext",
    ),
    (
        "hashComponents",
        "fp_hash_components",
        1,
        -1,
        "hashComponents",
    ),
    ("isAndroid", "fp_is_android", 0, -1, "isAndroid"),
    ("isChromium", "fp_is_chromium", 0, -1, "isChromium"),
    (
        "isDesktopWebKit",
        "fp_is_desktop_webkit",
        0,
        -1,
        "isDesktopWebKit",
    ),
    ("isEdgeHTML", "fp_is_edge_html", 0, -1, "isEdgeHTML"),
    ("isGecko", "fp_is_gecko", 0, -1, "isGecko"),
    (
        "isSamsungInternet",
        "fp_is_samsung_internet",
        0,
        -1,
        "isSamsungInternet",
    ),
    ("isTrident", "fp_is_trident", 0, -1, "isTrident"),
    ("isWebKit", "fp_is_webkit", 0, -1, "isWebKit"),
    ("load", "fp_load", 1, 0, "load"),
    ("loadSources", "fp_load_sources", 4, -1, "loadSources"),
    (
        "murmurX64Hash128",
        "fp_murmur_x64_hash128",
        2,
        -1,
        "x64hash128",
    ),
    (
        "prepareForSources",
        "fp_prepare_for_sources",
        1,
        0,
        "prepareForSources",
    ),
    (
        "transformSource",
        "fp_transform_source",
        2,
        -1,
        "transformSource",
    ),
    ("withIframe", "fp_with_iframe", 3, 2, "withIframe"),
];

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[js_export]
pub fn fp_components_to_debug_string(components: JsValue) -> JsValue {
    fingerprintjs::components_to_debug_string(components)
}

#[js_export]
pub fn fp_get_fullscreen_element() -> JsValue {
    fingerprintjs::get_fullscreen_element()
}

#[js_export]
pub fn fp_get_unstable_audio_fingerprint() -> JsValue {
    fingerprintjs::get_unstable_audio_fingerprint()
}

#[js_export]
pub fn fp_get_unstable_canvas_fingerprint(skip_images: JsValue) -> JsValue {
    fingerprintjs::get_unstable_canvas_fingerprint(skip_images)
}

#[js_export]
pub fn fp_get_unstable_hardware_concurrency() -> JsValue {
    fingerprintjs::runtime_unstable_hardware_concurrency()
}

#[js_export]
pub fn fp_get_unstable_screen_frame() -> JsValue {
    fingerprintjs::get_unstable_screen_frame()
}

#[js_export]
pub fn fp_get_unstable_screen_resolution() -> JsValue {
    fingerprintjs::runtime_unstable_screen_resolution()
}

#[js_export]
pub fn fp_get_webgl_context(cache: JsValue) -> JsValue {
    fingerprintjs::get_webgl_context(cache)
}

#[js_export]
pub fn fp_hash_components(components: JsValue) -> JsValue {
    fingerprintjs::hash_components_host(components)
}

#[js_export]
pub fn fp_is_android() -> JsValue {
    JsValue::erase(fingerprintjs::is_android())
}

#[js_export]
pub fn fp_is_chromium() -> JsValue {
    JsValue::erase(fingerprintjs::is_chromium())
}

#[js_export]
pub fn fp_is_desktop_webkit() -> JsValue {
    JsValue::erase(fingerprintjs::is_desktop_webkit())
}

#[js_export]
pub fn fp_is_edge_html() -> JsValue {
    JsValue::erase(fingerprintjs::is_edge_html())
}

#[js_export]
pub fn fp_is_gecko() -> JsValue {
    JsValue::erase(fingerprintjs::is_gecko())
}

#[js_export]
pub fn fp_is_samsung_internet() -> JsValue {
    JsValue::erase(fingerprintjs::is_samsung_internet())
}

#[js_export]
pub fn fp_is_trident() -> JsValue {
    JsValue::erase(fingerprintjs::is_trident())
}

#[js_export]
pub fn fp_is_webkit() -> JsValue {
    JsValue::erase(fingerprintjs::is_webkit())
}

#[js_export]
pub fn fp_load(options: JsValue) -> JsValue {
    fingerprintjs::load(options)
}

#[js_export]
pub fn fp_load_sources(
    sources: JsValue,
    source_options: JsValue,
    exclude_sources: JsValue,
    loop_release_interval: JsValue,
) -> JsValue {
    fingerprintjs::load_sources(
        sources,
        source_options,
        exclude_sources,
        loop_release_interval,
    )
}

#[js_export]
pub fn fp_murmur_x64_hash128(input: JsValue, seed: JsValue) -> JsValue {
    fingerprintjs::x64hash128_host(input, seed)
}

#[js_export]
pub fn fp_prepare_for_sources(delay_fallback: JsValue) -> JsValue {
    let delay_fallback = if delay_fallback.is_undefined() {
        None
    } else {
        Some(delay_fallback.read::<f64>())
    };
    fingerprintjs::prepare_for_sources(delay_fallback)
}

#[js_export]
pub fn fp_sources() -> JsValue {
    fingerprintjs::make_sources()
}

#[js_export]
pub fn fp_transform_source(source: JsValue, transform_value: JsValue) -> JsValue {
    fingerprintjs::transform_source(source, transform_value)
}

#[js_export]
pub fn fp_with_iframe(
    action: JsValue,
    initial_html: JsValue,
    poll_interval: JsValue,
) -> JsValue {
    fingerprintjs::with_iframe(action, initial_html, poll_interval)
}
