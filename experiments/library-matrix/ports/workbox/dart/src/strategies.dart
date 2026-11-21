// workbox-strategies: the plugin interface, Strategy with its handle/handleAll lifecycle,
// StrategyHandler, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.

part of 'workbox.dart';

/// The state object a handler keeps for one plugin (`state` in every callback), as Workbox's
/// `MapLikeObject`.
typedef PluginState = Map<String, Object?>;

/// The route parameters a handler receives: `{cacheKey, integrity}` from a PrecacheRoute match,
/// `{cacheKey}` from createHandlerBoundToURL.
class Params {
  const Params({this.cacheKey, this.integrity});

  final String? cacheKey;
  final String? integrity;
}

class HandlerWillStartParam {
  HandlerWillStartParam(this.request, this.event, this.state);
  final Request request;
  final ExtendableEvent event;
  final PluginState? state;
}

class CacheKeyWillBeUsedParam {
  CacheKeyWillBeUsedParam(this.mode, this.request, this.event, this.params, this.state);
  final String mode;
  final Request request;
  final ExtendableEvent event;
  final Params? params;
  final PluginState? state;
}

class CachedResponseWillBeUsedParam {
  CachedResponseWillBeUsedParam(this.cacheName, this.request, this.cachedResponse, this.event, this.state);
  final String cacheName;
  final Request request;
  final Response? cachedResponse;
  final ExtendableEvent event;
  final PluginState? state;
}

class CacheWillUpdateParam {
  CacheWillUpdateParam(this.request, this.response, this.event, this.state);
  final Request request;
  final Response response;
  final ExtendableEvent event;
  final PluginState? state;
}

class CacheDidUpdateParam {
  CacheDidUpdateParam(this.cacheName, this.oldResponse, this.newResponse, this.request, this.event, this.state);
  final String cacheName;
  final Response? oldResponse;
  final Response newResponse;
  final Request request;
  final ExtendableEvent event;
  final PluginState? state;
}

class RequestWillFetchParam {
  RequestWillFetchParam(this.request, this.event, this.state);
  final Request request;
  final ExtendableEvent event;
  final PluginState? state;
}

class FetchDidSucceedParam {
  FetchDidSucceedParam(this.request, this.response, this.event, this.state);
  final Request request;
  final Response response;
  final ExtendableEvent event;
  final PluginState? state;
}

class FetchDidFailParam {
  FetchDidFailParam(this.error, this.originalRequest, this.request, this.event, this.state);
  final Object error;
  final Request originalRequest;
  final Request request;
  final ExtendableEvent event;
  final PluginState? state;
}

class HandlerWillRespondParam {
  HandlerWillRespondParam(this.request, this.response, this.event, this.state);
  final Request request;
  final Response response;
  final ExtendableEvent event;
  final PluginState? state;
}

class HandlerDidRespondParam {
  HandlerDidRespondParam(this.request, this.response, this.event, this.state);
  final Request request;
  final Response? response;
  final ExtendableEvent event;
  final PluginState? state;
}

class HandlerDidCompleteParam {
  HandlerDidCompleteParam(this.request, this.response, this.error, this.event, this.state);
  final Request request;
  final Response? response;
  final Object? error;
  final ExtendableEvent event;
  final PluginState? state;
}

class HandlerDidErrorParam {
  HandlerDidErrorParam(this.request, this.error, this.event, this.state);
  final Request request;
  final Object error;
  final ExtendableEvent event;
  final PluginState? state;
}

typedef Callback<P, R> = Future<R> Function(P param);

