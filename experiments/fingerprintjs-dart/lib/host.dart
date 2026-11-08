/// JS interop used by the FingerprintJS port.
///
/// Nothing here reads `window` / `document` / `navigator` / `screen` at
/// library load. Browser globals are only touched from exported functions.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:web/web.dart' as web;

const algorithmVersion = '5.2.0';

final JSObject _global = globalContext;

JSAny? get jsUndefined => _helpers.callMethod('undef'.toJS);
JSAny? get jsNull => _helpers.callMethod('nul'.toJS);

JSObject? _helpersCache;

JSObject get hostHelpers => _helpers;

JSObject get _helpers {
  final cached = _helpersCache;
  if (cached != null) return cached;
  final ctor = _global.getProperty('Function'.toJS) as JSFunction;
  final factory = ctor.callAsFunction(
    null,
    r'''
return {
  undef: function() { return undefined; },
  nul: function() { return null; },
  typof: function(v) { return typeof v; },
  hasIn: function(o, k) {
    try { return k in Object(o); } catch (e) { return false; }
  },
  copyArgs: function(args) {
    var out = [];
    if (!args) return out;
    var n = args.length || 0;
    for (var i = 0; i < n; i++) out.push(args[i]);
    return out;
  },
  tryCall: function(fn, th, args) {
    var real = [];
    if (args) {
      for (var i = 0; i < (args.length || 0); i++) real.push(args[i]);
    }
    try { return [true, fn.apply(th, real)]; }
    catch (e) { return [false, e]; }
  },
  tryCallNull: function(fn, args) {
    var real = [];
    if (args) {
      for (var i = 0; i < (args.length || 0); i++) real.push(args[i]);
    }
    try { return [true, fn.apply(null, real)]; }
    catch (e) { return [false, e]; }
  },
  tryCallUndef: function(fn, args) {
    var real = [];
    if (args) {
      for (var i = 0; i < (args.length || 0); i++) real.push(args[i]);
    }
    try { return [true, fn.apply(undefined, real)]; }
    catch (e) { return [false, e]; }
  },
  call2: function(fn, a, b) {
    return fn(a, b);
  },
  resolve1: function(resolve, value) {
    resolve(value);
  },
  makeArray: function() {
    return Array.prototype.slice.call(arguments);
  },
  hashSignal: function(signal) {
    var hash = 0;
    if (!signal) return 0;
    for (var i = 0; i < signal.length; ++i) hash += Math.abs(signal[i]);
    return hash;
  },
  call1: function(fn, th, a) {
    return fn.call(th, a);
  },
  isPromise: function(v) { return !!v && typeof v.then === "function"; },
  named: function(name, arity, impl) {
    var args = [];
    for (var i = 0; i < arity; i++) args.push("a" + i);
    var src = "return function " + name + "(" + args.join(",") + "){return impl(arguments[0], arguments[1], arguments[2], arguments[3]);}";
    return Function("impl", src)(impl);
  },
  defineName: function(fn, name) {
    Object.defineProperty(fn, "name", { value: name, configurable: true });
    return fn;
  },
  defineGetter: function(obj, key, get, set) {
    Object.defineProperty(obj, key, {
      enumerable: true,
      configurable: true,
      get: get,
      set: set
    });
  },
  jsonStringify: function(v, r, s) { return JSON.stringify(v, r, s); },
  objectKeys: function(o) { return Object.keys(o); },
  now: function() { return Date.now(); },
  parseInt: function(v) { return parseInt(v); },
  parseFloat: function(v) { return parseFloat(v); },
  isNaNNum: function(v) { return typeof v === "number" && isNaN(v); },
  isFiniteNum: function(v) { return typeof v === "number" && isFinite(v); },
  arrayIsArray: function(v) { return Array.isArray(v); },
  instanceOf: function(v, c) {
    try { return typeof c === "function" && v instanceof c; }
    catch (e) { return false; }
  },
  isError: function(v) { return v instanceof Error; },
  new0: function(C) { return new C(); },
  new1: function(C, a) { return new C(a); },
  new2: function(C, a, b) { return new C(a, b); },
  new3: function(C, a, b, c) { return new C(a, b, c); },
  atob: function(s) { return atob(s); },
  release: function() {
    return new Promise(function(resolve) {
      var ch = new MessageChannel();
      ch.port1.onmessage = function() { resolve(); };
      ch.port2.postMessage(null);
    });
  },
  wait: function(ms, value) {
    return new Promise(function(resolve) { setTimeout(resolve, ms, value); });
  },
  promiseAll: function(arr) { return Promise.all(arr); },
  promiseResolve: function(v) { return Promise.resolve(v); },
  newPromise: function(ex) { return new Promise(ex); },
  suppress: function(p) { p.then(undefined, function() {}); return p; },
  str: function(v) { return String(v); },
  fromCharCode: function(a, b) { return String.fromCharCode(a, b); },
  getProto: function(o) { return o == null ? undefined : o.__proto__; },
  regex: function(src, flags) { return new RegExp(src, flags); },
  test: function(re, s) { return re.test(s); },
  replace: function(s, re, rep) { return String(s).replace(re, rep); },
  split: function(s, sep) { return String(s).split(sep); },
  indexOf: function(s, n) { return String(s).indexOf(n); },
  mathCall: function(name, a) {
    var fn = Math[name];
    return typeof fn === "function" ? fn(a) : 0;
  },
  mathPow: function(a, b) { return Math.pow(a, b); },
  mathPI: function() { return Math.PI; },
  mathAbs: function(a) { return Math.abs(a); },
  mathMax: function(a, b) { return Math.max(a, b); },
  mathMin: function(a, b) { return Math.min(a, b); },
  mathRound: function(a) { return Math.round(a); },
  mathFloor: function(a) { return Math.floor(a); },
  mathRandom: function() { return Math.random(); },
  monitor: function() {
    if (window.__fpjs_d_m || Math.random() >= 0.001) return;
    try {
      var request = new XMLHttpRequest();
      request.open("get", "https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring", true);
      request.send();
    } catch (error) {
      console.error(error);
    }
  },
  reject: function(error) { return Promise.reject(error); },
  withIframeBridge: function(setupPromise, action) {
    var iframe = null;
    return Promise.resolve(setupPromise).then(function(pair) {
      iframe = pair.iframe;
      try {
        var value = action(iframe, pair.window);
        if (value && typeof value.then === "function") return value;
        return value;
      } catch (error) {
        return Promise.reject(error);
      }
    }).finally(function() {
      if (iframe && iframe.parentNode) iframe.parentNode.removeChild(iframe);
    });
  },
  dateYear: function() { return new Date().getFullYear(); },
  dateMonthOffset: function(year, month) {
    return new Date(year, month, 1).getTimezoneOffset();
  },
  setTimeout: function(fn, ms) { return setTimeout(fn, ms); },
  clearTimeout: function(id) { clearTimeout(id); },
  idle: function(timeout) {
    var w = typeof globalThis !== "undefined" ? globalThis : undefined;
    var ric = w && w.requestIdleCallback;
    if (ric) {
      return new Promise(function(resolve) {
        ric.call(w, function() { resolve(); }, { timeout: timeout });
      });
    }
    return null;
  },
  errorCtor: function() { return Error; },
  typeErrorCtor: function() { return TypeError; },
  makeError: function(name, message) {
    var e = new Error(message);
    e.name = name;
    return e;
  },
  awaitIfAsyncCall: function(fn, thisMode, args, callback) {
    try {
      var result = thisMode === "null" ? fn.apply(null, args) : fn.apply(undefined, args);
      if (result && typeof result.then === "function") {
        result.then(
          function(value) { callback(true, value); },
          function(error) { callback(false, error); }
        );
      } else {
        callback(true, result);
      }
    } catch (error) {
      callback(false, error);
    }
  },
  loadSource: function(source, sourceOptions, makeValue, makeError, makeStage) {
    var loadStart = Date.now();
    var sourceLoadPromise = new Promise(function(resolveLoad) {
      try {
        var loadResult = source.call(null, sourceOptions);
        var finish = function(ok, value) {
          var loadDuration = Date.now() - loadStart;
          if (!ok) {
            resolveLoad(function() { return Promise.resolve(makeError(value, loadDuration)); });
          } else if (typeof value === "function") {
            resolveLoad(function() {
              return new Promise(function(resolveGet) {
                var getStart = Date.now();
                try {
                  var getResult = value();
                  var finishGet = function(gok, gvalue) {
                    var duration = loadDuration + Date.now() - getStart;
                    resolveGet(gok ? makeValue(gvalue, duration) : makeError(gvalue, duration));
                  };
                  if (getResult && typeof getResult.then === "function") {
                    getResult.then(
                      function(v) { finishGet(true, v); },
                      function(e) { finishGet(false, e); }
                    );
                  } else {
                    finishGet(true, getResult);
                  }
                } catch (error) {
                  resolveGet(makeError(error, loadDuration + Date.now() - getStart));
                }
              });
            });
          } else {
            resolveLoad(function() { return Promise.resolve(makeValue(value, loadDuration)); });
          }
        };
        if (loadResult && typeof loadResult.then === "function") {
          loadResult.then(
            function(value) { finish(true, value); },
            function(error) { finish(false, error); }
          );
        } else {
          finish(true, loadResult);
        }
      } catch (error) {
        var loadDuration = Date.now() - loadStart;
        resolveLoad(function() { return Promise.resolve(makeError(error, loadDuration)); });
      }
    });
    sourceLoadPromise.then(undefined, function() {});
    return function() {
      return sourceLoadPromise.then(function(finalize) { return finalize(); });
    };
  },
  transformSource: function(source, transformValue) {
    var transformLoadResult = function(loadResult) {
      if (typeof loadResult !== "function") {
        return transformValue(loadResult);
      }
      return function() {
        var getResult = loadResult();
        if (getResult && typeof getResult.then === "function") {
          return getResult.then(transformValue);
        }
        return transformValue(getResult);
      };
    };
    return function(options) {
      var loadResult = source(options);
      if (loadResult && typeof loadResult.then === "function") {
        return loadResult.then(transformLoadResult);
      }
      return transformLoadResult(loadResult);
    };
  },
  makeLazyResult: function(components, hashFn, confidence, version) {
    var visitorIdCache;
    return {
      get visitorId() {
        if (visitorIdCache === undefined) {
          visitorIdCache = hashFn(this.components, undefined, undefined, undefined);
        }
        return visitorIdCache;
      },
      set visitorId(v) {
        visitorIdCache = v;
      },
      confidence: confidence,
      components: components,
      version: version
    };
  }
};
'''
        .toJS,
  ) as JSFunction;
  final created = factory.callAsFunction() as JSObject;
  _helpersCache = created;
  return created;
}

