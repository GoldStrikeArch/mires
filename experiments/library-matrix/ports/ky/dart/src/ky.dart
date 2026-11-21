// Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
// utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md. Ky's async
// methods are async functions here too; what Ky throws, they throw.

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;

import 'host.dart';
import 'retry_timing.dart';

/// errors/*.js: Ky's own errors. Everything else a call can fail with (a TypeError, RangeError,
/// SyntaxError or abort reason, from a host API, the transport or Ky's option checks) is the JS
/// error that was thrown, a [JSObject].
sealed class KyError implements Exception {
  String get name;
}

final class HTTPError extends KyError {
  HTTPError(this.response, this.request, this.data);
  final Response response;
  final Request request;
  final JSAny? data;
  @override
  String get name => 'HTTPError';
}

final class NetworkError extends KyError {
  NetworkError(this.request, this.cause);
  final Request request;
  final JSObject cause;
  @override
  String get name => 'NetworkError';
}

final class TimeoutError extends KyError {
  TimeoutError(this.request);
  final Request request;
  @override
  String get name => 'TimeoutError';
}

/// `error.name`, as the hooks and the caller read it.
String errorName(Object error) => switch (error) {
  KyError(:final name) => name,
  JSObject() => hostErrorName(error),
  _ => 'Error',
};

typedef RequestHook = void Function(Request request, int retryCount);
typedef ResponseHook = void Function(Request request, Response response, int retryCount);
typedef ErrorHook = void Function(Request request, Object error, int retryCount);

class Hooks {
  const Hooks({
    required this.init,
    required this.beforeRequest,
    required this.afterResponse,
    required this.beforeRetry,
    required this.beforeError,
  });
  final List<void Function()> init;
  final List<RequestHook> beforeRequest;
  final List<ResponseHook> afterResponse;
  final List<ErrorHook> beforeRetry;
  final List<ErrorHook> beforeError;
}

/// The caller's retry options, as the JS values given (undefined where omitted); Ky's
/// normalizeRetryOptions validates them.
class RetryOptions {
  const RetryOptions({
    required this.limit,
    required this.methods,
    required this.statusCodes,
    required this.maxRetryAfter,
    required this.backoffLimit,
    required this.retryOnTimeout,
    required this.delay,
  });
  final JSAny? limit;
  final JSAny? methods;
  final JSAny? statusCodes;
  final JSAny? maxRetryAfter;
  final JSAny? backoffLimit;
  final JSAny? retryOnTimeout;
  final double Function(int attemptCount) delay;
}

class Options {
  const Options({
    required this.method,
    required this.headers,
    required this.json,
    required this.retry,
    required this.timeout,
    required this.throwHttpErrors,
    required this.hooks,
    required this.fetch,
    required this.signal,
  });
  final String? method;

  /// A plain object of header names to values.
  final JSObject? headers;
  final JSAny? json;
  final RetryOptions retry;
  final JSAny? timeout;
  final JSAny? throwHttpErrors;
  final Hooks hooks;
  final Future<Response> Function(Request request) fetch;
  final AbortSignal? signal;
}

/// The body shortcuts, and the media type each one advertises.
enum Shortcut {
  json('application/json'),
  text('text/*');

  const Shortcut(this.mimeType);
  final String mimeType;
}

class _Retry {
  const _Retry({
    required this.limit,
    required this.methods,
    required this.statusCodes,
    required this.maxRetryAfter,
    required this.backoffLimit,
    required this.delay,
    required this.retryOnTimeout,
  });
  final double limit;
  final List<String> methods;
  final List<double> statusCodes;
  final List<double> afterStatusCodes = _retryAfterStatusCodes;
  final double maxRetryAfter;
  final double backoffLimit;
  final double Function(int attemptCount) delay;
  final bool retryOnTimeout;
}

// core/constants.js and utils/normalize.js
const _requestMethods = ['get', 'post', 'put', 'patch', 'head', 'delete', 'query'];
const _caseInsensitiveMethods = [..._requestMethods, 'options', 'trace'];
const _retryMethods = ['get', 'put', 'head', 'delete', 'options', 'trace', 'query'];
const _retryStatusCodes = [408.0, 413.0, 429.0, 500.0, 502.0, 503.0, 504.0];
const _retryAfterStatusCodes = [413.0, 429.0, 503.0];

