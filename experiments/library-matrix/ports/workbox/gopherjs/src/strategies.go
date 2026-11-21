// workbox-strategies 7.4.1: the plugin interface, Strategy with its handle/handleAll lifecycle,
// StrategyHandler, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.

package main

import "github.com/gopherjs/gopherjs/js"

// callback names a plugin callback.
type callback int

const (
	handlerWillStart callback = iota
	cacheKeyWillBeUsed
	cachedResponseWillBeUsed
	cacheWillUpdate
	cacheDidUpdate
	requestWillFetch
	fetchDidSucceed
	fetchDidFail
	handlerWillRespond
	handlerDidRespond
	handlerDidComplete
	handlerDidError
)

// Cx is what every callback receives besides its own arguments: the event, and the plugin's state
// in this handler (a plain object, as Workbox's MapLikeObject), which is nil for a plugin the
// handler did not start with.
type Cx struct {
	Event ExtendableEvent
	State *js.Object
}

// Plugin is a WorkboxPlugin: optional lifecycle callbacks, nil where the plugin does not define
// one. Its identity is the pointer.
type Plugin struct {
	HandlerWillStart         func(cx Cx, request Request) *Task[struct{}]
	CacheKeyWillBeUsed       func(cx Cx, mode string, request Request, params *Params) *Task[Request]
	CachedResponseWillBeUsed func(cx Cx, cacheName string, request Request, cachedResponse *Response) *Task[*Response]
	CacheWillUpdate          func(cx Cx, request Request, response Response) *Task[*Response]
	CacheDidUpdate           func(cx Cx, cacheName string, request Request, oldResponse *Response, newResponse Response) *Task[struct{}]
	RequestWillFetch         func(cx Cx, request Request) *Task[Request]
	FetchDidSucceed          func(cx Cx, request Request, response Response) *Task[Response]
	FetchDidFail             func(cx Cx, err error, originalRequest, request Request) *Task[struct{}]
	HandlerWillRespond       func(cx Cx, request Request, response Response) *Task[Response]
	HandlerDidRespond        func(cx Cx, request Request, response *Response) *Task[struct{}]
	HandlerDidComplete       func(cx Cx, request Request, response *Response, err error) *Task[struct{}]
	HandlerDidError          func(cx Cx, request Request, err error) *Task[*Response]
}

// defines is `name in plugin`.
func (p *Plugin) defines(name callback) bool {
	switch name {
	case handlerWillStart:
		return p.HandlerWillStart != nil
	case cacheKeyWillBeUsed:
		return p.CacheKeyWillBeUsed != nil
	case cachedResponseWillBeUsed:
		return p.CachedResponseWillBeUsed != nil
	case cacheWillUpdate:
		return p.CacheWillUpdate != nil
	case cacheDidUpdate:
		return p.CacheDidUpdate != nil
	case requestWillFetch:
		return p.RequestWillFetch != nil
	case fetchDidSucceed:
		return p.FetchDidSucceed != nil
	case fetchDidFail:
		return p.FetchDidFail != nil
	case handlerWillRespond:
		return p.HandlerWillRespond != nil
	case handlerDidRespond:
		return p.HandlerDidRespond != nil
	case handlerDidComplete:
		return p.HandlerDidComplete != nil
	default:
		return p.HandlerDidError != nil
	}
}

// Strategy is a cache name, a plugin list (which Workbox and the caller change), and the
// subclass's _handle, which receives the strategy as `this`.
type Strategy struct {
	CacheName string
	Plugins   []*Plugin
	handle    func(strategy *Strategy, request Request, handler *StrategyHandler) *Task[Response]
}

// newStrategy is new Strategy({cacheName, plugins}). The cache name is always supplied, so
// cacheNames.getRuntimeName(cacheName) returns it.
func newStrategy(cacheName string, plugins []*Plugin, handle func(*Strategy, Request, *StrategyHandler) *Task[Response]) *Strategy {
	return &Strategy{CacheName: cacheName, Plugins: plugins, handle: handle}
}

// StrategyHandler is one request's run through a strategy.
type StrategyHandler struct {
	Request                Request
	Event                  ExtendableEvent
	Params                 *Params
	strategy               *Strategy
	cacheKeys              map[string]Request
	extendLifetimePromises []*Task[struct{}]
	resolveHandler         func()
	pluginStates           []pluginState
}

