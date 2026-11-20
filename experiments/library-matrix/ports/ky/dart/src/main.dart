import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;

typedef J = JSAny?;
typedef Step = J Function(J);
@JS('undefined')
external J get undefinedValue;
@JS('Boolean')
external bool truth(J x);
@JS('Number.isFinite')
external bool isNumeric(J x);
@JS('Reflect.construct')
external JSObject construct(JSObject ctor, JSArray<J> args);

@JS('Promise')
extension type EventPromise._(JSObject _) implements JSObject {
  external factory EventPromise(JSFunction executor);
}

@JS('setTimeout')
external J schedule(J f, J ms, J argument);
J timer(J f, J ms) => schedule(f, ms, undef());
@JS('clearTimeout')
external void stopTimer(J id);
J globalValue(String k) => globalContext[k];
J makeHost(String k, List<J> args) =>
    construct(globalValue(k) as JSObject, args.toJS);
J get(J x, String k) => (x as JSObject).getProperty(k.toJS);
void setProp(J x, String k, J v) {
  (x as JSObject)[k] = v;
}

J at(J x, int i) => (x as JSArray<J>).toDart[i];
J strVal(String s) => s.toJS;
J num(int i) => i.toJS;
J boolean(bool b) => b.toJS;
J nil() => null;
J undef() => undefinedValue;
J objectValue() => JSObject();
J pack(List<J> xs) => xs.toJS;
void push(J a, J v) {
  (a as JSArray<J>).toDart.add(v);
}

J call(J x, String k, List<J> args) =>
    (x as JSObject).callMethodVarArgs(k.toJS, args);
bool missing(J x) => x.isUndefined;
bool nullish(J x) => x == null;
bool equalText(J x, String s) => x != null && (x as JSString).toDart == s;
bool equalNum(J x, int n) => x != null && (x as JSNumber).toDartDouble == n;
J callback(Step f) => f.toJS;
J promise(J Function(J, J) f) => EventPromise(f.toJS);
void settle(J f, J v) {
  (f as JSFunction).callAsFunction(null, v);
}

J chain(J p, Step f) => call(p, 'then', [callback(f)]);
J recoverPromise(J p, Step f) => call(p, 'catch', [callback(f)]);
J handled(J p, Step a, Step b) => call(p, 'then', [callback(a), callback(b)]);
J always(J p, Step f) => call(p, 'finally', [(() => f(undef())).toJS]);
J header(J h, String k) => call(h, 'get', [k.toJS]);
J ok(J v) => call(globalValue('Promise'), 'resolve', [v]);
J bad(J v) => call(globalValue('Promise'), 'reject', [v]);
J sequence(J xs, Step f) {
  var p = ok(undef());
  for (final item in (xs as JSArray<J>).toDart) {
    p = chain(p, (_) => f(item));
  }
  return p;
}

J waitTime(J ms) => promise((resolve, reject) {
  timer(resolve, ms);
  return undef();
});
J lastOrIndex(J xs, J n) {
  final a = (xs as JSArray<J>).toDart;
  return a[math.min((n as JSNumber).toDartInt, a.length - 1)];
}

J increment(J a) => ((a as JSNumber).toDartInt + 1).toJS;
bool smaller(J a, J b) =>
    (a as JSNumber).toDartDouble < (b as JSNumber).toDartDouble;
J lesser(J a, J b) => smaller(a, b) ? a : b;
J retryTag(J a) => ('retry-' + (a as JSNumber).toDartInt.toString()).toJS;
J exponential(J a) => (300 * math.pow(2, (a as JSNumber).toDartInt)).toJS;
bool jsonType(J a) {
  if (a == null) return false;
  final m = (a as JSString).toDart.split(';').first.trim().toLowerCase();
  return m.contains('/') &&
      ['/json', '+json', '-json', '.json'].any(m.endsWith);
}

J parseAfter(J a) {
  final s = (a as JSString).toDart.trim();
  return RegExp(r'^[0-9]+$').hasMatch(s) ? (1000 * double.parse(s)).toJS : null;
}

String canonicalMethod(String method) {
  return const {
        'get',
        'post',
        'put',
        'patch',
        'head',
        'delete',
        'query',
        'options',
        'trace',
      }.contains(method.toLowerCase())
      ? method.toUpperCase()
      : method;
}

bool allowsMethod(J c, J m) {
  final raw = get(c, 'methods') as JSArray<JSString>?;
  final k = canonicalMethod((m as JSString).toDart);
  return raw == null
      ? [
          'get',
          'put',
          'head',
          'delete',
          'options',
          'trace',
          'query',
        ].any((x) => canonicalMethod(x) == k)
      : raw.toDart.any((x) => canonicalMethod(x.toDart) == k);
}

bool allowsStatus(J c, J s) {
  if (s == null) return false;
  final raw = get(c, 'statusCodes') as JSArray<JSNumber>?;
  final v = (s as JSNumber).toDartInt;
  return raw == null
      ? [408, 413, 429, 500, 502, 503, 504].contains(v)
      : raw.toDart.any((x) => x.toDartInt == v);
}