/// The maximum value of a 32bit int (see issue #117)
const _maxSafeTimeout = 2147483647.0;
const _maxErrorResponseBodySize = 10 * 1024 * 1024;

// On the web, Dart's toLowerCase and toUpperCase are JS's.
String _normalizeRequestMethod(String input) =>
    _requestMethods.contains(input.toLowerCase()) ? input.toUpperCase() : input;

/// Common methods for retry matching are case-insensitive; custom ones keep their case.
String _normalizeRetryMethod(String method) {
  final lowercase = method.toLowerCase();
  return _caseInsensitiveMethods.contains(lowercase) ? lowercase : method;
}

double? _nonNegative(JSAny? value) {
  if (!value.isA<JSNumber>()) return null;
  final number = (value as JSNumber).toDartDouble;
  return number >= 0 ? number : null;
}

/// The defaults for undefined values, and Ky's checks, in Ky's order.
_Retry _normalizeRetryOptions(RetryOptions retry) {
  final double limit;
  if (retry.limit.isUndefined) {
    limit = 2;
  } else if (isInteger(retry.limit) && (retry.limit as JSNumber).toDartDouble >= 0) {
    limit = (retry.limit as JSNumber).toDartDouble;
  } else {
    throw HostTypeError('`retry.limit` must be a finite, non-negative integer');
  }
  List<JSAny?>? array(String key, JSAny? value) {
    if (value.isUndefined) return null;
    if (value.isA<JSArray>()) return (value as JSArray<JSAny?>).toDart;
    throw HostError('retry.$key must be an array');
  }

  final methods = array('methods', retry.methods);
  final statusCodes = array('statusCodes', retry.statusCodes);
  // A mistyped entry only ever fails to match, so it is reported instead.
  if (methods != null && methods.any((method) => !method.isA<JSString>())) {
    throw HostTypeError('`retry.methods` must only contain strings');
  }
  if (statusCodes != null && statusCodes.any((status) => !isInteger(status))) {
    throw HostTypeError('`retry.statusCodes` must only contain numbers');
  }
  double limitOption(String key, JSAny? value) => value.isUndefined
      ? double.infinity
      : _nonNegative(value) ?? (throw HostTypeError('`retry.$key` must be a non-negative number or `Infinity`'));
  final maxRetryAfter = limitOption('maxRetryAfter', retry.maxRetryAfter);
  final backoffLimit = limitOption('backoffLimit', retry.backoffLimit);
  final bool retryOnTimeout;
  if (retry.retryOnTimeout.isUndefined) {
    retryOnTimeout = false;
  } else if (retry.retryOnTimeout.isA<JSBoolean>()) {
    retryOnTimeout = (retry.retryOnTimeout as JSBoolean).toDart;
  } else {
    throw HostTypeError('`retry.retryOnTimeout` must be a boolean');
  }
  return _Retry(
    limit: limit,
    methods: [
      for (final method in methods?.map((method) => (method as JSString).toDart) ?? _retryMethods)
        _normalizeRetryMethod(method),
    ],
    statusCodes: statusCodes?.map((status) => (status as JSNumber).toDartDouble).toList() ?? _retryStatusCodes,
    maxRetryAfter: maxRetryAfter,
    backoffLimit: backoffLimit,
    delay: retry.delay,
    retryOnTimeout: retryOnTimeout,
  );
}

/// Milliseconds, or null for `false`.
double? _validateTimeoutOption(JSAny? value) {
  if (value.isA<JSBoolean>() && !(value as JSBoolean).toDart) return null;
  final milliseconds = _nonNegative(value);
  if (milliseconds == null) throw HostTypeError('The `timeout` option must be a non-negative number or `false`');
  // Infinity lands here too, since it is not a usable delay either.
  if (milliseconds > _maxSafeTimeout) throw HostRangeError('The `timeout` option cannot be greater than 2147483647');
  return milliseconds;
}

/// Option merging (deepMerge, mergeHeaderContainers) keys the headers by their lowercased name,
/// the last spelling of a name winning at the place of its first. A Dart map keeps insertion
/// order.
Map<String, JSAny?> _lowercaseHeaders(JSObject headers) => {
  for (final name in objectKeys(headers).toDart) name.toDart.toLowerCase(): headers.getProperty(name),
};