type pluginState struct {
	plugin *Plugin
	state  *js.Object
}

// newStrategyHandler: a fresh state object for each plugin the strategy has now, then the
// handler's own promise extends the event's lifetime.
func newStrategyHandler(strategy *Strategy, options HandlerOptions) *StrategyHandler {
	handler := &StrategyHandler{
		Request:   options.Request,
		Event:     options.Event,
		Params:    options.Params,
		strategy:  strategy,
		cacheKeys: map[string]Request{},
	}
	for _, plugin := range strategy.Plugins {
		handler.pluginStates = append(handler.pluginStates, pluginState{plugin, jsObject.New()})
	}
	promise, resolve := deferred()
	handler.resolveHandler = resolve
	handler.Event.WaitUntil(promise)
	return handler
}

func (h *StrategyHandler) stateOf(plugin *Plugin) *js.Object {
	for _, known := range h.pluginStates {
		if known.plugin == plugin {
			return known.state
		}
	}
	return nil
}

// nextCallback is one step of iterateCallbacks(name): the next plugin from index on that defines
// the callback, with what it receives, and the index after it. Like the generator's for…of, each
// step reads the strategy's live list, so a plugin added meanwhile is reached, without state.
func (h *StrategyHandler) nextCallback(name callback, index int) (*Plugin, Cx, int, bool) {
	for ; index < len(h.strategy.Plugins); index++ {
		if plugin := h.strategy.Plugins[index]; plugin.defines(name) {
			return plugin, Cx{Event: h.Event, State: h.stateOf(plugin)}, index + 1, true
		}
	}
	return nil, Cx{}, index, false
}

// runCallbacks is runCallbacks(name, param): every callback in turn.
func (h *StrategyHandler) runCallbacks(name callback, call func(*Plugin, Cx) *Task[struct{}]) *Task[struct{}] {
	return start(func(settle func(struct{}, error)) {
		var from func(int)
		from = func(index int) {
			plugin, cx, next, ok := h.nextCallback(name, index)
			if !ok {
				settle(struct{}{}, nil)
				return
			}
			call(plugin, cx).then(func(_ struct{}, err error) {
				if err != nil {
					settle(struct{}{}, err)
					return
				}
				from(next)
			})
		}
		from(0)
	})
}

// chainCallbacks runs the loops that pass each callback's result to the next one.
func chainCallbacks[T any](h *StrategyHandler, name callback, value T, call func(*Plugin, Cx, T) *Task[T]) *Task[T] {
	return start(func(settle func(T, error)) {
		var from func(int, T)
		from = func(index int, value T) {
			plugin, cx, next, ok := h.nextCallback(name, index)
			if !ok {
				settle(value, nil)
				return
			}
			call(plugin, cx, value).then(func(value T, err error) {
				if err != nil {
					settle(value, err)
					return
				}
				from(next, value)
			})
		}
		from(0, value)
	})
}

// hasCallback is hasCallback(name), over the strategy's current plugins.
func (h *StrategyHandler) hasCallback(name callback) bool {
	for _, plugin := range h.strategy.Plugins {
		if plugin.defines(name) {
			return true
		}
	}
	return false
}

// waitUntil extends the handler's lifetime until the task settles.
func handlerWaitUntil[T any](h *StrategyHandler, task *Task[T]) *Task[T] {
	h.extendLifetimePromises = append(h.extendLifetimePromises, start(func(settle func(struct{}, error)) {
		task.then(func(_ T, err error) { settle(struct{}{}, err) })
	}))
	return task
}

// getCacheKey is getCacheKey(request, mode): cacheKeyWillBeUsed, once per URL and mode for this
// handler.
func (h *StrategyHandler) getCacheKey(request Request, mode string) *Task[Request] {
	key := request.URL() + " | " + mode
	if effectiveRequest, ok := h.cacheKeys[key]; ok {
		return resolved(effectiveRequest)
	}
	return start(func(settle func(Request, error)) {
		chainCallbacks(h, cacheKeyWillBeUsed, request, func(plugin *Plugin, cx Cx, request Request) *Task[Request] {
			return plugin.CacheKeyWillBeUsed(cx, mode, request, h.Params)
		}).then(func(effectiveRequest Request, err error) {
			if err != nil {
				settle(Request{}, err)
				return
			}
			h.cacheKeys[key] = effectiveRequest
			settle(effectiveRequest, nil)
		})
	})
}