// package:web is imported so the emit is a real Dart 3 + package:web port.
// The top-level `window` getter is never read at import time.
void _touchWebPackage() {
  // Referencing the library prefix keeps the import live without calling
  // `web.window` / `web.document` during library initialization.
  final _ = web.console;
}

JSAny? g(String name) => _global.getProperty(name.toJS);

JSAny? get jsWindow => g('window');
JSAny? get jsDocument => g('document');
JSAny? get jsNavigator => g('navigator');
JSAny? get jsScreen => g('screen');

JSAny? getp(JSAny? obj, String key) {
  if (obj.isUndefinedOrNull) return jsUndefined;
  return (obj as JSObject).getProperty(key.toJS);
}

void setp(JSAny? obj, String key, JSAny? value) {
  (obj as JSObject).setProperty(key.toJS, value);
}

bool hasOwn(JSAny? obj, String key) {
  if (obj.isUndefinedOrNull) return false;
  return (obj as JSObject).has(key);
}

bool hasIn(JSAny? obj, String key) {
  if (obj.isUndefinedOrNull) return false;
  return (_helpers.callMethod('hasIn'.toJS, obj, key.toJS) as JSBoolean).toDart;
}

String typof(JSAny? value) =>
    (_helpers.callMethod('typof'.toJS, value) as JSString).toDart;