/// `WorkboxPlugin`: an object of optional lifecycle callbacks. A plugin defines a callback by
/// overriding its getter; Workbox calls only the callbacks a plugin defines.
abstract class WorkboxPlugin {
  Callback<HandlerWillStartParam, void>? get handlerWillStart => null;
  Callback<CacheKeyWillBeUsedParam, Request>? get cacheKeyWillBeUsed => null;
  Callback<CachedResponseWillBeUsedParam, Response?>? get cachedResponseWillBeUsed => null;
  Callback<CacheWillUpdateParam, Response?>? get cacheWillUpdate => null;
  Callback<CacheDidUpdateParam, void>? get cacheDidUpdate => null;
  Callback<RequestWillFetchParam, Request>? get requestWillFetch => null;
  Callback<FetchDidSucceedParam, Response>? get fetchDidSucceed => null;
  Callback<FetchDidFailParam, void>? get fetchDidFail => null;
  Callback<HandlerWillRespondParam, Response>? get handlerWillRespond => null;
  Callback<HandlerDidRespondParam, void>? get handlerDidRespond => null;
  Callback<HandlerDidCompleteParam, void>? get handlerDidComplete => null;
  Callback<HandlerDidErrorParam, Response?>? get handlerDidError => null;
}

/// `HandlerCallbackOptions`: what a route handler is called with.
class HandlerCallbackOptions {
  HandlerCallbackOptions(this.request, this.event, [this.params]);

  final Request request;
  final ExtendableEvent event;
  final Params? params;
}

/// A route handler as `normalizeHandler` leaves it: an object with `handle`.
abstract interface class RouteHandler {
  Future<Response> handle(HandlerCallbackOptions options);
}

/// A handler callback, as `normalizeHandler` wraps a function.
class CallbackHandler implements RouteHandler {
  CallbackHandler(this._callback);

  final Future<Response> Function(HandlerCallbackOptions options) _callback;

  @override
  Future<Response> handle(HandlerCallbackOptions options) => _callback(options);
}

/// `Strategy`: a cache name, a mutable plugin list and the subclass's `_handle`. The cache name is
/// always supplied (`cacheNames.getRuntimeName(cacheName)` returns it unchanged).
abstract class Strategy implements RouteHandler {
  Strategy(this.cacheName, [List<WorkboxPlugin> plugins = const []]) : plugins = [...plugins];

  final String cacheName;
  final List<WorkboxPlugin> plugins;

  /// `handle(options)`: the response promise of `handleAll`; the completion keeps running.
  @override
  Future<Response> handle(HandlerCallbackOptions options) => handleAll(options).$1;

  /// `handleAll(options)`: a handler for the request, and its response and completion promises,
  /// both already running.
  (Future<Response>, Future<void>) handleAll(HandlerCallbackOptions options) {
    final handler = StrategyHandler._(this, options);
    final responseDone = _getResponse(handler, options.request, options.event);
    final handlerDone = _awaitComplete(responseDone, handler, options.request, options.event);
    return (responseDone, handlerDone);
  }

  Future<Response> _handle(Request request, StrategyHandler handler);

  /// `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond.
  Future<Response> _getResponse(StrategyHandler handler, Request request, ExtendableEvent event) async {
    for (final (callback, state) in handler.iterateCallbacks((plugin) => plugin.handlerWillStart)) {
      await callback(HandlerWillStartParam(request, event, state));
    }
    Response? response;
    try {
      response = await _handle(request, handler);
      // A response of type "error" stays the response unless a handlerDidError callback replaces
      // it, so it is returned when no callback does.
      if (response.responseType == 'error') throw WorkboxError('no-response', JSObject()..['url'] = request.url.toJS);
    } catch (error) {
      if (isError(error)) {
        for (final (callback, state) in handler.iterateCallbacks((plugin) => plugin.handlerDidError)) {
          response = await callback(HandlerDidErrorParam(request, error, event, state));
          if (response != null) break;
        }
      }
      if (response == null) rethrow;
    }
    for (final (callback, state) in handler.iterateCallbacks((plugin) => plugin.handlerWillRespond)) {
      response = await callback(HandlerWillRespondParam(request, response!, event, state));
    }
    return response!;
  }

