// The adapter (CONTRACT.md "Steps", "Fixtures" and "Output"): runs one script of Workbox API calls
// against this port, with the fixture plugins and the stand-in events, and builds the canonical
// output. The scenario's caches are deleted on every path once it has reported, or failed.

package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const fixture = "experiments/library-matrix/libraries/workbox/"

// wrongIntegrity is well-formed SRI metadata that matches no fixture, so the fetch fails
// integrity.
var wrongIntegrity = "sha256-" + strings.Repeat("A", 43) + "="

// optionalString is a member that is a string, or nil.
func optionalString(value *js.Object, key string) *string {
	if member := value.Get(key); isString(member) {
		return some(member.String())
	}
	return nil
}

func stringField(value *js.Object, key string) string {
	if member := optionalString(value, key); member != nil {
		return *member
	}
	return ""
}

func listField(value *js.Object, key string) []*js.Object {
	if member := value.Get(key); member != nil && isArray(member) {
		return elements(member)
	}
	return nil
}

// standInEvent is the stand-in ExtendableEvent of one event-driven step.
type standInEvent struct {
	eventType string
	pending   *[]*js.Object
}

func (e standInEvent) Type() string                 { return e.eventType }
func (e standInEvent) WaitUntil(promise *js.Object) { *e.pending = append(*e.pending, promise) }

// hostEvent is a native event the caller supplied (service-worker mode).
type hostEvent struct{ object *js.Object }

func (e hostEvent) Type() string                 { return e.object.Get("type").String() }
func (e hostEvent) WaitUntil(promise *js.Object) { e.object.Call("waitUntil", promise) }

// markRedirected marks a response, and every clone of it, as a followed redirect leaves it.
func markRedirected(response Response) Response {
	clone := response.object.Get("clone").Call("bind", response.object)
	jsObject.Call("defineProperty", response.object, "redirected", newObject("value", true))
	jsObject.Call("defineProperty", response.object, "clone", newObject("value", func() *js.Object {
		return markRedirected(Response{clone.Invoke()}).object
	}))
	return response
}

// scenario is one call's state.
type scenario struct {
	input       *js.Object
	base        string
	prefix      string
	source      string
	controllers map[string]*PrecacheController
	routers     map[string]*Router
	routes      map[string]*Route
	results     *js.Object
	events      *js.Object
	handlers    *js.Object
}

func errorTypeError(message string) error { return newHostError("TypeError", message) }

// jsString is a Go string as a JS string value.
func jsString(text string) *js.Object { return js.Global.Call("String", text) }

func (s *scenario) relative(url string) string   { return strings.TrimPrefix(url, s.base) }
func (s *scenario) cacheName(name string) string { return s.prefix + "-" + name }

func (s *scenario) cacheLabel(name string) string { return strings.TrimPrefix(name, s.prefix+"-") }

func (s *scenario) isOwnCache(name string) bool { return strings.HasPrefix(name, s.prefix+"-") }

func (s *scenario) controller(id string) (*PrecacheController, error) {
	if controller, ok := s.controllers[id]; ok {
		return controller, nil
	}
	return nil, errorTypeError("the script names no controller " + id)
}

func (s *scenario) router(id string) (*Router, error) {
	if router, ok := s.routers[id]; ok {
		return router, nil
	}
	return nil, errorTypeError("the script names no router " + id)
}

func (s *scenario) route(id string) (*Route, error) {
	if route, ok := s.routes[id]; ok {
		return route, nil
	}
	return nil, errorTypeError("the script names no route " + id)
}

// nameOf is an Error's name; a thrown non-Error (the fail fixture's value) prints as its string.
func nameOf(err error) string {
	if isError(err) {
		return jsValue(err).Get("name").String()
	}
	return stringOf(jsValue(err))
}

// failure is {error, message} for a WorkboxError, {error} for another Error, {thrown} for a thrown
// non-Error.
func (s *scenario) failure(err error) *js.Object {
	if workboxError, ok := err.(*WorkboxError); ok {
		message := stringOf(workboxError.object.Get("message"))
		message = strings.ReplaceAll(strings.ReplaceAll(message, s.base, ""), s.prefix+"-", "")
		return newObject("error", workboxError.object.Get("name"), "message", message)
	}
	if isError(err) {
		return newObject("error", jsValue(err).Get("name"))
	}
	return newObject("thrown", stringOf(jsValue(err)))
}

