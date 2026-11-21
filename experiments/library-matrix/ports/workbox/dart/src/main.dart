// Entry of the Dart port: what libraries/workbox/reference.mjs does, on the port's Workbox. One call
// runs one script of Workbox API calls (CONTRACT.md "Steps") with the fixture plugins, builds the
// canonical output, and deletes the script's caches on every path.
//
// The boundary: the corpus input is the JS object itself, seen through dart:js_interop extension
// types (no copy); manifest entries become `PrecacheEntry` values, with each revision kept as the
// caller's JS value. The output is built from JS objects and arrays in the contract's key order.
//
// dart2js cannot emit ES module exports, so main() publishes `run` on globalThis.__matrixExports,
// which the builder's facade reads once and deletes.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'host.dart';
import 'workbox.dart';

const _fixture = 'experiments/library-matrix/libraries/workbox/';

/// Well-formed SRI metadata that matches no fixture, so the fetch fails integrity.
const _wrongIntegrity = 'sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=';

extension type NativeEvent._(JSObject _) implements JSObject {
  external String get type;
  external void waitUntil(JSPromise<JSAny?> promise);
}

extension type PluginSpec._(JSObject _) implements JSObject {
  external String get kind;
  external String get label;
  external String get on;
  external JSAny? get value;
  external String get url;
  external String get controller;
}

extension type HandlerSpec._(JSObject _) implements JSObject {
  external String? get strategy;
  external String get cacheName;
  external JSArray<PluginSpec>? get plugins;
  external String? get precache;
  external String? get bound;
  external String get controller;
  external JSAny? get respond;
  external JSAny? get status;
}

extension type RouteOptionsSpec._(JSObject _) implements JSObject {
  external JSAny? get directoryIndex;
  external JSAny? get cleanURLs;
  external JSArray<JSString>? get ignoreURLParametersMatching;
  external JSArray<JSString>? get urlManipulation;
}

extension type Step._(JSObject _) implements JSObject {
  external String get op;
  external String? get id;
  external String get source;
  external String get cacheName;
  external JSArray<PluginSpec>? get plugins;
  external JSAny? get fallbackToNetwork;
  external String get controller;
  external JSArray<JSAny?> get entries;
  external String get router;
  external String? get route;
  external RouteOptionsSpec? get options;
  external String? get prefix;
  external bool? get crossOrigin;
  external HandlerSpec get handler;
  external String? get method;
  external String? get url;
  external String? get mode;
  external String? get integrity;
  external JSAny? get request;
}

extension type Input._(JSObject _) implements JSObject {
  external JSArray<Step> get steps;
  external String? get cachePrefix;
  external NativeEvent? get event;
  external Request? get request;
}

/// One handler's trace: the trace plugin's label, the request and the records of its callbacks.
class _HandlerTrace {
  _HandlerTrace(this.label, this.url);

  final String label;
  final String url;
  final log = <JSArray<JSAny?>>[];
}

/// The stand-in ExtendableEvent of one event-driven step: it collects its lifetime promises.
class _StandInEvent implements ExtendableEvent {
  _StandInEvent(this.type);

  @override
  final String type;
  final pending = <Future<Object?>>[];

  @override
  void waitUntil(Future<Object?> promise) => pending.add(promise);
}

/// A native event the caller supplied (service-worker mode).
class _HostEvent implements ExtendableEvent {
  _HostEvent(this._event);

  final NativeEvent _event;

  @override
  String get type => _event.type;

  @override
  void waitUntil(Future<Object?> promise) => _event.waitUntil(promise.then((_) => null).toJS);
}

/// A response as a followed redirect leaves it: it and every clone of it report redirected.
Response _redirected(Response response) {
  final clone = (response['clone'] as JSFunction).callMethod<JSFunction>('bind'.toJS, response);
  defineProperty(response, 'redirected', JSObject()..['value'] = true.toJS);
  defineProperty(response, 'clone', JSObject()..['value'] = (() => _redirected(clone.callAsFunction() as Response)).toJS);
  return response;
}

JSNumber? _status(Response? response) => response?.status.toJS;

/// The adapter's urlManipulation variant: `new URL(url.href)` with `pathname += suffix`.
URL _withSuffix(URL url, String suffix) {
  final variant = URL.from(url.href);
  variant.pathname = variant.pathname + suffix;
  return variant;
}

/// An Error's name; a thrown non-Error (the fail fixture's `value`) prints as its string.
String _nameOf(Object error) => switch (error) {
      WorkboxError(:final name) => name,
      final JSObject value when isError(value) => hostErrorName(value),
      // A thrown string prints as itself, as `String(value)` prints it.
      _ => error.toString(),
    };