// fetch is fetch(input): requestWillFetch, the network, then fetchDidSucceed, or fetchDidFail. The
// navigate branches (navigation preload, dropping fetchOptions) are out of the subset.
func (h *StrategyHandler) fetch(input Request) *Task[Response] {
	return start(func(settle func(Response, error)) {
		var originalRequest *Request
		if h.hasCallback(fetchDidFail) {
			clone := input.Clone()
			originalRequest = &clone
		}
		var willFetch func(int, Request)
		willFetch = func(index int, request Request) {
			plugin, cx, next, ok := h.nextCallback(requestWillFetch, index)
			if !ok {
				h.send(request, originalRequest, settle)
				return
			}
			plugin.RequestWillFetch(cx, request.Clone()).then(func(filtered Request, err error) {
				switch {
				case err == nil:
					willFetch(next, filtered)
				case isError(err):
					details := newObject("thrownErrorMessage", jsValue(err).Get("message"))
					settle(Response{}, newWorkboxError("plugin-error-request-will-fetch", details))
				default:
					// A thrown non-Error ends the loop, and the request goes out as it is.
					h.send(request, originalRequest, settle)
				}
			})
		}
		willFetch(0, input)
	})
}

// send is the rest of fetch, after requestWillFetch.
func (h *StrategyHandler) send(request Request, originalRequest *Request, settle func(Response, error)) {
	pluginFilteredRequest := request.Clone()
	failed := func(err error) {
		if originalRequest == nil {
			settle(Response{}, err)
			return
		}
		// One clone of each for every fetchDidFail callback, as runCallbacks passes one param.
		original, filtered := originalRequest.Clone(), pluginFilteredRequest.Clone()
		h.runCallbacks(fetchDidFail, func(plugin *Plugin, cx Cx) *Task[struct{}] {
			return plugin.FetchDidFail(cx, err, original, filtered)
		}).then(func(_ struct{}, callbackErr error) {
			if callbackErr != nil {
				err = callbackErr
			}
			settle(Response{}, err)
		})
	}
	fetch(request).then(func(fetchResponse Response, err error) {
		if err != nil {
			failed(err)
			return
		}
		chainCallbacks(h, fetchDidSucceed, fetchResponse, func(plugin *Plugin, cx Cx, response Response) *Task[Response] {
			return plugin.FetchDidSucceed(cx, pluginFilteredRequest, response)
		}).then(func(response Response, err error) {
			if err != nil {
				failed(err)
				return
			}
			settle(response, nil)
		})
	})
}

// ensureResponseSafeToCache is _ensureResponseSafeToCache(response): each cacheWillUpdate in
// turn, until one refuses; with none, only status 200.
func (h *StrategyHandler) ensureResponseSafeToCache(response Response) *Task[*Response] {
	return start(func(settle func(*Response, error)) {
		var willUpdate func(int, Response, bool)
		willUpdate = func(index int, response Response, pluginsUsed bool) {
			plugin, cx, next, ok := h.nextCallback(cacheWillUpdate, index)
			if !ok {
				if pluginsUsed || response.Status() == 200 {
					settle(&response, nil)
				} else {
					settle(nil, nil)
				}
				return
			}
			plugin.CacheWillUpdate(cx, h.Request, response).then(func(responseToCache *Response, err error) {
				switch {
				case err != nil:
					settle(nil, err)
				case responseToCache == nil:
					settle(nil, nil)
				default:
					willUpdate(next, *responseToCache, true)
				}
			})
		}
		willUpdate(0, response, false)
	})
}