bool isUndef(JSAny? value) => value.isUndefined;

bool isNull(JSAny? value) => value.isNull;

bool isJsNull(JSAny? value) => value.isNull;

bool isUndefOrNull(JSAny? value) => value.isUndefinedOrNull;

bool isFunction(JSAny? value) => typof(value) == 'function';

bool isObject(JSAny? value) =>
    typof(value) == 'object' && value.isDefinedAndNotNull;

bool isPromise(JSAny? value) =>
    (_helpers.callMethod('isPromise'.toJS, value) as JSBoolean).toDart;

bool isArray(JSAny? value) =>
    (_helpers.callMethod('arrayIsArray'.toJS, value) as JSBoolean).toDart;

bool instanceOf(JSAny? value, JSAny? ctor) =>
    (_helpers.callMethod('instanceOf'.toJS, value, ctor) as JSBoolean).toDart;

bool isError(JSAny? value) =>
    (_helpers.callMethod('isError'.toJS, value) as JSBoolean).toDart;

JSObject jsObject() => JSObject();

JSArray<JSAny?> jsArray([Iterable<JSAny?> items = const []]) {
  final list = items.toList();
  switch (list.length) {
    case 0:
      return _helpers.callMethod('makeArray'.toJS) as JSArray<JSAny?>;
    case 1:
      return _helpers.callMethod('makeArray'.toJS, list[0]) as JSArray<JSAny?>;
    case 2:
      return _helpers.callMethod('makeArray'.toJS, list[0], list[1])
          as JSArray<JSAny?>;
    case 3:
      return _helpers.callMethod('makeArray'.toJS, list[0], list[1], list[2])
          as JSArray<JSAny?>;
    default:
      var array = _helpers.callMethod('makeArray'.toJS) as JSArray<JSAny?>;
      for (final item in list) {
        jsPush(array, item);
      }
      return array;
  }
}

