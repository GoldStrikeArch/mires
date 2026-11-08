//! Canvas and WebGL entropy sources matching FingerprintJS v5.2.0.

#[cfg(fingerprint_fiber)]
use mires_js::future_to_promise;
use mires_js::JsValue;

use crate::host_graphics::{
    cached_webgl_context, canvas_test_text, computed, constants_from_prototype, join_as_strings,
    webgl_string,
};
use crate::{
    array, call, document, get, index, is_chromium, is_gecko, is_gecko_120_or_newer,
    is_safari_webkit, is_webkit, is_webkit_616_or_newer, length, number_value, object, object_keys,
    push, set, sort,
};

/// WebGL context is not available.
pub const STATUS_NO_GL_CONTEXT: i32 = -1;
/// The WebGL context's `getParameter` property isn't a function.
pub const STATUS_GET_PARAMETER_NOT_A_FUNCTION: i32 = -2;

const IMAGE_UNSUPPORTED: &str = "unsupported";
const IMAGE_SKIPPED: &str = "skipped";
const IMAGE_UNSTABLE: &str = "unstable";

const VALID_CONTEXT_PARAMETERS: &[u32] = &[
    10_752, 2_849, 2_884, 2_885, 2_886, 2_928, 2_929, 2_930, 2_931, 2_932, 2_960, 2_961, 2_962,
    2_963, 2_964, 2_965, 2_966, 2_967, 2_968, 2_978, 3_024, 3_042, 3_088, 3_089, 3_106, 3_107,
    32_773, 32_777, 32_777, 32_823, 32_824, 32_936, 32_937, 32_938, 32_939, 32_968, 32_969, 32_970,
    32_971, 3_317, 33_170, 3_333, 3_379, 3_386, 33_901, 33_902, 34_016, 34_024, 34_076, 3_408,
    3_410, 3_411, 3_412, 3_413, 3_414, 3_415, 34_467, 34_816, 34_817, 34_818, 34_819, 34_877,
    34_921, 34_930, 35_660, 35_661, 35_724, 35_738, 35_739, 36_003, 36_004, 36_005, 36_347, 36_348,
    36_349, 37_440, 37_441, 37_443, 7_936, 7_937, 7_938,
];
const VALID_EXTENSION_PARAMETERS: &[u32] = &[
    34_047, 35_723, 36_063, 34_852, 34_853, 34_854, 34_229, 36_392, 36_795, 38_449,
];
const SHADER_TYPES: [&str; 2] = ["FRAGMENT_SHADER", "VERTEX_SHADER"];
const PRECISION_TYPES: [&str; 6] = [
    "LOW_FLOAT",
    "MEDIUM_FLOAT",
    "HIGH_FLOAT",
    "LOW_INT",
    "MEDIUM_INT",
    "HIGH_INT",
];
const RENDERER_INFO_EXTENSION: &str = "WEBGL_debug_renderer_info";
const POLYGON_MODE_EXTENSION: &str = "WEBGL_polygon_mode";

/// Loads the stabilized canvas fingerprint.
#[must_use]
pub fn source_canvas(options: JsValue) -> JsValue {
    let _ = options;
    get_unstable_canvas_fingerprint(JsValue::erase(browser_performs_anti_fingerprinting()))
}

/// Uses resumable canvas phases for built-in collection in the fiber artifact.
#[must_use]
pub fn source_canvas_for_collection(options: JsValue) -> JsValue {
    #[cfg(fingerprint_fiber)]
    {
        let _ = options;
        let skip_images = browser_performs_anti_fingerprinting();
        future_to_promise(async move { Ok(collect_canvas_fiber(skip_images).await) })
    }
    #[cfg(not(fingerprint_fiber))]
    {
        source_canvas(options)
    }
}