func status(response *Response) any {
	if response == nil {
		return nil
	}
	return response.Status()
}

func (s *scenario) urls(urls []string) *js.Object {
	out := jsArray.New()
	for _, url := range urls {
		out.Call("push", s.relative(url))
	}
	return out
}

// record is {status, url, body} of a response, reading its body.
func (s *scenario) record(response Response) *Task[*js.Object] {
	return start(func(settle func(*js.Object, error)) {
		statusCode, url := response.Status(), s.relative(response.URL())
		response.Text().then(func(body *js.Object, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			settle(newObject("status", statusCode, "url", url, "body", body), nil)
		})
	})
}

// dump lists every cache of the scenario, in CacheStorage order, with each entry's record.
func (s *scenario) dump() *Task[*js.Object] {
	return start(func(settle func(*js.Object, error)) {
		out := jsArray.New()
		cachesKeys().then(func(names []string, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			var eachCache func(int)
			eachCache = func(index int) {
				if index == len(names) {
					settle(out, nil)
					return
				}
				name := names[index]
				if !s.isOwnCache(name) {
					eachCache(index + 1)
					return
				}
				cachesOpen(name).then(func(cache Cache, err error) {
					if err != nil {
						settle(nil, err)
						return
					}
					cache.Keys().then(func(requests []Request, err error) {
						if err != nil {
							settle(nil, err)
							return
						}
						entries := jsArray.New()
						var eachEntry func(int)
						eachEntry = func(entry int) {
							if entry == len(requests) {
								out.Call("push", newArray(s.cacheLabel(name), entries))
								eachCache(index + 1)
								return
							}
							request := requests[entry]
							cache.Match(request.object).then(func(response *Response, err error) {
								if err == nil && response == nil {
									err = errorTypeError("a cache entry without a response")
								}
								if err != nil {
									settle(nil, err)
									return
								}
								s.record(*response).then(func(record *js.Object, err error) {
									if err != nil {
										settle(nil, err)
										return
									}
									entries.Call("push", newArray(s.relative(request.URL()), record))
									eachEntry(entry + 1)
								})
							})
						}
						eachEntry(0)
					})
				})
			}
			eachCache(0)
		})
	})
}

// networkPlugin is the network fixture: every fetch goes to one of the two fixture files, or fails.
func (s *scenario) networkPlugin() *Plugin {
	return &Plugin{
		RequestWillFetch: func(_ Cx, request Request) *Task[Request] {
			if s.source == "offline" {
				return failed[Request](errorTypeError("offline"))
			}
			file := "alpha"
			if s.source == "beta" {
				file = "beta"
			}
			target, err := resolveURL(fixture + file + ".mjs")
			if err != nil {
				return failed[Request](err)
			}
			integrity := request.Integrity()
			if s.source == "broken" {
				integrity = wrongIntegrity
			}
			replaced, err := newRequest(target.Href(), RequestInit{Integrity: some(integrity)})
			if err != nil {
				return failed[Request](err)
			}
			return resolved(replaced)
		},
		FetchDidSucceed: func(_ Cx, _ Request, response Response) *Task[Response] {
			switch s.source {
			case "missing":
				missing, err := newResponse("missing", 404)
				if err != nil {
					return failed[Response](err)
				}
				return resolved(missing)
			case "redirected":
				return resolved(markRedirected(response))
			}
			return resolved(response)
		},
	}
}

