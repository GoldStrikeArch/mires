library;

import 'dart:js_interop';

import 'data.dart';
import 'host.dart';
import 'misc.dart';

JSAny? getFullscreenElement() {
  final d = jsDocument;
  final value = getp(d, 'fullscreenElement');
  if (value.isDefinedAndNotNull) return value;
  final ms = getp(d, 'msFullscreenElement');
  if (ms.isDefinedAndNotNull) return ms;
  final moz = getp(d, 'mozFullScreenElement');
  if (moz.isDefinedAndNotNull) return moz;
  final webkit = getp(d, 'webkitFullscreenElement');
  if (webkit.isDefinedAndNotNull) return webkit;
  return jsNull;
}

JSAny? exitFullscreen() {
  final d = jsDocument;
  final fn = getp(d, 'exitFullscreen');
  final chosen = fn.isDefinedAndNotNull
      ? fn
      : getp(d, 'msExitFullscreen').isDefinedAndNotNull
          ? getp(d, 'msExitFullscreen')
          : getp(d, 'mozCancelFullScreen').isDefinedAndNotNull
              ? getp(d, 'mozCancelFullScreen')
              : getp(d, 'webkitExitFullscreen');
  return jsCall(chosen, d);
}

bool isTrident() {
  final w = jsWindow;
  final n = jsNavigator;
  return countTruthy([
        hasIn(w, 'MSCSSMatrix'),
        hasIn(w, 'msSetImmediate'),
        hasIn(w, 'msIndexedDB'),
        hasIn(n, 'msMaxTouchPoints'),
        hasIn(n, 'msPointerEnabled'),
      ]) >=
      4;
}

bool isEdgeHTML() {
  final w = jsWindow;
  final n = jsNavigator;
  return countTruthy([
            hasIn(w, 'msWriteProfilerMark'),
            hasIn(w, 'MSStream'),
            hasIn(n, 'msLaunchUri'),
            hasIn(n, 'msSaveBlob'),
          ]) >=
          3 &&
      !isTrident();
}

bool isChromium() {
  final w = jsWindow;
  final n = jsNavigator;
  final vendor = getp(n, 'vendor');
  final vendorStr = vendor.isUndefinedOrNull ? '' : jsString(vendor);
  return countTruthy([
        hasIn(n, 'webkitPersistentStorage'),
        hasIn(n, 'webkitTemporaryStorage'),
        vendorStr.indexOf('Google') == 0,
        hasIn(w, 'webkitResolveLocalFileSystemURL'),
        hasIn(w, 'BatteryManager'),
        hasIn(w, 'webkitMediaStream'),
        hasIn(w, 'webkitSpeechGrammar'),
      ]) >=
      5;
}

bool isWebKit() {
  final w = jsWindow;
  final n = jsNavigator;
  final vendor = getp(n, 'vendor');
  final vendorStr = vendor.isUndefinedOrNull ? '' : jsString(vendor);
  return countTruthy([
        hasIn(w, 'ApplePayError'),
        hasIn(w, 'CSSPrimitiveValue'),
        hasIn(w, 'Counter'),
        vendorStr.indexOf('Apple') == 0,
        hasIn(w, 'RGBColor'),
        hasIn(w, 'WebKitMediaKeys'),
      ]) >=
      4;
}

bool isDesktopWebKit() {
  final w = jsWindow;
  final htmlElement = getp(w, 'HTMLElement');
  final documentCtor = getp(w, 'Document');
  final htmlProto = getp(htmlElement, 'prototype');
  final documentProto = getp(documentCtor, 'prototype');
  return countTruthy([
        hasIn(w, 'safari'),
        !hasIn(w, 'ongestureend'),
        !hasIn(w, 'TouchEvent'),
        !hasIn(w, 'orientation'),
        htmlElement.isDefinedAndNotNull && !hasIn(htmlProto, 'autocapitalize'),
        documentCtor.isDefinedAndNotNull &&
            hasIn(documentProto, 'pointerLockElement'),
      ]) >=
      4;
}

bool isSafariWebKit() {
  final w = jsWindow;
  return isFunctionNative(getp(w, 'print')) &&
      jsString(getp(w, 'browser')) == '[object WebPageNamespace]';
}

bool isGecko() {
  final w = jsWindow;
  final style = getp(getp(jsDocument, 'documentElement'), 'style');
  final styleTarget = style.isDefinedAndNotNull ? style : jsObject();
  return countTruthy([
        hasIn(jsNavigator, 'buildID'),
        hasIn(styleTarget, 'MozAppearance'),
        hasIn(w, 'onmozfullscreenchange'),
        hasIn(w, 'mozInnerScreenX'),
        hasIn(w, 'CSSMozDocumentRule'),
        hasIn(w, 'CanvasCaptureMediaStream'),
      ]) >=
      4;
}

bool isGecko120OrNewer() {
  final w = jsWindow;
  final n = jsNavigator;
  return countTruthy([
        hasIn(n, 'userActivation'),
        cssSupports('color', 'light-dark(#000, #fff)'),
        cssSupports('height', '1lh'),
        hasIn(n, 'globalPrivacyControl'),
      ]) >=
      3;
}

