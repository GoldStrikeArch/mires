// The Web APIs Workbox runs on, as dart:js_interop extension types: CacheStorage and Cache, fetch,
// Request, Response and Headers, URL and URLSearchParams, RegExp, and the JS globals the adapter
// reads. A promise's rejection keeps the JS value it rejected with through `await` and `catch`.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Headers._(JSObject _) implements JSObject {
  external Headers(Headers init);
}

extension type Request._(JSObject _) implements JSObject {
  external Request(JSAny input, [JSObject init]);
  external String get url;
  external String get method;
  external String get mode;
  external String get integrity;
  external Headers get headers;
  @JS('clone')
  external Request copy();
}

extension type Response._(JSObject _) implements JSObject {
  external Response(JSAny? body, JSObject init);
  external int get status;
  external String get statusText;
  external String get url;
  external bool get redirected;
  @JS('type')
  external String get responseType;
  external Headers get headers;
  @JS('clone')
  external Response copy();
  external JSPromise<JSString> text();
  external JSPromise<JSAny> blob();
}

extension type URLSearchParams._(JSObject _) implements JSObject {
  external JSObject keys();
  external void set(String name, JSAny? value);
  external void delete(String name);
}

extension type URL._(JSObject _) implements JSObject {
  /// `URL.parse(url, base)`: the URL, or null where `new URL` throws its TypeError.
  @JS('parse')
  external static URL? _parse(String url, [String base]);
  external String get href;
  external String get origin;
  external String get protocol;
  external String get pathname;
  external set pathname(String value);
  external set hash(String value);
  external URLSearchParams get searchParams;

  /// `new URL(url, base)`. dart2js turns a JS TypeError caught by `catch` into a Dart error that
  /// has lost its name, so the URL is parsed with `URL.parse` and the TypeError `new URL` would
  /// throw is thrown here as the host's own TypeError.
  static URL from(String url, [String? base]) =>
      (base == null ? _parse(url) : _parse(url, base)) ?? (throw HostTypeError("Failed to construct 'URL': Invalid URL"));
}

/// The JS `RegExp`: JavaScript's matching semantics, and its SyntaxError for a bad source.
@JS('RegExp')
extension type HostRegExp._(JSObject _) implements JSObject {
  external HostRegExp(String source);
  external bool test(String input);
}

extension type Cache._(JSObject _) implements JSObject {
  @JS('match')
  external JSPromise<Response?> matching(JSAny request);
  external JSPromise<JSAny?> put(Request request, Response response);
  external JSPromise<JSBoolean> delete(Request request);
  external JSPromise<JSArray<Request>> keys([Request request, JSObject options]);
}

extension type CacheStorage._(JSObject _) implements JSObject {
  external JSPromise<Cache> open(String cacheName);
  @JS('match')
  external JSPromise<Response?> matching(Request request, JSObject options);
  external JSPromise<JSArray<JSString>> keys();
  external JSPromise<JSBoolean> delete(String cacheName);
}

extension type Location._(JSObject _) implements JSObject {
  external String get href;
  external String get origin;
}

extension type Crypto._(JSObject _) implements JSObject {
  external String randomUUID();
}

@JS('caches')
external CacheStorage get caches;

@JS('location')
external Location get location;

@JS('crypto')
external Crypto get crypto;

@JS('fetch')
external JSPromise<Response> hostFetch(Request request);

@JS('console.warn')
external void consoleWarn(String message);

@JS('JSON.stringify')
external String jsonStringify(JSAny? value);

@JS('Array.from')
external JSArray<JSString> arrayFrom(JSObject iterable);

@JS('Object.defineProperty')
external void defineProperty(JSObject target, String key, JSObject descriptor);

@JS('String')
external String jsString(JSAny? value);

/// The JS `Error` and `TypeError` constructors.
@JS('Error')
extension type HostError._(JSObject _) implements JSObject {
  external HostError(String message);
}

@JS('TypeError')
extension type HostTypeError._(JSObject _) implements JSObject {
  external HostTypeError(String message);
}

@JS('Promise')
extension type _HostPromise._(JSPromise<JSAny?> _) implements JSPromise<JSAny?> {
  external _HostPromise(JSFunction executor);
}

/// Runs [operation], a host call that can throw, and completes with its result or with the JS
/// value it threw. dart2js turns a JS TypeError caught by `catch` into a Dart error that has lost
/// its name, so the operation runs inside a JS Promise executor, whose rejection keeps the thrown
/// value as it is.
Future<T> hostCall<T extends JSAny?>(T Function() operation) async =>
    await _HostPromise(((JSFunction resolve, JSFunction reject) {
      resolve.callAsFunction(null, operation());
    }).toJS)
        .toDart as T;

/// `error instanceof Error`, the guard Workbox applies before it keeps or reports a caught value.
/// A thrown JS value is an Error when it is an instance of the page's Error; a JS primitive is not;
/// the port's own errors (WorkboxError) are.
bool isError(Object error) => switch (error) {
      final JSObject value => value.instanceOfString('Error'),
      String() || num() || bool() => false,
      _ => true,
    };

/// The JS `name` of a thrown JS Error.
String hostErrorName(JSObject error) => (error['name'] as JSString).toDart;

/// The JS `message` of a thrown JS Error.
JSAny? hostErrorMessage(JSObject error) => error['message'];

/// JavaScript truthiness of a JSON value (null covers undefined as well).
bool isTruthy(JSAny? value) {
  if (value == null) return false;
  if (value.isA<JSString>()) return (value as JSString).toDart.isNotEmpty;
  if (value.isA<JSNumber>()) {
    final number = (value as JSNumber).toDartDouble;
    return number != 0 && !number.isNaN;
  }
  if (value.isA<JSBoolean>()) return (value as JSBoolean).toDart;
  return true;
}