// tracePlugin records every lifecycle callback but cacheWillUpdate into its handler's log. Records
// go only into a log the plugin's own handlerWillStart created: a plugin before it can fail that
// callback, and the trace must not throw in the callbacks that follow.
func (s *scenario) tracePlugin(label string) *Plugin {
	record := func(cx Cx, entry ...any) {
		if cx.State == nil {
			return
		}
		if log := cx.State.Get("log"); !isUndefined(log) {
			log.Call("push", newArray(entry...))
		}
	}
	url := func(request Request) string { return s.relative(request.URL()) }
	done := func() *Task[struct{}] { return resolved(struct{}{}) }
	return &Plugin{
		HandlerWillStart: func(cx Cx, request Request) *Task[struct{}] {
			log := jsArray.New()
			if cx.State != nil {
				cx.State.Set("log", log)
			}
			s.handlers.Call("push", newArray(label, url(request), log))
			return done()
		},
		CacheKeyWillBeUsed: func(cx Cx, mode string, request Request, _ *Params) *Task[Request] {
			record(cx, "cacheKeyWillBeUsed", mode, url(request))
			return resolved(request)
		},
		CachedResponseWillBeUsed: func(cx Cx, cacheName string, request Request, cachedResponse *Response) *Task[*Response] {
			record(cx, "cachedResponseWillBeUsed", s.cacheLabel(cacheName), url(request), status(cachedResponse))
			return resolved(cachedResponse)
		},
		RequestWillFetch: func(cx Cx, request Request) *Task[Request] {
			record(cx, "requestWillFetch", url(request))
			return resolved(request)
		},
		FetchDidSucceed: func(cx Cx, request Request, response Response) *Task[Response] {
			record(cx, "fetchDidSucceed", url(request), response.Status())
			return resolved(response)
		},
		FetchDidFail: func(cx Cx, err error, originalRequest, request Request) *Task[struct{}] {
			record(cx, "fetchDidFail", url(originalRequest), url(request), nameOf(err))
			return done()
		},
		CacheDidUpdate: func(cx Cx, cacheName string, request Request, oldResponse *Response, newResponse Response) *Task[struct{}] {
			record(cx, "cacheDidUpdate", s.cacheLabel(cacheName), url(request), status(oldResponse), newResponse.Status())
			return done()
		},
		HandlerWillRespond: func(cx Cx, _ Request, response Response) *Task[Response] {
			record(cx, "handlerWillRespond", response.Status())
			return resolved(response)
		},
		HandlerDidRespond: func(cx Cx, _ Request, response *Response) *Task[struct{}] {
			record(cx, "handlerDidRespond", status(response))
			return done()
		},
		HandlerDidComplete: func(cx Cx, _ Request, response *Response, err error) *Task[struct{}] {
			var name any
			if err != nil {
				name = nameOf(err)
			}
			record(cx, "handlerDidComplete", status(response), name)
			return done()
		},
		HandlerDidError: func(cx Cx, _ Request, err error) *Task[*Response] {
			record(cx, "handlerDidError", nameOf(err))
			return resolved[*Response](nil)
		},
	}
}

// failPlugin defines only the callback named on, which throws new Error('fail'), or, with value,
// the value itself (a non-Error).
func failPlugin(on string, value *js.Object) (*Plugin, error) {
	thrown := func() error {
		if isUndefined(value) {
			return newHostError("Error", "fail")
		}
		return &js.Error{Object: value}
	}
	done := func() *Task[struct{}] { return failed[struct{}](thrown()) }
	request := func() *Task[Request] { return failed[Request](thrown()) }
	response := func() *Task[Response] { return failed[Response](thrown()) }
	maybe := func() *Task[*Response] { return failed[*Response](thrown()) }
	switch on {
	case "handlerWillStart":
		return &Plugin{HandlerWillStart: func(Cx, Request) *Task[struct{}] { return done() }}, nil
	case "cacheKeyWillBeUsed":
		return &Plugin{CacheKeyWillBeUsed: func(Cx, string, Request, *Params) *Task[Request] { return request() }}, nil
	case "cachedResponseWillBeUsed":
		return &Plugin{CachedResponseWillBeUsed: func(Cx, string, Request, *Response) *Task[*Response] { return maybe() }}, nil
	case "cacheWillUpdate":
		return &Plugin{CacheWillUpdate: func(Cx, Request, Response) *Task[*Response] { return maybe() }}, nil
	case "cacheDidUpdate":
		return &Plugin{CacheDidUpdate: func(Cx, string, Request, *Response, Response) *Task[struct{}] { return done() }}, nil
	case "requestWillFetch":
		return &Plugin{RequestWillFetch: func(Cx, Request) *Task[Request] { return request() }}, nil
	case "fetchDidSucceed":
		return &Plugin{FetchDidSucceed: func(Cx, Request, Response) *Task[Response] { return response() }}, nil
	case "fetchDidFail":
		return &Plugin{FetchDidFail: func(Cx, error, Request, Request) *Task[struct{}] { return done() }}, nil
	case "handlerWillRespond":
		return &Plugin{HandlerWillRespond: func(Cx, Request, Response) *Task[Response] { return response() }}, nil
	case "handlerDidRespond":
		return &Plugin{HandlerDidRespond: func(Cx, Request, *Response) *Task[struct{}] { return done() }}, nil
	case "handlerDidComplete":
		return &Plugin{HandlerDidComplete: func(Cx, Request, *Response, error) *Task[struct{}] { return done() }}, nil
	case "handlerDidError":
		return &Plugin{HandlerDidError: func(Cx, Request, error) *Task[*Response] { return maybe() }}, nil
	}
	return nil, errorTypeError("unknown plugin callback " + on)
}

