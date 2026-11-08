library;

import 'dart:js_interop';

import 'confidence.dart';
import 'entropy.dart';
import 'fp_async.dart';
import 'hashing.dart';
import 'host.dart';
import 'misc.dart';
import 'sources.dart';

JSAny? componentsToCanonicalString(JSAny? components) {
  final keys = objectKeys(components);
  method(keys, 'sort');
  final parts = <String>[];
  for (var i = 0; i < keys.length; i++) {
    final key = jsString(jsIndex(keys, i));
    final component = getp(components, key);
    final value = hasIn(component, 'error')
        ? 'error'
        : () {
            final serialized = jsonStringify(getp(component, 'value'));
            if (serialized.isUndefined) return 'undefined';
            return jsString(serialized);
          }();
    final escaped = key.replaceAllMapped(
      RegExp(r'([:|\\])'),
      (match) => '\\${match[1]}',
    );
    parts.add('$escaped:$value');
  }
  return parts.join('|').toJS;
}

JSAny? hashComponents(JSAny? components) {
  return x64hash128(jsString(componentsToCanonicalString(components))).toJS;
}

JSAny? componentsToDebugString(JSAny? components) {
  final replacer = ((JSAny? key, JSAny? value) {
    if (isError(value)) return errorToObject(value);
    return value;
  }).toJS;
  return jsonStringify(components, replacer, 2.toJS);
}

JSAny? prepareForSources([JSAny? delayFallback]) {
  final delay = delayFallback.isUndefinedOrNull ? 50.0 : jsNumber(delayFallback);
  return requestIdleCallbackIfAvailable(delay, delay * 2);
}

void monitor() {
  runMonitor();
}

JSAny? load(JSAny? options) {
  final opts = options.isUndefinedOrNull ? jsObject() : options;
  final delayFallback = getp(opts, 'delayFallback');
  final debug = getp(opts, 'debug');
  final monitoring = getp(opts, 'monitoring');
  if (monitoring.isUndefined || jsTruthy(monitoring)) {
    monitor();
  }
  return futureToPromise(() async {
    await _awaitJs(prepareForSources(delayFallback));
    final sourceOptions = jsObject();
    setp(sourceOptions, 'cache', jsObject());
    setp(sourceOptions, 'debug', debug);
    final getComponents = loadSources(makeSources(), sourceOptions, jsArray());
    return makeAgent(getComponents, debug);
  }());
}

JSAny? makeAgent(JSAny? getComponents, JSAny? debug) {
  final creationTime = dateNow();
  final agent = jsObject();
  setp(
    agent,
    'get',
    dartFun1('get', (getOptions) {
      final startTime = dateNow();
      return then1(
        jsCall(getComponents, jsUndefined),
        ((JSAny? components) {
          final result = makeLazyGetResult(components);
          if (jsTruthy(debug) ||
              (getOptions.isDefinedAndNotNull &&
                  jsTruthy(getp(getOptions, 'debug')))) {
            _printDebug(result, startTime - creationTime);
          }
          return result;
        }).toJS,
      );
    }, arity: 0),
  );
  return agent;
}

JSAny? makeLazyGetResult(JSAny? components) {
  return makeLazyResult(
    components,
    dartFun1('', hashComponents),
    getConfidence(components),
    algorithmVersion,
  );
}

void _printDebug(JSAny? result, double timeBetween) {
  final visitorId = getp(result, 'visitorId');
  final components = getp(result, 'components');
  final ua = jsString(getp(jsNavigator, 'userAgent'));
  logDebug(
    'Copy the text below to get the debug data:\n\n'
    '```\n'
    'version: ${jsString(getp(result, 'version'))}\n'
    'userAgent: $ua\n'
    'timeBetweenLoadAndGet: $timeBetween\n'
    'visitorId: ${jsString(visitorId)}\n'
    'components: ${jsString(componentsToDebugString(components))}\n'
    '```',
  );
}

Future<JSAny?> _awaitJs(JSAny? promise) async {
  if (!isPromise(promise)) return promise;
  return (promise as JSPromise<JSAny?>).toDart;
}