/// mergeHeaders: each header set on a fresh Headers, which validates it.
Future<Headers> _mergeHeaders(Map<String, JSAny?> headers) async {
  final result = Headers();
  for (final MapEntry(key: name, :value) in headers.entries) {
    await hostCall<JSAny?>(() {
      result.set(name, value);
      return null;
    });
  }
  return result;
}

const _networkErrorMessages = {
  'network error',
  'NetworkError when attempting to fetch resource.',
  'The Internet connection appears to be offline.',
  'Network request failed',
  'fetch failed',
  'terminated',
  ' A network error occurred.',
  'Network connection lost',
};

/// utils/is-network-error.js, inlined by Ky from is-network-error 1.3.2.
bool _isRawNetworkError(Object error) {
  if (error is! JSObject || !isHostError(error) || hostErrorName(error) != 'TypeError') return false;
  final message = error['message'];
  if (!message.isA<JSString>()) return false;
  final text = (message as JSString).toDart;
  bool wrapped(String prefix) => text.startsWith(prefix) && text.endsWith(')');
  if (text == 'Load failed' || wrapped('Load failed (')) {
    // Safari 17+ has generic message but no stack for network errors
    return error['stack'].isUndefined || reflectHas(error, '__sentry_captured__');
  }
  return text.startsWith('error sending request for url') ||
      text == 'Failed to fetch' ||
      wrapped('Failed to fetch (') ||
      _networkErrorMessages.contains(text);
}

/// JS whitespace: what String.prototype.trim removes and `\s` matches. Dart's trim adds U+0085.
bool _isJsWhitespace(int unit) => switch (unit) {
  0x09 || 0x0a || 0x0b || 0x0c || 0x0d || 0x20 || 0xa0 || 0x1680 || 0x2028 || 0x2029 => true,
  0x202f || 0x205f || 0x3000 || 0xfeff => true,
  _ => unit >= 0x2000 && unit <= 0x200a,
};

String _jsTrim(String text) {
  var start = 0;
  var end = text.length;
  while (start < end && _isJsWhitespace(text.codeUnitAt(start))) {
    start++;
  }
  while (end > start && _isJsWhitespace(text.codeUnitAt(end - 1))) {
    end--;
  }
  return text.substring(start, end);
}

/// Matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`, written out rather than
/// matched with a pattern, as Ky does.
bool _isJsonContentType(String contentType) {
  final semicolon = contentType.indexOf(';');
  final mimeType = _jsTrim(semicolon == -1 ? contentType : contentType.substring(0, semicolon)).toLowerCase();
  if (!mimeType.endsWith('json')) return false;
  if (mimeType.length < 5) return false;
  return switch (mimeType[mimeType.length - 5]) {
    '/' => true,
    // The separator is not the slash, so any slash in the type comes before it.
    '.' || '+' || '-' => mimeType.contains('/'),
    _ => false,
  };
}

final _charsetPattern = RegExp(r';\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))', caseSensitive: false);

TextDecoder _createTextDecoder(String contentType) {
  final match = _charsetPattern.firstMatch(contentType);
  final charset = match?[1] ?? match?[2];
  if (charset != null && charset.isNotEmpty) {
    try {
      return TextDecoder(charset);
    } catch (_) {
      // An unknown label falls back to UTF-8.
    }
  }
  return TextDecoder();
}

/// An error body as text, or null when it cannot be read or is larger than
/// maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259).
Future<String?> _readResponseText(Response response) async {
  if (response.body == null) {
    try {
      return (await response.text().toDart).toDart;
    } catch (_) {
      return null;
    }
  }
  final contentType = response.headers.get('content-type') ?? '';
  final decoder = _isJsonContentType(contentType) ? TextDecoder() : _createTextDecoder(contentType);
  try {
    final bytes = await response.arrayBuffer().toDart;
    return bytes.toDart.lengthInBytes > _maxErrorResponseBodySize ? null : decoder.decode(bytes);
  } catch (_) {
    return null;
  }
}

/// #getResponseData: the parsed JSON or the text of an error body, or null when it is empty,
/// unreadable or unparsable.
Future<JSAny?> _getResponseData(Response response) async {
  final text = await _readResponseText(response);
  if (text == null || text.isEmpty) return null;
  if (!_isJsonContentType(response.headers.get('content-type') ?? '')) return text.toJS;
  try {
    return jsonParse(text);
  } catch (_) {
    return null;
  }
}