func (s *scenario) fixturePlugin(spec *js.Object) (*Plugin, error) {
	switch kind := stringField(spec, "kind"); kind {
	case "trace":
		return s.tracePlugin(stringField(spec, "label")), nil
	case "keep":
		return &Plugin{CacheWillUpdate: func(_ Cx, _ Request, response Response) *Task[*Response] { return resolved(&response) }}, nil
	case "drop":
		return &Plugin{CacheWillUpdate: func(Cx, Request, Response) *Task[*Response] { return resolved[*Response](nil) }}, nil
	case "fail":
		return failPlugin(stringField(spec, "on"), spec.Get("value"))
	case "fallback":
		controller, err := s.controller(stringField(spec, "controller"))
		if err != nil {
			return nil, err
		}
		return newPrecacheFallbackPlugin(stringField(spec, "url"), controller), nil
	default:
		return nil, errorTypeError("unknown plugin " + kind)
	}
}

func (s *scenario) fixturePlugins(specs []*js.Object) ([]*Plugin, error) {
	var plugins []*Plugin
	for _, spec := range specs {
		plugin, err := s.fixturePlugin(spec)
		if err != nil {
			return nil, err
		}
		plugins = append(plugins, plugin)
	}
	return plugins, nil
}

// plugins: every plugin list the adapter builds starts with the network plugin.
func (s *scenario) plugins(specs []*js.Object) ([]*Plugin, error) {
	plugins, err := s.fixturePlugins(specs)
	return append([]*Plugin{s.networkPlugin()}, plugins...), err
}

// handler is a route handler: a strategy, a controller's precache strategy, a handler bound to a
// precached URL, or the respond callback (an async function with a fixed response).
func (s *scenario) handler(spec *js.Object) (Handler, error) {
	if name := optionalString(spec, "strategy"); name != nil {
		plugins, err := s.plugins(listField(spec, "plugins"))
		if err != nil {
			return nil, err
		}
		cacheName := s.cacheName(stringField(spec, "cacheName"))
		switch *name {
		case "CacheFirst":
			return newCacheFirst(cacheName, plugins), nil
		case "CacheOnly":
			return newCacheOnly(cacheName, plugins), nil
		case "NetworkFirst":
			return newNetworkFirst(cacheName, plugins), nil
		case "NetworkOnly":
			return newNetworkOnly(cacheName, plugins), nil
		}
		return nil, errorTypeError("unknown strategy " + *name)
	}
	if id := optionalString(spec, "precache"); id != nil {
		controller, err := s.controller(*id)
		if err != nil {
			return nil, err
		}
		return controller.Strategy(), nil
	}
	if url := optionalString(spec, "bound"); url != nil {
		controller, err := s.controller(stringField(spec, "controller"))
		if err != nil {
			return nil, err
		}
		return controller.CreateHandlerBoundToURL(*url)
	}
	body, statusCode := spec.Get("respond"), spec.Get("status")
	return HandlerFunc(func(HandlerOptions) *Task[Response] {
		response, err := newResponse(body, statusCode)
		if err != nil {
			return failed[Response](err)
		}
		return resolved(response)
	}), nil
}

