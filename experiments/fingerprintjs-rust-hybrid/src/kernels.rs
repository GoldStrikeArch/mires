//! Allow-listed JS kernels included from `js/` and called through `js_bindings!`.

use js_macro::js;
use mires_js::{js_bindings, JsValue};

js_bindings! {
    module = "./hash.js";

    #[js(call = "murmurX64Hash128")]
    pub fn kernel_murmur(input: JsValue, seed: f64) -> JsValue;
}

js_bindings! {
    module = "./canonical.js";

    #[js(call = "hashComponents")]
    pub fn kernel_hash_components(components: JsValue) -> JsValue;

    #[js(call = "componentsToCanonicalString")]
    pub fn kernel_canonical_string(components: JsValue) -> JsValue;
}

js_bindings! {
    module = "./probes.js";

    #[js(call = "navigatorGet")]
    pub fn navigator_get(name: &str) -> JsValue;

    #[js(call = "screenGet")]
    pub fn screen_get(name: &str) -> JsValue;

    #[js(call = "matchMediaMatches")]
    pub fn match_media_matches(query: &str) -> bool;
}

js_bindings! {
    module = "./graphics.js";

    #[js(call = "createCanvas")]
    pub fn create_canvas() -> JsValue;

    #[js(call = "get2dContext")]
    pub fn get_2d_context(canvas: JsValue) -> JsValue;

    #[js(call = "canvasTestText")]
    pub fn canvas_test_text() -> JsValue;

    #[js(call = "supportsWinding")]
    pub fn supports_winding(context: JsValue) -> bool;

    #[js(call = "renderTextImage")]
    pub fn render_text_image(canvas: JsValue, context: JsValue);

    #[js(call = "renderGeometryImage")]
    pub fn render_geometry_image(canvas: JsValue, context: JsValue);

    #[js(call = "cachedWebglContext")]
    pub fn cached_webgl_context(cache: JsValue) -> JsValue;

    #[js(call = "createOfflineAudioContext")]
    pub fn create_offline_audio_context(channels: f64, length: f64, sample_rate: f64) -> JsValue;

    #[js(call = "configureAudioGraph")]
    pub fn configure_audio_graph(context: JsValue);

    #[js(call = "hashAudioSignal")]
    pub fn hash_audio_signal(signal: JsValue) -> f64;
}

/// One-expression `toDataURL` wrapper. Decision to call it stays in Rust.
pub fn canvas_to_data_url(canvas: JsValue) -> JsValue {
    js! { canvas.toDataURL() }
}