AbortSignal _createManagedSignal(AbortSignal? user, AbortController controller) =>
    user == null ? controller.signal : AbortSignal.any([user, controller.signal].toJS);

Request _withSignal(Request request, AbortSignal signal) => Request(request, JSObject()..['signal'] = signal);

class Ky {
  Ky._(
    this.request,
    this._abortController,
    this._signal,
    this._userProvidedAbortSignal,
    this._retry,
    this._timeout,
    this._throwHttpErrors,
    this._hooks,
    this._transport,
  );

  Request request;
  AbortController _abortController;

  /// #options.signal: the user's signal and the abort controller's, combined.
  AbortSignal _signal;
  final AbortSignal? _userProvidedAbortSignal;
  final _Retry _retry;
  final double? _timeout;
  final bool _throwHttpErrors;
  final Hooks _hooks;

  /// The `fetch` option.
  final Future<Response> Function(Request request) _transport;
  int _retryCount = 0;
  Request? _originalRequest;

  static Future<Ky> _create(String input, Options options) async {
    final headers = options.headers == null ? <String, JSAny?>{} : _lowercaseHeaders(options.headers!);
    final requestHeaders = await _mergeHeaders(headers);
    final method = _normalizeRequestMethod(options.method ?? 'GET');
    final retry = _normalizeRetryOptions(options.retry);
    // Defaults only replace undefined, so a null reaches the validation and is reported.
    final bool throwHttpErrors;
    if (options.throwHttpErrors.isUndefined) {
      throwHttpErrors = true;
    } else if (options.throwHttpErrors.isA<JSBoolean>()) {
      throwHttpErrors = (options.throwHttpErrors as JSBoolean).toDart;
    } else {
      throw HostTypeError('The `throwHttpErrors` option must be a boolean or a function');
    }
    final timeout = _validateTimeoutOption(options.timeout.isUndefined ? 10000.toJS : options.timeout);
    final abortController = AbortController();
    final signal = _createManagedSignal(options.signal, abortController);
    // A content-type from options.headers counts as user-provided.
    final body = options.json.isUndefined ? null : jsonStringify(options.json);
    if (body != null && !headers.containsKey('content-type')) {
      requestHeaders.set('content-type', 'application/json'.toJS);
    }
    final init = JSObject()
      ..['method'] = method.toJS
      ..['headers'] = requestHeaders
      ..['signal'] = signal;
    if (body != null) init['body'] = body.toJS;
    final request = await hostCall(() => Request(input.toJS, init));
    return Ky._(
      request,
      abortController,
      signal,
      options.signal,
      retry,
      timeout,
      throwHttpErrors,
      options.hooks,
      options.fetch,
    );
  }

  /// `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body read. A
  /// construction error fails without running the beforeError hooks, and so does a body that is
  /// not JSON.
  static Future<JSAny?> send(String input, Options options, Shortcut shortcut) async {
    for (final hook in options.hooks.init) {
      hook();
    }
    final ky = await _create(input, options);
    // Before dispatch, ky.request is the request that will be sent.
    final headers = ky.request.headers;
    final accept = headers.get('accept');
    headers.set('accept', (accept != null && accept.isNotEmpty ? accept : shortcut.mimeType).toJS);
    final Response response;
    try {
      response = await ky._run();
    } catch (error) {
      ky._throwProcessedError(error);
    }
    final text = (await response.text().toDart).toDart;
    return switch (shortcut) {
      Shortcut.text => text.toJS,
      Shortcut.json => jsonParse(text),
    };
  }

  /// #sentRequest: [_fetch] replaces [request] with the clone it prepares for a possible retry, so
  /// the request that was actually sent is reported.
  Request get _sentRequest => _originalRequest ?? request;

  /// Chromium reports a user abort during a fetch as a network error; the abort reason is
  /// surfaced instead.
  void _throwIfAbortedByUser() => _userProvidedAbortSignal?.throwIfAborted();

  /// A hook runs inside #raceWithTotalTimeout(), which first throws a user abort.
  void _runHooks<H>(List<H> hooks, void Function(H hook) call) {
    for (final hook in hooks) {
      _throwIfAbortedByUser();
      call(hook);
    }
  }