JSAny? call2(JSAny? fn, JSAny? a, JSAny? b) =>
    _helpers.callMethod('call2'.toJS, fn, a, b);

void resolve1(JSAny? resolve, JSAny? value) {
  _helpers.callMethod('resolve1'.toJS, resolve, value);
}

int jsLength(JSAny? value) {
  final len = getp(value, 'length');
  if (len == null || isUndef(len)) return 0;
  return (len as JSNumber).toDartInt;
}

JSAny? jsIndex(JSAny? value, int index) {
  if (value.isUndefinedOrNull) return jsUndefined;
  return (value as JSObject).getProperty(index.toJS);
}

void jsSetIndex(JSAny? value, int index, JSAny? item) {
  (value as JSObject).setProperty(index.toJS, item);
}

void jsPush(JSAny? array, JSAny? item) {
  (array as JSObject).callMethod('push'.toJS, item);
}

JSAny? jsCall(JSAny? fn, JSAny? thisArg, [List<JSAny?> args = const []]) {
  final applied = jsArray(args);
  final result = _helpers.callMethod('tryCall'.toJS, fn, thisArg, applied);
  return jsIndex(result, 1);
}

(bool, JSAny?) tryCall(JSAny? fn, JSAny? thisArg, [List<JSAny?> args = const []]) {
  final applied = jsArray(args);
  final result = thisArg.isNull
      ? _helpers.callMethod('tryCallNull'.toJS, fn, applied)
      : thisArg.isUndefined
          ? _helpers.callMethod('tryCallUndef'.toJS, fn, applied)
          : _helpers.callMethod('tryCall'.toJS, fn, thisArg, applied);
  final ok = jsIndex(result, 0);
  return ((ok as JSBoolean).toDart, jsIndex(result, 1));
}

