// workbox-precaching 7.4.1: createCacheKey, PrecacheController, PrecacheStrategy, PrecacheRoute
// with its URL variations, and the precache plugins (cache key, install report, fallback, default
// cacheability and redirect copy).

package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const revisionSearchParam = "__WB_REVISION__"

// resolveURL is new URL(url, location.href).
func resolveURL(url string) (URL, error) { return parseURL(url, locationHref()) }

// truthy is JavaScript truthiness.
func truthy(value *js.Object) bool { return value != nil && !isUndefined(value) && value.Bool() }

// typeTag is Object.prototype.toString.call(value), which tells a string from an object.
func typeTag(value *js.Object) string {
	return jsObject.Get("prototype").Get("toString").Call("call", value).String()
}

func isString(value *js.Object) bool { return value != nil && typeTag(value) == "[object String]" }

// createCacheKey is createCacheKey(entry): the entry's URL resolved against the location, and for
// a truthy revision a cache key that carries it in __WB_REVISION__. The entry is the caller's
// value: a string, null, or a {url, revision, integrity} object.
func createCacheKey(entry *js.Object) (cacheKey, url string, err error) {
	if !truthy(entry) {
		return "", "", newWorkboxError("add-to-cache-list-unexpected-type", newObject("entry", entry))
	}
	if isString(entry) {
		parsed, err := resolveURL(entry.String())
		if err != nil {
			return "", "", err
		}
		return parsed.Href(), parsed.Href(), nil
	}
	revision, entryURL := entry.Get("revision"), entry.Get("url")
	if !truthy(entryURL) {
		return "", "", newWorkboxError("add-to-cache-list-unexpected-type", newObject("entry", entry))
	}
	// A URL that is not a string (outside the domain) goes through ToString, as new URL() does.
	if !truthy(revision) {
		parsed, err := resolveURL(stringOf(entryURL))
		if err != nil {
			return "", "", err
		}
		return parsed.Href(), parsed.Href(), nil
	}
	cacheKeyURL, err := resolveURL(stringOf(entryURL))
	if err != nil {
		return "", "", err
	}
	originalURL, err := resolveURL(stringOf(entryURL))
	if err != nil {
		return "", "", err
	}
	cacheKeyURL.SetSearchParam(revisionSearchParam, revision)
	return cacheKeyURL.Href(), originalURL.Href(), nil
}

// orderedMap is a JS Map from strings: insertion-ordered, and setting a known key keeps its place.
type orderedMap struct {
	keys   []string
	values map[string]string
}

func newOrderedMap() *orderedMap { return &orderedMap{values: map[string]string{}} }

func (m *orderedMap) get(key string) (string, bool) {
	value, ok := m.values[key]
	return value, ok
}

func (m *orderedMap) set(key, value string) {
	if _, ok := m.values[key]; !ok {
		m.keys = append(m.keys, key)
	}
	m.values[key] = value
}

// installReport is what one PrecacheInstallReportPlugin records.
type installReport struct{ updatedURLs, notUpdatedURLs []string }

// newInstallReportPlugin is new PrecacheInstallReportPlugin(): an install-time cache hit is not
// updated, a miss is.
func newInstallReportPlugin() (*installReport, *Plugin) {
	report := &installReport{}
	return report, &Plugin{
		HandlerWillStart: func(cx Cx, request Request) *Task[struct{}] {
			if cx.State != nil {
				cx.State.Set("originalRequest", request.object)
			}
			return resolved(struct{}{})
		},
		CachedResponseWillBeUsed: func(cx Cx, _ string, _ Request, cachedResponse *Response) *Task[*Response] {
			if cx.Event.Type() == "install" && cx.State != nil {
				if originalRequest, ok := asRequest(cx.State.Get("originalRequest")); ok {
					if cachedResponse != nil {
						report.notUpdatedURLs = append(report.notUpdatedURLs, originalRequest.URL())
					} else {
						report.updatedURLs = append(report.updatedURLs, originalRequest.URL())
					}
				}
			}
			return resolved(cachedResponse)
		},
	}
}