#[cfg(fingerprint_fiber)]
async fn collect_canvas_fiber(skip_images: bool) -> JsValue {
    let canvas = call(document(), "createElement", &[JsValue::erase("canvas")]);
    set(canvas, "width", JsValue::erase(1.0_f64));
    set(canvas, "height", JsValue::erase(1.0_f64));
    let context = call(canvas, "getContext", &[JsValue::erase("2d")]);

    if !context.is_truthy() || !get(canvas, "toDataURL").is_function() {
        return canvas_payload(
            false,
            JsValue::erase(IMAGE_UNSUPPORTED),
            JsValue::erase(IMAGE_UNSUPPORTED),
        );
    }
    let winding = supports_winding(context);
    if skip_images {
        return canvas_payload(
            winding,
            JsValue::erase(IMAGE_SKIPPED),
            JsValue::erase(IMAGE_SKIPPED),
        );
    }

    let mut last_release = crate::date_now();
    render_text_image(canvas, context);
    crate::source_checkpoint(&mut last_release).await;
    let text_image_1 = call(canvas, "toDataURL", &[]);
    crate::source_checkpoint(&mut last_release).await;
    let text_image_2 = call(canvas, "toDataURL", &[]);
    crate::source_checkpoint(&mut last_release).await;
    if !text_image_1.strictly_equals(text_image_2) {
        return canvas_payload(
            winding,
            JsValue::erase(IMAGE_UNSTABLE),
            JsValue::erase(IMAGE_UNSTABLE),
        );
    }
    render_geometry_image(canvas, context);
    crate::source_checkpoint(&mut last_release).await;
    canvas_payload(winding, call(canvas, "toDataURL", &[]), text_image_1)
}

/// Returns the canvas fingerprint without browser-specific stabilization.
#[must_use]
pub fn get_unstable_canvas_fingerprint(skip_images: JsValue) -> JsValue {
    let canvas = call(document(), "createElement", &[JsValue::erase("canvas")]);
    set(canvas, "width", JsValue::erase(1.0_f64));
    set(canvas, "height", JsValue::erase(1.0_f64));
    let context = call(canvas, "getContext", &[JsValue::erase("2d")]);

    let mut winding = false;
    let (geometry, text) = if !context.is_truthy() || !get(canvas, "toDataURL").is_function() {
        (
            JsValue::erase(IMAGE_UNSUPPORTED),
            JsValue::erase(IMAGE_UNSUPPORTED),
        )
    } else {
        winding = supports_winding(context);
        if skip_images.is_truthy() {
            (JsValue::erase(IMAGE_SKIPPED), JsValue::erase(IMAGE_SKIPPED))
        } else {
            render_canvas_images(canvas, context)
        }
    };

    canvas_payload(winding, geometry, text)
}

fn canvas_payload(winding: bool, geometry: JsValue, text: JsValue) -> JsValue {
    let result = object();
    set(result, "winding", JsValue::erase(winding));
    set(result, "geometry", geometry);
    set(result, "text", text);
    result
}

fn supports_winding(context: JsValue) -> bool {
    let _ = call(
        context,
        "rect",
        &[
            JsValue::erase(0.0_f64),
            JsValue::erase(0.0_f64),
            JsValue::erase(10.0_f64),
            JsValue::erase(10.0_f64),
        ],
    );
    let _ = call(
        context,
        "rect",
        &[
            JsValue::erase(2.0_f64),
            JsValue::erase(2.0_f64),
            JsValue::erase(6.0_f64),
            JsValue::erase(6.0_f64),
        ],
    );
    !call(
        context,
        "isPointInPath",
        &[
            JsValue::erase(5.0_f64),
            JsValue::erase(5.0_f64),
            JsValue::erase("evenodd"),
        ],
    )
    .is_truthy()
}

fn render_canvas_images(canvas: JsValue, context: JsValue) -> (JsValue, JsValue) {
    render_text_image(canvas, context);
    let text_image_1 = call(canvas, "toDataURL", &[]);
    let text_image_2 = call(canvas, "toDataURL", &[]);
    if !text_image_1.strictly_equals(text_image_2) {
        return (
            JsValue::erase(IMAGE_UNSTABLE),
            JsValue::erase(IMAGE_UNSTABLE),
        );
    }

    source_checkpoint();
    render_geometry_image(canvas, context);
    (call(canvas, "toDataURL", &[]), text_image_1)
}