/// A manifest entry from the corpus: falsy, a string, or a `PrecacheEntry` object.
PrecacheEntry _precacheEntry(JSAny? value) {
  if (!isTruthy(value)) return FalsyEntry(value);
  if (value.isA<JSString>()) return UrlEntry((value as JSString).toDart);
  final entry = value as JSObject;
  String? text(String name) => (entry[name] as JSString?)?.toDart;
  return ObjectEntry(text('url'), entry['revision'], text('integrity'), entry);
}

class _Scenario {
  _Scenario(this._input)
      : _base = URL.from('./', location.href).href,
        _prefix = _input.cachePrefix ?? 'mires-workbox-${crypto.randomUUID()}',
        _owned = _input.cachePrefix == null;

  final Input _input;
  final String _base;
  final String _prefix;
  final bool _owned;
  var _source = 'alpha';
  final _controllers = <String, PrecacheController>{};
  final _routers = <String, Router>{};
  final _routes = <String, Route>{};
  final _results = <JSAny?>[];
  final _events = <JSAny?>[];
  final _handlers = <_HandlerTrace>[];

  String _relative(String url) => url.startsWith(_base) ? url.substring(_base.length) : url;

  String _cacheName(String name) => '$_prefix-$name';

  String _cacheLabel(String name) => name.startsWith('$_prefix-') ? name.substring(_prefix.length + 1) : name;

  /// Runs every step, reports, and deletes the scenario's caches whatever happened.
  Future<JSObject> run() async {
    try {
      for (final step in _input.steps.toDart) {
        JSAny? result;
        try {
          result = await _step(step);
        } catch (error) {
          result = _failure(error);
        }
        _results.add(result);
      }
      final caches = await _dump();
      final traces = [for (final trace in _handlers) [trace.label.toJS, trace.url.toJS, trace.log.toJS].toJS];
      return JSObject()
        ..['results'] = _results.toJS
        ..['events'] = _events.toJS
        ..['handlers'] = traces.toJS
        ..['caches'] = caches;
    } finally {
      if (_owned) {
        for (final name in (await caches.keys().toDart).toDart) {
          if (name.toDart.startsWith('$_prefix-')) await caches.delete(name.toDart).toDart;
        }
      }
    }
  }

  Future<JSAny?> _step(Step step) async {
    switch (step.op) {
      case 'network':
        _source = step.source;
        return null;
      case 'controller':
        final fallbackToNetwork = step.fallbackToNetwork.dartify() != false;
        _controllers[step.id!] = PrecacheController(_cacheName(step.cacheName), plugins: _plugins(step.plugins), fallbackToNetwork: fallbackToNetwork);
        return null;
      case 'addToCacheList':
        _controller(step.controller).addToCacheList([for (final entry in step.entries.toDart) _precacheEntry(entry)]);
        return null;
      case 'addPlugins':
        _controller(step.controller).strategy.plugins.addAll([for (final spec in step.plugins?.toDart ?? <PluginSpec>[]) _plugin(spec)]);
        return null;
      case 'install':
        final result = await _lifetime('install', (event) => _controller(step.controller).install(event));
        return JSObject()
          ..['updatedURLs'] = _urls(result.updatedURLs)
          ..['notUpdatedURLs'] = _urls(result.notUpdatedURLs);
      case 'activate':
        final result = await _lifetime('activate', (event) => _controller(step.controller).activate(event));
        return JSObject()..['deletedURLs'] = _urls(result.deletedURLs);
      case 'getCachedURLs':
        return _urls(_controller(step.controller).getCachedURLs());
      case 'getURLsToCacheKeys':
        return [
          for (final entry in _controller(step.controller).getURLsToCacheKeys().entries)
            [_relative(entry.key).toJS, _relative(entry.value).toJS].toJS,
        ].toJS;
      case 'getCacheKeyForURL':
        final key = _controller(step.controller).getCacheKeyForURL(step.url!);
        return key == null ? null : _relative(key).toJS;
      case 'matchPrecache':
        final url = step.url!;
        final Object request = isTruthy(step.request) ? await hostCall(() => Request(url.toJS)) : url;
        final response = await _controller(step.controller).matchPrecache(request);
        return response == null ? null : _record(response);
      case 'router':
        _routers[step.id!] = Router();
        return null;
      case 'precacheRoute':
        return _register(step, PrecacheRoute(_controller(step.controller), _routeOptions(step.options)));
      case 'route':
        final prefix = step.prefix;
        final bool Function(MatchCallbackOptions) match =
            step.crossOrigin == true ? (options) => !options.sameOrigin : (options) => _relative(options.url.href).startsWith(prefix!);
        return _register(step, Route(match, _handler(step.handler), step.method ?? 'GET'));
      case 'defaultHandler':
        _router(step.router).setDefaultHandler(_handler(step.handler), step.method ?? 'GET');
        return null;
      case 'catchHandler':
        final route = step.route;
        if (route == null) {
          _router(step.router).setCatchHandler(_handler(step.handler));
        } else {
          _routes[route]!.setCatchHandler(_handler(step.handler));
        }
        return null;
      case 'unregisterRoute':
        _router(step.router).unregisterRoute(_routes[step.route!]!);
        return null;
      case 'fetch':
        final url = step.url;
        final request = url == null
            ? _input.request!
            : await hostCall(() => Request(url.toJS, requestInit(method: step.method, mode: step.mode, integrity: step.integrity)));
        final response = await _lifetime('fetch', (event) async {
          final responsePromise = _router(step.router).handleRequest(request, event);
          return responsePromise == null ? null : await responsePromise;
        });
        return response == null ? null : _record(response);
      case 'caches':
        return _dump();
    }
    throw HostTypeError('unknown step ${step.op}');
  }