// cacheMatch is cacheMatch(key): caches.match of the read cache key in the strategy's cache, which
// opens no cache, then cachedResponseWillBeUsed.
func (h *StrategyHandler) cacheMatch(key Request) *Task[*Response] {
	cacheName := h.strategy.CacheName
	return start(func(settle func(*Response, error)) {
		h.getCacheKey(key, "read").then(func(effectiveRequest Request, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			cachesMatch(effectiveRequest, cacheName).then(func(cachedResponse *Response, err error) {
				if err != nil {
					settle(nil, err)
					return
				}
				chainCallbacks(h, cachedResponseWillBeUsed, cachedResponse, func(plugin *Plugin, cx Cx, cachedResponse *Response) *Task[*Response] {
					return plugin.CachedResponseWillBeUsed(cx, cacheName, effectiveRequest, cachedResponse)
				}).then(settle)
			})
		})
	})
}

// cachePut is cachePut(key, response): one timer task later, the write cache key, the cacheability
// check, the put and cacheDidUpdate. It answers whether it cached. cache-put-with-no-response
// cannot happen: the response is a Response by type.
func (h *StrategyHandler) cachePut(key Request, response Response) *Task[bool] {
	return start(func(settle func(bool, error)) {
		fail := func(err error) { settle(false, err) }
		timeout(0).then(func(struct{}, error) {
			h.getCacheKey(key, "write").then(func(effectiveRequest Request, err error) {
				if err != nil {
					fail(err)
					return
				}
				h.ensureResponseSafeToCache(response).then(func(responseToCache *Response, err error) {
					switch {
					case err != nil:
						fail(err)
					case responseToCache == nil:
						settle(false, nil)
					default:
						h.store(effectiveRequest, *responseToCache, settle)
					}
				})
			})
		})
	})
}

// store is the rest of cachePut, once the response may be cached.
func (h *StrategyHandler) store(effectiveRequest Request, responseToCache Response, settle func(bool, error)) {
	cacheName := h.strategy.CacheName
	fail := func(err error) { settle(false, err) }
	cachesOpen(cacheName).then(func(cache Cache, err error) {
		if err != nil {
			fail(err)
			return
		}
		hasCacheUpdateCallback := h.hasCallback(cacheDidUpdate)
		oldResponse := resolved[*Response](nil)
		if hasCacheUpdateCallback {
			oldResponse = cacheMatchIgnoreParams(cache, effectiveRequest.Clone(), []string{"__WB_REVISION__"})
		}
		oldResponse.then(func(oldResponse *Response, err error) {
			if err != nil {
				fail(err)
				return
			}
			stored := responseToCache
			if hasCacheUpdateCallback {
				stored = responseToCache.Clone()
			}
			cache.Put(effectiveRequest, stored).then(func(_ struct{}, err error) {
				// An Error is rethrown (a QuotaExceededError after the registered quota callbacks,
				// of which there are none here); any other thrown value is ignored.
				if err != nil && isError(err) {
					fail(err)
					return
				}
				h.runCallbacks(cacheDidUpdate, func(plugin *Plugin, cx Cx) *Task[struct{}] {
					return plugin.CacheDidUpdate(cx, cacheName, effectiveRequest, oldResponse, responseToCache.Clone())
				}).then(func(_ struct{}, err error) { settle(err == nil, err) })
			})
		})
	})
}

// fetchAndCachePut is fetchAndCachePut(input): the response now, and its cachePut as an extended
// lifetime.
func (h *StrategyHandler) fetchAndCachePut(input Request) *Task[Response] {
	return start(func(settle func(Response, error)) {
		h.fetch(input).then(func(response Response, err error) {
			if err != nil {
				settle(Response{}, err)
				return
			}
			responseClone := response.Clone()
			handlerWaitUntil(h, h.cachePut(input, responseClone))
			settle(response, nil)
		})
	})
}

// doneWaiting settles every extended lifetime in batches, including ones added meanwhile, and
// fails with the first rejection of a batch.
func (h *StrategyHandler) doneWaiting() *Task[struct{}] {
	return start(func(settle func(struct{}, error)) {
		var batch func()
		batch = func() {
			promises := h.extendLifetimePromises
			if len(promises) == 0 {
				settle(struct{}{}, nil)
				return
			}
			h.extendLifetimePromises = nil
			var first error
			var each func(int)
			each = func(index int) {
				if index == len(promises) {
					if first != nil {
						settle(struct{}{}, first)
					} else {
						batch()
					}
					return
				}
				promises[index].then(func(_ struct{}, err error) {
					if first == nil {
						first = err
					}
					each(index + 1)
				})
			}
			each(0)
		}
		batch()
	})
}

