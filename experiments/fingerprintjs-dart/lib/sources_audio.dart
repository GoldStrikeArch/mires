library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'browser.dart';
import 'fp_async.dart';
import 'host.dart';

const knownForSuspending = -1;
const notSupported = -2;
const timeoutStatus = -3;
const knownForAntifingerprinting = -4;

JSAny? getAudioFingerprint([JSAny? _]) {
  if (_doesBrowserPerformAntifingerprinting()) {
    return knownForAntifingerprinting.toJS;
  }
  return getUnstableAudioFingerprint();
}

JSAny? getUnstableAudioFingerprint([JSAny? _]) {
  final w = jsWindow;
  final audioContext = getp(w, 'OfflineAudioContext').isDefinedAndNotNull
      ? getp(w, 'OfflineAudioContext')
      : getp(w, 'webkitOfflineAudioContext');
  if (!jsTruthy(audioContext)) return notSupported.toJS;
  if (_doesBrowserSuspendAudioContext()) return knownForSuspending.toJS;

  const hashFromIndex = 4500;
  const hashToIndex = 5000;
  final context = jsNew3(audioContext, 1.toJS, hashToIndex.toJS, 44100.toJS);

  final oscillator = method(context, 'createOscillator');
  setp(oscillator, 'type', 'triangle'.toJS);
  setp(getp(oscillator, 'frequency'), 'value', 10000.toJS);

  final compressor = method(context, 'createDynamicsCompressor');
  setp(getp(compressor, 'threshold'), 'value', (-50).toJS);
  setp(getp(compressor, 'knee'), 'value', 40.toJS);
  setp(getp(compressor, 'ratio'), 'value', 12.toJS);
  setp(getp(compressor, 'attack'), 'value', 0.toJS);
  setp(getp(compressor, 'release'), 'value', 0.25.toJS);

  hostHelpers.callMethod('call1'.toJS, getp(oscillator, 'connect'), oscillator, compressor);
  hostHelpers.callMethod(
    'call1'.toJS,
    getp(compressor, 'connect'),
    compressor,
    getp(context, 'destination'),
  );
  hostHelpers.callMethod('call1'.toJS, getp(oscillator, 'start'), oscillator, 0.toJS);

  final rendering = _startRenderingAudio(context);
  final guarded = suppress(newPromise(((JSAny? resolve, JSAny? reject) {
    then2(
      rendering.promise,
      ((JSAny? buffer) {
        final data = hostHelpers.callMethod(
          'call1'.toJS,
          getp(buffer, 'getChannelData'),
          buffer,
          0.toJS,
        );
        final signal = hostHelpers.callMethod(
          'call1'.toJS,
          getp(data, 'subarray'),
          data,
          hashFromIndex.toJS,
        );
        resolve1(resolve, hostHelpers.callMethod('hashSignal'.toJS, signal));
      }).toJS,
      ((JSAny? error) {
        final name = jsString(getp(error, 'name'));
        if (name == 'timeout' || name == 'suspended') {
          jsCall(resolve, jsUndefined, [timeoutStatus.toJS]);
        } else {
          jsCall(reject, jsUndefined, [error]);
        }
      }).toJS,
    );
  }).toJS));

  return dartFun0('', () {
    rendering.finish();
    return guarded;
  });
}

class _RenderHandle {
  _RenderHandle(this.promise, this.finish);
  final JSAny? promise;
  final void Function() finish;
}

_RenderHandle _startRenderingAudio(JSAny? context) {
  const renderTryMaxCount = 3;
  const renderRetryDelay = 500;
  const runningMaxAwaitTime = 500;
  const runningSufficientTime = 5000;
  var finalize = () {};

  final resultPromise = newPromise(((JSAny? resolve, JSAny? reject) {
    var isFinalized = false;
    var renderTryCount = 0;
    var startedRunningAt = 0.0;

    setp(context, 'oncomplete', ((JSAny? event) {
      resolve1(resolve, getp(event, 'renderedBuffer'));
    }).toJS);

    void startRunningTimeout() {
      jsSetTimeout((() {
        jsCall(reject, jsUndefined, [makeError('timeout', 'timeout')]);
      }).toJS, mathMin(runningMaxAwaitTime, startedRunningAt + runningSufficientTime - dateNow()));
    }

    void tryRender() {
      final (ok, error) = tryHost(() {
        final renderingPromise = method(context, 'startRendering');
        if (isPromise(renderingPromise)) {
          suppress(renderingPromise);
        }
        final state = jsString(getp(context, 'state'));
        if (state == 'running') {
          startedRunningAt = dateNow();
          if (isFinalized) startRunningTimeout();
        } else if (state == 'suspended') {
          if (!jsTruthy(getp(jsDocument, 'hidden'))) {
            renderTryCount++;
          }
          if (isFinalized && renderTryCount >= renderTryMaxCount) {
            jsCall(reject, jsUndefined, [makeError('suspended', 'suspended')]);
          } else {
            jsSetTimeout((() {
              tryRender();
            }).toJS, renderRetryDelay);
          }
        }
        return jsUndefined;
      });
      if (!ok) {
        jsCall(reject, jsUndefined, [error]);
      }
    }

    tryRender();
    finalize = () {
      if (!isFinalized) {
        isFinalized = true;
        if (startedRunningAt > 0) startRunningTimeout();
      }
    };
  }).toJS);

  return _RenderHandle(resultPromise, () => finalize());
}

bool _doesBrowserSuspendAudioContext() {
  return isWebKit() && !isDesktopWebKit() && !isWebKit606OrNewer();
}

bool _doesBrowserPerformAntifingerprinting() {
  return (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) ||
      (isChromium() && isSamsungInternet() && isChromium122OrNewer());
}