(bool, JSAny?) tryHost(JSAny? Function() action) {
  try {
    return (true, action());
  } catch (error) {
    return (false, _asJsError(error));
  }
}

JSAny? _asJsError(Object error) {
  if (error is JSObject) return error;
  return makeError('Error', error.toString());
}

JSFunction namedFun(String name, int arity, JSFunction impl) {
  return _helpers.callMethod('named'.toJS, name.toJS, arity.toJS, impl)
      as JSFunction;
}

JSFunction dartFun0(String name, JSAny? Function() impl, {int arity = 0}) {
  return namedFun(
    name,
    arity,
    ((JSAny? a, JSAny? b, JSAny? c, JSAny? d) {
      return impl();
    }).toJS,
  );
}

JSFunction dartFun1(String name, JSAny? Function(JSAny?) impl, {int arity = 1}) {
  return namedFun(
    name,
    arity,
    ((JSAny? a, JSAny? b, JSAny? c, JSAny? d) {
      return impl(a);
    }).toJS,
  );
}

JSFunction dartFun2(
  String name,
  JSAny? Function(JSAny?, JSAny?) impl, {
  int arity = 2,
}) {
  return namedFun(
    name,
    arity,
    ((JSAny? a, JSAny? b, JSAny? c, JSAny? d) {
      return impl(a, b);
    }).toJS,
  );
}

JSFunction dartFun4(
  String name,
  JSAny? Function(JSAny?, JSAny?, JSAny?, JSAny?) impl, {
  int arity = 3,
}) {
  return namedFun(
    name,
    arity,
    ((JSAny? a, JSAny? b, JSAny? c, JSAny? d) {
      return impl(a, b, c, d);
    }).toJS,
  );
}

JSAny? jsonStringify(JSAny? value, [JSAny? replacer, JSAny? space]) {
  return _helpers.callMethod('jsonStringify'.toJS, value, replacer, space);
}

JSArray<JSAny?> objectKeys(JSAny? value) {
  return _helpers.callMethod('objectKeys'.toJS, value) as JSArray<JSAny?>;
}

double dateNow() => (_helpers.callMethod('now'.toJS) as JSNumber).toDartDouble;

double jsParseInt(JSAny? value) =>
    (_helpers.callMethod('parseInt'.toJS, value) as JSNumber).toDartDouble;

double jsParseFloat(JSAny? value) =>
    (_helpers.callMethod('parseFloat'.toJS, value) as JSNumber).toDartDouble;

bool jsIsNaN(JSAny? value) =>
    (_helpers.callMethod('isNaNNum'.toJS, value) as JSBoolean).toDart;

bool jsIsFinite(JSAny? value) =>
    (_helpers.callMethod('isFiniteNum'.toJS, value) as JSBoolean).toDart;

JSAny? jsNew0(JSAny? ctor) => _helpers.callMethod('new0'.toJS, ctor);

JSAny? jsNew1(JSAny? ctor, JSAny? a) =>
    _helpers.callMethod('new1'.toJS, ctor, a);

JSAny? jsNew2(JSAny? ctor, JSAny? a, JSAny? b) =>
    _helpers.callMethod('new2'.toJS, ctor, a, b);

JSAny? jsNew3(JSAny? ctor, JSAny? a, JSAny? b, JSAny? c) =>
    _helpers.callMethod('new3'.toJS, ctor, a, b, c);

String jsAtob(String encoded) =>
    (_helpers.callMethod('atob'.toJS, encoded.toJS) as JSString).toDart;