  Future<Response> _run() async {
    _runHooks(_hooks.beforeRequest, (hook) => hook(request, 0));
    var response = await _retryRequest();
    for (;;) {
      _runHooks(_hooks.afterResponse, (hook) => hook(request, response, _retryCount));
      if (response.ok || !_throwHttpErrors) return response;
      final error = HTTPError(response, _sentRequest, await _getResponseData(response));
      _throwIfAbortedByUser();
      response = await _retryFromError(error);
    }
  }

  Future<Response> _retryRequest() async {
    try {
      return await _fetch();
    } catch (error) {
      return _retryFromError(error);
    }
  }

  Future<Response> _retryFromError(Object error) async {
    _throwIfAbortedByUser();
    final retryDelay = math.min(_calculateRetryDelay(error), _maxSafeTimeout);
    await Future<void>.delayed(Duration(microseconds: (retryDelay * 1000).round()));
    // Reset the abort controller if it was aborted (on a timeout retry), so hooks and the retried
    // request get a fresh signal.
    if (_abortController.signal.aborted) {
      _abortController = AbortController();
      _signal = _createManagedSignal(_userProvidedAbortSignal, _abortController);
      request = _withSignal(request, _signal);
    }
    _runHooks(_hooks.beforeRetry, (hook) => hook(request, error, _retryCount + 1));
    _retryCount++;
    return _retryRequest();
  }

  Future<Response> _fetch() async {
    // Cloning is done here to prepare in advance for retries, and skipped when they are disabled.
    final retryRequest = _retry.limit > 0 ? _withSignal(request.copy(), _signal) : null;
    final sent = request;
    _originalRequest = sent;
    if (retryRequest != null) request = retryRequest;
    try {
      final timeout = _timeout;
      return await (timeout == null ? _transport(sent) : _race(sent, timeout));
    } catch (error) {
      _throwIfAbortedByUser();
      if (_isRawNetworkError(error)) throw NetworkError(sent, error as JSObject);
      rethrow;
    }
  }

  /// utils/timeout.js: the transport raced against a timer that aborts the controller and fails
  /// with a TimeoutError. A response arriving after the timeout is discarded.
  Future<Response> _race(Request request, double milliseconds) async {
    final outcome = Completer<Response>();
    final timer = Timer(Duration(microseconds: (milliseconds * 1000).round()), () {
      _abortController.abort();
      if (!outcome.isCompleted) outcome.completeError(TimeoutError(request));
    });
    _transport(request).then(
      (response) {
        if (!outcome.isCompleted) outcome.complete(response);
      },
      onError: (Object error, StackTrace stack) {
        if (!outcome.isCompleted) outcome.completeError(error, stack);
      },
    );
    try {
      return await outcome.future;
    } finally {
      timer.cancel();
    }
  }

  /// #calculateDelay(): the retry.delay backoff, capped by retry.backoffLimit.
  double _calculateDelay() {
    final retryDelay = _retry.delay(_retryCount + 1);
    if (!(retryDelay >= 0)) throw HostTypeError('`retry.delay` must return a non-negative number or `Infinity`');
    return math.min(_retry.backoffLimit, retryDelay);
  }

  /// #calculateRetryDelay(): the wait before retrying [error], which is thrown again when it is
  /// not retried.
  double _calculateRetryDelay(Object error) {
    if (_retryCount >= _retry.limit) throw error;
    if (!_retry.methods.contains(_normalizeRetryMethod(request.method))) throw error;
    switch (error) {
      case TimeoutError():
        return _retry.retryOnTimeout ? _calculateDelay() : throw error;
      case HTTPError(:final response):
        final status = response.status.toDouble();
        if (!_retry.statusCodes.contains(status)) throw error;
        final header = getRetryTimingHeader(response.headers);
        if (header != null && _retry.afterStatusCodes.contains(status)) {
          // Malformed retry timing headers should not disable retries; they only lose their
          // server-provided timing. No jitter applies to a server's timing.
          final after = calculateRetryTimingDelay(header);
          return after == null ? _calculateDelay() : math.min(_retry.maxRetryAfter, after);
        }
        if (status == 413) throw error;
        return _calculateDelay();
      case NetworkError():
        return _calculateDelay();
      default:
        // Only known retriable error types are retried.
        throw error;
    }
  }

  /// #throwProcessedError(): the beforeError hooks see the error, which then fails the call.
  Never _throwProcessedError(Object error) {
    for (final hook in _hooks.beforeError) {
      hook(_sentRequest, error, _retryCount);
    }
    throw error;
  }
}