// newPrecacheCacheKeyPlugin is new PrecacheCacheKeyPlugin({precacheController}): the route's cache
// key, else the manifest's key for the exact URL.
func newPrecacheCacheKeyPlugin(precacheController *PrecacheController) *Plugin {
	return &Plugin{CacheKeyWillBeUsed: func(_ Cx, _ string, request Request, params *Params) *Task[Request] {
		cacheKey, ok := "", false
		if params != nil && params.CacheKey != "" {
			cacheKey, ok = params.CacheKey, true
		} else {
			var err error
			if cacheKey, ok, err = precacheController.GetCacheKeyForURL(request.URL()); err != nil {
				return failed[Request](err)
			}
		}
		if !ok {
			return resolved(request)
		}
		keyed, err := newRequest(cacheKey, RequestInit{Headers: request.Headers()})
		if err != nil {
			return failed[Request](err)
		}
		return resolved(keyed)
	}}
}

// defaultPrecacheCacheabilityPlugin is PrecacheStrategy.defaultPrecacheCacheabilityPlugin:
// statuses below 400.
var defaultPrecacheCacheabilityPlugin = &Plugin{CacheWillUpdate: func(_ Cx, _ Request, response Response) *Task[*Response] {
	if response.Status() >= 400 {
		return resolved[*Response](nil)
	}
	return resolved(&response)
}}

// copyRedirectedCacheableResponsesPlugin is PrecacheStrategy.copyRedirectedCacheableResponsesPlugin.
var copyRedirectedCacheableResponsesPlugin = &Plugin{CacheWillUpdate: func(_ Cx, _ Request, response Response) *Task[*Response] {
	if !response.Redirected() {
		return resolved(&response)
	}
	return start(func(settle func(*Response, error)) {
		copyResponse(response).then(func(copied Response, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			settle(&copied, nil)
		})
	})
}}

// newPrecacheStrategy is new PrecacheStrategy({cacheName, plugins, fallbackToNetwork}), which
// appends the redirect copy plugin.
func newPrecacheStrategy(cacheName string, plugins []*Plugin, fallbackToNetwork bool) *Strategy {
	strategy := newStrategy(cacheName, plugins, func(strategy *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
		return precacheHandle(strategy, fallbackToNetwork, request, handler)
	})
	strategy.Plugins = append(strategy.Plugins, copyRedirectedCacheableResponsesPlugin)
	return strategy
}

// precacheHandle is _handle: the cache, then install-time population, then the network fallback.
func precacheHandle(strategy *Strategy, fallbackToNetwork bool, request Request, handler *StrategyHandler) *Task[Response] {
	return start(func(settle func(Response, error)) {
		handler.cacheMatch(request).then(func(cached *Response, err error) {
			switch {
			case err != nil:
				settle(Response{}, err)
			case cached != nil:
				settle(*cached, nil)
			case handler.Event.Type() == "install":
				handleInstall(strategy, request, handler).then(settle)
			default:
				handleFetch(strategy, fallbackToNetwork, request, handler).then(settle)
			}
		})
	})
}

// handleFetch is _handleFetch: the network, with the manifest's integrity unless the request is
// no-cors, and a cache repair only when that integrity vouches for the response.
func handleFetch(strategy *Strategy, fallbackToNetwork bool, request Request, handler *StrategyHandler) *Task[Response] {
	params := Params{}
	if handler.Params != nil {
		params = *handler.Params
	}
	if !fallbackToNetwork {
		details := newObject("cacheName", strategy.CacheName, "url", request.URL())
		return failed[Response](newWorkboxError("missing-precache-entry", details))
	}
	integrityInManifest := params.Integrity
	integrityInRequest := request.Integrity()
	noIntegrityConflict := integrityInRequest == "" || integrityInManifest != nil && *integrityInManifest == integrityInRequest
	cors := request.Mode() != "no-cors"
	var integrity *string
	if cors && integrityInRequest != "" {
		integrity = some(integrityInRequest)
	} else if cors {
		integrity = integrityInManifest
	}
	fetched, err := newRequest(request.object, RequestInit{Integrity: integrity})
	if err != nil {
		return failed[Response](err)
	}
	return start(func(settle func(Response, error)) {
		handler.fetch(fetched).then(func(response Response, err error) {
			if err != nil || integrityInManifest == nil || !noIntegrityConflict || !cors {
				settle(response, err)
				return
			}
			useDefaultCacheabilityPluginIfNeeded(strategy)
			handler.cachePut(request, response.Clone()).then(func(_ bool, err error) { settle(response, err) })
		})
	})
}