// routeOptions applies the given PrecacheRoute options over the defaults: an absent member keeps
// the default, and a falsy directoryIndex or cleanURLs disables it.
func (s *scenario) routeOptions(given *js.Object) (PrecacheRouteOptions, error) {
	options := defaultPrecacheRouteOptions()
	if isUndefined(given) {
		return options, nil
	}
	if directoryIndex := given.Get("directoryIndex"); !isUndefined(directoryIndex) {
		options.DirectoryIndex = nil
		if truthy(directoryIndex) {
			options.DirectoryIndex = some(stringOf(directoryIndex))
		}
	}
	if cleanURLs := given.Get("cleanURLs"); !isUndefined(cleanURLs) {
		options.CleanURLs = truthy(cleanURLs)
	}
	if sources := given.Get("ignoreURLParametersMatching"); !isUndefined(sources) {
		options.IgnoreURLParametersMatching = nil
		for _, source := range elements(sources) {
			regexp, err := newRegExp(stringOf(source))
			if err != nil {
				return options, err
			}
			options.IgnoreURLParametersMatching = append(options.IgnoreURLParametersMatching, regexp)
		}
	}
	if suffixes := given.Get("urlManipulation"); !isUndefined(suffixes) {
		list := elements(suffixes)
		options.URLManipulation = func(url URL) []URL {
			var variants []URL
			for _, suffix := range list {
				variant := url.Copy()
				variant.SetPathname(variant.Pathname() + stringOf(suffix))
				variants = append(variants, variant)
			}
			return variants
		}
	}
	return options, nil
}

func (s *scenario) register(step *js.Object, route *Route) (*js.Object, error) {
	if id := optionalString(step, "id"); id != nil {
		s.routes[*id] = route
	}
	router, err := s.router(stringField(step, "router"))
	if err != nil {
		return nil, err
	}
	router.RegisterRoute(route)
	return nil, nil
}

// lifetime runs one event-driven step under a fresh stand-in event and waits for every lifetime it
// was given, in rounds, or, in service-worker mode, under the caller's native event.
func lifetime[T any](s *scenario, eventType string, action func(ExtendableEvent) *Task[T]) *Task[T] {
	if native := s.input.Get("event"); !isUndefined(native) {
		return action(hostEvent{native})
	}
	return start(func(settle func(T, error)) {
		var pending []*js.Object
		action(standInEvent{eventType: eventType, pending: &pending}).then(func(value T, err error) {
			count := 0
			var round func()
			round = func() {
				if len(pending) == 0 {
					s.events.Call("push", newArray(eventType, count))
					settle(value, err)
					return
				}
				batch := pending
				pending = nil
				count += len(batch)
				allSettled(batch).then(func(struct{}, error) { round() })
			}
			round()
		})
	})
}

