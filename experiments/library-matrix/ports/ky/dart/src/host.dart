// The host APIs Ky drives: Fetch (Headers, Request, Response), AbortController and AbortSignal,
// TextDecoder and the JS globals Ky calls, as dart:js_interop extension types. Ky is a thin layer
// over these, so their semantics (method and URL normalisation, header validation, body encoding,
// encoding labels) are the engine's own.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Headers._(JSObject _) implements JSObject {
  external Headers();

  /// Validates the name and the value (a TypeError) and stores the value's ByteString
  /// conversion, so any JSON value is accepted as JS accepts it.
  external void set(String name, JSAny? value);
  external String? get(String name);
}

extension type Request._(JSObject _) implements JSObject {
  external Request(JSAny input, JSObject init);
  external String get method;
  external String get url;
  external Headers get headers;
  external AbortSignal get signal;
  @JS('clone')
  external Request copy();
  external JSPromise<JSString> text();
}

extension type Response._(JSObject _) implements JSObject {
  external Response(JSAny? body, JSObject init);
  external int get status;
  external bool get ok;
  external Headers get headers;
  external JSAny? get body;
  external JSPromise<JSString> text();
  external JSPromise<JSArrayBuffer> arrayBuffer();
}

extension type AbortController._(JSObject _) implements JSObject {
  external AbortController();
  external AbortSignal get signal;

  /// Without a reason, the platform's AbortError DOMException.
  external void abort([JSAny? reason]);
}

extension type AbortSignal._(JSObject _) implements JSObject {
  /// Aborted as soon as one of [signals] is, with that one's reason.
  external static AbortSignal any(JSArray<AbortSignal> signals);
  external bool get aborted;
  external JSAny? get reason;
  external void throwIfAborted();
  external void addEventListener(String type, JSFunction listener, JSObject options);
}

extension type TextDecoder._(JSObject _) implements JSObject {
  external TextDecoder([String label]);
  external String decode(JSArrayBuffer input);
}

/// The JS `Error`, `TypeError` and `RangeError` constructors, which Ky throws from its checks.
@JS('Error')
extension type HostError._(JSObject _) implements JSObject {
  external HostError(String message);
  external set name(String name);
}

@JS('TypeError')
extension type HostTypeError._(JSObject _) implements JSObject {
  external HostTypeError(String message);
}

@JS('RangeError')
extension type HostRangeError._(JSObject _) implements JSObject {
  external HostRangeError(String message);
}

@JS('Promise')
extension type _HostPromise._(JSPromise<JSAny?> _) implements JSPromise<JSAny?> {
  external _HostPromise(JSFunction executor);
}

/// Runs [operation], a host call that can throw, and completes with its result or with the JS
/// value it threw. dart2js turns a JS TypeError or RangeError caught by `catch` into a Dart
/// error that has lost its name, so the operation runs inside a JS Promise executor, whose
/// rejection keeps the thrown value as it is.
Future<T> hostCall<T extends JSAny?>(T Function() operation) async =>
    await _HostPromise(((JSFunction resolve, JSFunction reject) {
      resolve.callAsFunction(null, operation());
    }).toJS)
        .toDart as T;

@JS('Number.isInteger')
external bool isInteger(JSAny? value);

@JS('Object.keys')
external JSArray<JSString> objectKeys(JSObject value);

@JS('Reflect.has')
external bool reflectHas(JSObject target, String key);

@JS('Object.prototype.toString.call')
external String _toStringTag(JSAny? value);

/// Ky's `isError`: an Error of this realm, or one of another realm by its internal brand.
bool isHostError(JSObject value) => value.instanceOfString('Error') || _toStringTag(value) == '[object Error]';

@JS('JSON.parse')
external JSAny? jsonParse(String text);

@JS('JSON.stringify')
external String jsonStringify(JSAny? value);

/// The `name` of a JS value that was thrown, as `error.name` reads it.
String hostErrorName(JSObject error) => (error['name'] as JSString?)?.toDart ?? 'Error';