// handleInstall is _handleInstall: fetch, then cache, and fail the install when the response may
// not be cached.
func handleInstall(strategy *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
	return start(func(settle func(Response, error)) {
		useDefaultCacheabilityPluginIfNeeded(strategy)
		handler.fetch(request).then(func(response Response, err error) {
			if err != nil {
				settle(Response{}, err)
				return
			}
			handler.cachePut(request, response.Clone()).then(func(wasCached bool, err error) {
				switch {
				case err != nil:
					settle(Response{}, err)
				case !wasCached:
					details := newObject("url", request.URL(), "status", response.Status())
					settle(Response{}, newWorkboxError("bad-precaching-response", details))
				default:
					settle(response, nil)
				}
			})
		})
	})
}

// useDefaultCacheabilityPluginIfNeeded is _useDefaultCacheabilityPluginIfNeeded: one cacheWillUpdate
// besides the redirect copy. It adds the default plugin when there is none, and removes it when
// there are more.
func useDefaultCacheabilityPluginIfNeeded(strategy *Strategy) {
	defaultPluginIndex, hasDefaultPlugin := 0, false
	cacheWillUpdatePluginCount := 0
	for index, plugin := range strategy.Plugins {
		if plugin == copyRedirectedCacheableResponsesPlugin {
			continue
		}
		if plugin == defaultPrecacheCacheabilityPlugin {
			defaultPluginIndex, hasDefaultPlugin = index, true
		}
		if plugin.defines(cacheWillUpdate) {
			cacheWillUpdatePluginCount++
		}
	}
	if cacheWillUpdatePluginCount == 0 {
		strategy.Plugins = append(strategy.Plugins, defaultPrecacheCacheabilityPlugin)
	} else if cacheWillUpdatePluginCount > 1 && hasDefaultPlugin {
		strategy.Plugins = append(strategy.Plugins[:defaultPluginIndex:defaultPluginIndex], strategy.Plugins[defaultPluginIndex+1:]...)
	}
}

// InstallResult is what install answers.
type InstallResult struct{ UpdatedURLs, NotUpdatedURLs []string }

// CleanupResult is what activate answers.
type CleanupResult struct{ DeletedURLs []string }

// PrecacheController is new PrecacheController({cacheName, plugins, fallbackToNetwork}).
type PrecacheController struct {
	urlsToCacheKeys        *orderedMap
	urlsToCacheModes       map[string]string
	cacheKeysToIntegrities map[string]string
	strategy               *Strategy
}

// newPrecacheController builds a PrecacheStrategy whose plugins end with this controller's
// cache-key plugin.
func newPrecacheController(cacheName string, plugins []*Plugin, fallbackToNetwork bool) *PrecacheController {
	controller := &PrecacheController{
		urlsToCacheKeys:        newOrderedMap(),
		urlsToCacheModes:       map[string]string{},
		cacheKeysToIntegrities: map[string]string{},
	}
	controller.strategy = newPrecacheStrategy(cacheName, append(plugins, newPrecacheCacheKeyPlugin(controller)), fallbackToNetwork)
	return controller
}

func (c *PrecacheController) Strategy() *Strategy { return c.strategy }