  PrecacheController _controller(String id) => _controllers[id]!;

  Router _router(String id) => _routers[id]!;

  JSAny? _register(Step step, Route route) {
    final id = step.id;
    if (id != null) _routes[id] = route;
    _router(step.router).registerRoute(route);
    return null;
  }

  JSArray<JSString> _urls(List<String> urls) => [for (final url in urls) _relative(url).toJS].toJS;

  /// The network fixture, then the plugins the script names.
  List<WorkboxPlugin> _plugins(JSArray<PluginSpec>? specs) => [_NetworkPlugin(this), for (final spec in specs?.toDart ?? <PluginSpec>[]) _plugin(spec)];

  WorkboxPlugin _plugin(PluginSpec spec) => switch (spec.kind) {
        'trace' => _TracePlugin(this, spec.label),
        'keep' => _CacheabilityPlugin(keep: true),
        'drop' => _CacheabilityPlugin(keep: false),
        'fail' => _FailPlugin(spec.on, spec.value),
        'fallback' => PrecacheFallbackPlugin(fallbackURL: spec.url, precacheController: _controller(spec.controller)),
        final kind => throw HostTypeError('unknown plugin $kind'),
      };

  /// A route handler: a strategy, a controller's precache strategy, a handler bound to a precached
  /// URL, or an async function answering a fixed response.
  RouteHandler _handler(HandlerSpec spec) {
    final strategy = spec.strategy;
    if (strategy != null) {
      final cacheName = _cacheName(spec.cacheName);
      final plugins = _plugins(spec.plugins);
      return switch (strategy) {
        'CacheFirst' => CacheFirst(cacheName, plugins),
        'CacheOnly' => CacheOnly(cacheName, plugins),
        'NetworkFirst' => NetworkFirst(cacheName, plugins),
        'NetworkOnly' => NetworkOnly(cacheName, plugins),
        _ => throw HostTypeError('unknown strategy $strategy'),
      };
    }
    final precache = spec.precache;
    if (precache != null) return _controller(precache).strategy;
    final bound = spec.bound;
    if (bound != null) return _controller(spec.controller).createHandlerBoundToURL(bound);
    return CallbackHandler((_) => hostCall(() => Response(spec.respond, JSObject()..['status'] = spec.status)));
  }

  /// PrecacheRoute options as given: an absent member keeps the default, `null` disables
  /// `directoryIndex` and `cleanURLs`.
  PrecacheRouteOptions? _routeOptions(RouteOptionsSpec? options) {
    if (options == null) return null;
    final defaults = PrecacheRouteOptions();
    final suffixes = options.urlManipulation?.toDart.map((suffix) => suffix.toDart).toList();
    return PrecacheRouteOptions(
      directoryIndex: options.directoryIndex.isUndefined ? defaults.directoryIndex : (options.directoryIndex as JSString?)?.toDart,
      ignoreURLParametersMatching: options.ignoreURLParametersMatching?.toDart.map((source) => HostRegExp(source.toDart)).toList(),
      cleanURLs: options.cleanURLs.isUndefined ? defaults.cleanURLs : isTruthy(options.cleanURLs),
      urlManipulation: suffixes == null ? null : (url) => [for (final suffix in suffixes) _withSuffix(url, suffix)],
    );
  }

