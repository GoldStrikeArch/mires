library;

import 'dart:js_interop';

import 'browser.dart';
import 'data.dart';
import 'host.dart';

JSAny? getOsCpu([JSAny? _]) => getp(jsNavigator, 'oscpu');

JSAny? getLanguages([JSAny? _]) {
  final n = jsNavigator;
  final result = jsArray();
  final language = getp(n, 'language').isDefinedAndNotNull
      ? getp(n, 'language')
      : getp(n, 'userLanguage').isDefinedAndNotNull
          ? getp(n, 'userLanguage')
          : getp(n, 'browserLanguage').isDefinedAndNotNull
              ? getp(n, 'browserLanguage')
              : getp(n, 'systemLanguage');
  if (!language.isUndefined) {
    jsPush(result, jsArray([language]));
  }
  final languages = getp(n, 'languages');
  if (isArray(languages)) {
    if (!(isChromium() && isChromium86OrNewer())) {
      jsPush(result, languages);
    }
  } else if (typof(languages) == 'string') {
    if (jsTruthy(languages)) {
      jsPush(result, jsSplit(languages, ','));
    }
  }
  return result;
}

JSAny? getColorDepth([JSAny? _]) => getp(jsScreen, 'colorDepth');

JSAny? getDeviceMemory([JSAny? _]) =>
    replaceNaN(toFloat(getp(jsNavigator, 'deviceMemory')), jsUndefined);

JSAny? getTimezone([JSAny? _]) {
  final intl = getp(jsWindow, 'Intl');
  final dateTimeFormat = getp(intl, 'DateTimeFormat');
  if (dateTimeFormat.isDefinedAndNotNull) {
    final timezone = getp(method(jsNew0(dateTimeFormat), 'resolvedOptions'), 'timeZone');
    if (jsTruthy(timezone)) return timezone;
  }
  final offset = -_timezoneOffset();
  final sign = offset >= 0 ? '+' : '';
  return 'UTC$sign${offset == offset.toInt() ? offset.toInt() : offset}'.toJS;
}

double _timezoneOffset() {
  final year = dateYear();
  return mathMax(jsParseFloat(fromDart(dateMonthOffset(year, 0))),
      jsParseFloat(fromDart(dateMonthOffset(year, 6))));
}

JSAny? getSessionStorage([JSAny? _]) {
  final (ok, value) = tryHost(() => fromDart(jsTruthy(getp(jsWindow, 'sessionStorage'))));
  if (!ok) return true.toJS;
  return value;
}

JSAny? getLocalStorage([JSAny? _]) {
  final (ok, value) = tryHost(() => fromDart(jsTruthy(getp(jsWindow, 'localStorage'))));
  if (!ok) return true.toJS;
  return value;
}

JSAny? getIndexedDB([JSAny? _]) {
  if (isTrident() || isEdgeHTML()) return jsUndefined;
  final (ok, value) = tryHost(() => fromDart(jsTruthy(getp(jsWindow, 'indexedDB'))));
  if (!ok) return true.toJS;
  return value;
}

JSAny? getOpenDatabase([JSAny? _]) =>
    fromDart(jsTruthy(getp(jsWindow, 'openDatabase')));

JSAny? getCpuClass([JSAny? _]) => getp(jsNavigator, 'cpuClass');

JSAny? getPlatform([JSAny? _]) {
  final platform = getp(jsNavigator, 'platform');
  if (jsString(platform) == 'MacIntel') {
    if (isWebKit() && !isDesktopWebKit()) {
      return (isIPad() ? 'iPad' : 'iPhone').toJS;
    }
  }
  return platform;
}

JSAny? getPlugins([JSAny? _]) {
  final rawPlugins = getp(jsNavigator, 'plugins');
  if (!jsTruthy(rawPlugins)) return jsUndefined;
  final plugins = jsArray();
  for (var i = 0; i < jsLength(rawPlugins); i++) {
    final plugin = jsIndex(rawPlugins, i);
    if (!jsTruthy(plugin)) continue;
    final mimeTypes = jsArray();
    for (var j = 0; j < jsLength(plugin); j++) {
      final mimeType = jsIndex(plugin, j);
      final item = jsObject();
      setp(item, 'type', getp(mimeType, 'type'));
      setp(item, 'suffixes', getp(mimeType, 'suffixes'));
      jsPush(mimeTypes, item);
    }
    final entry = jsObject();
    setp(entry, 'name', getp(plugin, 'name'));
    setp(entry, 'description', getp(plugin, 'description'));
    setp(entry, 'mimeTypes', mimeTypes);
    jsPush(plugins, entry);
  }
  return plugins;
}