func (h *StrategyHandler) destroy() { h.resolveHandler() }

// noResponse is new WorkboxError('no-response', {url, error}), where the caught error is kept only
// when it is an Error.
func noResponse(request Request, caught error) error {
	details := newObject("url", request.URL())
	if err := kept(caught); err != nil {
		details.Set("error", jsValue(err))
	}
	return newWorkboxError("no-response", details)
}

// getResponse is _getResponse: handlerWillStart, _handle, handlerDidError on failure,
// handlerWillRespond.
func (s *Strategy) getResponse(handler *StrategyHandler, request Request) *Task[Response] {
	return start(func(settle func(Response, error)) {
		fail := func(err error) { settle(Response{}, err) }
		respond := func(response Response) {
			chainCallbacks(handler, handlerWillRespond, response, func(plugin *Plugin, cx Cx, response Response) *Task[Response] {
				return plugin.HandlerWillRespond(cx, request, response)
			}).then(settle)
		}
		handler.runCallbacks(handlerWillStart, func(plugin *Plugin, cx Cx) *Task[struct{}] {
			return plugin.HandlerWillStart(cx, request)
		}).then(func(_ struct{}, err error) {
			if err != nil {
				fail(err)
				return
			}
			s.handle(s, request, handler).then(func(handled Response, err error) {
				var response *Response
				if err == nil {
					if handled.Type() != "error" {
						respond(handled)
						return
					}
					response = &handled
					err = noResponse(request, nil)
				}
				// A response of type "error" that no handlerDidError replaced is returned, not
				// rethrown.
				var didError func(int, *Response)
				didError = func(index int, response *Response) {
					plugin, cx, next, ok := handler.nextCallback(handlerDidError, index)
					if !ok || !isError(err) {
						if response == nil {
							fail(err)
						} else {
							respond(*response)
						}
						return
					}
					plugin.HandlerDidError(cx, request, err).then(func(replacement *Response, callbackErr error) {
						switch {
						case callbackErr != nil:
							fail(callbackErr)
						case replacement != nil:
							respond(*replacement)
						default:
							didError(next, nil)
						}
					})
				}
				didError(0, response)
			})
		})
	})
}

// awaitComplete is _awaitComplete: handlerDidRespond and the handler's extended lifetimes, then
// handlerDidComplete with the Error among them, if any, then the handler's own promise.
func awaitComplete(responseDone *Task[Response], handler *StrategyHandler, request Request) *Task[struct{}] {
	return start(func(settle func(struct{}, error)) {
		responseDone.then(func(value Response, err error) {
			var response *Response
			if err == nil {
				response = &value
			}
			finish := func(err error) {
				// A thrown non-Error is dropped.
				err = kept(err)
				handler.runCallbacks(handlerDidComplete, func(plugin *Plugin, cx Cx) *Task[struct{}] {
					return plugin.HandlerDidComplete(cx, request, response, err)
				}).then(func(_ struct{}, completeErr error) {
					if completeErr != nil {
						settle(struct{}{}, completeErr)
						return
					}
					handler.destroy()
					settle(struct{}{}, err)
				})
			}
			handler.runCallbacks(handlerDidRespond, func(plugin *Plugin, cx Cx) *Task[struct{}] {
				return plugin.HandlerDidRespond(cx, request, response)
			}).then(func(_ struct{}, err error) {
				if err != nil {
					finish(err)
					return
				}
				handler.doneWaiting().then(func(_ struct{}, err error) { finish(err) })
			})
		})
	})
}

// handleAll is handleAll(options): a handler for the request, and two tasks that both start now:
// the response, and the handler's completion.
func (s *Strategy) handleAll(options HandlerOptions) (*Task[Response], *Task[struct{}]) {
	handler := newStrategyHandler(s, options)
	responseDone := s.getResponse(handler, options.Request)
	handlerDone := awaitComplete(responseDone, handler, options.Request)
	return responseDone, handlerDone
}

// Handle is handle(options): the response; the completion keeps running.
func (s *Strategy) Handle(options HandlerOptions) *Task[Response] {
	responseDone, _ := s.handleAll(options)
	return responseDone
}