fn render_text_image(canvas: JsValue, context: JsValue) {
    set(canvas, "width", JsValue::erase(240.0_f64));
    set(canvas, "height", JsValue::erase(60.0_f64));
    set(context, "textBaseline", JsValue::erase("alphabetic"));
    set(context, "fillStyle", JsValue::erase("#f60"));
    let _ = call(
        context,
        "fillRect",
        &[
            JsValue::erase(100.0_f64),
            JsValue::erase(1.0_f64),
            JsValue::erase(62.0_f64),
            JsValue::erase(20.0_f64),
        ],
    );
    set(context, "fillStyle", JsValue::erase("#069"));
    set(context, "font", JsValue::erase("11pt \"Times New Roman\""));
    let printed_text = canvas_test_text();
    let _ = call(
        context,
        "fillText",
        &[
            printed_text,
            JsValue::erase(2.0_f64),
            JsValue::erase(15.0_f64),
        ],
    );
    set(
        context,
        "fillStyle",
        JsValue::erase("rgba(102, 204, 0, 0.2)"),
    );
    set(context, "font", JsValue::erase("18pt Arial"));
    let _ = call(
        context,
        "fillText",
        &[
            printed_text,
            JsValue::erase(4.0_f64),
            JsValue::erase(45.0_f64),
        ],
    );
}

fn render_geometry_image(canvas: JsValue, context: JsValue) {
    set(canvas, "width", JsValue::erase(122.0_f64));
    set(canvas, "height", JsValue::erase(110.0_f64));
    set(
        context,
        "globalCompositeOperation",
        JsValue::erase("multiply"),
    );

    for (color, x, y) in [
        ("#f2f", 40.0, 40.0),
        ("#2ff", 80.0, 40.0),
        ("#ff2", 60.0, 80.0),
    ] {
        set(context, "fillStyle", JsValue::erase(color));
        let _ = call(context, "beginPath", &[]);
        draw_arc(context, x, y, 40.0);
        let _ = call(context, "closePath", &[]);
        let _ = call(context, "fill", &[]);
    }

    set(context, "fillStyle", JsValue::erase("#f9c"));
    draw_arc(context, 60.0, 60.0, 60.0);
    draw_arc(context, 60.0, 60.0, 20.0);
    let _ = call(context, "fill", &[JsValue::erase("evenodd")]);
}

fn draw_arc(context: JsValue, x: f64, y: f64, radius: f64) {
    let _ = call(
        context,
        "arc",
        &[
            JsValue::erase(x),
            JsValue::erase(y),
            JsValue::erase(radius),
            JsValue::erase(0.0_f64),
            JsValue::erase(core::f64::consts::PI * 2.0),
            JsValue::erase(true),
        ],
    );
}

fn browser_performs_anti_fingerprinting() -> bool {
    let safari_17_or_newer = is_webkit() && is_webkit_616_or_newer() && is_safari_webkit();
    safari_17_or_newer || (is_gecko() && is_gecko_120_or_newer())
}

#[cfg(test)]
const fn browser_performs_anti_fingerprinting_from(features: [bool; 5]) -> bool {
    (features[0] && features[1] && features[2]) || (features[3] && features[4])
}

/// Returns the cache's WebGL context, creating it once when absent.
#[must_use]
pub fn get_webgl_context(cache: JsValue) -> JsValue {
    cached_webgl_context(cache)
}