// AddToCacheList works entry by entry, so the entries before a failing one stay added. The
// revisionless warning goes to the console after each entry, as in the production build.
func (c *PrecacheController) AddToCacheList(entries []*js.Object) error {
	var urlsToWarnAbout []string
	for _, entry := range entries {
		if isString(entry) {
			urlsToWarnAbout = append(urlsToWarnAbout, entry.String())
		} else if truthy(entry) && isUndefined(entry.Get("revision")) {
			urlsToWarnAbout = append(urlsToWarnAbout, joinText(entry.Get("url")))
		}
		cacheKey, url, err := createCacheKey(entry)
		if err != nil {
			return err
		}
		cacheMode := "default"
		if !isString(entry) && truthy(entry.Get("revision")) {
			cacheMode = "reload"
		}
		if firstEntry, ok := c.urlsToCacheKeys.get(url); ok && firstEntry != cacheKey {
			return newWorkboxError("add-to-cache-list-conflicting-entries", newObject("firstEntry", firstEntry, "secondEntry", cacheKey))
		}
		if integrity := entry.Get("integrity"); !isString(entry) && isString(integrity) && integrity.String() != "" {
			if known, ok := c.cacheKeysToIntegrities[cacheKey]; ok && known != integrity.String() {
				return newWorkboxError("add-to-cache-list-conflicting-integrities", newObject("url", url))
			}
			c.cacheKeysToIntegrities[cacheKey] = integrity.String()
		}
		c.urlsToCacheKeys.set(url, cacheKey)
		c.urlsToCacheModes[url] = cacheMode
		if len(urlsToWarnAbout) > 0 {
			js.Global.Get("console").Call("warn", "Workbox is precaching URLs without revision info: "+strings.Join(urlsToWarnAbout, ", ")+
				"\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache")
		}
	}
	return nil
}

// joinText is how Array.prototype.join prints a value: null and undefined as empty.
func joinText(value *js.Object) string {
	if value == nil || isUndefined(value) {
		return ""
	}
	return stringOf(value)
}

// Install is install(event): each URL in turn through the strategy, under a new install report
// plugin, with the event's lifetime extended until it is done. The manifest is iterated live, as
// for…of iterates the Map.
func (c *PrecacheController) Install(event ExtendableEvent) *Task[InstallResult] {
	return waitUntil(event, func() *Task[InstallResult] {
		return start(func(settle func(InstallResult, error)) {
			report, reportPlugin := newInstallReportPlugin()
			c.strategy.Plugins = append(c.strategy.Plugins, reportPlugin)
			var each func(int)
			each = func(index int) {
				if index == len(c.urlsToCacheKeys.keys) {
					settle(InstallResult{UpdatedURLs: report.updatedURLs, NotUpdatedURLs: report.notUpdatedURLs}, nil)
					return
				}
				url := c.urlsToCacheKeys.keys[index]
				cacheKey, _ := c.urlsToCacheKeys.get(url)
				init := RequestInit{Credentials: some("same-origin")}
				if integrity, ok := c.cacheKeysToIntegrities[cacheKey]; ok {
					init.Integrity = some(integrity)
				}
				if cacheMode, ok := c.urlsToCacheModes[url]; ok {
					init.Cache = some(cacheMode)
				}
				request, err := newRequest(url, init)
				if err != nil {
					settle(InstallResult{}, err)
					return
				}
				responseDone, handlerDone := c.strategy.handleAll(HandlerOptions{Request: request, Event: event, Params: &Params{CacheKey: cacheKey}})
				// Promise.all: the response's failure, else the completion's.
				responseDone.then(func(_ Response, err error) {
					if err != nil {
						settle(InstallResult{}, err)
						return
					}
					handlerDone.then(func(_ struct{}, err error) {
						if err != nil {
							settle(InstallResult{}, err)
							return
						}
						each(index + 1)
					})
				})
			}
			each(0)
		})
	})
}

