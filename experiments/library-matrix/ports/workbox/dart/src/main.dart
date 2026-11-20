import 'dart:js_interop';
import 'dart:js_interop_unsafe';

typedef J = JSAny?;
typedef StepFn = J Function(J);
@JS('undefined')
external J get undefinedValue;
@JS('Boolean')
external bool truth(J x);
@JS('Reflect.construct')
external JSObject construct(JSObject ctor, JSArray<J> args);
@JS('skipWaiting')
external J skipWaiting();
@JS('fetch')
external J fetchRequest(J req);
@JS('Array.from')
external J collect(J iterator);
@JS('Object.is')
external bool same(J a, J b);
@JS('crypto.randomUUID')
external String uuid();
J uniqueName() => ('mires-workbox-' + uuid()).toJS;
J globalValue(String k) => globalContext[k];
J makeHost(String k, List<J> args) =>
    construct(globalValue(k) as JSObject, args.toJS);
J get(J x, String k) => (x as JSObject).getProperty(k.toJS);
void setProp(J x, String k, J v) {
  (x as JSObject)[k] = v;
}

J at(J x, int i) => (x as JSArray<J>).toDart[i];
J strVal(String s) => s.toJS;
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
bool equalNum(J x, int i) => x != null && (x as JSNumber).toDartDouble == i;
J chain(J p, StepFn f) => call(p, 'then', [f.toJS]);
J recoverPromise(J p, StepFn f) => call(p, 'catch', [f.toJS]);
J always(J p, StepFn f) => call(p, 'finally', [(() => f(undef())).toJS]);
J ok(J v) => call(globalValue('Promise'), 'resolve', [v]);
J bad(J v) => call(globalValue('Promise'), 'reject', [v]);
void mapValues(J xs, StepFn f) {
  for (final v in (xs as JSArray<J>).toDart) {
    f(v);
  }
}

J sequence(J xs, StepFn f) {
  var p = ok(undef());
  for (final v in (xs as JSArray<J>).toDart) {
    p = chain(p, (_) => f(v));
  }
  return p;
}

J suffix(J a, String tail) => ((a as JSString).toDart + tail).toJS;
bool starts(J a, String prefix) => (a as JSString).toDart.startsWith(prefix);
bool ends(J a, String tail) => (a as JSString).toDart.endsWith(tail);
J relative(J path, J base) =>
    (path as JSString).toDart.substring((base as JSString).toDart.length).toJS;
bool badStatus(J status) => (status as JSNumber).toDartInt >= 400;
J defaulted(J a, J b) => missing(a) ? b : a;

J absolute(J path, J base) {
  return get(makeHost("URL", <J>[path, base]), "href");
}

J network(J source, J base) {
  if (equalText(source, "offline")) {
    return bad(strVal("offline"));
  } else {
    final req = makeHost("Request", <J>[
      absolute(suffix(source, ".mjs"), base),
    ]);
    return fetchRequest(req);
  }
}

J precacheKey(J entries, J path, J base) {
  final url = makeHost("URL", <J>[path, base]);
  setProp(url, "hash", strVal(""));
  final exact = call(entries, "get", <J>[get(url, "href")]);
  if (!missing(exact)) {
    return exact;
  } else {
    final params = get(url, "searchParams");
    final keys = collect(call(params, "keys", <J>[]));
    J ignore(J key) {
      if ((equalText(key, "fbclid") || starts(key, "utm_"))) {
        call(params, "delete", <J>[key]);
      }
      return undef();
    }

    mapValues(keys, ignore);
    final stripped = call(entries, "get", <J>[get(url, "href")]);
    if (!missing(stripped)) {
      return stripped;
    } else {
      final original = get(url, "pathname");
      setProp(url, "pathname", suffix(original, "index.html"));
      final index = (ends(original, "/")
          ? call(entries, "get", <J>[get(url, "href")])
          : undef());
      if (!missing(index)) {
        return index;
      } else {
        setProp(url, "pathname", suffix(original, ".html"));
        return call(entries, "get", <J>[get(url, "href")]);
      }
    }
  }
}

J install(J entries, J cache, J source, J base) {
  final updated = pack(<J>[]);
  final unchanged = pack(<J>[]);
  J check(J entry) {
    final original = at(entry, 0);
    final key = at(entry, 1);
    J found(J hit) {
      if (!missing(hit)) {
        push(unchanged, relative(original, base));
        return undef();
      } else {
        J received(J res) {
          if (badStatus(get(res, "status"))) {
            return bad(strVal("bad-precaching-response"));
          } else {
            J stored(J unused) {
              push(updated, relative(original, base));
              return undef();
            }

            return chain(
              call(cache, "put", <J>[key, call(res, "clone", <J>[])]),
              stored,
            );
          }
        }

        return chain(network(source, base), received);
      }
    }

    return chain(call(cache, "match", <J>[key]), found);
  }

  J complete(J unused) {
    return pack(<J>[updated, unchanged]);
  }

  return chain(
    sequence(collect(call(entries, "entries", <J>[])), check),
    complete,
  );
}

J activate(J entries, J cache, J base) {
  final active = collect(call(entries, "values", <J>[]));
  final removed = pack(<J>[]);
  J listKeys(J keys) {
    J remove(J req) {
      final path = get(req, "url");
      if (truth(call(active, "includes", <J>[path]))) {
        return undef();
      } else {
        J deleted(J unused) {
          push(removed, relative(path, base));
          return undef();
        }

        return chain(call(cache, "delete", <J>[path]), deleted);
      }
    }

    J complete(J unused) {
      return call(removed, "sort", <J>[]);
    }

    return chain(sequence(keys, remove), complete);
  }

  return chain(call(cache, "keys", <J>[]), listKeys);
}