J makeError(J name, J message) {
  final value = makeHost("Error", <J>[message]);
  setProp(value, "name", name);
  return value;
}

J buildRequest(J c) {
  final init = objectValue();
  final headers = makeHost("Headers", <J>[get(c, "headers")]);
  setProp(
    init,
    "method",
    canonicalMethod((get(c, "method") as JSString?)?.toDart ?? "GET").toJS,
  );
  setProp(init, "headers", headers);
  final payload = get(c, "json");
  if (!missing(payload)) {
    setProp(init, "body", call(globalValue("JSON"), "stringify", <J>[payload]));
    if (nullish(header(headers, "content-type"))) {
      call(headers, "set", <J>[
        strVal("content-type"),
        strVal("application/json"),
      ]);
    }
  }
  if (!truth(header(headers, "accept"))) {
    final mime = (equalText(get(c, "read"), "json")
        ? strVal("application/json")
        : strVal("text/*"));
    call(headers, "set", <J>[strVal("accept"), mime]);
  }
  return makeHost("Request", <J>[get(c, "url"), init]);
}

J fixtureFetch(J c, J req, J user, J count, J trace) {
  final h = get(req, "headers");
  J respond(J body) {
    push(
      trace,
      pack(<J>[
        strVal("fetch"),
        get(req, "method"),
        get(req, "url"),
        header(h, "accept"),
        header(h, "content-type"),
        header(h, "x-matrix"),
        body,
      ]),
    );
    if (equalText(get(c, "abort"), "fetch")) {
      call(user, "abort", <J>[
        makeError(strVal("AbortError"), strVal("cancelled")),
      ]);
    }
    final signal = get(req, "signal");
    if (truth(get(signal, "aborted"))) {
      return bad(get(signal, "reason"));
    } else {
      final reply = lastOrIndex(get(c, "replies"), count);
      if (equalText(get(reply, "kind"), "network")) {
        return bad(makeHost("TypeError", <J>[strVal("Failed to fetch")]));
      } else {
        if (equalText(get(reply, "kind"), "stall")) {
          J stall(J resolve, J reject) {
            J aborted(J event) {
              settle(reject, get(signal, "reason"));
              return undef();
            }

            final options = objectValue();
            setProp(options, "once", boolean(true));
            call(signal, "addEventListener", <J>[
              strVal("abort"),
              callback(aborted),
              options,
            ]);
            return undef();
          }

          return promise(stall);
        } else {
          final options = objectValue();
          setProp(options, "status", get(reply, "status"));
          setProp(options, "headers", get(reply, "headers"));
          return makeHost("Response", <J>[get(reply, "body"), options]);
        }
      }
    }
  }

  return chain(call(req, "text", <J>[]), respond);
}

J dispatch(J c, J base, J user, J count, J trace) {
  final ctl = makeHost("AbortController", <J>[]);
  final signal = call(globalValue("AbortSignal"), "any", <J>[
    pack(<J>[get(user, "signal"), get(ctl, "signal")]),
  ]);
  final init = objectValue();
  setProp(init, "signal", signal);
  final req = makeHost("Request", <J>[call(base, "clone", <J>[]), init]);
  final fetching = fixtureFetch(c, req, user, count, trace);
  final timeout = get(c, "timeout");
  if (isNumeric(timeout)) {
    final holder = objectValue();
    J executor(J resolve, J reject) {
      J expiry(J unused) {
        settle(
          reject,
          makeError(strVal("TimeoutError"), strVal("Request timed out")),
        );
        call(ctl, "abort", <J>[]);
        return undef();
      }

      setProp(holder, "timer", timer(callback(expiry), timeout));
      return undef();
    }

    final expiryTask = promise(executor);
    J clear(J unused) {
      stopTimer(get(holder, "timer"));
      return undef();
    }

    return always(
      call(globalValue("Promise"), "race", <J>[
        pack(<J>[fetching, expiryTask]),
      ]),
      clear,
    );
  } else {
    return fetching;
  }
}

J parseJson(J body) {
  return call(globalValue("JSON"), "parse", <J>[body]);
}

J errorData(J response) {
  final h = get(response, "headers");
  J read(J body) {
    if (!truth(body)) {
      return nil();
    } else {
      if (jsonType(header(h, "content-type"))) {
        J invalid(J unused) {
          return nil();
        }

        return recoverPromise(chain(ok(body), parseJson), invalid);
      } else {
        return body;
      }
    }
  }

  return chain(call(response, "text", <J>[]), read);
}