// Activate is activate(event): deletes every request in the precache whose URL is no current
// cache key.
func (c *PrecacheController) Activate(event ExtendableEvent) *Task[CleanupResult] {
	return waitUntil(event, func() *Task[CleanupResult] {
		return start(func(settle func(CleanupResult, error)) {
			cachesOpen(c.strategy.CacheName).then(func(cache Cache, err error) {
				if err != nil {
					settle(CleanupResult{}, err)
					return
				}
				cache.Keys().then(func(currentlyCachedRequests []Request, err error) {
					if err != nil {
						settle(CleanupResult{}, err)
						return
					}
					expectedCacheKeys := map[string]bool{}
					for _, url := range c.urlsToCacheKeys.keys {
						expectedCacheKeys[c.urlsToCacheKeys.values[url]] = true
					}
					var deletedURLs []string
					var each func(int)
					each = func(index int) {
						if index == len(currentlyCachedRequests) {
							settle(CleanupResult{DeletedURLs: deletedURLs}, nil)
							return
						}
						request := currentlyCachedRequests[index]
						if expectedCacheKeys[request.URL()] {
							each(index + 1)
							return
						}
						cache.Delete(request).then(func(_ bool, err error) {
							if err != nil {
								settle(CleanupResult{}, err)
								return
							}
							deletedURLs = append(deletedURLs, request.URL())
							each(index + 1)
						})
					}
					each(0)
				})
			})
		})
	})
}

// GetURLsToCacheKeys is the manifest, in insertion order.
func (c *PrecacheController) GetURLsToCacheKeys() [][2]string {
	var pairs [][2]string
	for _, url := range c.urlsToCacheKeys.keys {
		pairs = append(pairs, [2]string{url, c.urlsToCacheKeys.values[url]})
	}
	return pairs
}

func (c *PrecacheController) GetCachedURLs() []string {
	return append([]string(nil), c.urlsToCacheKeys.keys...)
}

// GetCacheKeyForURL is getCacheKeyForURL(url): the exact URL resolved against the location,
// without variations.
func (c *PrecacheController) GetCacheKeyForURL(url string) (string, bool, error) {
	parsed, err := resolveURL(url)
	if err != nil {
		return "", false, err
	}
	cacheKey, ok := c.urlsToCacheKeys.get(parsed.Href())
	return cacheKey, ok, nil
}

func (c *PrecacheController) GetIntegrityForCacheKey(cacheKey string) *string {
	if integrity, ok := c.cacheKeysToIntegrities[cacheKey]; ok {
		return &integrity
	}
	return nil
}

// MatchPrecache is matchPrecache(request), for a URL string or a Request: it opens the precache
// only for a URL in the manifest.
func (c *PrecacheController) MatchPrecache(request any) *Task[*Response] {
	var url string
	switch request := request.(type) {
	case Request:
		url = request.URL()
	case string:
		url = request
	}
	cacheKey, ok, err := c.GetCacheKeyForURL(url)
	switch {
	case err != nil:
		return failed[*Response](err)
	case !ok:
		return resolved[*Response](nil)
	}
	return start(func(settle func(*Response, error)) {
		cachesOpen(c.strategy.CacheName).then(func(cache Cache, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			cache.Match(cacheKey).then(settle)
		})
	})
}

// CreateHandlerBoundToURL is createHandlerBoundToURL(url): a handler callback that answers any
// request with this precached URL's entry, with {cacheKey, ...params}.
func (c *PrecacheController) CreateHandlerBoundToURL(url string) (Handler, error) {
	cacheKey, ok, err := c.GetCacheKeyForURL(url)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, newWorkboxError("non-precached-url", newObject("url", url))
	}
	return HandlerFunc(func(options HandlerOptions) *Task[Response] {
		request, err := newRequest(url, RequestInit{})
		if err != nil {
			return failed[Response](err)
		}
		params := &Params{CacheKey: cacheKey}
		if options.Params != nil {
			params = &Params{CacheKey: options.Params.CacheKey, Integrity: options.Params.Integrity}
		}
		options.Request, options.Params = request, params
		return c.strategy.Handle(options)
	}), nil
}

