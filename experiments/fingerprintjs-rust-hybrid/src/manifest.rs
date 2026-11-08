/// The FingerprintJS algorithm version reproduced by this experiment.
pub const ALGORITHM_VERSION: &str = "5.2.0";

/// Built-in entropy sources in their observable collection order.
pub const SOURCE_NAMES: [&str; 42] = [
    "userAgentData",
    "fonts",
    "domBlockers",
    "fontPreferences",
    "audio",
    "screenFrame",
    "canvas",
    "osCpu",
    "languages",
    "colorDepth",
    "deviceMemory",
    "screenResolution",
    "hardwareConcurrency",
    "timezone",
    "sessionStorage",
    "localStorage",
    "indexedDB",
    "openDatabase",
    "cpuClass",
    "platform",
    "plugins",
    "touchSupport",
    "vendor",
    "vendorFlavors",
    "cookiesEnabled",
    "colorGamut",
    "invertedColors",
    "forcedColors",
    "monochrome",
    "contrast",
    "reducedMotion",
    "reducedTransparency",
    "hdr",
    "math",
    "pdfViewerEnabled",
    "architecture",
    "applePay",
    "privateClickMeasurement",
    "audioBaseLatency",
    "dateTimeLocale",
    "webGlBasics",
    "webGlExtensions",
];

/// JavaScript function names of the built-in source callbacks.
pub const SOURCE_FUNCTION_NAMES: [&str; 42] = [
    "getUserAgentData",
    "getFonts",
    "getDomBlockers",
    "getFontPreferences",
    "getAudioFingerprint",
    "getScreenFrame",
    "getCanvasFingerprint",
    "getOsCpu",
    "getLanguages",
    "getColorDepth",
    "getDeviceMemory",
    "getScreenResolution",
    "getHardwareConcurrency",
    "getTimezone",
    "getSessionStorage",
    "getLocalStorage",
    "getIndexedDB",
    "getOpenDatabase",
    "getCpuClass",
    "getPlatform",
    "getPlugins",
    "getTouchSupport",
    "getVendor",
    "getVendorFlavors",
    "areCookiesEnabled",
    "getColorGamut",
    "areColorsInverted",
    "areColorsForced",
    "getMonochromeDepth",
    "getContrastPreference",
    "isMotionReduced",
    "isTransparencyReduced",
    "isHDR",
    "getMathFingerprint",
    "isPdfViewerEnabled",
    "getArchitecture",
    "getApplePayState",
    "getPrivateClickMeasurement",
    "getAudioContextBaseLatency",
    "getDateTimeLocale",
    "getWebGlBasics",
    "getWebGlExtensions",
];

/// Runtime keys exposed by the ESM namespace, in namespace enumeration order.
pub const RUNTIME_EXPORTS: [&str; 25] = [
    "componentsToDebugString",
    "default",
    "getFullscreenElement",
    "getUnstableAudioFingerprint",
    "getUnstableCanvasFingerprint",
    "getUnstableHardwareConcurrency",
    "getUnstableScreenFrame",
    "getUnstableScreenResolution",
    "getWebGLContext",
    "hashComponents",
    "isAndroid",
    "isChromium",
    "isDesktopWebKit",
    "isEdgeHTML",
    "isGecko",
    "isSamsungInternet",
    "isTrident",
    "isWebKit",
    "load",
    "loadSources",
    "murmurX64Hash128",
    "prepareForSources",
    "sources",
    "transformSource",
    "withIframe",
];

/// Values contained in the default export object.
pub const DEFAULT_EXPORTS: [&str; 3] = ["load", "hashComponents", "componentsToDebugString"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_match_the_frozen_contract() {
        assert_eq!(SOURCE_NAMES.len(), 42);
        assert_eq!(
            &SOURCE_NAMES[..6],
            [
                "userAgentData",
                "fonts",
                "domBlockers",
                "fontPreferences",
                "audio",
                "screenFrame",
            ]
        );
        assert_eq!(&SOURCE_NAMES[40..], ["webGlBasics", "webGlExtensions"]);

        assert_eq!(RUNTIME_EXPORTS.len(), 25);
        assert!(RUNTIME_EXPORTS.windows(2).all(|names| names[0] < names[1]));
    }
}