/// Loads the basic WebGL payload.
#[must_use]
pub fn source_web_gl_basics(options: JsValue) -> JsValue {
    let gl = get_webgl_context(get(options, "cache"));
    if !gl.is_truthy() {
        return JsValue::erase(STATUS_NO_GL_CONTEXT);
    }
    if !get(gl, "getParameter").is_function() {
        return JsValue::erase(STATUS_GET_PARAMETER_NOT_A_FUNCTION);
    }

    let debug_extension = if should_avoid_debug_renderer_info() {
        JsValue::null()
    } else {
        call(
            gl,
            "getExtension",
            &[JsValue::erase(RENDERER_INFO_EXTENSION)],
        )
    };
    let result = object();
    set(
        result,
        "version",
        webgl_parameter_string(gl, get(gl, "VERSION")),
    );
    set(
        result,
        "vendor",
        webgl_parameter_string(gl, get(gl, "VENDOR")),
    );
    set(
        result,
        "vendorUnmasked",
        if debug_extension.is_truthy() {
            webgl_parameter_string(gl, get(debug_extension, "UNMASKED_VENDOR_WEBGL"))
        } else {
            JsValue::erase("")
        },
    );
    set(
        result,
        "renderer",
        webgl_parameter_string(gl, get(gl, "RENDERER")),
    );
    set(
        result,
        "rendererUnmasked",
        if debug_extension.is_truthy() {
            webgl_parameter_string(gl, get(debug_extension, "UNMASKED_RENDERER_WEBGL"))
        } else {
            JsValue::erase("")
        },
    );
    set(
        result,
        "shadingLanguageVersion",
        webgl_parameter_string(gl, get(gl, "SHADING_LANGUAGE_VERSION")),
    );
    result
}

fn webgl_parameter_string(gl: JsValue, parameter: JsValue) -> JsValue {
    webgl_string(call(gl, "getParameter", &[parameter]))
}

/// Loads the extended WebGL payload.
#[must_use]
pub fn source_web_gl_extensions(options: JsValue) -> JsValue {
    let gl = get_webgl_context(get(options, "cache"));
    if !gl.is_truthy() {
        return JsValue::erase(STATUS_NO_GL_CONTEXT);
    }
    if !get(gl, "getParameter").is_function() {
        return JsValue::erase(STATUS_GET_PARAMETER_NOT_A_FUNCTION);
    }

    let extensions = call(gl, "getSupportedExtensions", &[]);
    let context_attributes = call(gl, "getContextAttributes", &[]);
    let attributes = array();
    let parameters = array();
    let extension_parameters = array();
    let shader_precisions = array();
    let unsupported_extensions = array();

    if context_attributes.is_truthy() {
        let names = object_keys(context_attributes);
        for name_index in 0..length(names) {
            let name = index(names, name_index);
            push(
                attributes,
                join_as_strings(&[
                    name,
                    JsValue::erase("="),
                    computed(context_attributes, name),
                ]),
            );
        }
    }

    let constants = constants_from_prototype(gl);
    for constant_index in 0..length(constants) {
        let constant = index(constants, constant_index);
        let code = computed(gl, constant);
        let include_value = valid_parameter(code, VALID_CONTEXT_PARAMETERS);
        let value = if include_value {
            call(gl, "getParameter", &[code])
        } else {
            JsValue::undefined()
        };
        let mut parts = [JsValue::undefined(); 4];
        parts[0] = constant;
        parts[1] = JsValue::erase("=");
        parts[2] = code;
        let part_count = if include_value {
            parts[3] = join_as_strings(&[JsValue::erase("="), value]);
            4
        } else {
            3
        };
        push(parameters, join_as_strings(&parts[..part_count]));
        if constant_index % 32 == 31 {
            source_checkpoint();
        }
    }

    if extensions.is_truthy() {
        for extension_index in 0..length(extensions) {
            let name = index(extensions, extension_index);
            if (name.strictly_equals(JsValue::erase(RENDERER_INFO_EXTENSION))
                && should_avoid_debug_renderer_info())
                || (name.strictly_equals(JsValue::erase(POLYGON_MODE_EXTENSION))
                    && should_avoid_polygon_mode_extensions())
            {
                continue;
            }

            let extension = call(gl, "getExtension", &[name]);
            if !extension.is_truthy() {
                push(unsupported_extensions, name);
                continue;
            }

            let constants = constants_from_prototype(extension);
            for constant_index in 0..length(constants) {
                let constant = index(constants, constant_index);
                let code = computed(extension, constant);
                let include_value = valid_parameter(code, VALID_EXTENSION_PARAMETERS);
                let value = if include_value {
                    call(gl, "getParameter", &[code])
                } else {
                    JsValue::undefined()
                };
                let mut parts = [JsValue::undefined(); 4];
                parts[0] = constant;
                parts[1] = JsValue::erase("=");
                parts[2] = code;
                let part_count = if include_value {
                    parts[3] = join_as_strings(&[JsValue::erase("="), value]);
                    4
                } else {
                    3
                };
                push(extension_parameters, join_as_strings(&parts[..part_count]));
            }
            source_checkpoint();
        }
    }

    for shader_type in SHADER_TYPES {
        for precision_type in PRECISION_TYPES {
            let precision = shader_precision(gl, shader_type, precision_type);
            push(
                shader_precisions,
                join_as_strings(&[
                    JsValue::erase(shader_type),
                    JsValue::erase("."),
                    JsValue::erase(precision_type),
                    JsValue::erase("="),
                    precision,
                ]),
            );
        }
        source_checkpoint();
    }

    sort(extension_parameters);
    sort(parameters);

    let result = object();
    set(result, "contextAttributes", attributes);
    set(result, "parameters", parameters);
    set(result, "shaderPrecisions", shader_precisions);
    set(result, "extensions", extensions);
    set(result, "extensionParameters", extension_parameters);
    set(result, "unsupportedExtensions", unsupported_extensions);
    result
}