bool isGecko143OrNewer() {
  final composition = getp(jsWindow, 'CompositionEvent');
  final proto = getp(composition, 'prototype');
  return countTruthy([
        cssSupportsQuery('selector(::details-content)'),
        cssSupportsQuery('selector(::before::marker)'),
        cssSupportsQuery('selector(::after::marker)'),
        !hasIn(proto, 'locale'),
      ]) >=
      3;
}

bool isChromium86OrNewer() {
  final w = jsWindow;
  return countTruthy([
        !hasIn(w, 'MediaSettingsRange'),
        hasIn(w, 'RTCEncodedAudioFrame'),
        jsString(getp(w, 'Intl')) == '[object Intl]',
        jsString(getp(w, 'Reflect')) == '[object Reflect]',
      ]) >=
      3;
}

bool isChromium122OrNewer() {
  final w = jsWindow;
  final urlPattern = getp(w, 'URLPattern');
  final setCtor = g('Set');
  final setProto = getp(setCtor, 'prototype');
  final webgl = getp(w, 'WebGLRenderingContext');
  final webglProto = getp(webgl, 'prototype');
  return countTruthy([
        hasIn(setProto, 'union'),
        hasIn(w, 'Iterator'),
        urlPattern.isDefinedAndNotNull &&
            hasIn(getp(urlPattern, 'prototype'), 'hasRegExpGroups'),
        hasIn(webglProto, 'RGB8'),
      ]) >=
      3;
}

bool isChromium128OrNewer() {
  final w = jsWindow;
  final d = jsDocument;
  final promise = getp(w, 'Promise');
  final audio = getp(w, 'AudioContext');
  return countTruthy([
        promise.isDefinedAndNotNull && hasIn(promise, 'try'),
        hasIn(d, 'caretPositionFromPoint'),
        audio.isDefinedAndNotNull &&
            hasIn(getp(audio, 'prototype'), 'onerror'),
        cssSupports('ruby-align', 'space-around'),
      ]) >=
      3;
}

bool isWebKit606OrNewer() {
  final w = jsWindow;
  return countTruthy([
        hasIn(w, 'DOMRectList'),
        hasIn(w, 'RTCPeerConnectionIceEvent'),
        hasIn(w, 'SVGGeometryElement'),
        hasIn(w, 'ontransitioncancel'),
      ]) >=
      3;
}

bool isWebKit616OrNewer() {
  final w = jsWindow;
  final n = jsNavigator;
  final button = getp(w, 'HTMLButtonElement');
  return countTruthy([
        !hasIn(n, 'getStorageUpdates'),
        button.isDefinedAndNotNull &&
            hasIn(getp(button, 'prototype'), 'popover'),
        hasIn(w, 'CSSCounterStyleRule'),
        cssSupportsQuery('font-size-adjust: ex-height 0.5'),
        cssSupportsQuery('text-transform: full-width'),
      ]) >=
      4;
}

bool isIPad() {
  if (jsString(getp(jsNavigator, 'platform')) == 'iPad') {
    return true;
  }
  final s = jsScreen;
  final width = jsNumber(getp(s, 'width'));
  final height = jsNumber(getp(s, 'height'));
  final screenRatio = width / height;
  final element = getp(jsWindow, 'Element');
  final proto = getp(element, 'prototype');
  return countTruthy([
        hasIn(jsWindow, 'MediaSource'),
        jsTruthy(getp(proto, 'webkitRequestFullscreen')),
        screenRatio > 0.65 && screenRatio < 1.53,
      ]) >=
      2;
}

bool isAndroid() {
  final isItChromium = isChromium();
  final isItGecko = isGecko();
  final w = jsWindow;
  final n = jsNavigator;
  if (isItChromium) {
    final connection = getp(n, 'connection');
    final audioCtor = getp(w, 'Audio');
    final audio = jsNew0(audioCtor);
    return countTruthy([
          !hasIn(w, 'SharedWorker'),
          connection.isDefinedAndNotNull && hasIn(connection, 'ontypechange'),
          !hasIn(audio, 'sinkId'),
        ]) >=
        2;
  } else if (isItGecko) {
    return countTruthy([
          hasIn(w, 'onorientationchange'),
          hasIn(w, 'orientation'),
          jsTest(jsRegex('android', 'i'), jsString(getp(n, 'appVersion'))),
        ]) >=
        2;
  }
  return false;
}

bool isSamsungInternet() {
  final n = jsNavigator;
  final w = jsWindow;
  final audio = getp(w, 'Audio');
  final audioPrototype = getp(audio, 'prototype');
  final visualViewport = getp(w, 'visualViewport');
  final image = getp(w, 'Image');
  return countTruthy([
        hasIn(audioPrototype, 'srLatency'),
        hasIn(audioPrototype, 'srChannelCount'),
        hasIn(n, 'devicePosture'),
        visualViewport.isDefinedAndNotNull && hasIn(visualViewport, 'segments'),
        hasIn(getp(image, 'prototype'), 'getTextInformation'),
      ]) >=
      3;
}

JSAny? exportBool(String name, bool Function() impl) {
  return dartFun0(name, () => jsBool(impl()));
}
