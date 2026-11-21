// workbox-precaching: PrecacheController, PrecacheStrategy, PrecacheRoute with its URL variations,
// createCacheKey, and the cache-key, install-report and fallback plugins.

part of 'workbox.dart';

const _revisionSearchParam = '__WB_REVISION__';

/// A precache manifest entry as the caller wrote it (`PrecacheEntry | string`).
sealed class PrecacheEntry {
  const PrecacheEntry();
}

/// A string: a URL that is its own cache key.
class UrlEntry extends PrecacheEntry {
  const UrlEntry(this.url);
  final String url;
}

/// A `PrecacheEntry` object. [revision] stays the caller's value (a string, a number, null or
/// undefined): its truthiness decides, and the host prints a number. [source] is the object
/// itself, which add-to-cache-list-unexpected-type reports.
class ObjectEntry extends PrecacheEntry {
  const ObjectEntry(this.url, this.revision, this.integrity, this.source);
  final String? url;
  final JSAny? revision;
  final String? integrity;
  final JSObject source;
}

/// `null`, `''` or another falsy value.
class FalsyEntry extends PrecacheEntry {
  const FalsyEntry(this.source);
  final JSAny? source;
}

WorkboxError _unexpectedType(JSAny? entry) => WorkboxError('add-to-cache-list-unexpected-type', JSObject()..['entry'] = entry);

/// `createCacheKey(entry)`: the URL resolved against the location, and with a truthy revision a
/// cache key that carries it in `__WB_REVISION__`.
({String cacheKey, String url}) _createCacheKey(PrecacheEntry entry) {
  switch (entry) {
    case FalsyEntry(:final source):
      throw _unexpectedType(source);
    case UrlEntry(:final url):
      final href = URL.from(url, location.href).href;
      return (cacheKey: href, url: href);
    case ObjectEntry(:final url, :final revision, :final source):
      if (url == null || url.isEmpty) throw _unexpectedType(source);
      if (!isTruthy(revision)) {
        final href = URL.from(url, location.href).href;
        return (cacheKey: href, url: href);
      }
      final cacheKeyURL = URL.from(url, location.href);
      final originalURL = URL.from(url, location.href);
      cacheKeyURL.searchParams.set(_revisionSearchParam, revision);
      return (cacheKey: cacheKeyURL.href, url: originalURL.href);
  }
}

class InstallResult {
  InstallResult(this.updatedURLs, this.notUpdatedURLs);
  final List<String> updatedURLs;
  final List<String> notUpdatedURLs;
}

class CleanupResult {
  CleanupResult(this.deletedURLs);
  final List<String> deletedURLs;
}

/// `PrecacheController`.
class PrecacheController {
  PrecacheController(String cacheName, {List<WorkboxPlugin> plugins = const [], bool fallbackToNetwork = true}) {
    strategy = PrecacheStrategy(cacheName, [...plugins, PrecacheCacheKeyPlugin(this)], fallbackToNetwork: fallbackToNetwork);
  }

  late final PrecacheStrategy strategy;
  final _urlsToCacheKeys = <String, String>{};
  final _urlsToCacheModes = <String, String>{};
  final _cacheKeysToIntegrities = <String, String>{};

  /// `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added.
  /// Like the production build it warns about revisionless entries.
  void addToCacheList(List<PrecacheEntry> entries) {
    final urlsToWarnAbout = <String>[];
    for (final entry in entries) {
      switch (entry) {
        case UrlEntry(:final url):
          urlsToWarnAbout.add(url);
        case ObjectEntry(:final url, :final revision) when revision.isUndefined:
          urlsToWarnAbout.add(url ?? '');
        default:
      }
      final (:cacheKey, :url) = _createCacheKey(entry);
      final cacheMode = entry is ObjectEntry && isTruthy(entry.revision) ? 'reload' : 'default';
      final firstEntry = _urlsToCacheKeys[url];
      if (firstEntry != null && firstEntry != cacheKey) {
        throw WorkboxError(
          'add-to-cache-list-conflicting-entries',
          JSObject()
            ..['firstEntry'] = firstEntry.toJS
            ..['secondEntry'] = cacheKey.toJS,
        );
      }
      if (entry case ObjectEntry(:final integrity?) when integrity.isNotEmpty) {
        final known = _cacheKeysToIntegrities[cacheKey];
        if (known != null && known != integrity) {
          throw WorkboxError('add-to-cache-list-conflicting-integrities', JSObject()..['url'] = url.toJS);
        }
        _cacheKeysToIntegrities[cacheKey] = integrity;
      }
      _urlsToCacheKeys[url] = cacheKey;
      _urlsToCacheModes[url] = cacheMode;
      if (urlsToWarnAbout.isNotEmpty) {
        consoleWarn('Workbox is precaching URLs without revision info: ${urlsToWarnAbout.join(', ')}\n'
            'This is generally NOT safe. Learn more at https://bit.ly/wb-precache');
      }
    }
  }