  /// `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
  /// handlerDidComplete with the Error among them, then the handler's own promise resolves.
  Future<void> _awaitComplete(Future<Response> responseDone, StrategyHandler handler, Request request, ExtendableEvent event) async {
    Response? response;
    Object? error;
    try {
      response = await responseDone;
    } catch (_) {
      // The caller sees this failure; completion goes on without a response.
    }
    try {
      for (final (callback, state) in handler.iterateCallbacks((plugin) => plugin.handlerDidRespond)) {
        await callback(HandlerDidRespondParam(request, response, event, state));
      }
      await handler.doneWaiting();
    } catch (waitUntilError) {
      if (isError(waitUntilError)) error = waitUntilError;
    }
    for (final (callback, state) in handler.iterateCallbacks((plugin) => plugin.handlerDidComplete)) {
      await callback(HandlerDidCompleteParam(request, response, error, event, state));
    }
    handler.destroy();
    if (error != null) throw error;
  }
}

/// `StrategyHandler`: one request's run through a strategy.
class StrategyHandler {
  StrategyHandler._(this._strategy, HandlerCallbackOptions options)
      : request = options.request,
        event = options.event,
        params = options.params {
    // The plugins present now get a state; a plugin added later is called without one.
    for (final plugin in _strategy.plugins) {
      _pluginStateMap[plugin] = <String, Object?>{};
    }
    event.waitUntil(_handlerDeferred.future);
  }

  final Strategy _strategy;
  final Request request;
  final ExtendableEvent event;
  final Params? params;
  final _cacheKeys = <String, Request>{};
  final _handlerDeferred = Completer<void>();
  final _extendLifetimePromises = <Future<Object?>>[];
  final _pluginStateMap = Map<WorkboxPlugin, PluginState>.identity();

  /// `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed or fetchDidFail.
  Future<Response> fetch(Request input) async {
    var request = input;
    final originalRequest = hasCallback((plugin) => plugin.fetchDidFail) ? request.copy() : null;
    try {
      for (final (callback, state) in iterateCallbacks((plugin) => plugin.requestWillFetch)) {
        request = await callback(RequestWillFetchParam(request.copy(), event, state));
      }
    } catch (error) {
      // A thrown non-Error ends the loop, and the request goes out as it is.
      if (isError(error)) {
        final message = switch (error) {
          WorkboxError() => error.message.toJS,
          final JSObject value => hostErrorMessage(value),
          _ => error.toString().toJS,
        };
        throw WorkboxError('plugin-error-request-will-fetch', JSObject()..['thrownErrorMessage'] = message);
      }
    }
    final pluginFilteredRequest = request.copy();
    try {
      var fetchResponse = await hostFetch(request).toDart;
      for (final (callback, state) in iterateCallbacks((plugin) => plugin.fetchDidSucceed)) {
        fetchResponse = await callback(FetchDidSucceedParam(pluginFilteredRequest, fetchResponse, event, state));
      }
      return fetchResponse;
    } catch (error) {
      if (originalRequest != null) {
        // One clone of each for every callback, as runCallbacks passes one param object.
        final original = originalRequest.copy();
        final filtered = pluginFilteredRequest.copy();
        for (final (callback, state) in iterateCallbacks((plugin) => plugin.fetchDidFail)) {
          await callback(FetchDidFailParam(error, original, filtered, event, state));
        }
      }
      rethrow;
    }
  }

  /// `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime.
  Future<Response> fetchAndCachePut(Request input) async {
    final response = await fetch(input);
    final responseClone = response.copy();
    waitUntil(cachePut(input, responseClone));
    return response;
  }

  /// `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache (which opens
  /// no cache), through cachedResponseWillBeUsed.
  Future<Response?> cacheMatch(Request key) async {
    final effectiveRequest = await getCacheKey(key, 'read');
    final options = JSObject()..['cacheName'] = _strategy.cacheName.toJS;
    var cachedResponse = await caches.matching(effectiveRequest, options).toDart;
    for (final (callback, state) in iterateCallbacks((plugin) => plugin.cachedResponseWillBeUsed)) {
      cachedResponse = await callback(CachedResponseWillBeUsedParam(_strategy.cacheName, effectiveRequest, cachedResponse, event, state));
    }
    return cachedResponse;
  }