JSAny? getTouchSupport([JSAny? _]) {
  final n = jsNavigator;
  var maxTouchPoints = 0.0;
  if (!getp(n, 'maxTouchPoints').isUndefined) {
    maxTouchPoints = jsParseInt(getp(n, 'maxTouchPoints'));
  } else if (!getp(n, 'msMaxTouchPoints').isUndefined) {
    maxTouchPoints = jsNumber(getp(n, 'msMaxTouchPoints'));
  }
  var touchEvent = false;
  final createEvent = getp(jsDocument, 'createEvent');
  final (ok, _) = tryCall(createEvent, jsDocument, ['TouchEvent'.toJS]);
  touchEvent = ok;
  final result = jsObject();
  setp(result, 'maxTouchPoints', maxTouchPoints.toJS);
  setp(result, 'touchEvent', touchEvent.toJS);
  setp(result, 'touchStart', hasIn(jsWindow, 'ontouchstart').toJS);
  return result;
}

JSAny? getVendor([JSAny? _]) {
  final vendor = getp(jsNavigator, 'vendor');
  return vendor.isUndefinedOrNull ? ''.toJS : vendor;
}

JSAny? getVendorFlavors([JSAny? _]) {
  final flavors = jsArray();
  const keys = [
    'chrome',
    'safari',
    '__crWeb',
    '__gCrWeb',
    'yandex',
    '__yb',
    '__ybro',
    '__firefox__',
    '__edgeTrackingPreventionStatistics',
    'webkit',
    'oprt',
    'samsungAr',
    'ucweb',
    'UCShellJava',
    'puffinDevice',
  ];
  for (final key in keys) {
    final value = getp(jsWindow, key);
    if (jsTruthy(value) && typof(value) == 'object') {
      jsPush(flavors, key.toJS);
    }
  }
  method(flavors, 'sort');
  return flavors;
}

JSAny? areCookiesEnabled([JSAny? _]) {
  final d = jsDocument;
  final (ok, value) = tryHost(() {
    setp(d, 'cookie', 'cookietest=1; SameSite=Strict;'.toJS);
    final result = jsIndexOf(getp(d, 'cookie'), 'cookietest=') != -1;
    setp(
      d,
      'cookie',
      'cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT'
          .toJS,
    );
    return result.toJS;
  });
  if (!ok) return false.toJS;
  return value;
}

JSAny? getColorGamut([JSAny? _]) {
  for (final gamut in ['rec2020', 'p3', 'srgb']) {
    if (matchMediaMatches('(color-gamut: $gamut)')) return gamut.toJS;
  }
  return jsUndefined;
}

JSAny? areColorsInverted([JSAny? _]) {
  if (matchMediaMatches('(inverted-colors: inverted)')) return true.toJS;
  if (matchMediaMatches('(inverted-colors: none)')) return false.toJS;
  return jsUndefined;
}

JSAny? areColorsForced([JSAny? _]) {
  if (matchMediaMatches('(forced-colors: active)')) return true.toJS;
  if (matchMediaMatches('(forced-colors: none)')) return false.toJS;
  return jsUndefined;
}

JSAny? getMonochromeDepth([JSAny? _]) {
  if (!matchMediaMatches('(min-monochrome: 0)')) return jsUndefined;
  for (var i = 0; i <= 100; i++) {
    if (matchMediaMatches('(max-monochrome: $i)')) return i.toJS;
  }
  throw makeError('Error', 'Too high value')!;
}