  /// `install(event)`: precaches each URL in turn through the strategy, under an install report
  /// plugin, and extends the event's lifetime until it is done.
  Future<InstallResult> install(ExtendableEvent event) => waitUntil(event, () async {
        final installReportPlugin = PrecacheInstallReportPlugin();
        strategy.plugins.add(installReportPlugin);
        // Steps run one at a time, so the map cannot change during an install.
        for (final entry in [..._urlsToCacheKeys.entries]) {
          final (url, cacheKey) = (entry.key, entry.value);
          final integrity = _cacheKeysToIntegrities[cacheKey];
          final cacheMode = _urlsToCacheModes[url];
          final request = Request(url.toJS, requestInit(integrity: integrity, cache: cacheMode, credentials: 'same-origin'));
          final (responseDone, handlerDone) = strategy.handleAll(HandlerCallbackOptions(request, event, Params(cacheKey: cacheKey)));
          await Future.wait([responseDone, handlerDone], eagerError: true);
        }
        return InstallResult([...installReportPlugin.updatedURLs], [...installReportPlugin.notUpdatedURLs]);
      });

  /// `activate(event)`: deletes every request in the precache whose URL is no current cache key.
  Future<CleanupResult> activate(ExtendableEvent event) => waitUntil(event, () async {
        final cache = await caches.open(strategy.cacheName).toDart;
        final currentlyCachedRequests = await cache.keys().toDart;
        final expectedCacheKeys = {..._urlsToCacheKeys.values};
        final deletedURLs = <String>[];
        for (final request in currentlyCachedRequests.toDart) {
          if (!expectedCacheKeys.contains(request.url)) {
            await cache.delete(request).toDart;
            deletedURLs.add(request.url);
          }
        }
        return CleanupResult(deletedURLs);
      });

  Map<String, String> getURLsToCacheKeys() => _urlsToCacheKeys;

  List<String> getCachedURLs() => [..._urlsToCacheKeys.keys];

  /// `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations.
  String? getCacheKeyForURL(String url) => _urlsToCacheKeys[URL.from(url, location.href).href];

  String? getIntegrityForCacheKey(String cacheKey) => _cacheKeysToIntegrities[cacheKey];

  /// `matchPrecache(request)`: opens the precache only for a URL in the manifest. [request] is a
  /// URL string or a Request.
  Future<Response?> matchPrecache(Object request) async {
    final url = request is String ? request : (request as Request).url;
    final cacheKey = getCacheKeyForURL(url);
    if (cacheKey == null) return null;
    final cache = await caches.open(strategy.cacheName).toDart;
    return cache.matching(cacheKey.toJS).toDart;
  }

  /// `createHandlerBoundToURL(url)`: a handler that answers any request with this precached URL.
  RouteHandler createHandlerBoundToURL(String url) {
    final cacheKey = getCacheKeyForURL(url) ?? (throw WorkboxError('non-precached-url', JSObject()..['url'] = url.toJS));
    return CallbackHandler((options) {
      // `{cacheKey, ...options.params}`.
      final params = Params(cacheKey: options.params?.cacheKey ?? cacheKey, integrity: options.params?.integrity);
      return strategy.handle(HandlerCallbackOptions(Request(url.toJS), options.event, params));
    });
  }
}

/// The members of `new Request(input, init)` the subset uses; an absent one stays out of the
/// dictionary.
JSObject requestInit({String? method, String? mode, String? integrity, String? cache, String? credentials, Headers? headers}) {
  final init = JSObject();
  if (method != null) init['method'] = method.toJS;
  if (mode != null) init['mode'] = mode.toJS;
  if (integrity != null) init['integrity'] = integrity.toJS;
  if (cache != null) init['cache'] = cache.toJS;
  if (credentials != null) init['credentials'] = credentials.toJS;
  if (headers != null) init['headers'] = headers;
  return init;
}

/// `PrecacheStrategy`: a Strategy that answers from the precache, populates it during install,
/// and otherwise falls back to the network.
class PrecacheStrategy extends Strategy {
  PrecacheStrategy(super.cacheName, super.plugins, {bool fallbackToNetwork = true}) : _fallbackToNetwork = fallbackToNetwork {
    plugins.add(copyRedirectedCacheableResponsesPlugin);
  }

