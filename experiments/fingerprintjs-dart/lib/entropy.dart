library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'data.dart';
import 'fp_async.dart';
import 'host.dart';

JSAny? loadSource(JSAny? source, JSAny? sourceOptions) {
  final makeValue = ((JSAny? value, JSAny? duration) {
    return component(true, value, jsNumber(duration));
  }).toJS;
  final makeError = ((JSAny? value, JSAny? duration) {
    return component(false, value, jsNumber(duration));
  }).toJS;
  return hostHelpers.callMethod(
    'loadSource'.toJS,
    source,
    sourceOptions,
    makeValue,
    makeError,
  );
}

JSAny? component(bool ok, JSAny? value, double duration) {
  final result = jsObject();
  setp(result, ok ? 'value' : 'error', value);
  setp(result, 'duration', duration.toJS);
  return result;
}

JSAny? loadSources(
  JSAny? sources,
  JSAny? sourceOptions,
  JSAny? excludeSources, [
  JSAny? loopRelease,
]) {
  final interval = loopRelease.isUndefinedOrNull
      ? loopReleaseInterval
      : jsNumber(loopRelease);
  final keys = objectKeys(sources);
  final included = jsArray();
  for (var i = 0; i < keys.length; i++) {
    final key = jsIndex(keys, i);
    if (excludes(excludeSources, key)) {
      jsPush(included, key);
    }
  }

  final sourceGettersPromise = suppress(futureToPromise(() async {
    return await mapWithBreaks(included, (sourceKey, _) {
      return loadSource(getp(sources, jsString(sourceKey)), sourceOptions);
    }, interval);
  }()));

  return dartFun0('', () {
    return futureToPromise(() async {
      final sourceGetters = await _awaitJs(sourceGettersPromise);
      final componentPromises = await mapWithBreaks(
        sourceGetters as JSArray<JSAny?>,
        (sourceGetter, _) {
          return suppress(jsCall(sourceGetter, jsUndefined));
        },
        interval,
      );
      return await _awaitJs(then1(
        promiseAll(componentPromises),
        ((JSAny? values) {
          final result = jsObject();
          for (var i = 0; i < jsLength(included); i++) {
            setp(result, jsString(jsIndex(included, i)), jsIndex(values, i));
          }
          return result;
        }).toJS,
      ));
    }());
  });
}

JSAny? transformSource(JSAny? source, JSAny? transformValue) {
  return hostHelpers.callMethod('transformSource'.toJS, source, transformValue);
}

JSAny? loadSourcesExport(
  JSAny? sources,
  JSAny? sourceOptions,
  JSAny? excludeSources,
  JSAny? loopRelease,
) {
  return loadSources(
    sources.isUndefinedOrNull ? jsObject() : sources,
    sourceOptions.isUndefinedOrNull ? jsObject() : sourceOptions,
    excludeSources.isUndefinedOrNull ? jsArray() : excludeSources,
    loopRelease,
  );
}

Future<JSAny?> _awaitJs(JSAny? promise) async {
  if (!isPromise(promise)) return promise;
  return (promise as JSPromise<JSAny?>).toDart;
}