fn shader_precision(gl: JsValue, shader_type: &str, precision_type: &str) -> JsValue {
    let format = call(
        gl,
        "getShaderPrecisionFormat",
        &[get(gl, shader_type), get(gl, precision_type)],
    );
    let values = array();
    if format.is_truthy() {
        push(values, get(format, "rangeMin"));
        push(values, get(format, "rangeMax"));
        push(values, get(format, "precision"));
    }
    call(values, "join", &[JsValue::erase(",")])
}

fn valid_parameter(code: JsValue, valid: &[u32]) -> bool {
    if !code.is_number() {
        return false;
    }
    let code = number_value(code);
    valid.iter().any(|valid| f64::from(*valid) == code)
}

/// Returns whether requesting `WEBGL_debug_renderer_info` is known to warn.
#[must_use]
pub fn should_avoid_debug_renderer_info() -> bool {
    is_gecko()
}

/// Returns whether requesting `WEBGL_polygon_mode` is known to warn.
#[must_use]
pub fn should_avoid_polygon_mode_extensions() -> bool {
    is_chromium() || is_webkit()
}

// The fiber artifact can replace this no-op at the measured, side-effect-neutral boundaries above.
#[inline(always)]
fn source_checkpoint() {}

#[cfg(test)]
mod tests {
    use super::{
        browser_performs_anti_fingerprinting_from, PRECISION_TYPES, SHADER_TYPES,
        VALID_CONTEXT_PARAMETERS, VALID_EXTENSION_PARAMETERS,
    };

    #[test]
    fn canvas_stabilization_matches_v5_2_0() {
        assert!(browser_performs_anti_fingerprinting_from([
            true, true, true, false, false
        ]));
        assert!(browser_performs_anti_fingerprinting_from([
            false, false, false, true, true
        ]));
        assert!(!browser_performs_anti_fingerprinting_from([
            true, true, false, false, false
        ]));
        assert!(!browser_performs_anti_fingerprinting_from([
            false, false, false, true, false
        ]));
    }

    #[test]
    fn webgl_parameter_allowlists_keep_warning_codes_excluded() {
        assert!(VALID_CONTEXT_PARAMETERS.contains(&10_752));
        assert!(VALID_CONTEXT_PARAMETERS.contains(&7_938));
        assert!(!VALID_CONTEXT_PARAMETERS.contains(&32_926));
        assert!(!VALID_CONTEXT_PARAMETERS.contains(&32_928));
        assert_eq!(VALID_EXTENSION_PARAMETERS.len(), 10);
        assert!(VALID_EXTENSION_PARAMETERS.contains(&38_449));
    }

    #[test]
    fn shader_precision_order_is_exact() {
        assert_eq!(SHADER_TYPES, ["FRAGMENT_SHADER", "VERTEX_SHADER"]);
        assert_eq!(
            PRECISION_TYPES,
            [
                "LOW_FLOAT",
                "MEDIUM_FLOAT",
                "HIGH_FLOAT",
                "LOW_INT",
                "MEDIUM_INT",
                "HIGH_INT"
            ]
        );
    }
}