  final bool _fallbackToNetwork;

  /// Statuses below 400.
  static final WorkboxPlugin defaultPrecacheCacheabilityPlugin = _DefaultPrecacheCacheabilityPlugin();

  /// A redirected response is cached as a copy without its redirect.
  static final WorkboxPlugin copyRedirectedCacheableResponsesPlugin = _CopyRedirectedCacheableResponsesPlugin();

  @override
  Future<Response> _handle(Request request, StrategyHandler handler) async {
    final response = await handler.cacheMatch(request);
    if (response != null) return response;
    return handler.event.type == 'install' ? _handleInstall(request, handler) : _handleFetch(request, handler);
  }

  /// `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors, and
  /// a cache repair only when that integrity vouches for the response.
  Future<Response> _handleFetch(Request request, StrategyHandler handler) async {
    final params = handler.params ?? const Params();
    if (!_fallbackToNetwork) {
      throw WorkboxError(
        'missing-precache-entry',
        JSObject()
          ..['cacheName'] = cacheName.toJS
          ..['url'] = request.url.toJS,
      );
    }
    final integrityInManifest = params.integrity;
    final integrityInRequest = request.integrity;
    final noIntegrityConflict = integrityInRequest.isEmpty || integrityInRequest == integrityInManifest;
    final cors = request.mode != 'no-cors';
    final integrity = cors ? (integrityInRequest.isNotEmpty ? integrityInRequest : integrityInManifest) : null;
    final response = await handler.fetch(Request(request, requestInit(integrity: integrity)));
    if (integrityInManifest != null && integrityInManifest.isNotEmpty && noIntegrityConflict && cors) {
      _useDefaultCacheabilityPluginIfNeeded();
      await handler.cachePut(request, response.copy());
    }
    return response;
  }

  /// `_handleInstall`: fetch, then cache, and fail the install when the response may not be
  /// cached.
  Future<Response> _handleInstall(Request request, StrategyHandler handler) async {
    _useDefaultCacheabilityPluginIfNeeded();
    final response = await handler.fetch(request);
    final wasCached = await handler.cachePut(request, response.copy());
    if (!wasCached) {
      throw WorkboxError(
        'bad-precaching-response',
        JSObject()
          ..['url'] = request.url.toJS
          ..['status'] = response.status.toJS,
      );
    }
    return response;
  }

  /// `_useDefaultCacheabilityPluginIfNeeded`: exactly one cacheWillUpdate besides the redirect
  /// copy, adding the default one when there is none and dropping it when there are more.
  void _useDefaultCacheabilityPluginIfNeeded() {
    int? defaultPluginIndex;
    var cacheWillUpdatePluginCount = 0;
    for (final (index, plugin) in plugins.indexed) {
      if (identical(plugin, copyRedirectedCacheableResponsesPlugin)) continue;
      if (identical(plugin, defaultPrecacheCacheabilityPlugin)) defaultPluginIndex = index;
      if (plugin.cacheWillUpdate != null) cacheWillUpdatePluginCount++;
    }
    if (cacheWillUpdatePluginCount == 0) {
      plugins.add(defaultPrecacheCacheabilityPlugin);
    } else if (cacheWillUpdatePluginCount > 1 && defaultPluginIndex != null) {
      plugins.removeAt(defaultPluginIndex);
    }
  }
}

class _DefaultPrecacheCacheabilityPlugin extends WorkboxPlugin {
  @override
  Callback<CacheWillUpdateParam, Response?> get cacheWillUpdate => (param) async => param.response.status >= 400 ? null : param.response;
}

class _CopyRedirectedCacheableResponsesPlugin extends WorkboxPlugin {
  @override
  Callback<CacheWillUpdateParam, Response?> get cacheWillUpdate =>
      (param) async => param.response.redirected ? await copyResponse(param.response) : param.response;
}

/// `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL.
class PrecacheCacheKeyPlugin extends WorkboxPlugin {
  PrecacheCacheKeyPlugin(this._precacheController);

  final PrecacheController _precacheController;

  @override
  Callback<CacheKeyWillBeUsedParam, Request> get cacheKeyWillBeUsed => (param) async {
        final routeCacheKey = param.params?.cacheKey;
        final cacheKey = routeCacheKey != null && routeCacheKey.isNotEmpty
            ? routeCacheKey
            : _precacheController.getCacheKeyForURL(param.request.url);
        return cacheKey != null ? Request(cacheKey.toJS, requestInit(headers: param.request.headers)) : param.request;
      };
}