// newCacheFirst is new CacheFirst({cacheName, plugins}).
func newCacheFirst(cacheName string, plugins []*Plugin) *Strategy {
	return newStrategy(cacheName, plugins, func(_ *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
		return start(func(settle func(Response, error)) {
			handler.cacheMatch(request).then(func(cached *Response, err error) {
				switch {
				case err != nil:
					settle(Response{}, err)
				case cached != nil:
					settle(*cached, nil)
				default:
					handler.fetchAndCachePut(request).then(func(response Response, err error) {
						if err != nil {
							settle(Response{}, noResponse(request, err))
							return
						}
						settle(response, nil)
					})
				}
			})
		})
	})
}

// newCacheOnly is new CacheOnly({cacheName, plugins}).
func newCacheOnly(cacheName string, plugins []*Plugin) *Strategy {
	return newStrategy(cacheName, plugins, func(_ *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
		return start(func(settle func(Response, error)) {
			handler.cacheMatch(request).then(func(cached *Response, err error) {
				switch {
				case err != nil:
					settle(Response{}, err)
				case cached == nil:
					settle(Response{}, noResponse(request, nil))
				default:
					settle(*cached, nil)
				}
			})
		})
	})
}

// cacheOkAndOpaquePlugin accepts status 200, or 0 for an opaque response.
var cacheOkAndOpaquePlugin = &Plugin{CacheWillUpdate: func(_ Cx, _ Request, response Response) *Task[*Response] {
	if response.Status() == 200 || response.Status() == 0 {
		return resolved(&response)
	}
	return resolved[*Response](nil)
}}

// newNetworkFirst is new NetworkFirst({cacheName, plugins}), without networkTimeoutSeconds. The
// constructor puts its 200-or-opaque plugin first unless a plugin has a cacheWillUpdate.
func newNetworkFirst(cacheName string, plugins []*Plugin) *Strategy {
	strategy := newStrategy(cacheName, plugins, networkFirstHandle)
	for _, plugin := range strategy.Plugins {
		if plugin.defines(cacheWillUpdate) {
			return strategy
		}
	}
	strategy.Plugins = append([]*Plugin{cacheOkAndOpaquePlugin}, strategy.Plugins...)
	return strategy
}

// getNetworkPromise is _getNetworkPromise: fetchAndCachePut, and the cache when that failed or
// answered nothing. There is no timeout to clear. Only an Error is kept from the catch, but both
// branches go to the cache.
func getNetworkPromise(request Request, handler *StrategyHandler) *Task[*Response] {
	return start(func(settle func(*Response, error)) {
		handler.fetchAndCachePut(request).then(func(response Response, err error) {
			if err == nil {
				settle(&response, nil)
				return
			}
			handler.cacheMatch(request).then(settle)
		})
	})
}

// networkFirstHandle is _handle: the network task raced alone (Promise.race([a]) settles as a
// does), then itself again, and both waits extend the handler's lifetime.
func networkFirstHandle(_ *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
	return start(func(settle func(Response, error)) {
		networkPromise := getNetworkPromise(request, handler)
		raced := start(func(settle func(*Response, error)) {
			handlerWaitUntil(handler, networkPromise).then(func(response *Response, err error) {
				if err != nil || response != nil {
					settle(response, err)
					return
				}
				networkPromise.then(settle)
			})
		})
		handlerWaitUntil(handler, raced).then(func(response *Response, err error) {
			switch {
			case err != nil:
				settle(Response{}, err)
			case response == nil:
				settle(Response{}, noResponse(request, nil))
			default:
				settle(*response, nil)
			}
		})
	})
}

// newNetworkOnly is new NetworkOnly({cacheName, plugins}), without networkTimeoutSeconds: the race
// holds the fetch alone.
func newNetworkOnly(cacheName string, plugins []*Plugin) *Strategy {
	return newStrategy(cacheName, plugins, func(_ *Strategy, request Request, handler *StrategyHandler) *Task[Response] {
		return start(func(settle func(Response, error)) {
			handler.fetch(request).then(func(response Response, err error) {
				if err != nil {
					settle(Response{}, noResponse(request, err))
					return
				}
				settle(response, nil)
			})
		})
	})
}