// step runs one step: its result, or what it threw.
func (s *scenario) step(step *js.Object) *Task[*js.Object] {
	done := func(result *js.Object, err error) *Task[*js.Object] {
		if err != nil {
			return failed[*js.Object](err)
		}
		return resolved(result)
	}
	switch op := stringField(step, "op"); op {
	case "network":
		s.source = stringField(step, "source")
		return done(nil, nil)
	case "controller":
		plugins, err := s.plugins(listField(step, "plugins"))
		if err != nil {
			return done(nil, err)
		}
		// fallbackToNetwork defaults to true; PrecacheStrategy keeps it unless it is false.
		fallback := step.Get("fallbackToNetwork")
		fallbackToNetwork := !(typeTag(fallback) == "[object Boolean]" && !fallback.Bool())
		s.controllers[stringField(step, "id")] = newPrecacheController(s.cacheName(stringField(step, "cacheName")), plugins, fallbackToNetwork)
		return done(nil, nil)
	case "addToCacheList":
		controller, err := s.controller(stringField(step, "controller"))
		if err == nil {
			err = controller.AddToCacheList(listField(step, "entries"))
		}
		return done(nil, err)
	case "addPlugins":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		added, err := s.fixturePlugins(listField(step, "plugins"))
		if err != nil {
			return done(nil, err)
		}
		controller.Strategy().Plugins = append(controller.Strategy().Plugins, added...)
		return done(nil, nil)
	case "install":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		return mapTask(lifetime(s, "install", controller.Install), func(result InstallResult) *js.Object {
			return newObject("updatedURLs", s.urls(result.UpdatedURLs), "notUpdatedURLs", s.urls(result.NotUpdatedURLs))
		})
	case "activate":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		return mapTask(lifetime(s, "activate", controller.Activate), func(result CleanupResult) *js.Object {
			return newObject("deletedURLs", s.urls(result.DeletedURLs))
		})
	case "getCachedURLs":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		return done(s.urls(controller.GetCachedURLs()), nil)
	case "getURLsToCacheKeys":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		out := jsArray.New()
		for _, pair := range controller.GetURLsToCacheKeys() {
			out.Call("push", newArray(s.relative(pair[0]), s.relative(pair[1])))
		}
		return done(out, nil)
	case "getCacheKeyForURL":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		key, ok, err := controller.GetCacheKeyForURL(stringField(step, "url"))
		if err != nil || !ok {
			return done(nil, err)
		}
		return done(jsString(s.relative(key)), nil)
	case "matchPrecache":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		var request any = stringField(step, "url")
		if truthy(step.Get("request")) {
			if request, err = newRequest(stringField(step, "url"), RequestInit{}); err != nil {
				return done(nil, err)
			}
		}
		return s.recordOrNull(controller.MatchPrecache(request))
	case "router":
		s.routers[stringField(step, "id")] = newRouter()
		return done(nil, nil)
	case "precacheRoute":
		controller, err := s.controller(stringField(step, "controller"))
		if err != nil {
			return done(nil, err)
		}
		options, err := s.routeOptions(step.Get("options"))
		if err != nil {
			return done(nil, err)
		}
		return done(s.register(step, newPrecacheRoute(controller, options)))
	case "route":
		var match MatchCallback
		if truthy(step.Get("crossOrigin")) {
			match = func(options MatchOptions) (bool, *Params, error) { return !options.SameOrigin, nil, nil }
		} else {
			prefix := stringField(step, "prefix")
			match = func(options MatchOptions) (bool, *Params, error) {
				return strings.HasPrefix(s.relative(options.URL.Href()), prefix), nil, nil
			}
		}
		handler, err := s.handler(step.Get("handler"))
		if err != nil {
			return done(nil, err)
		}
		return done(s.register(step, newRoute(match, handler, optionalString(step, "method"))))
	case "defaultHandler":
		router, err := s.router(stringField(step, "router"))
		if err != nil {
			return done(nil, err)
		}
		handler, err := s.handler(step.Get("handler"))
		if err != nil {
			return done(nil, err)
		}
		router.SetDefaultHandler(handler, optionalString(step, "method"))
		return done(nil, nil)
	case "catchHandler":
		var target interface{ SetCatchHandler(Handler) }
		var err error
		if id := optionalString(step, "route"); id != nil {
			target, err = s.route(*id)
		} else {
			target, err = s.router(stringField(step, "router"))
		}
		if err != nil {
			return done(nil, err)
		}
		handler, err := s.handler(step.Get("handler"))
		if err != nil {
			return done(nil, err)
		}
		target.SetCatchHandler(handler)
		return done(nil, nil)
	case "unregisterRoute":
		router, err := s.router(stringField(step, "router"))
		if err != nil {
			return done(nil, err)
		}
		route, err := s.route(stringField(step, "route"))
		if err != nil {
			return done(nil, err)
		}
		return done(nil, router.UnregisterRoute(route))
	case "fetch":
		var request Request
		if url := optionalString(step, "url"); url == nil {
			request = Request{s.input.Get("request")}
		} else {
			init := RequestInit{Method: optionalString(step, "method"), Mode: optionalString(step, "mode"), Integrity: optionalString(step, "integrity")}
			var err error
			if request, err = newRequest(*url, init); err != nil {
				return done(nil, err)
			}
		}
		router, err := s.router(stringField(step, "router"))
		if err != nil {
			return done(nil, err)
		}
		return s.recordOrNull(lifetime(s, "fetch", func(event ExtendableEvent) *Task[*Response] {
			response, err := router.HandleRequest(request, event)
			switch {
			case err != nil:
				return failed[*Response](err)
			case response == nil:
				return resolved[*Response](nil)
			}
			return mapTask(response, func(response Response) *Response { return &response })
		}))
	case "caches":
		return s.dump()
	default:
		return done(nil, errorTypeError("unknown step "+op))
	}
}