  /// Runs one event-driven step under a fresh stand-in event and waits for every lifetime it was
  /// given, in rounds, or, in service-worker mode, under the caller's native event.
  Future<T> _lifetime<T>(String type, Future<T> Function(ExtendableEvent event) action) async {
    final native = _input.event;
    if (native != null) return action(_HostEvent(native));
    final event = _StandInEvent(type);
    var count = 0;
    try {
      return await action(event);
    } finally {
      while (event.pending.isNotEmpty) {
        final batch = [...event.pending];
        event.pending.clear();
        count += batch.length;
        await Future.wait([for (final promise in batch) promise.then((_) => null, onError: (Object _) => null)]);
      }
      _events.add([type.toJS, count.toJS].toJS);
    }
  }

  /// `{status, url, body}` of a response, reading its body.
  Future<JSObject> _record(Response response) async {
    final status = response.status;
    final url = _relative(response.url);
    final body = await response.text().toDart;
    return JSObject()
      ..['status'] = status.toJS
      ..['url'] = url.toJS
      ..['body'] = body;
  }

  /// `{error, message}` for a WorkboxError, `{error}` for another Error, `{thrown}` for a thrown
  /// non-Error.
  JSObject _failure(Object error) => switch (error) {
        WorkboxError(:final name, :final message) => JSObject()
          ..['error'] = name.toJS
          ..['message'] = message.split(_base).join().split('$_prefix-').join().toJS,
        _ when isError(error) => JSObject()..['error'] = _nameOf(error).toJS,
        _ => JSObject()..['thrown'] = _nameOf(error).toJS,
      };

  /// Every cache of the scenario, in CacheStorage order, with each entry's response record.
  Future<JSArray<JSAny?>> _dump() async {
    final out = <JSAny?>[];
    for (final name in (await caches.keys().toDart).toDart) {
      if (!name.toDart.startsWith('$_prefix-')) continue;
      final cache = await caches.open(name.toDart).toDart;
      final entries = <JSAny?>[];
      for (final request in (await cache.keys().toDart).toDart) {
        final response = await cache.matching(request).toDart;
        entries.add([_relative(request.url).toJS, await _record(response!)].toJS);
      }
      out.add([_cacheLabel(name.toDart).toJS, entries.toJS].toJS);
    }
    return out.toJS;
  }
}

/// The network fixture: every fetch goes to one of the two fixture files, or fails.
class _NetworkPlugin extends WorkboxPlugin {
  _NetworkPlugin(this._scenario);

  final _Scenario _scenario;

  @override
  Callback<RequestWillFetchParam, Request> get requestWillFetch => (param) async {
        final source = _scenario._source;
        if (source == 'offline') throw HostTypeError('offline');
        final target = URL.from('$_fixture${source == 'beta' ? 'beta' : 'alpha'}.mjs', location.href).href;
        return Request(target.toJS, requestInit(integrity: source == 'broken' ? _wrongIntegrity : param.request.integrity));
      };

  @override
  Callback<FetchDidSucceedParam, Response> get fetchDidSucceed => (param) async => switch (_scenario._source) {
        'missing' => Response('missing'.toJS, JSObject()..['status'] = 404.toJS),
        'redirected' => _redirected(param.response),
        _ => param.response,
      };
}

/// The trace plugin: every lifecycle callback but cacheWillUpdate, recorded per handler. A callback
/// records only into a log its own handlerWillStart created: a plugin before it can fail that
/// callback.
class _TracePlugin extends WorkboxPlugin {
  _TracePlugin(this._scenario, this._label);

  final _Scenario _scenario;
  final String _label;

  String _url(Request request) => _scenario._relative(request.url);

  void _record(PluginState? state, List<JSAny?> entry) {
    final trace = state?['log'];
    if (trace is _HandlerTrace) trace.log.add(entry.toJS);
  }

  @override
  Callback<HandlerWillStartParam, void> get handlerWillStart => (param) async {
        final trace = _HandlerTrace(_label, _url(param.request));
        param.state?['log'] = trace;
        _scenario._handlers.add(trace);
      };

  @override
  Callback<CacheKeyWillBeUsedParam, Request> get cacheKeyWillBeUsed => (param) async {
        _record(param.state, ['cacheKeyWillBeUsed'.toJS, param.mode.toJS, _url(param.request).toJS]);
        return param.request;
      };

  @override
  Callback<CachedResponseWillBeUsedParam, Response?> get cachedResponseWillBeUsed => (param) async {
        _record(param.state, [
          'cachedResponseWillBeUsed'.toJS,
          _scenario._cacheLabel(param.cacheName).toJS,
          _url(param.request).toJS,
          _status(param.cachedResponse),
        ]);
        return param.cachedResponse;
      };