// newPrecacheFallbackPlugin is new PrecacheFallbackPlugin({fallbackURL, precacheController}): a
// failed strategy answers with a precached URL, when it is there.
func newPrecacheFallbackPlugin(fallbackURL string, precacheController *PrecacheController) *Plugin {
	return &Plugin{HandlerDidError: func(Cx, Request, error) *Task[*Response] {
		return precacheController.MatchPrecache(fallbackURL)
	}}
}

// PrecacheRouteOptions are the PrecacheRoute options once generateURLVariations has applied its
// defaults. DirectoryIndex is nil when the option is falsy.
type PrecacheRouteOptions struct {
	IgnoreURLParametersMatching []RegExp
	DirectoryIndex              *string
	CleanURLs                   bool
	URLManipulation             func(url URL) []URL
}

// defaultPrecacheRouteOptions: [/^utm_/, /^fbclid$/], index.html, clean URLs, no manipulation.
func defaultPrecacheRouteOptions() PrecacheRouteOptions {
	var ignored []RegExp
	for _, source := range []string{"^utm_", "^fbclid$"} {
		if regexp, err := newRegExp(source); err == nil {
			ignored = append(ignored, regexp)
		}
	}
	return PrecacheRouteOptions{IgnoreURLParametersMatching: ignored, DirectoryIndex: some("index.html"), CleanURLs: true}
}

// removeIgnoredSearchParams is removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching),
// on the URL itself.
func removeIgnoredSearchParams(urlObject URL, ignoreURLParametersMatching []RegExp) URL {
	for _, paramName := range urlObject.SearchParamNames() {
		for _, regexp := range ignoreURLParametersMatching {
			if regexp.Test(paramName) {
				urlObject.DeleteSearchParam(paramName)
				break
			}
		}
	}
	return urlObject
}

// generateURLVariations is generateURLVariations(url, options), consumed lazily: each variation
// goes to visit in order until visit answers true, and the later ones (and urlManipulation) are
// not computed.
func generateURLVariations(urlObject URL, options PrecacheRouteOptions, visit func(string) bool) {
	urlObject.SetHash("")
	if visit(urlObject.Href()) {
		return
	}
	// The URL object itself loses its ignored parameters; urlManipulation receives it so.
	urlWithoutIgnoredParams := removeIgnoredSearchParams(urlObject, options.IgnoreURLParametersMatching)
	if visit(urlWithoutIgnoredParams.Href()) {
		return
	}
	withSuffix := func(suffix string) string {
		variant := urlWithoutIgnoredParams.Copy()
		variant.SetPathname(variant.Pathname() + suffix)
		return variant.Href()
	}
	if options.DirectoryIndex != nil && strings.HasSuffix(urlWithoutIgnoredParams.Pathname(), "/") {
		if visit(withSuffix(*options.DirectoryIndex)) {
			return
		}
	}
	if options.CleanURLs && visit(withSuffix(".html")) {
		return
	}
	if options.URLManipulation != nil {
		for _, urlToAttempt := range options.URLManipulation(urlObject) {
			if visit(urlToAttempt.Href()) {
				return
			}
		}
	}
}

// newPrecacheRoute is new PrecacheRoute(controller, options): a GET route to the controller's
// strategy that matches the first variation of the request URL in the manifest, with {cacheKey,
// integrity}.
func newPrecacheRoute(controller *PrecacheController, options PrecacheRouteOptions) *Route {
	match := func(context MatchOptions) (bool, *Params, error) {
		urlObject, err := resolveURL(context.Request.URL())
		if err != nil {
			return false, nil, err
		}
		var params *Params
		generateURLVariations(urlObject, options, func(possibleURL string) bool {
			if cacheKey, ok := controller.urlsToCacheKeys.get(possibleURL); ok && cacheKey != "" {
				params = &Params{CacheKey: cacheKey, Integrity: controller.GetIntegrityForCacheKey(cacheKey)}
				return true
			}
			return false
		})
		return params != nil, params, nil
	}
	return newRoute(match, controller.strategy, nil)
}
