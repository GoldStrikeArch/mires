// workbox-strategies 7.4.1: the plugin interface, `Strategy` with its `handle`/`handleAll`
// lifecycle, `StrategyHandler`, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly
// strategies.
open Host
open WorkboxCore

// The `mode` a cacheKeyWillBeUsed callback receives.
type mode = [#read | #write]

// The plain object a handler keeps for one plugin (`state` in every callback), as Workbox's
// MapLikeObject is. Each plugin reads and writes its own members.
type state
external toState: {..} => state = "%identity"

// Route parameters: a PrecacheRoute match (`{cacheKey, integrity}`), or the `{cacheKey}` that
// install and createHandlerBoundToURL pass.
type params = {cacheKey?: string, integrity?: string}

// The parameters of each plugin callback, with the plugin's state in this handler.
type handlerWillStartParam = {request: Request.t, event: Event.t, state: option<state>}
type cacheKeyWillBeUsedParam = {
  mode: mode,
  request: Request.t,
  event: Event.t,
  params: option<params>,
  state: option<state>,
}
type cachedResponseWillBeUsedParam = {
  cacheName: string,
  cachedResponse: option<Response.t>,
  request: Request.t,
  event: Event.t,
  state: option<state>,
}
type cacheWillUpdateParam = {
  request: Request.t,
  response: Response.t,
  event: Event.t,
  state: option<state>,
}
type cacheDidUpdateParam = {
  cacheName: string,
  oldResponse: option<Response.t>,
  newResponse: Response.t,
  request: Request.t,
  event: Event.t,
  state: option<state>,
}
type requestWillFetchParam = {request: Request.t, event: Event.t, state: option<state>}
type fetchDidSucceedParam = {
  event: Event.t,
  request: Request.t,
  response: Response.t,
  state: option<state>,
}
type fetchDidFailParam = {
  error: JsExn.t,
  event: Event.t,
  originalRequest: Request.t,
  request: Request.t,
  state: option<state>,
}
type handlerWillRespondParam = {
  event: Event.t,
  request: Request.t,
  response: Response.t,
  state: option<state>,
}
type handlerDidRespondParam = {
  event: Event.t,
  request: Request.t,
  response: option<Response.t>,
  state: option<state>,
}
type handlerDidCompleteParam = {
  event: Event.t,
  request: Request.t,
  response: option<Response.t>,
  error: option<JsExn.t>,
  state: option<state>,
}
type handlerDidErrorParam = {
  error: JsExn.t,
  event: Event.t,
  request: Request.t,
  state: option<state>,
}

// `WorkboxPlugin`: an object of optional lifecycle callbacks. Workbox calls only the callbacks a
// plugin defines; each receives its parameters and the plugin's state in this handler.
type plugin = {
  handlerWillStart?: handlerWillStartParam => promise<unit>,
  cacheKeyWillBeUsed?: cacheKeyWillBeUsedParam => promise<Request.t>,
  cachedResponseWillBeUsed?: cachedResponseWillBeUsedParam => promise<option<Response.t>>,
  cacheWillUpdate?: cacheWillUpdateParam => promise<option<Response.t>>,
  cacheDidUpdate?: cacheDidUpdateParam => promise<unit>,
  requestWillFetch?: requestWillFetchParam => promise<Request.t>,
  fetchDidSucceed?: fetchDidSucceedParam => promise<Response.t>,
  fetchDidFail?: fetchDidFailParam => promise<unit>,
  handlerWillRespond?: handlerWillRespondParam => promise<Response.t>,
  handlerDidRespond?: handlerDidRespondParam => promise<unit>,
  handlerDidComplete?: handlerDidCompleteParam => promise<unit>,
  handlerDidError?: handlerDidErrorParam => promise<option<Response.t>>,
}

// `Strategy`: its cache name, its plugin list (which Workbox and the caller mutate), and the
// subclass's `_handle`, which receives the strategy as `this`.
type rec strategy = {
  cacheName: string,
  plugins: array<plugin>,
  _handle: (strategy, Request.t, handler) => promise<Response.t>,
}
// `StrategyHandler`: one request's run through a strategy.
and handler = {
  request: Request.t,
  event: Event.t,
  params: option<params>,
  strategy: strategy,
  cacheKeys: dict<Request.t>,
  mutable extendLifetimePromises: array<promise<unknown>>,
  handlerDeferred: Deferred.t,
  pluginStateMap: Map.t<plugin, state>,
}

// What `handle` and `handleAll` take (`HandlerCallbackOptions`).
type handlerOptions = {url?: Url.t, request: Request.t, event: Event.t, params?: params}

external erase: promise<'a> => promise<unknown> = "%identity"

module StrategyHandler = {
  // The constructor: a fresh state object for each plugin the strategy has now (a plugin added
  // later gets none), then the handler's own promise extends the event's lifetime.
  let make = (strategy: strategy, options: handlerOptions) => {
    let handler = {
      request: options.request,
      event: options.event,
      params: options.params,
      strategy,
      cacheKeys: Dict.make(),
      extendLifetimePromises: [],
      handlerDeferred: Deferred.make(),
      pluginStateMap: Map.make(),
    }
    strategy.plugins
    ->Array.copy
    ->Array.forEach(plugin => handler.pluginStateMap->Map.set(plugin, Object.make()->toState))
    handler.event->Event.waitUntil(handler.handlerDeferred.promise)
    handler
  }

  // `iterateCallbacks(name)`: the plugins that define the callback, in order. Like the
  // generator's `for…of`, each step reads the strategy's live list, so a plugin added meanwhile
  // is reached, with its state, which is undefined for a plugin the handler did not start with.
  type callbacks<'callback> = {
    handler: handler,
    select: plugin => option<'callback>,
    mutable index: int,
  }

  let iterateCallbacks = (handler: handler, select) => {handler, select, index: 0}

  let rec nextCallback = callbacks =>
    switch callbacks.handler.strategy.plugins[callbacks.index] {
    | None => None
    | Some(plugin) =>
      callbacks.index = callbacks.index + 1
      switch callbacks.select(plugin) {
      | Some(callback) => Some((callback, callbacks.handler.pluginStateMap->Map.get(plugin)))
      | None => nextCallback(callbacks)
      }
    }

  // `runCallbacks(name, param)`: every callback in turn.
  let rec runCallbacks = async (callbacks, call) =>
    switch callbacks->nextCallback {
    | Some((callback, state)) =>
      await call(callback, state)
      await callbacks->runCallbacks(call)
    | None => ()
    }

  // The loops that pass each callback's result to the next one.
  let rec chainCallbacks = async (callbacks, value, call) =>
    switch callbacks->nextCallback {
    | Some((callback, state)) =>
      await callbacks->chainCallbacks(await call(callback, state, value), call)
    | None => value
    }

  // `hasCallback(name)`: `name in plugin`, over the strategy's current plugins.
  let hasCallback = (handler: handler, select) =>
    handler.strategy.plugins->Array.some(plugin => select(plugin)->Option.isSome)

  // `waitUntil(promise)`: extends the handler's lifetime until the promise settles.
  let waitUntil = (handler: handler, promise) => {
    handler.extendLifetimePromises->Array.push(promise->erase)
    promise
  }

  // `getCacheKey(request, mode)`: cacheKeyWillBeUsed, once per URL and mode for this handler.
  let getCacheKey = async (handler: handler, request, mode: mode) => {
    let key = `${request->Request.url} | ${(mode :> string)}`
    switch handler.cacheKeys->Dict.get(key) {
    | Some(effectiveRequest) => effectiveRequest
    | None =>
      let effectiveRequest = await handler
      ->iterateCallbacks(plugin => plugin.cacheKeyWillBeUsed)
      ->chainCallbacks(request, (callback, state, request) =>
        callback({mode, request, event: handler.event, params: handler.params, state})
      )
      handler.cacheKeys->Dict.set(key, effectiveRequest)
      effectiveRequest
    }
  }

  // `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed, or fetchDidFail.
  let fetch = async (handler: handler, input) => {
    let event = handler.event
    let request = ref(input)
    // The `navigate` branches (navigation preload, dropping fetchOptions) are out of the subset.
    let originalRequest =
      handler->hasCallback(plugin => plugin.fetchDidFail) ? Some(input->Request.clone) : None
    try {
      let callbacks = handler->iterateCallbacks(plugin => plugin.requestWillFetch)
      let rec loop = async () =>
        switch callbacks->nextCallback {
        | Some((callback, state)) =>
          request := (await callback({request: request.contents->Request.clone, event, state}))
          await loop()
        | None => ()
        }
      await loop()
    } catch {
    // A thrown non-Error ends the loop, and the request goes out as it is.
    | JsExn(error) =>
      if isError(error) {
        WorkboxError.throw(
          "plugin-error-request-will-fetch",
          ~details=dict{"thrownErrorMessage": error->errorMessage->toUnknown},
        )
      }
    }
    let pluginFilteredRequest = request.contents->Request.clone
    try {
      let fetchResponse = await Host.fetch(request.contents)
      await handler
      ->iterateCallbacks(plugin => plugin.fetchDidSucceed)
      ->chainCallbacks(fetchResponse, (callback, state, response) =>
        callback({event, request: pluginFilteredRequest, response, state})
      )
    } catch {
    | JsExn(error) =>
      switch originalRequest {
      | Some(originalRequest) =>
        // One clone of each for every fetchDidFail callback, as runCallbacks passes one param.
        let originalRequest = originalRequest->Request.clone
        let request = pluginFilteredRequest->Request.clone
        await handler
        ->iterateCallbacks(plugin => plugin.fetchDidFail)
        ->runCallbacks((callback, state) =>
          callback({error, event, originalRequest, request, state})
        )
      | None => ()
      }
      JsExn.throw(error)
    }
  }

  // `_ensureResponseSafeToCache(response)`: each cacheWillUpdate in turn, until one refuses; with
  // none, only status 200.
  let ensureResponseSafeToCache = async (handler: handler, response) => {
    let callbacks = handler->iterateCallbacks(plugin => plugin.cacheWillUpdate)
    let pluginsUsed = ref(false)
    let rec loop = async responseToCache =>
      switch callbacks->nextCallback {
      | Some((callback, state)) =>
        let responseToCache = await callback({
          request: handler.request,
          response: responseToCache,
          event: handler.event,
          state,
        })
        pluginsUsed := true
        switch responseToCache {
        | Some(responseToCache) => await loop(responseToCache)
        | None => None
        }
      | None => Some(responseToCache)
      }
    let responseToCache = await loop(response)
    switch responseToCache {
    | Some(response) if !pluginsUsed.contents && response->Response.status !== 200 => None
    | responseToCache => responseToCache
    }
  }

  // `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache, which opens
  // no cache, then cachedResponseWillBeUsed.
  let cacheMatch = async (handler: handler, key) => {
    let {cacheName} = handler.strategy
    let effectiveRequest = await handler->getCacheKey(key, #read)
    let cachedResponse = await CacheStorage.matchRequest(effectiveRequest, {cacheName: cacheName})
    await handler
    ->iterateCallbacks(plugin => plugin.cachedResponseWillBeUsed)
    ->chainCallbacks(cachedResponse, (callback, state, cachedResponse) =>
      callback({cacheName, cachedResponse, request: effectiveRequest, event: handler.event, state})
    )
  }

  // `cachePut(key, response)`: one timer task later, the write cache key, the cacheability check,
  // the put and cacheDidUpdate. Answers whether it cached.
  let cachePut = async (handler: handler, key, response) => {
    await timeout(0)
    let effectiveRequest = await handler->getCacheKey(key, #write)
    // `cache-put-with-no-response` cannot happen: the response is a Response by type.
    switch await handler->ensureResponseSafeToCache(response) {
    | None => false
    | Some(responseToCache) =>
      let {cacheName} = handler.strategy
      let cache = await CacheStorage.openCache(cacheName)
      let hasCacheUpdateCallback = handler->hasCallback(plugin => plugin.cacheDidUpdate)
      let oldResponse = hasCacheUpdateCallback
        ? await cacheMatchIgnoreParams(cache, effectiveRequest->Request.clone, ["__WB_REVISION__"])
        : None
      try {
        await cache->Cache.put(
          effectiveRequest,
          hasCacheUpdateCallback ? responseToCache->Response.clone : responseToCache,
        )
      } catch {
      // An Error is rethrown (a QuotaExceededError after the registered quota callbacks, of
      // which there are none here); any other thrown value is ignored.
      | JsExn(error) =>
        if isError(error) {
          JsExn.throw(error)
        }
      }
      await handler
      ->iterateCallbacks(plugin => plugin.cacheDidUpdate)
      ->runCallbacks((callback, state) =>
        callback({
          cacheName,
          oldResponse,
          newResponse: responseToCache->Response.clone,
          request: effectiveRequest,
          event: handler.event,
          state,
        })
      )
      true
    }
  }

  // `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime.
  let fetchAndCachePut = async (handler: handler, input) => {
    let response = await handler->fetch(input)
    let responseClone = response->Response.clone
    handler->waitUntil(handler->cachePut(input, responseClone))->ignore
    response
  }

  // `doneWaiting()`: settles every extended lifetime in batches, including ones added meanwhile,
  // and throws the first rejection of a batch.
  let rec doneWaiting = async (handler: handler) =>
    if handler.extendLifetimePromises->Array.length > 0 {
      let promises = handler.extendLifetimePromises
      handler.extendLifetimePromises = []
      let result = await Promise.allSettled(promises)
      switch result->Array.find(outcome =>
        switch outcome {
        | Rejected(_) => true
        | Fulfilled(_) => false
        }
      ) {
      | Some(Rejected({reason})) => throw(reason)
      | _ => await handler->doneWaiting
      }
    }

  let destroy = (handler: handler) => handler.handlerDeferred.resolve()
}

module Strategy = {
  // `new Strategy({cacheName, plugins})`. The cache name is always supplied, so
  // `cacheNames.getRuntimeName(cacheName)` returns it.
  let make = (~cacheName, ~plugins, ~handle) => {cacheName, plugins, _handle: handle}

  // `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond.
  let getResponse = async (handler: handler, request, event) => {
    await handler
    ->StrategyHandler.iterateCallbacks(plugin => plugin.handlerWillStart)
    ->StrategyHandler.runCallbacks((callback, state) => callback({event, request, state}))
    // `response` as `_handle` left it: a response of type "error" stays it.
    let handled = ref(None)
    let response = try {
      let response = await handler.strategy._handle(handler.strategy, request, handler)
      handled := Some(response)
      if response->Response.type_ === "error" {
        WorkboxError.throw("no-response", ~details=dict{"url": request->Request.url->toUnknown})
      }
      response
    } catch {
    | JsExn(error) =>
      // handlerDidError, each assigning the response, until one returns one.
      let callbacks = handler->StrategyHandler.iterateCallbacks(plugin => plugin.handlerDidError)
      let rec didError = async response =>
        switch callbacks->StrategyHandler.nextCallback {
        | Some((callback, state)) =>
          switch await callback({error, event, request, state}) {
          | Some(_) as response => response
          | None => await didError(None)
          }
        | None => response
        }
      let response = isError(error) ? await didError(handled.contents) : handled.contents
      // A response of type "error" that no handlerDidError replaced is returned, not rethrown.
      switch response {
      | Some(response) => response
      | None => JsExn.throw(error)
      }
    }
    await handler
    ->StrategyHandler.iterateCallbacks(plugin => plugin.handlerWillRespond)
    ->StrategyHandler.chainCallbacks(response, (callback, state, response) =>
      callback({event, request, response, state})
    )
  }

  // `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
  // handlerDidComplete with the Error among them, if any, then the handler's own promise.
  let awaitComplete = async (responseDone, handler: handler, request, event) => {
    let response = try Some(await responseDone) catch {
    | JsExn(_) => None
    }
    let error = try {
      await handler
      ->StrategyHandler.iterateCallbacks(plugin => plugin.handlerDidRespond)
      ->StrategyHandler.runCallbacks((callback, state) =>
        callback({event, request, response, state})
      )
      await handler->StrategyHandler.doneWaiting
      None
    } catch {
    // A thrown non-Error is dropped.
    | JsExn(waitUntilError) => isError(waitUntilError) ? Some(waitUntilError) : None
    }
    await handler
    ->StrategyHandler.iterateCallbacks(plugin => plugin.handlerDidComplete)
    ->StrategyHandler.runCallbacks((callback, state) =>
      callback({event, request, response, error, state})
    )
    handler->StrategyHandler.destroy
    switch error {
    | Some(error) => JsExn.throw(error)
    | None => ()
    }
  }

  // `handleAll(options)`: a handler for the request, and two promises that both start now: the
  // response, and the handler's completion.
  let handleAll = (strategy, options: handlerOptions) => {
    let {event, request} = options
    let handler = StrategyHandler.make(strategy, options)
    let responseDone = getResponse(handler, request, event)
    let handlerDone = awaitComplete(responseDone, handler, request, event)
    (responseDone, handlerDone)
  }

  // `handle(options)`: the response; the completion keeps running.
  let handle = (strategy, options) => {
    let (responseDone, _) = handleAll(strategy, options)
    responseDone
  }
}

let noResponse = (request, ~error=?) => {
  let details = dict{"url": request->Request.url->toUnknown}
  error->Option.forEach(error => details->Dict.set("error", error->toUnknown))
  WorkboxError.throw("no-response", ~details)
}

module CacheFirst = {
  let handle = async (_, request, handler) => {
    let error = ref(None)
    let response = switch await handler->StrategyHandler.cacheMatch(request) {
    | Some(_) as response => response
    | None =>
      try Some(await handler->StrategyHandler.fetchAndCachePut(request)) catch {
      | JsExn(caught) =>
        if isError(caught) {
          error := Some(caught)
        }
        None
      }
    }
    switch response {
    | Some(response) => response
    | None => noResponse(request, ~error=?error.contents)
    }
  }

  let make = (~cacheName, ~plugins) => Strategy.make(~cacheName, ~plugins, ~handle)
}

module CacheOnly = {
  let handle = async (_, request, handler) =>
    switch await handler->StrategyHandler.cacheMatch(request) {
    | Some(response) => response
    | None => noResponse(request)
    }

  let make = (~cacheName, ~plugins) => Strategy.make(~cacheName, ~plugins, ~handle)
}

// `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response.
let cacheOkAndOpaquePlugin: plugin = {
  cacheWillUpdate: async ({response}) =>
    response->Response.status === 200 || response->Response.status === 0 ? Some(response) : None,
}

module NetworkFirst = {
  // `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing.
  // There is no timeout to clear.
  let getNetworkPromise = async (request, handler) => {
    let error = ref(None)
    let response = try Some(await handler->StrategyHandler.fetchAndCachePut(request)) catch {
    | JsExn(caught) =>
      if isError(caught) {
        error := Some(caught)
      }
      None
    }
    switch (error.contents, response) {
    | (None, Some(_)) => response
    | _ => await handler->StrategyHandler.cacheMatch(request)
    }
  }

  // `_handle` without networkTimeoutSeconds: the network promise raced alone, then itself again,
  // and both waits extend the handler's lifetime.
  let handle = async (_, request, handler) => {
    let promises = []
    let networkPromise = getNetworkPromise(request, handler)
    promises->Array.push(networkPromise)
    let response = await handler->StrategyHandler.waitUntil(
      (
        async () =>
          switch await handler->StrategyHandler.waitUntil(Promise.race(promises)) {
          | Some(_) as response => response
          | None => await networkPromise
          }
      )(),
    )
    switch response {
    | Some(response) => response
    | None => noResponse(request)
    }
  }

  // The constructor puts its 200-or-opaque plugin first unless a plugin has a cacheWillUpdate.
  let make = (~cacheName, ~plugins) => {
    let strategy = Strategy.make(~cacheName, ~plugins, ~handle)
    if !(strategy.plugins->Array.some(plugin => plugin.cacheWillUpdate->Option.isSome)) {
      strategy.plugins->Array.unshift(cacheOkAndOpaquePlugin)
    }
    strategy
  }
}

module NetworkOnly = {
  // `_handle` without networkTimeoutSeconds: the race holds the fetch alone.
  let handle = async (_, request, handler) => {
    let error = ref(None)
    let response = try {
      let promises = [handler->StrategyHandler.fetch(request)]
      Some(await Promise.race(promises))
    } catch {
    | JsExn(caught) =>
      if isError(caught) {
        error := Some(caught)
      }
      None
    }
    switch response {
    | Some(response) => response
    | None => noResponse(request, ~error=?error.contents)
    }
  }

  let make = (~cacheName, ~plugins) => Strategy.make(~cacheName, ~plugins, ~handle)
}