JSAny? getContrastPreference([JSAny? _]) {
  if (matchMediaMatches('(prefers-contrast: no-preference)')) return 0.toJS;
  if (matchMediaMatches('(prefers-contrast: high)') ||
      matchMediaMatches('(prefers-contrast: more)')) {
    return 1.toJS;
  }
  if (matchMediaMatches('(prefers-contrast: low)') ||
      matchMediaMatches('(prefers-contrast: less)')) {
    return (-1).toJS;
  }
  if (matchMediaMatches('(prefers-contrast: forced)')) return 10.toJS;
  return jsUndefined;
}

JSAny? isMotionReduced([JSAny? _]) {
  if (matchMediaMatches('(prefers-reduced-motion: reduce)')) return true.toJS;
  if (matchMediaMatches('(prefers-reduced-motion: no-preference)')) {
    return false.toJS;
  }
  return jsUndefined;
}

JSAny? isTransparencyReduced([JSAny? _]) {
  if (matchMediaMatches('(prefers-reduced-transparency: reduce)')) {
    return true.toJS;
  }
  if (matchMediaMatches('(prefers-reduced-transparency: no-preference)')) {
    return false.toJS;
  }
  return jsUndefined;
}

JSAny? isHDR([JSAny? _]) {
  if (matchMediaMatches('(dynamic-range: high)')) return true.toJS;
  if (matchMediaMatches('(dynamic-range: standard)')) return false.toJS;
  return jsUndefined;
}

JSAny? getMathFingerprint([JSAny? _]) {
  double call(String name, num value) => mathCall(name, value);
  double fallback(num value) => 0;
  double use(String name, num value) =>
      isFunction(getp(g('Math'), name)) ? call(name, value) : fallback(value);

  double powPI(num value) => mathPow(mathPI(), value);
  double acoshPf(num value) =>
      mathCall('log', value + mathCall('sqrt', value * value - 1));
  double asinhPf(num value) =>
      mathCall('log', value + mathCall('sqrt', value * value + 1));
  double atanhPf(num value) => mathCall('log', (1 + value) / (1 - value)) / 2;
  double sinhPf(num value) =>
      mathCall('exp', value) - 1 / mathCall('exp', value) / 2;
  double coshPf(num value) =>
      (mathCall('exp', value) + 1 / mathCall('exp', value)) / 2;
  double expm1Pf(num value) => mathCall('exp', value) - 1;
  double tanhPf(num value) =>
      (mathCall('exp', 2 * value) - 1) / (mathCall('exp', 2 * value) + 1);
  double log1pPf(num value) => mathCall('log', 1 + value);

  final result = jsObject();
  setp(result, 'acos', use('acos', 0.123124234234234242).toJS);
  setp(result, 'acosh', use('acosh', 1e308).toJS);
  setp(result, 'acoshPf', acoshPf(1e154).toJS);
  setp(result, 'asin', use('asin', 0.123124234234234242).toJS);
  setp(result, 'asinh', use('asinh', 1).toJS);
  setp(result, 'asinhPf', asinhPf(1).toJS);
  setp(result, 'atanh', use('atanh', 0.5).toJS);
  setp(result, 'atanhPf', atanhPf(0.5).toJS);
  setp(result, 'atan', use('atan', 0.5).toJS);
  setp(result, 'sin', use('sin', -1e300).toJS);
  setp(result, 'sinh', use('sinh', 1).toJS);
  setp(result, 'sinhPf', sinhPf(1).toJS);
  setp(result, 'cos', use('cos', 10.000000000123).toJS);
  setp(result, 'cosh', use('cosh', 1).toJS);
  setp(result, 'coshPf', coshPf(1).toJS);
  setp(result, 'tan', use('tan', -1e300).toJS);
  setp(result, 'tanh', use('tanh', 1).toJS);
  setp(result, 'tanhPf', tanhPf(1).toJS);
  setp(result, 'exp', use('exp', 1).toJS);
  setp(result, 'expm1', use('expm1', 1).toJS);
  setp(result, 'expm1Pf', expm1Pf(1).toJS);
  setp(result, 'log1p', use('log1p', 10).toJS);
  setp(result, 'log1pPf', log1pPf(10).toJS);
  setp(result, 'powPI', powPI(-100).toJS);
  return result;
}

JSAny? isPdfViewerEnabled([JSAny? _]) => getp(jsNavigator, 'pdfViewerEnabled');

