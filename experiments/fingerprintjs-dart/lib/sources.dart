library;

import 'dart:js_interop';

import 'dom.dart';
import 'host.dart';
import 'sources_audio.dart';
import 'sources_canvas.dart';
import 'sources_dom_blockers.dart';
import 'sources_font_preferences.dart';
import 'sources_fonts.dart';
import 'sources_frame.dart';
import 'sources_simple.dart';
import 'sources_webgl.dart';

const sourceNames = [
  'userAgentData',
  'fonts',
  'domBlockers',
  'fontPreferences',
  'audio',
  'screenFrame',
  'canvas',
  'osCpu',
  'languages',
  'colorDepth',
  'deviceMemory',
  'screenResolution',
  'hardwareConcurrency',
  'timezone',
  'sessionStorage',
  'localStorage',
  'indexedDB',
  'openDatabase',
  'cpuClass',
  'platform',
  'plugins',
  'touchSupport',
  'vendor',
  'vendorFlavors',
  'cookiesEnabled',
  'colorGamut',
  'invertedColors',
  'forcedColors',
  'monochrome',
  'contrast',
  'reducedMotion',
  'reducedTransparency',
  'hdr',
  'math',
  'pdfViewerEnabled',
  'architecture',
  'applePay',
  'privateClickMeasurement',
  'audioBaseLatency',
  'dateTimeLocale',
  'webGlBasics',
  'webGlExtensions',
];

const sourceFunctionNames = [
  'getUserAgentData',
  'getFonts',
  'getDomBlockers',
  'getFontPreferences',
  'getAudioFingerprint',
  'getScreenFrame',
  'getCanvasFingerprint',
  'getOsCpu',
  'getLanguages',
  'getColorDepth',
  'getDeviceMemory',
  'getScreenResolution',
  'getHardwareConcurrency',
  'getTimezone',
  'getSessionStorage',
  'getLocalStorage',
  'getIndexedDB',
  'getOpenDatabase',
  'getCpuClass',
  'getPlatform',
  'getPlugins',
  'getTouchSupport',
  'getVendor',
  'getVendorFlavors',
  'areCookiesEnabled',
  'getColorGamut',
  'areColorsInverted',
  'areColorsForced',
  'getMonochromeDepth',
  'getContrastPreference',
  'isMotionReduced',
  'isTransparencyReduced',
  'isHDR',
  'getMathFingerprint',
  'isPdfViewerEnabled',
  'getArchitecture',
  'getApplePayState',
  'getPrivateClickMeasurement',
  'getAudioContextBaseLatency',
  'getDateTimeLocale',
  'getWebGlBasics',
  'getWebGlExtensions',
];

JSAny? Function(JSAny?) _sourceAt(int index) {
  switch (index) {
    case 0:
      return getUserAgentData;
    case 1:
      return getFonts;
    case 2:
      return getDomBlockers;
    case 3:
      return getFontPreferences;
    case 4:
      return getAudioFingerprint;
    case 5:
      return getScreenFrame;
    case 6:
      return getCanvasFingerprint;
    case 7:
      return getOsCpu;
    case 8:
      return getLanguages;
    case 9:
      return getColorDepth;
    case 10:
      return getDeviceMemory;
    case 11:
      return getScreenResolution;
    case 12:
      return getHardwareConcurrency;
    case 13:
      return getTimezone;
    case 14:
      return getSessionStorage;
    case 15:
      return getLocalStorage;
    case 16:
      return getIndexedDB;
    case 17:
      return getOpenDatabase;
    case 18:
      return getCpuClass;
    case 19:
      return getPlatform;
    case 20:
      return getPlugins;
    case 21:
      return getTouchSupport;
    case 22:
      return getVendor;
    case 23:
      return getVendorFlavors;
    case 24:
      return areCookiesEnabled;
    case 25:
      return getColorGamut;
    case 26:
      return areColorsInverted;
    case 27:
      return areColorsForced;
    case 28:
      return getMonochromeDepth;
    case 29:
      return getContrastPreference;
    case 30:
      return isMotionReduced;
    case 31:
      return isTransparencyReduced;
    case 32:
      return isHDR;
    case 33:
      return getMathFingerprint;
    case 34:
      return isPdfViewerEnabled;
    case 35:
      return getArchitecture;
    case 36:
      return getApplePayState;
    case 37:
      return getPrivateClickMeasurement;
    case 38:
      return getAudioContextBaseLatency;
    case 39:
      return getDateTimeLocale;
    case 40:
      return getWebGlBasics;
    case 41:
      return getWebGlExtensions;
    default:
      throw StateError('unknown source $index');
  }
}

JSObject? _sourcesCache;

JSObject makeSources() {
  final cached = _sourcesCache;
  if (cached != null) return cached;
  bindCrossOrigin(isAnyParentCrossOrigin);
  final sources = jsObject();
  for (var i = 0; i < sourceNames.length; i++) {
    final impl = _sourceAt(i);
    final arity = i >= 40 ? 1 : 0;
    setp(sources, sourceNames[i], dartFun1(sourceFunctionNames[i], impl, arity: arity));
  }
  _sourcesCache = sources;
  return sources;
}