  /// `cachePut(key, response)`: one timer task later, the write cache key, cacheWillUpdate (or the
  /// status-200 rule), the put, then cacheDidUpdate. Answers whether it cached.
  Future<bool> cachePut(Request key, Response response) async {
    await timeout(0);
    final effectiveRequest = await getCacheKey(key, 'write');
    final responseToCache = await _ensureResponseSafeToCache(response);
    if (responseToCache == null) return false;
    final cache = await caches.open(_strategy.cacheName).toDart;
    final hasCacheUpdateCallback = hasCallback((plugin) => plugin.cacheDidUpdate);
    final oldResponse =
        hasCacheUpdateCallback ? await cacheMatchIgnoreParams(cache, effectiveRequest.copy(), ['__WB_REVISION__']) : null;
    try {
      await cache.put(effectiveRequest, hasCacheUpdateCallback ? responseToCache.copy() : responseToCache).toDart;
    } catch (error) {
      // An Error is rethrown (a QuotaExceededError after the registered quota callbacks, of which
      // the subset has none); anything else is ignored.
      if (isError(error)) rethrow;
    }
    for (final (callback, state) in iterateCallbacks((plugin) => plugin.cacheDidUpdate)) {
      await callback(CacheDidUpdateParam(_strategy.cacheName, oldResponse, responseToCache.copy(), effectiveRequest, event, state));
    }
    return true;
  }

  /// `getCacheKey(request, mode)`: cacheKeyWillBeUsed once per URL and mode for this handler.
  Future<Request> getCacheKey(Request request, String mode) async {
    final key = '${request.url} | $mode';
    final known = _cacheKeys[key];
    if (known != null) return known;
    var effectiveRequest = request;
    for (final (callback, state) in iterateCallbacks((plugin) => plugin.cacheKeyWillBeUsed)) {
      effectiveRequest = await callback(CacheKeyWillBeUsedParam(mode, effectiveRequest, event, params, state));
    }
    return _cacheKeys[key] = effectiveRequest;
  }

  /// `hasCallback(name)`, over the strategy's current plugins.
  bool hasCallback(Object? Function(WorkboxPlugin plugin) callback) => _strategy.plugins.any((plugin) => callback(plugin) != null);

  /// `iterateCallbacks(name)`: a generator over the strategy's plugins that define the callback.
  /// It reads the live list at every step, so a plugin added meanwhile is reached (without state),
  /// as Workbox's generator over `this._strategy.plugins` reaches it.
  Iterable<(T, PluginState?)> iterateCallbacks<T extends Object>(T? Function(WorkboxPlugin plugin) callback) sync* {
    for (var index = 0; index < _strategy.plugins.length; index++) {
      final plugin = _strategy.plugins[index];
      final each = callback(plugin);
      if (each != null) yield (each, _pluginStateMap[plugin]);
    }
  }

  /// `waitUntil(promise)`: extends the handler's lifetime until the promise settles.
  Future<T> waitUntil<T>(Future<T> promise) {
    _extendLifetimePromises.add(promise);
    return promise;
  }

  /// `doneWaiting()`: settles every extended lifetime, including ones added meanwhile, and throws
  /// the first rejection of a batch.
  Future<void> doneWaiting() async {
    while (_extendLifetimePromises.isNotEmpty) {
      final batch = [..._extendLifetimePromises];
      _extendLifetimePromises.clear();
      final outcomes = await Future.wait([
        for (final promise in batch) promise.then<(Object,)?>((_) => null, onError: (Object error) => (error,)),
      ]);
      final rejected = outcomes.nonNulls.firstOrNull;
      if (rejected != null) throw rejected.$1;
    }
  }