J strategy(J entries, J cache, J name, J path, J source, J base) {
  final key = precacheKey(entries, path, base);
  final route = relative(path, base);
  if (!missing(key)) {
    J precached(J hit) {
      return (missing(hit) ? network(source, base) : hit);
    }

    return chain(call(cache, "match", <J>[key]), precached);
  } else {
    if (starts(route, "network/")) {
      return network(source, base);
    } else {
      if ((starts(route, "first/") || starts(route, "only/"))) {
        J opened(J runtime) {
          J matched(J hit) {
            if (!missing(hit)) {
              return hit;
            } else {
              if (starts(route, "only/")) {
                return bad(strVal("no-response"));
              } else {
                J fetched(J response) {
                  if (equalNum(get(response, "status"), 200)) {
                    J stored(J unused) {
                      return response;
                    }

                    return chain(
                      call(runtime, "put", <J>[
                        path,
                        call(response, "clone", <J>[]),
                      ]),
                      stored,
                    );
                  } else {
                    return response;
                  }
                }

                return chain(network(source, base), fetched);
              }
            }
          }

          return chain(call(runtime, "match", <J>[path]), matched);
        }

        return chain(
          call(globalValue("caches"), "open", <J>[suffix(name, "-runtime")]),
          opened,
        );
      } else {
        return ok(nil());
      }
    }
  }
}

J fetchRequestForStep(J step, J base) {
  final request = get(step, "request");
  if (truth(request)) return request;
  final init = objectValue();
  final method = get(step, "method");
  if (truth(method)) setProp(init, "method", method);
  return makeHost("Request", <J>[absolute(get(step, "path"), base), init]);
}

J phase(J step, J name, J items, J base) {
  final entries = makeHost("Map", <J>[]);
  final state = objectValue();
  J add(J item) {
    final url = makeHost("URL", <J>[get(item, "path"), base]);
    final original = get(url, "href");
    final revision = get(item, "revision");
    if (truth(revision)) {
      call(get(url, "searchParams"), "set", <J>[
        strVal("__WB_REVISION__"),
        revision,
      ]);
    }
    final key = get(url, "href");
    final previous = call(entries, "get", <J>[original]);
    if ((!missing(previous) && !same(previous, key))) {
      setProp(state, "conflict", boolean(true));
    }
    call(entries, "set", <J>[original, key]);
    return undef();
  }

  mapValues(items, add);
  if (truth(get(state, "conflict"))) {
    return bad(strVal("conflicting revisions"));
  } else {
    final op = get(step, "phase");
    final source = defaulted(get(step, "source"), strVal("alpha"));
    J opened(J cache) {
      if (equalText(op, "install")) {
        return install(entries, cache, source, base);
      } else {
        if (equalText(op, "activate")) {
          return activate(entries, cache, base);
        } else {
          if (equalText(op, "message")) {
            if (equalText(get(step, "message"), "SKIP_WAITING")) {
              J skipped(J unused) {
                return boolean(true);
              }

              return chain(skipWaiting(), skipped);
            } else {
              return boolean(false);
            }
          } else {
            final req = fetchRequestForStep(step, base);
            final method = get(req, "method");
            if (!equalText(method, "GET")) {
              return nil();
            } else {
              final path = get(req, "url");
              J received(J response) {
                if (nullish(response)) {
                  return nil();
                } else {
                  J body(J text) {
                    return pack(<J>[get(response, "status"), text]);
                  }

                  return chain(call(response, "text", <J>[]), body);
                }
              }

              J failed(J unused) {
                return pack(<J>[strVal("no-response")]);
              }

              return recoverPromise(
                chain(
                  strategy(entries, cache, name, path, source, base),
                  received,
                ),
                failed,
              );
            }
          }
        }
      }
    }

    return chain(call(globalValue("caches"), "open", <J>[name]), opened);
  }
}

J run(J input) {
  final base = absolute(
    strVal("experiments/library-matrix/libraries/workbox/"),
    get(globalValue("location"), "href"),
  );
  if (!missing(get(input, "phase"))) {
    final task = phase(
      input,
      get(input, "name"),
      defaulted(get(input, "manifest"), pack(<J>[])),
      base,
    );
    final event = get(input, "event");
    if (truth(event)) {
      call(event, "waitUntil", <J>[task]);
    }
    return task;
  } else {
    final name = uniqueName();
    final state = objectValue();
    setProp(state, "manifest", pack(<J>[]));
    final results = pack(<J>[]);
    J execute(J step) {
      final items = defaulted(get(step, "manifest"), get(state, "manifest"));
      setProp(state, "manifest", items);
      J collectResult(J result) {
        push(results, result);
        return undef();
      }

      return chain(phase(step, name, items, base), collectResult);
    }

    J complete(J unused) {
      return results;
    }

    J cleanup(J unused) {
      final caches = globalValue("caches");
      return call(globalValue("Promise"), "all", <J>[
        pack(<J>[
          call(caches, "delete", <J>[name]),
          call(caches, "delete", <J>[suffix(name, "-runtime")]),
        ]),
      ]);
    }

    return always(
      chain(sequence(get(input, "steps"), execute), complete),
      cleanup,
    );
  }
}

void main() {
  final exports = JSObject();
  exports["run"] = run.toJS;
  globalContext["__matrixExports"] = exports;
}