// mapTask maps a task's value.
func mapTask[T, U any](task *Task[T], convert func(T) U) *Task[U] {
	return start(func(settle func(U, error)) {
		task.then(func(value T, err error) {
			if err != nil {
				var zero U
				settle(zero, err)
				return
			}
			settle(convert(value), nil)
		})
	})
}

// recordOrNull is the response's record, or null for no response.
func (s *scenario) recordOrNull(response *Task[*Response]) *Task[*js.Object] {
	return start(func(settle func(*js.Object, error)) {
		response.then(func(response *Response, err error) {
			if err != nil || response == nil {
				settle(nil, err)
				return
			}
			s.record(*response).then(settle)
		})
	})
}

// report runs every step, recording each result or failure, then dumps the caches.
func (s *scenario) report() *Task[*js.Object] {
	return start(func(settle func(*js.Object, error)) {
		steps := listField(s.input, "steps")
		var each func(int)
		each = func(index int) {
			if index == len(steps) {
				s.dump().then(func(caches *js.Object, err error) {
					if err != nil {
						settle(nil, err)
						return
					}
					settle(newObject("results", s.results, "events", s.events, "handlers", s.handlers, "caches", caches), nil)
				})
				return
			}
			s.step(steps[index]).then(func(result *js.Object, err error) {
				if err != nil {
					result = s.failure(err)
				}
				s.results.Call("push", result)
				each(index + 1)
			})
		}
		each(0)
	})
}

// deleteCaches deletes every cache of the scenario.
func (s *scenario) deleteCaches() *Task[struct{}] {
	return start(func(settle func(struct{}, error)) {
		cachesKeys().then(func(names []string, err error) {
			if err != nil {
				settle(struct{}{}, err)
				return
			}
			var each func(int)
			each = func(index int) {
				if index == len(names) {
					settle(struct{}{}, nil)
					return
				}
				if !s.isOwnCache(names[index]) {
					each(index + 1)
					return
				}
				cachesDelete(names[index]).then(func(_ bool, err error) {
					if err != nil {
						settle(struct{}{}, err)
						return
					}
					each(index + 1)
				})
			}
			each(0)
		})
	})
}

// run runs one corpus script and answers a promise of its output. As the reference's finally, the
// scenario's caches go whatever happened, then the outcome.
func run(input *js.Object) *js.Object {
	s := &scenario{
		input:       input,
		source:      "alpha",
		controllers: map[string]*PrecacheController{},
		routers:     map[string]*Router{},
		routes:      map[string]*Route{},
		results:     jsArray.New(),
		events:      jsArray.New(),
		handlers:    jsArray.New(),
	}
	if base, err := resolveURL("./"); err == nil {
		s.base = base.Href()
	}
	owned := isUndefined(input.Get("cachePrefix"))
	if owned {
		s.prefix = "mires-workbox-" + js.Global.Get("crypto").Call("randomUUID").String()
	} else {
		s.prefix = stringOf(input.Get("cachePrefix"))
	}
	output := start(func(settle func(*js.Object, error)) {
		s.report().then(func(output *js.Object, err error) {
			if !owned {
				settle(output, err)
				return
			}
			s.deleteCaches().then(func(_ struct{}, deleteErr error) {
				if deleteErr != nil {
					err = deleteErr
				}
				settle(output, err)
			})
		})
	})
	return output.promise.Call("then", func() *js.Object {
		if output.err != nil {
			return jsPromise.Call("reject", jsValue(output.err))
		}
		return output.value
	})
}