/// `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is.
class PrecacheInstallReportPlugin extends WorkboxPlugin {
  final updatedURLs = <String>[];
  final notUpdatedURLs = <String>[];

  @override
  Callback<HandlerWillStartParam, void> get handlerWillStart => (param) async {
        param.state?['originalRequest'] = param.request;
      };

  @override
  Callback<CachedResponseWillBeUsedParam, Response?> get cachedResponseWillBeUsed => (param) async {
        final originalRequest = param.state?['originalRequest'];
        if (param.event.type == 'install' && originalRequest is JSObject && originalRequest.instanceOfString('Request')) {
          final url = (originalRequest as Request).url;
          (param.cachedResponse != null ? notUpdatedURLs : updatedURLs).add(url);
        }
        return param.cachedResponse;
      };
}

/// `PrecacheFallbackPlugin`: a failed strategy answers with a precached URL, when it is there.
class PrecacheFallbackPlugin extends WorkboxPlugin {
  PrecacheFallbackPlugin({required String fallbackURL, required PrecacheController precacheController})
      : _fallbackURL = fallbackURL,
        _precacheController = precacheController;

  final String _fallbackURL;
  final PrecacheController _precacheController;

  @override
  Callback<HandlerDidErrorParam, Response?> get handlerDidError => (_) => _precacheController.matchPrecache(_fallbackURL);
}

/// `PrecacheRouteOptions`. A falsy `directoryIndex` or `cleanURLs` disables it.
class PrecacheRouteOptions {
  PrecacheRouteOptions({this.directoryIndex = 'index.html', List<HostRegExp>? ignoreURLParametersMatching, this.cleanURLs = true, this.urlManipulation})
      : ignoreURLParametersMatching = ignoreURLParametersMatching ?? [HostRegExp('^utm_'), HostRegExp(r'^fbclid$')];

  final String? directoryIndex;
  final List<HostRegExp> ignoreURLParametersMatching;
  final bool cleanURLs;
  final List<URL> Function(URL url)? urlManipulation;
}

/// `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself.
URL _removeIgnoredSearchParams(URL urlObject, List<HostRegExp> ignoreURLParametersMatching) {
  for (final paramName in arrayFrom(urlObject.searchParams.keys()).toDart) {
    if (ignoreURLParametersMatching.any((regExp) => regExp.test(paramName.toDart))) urlObject.searchParams.delete(paramName.toDart);
  }
  return urlObject;
}

/// `generateURLVariations(url, options)`: a lazy generator, as Workbox's is, so a later variation
/// (and `urlManipulation`) is computed only when the earlier ones missed.
Iterable<String> _generateURLVariations(String url, PrecacheRouteOptions options) sync* {
  final urlObject = URL.from(url, location.href);
  urlObject.hash = '';
  yield urlObject.href;
  // The URL object itself loses its ignored parameters; urlManipulation sees it that way.
  final urlWithoutIgnoredParams = _removeIgnoredSearchParams(urlObject, options.ignoreURLParametersMatching);
  yield urlWithoutIgnoredParams.href;
  final directoryIndex = options.directoryIndex;
  if (directoryIndex != null && directoryIndex.isNotEmpty && urlWithoutIgnoredParams.pathname.endsWith('/')) {
    final directoryURL = URL.from(urlWithoutIgnoredParams.href);
    directoryURL.pathname = directoryURL.pathname + directoryIndex;
    yield directoryURL.href;
  }
  if (options.cleanURLs) {
    final cleanURL = URL.from(urlWithoutIgnoredParams.href);
    cleanURL.pathname = '${cleanURL.pathname}.html';
    yield cleanURL.href;
  }
  final urlManipulation = options.urlManipulation;
  if (urlManipulation != null) {
    for (final urlToAttempt in urlManipulation(urlObject)) {
      yield urlToAttempt.href;
    }
  }
}

/// `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that matches
/// the first variation of the request URL in the manifest, with `{cacheKey, integrity}`.
class PrecacheRoute extends Route {
  PrecacheRoute(PrecacheController precacheController, [PrecacheRouteOptions? options])
      : super((matchOptions) {
          final urlsToCacheKeys = precacheController.getURLsToCacheKeys();
          for (final possibleURL in _generateURLVariations(matchOptions.request.url, options ?? PrecacheRouteOptions())) {
            final cacheKey = urlsToCacheKeys[possibleURL];
            if (cacheKey != null) return Params(cacheKey: cacheKey, integrity: precacheController.getIntegrityForCacheKey(cacheKey));
          }
          return null;
        }, precacheController.strategy);
}