JSAny? jsWait(num ms, [JSAny? value]) =>
    _helpers.callMethod('wait'.toJS, ms.toJS, value);

JSAny? jsRelease() => _helpers.callMethod('release'.toJS);

JSAny? promiseAll(JSAny? array) =>
    _helpers.callMethod('promiseAll'.toJS, array);

JSAny? promiseResolve(JSAny? value) =>
    _helpers.callMethod('promiseResolve'.toJS, value);

JSAny? newPromise(JSFunction executor) =>
    _helpers.callMethod('newPromise'.toJS, executor);

JSAny? suppress(JSAny? promise) =>
    _helpers.callMethod('suppress'.toJS, promise);

JSAny? then1(JSAny? promise, JSFunction onFulfilled) {
  return (promise as JSObject).callMethod('then'.toJS, onFulfilled);
}

JSAny? then2(JSAny? promise, JSFunction onFulfilled, JSFunction onRejected) {
  return (promise as JSObject).callMethod('then'.toJS, onFulfilled, onRejected);
}

String jsString(JSAny? value) {
  if (value == null) return 'null';
  if (value is JSString) return value.toDart;
  return (_helpers.callMethod('str'.toJS, value) as JSString).toDart;
}

double jsNumber(JSAny? value) {
  if (value is JSNumber) return value.toDartDouble;
  return jsParseFloat(value);
}

bool jsTruthy(JSAny? value) {
  if (value.isUndefinedOrNull) return false;
  if (value is JSBoolean) return value.toDart;
  if (value is JSNumber) {
    final n = value.toDartDouble;
    return n != 0 && n == n;
  }
  if (value is JSString) return value.toDart.isNotEmpty;
  return true;
}

JSAny? fromDart(Object? value) {
  if (value == null) return jsUndefined;
  if (value is JSAny) return value;
  if (value is String) return value.toJS;
  if (value is bool) return value.toJS;
  if (value is int) return value.toJS;
  if (value is double) return value.toJS;
  if (value is List) {
    return jsArray(value.map(fromDart));
  }
  throw ArgumentError('cannot convert $value');
}

JSAny? jsBool(bool value) => value.toJS;
JSAny? jsNum(num value) => value.toJS;
JSAny? jsStr(String value) => value.toJS;

JSAny? getProto(JSAny? value) => _helpers.callMethod('getProto'.toJS, value);

JSAny? jsRegex(String source, [String flags = '']) =>
    _helpers.callMethod('regex'.toJS, source.toJS, flags.toJS);

bool jsTest(JSAny? regex, String input) =>
    (_helpers.callMethod('test'.toJS, regex, input.toJS) as JSBoolean).toDart;

String jsReplace(JSAny? input, JSAny? regex, String replacement) =>
    (_helpers.callMethod('replace'.toJS, input, regex, replacement.toJS)
            as JSString)
        .toDart;

JSAny? jsSplit(JSAny? input, String separator) =>
    _helpers.callMethod('split'.toJS, input, separator.toJS);

int jsIndexOf(JSAny? input, String needle) =>
    (_helpers.callMethod('indexOf'.toJS, input, needle.toJS) as JSNumber)
        .toDartInt;

double mathCall(String name, num arg) =>
    (_helpers.callMethod('mathCall'.toJS, name.toJS, arg.toJS) as JSNumber)
        .toDartDouble;

double mathPow(num a, num b) =>
    (_helpers.callMethod('mathPow'.toJS, a.toJS, b.toJS) as JSNumber)
        .toDartDouble;

double mathPI() =>
    (_helpers.callMethod('mathPI'.toJS) as JSNumber).toDartDouble;

double mathAbs(num a) =>
    (_helpers.callMethod('mathAbs'.toJS, a.toJS) as JSNumber).toDartDouble;

