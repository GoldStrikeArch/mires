use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{detect_android, detect_edge_html, detect_trident, to_float, to_int};

/// Values returned by media query matching signals.
pub trait MediaQueries {
    fn matches(&self, query: &str) -> bool;
}

/// Browser engine flags needed by synchronous stabilization rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserStability {
    pub webkit: bool,
    pub desktop_webkit: bool,
    pub safari_webkit: bool,
    pub webkit_616_or_newer: bool,
    pub gecko: bool,
    pub gecko_143_or_newer: bool,
}

/// Touch signal value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchSupport {
    pub max_touch_points: f64,
    pub touch_event: bool,
    pub touch_start: bool,
}

/// CSS color gamut reported by a browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorGamut {
    Srgb,
    P3,
    Rec2020,
}

/// CSS contrast preference values used by FingerprintJS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ContrastPreference {
    Less = -1,
    None = 0,
    More = 1,
    ForcedColors = 10,
}

/// Reads `screen.colorDepth` without transforming it.
pub const fn get_color_depth(color_depth: i32) -> i32 {
    color_depth
}

/// Normalizes the string-or-number `navigator.deviceMemory` value.
pub fn get_device_memory(value: Option<&str>) -> Option<f64> {
    value.and_then(|value| {
        let value = to_float(value);
        (!value.is_nan()).then_some(value)
    })
}

/// Reads `navigator.oscpu` without transforming it.
pub fn get_os_cpu(value: Option<&str>) -> Option<String> {
    value.map(ToString::to_string)
}

/// Reads `navigator.pdfViewerEnabled` without transforming it.
pub const fn get_pdf_viewer_enabled(value: Option<bool>) -> Option<bool> {
    value
}

/// Normalizes an absent or empty browser vendor to an empty string.
pub fn get_vendor(value: Option<&str>) -> String {
    value.unwrap_or_default().to_string()
}

/// Reads `navigator.cpuClass` without transforming it.
pub fn get_cpu_class(value: Option<&str>) -> Option<String> {
    value.map(ToString::to_string)
}

/// Normalizes the first language and `navigator.languages` source values.
pub fn get_languages(
    language: Option<&str>,
    languages: Option<&[&str]>,
    chromium_86_or_newer: bool,
) -> Vec<Vec<String>> {
    let mut output = Vec::new();
    if let Some(language) = language {
        output.push(alloc::vec![language.to_string()]);
    }
    if !chromium_86_or_newer {
        if let Some(languages) = languages {
            output.push(languages.iter().map(ToString::to_string).collect());
        }
    }
    output
}

/// Corrects WebKit's desktop-mode platform value on iPhones and iPads.
pub fn get_platform(platform: &str, stability: BrowserStability, ipad: bool) -> String {
    if platform == "MacIntel" && stability.webkit && !stability.desktop_webkit {
        if ipad { "iPad" } else { "iPhone" }.to_string()
    } else {
        platform.to_string()
    }
}

/// Normalizes and stabilizes `navigator.hardwareConcurrency`.
pub fn get_hardware_concurrency(value: Option<&str>, stability: BrowserStability) -> Option<f64> {
    let value = get_unstable_hardware_concurrency(value)?;
    if stability.gecko && stability.gecko_143_or_newer {
        Some(if value >= 8.0 { 8.0 } else { 4.0 })
    } else {
        Some(value)
    }
}

/// Normalizes `navigator.hardwareConcurrency` without stabilization.
pub fn get_unstable_hardware_concurrency(value: Option<&str>) -> Option<f64> {
    let value = to_int(value?);
    (!value.is_nan()).then_some(value)
}

/// Returns descending screen dimensions or disables them on Safari 17+.
pub fn get_screen_resolution(
    width: Option<&str>,
    height: Option<&str>,
    stability: BrowserStability,
) -> Option<[Option<f64>; 2]> {
    if stability.webkit && stability.webkit_616_or_newer && stability.safari_webkit {
        None
    } else {
        Some(get_unstable_screen_resolution(width, height))
    }
}

/// Returns descending normalized screen dimensions.
pub fn get_unstable_screen_resolution(
    width: Option<&str>,
    height: Option<&str>,
) -> [Option<f64>; 2] {
    let parse = |value: Option<&str>| {
        let value = to_int(value.unwrap_or("undefined"));
        (!value.is_nan()).then_some(value)
    };
    let mut dimensions = [parse(width), parse(height)];
    dimensions.sort_by(|left, right| {
        let left = left.map_or_else(|| "null".to_string(), js_number_string);
        let right = right.map_or_else(|| "null".to_string(), js_number_string);
        right.cmp(&left)
    });
    dimensions
}

