// workbox-core: WorkboxError with its production messages, copyResponse, cacheMatchIgnoreParams,
// timeout, waitUntil, and the ExtendableEvent a strategy, install or activate runs under.

part of 'workbox.dart';

/// `new WorkboxError(code, details)` from the production build: the message is
/// `code :: JSON.stringify([details])`, so absent details print `[null]`. [name] is the code.
class WorkboxError implements Exception {
  WorkboxError(this.name, [this.details]) : message = '$name :: ${jsonStringify([details].toJS)}';

  final String name;
  final JSObject? details;
  final String message;

  @override
  String toString() => message;
}

/// What `JSON.stringify` prints for an error nested in a WorkboxError's details: a WorkboxError's
/// own enumerable properties (`name`, `details`), and nothing for a native Error.
JSObject asDetail(Object error) {
  final detail = JSObject();
  if (error is WorkboxError) {
    detail['name'] = error.name.toJS;
    if (error.details != null) detail['details'] = error.details;
  }
  return detail;
}

/// The event a strategy, `install` or `activate` runs under: a native ExtendableEvent, or the
/// stand-in a page supplies.
abstract interface class ExtendableEvent {
  String get type;
  void waitUntil(Future<Object?> promise);
}

/// `waitUntil(event, asyncFn)`: the started call extends the event's lifetime and is returned.
Future<T> waitUntil<T>(ExtendableEvent event, Future<T> Function() asyncFn) {
  final returnPromise = asyncFn();
  event.waitUntil(returnPromise);
  return returnPromise;
}

/// `timeout(ms)`: one timer task.
Future<void> timeout(int ms) => Future<void>.delayed(Duration(milliseconds: ms));

/// `copyResponse(response)`: refuses a response from another origin (or without a URL), then
/// builds a new Response from a clone's headers, status, status text and body. Workbox's
/// `canConstructResponseFromBodyStream` always answers false, so the body is read as a Blob.
Future<Response> copyResponse(Response response) async {
  final origin = response.url.isNotEmpty ? URL.from(response.url).origin : null;
  if (origin != location.origin) {
    throw WorkboxError('cross-origin-copy-response', JSObject()..['origin'] = origin?.toJS);
  }
  final clonedResponse = response.copy();
  final headers = await hostCall(() => Headers(clonedResponse.headers));
  final responseInit = JSObject()
    ..['headers'] = headers
    ..['status'] = clonedResponse.status.toJS
    ..['statusText'] = clonedResponse.statusText.toJS;
  final body = await clonedResponse.blob().toDart;
  return hostCall(() => Response(body, responseInit));
}

String _stripParams(String fullURL, List<String> ignoreParams) {
  final strippedURL = URL.from(fullURL);
  for (final param in ignoreParams) {
    strippedURL.searchParams.delete(param);
  }
  return strippedURL.href;
}

/// `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when the request has
/// none of the parameters, else the first key that is the request once both are stripped.
Future<Response?> cacheMatchIgnoreParams(Cache cache, Request request, List<String> ignoreParams) async {
  final strippedRequestURL = _stripParams(request.url, ignoreParams);
  if (request.url == strippedRequestURL) return cache.matching(request).toDart;
  final keysOptions = JSObject()..['ignoreSearch'] = true.toJS;
  final cacheKeys = await cache.keys(request, keysOptions).toDart;
  for (final cacheKey in cacheKeys.toDart) {
    if (strippedRequestURL == _stripParams(cacheKey.url, ignoreParams)) return cache.matching(cacheKey).toDart;
  }
  return null;
}
