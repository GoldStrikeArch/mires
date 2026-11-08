library;

import 'dart:js_interop';

import 'browser.dart';
import 'data.dart';
import 'fp_async.dart';
import 'host.dart';

const screenFrameCheckInterval = 2500;
const roundingPrecision = 10.0;

JSAny? _screenFrameBackup;
JSAny? _screenFrameSizeTimeoutId;

void resetScreenFrameWatch() {
  if (_screenFrameSizeTimeoutId.isDefinedAndNotNull) {
    jsClearTimeout(_screenFrameSizeTimeoutId);
    _screenFrameSizeTimeoutId = jsUndefined;
  }
  _screenFrameBackup = jsUndefined;
}

void watchScreenFrame() {
  if (_screenFrameSizeTimeoutId.isDefinedAndNotNull) return;
  void checkScreenFrame() {
    final frameSize = getCurrentScreenFrame();
    if (isFrameSizeNull(frameSize)) {
      _screenFrameSizeTimeoutId = jsSetTimeout((() {
        checkScreenFrame();
      }).toJS, screenFrameCheckInterval);
    } else {
      _screenFrameBackup = frameSize;
      _screenFrameSizeTimeoutId = jsUndefined;
    }
  }

  checkScreenFrame();
}

JSAny? getUnstableScreenFrame([JSAny? _]) {
  watchScreenFrame();
  return dartFun0('', () {
    return futureToPromise(() async {
      var frameSize = getCurrentScreenFrame();
      if (isFrameSizeNull(frameSize)) {
        if (_screenFrameBackup.isDefinedAndNotNull) {
          return method(_screenFrameBackup, 'slice');
        }
        if (jsTruthy(getFullscreenElement())) {
          await _awaitJs(exitFullscreen());
          frameSize = getCurrentScreenFrame();
        }
      }
      if (!isFrameSizeNull(frameSize)) {
        _screenFrameBackup = frameSize;
      }
      return frameSize;
    }());
  });
}

JSAny? getScreenFrame([JSAny? _]) {
  final isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit();
  final isFirefox143OrAbove = isGecko() && isGecko143OrNewer();
  if (isSafari17OrAbove || isFirefox143OrAbove) {
    return dartFun0('', () => promiseResolve(jsUndefined));
  }
  final getter = getUnstableScreenFrame();
  return dartFun0('', () {
    return then1(
      jsCall(getter, jsUndefined),
      ((JSAny? frameSize) {
        JSAny? processSize(JSAny? side) {
          if (side.isNull || jsString(side) == 'null') return jsNull;
          if (side.isUndefinedOrNull) return jsNull;
          return fromDart(roundTo(jsNumber(side), roundingPrecision));
        }

        return jsArray([
          processSize(jsIndex(frameSize, 0)),
          processSize(jsIndex(frameSize, 1)),
          processSize(jsIndex(frameSize, 2)),
          processSize(jsIndex(frameSize, 3)),
        ]);
      }).toJS,
    );
  });
}

JSAny? getCurrentScreenFrame() {
  final s = jsScreen;
  JSAny? nanToNull(JSAny? value) => replaceNaN(toFloat(value), jsNull);
  final availTop = nanToNull(getp(s, 'availTop'));
  final availLeft = replaceNaN(toFloat(getp(s, 'availLeft')), 0.toJS);
  return jsArray([
    availTop,
    replaceNaN(
      fromDart(
        jsNumber(toFloat(getp(s, 'width'))) -
            jsNumber(toFloat(getp(s, 'availWidth'))) -
            jsNumber(availLeft),
      ),
      jsNull,
    ),
    replaceNaN(
      fromDart(
        jsNumber(toFloat(getp(s, 'height'))) -
            jsNumber(toFloat(getp(s, 'availHeight'))) -
            jsNumber(replaceNaN(toFloat(getp(s, 'availTop')), 0.toJS)),
      ),
      jsNull,
    ),
    nanToNull(getp(s, 'availLeft')),
  ]);
}

bool isFrameSizeNull(JSAny? frameSize) {
  for (var i = 0; i < 4; i++) {
    if (jsTruthy(jsIndex(frameSize, i))) return false;
  }
  return true;
}

JSAny? getUserAgentData([JSAny? _]) {
  final uaData = getp(jsNavigator, 'userAgentData');
  if (!jsTruthy(uaData)) return promiseResolve(jsUndefined);

  final brandsRaw = getp(uaData, 'brands');
  final filtered = jsArray();
  for (var i = 0; i < jsLength(brandsRaw); i++) {
    final brand = getp(jsIndex(brandsRaw, i), 'brand');
    if (!jsTest(jsRegex('not', 'i'), jsString(brand))) {
      jsPush(filtered, brand);
    }
  }
  JSAny? brands = filtered;
  if (jsLength(filtered) > 1) {
    final without = jsArray();
    for (var i = 0; i < jsLength(filtered); i++) {
      if (jsString(jsIndex(filtered, i)) != 'Chromium') {
        jsPush(without, jsIndex(filtered, i));
      }
    }
    brands = without;
  }

  final result = jsObject();
  setp(result, 'brands', brands);
  setp(result, 'mobile', getp(uaData, 'mobile'));
  setp(result, 'platform', getp(uaData, 'platform'));

  final getHigh = getp(uaData, 'getHighEntropyValues');
  if (!isFunction(getHigh)) return promiseResolve(result);

  return futureToPromise(() async {
    final hints = jsArray([
      'architecture'.toJS,
      'bitness'.toJS,
      'model'.toJS,
      'platformVersion'.toJS,
    ]);
    final (ok, value) = tryCall(getHigh, uaData, [hints]);
    if (!ok) {
      if (_isNotAllowed(value)) {
        setp(result, 'highEntropyStatus', 'not_allowed'.toJS);
        return result;
      }
      throw value ?? makeError('Error', 'userAgentData')!;
    }
    JSAny? highEntropy = value;
    if (isPromise(value)) {
      try {
        highEntropy = await (value as JSPromise<JSAny?>).toDart;
      } catch (error) {
        final jsError = error is JSAny ? error : makeError('Error', error.toString());
        if (_isNotAllowed(jsError)) {
          setp(result, 'highEntropyStatus', 'not_allowed'.toJS);
          return result;
        }
        rethrow;
      }
    }
    setp(result, 'architecture', getp(highEntropy, 'architecture'));
    setp(result, 'bitness', getp(highEntropy, 'bitness'));
    setp(result, 'model', getp(highEntropy, 'model'));
    setp(result, 'platformVersion', getp(highEntropy, 'platformVersion'));
    return result;
  }());
}

bool _isNotAllowed(JSAny? error) {
  final ctor = g('DOMException');
  return instanceOf(error, ctor) && jsString(getp(error, 'name')) == 'NotAllowedError';
}

Future<JSAny?> _awaitJs(JSAny? promise) async {
  if (!isPromise(promise)) return promise;
  return (promise as JSPromise<JSAny?>).toDart;
}