/// Uses an IANA name when present, otherwise formats the non-DST offset.
pub fn get_timezone(time_zone: Option<&str>, january_offset: f64, july_offset: f64) -> String {
    if let Some(time_zone) = time_zone.filter(|value| !value.is_empty()) {
        return time_zone.to_string();
    }
    let offset = -january_offset.max(july_offset);
    if offset >= 0.0 {
        alloc::format!("UTC+{}", js_number_string(offset))
    } else {
        alloc::format!("UTC{}", js_number_string(offset))
    }
}

/// Reads the three storage-presence signals with their error semantics.
pub const fn get_storage(present: bool, access_threw: bool) -> bool {
    present || access_threw
}

/// Reads IndexedDB, except on Trident and EdgeHTML.
pub fn get_indexed_db(
    present: bool,
    access_threw: bool,
    trident_features: [bool; 5],
    edge_features: [bool; 4],
) -> Option<bool> {
    let trident = detect_trident(trident_features);
    if trident || detect_edge_html(edge_features, trident) {
        None
    } else {
        Some(get_storage(present, access_threw))
    }
}

/// Reads the legacy `openDatabase` presence signal.
pub const fn get_open_database(present: bool) -> bool {
    present
}

/// Returns recognized browser-specific object globals in sorted order.
pub fn get_vendor_flavors(globals: &[(&str, bool)]) -> Vec<String> {
    const KEYS: [&str; 15] = [
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
    ];
    let mut output = globals
        .iter()
        .filter(|(name, is_object)| *is_object && KEYS.contains(name))
        .map(|(name, _)| (*name).to_string())
        .collect::<Vec<_>>();
    output.sort();
    output.dedup();
    output
}

/// Builds the touch support component from normalized host observations.
pub fn get_touch_support(
    max_touch_points: Option<&str>,
    ms_max_touch_points: Option<f64>,
    touch_event: bool,
    touch_start: bool,
) -> TouchSupport {
    TouchSupport {
        max_touch_points: max_touch_points
            .map_or_else(|| ms_max_touch_points.unwrap_or(0.0), to_int),
        touch_event,
        touch_start,
    }
}

/// Returns the widest supported CSS color gamut.
pub fn get_color_gamut(media: &impl MediaQueries) -> Option<ColorGamut> {
    [
        ("(color-gamut: rec2020)", ColorGamut::Rec2020),
        ("(color-gamut: p3)", ColorGamut::P3),
        ("(color-gamut: srgb)", ColorGamut::Srgb),
    ]
    .into_iter()
    .find_map(|(query, gamut)| media.matches(query).then_some(gamut))
}

/// Reads the inverted-colors CSS signal.
pub fn get_inverted_colors(media: &impl MediaQueries) -> Option<bool> {
    boolean_media(
        media,
        "(inverted-colors: inverted)",
        "(inverted-colors: none)",
    )
}

/// Reads the forced-colors CSS signal.
pub fn get_forced_colors(media: &impl MediaQueries) -> Option<bool> {
    boolean_media(media, "(forced-colors: active)", "(forced-colors: none)")
}

/// Reads the prefers-reduced-motion CSS signal.
pub fn get_reduced_motion(media: &impl MediaQueries) -> Option<bool> {
    boolean_media(
        media,
        "(prefers-reduced-motion: reduce)",
        "(prefers-reduced-motion: no-preference)",
    )
}

/// Reads the prefers-reduced-transparency CSS signal.
pub fn get_reduced_transparency(media: &impl MediaQueries) -> Option<bool> {
    boolean_media(
        media,
        "(prefers-reduced-transparency: reduce)",
        "(prefers-reduced-transparency: no-preference)",
    )
}

/// Reads the dynamic-range CSS signal.
pub fn get_hdr(media: &impl MediaQueries) -> Option<bool> {
    boolean_media(media, "(dynamic-range: high)", "(dynamic-range: standard)")
}

/// Reads the prefers-contrast CSS signal.
pub fn get_contrast(media: &impl MediaQueries) -> Option<ContrastPreference> {
    if media.matches("(prefers-contrast: no-preference)") {
        Some(ContrastPreference::None)
    } else if media.matches("(prefers-contrast: high)") || media.matches("(prefers-contrast: more)")
    {
        Some(ContrastPreference::More)
    } else if media.matches("(prefers-contrast: low)") || media.matches("(prefers-contrast: less)")
    {
        Some(ContrastPreference::Less)
    } else if media.matches("(prefers-contrast: forced)") {
        Some(ContrastPreference::ForcedColors)
    } else {
        None
    }
}

