use crate::{call, count_truthy};
use mires_js::{js_bindings, JsValue};

js_bindings! {
    #[js(new = "RegExp")]
    fn regexp(pattern: &str) -> JsValue;
}

/// Applies the v5.2.0 Trident feature threshold.
pub fn detect_trident(features: [bool; 5]) -> bool {
    count_truthy(&features) >= 4
}

/// Applies the v5.2.0 EdgeHTML feature threshold.
pub fn detect_edge_html(features: [bool; 4], trident: bool) -> bool {
    count_truthy(&features) >= 3 && !trident
}

/// Applies the v5.2.0 Chromium feature threshold.
pub fn detect_chromium(features: [bool; 7]) -> bool {
    count_truthy(&features) >= 5
}

/// Applies the v5.2.0 WebKit feature threshold.
pub fn detect_webkit(features: [bool; 6]) -> bool {
    count_truthy(&features) >= 4
}

/// Applies the v5.2.0 desktop WebKit feature threshold.
pub fn detect_desktop_webkit(features: [bool; 6]) -> bool {
    count_truthy(&features) >= 4
}

/// Applies the v5.2.0 Gecko feature threshold.
pub fn detect_gecko(features: [bool; 6]) -> bool {
    count_truthy(&features) >= 4
}

/// Applies the v5.2.0 Gecko 120 feature threshold.
pub fn detect_gecko_120_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 Gecko 143 feature threshold.
pub fn detect_gecko_143_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 Chromium 86 feature threshold.
pub fn detect_chromium_86_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 Chromium 122 feature threshold.
pub fn detect_chromium_122_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 Chromium 128 feature threshold.
pub fn detect_chromium_128_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 WebKit 606 feature threshold.
pub fn detect_webkit_606_or_newer(features: [bool; 4]) -> bool {
    count_truthy(&features) >= 3
}

/// Applies the v5.2.0 WebKit 616 feature threshold.
pub fn detect_webkit_616_or_newer(features: [bool; 5]) -> bool {
    count_truthy(&features) >= 4
}

/// Detects Safari from the native `print` function and WebPage namespace.
pub const fn detect_safari_webkit(print_is_native: bool, browser_is_namespace: bool) -> bool {
    print_is_native && browser_is_namespace
}

/// Applies the iPad platform and feature checks.
pub fn detect_ipad(platform_is_ipad: bool, features: [bool; 3]) -> bool {
    platform_is_ipad || count_truthy(&features) >= 2
}

/// Applies the Android checks after engine detection.
pub fn detect_android(
    chromium: bool,
    gecko: bool,
    chromium_features: [bool; 3],
    gecko_features: [bool; 3],
) -> bool {
    if chromium {
        count_truthy(&chromium_features) >= 2
    } else if gecko {
        count_truthy(&gecko_features) >= 2
    } else {
        false
    }
}

/// Applies the v5.2.0 Samsung Internet feature threshold.
pub fn detect_samsung_internet(features: [bool; 5]) -> bool {
    count_truthy(&features) >= 3
}

/// Checks the source text shape used for a native browser function.
pub fn is_function_native(source: &str) -> bool {
    let pattern = regexp(r"^function\s.*?\{\s*\[native code\]\s*}$");
    call(pattern, "test", &[JsValue::erase(source)]).read()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_thresholds_match_upstream() {
        assert!(detect_trident([true, true, true, true, false]));
        assert!(!detect_trident([true, true, true, false, false]));
        assert!(detect_edge_html([true, true, true, false], false));
        assert!(!detect_edge_html([true, true, true, true], true));
        assert!(detect_chromium([
            true, true, true, true, true, false, false
        ]));
        assert!(detect_webkit([true, true, true, true, false, false]));
        assert!(detect_samsung_internet([true, true, true, false, false]));
    }

    #[test]
    fn selects_android_engine_rules() {
        assert!(detect_android(true, false, [true, true, false], [false; 3]));
        assert!(detect_android(false, true, [false; 3], [true, false, true]));
        assert!(!detect_android(false, false, [true; 3], [true; 3]));
    }
}