J retryDelay(J c, J method, J count, J fault, J trace) {
  final name = at(fault, 0);
  final status = at(fault, 1);
  final raw = at(fault, 3);
  final eligible = (allowsMethod(c, method) && smaller(count, get(c, "limit")));
  final serverStatus =
      (equalNum(status, 413) || equalNum(status, 429) || equalNum(status, 503));
  final http = equalText(name, "HTTPError");
  final retriable = (http
      ? allowsStatus(c, status)
      : (equalText(name, "NetworkError") ||
            (equalText(name, "TimeoutError") &&
                truth(get(c, "retryOnTimeout")))));
  if ((!eligible ||
      !retriable ||
      (http && equalNum(status, 413) && !truth(raw)))) {
    return nil();
  } else {
    final after = ((http && serverStatus && truth(raw))
        ? parseAfter(raw)
        : nil());
    if (!nullish(after)) {
      return lesser(after, get(c, "maxRetryAfter"));
    } else {
      final ms = exponential(count);
      push(trace, pack(<J>[strVal("backoff"), increment(count), ms]));
      return lesser(ms, get(c, "backoffLimit"));
    }
  }
}

J outputError(J trace, J count, J f, J hook) {
  if (truth(hook)) {
    push(trace, pack(<J>[strVal("beforeError"), count, at(f, 0)]));
  }
  return pack(<J>[
    trace,
    pack(<J>[strVal("error"), at(f, 0), at(f, 1), at(f, 2)]),
  ]);
}

J attempt(J c, J base, J user, J count, J trace) {
  J onFailure(J f) {
    final delay = retryDelay(c, get(base, "method"), count, f, trace);
    if (nullish(delay)) {
      return outputError(trace, count, f, boolean(true));
    } else {
      J proceed(J unused) {
        final following = increment(count);
        push(trace, pack(<J>[strVal("beforeRetry"), following, at(f, 0)]));
        call(get(base, "headers"), "set", <J>[
          strVal("x-matrix"),
          retryTag(following),
        ]);
        if (equalText(get(c, "abort"), "retry")) {
          call(user, "abort", <J>[
            makeError(strVal("AbortError"), strVal("cancelled")),
          ]);
        }
        return attempt(c, base, user, following, trace);
      }

      return chain(waitTime(delay), proceed);
    }
  }

  J success(J response) {
    final status = get(response, "status");
    push(trace, pack(<J>[strVal("afterResponse"), count, status]));
    if ((!truth(get(response, "ok")) && truth(get(c, "throwHttpErrors")))) {
      J failed(J data) {
        return onFailure(
          pack(<J>[
            strVal("HTTPError"),
            status,
            data,
            header(get(response, "headers"), "retry-after"),
          ]),
        );
      }

      return chain(errorData(response), failed);
    } else {
      J result(J payload) {
        return pack(<J>[
          trace,
          pack(<J>[strVal("ok"), payload]),
        ]);
      }

      final body = call(response, "text", <J>[]);
      if (equalText(get(c, "read"), "json")) {
        J invalid(J unused) {
          return outputError(
            trace,
            count,
            pack(<J>[strVal("SyntaxError"), nil(), nil(), nil()]),
            boolean(false),
          );
        }

        return recoverPromise(chain(chain(body, parseJson), result), invalid);
      } else {
        return chain(body, result);
      }
    }
  }

  J failure(J error) {
    final name = get(error, "name");
    final wrapped =
        ((equalText(name, "TypeError") &&
            equalText(get(error, "message"), "Failed to fetch"))
        ? strVal("NetworkError")
        : name);
    return onFailure(pack(<J>[wrapped, nil(), nil(), nil()]));
  }

  return handled(dispatch(c, base, user, count, trace), success, failure);
}

J requestCase(J c) {
  final trace = pack(<J>[
    pack(<J>[strVal("init")]),
  ]);
  final user = makeHost("AbortController", <J>[]);
  if (equalText(get(c, "abort"), "before")) {
    call(user, "abort", <J>[
      makeError(strVal("AbortError"), strVal("cancelled")),
    ]);
  }
  J ready(J base) {
    if (truth(get(get(user, "signal"), "aborted"))) {
      return ok(
        outputError(
          trace,
          num(0),
          pack(<J>[strVal("AbortError"), nil(), nil(), nil()]),
          boolean(true),
        ),
      );
    } else {
      push(trace, pack(<J>[strVal("beforeRequest"), num(0)]));
      call(get(base, "headers"), "set", <J>[
        strVal("x-matrix"),
        strVal("initial"),
      ]);
      return attempt(c, base, user, num(0), trace);
    }
  }

  J invalid(J error) => outputError(
    trace,
    num(0),
    pack(<J>[get(error, "name"), nil(), nil(), nil()]),
    boolean(false),
  );
  return handled(chain(ok(c), buildRequest), ready, invalid);
}

J run(J input) {
  final results = pack(<J>[]);
  J evaluate(J c) {
    J save(J result) {
      push(results, result);
      return undef();
    }

    return chain(requestCase(c), save);
  }

  J complete(J unused) {
    return results;
  }

  return chain(sequence(get(input, "cases"), evaluate), complete);
}

void main() {
  final exports = JSObject();
  exports["run"] = run.toJS;
  globalContext["__matrixExports"] = exports;
}