/// Reads the monochrome bit depth or reports an unsupported media feature.
pub fn get_monochrome_depth(media: &impl MediaQueries) -> Result<Option<u8>, &'static str> {
    if !media.matches("(min-monochrome: 0)") {
        return Ok(None);
    }
    for depth in 0..=100 {
        if media.matches(&alloc::format!("(max-monochrome: {depth})")) {
            return Ok(Some(depth));
        }
    }
    Err("Too high value")
}

/// Returns the high byte of the host's typed-array architecture probe.
pub const fn get_architecture(nan_high_byte: u8) -> u8 {
    nan_high_byte
}

/// Converts the supported date-time locale states into the v5.2.0 union.
pub fn get_date_time_locale(
    has_intl: bool,
    has_date_time_format: bool,
    locale: Option<&str>,
) -> Result<String, i8> {
    if !has_intl {
        Err(-1)
    } else if !has_date_time_format {
        Err(-2)
    } else {
        locale.map(ToString::to_string).ok_or(-3)
    }
}

/// Classifies an Apple Pay `InvalidAccessError` from an insecure context.
pub fn apple_pay_state_from_error(name: &str, message: &str) -> Option<i8> {
    (name == "InvalidAccessError"
        && contains_words_in_order_ascii_case(message, "from", "insecure"))
    .then_some(-2)
}

/// Returns the browser's own attribution-source string.
pub fn get_private_click_measurement(value: Option<&str>) -> Option<String> {
    value.map(ToString::to_string)
}

/// Applies the AudioContext base-latency status rules.
pub fn get_audio_base_latency(
    android: bool,
    webkit: bool,
    audio_context: bool,
    latency: Option<f64>,
) -> f64 {
    if !android && !webkit {
        -2.0
    } else if !audio_context || latency.is_none() {
        -1.0
    } else if !latency.unwrap_or_default().is_finite() {
        -3.0
    } else {
        latency.unwrap_or_default()
    }
}

/// Re-exports the engine-specific Android rule for source modules.
pub fn get_android_from_features(
    chromium: bool,
    gecko: bool,
    chromium_features: [bool; 3],
    gecko_features: [bool; 3],
) -> bool {
    detect_android(chromium, gecko, chromium_features, gecko_features)
}

fn boolean_media(media: &impl MediaQueries, active: &str, inactive: &str) -> Option<bool> {
    if media.matches(active) {
        Some(true)
    } else if media.matches(inactive) {
        Some(false)
    } else {
        None
    }
}

fn js_number_string(value: f64) -> String {
    if value % 1.0 == 0.0 {
        alloc::format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn contains_words_in_order_ascii_case(value: &str, first: &str, second: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(first_index) = lower.find(first) else {
        return false;
    };
    lower[first_index + first.len()..].contains(second)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;

    use super::*;

    struct MockMedia(BTreeSet<&'static str>);

    impl MediaQueries for MockMedia {
        fn matches(&self, query: &str) -> bool {
            self.0.contains(query)
        }
    }

    #[test]
    fn stabilizes_browser_values() {
        assert_eq!(get_device_memory(Some("4")), Some(4.0));
        assert_eq!(get_device_memory(None), None);
        assert_eq!(
            get_hardware_concurrency(
                Some("12"),
                BrowserStability {
                    gecko: true,
                    gecko_143_or_newer: true,
                    ..BrowserStability::default()
                }
            ),
            Some(8.0)
        );
        assert_eq!(
            get_unstable_screen_resolution(Some("800"), Some("1200")),
            [Some(800.0), Some(1200.0)]
        );
        assert_eq!(get_timezone(None, 60.0, 120.0), "UTC-120");
    }

    #[test]
    fn reads_media_preferences_in_upstream_order() {
        let media = MockMedia(BTreeSet::from([
            "(color-gamut: srgb)",
            "(color-gamut: p3)",
            "(prefers-contrast: more)",
            "(dynamic-range: standard)",
            "(min-monochrome: 0)",
            "(max-monochrome: 8)",
        ]));
        assert_eq!(get_color_gamut(&media), Some(ColorGamut::P3));
        assert_eq!(get_contrast(&media), Some(ContrastPreference::More));
        assert_eq!(get_hdr(&media), Some(false));
        assert_eq!(get_monochrome_depth(&media), Ok(Some(8)));
    }

    #[test]
    fn matches_miscellaneous_source_rules() {
        assert_eq!(
            get_vendor_flavors(&[("yandex", true), ("chrome", true), ("junk", true)]),
            ["chrome", "yandex"]
        );
        assert_eq!(
            apple_pay_state_from_error(
                "InvalidAccessError",
                "Must not call this from an insecure document"
            ),
            Some(-2)
        );
        assert_eq!(get_date_time_locale(false, false, None), Err(-1));
        assert_eq!(
            get_date_time_locale(true, true, Some("")),
            Ok(String::new())
        );
    }
}