  void destroy() => _handlerDeferred.complete();

  /// `_ensureResponseSafeToCache(response)`: every cacheWillUpdate in turn until one refuses, or,
  /// without any, only a status 200.
  Future<Response?> _ensureResponseSafeToCache(Response response) async {
    Response? responseToCache = response;
    var pluginsUsed = false;
    for (final (callback, state) in iterateCallbacks((plugin) => plugin.cacheWillUpdate)) {
      responseToCache = await callback(CacheWillUpdateParam(request, responseToCache!, event, state));
      pluginsUsed = true;
      if (responseToCache == null) break;
    }
    if (!pluginsUsed && responseToCache != null && responseToCache.status != 200) responseToCache = null;
    return responseToCache;
  }
}

/// `new WorkboxError('no-response', {url, error})`; the caught value is kept only when it is an
/// Error (`error` is then left out of the details).
WorkboxError _noResponse(Request request, [Object? error]) {
  final details = JSObject()..['url'] = request.url.toJS;
  if (error != null) details['error'] = asDetail(error);
  return WorkboxError('no-response', details);
}

class CacheFirst extends Strategy {
  CacheFirst(super.cacheName, [super.plugins]);

  @override
  Future<Response> _handle(Request request, StrategyHandler handler) async {
    Object? error;
    var response = await handler.cacheMatch(request);
    if (response == null) {
      try {
        response = await handler.fetchAndCachePut(request);
      } catch (caught) {
        if (isError(caught)) error = caught;
      }
    }
    return response ?? (throw _noResponse(request, error));
  }
}

class CacheOnly extends Strategy {
  CacheOnly(super.cacheName, [super.plugins]);

  @override
  Future<Response> _handle(Request request, StrategyHandler handler) async =>
      await handler.cacheMatch(request) ?? (throw _noResponse(request));
}

/// `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response.
class _CacheOkAndOpaquePlugin extends WorkboxPlugin {
  @override
  Callback<CacheWillUpdateParam, Response?> get cacheWillUpdate =>
      (param) async => param.response.status == 200 || param.response.status == 0 ? param.response : null;
}

final _cacheOkAndOpaquePlugin = _CacheOkAndOpaquePlugin();

/// `NetworkFirst` without `networkTimeoutSeconds`. Its constructor puts the 200-or-opaque plugin
/// first unless a plugin has a cacheWillUpdate.
class NetworkFirst extends Strategy {
  NetworkFirst(super.cacheName, [super.plugins]) {
    if (!plugins.any((plugin) => plugin.cacheWillUpdate != null)) plugins.insert(0, _cacheOkAndOpaquePlugin);
  }

  @override
  Future<Response> _handle(Request request, StrategyHandler handler) async {
    final networkPromise = _getNetworkPromise(request, handler);
    final promises = [networkPromise];
    final response = await handler.waitUntil((() async => await handler.waitUntil(Future.any(promises)) ?? await networkPromise)());
    return response ?? (throw _noResponse(request));
  }

  /// `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing.
  Future<Response?> _getNetworkPromise(Request request, StrategyHandler handler) async {
    Object? error;
    Response? response;
    try {
      response = await handler.fetchAndCachePut(request);
    } catch (caught) {
      if (isError(caught)) error = caught;
    }
    if (error != null || response == null) response = await handler.cacheMatch(request);
    return response;
  }
}

/// `NetworkOnly` without `networkTimeoutSeconds`.
class NetworkOnly extends Strategy {
  NetworkOnly(super.cacheName, [super.plugins]);

  @override
  Future<Response> _handle(Request request, StrategyHandler handler) async {
    Object? error;
    Response? response;
    try {
      response = await handler.fetch(request);
    } catch (caught) {
      if (isError(caught)) error = caught;
    }
    return response ?? (throw _noResponse(request, error));
  }
}
