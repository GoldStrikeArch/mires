open Host

let make_sources () : js =
  let sources = object_ () in
  let add0 key name impl = set sources key (source0 name impl) in
  let add1 key name impl = set sources key (source1 name impl) in
  add0 "userAgentData" "getUserAgentData" User_agent.get_user_agent_data;
  add0 "fonts" "getFonts" Fonts.get_fonts;
  add0 "domBlockers" "getDomBlockers" Blockers.get_dom_blockers;
  add0 "fontPreferences" "getFontPreferences" Font_preferences.get_font_preferences;
  add0 "audio" "getAudioFingerprint" Audio.get_audio_fingerprint;
  add0 "screenFrame" "getScreenFrame" Screen_frame.get_screen_frame;
  add0 "canvas" "getCanvasFingerprint" Canvas.get_canvas_fingerprint;
  add0 "osCpu" "getOsCpu" Sources_simple.get_os_cpu;
  add0 "languages" "getLanguages" Sources_simple.get_languages;
  add0 "colorDepth" "getColorDepth" Sources_simple.get_color_depth;
  add0 "deviceMemory" "getDeviceMemory" Sources_simple.get_device_memory;
  add0 "screenResolution" "getScreenResolution" Sources_simple.get_screen_resolution;
  add0 "hardwareConcurrency" "getHardwareConcurrency" Sources_simple.get_hardware_concurrency;
  add0 "timezone" "getTimezone" Sources_simple.get_timezone;
  add0 "sessionStorage" "getSessionStorage" Sources_simple.get_session_storage;
  add0 "localStorage" "getLocalStorage" Sources_simple.get_local_storage;
  add0 "indexedDB" "getIndexedDB" Sources_simple.get_indexed_db;
  add0 "openDatabase" "getOpenDatabase" Sources_simple.get_open_database;
  add0 "cpuClass" "getCpuClass" Sources_simple.get_cpu_class;
  add0 "platform" "getPlatform" Sources_simple.get_platform;
  add0 "plugins" "getPlugins" Sources_simple.get_plugins;
  add0 "touchSupport" "getTouchSupport" Sources_simple.get_touch_support;
  add0 "vendor" "getVendor" Sources_simple.get_vendor;
  add0 "vendorFlavors" "getVendorFlavors" Sources_simple.get_vendor_flavors;
  add0 "cookiesEnabled" "areCookiesEnabled" Sources_simple.are_cookies_enabled;
  add0 "colorGamut" "getColorGamut" Sources_media.get_color_gamut;
  add0 "invertedColors" "areColorsInverted" Sources_media.are_colors_inverted;
  add0 "forcedColors" "areColorsForced" Sources_media.are_colors_forced;
  add0 "monochrome" "getMonochromeDepth" Sources_media.get_monochrome_depth;
  add0 "contrast" "getContrastPreference" Sources_media.get_contrast_preference;
  add0 "reducedMotion" "isMotionReduced" Sources_media.is_motion_reduced;
  add0 "reducedTransparency" "isTransparencyReduced" Sources_media.is_transparency_reduced;
  add0 "hdr" "isHDR" Sources_media.is_hdr;
  add0 "math" "getMathFingerprint" Sources_simple.get_math_fingerprint;
  add0 "pdfViewerEnabled" "isPdfViewerEnabled" Sources_simple.get_pdf_viewer_enabled;
  add0 "architecture" "getArchitecture" Sources_simple.get_architecture;
  add0 "applePay" "getApplePayState" Sources_simple.get_apple_pay_state;
  add0 "privateClickMeasurement" "getPrivateClickMeasurement" Sources_simple.get_private_click_measurement;
  add0 "audioBaseLatency" "getAudioContextBaseLatency" Sources_simple.get_audio_base_latency;
  add0 "dateTimeLocale" "getDateTimeLocale" Sources_simple.get_date_time_locale;
  add1 "webGlBasics" "getWebGlBasics" Webgl.get_webgl_basics;
  add1 "webGlExtensions" "getWebGlExtensions" Webgl.get_webgl_extensions;
  sources

let load_builtin_sources (options : js) : js =
  let exclude = array_ () in
  Entropy.load_sources (make_sources ()) options exclude undefined
