// Entry of the Dart port: what libraries/ky/reference.mjs does, on the port's Ky. Each request
// case gets its own abort controller, transport and hooks, which record the trace.
//
// The boundary: the corpus input is the JS object itself, seen through dart:js_interop extension
// types (no copy). The option values Ky validates stay the JS values they are (JSAny?, undefined
// where omitted) until Ky's own checks read them. Each result is the JS array [trace, outcome].
//
// dart2js cannot emit ES module exports, so main() publishes `run` on globalThis.__matrixExports,
// which the builder's facade reads once and deletes.

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;

import 'host.dart';
import 'ky.dart';

extension type Reply._(JSObject _) implements JSObject {
  external String get kind;
  external JSAny? get status;
  external JSAny? get body;
  external JSAny? get headers;
}

extension type Case._(JSObject _) implements JSObject {
  external String get url;
  external String? get method;
  external JSObject? get headers;
  external JSAny? get json;
  external String get read;
  external JSArray<Reply> get replies;
  external JSAny? get limit;
  external JSAny? get backoffLimit;
  external JSAny? get maxRetryAfter;
  external JSAny? get timeout;
  external JSAny? get retryOnTimeout;
  external JSAny? get throwHttpErrors;
  external String get abort;
  external JSAny? get methods;
  external JSAny? get statusCodes;
}

extension type Input._(JSObject _) implements JSObject {
  external JSArray<Case> get cases;
}

/// What the adapter aborts its controller with.
void _cancel(AbortController controller) => controller.abort(HostError('cancelled')..name = 'AbortError');

JSAny? _nullable(String? text) => text?.toJS;

Future<JSArray<JSAny?>> _requestCase(Case c) async {
  final trace = <JSAny?>[];
  void record(List<JSAny?> entry) => trace.add(entry.toJS);
  final controller = AbortController();
  if (c.abort == 'before') _cancel(controller);
  final replies = c.replies.toDart;
  var sent = 0;
  // The injected transport: it records what it was sent and answers with the next reply.
  Future<Response> fetch(Request request) async {
    final body = (await request.text().toDart).toDart;
    final headers = request.headers;
    record([
      'fetch'.toJS,
      request.method.toJS,
      request.url.toJS,
      _nullable(headers.get('accept')),
      _nullable(headers.get('content-type')),
      _nullable(headers.get('x-matrix')),
      body.toJS,
    ]);
    if (c.abort == 'fetch') _cancel(controller);
    final signal = request.signal;
    signal.throwIfAborted();
    final reply = replies[math.min(sent++, replies.length - 1)];
    switch (reply.kind) {
      case 'network':
        throw HostTypeError('Failed to fetch');
      case 'stall':
        final stalled = Completer<Response>();
        signal.addEventListener(
          'abort',
          (() => stalled.completeError(signal.reason!)).toJS,
          JSObject()..['once'] = true.toJS,
        );
        return stalled.future;
      default:
        final init = JSObject()
          ..['status'] = reply.status
          ..['headers'] = reply.headers;
        return hostCall(() => Response(reply.body, init));
    }
  }

  double delay(int attempt) {
    final milliseconds = 300 * math.pow(2, attempt - 1).toDouble();
    record(['backoff'.toJS, attempt.toJS, milliseconds.toJS]);
    return milliseconds;
  }

  final hooks = Hooks(
    init: [() => record(['init'.toJS])],
    beforeRequest: [
      (request, retryCount) {
        record(['beforeRequest'.toJS, retryCount.toJS]);
        request.headers.set('x-matrix', 'initial'.toJS);
      },
    ],
    afterResponse: [
      (request, response, retryCount) => record(['afterResponse'.toJS, retryCount.toJS, response.status.toJS]),
    ],
    beforeRetry: [
      (request, error, retryCount) {
        record(['beforeRetry'.toJS, retryCount.toJS, errorName(error).toJS]);
        request.headers.set('x-matrix', 'retry-$retryCount'.toJS);
        if (c.abort == 'retry') _cancel(controller);
      },
    ],
    beforeError: [
      (request, error, retryCount) => record(['beforeError'.toJS, retryCount.toJS, errorName(error).toJS]),
    ],
  );
  final options = Options(
    method: c.method,
    headers: c.headers,
    json: c.json,
    retry: RetryOptions(
      limit: c.limit,
      methods: c.methods,
      statusCodes: c.statusCodes,
      maxRetryAfter: c.maxRetryAfter,
      backoffLimit: c.backoffLimit,
      retryOnTimeout: c.retryOnTimeout,
      delay: delay,
    ),
    timeout: c.timeout,
    throwHttpErrors: c.throwHttpErrors,
    hooks: hooks,
    fetch: fetch,
    signal: controller.signal,
  );
  List<JSAny?> outcome;
  try {
    outcome = ['ok'.toJS, await Ky.send(c.url, options, c.read == 'json' ? Shortcut.json : Shortcut.text)];
  } on HTTPError catch (error) {
    outcome = ['error'.toJS, 'HTTPError'.toJS, error.response.status.toJS, error.data];
  } catch (error) {
    outcome = ['error'.toJS, errorName(error).toJS, null, null];
  }
  return <JSAny?>[trace.toJS, outcome.toJS].toJS;
}

JSPromise<JSArray<JSAny?>> run(Input input) => () async {
  final results = <JSAny?>[];
  for (final c in input.cases.toDart) {
    results.add(await _requestCase(c));
  }
  return results.toJS;
}().toJS;

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
