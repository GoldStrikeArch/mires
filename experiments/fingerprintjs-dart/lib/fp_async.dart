library;

import 'dart:js_interop';

import 'host.dart';

const loopReleaseInterval = 16.0;

JSAny? wait(num durationMs, [JSAny? resolveWith]) =>
    jsWait(durationMs, resolveWith);

JSAny? releaseEventLoop() => jsRelease();

JSAny? requestIdleCallbackIfAvailable(
  num fallbackTimeout, [
  num deadlineTimeout = double.infinity,
]) {
  final timeout = deadlineTimeout.isFinite ? deadlineTimeout : 2147483647;
  final idle = requestIdle(timeout);
  if (idle.isDefinedAndNotNull) return idle;
  return wait(mathMin(fallbackTimeout, deadlineTimeout));
}

void awaitIfAsync(
  JSAny? Function() action,
  void Function(bool ok, JSAny? value) callback,
) {
  final (ok, value) = tryHost(action);
  if (!ok) {
    callback(false, value);
    return;
  }
  _settleMaybePromise(ok: true, value: value, callback: callback);
}

void awaitIfAsyncCall(
  JSAny? fn,
  JSAny? thisArg,
  List<JSAny?> args,
  void Function(bool ok, JSAny? value) callback,
) {
  final (ok, value) = tryCall(fn, thisArg, args);
  _settleMaybePromise(ok: ok, value: value, callback: callback);
}

void _settleMaybePromise({
  required bool ok,
  required JSAny? value,
  required void Function(bool ok, JSAny? value) callback,
}) {
  if (!ok) {
    callback(false, value);
    return;
  }
  if (isPromise(value)) {
    then2(
      value,
      ((JSAny? result) {
        callback(true, result);
      }).toJS,
      ((JSAny? error) {
        callback(false, error);
      }).toJS,
    );
  } else {
    callback(true, value);
  }
}

Future<JSArray<JSAny?>> mapWithBreaks(
  JSArray<JSAny?> items,
  JSAny? Function(JSAny? item, int index) callback, [
  double interval = loopReleaseInterval,
]) async {
  final length = items.length;
  final results = JSArray<JSAny?>();
  var lastLoopReleaseTime = dateNow();
  for (var i = 0; i < length; i++) {
    jsPush(results, callback(jsIndex(items, i), i));
    final now = dateNow();
    if (now >= lastLoopReleaseTime + interval) {
      lastLoopReleaseTime = now;
      await _awaitJs(releaseEventLoop());
    }
  }
  return results;
}

Future<JSAny?> _awaitJs(JSAny? promise) {
  if (!isPromise(promise)) return Future.value(promise);
  return (promise as JSPromise<JSAny?>).toDart;
}

JSAny? futureToPromise(Future<JSAny?> future) => future.toJS;