JSAny? getArchitecture([JSAny? _]) {
  final float32 = jsNew1(g('Float32Array'), 1.toJS);
  final u8 = jsNew1(g('Uint8Array'), getp(float32, 'buffer'));
  jsSetIndex(float32, 0, g('Infinity'));
  jsSetIndex(float32, 0, fromDart(jsNumber(jsIndex(float32, 0)) - jsNumber(jsIndex(float32, 0))));
  return jsIndex(u8, 3);
}

JSAny? getApplePayState([JSAny? _]) {
  final session = getp(jsWindow, 'ApplePaySession');
  if (typof(getp(session, 'canMakePayments')) != 'function') {
    return (-1).toJS;
  }
  if (isAnyParentCrossOriginSafe()) return (-3).toJS;
  final (ok, value) = tryCall(getp(session, 'canMakePayments'), session);
  if (!ok) return _applePayFromError(value);
  return jsTruthy(value) ? 1.toJS : 0.toJS;
}

bool isAnyParentCrossOriginSafe() {
  // Imported from dom via a late wrapper to avoid a cycle at initialization.
  return _crossOrigin();
}

bool Function() _crossOrigin = () => false;

void bindCrossOrigin(bool Function() impl) {
  _crossOrigin = impl;
}

JSAny? _applePayFromError(JSAny? error) {
  if (isError(error) &&
      jsString(getp(error, 'name')) == 'InvalidAccessError' &&
      jsTest(jsRegex(r'\bfrom\b.*\binsecure\b', 'i'), jsString(getp(error, 'message')))) {
    return (-2).toJS;
  }
  throw error ?? makeError('Error', 'Apple Pay')!;
}

JSAny? getPrivateClickMeasurement([JSAny? _]) {
  final link = method(jsDocument, 'createElement', ['a'.toJS]);
  var sourceId = getp(link, 'attributionSourceId');
  if (sourceId.isUndefined) sourceId = getp(link, 'attributionsourceid');
  return sourceId.isUndefined ? jsUndefined : fromDart(jsString(sourceId));
}

JSAny? getAudioContextBaseLatency([JSAny? _]) {
  final allowed = isAndroid() || isWebKit();
  if (!allowed) return (-2).toJS;
  final ctor = getp(jsWindow, 'AudioContext');
  if (!jsTruthy(ctor)) return (-1).toJS;
  final latency = getp(jsNew0(ctor), 'baseLatency');
  if (latency.isUndefinedOrNull) return (-1).toJS;
  if (!jsIsFinite(latency)) return (-3).toJS;
  return latency;
}

JSAny? getDateTimeLocale([JSAny? _]) {
  final intl = getp(jsWindow, 'Intl');
  if (!jsTruthy(intl)) return (-1).toJS;
  final dateTimeFormat = getp(intl, 'DateTimeFormat');
  if (!jsTruthy(dateTimeFormat)) return (-2).toJS;
  final locale = getp(method(jsCall(dateTimeFormat, jsUndefined), 'resolvedOptions'), 'locale');
  if (!jsTruthy(locale) && jsString(locale) != '') {
    return (-3).toJS;
  }
  return locale;
}

JSAny? getUnstableHardwareConcurrency([JSAny? _]) =>
    replaceNaN(toInt(getp(jsNavigator, 'hardwareConcurrency')), jsUndefined);

JSAny? getHardwareConcurrency([JSAny? _]) {
  final value = getUnstableHardwareConcurrency();
  if (!value.isUndefined && isGecko() && isGecko143OrNewer()) {
    return (jsNumber(value) >= 8 ? 8 : 4).toJS;
  }
  return value;
}

JSAny? getUnstableScreenResolution([JSAny? _]) {
  final s = jsScreen;
  JSAny? parseDimension(JSAny? value) => replaceNaN(toInt(value), jsNull);
  final dimensions = jsArray([
    parseDimension(getp(s, 'width')),
    parseDimension(getp(s, 'height')),
  ]);
  method(dimensions, 'sort');
  method(dimensions, 'reverse');
  return dimensions;
}

JSAny? getScreenResolution([JSAny? _]) {
  if (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) {
    return jsUndefined;
  }
  return getUnstableScreenResolution();
}