double mathMax(num a, num b) =>
    (_helpers.callMethod('mathMax'.toJS, a.toJS, b.toJS) as JSNumber)
        .toDartDouble;

double mathMin(num a, num b) =>
    (_helpers.callMethod('mathMin'.toJS, a.toJS, b.toJS) as JSNumber)
        .toDartDouble;

double mathRound(num a) =>
    (_helpers.callMethod('mathRound'.toJS, a.toJS) as JSNumber).toDartDouble;

double mathFloor(num a) =>
    (_helpers.callMethod('mathFloor'.toJS, a.toJS) as JSNumber).toDartDouble;

JSAny? jsSetTimeout(JSFunction fn, num ms) =>
    _helpers.callMethod('setTimeout'.toJS, fn, ms.toJS);

void jsClearTimeout(JSAny? id) {
  _helpers.callMethod('clearTimeout'.toJS, id);
}

JSAny? requestIdle(num timeout) => _helpers.callMethod('idle'.toJS, timeout.toJS);

JSAny? makeError(String name, String message) =>
    _helpers.callMethod('makeError'.toJS, name.toJS, message.toJS);

void runMonitor() {
  _helpers.callMethod('monitor'.toJS);
}

JSAny? rejectedPromise(JSAny? error) =>
    _helpers.callMethod('reject'.toJS, error);

JSAny? withIframeBridge(JSAny? setupPromise, JSAny? action) =>
    _helpers.callMethod('withIframeBridge'.toJS, setupPromise, action);

JSAny? makeLazyResult(
  JSAny? components,
  JSFunction hashFn,
  JSAny? confidence,
  String version,
) {
  return _helpers.callMethod(
    'makeLazyResult'.toJS,
    components,
    hashFn,
    confidence,
    version.toJS,
  );
}

String fromCharCode2(int a, int b) =>
    (_helpers.callMethod('fromCharCode'.toJS, a.toJS, b.toJS) as JSString)
        .toDart;

int dateYear() =>
    (_helpers.callMethod('dateYear'.toJS) as JSNumber).toDartInt;

double dateMonthOffset(int year, int month) =>
    (_helpers.callMethod('dateMonthOffset'.toJS, year.toJS, month.toJS)
            as JSNumber)
        .toDartDouble;

JSAny? method(JSAny? obj, String name, [List<JSAny?> args = const []]) {
  if (obj.isUndefinedOrNull) return jsUndefined;
  final fn = getp(obj, name);
  if (!isFunction(fn)) return jsUndefined;
  return jsCall(fn, obj, args);
}

JSAny? ctor(JSAny? obj, String name) => getp(obj, name);

bool cssSupports(String property, String value) {
  final css = getp(jsWindow, 'CSS');
  final fn = getp(css, 'supports');
  if (!isFunction(fn)) return false;
  final result = jsCall(fn, css, [property.toJS, value.toJS]);
  return jsTruthy(result);
}

bool cssSupportsQuery(String query) {
  final css = getp(jsWindow, 'CSS');
  final fn = getp(css, 'supports');
  if (!isFunction(fn)) return false;
  final result = jsCall(fn, css, [query.toJS]);
  return jsTruthy(result);
}

bool matchMediaMatches(String query) {
  final fn = getp(jsWindow, 'matchMedia');
  if (!isFunction(fn)) {
    final globalFn = g('matchMedia');
    if (!isFunction(globalFn)) return false;
    final mq = jsCall(globalFn, jsWindow, [query.toJS]);
    return jsTruthy(getp(mq, 'matches'));
  }
  final mq = jsCall(fn, jsWindow, [query.toJS]);
  return jsTruthy(getp(mq, 'matches'));
}

void logDebug(String message) {
  final console = g('console');
  final log = getp(console, 'log');
  if (isFunction(log)) {
    jsCall(log, console, [message.toJS]);
  }
}

/// Marks the package:web import as used without touching browser globals.
void retainWebImport() => _touchWebPackage();