  @override
  Callback<RequestWillFetchParam, Request> get requestWillFetch => (param) async {
        _record(param.state, ['requestWillFetch'.toJS, _url(param.request).toJS]);
        return param.request;
      };

  @override
  Callback<FetchDidSucceedParam, Response> get fetchDidSucceed => (param) async {
        _record(param.state, ['fetchDidSucceed'.toJS, _url(param.request).toJS, param.response.status.toJS]);
        return param.response;
      };

  @override
  Callback<FetchDidFailParam, void> get fetchDidFail => (param) async {
        _record(param.state, ['fetchDidFail'.toJS, _url(param.originalRequest).toJS, _url(param.request).toJS, _nameOf(param.error).toJS]);
      };

  @override
  Callback<CacheDidUpdateParam, void> get cacheDidUpdate => (param) async {
        _record(param.state, [
          'cacheDidUpdate'.toJS,
          _scenario._cacheLabel(param.cacheName).toJS,
          _url(param.request).toJS,
          _status(param.oldResponse),
          param.newResponse.status.toJS,
        ]);
      };

  @override
  Callback<HandlerWillRespondParam, Response> get handlerWillRespond => (param) async {
        _record(param.state, ['handlerWillRespond'.toJS, param.response.status.toJS]);
        return param.response;
      };

  @override
  Callback<HandlerDidRespondParam, void> get handlerDidRespond => (param) async {
        _record(param.state, ['handlerDidRespond'.toJS, _status(param.response)]);
      };

  @override
  Callback<HandlerDidCompleteParam, void> get handlerDidComplete => (param) async {
        final error = param.error;
        _record(param.state, ['handlerDidComplete'.toJS, _status(param.response), error == null ? null : _nameOf(error).toJS]);
      };

  @override
  Callback<HandlerDidErrorParam, Response?> get handlerDidError => (param) async {
        _record(param.state, ['handlerDidError'.toJS, _nameOf(param.error).toJS]);
        return null;
      };
}

/// A cacheWillUpdate that accepts every response (`keep`) or none (`drop`).
class _CacheabilityPlugin extends WorkboxPlugin {
  _CacheabilityPlugin({required this.keep});

  final bool keep;

  @override
  Callback<CacheWillUpdateParam, Response?> get cacheWillUpdate => (param) async => keep ? param.response : null;
}

/// A plugin whose one callback, the one named [_on], throws `new Error('fail')`, or, with a value,
/// that value itself (a non-Error).
class _FailPlugin extends WorkboxPlugin {
  _FailPlugin(this._on, this._value);

  final String _on;
  final JSAny? _value;

  Callback<P, R>? _failing<P, R>(String name) =>
      name != _on ? null : (_) async => throw (_value.isUndefined ? HostError('fail') : _value as Object);

  @override
  Callback<HandlerWillStartParam, void>? get handlerWillStart => _failing('handlerWillStart');
  @override
  Callback<CacheKeyWillBeUsedParam, Request>? get cacheKeyWillBeUsed => _failing('cacheKeyWillBeUsed');
  @override
  Callback<CachedResponseWillBeUsedParam, Response?>? get cachedResponseWillBeUsed => _failing('cachedResponseWillBeUsed');
  @override
  Callback<CacheWillUpdateParam, Response?>? get cacheWillUpdate => _failing('cacheWillUpdate');
  @override
  Callback<CacheDidUpdateParam, void>? get cacheDidUpdate => _failing('cacheDidUpdate');
  @override
  Callback<RequestWillFetchParam, Request>? get requestWillFetch => _failing('requestWillFetch');
  @override
  Callback<FetchDidSucceedParam, Response>? get fetchDidSucceed => _failing('fetchDidSucceed');
  @override
  Callback<FetchDidFailParam, void>? get fetchDidFail => _failing('fetchDidFail');
  @override
  Callback<HandlerWillRespondParam, Response>? get handlerWillRespond => _failing('handlerWillRespond');
  @override
  Callback<HandlerDidRespondParam, void>? get handlerDidRespond => _failing('handlerDidRespond');
  @override
  Callback<HandlerDidCompleteParam, void>? get handlerDidComplete => _failing('handlerDidComplete');
  @override
  Callback<HandlerDidErrorParam, Response?>? get handlerDidError => _failing('handlerDidError');
}

/// Runs one corpus script and resolves with its output.
JSPromise<JSObject> run(Input input) => _Scenario(input).run().toJS;

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
