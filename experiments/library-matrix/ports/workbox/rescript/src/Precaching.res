// workbox-precaching 7.4.1: `createCacheKey`, `PrecacheController`, `PrecacheStrategy`,
// `PrecacheRoute` with its URL variations, and the precache plugins (cache key, install report,
// fallback, default cacheability and redirect copy).
open Host
open WorkboxCore
open Strategies

let revisionSearchParam = "__WB_REVISION__"

// A manifest entry as the caller wrote it: a URL string, or a `{url, revision, integrity}`
// object. The adapter passes the corpus's values as they are, null included.
type precacheEntry = JSON.t

// JavaScript truthiness of a manifest value.
let truthy = (value: option<JSON.t>) =>
  switch value {
  | None | Some(Null) | Some(Boolean(false)) | Some(String("")) => false
  | Some(Number(number)) => number !== 0. && !Float.isNaN(number)
  | Some(_) => true
  }

let stringField = (entry: dict<JSON.t>, key) =>
  switch entry->Dict.get(key) {
  | Some(String(text)) => Some(text)
  | _ => None
  }

// `createCacheKey(entry)`: the URL resolved against the location, and for a truthy revision a
// cache key that carries it in `__WB_REVISION__`.
let createCacheKey = (entry: precacheEntry) => {
  let unexpectedType = () =>
    WorkboxError.throw(
      "add-to-cache-list-unexpected-type",
      ~details=dict{"entry": entry->toUnknown},
    )
  switch entry {
  | String(url) if url !== "" =>
    let urlObject = Url.makeWithBase(url, Location.href)
    (urlObject->Url.href, urlObject->Url.href)
  | Object(fields) =>
    let revision = fields->Dict.get("revision")
    // `new URL(url, location.href)` converts a URL that is not a string (outside the
    // contract's domain) with ToString.
    let url = switch fields->Dict.get("url") {
    | Some(String(url)) if url !== "" => url
    | Some(url) if truthy(Some(url)) => String.make(url)
    | _ => unexpectedType()
    }
    switch revision {
    | Some(revision) if truthy(Some(revision)) =>
      let cacheKeyURL = Url.makeWithBase(url, Location.href)
      let originalURL = Url.makeWithBase(url, Location.href)
      cacheKeyURL->Url.searchParams->SearchParams.set(revisionSearchParam, revision)
      (cacheKeyURL->Url.href, originalURL->Url.href)
    | _ =>
      let urlObject = Url.makeWithBase(url, Location.href)
      (urlObject->Url.href, urlObject->Url.href)
    }
  | _ => unexpectedType()
  }
}

// The state member the install report plugin keeps.
@get external originalRequest: state => option<unknown> = "originalRequest"
@set external setOriginalRequest: (state, Request.t) => unit = "originalRequest"
@val @scope(("Request", "prototype")) external isRequest: unknown => bool = "isPrototypeOf"
external asRequest: unknown => Request.t = "%identity"

// `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is.
module PrecacheInstallReportPlugin = {
  type t = {updatedURLs: array<string>, notUpdatedURLs: array<string>, plugin: plugin}

  let make = () => {
    let updatedURLs = []
    let notUpdatedURLs = []
    let plugin = {
      handlerWillStart: async ({request, state}) =>
        state->Option.forEach(state => state->setOriginalRequest(request)),
      cachedResponseWillBeUsed: async ({event, state, cachedResponse}) => {
        if event->Event.type_ === "install" {
          switch state->Option.flatMap(originalRequest) {
          | Some(originalRequest) if isRequest(originalRequest) =>
            let url = originalRequest->asRequest->Request.url
            switch cachedResponse {
            | Some(_) => notUpdatedURLs->Array.push(url)
            | None => updatedURLs->Array.push(url)
            }
          | _ => ()
          }
        }
        cachedResponse
      },
    }
    {updatedURLs, notUpdatedURLs, plugin}
  }
}

// `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL. It
// holds the controller's `getCacheKeyForURL`: the controller is being built when it is made.
let precacheCacheKeyPlugin = (~getCacheKeyForURL): plugin => {
  cacheKeyWillBeUsed: async ({request, params}) => {
    let cacheKey = switch params->Option.flatMap(params => params.cacheKey) {
    | Some(cacheKey) if cacheKey !== "" => Some(cacheKey)
    | _ => getCacheKeyForURL(request->Request.url)
    }
    switch cacheKey {
    | Some(cacheKey) => Request.make(cacheKey, {headers: request->Request.headers})
    | None => request
    }
  },
}

module PrecacheStrategy = {
  // `PrecacheStrategy.defaultPrecacheCacheabilityPlugin`: statuses below 400.
  let defaultPrecacheCacheabilityPlugin: plugin = {
    cacheWillUpdate: async ({response}) => response->Response.status >= 400 ? None : Some(response),
  }

  // `PrecacheStrategy.copyRedirectedCacheableResponsesPlugin`.
  let copyRedirectedCacheableResponsesPlugin: plugin = {
    cacheWillUpdate: async ({response}) => Some(
      response->Response.redirected ? await copyResponse(response) : response,
    ),
  }

  // `_useDefaultCacheabilityPluginIfNeeded`: one cacheWillUpdate besides the redirect copy. It
  // adds the default plugin when there is none, and removes it when there are more.
  let useDefaultCacheabilityPluginIfNeeded = (strategy: strategy) => {
    let defaultPluginIndex = ref(None)
    let cacheWillUpdatePluginCount = ref(0)
    strategy.plugins->Array.forEachWithIndex((plugin, index) =>
      if plugin !== copyRedirectedCacheableResponsesPlugin {
        if plugin === defaultPrecacheCacheabilityPlugin {
          defaultPluginIndex := Some(index)
        }
        if plugin.cacheWillUpdate->Option.isSome {
          cacheWillUpdatePluginCount := cacheWillUpdatePluginCount.contents + 1
        }
      }
    )
    if cacheWillUpdatePluginCount.contents === 0 {
      strategy.plugins->Array.push(defaultPrecacheCacheabilityPlugin)
    } else if cacheWillUpdatePluginCount.contents > 1 {
      defaultPluginIndex.contents->Option.forEach(index =>
        strategy.plugins->Array.removeInPlace(index)
      )
    }
  }

  // `_handleInstall`: fetch, then cache, and fail the install when the response may not be cached.
  let handleInstall = async (strategy, request, handler) => {
    useDefaultCacheabilityPluginIfNeeded(strategy)
    let response = await handler->StrategyHandler.fetch(request)
    let wasCached = await handler->StrategyHandler.cachePut(request, response->Response.clone)
    if !wasCached {
      WorkboxError.throw(
        "bad-precaching-response",
        ~details=dict{
          "url": request->Request.url->toUnknown,
          "status": response->Response.status->toUnknown,
        },
      )
    }
    response
  }

  // `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors,
  // and a cache repair only when that integrity vouches for the response.
  let handleFetch = async (strategy: strategy, ~fallbackToNetwork, request, handler: handler) => {
    let params = handler.params->Option.getOr({})
    if !fallbackToNetwork {
      WorkboxError.throw(
        "missing-precache-entry",
        ~details=dict{
          "cacheName": strategy.cacheName->toUnknown,
          "url": request->Request.url->toUnknown,
        },
      )
    }
    let integrityInManifest = params.integrity->Option.filter(integrity => integrity !== "")
    let integrityInRequest = request->Request.integrity
    let noIntegrityConflict =
      integrityInRequest === "" || integrityInManifest == Some(integrityInRequest)
    let cors = request->Request.mode !== "no-cors"
    let integrity = switch (cors, integrityInRequest) {
    | (false, _) => None
    | (true, "") => integrityInManifest
    | (true, integrity) => Some(integrity)
    }
    let response = await handler->StrategyHandler.fetch(
      Request.copy(request, {integrity: ?integrity}),
    )
    if integrityInManifest->Option.isSome && noIntegrityConflict && cors {
      useDefaultCacheabilityPluginIfNeeded(strategy)
      let _wasCached = await handler->StrategyHandler.cachePut(request, response->Response.clone)
    }
    response
  }

  // `_handle`: the cache, then install-time population, then the network fallback.
  let handle = async (strategy, ~fallbackToNetwork, request, handler: handler) =>
    switch await handler->StrategyHandler.cacheMatch(request) {
    | Some(response) => response
    | None =>
      if handler.event->Event.type_ === "install" {
        await handleInstall(strategy, request, handler)
      } else {
        await handleFetch(strategy, ~fallbackToNetwork, request, handler)
      }
    }

  // `new PrecacheStrategy({cacheName, plugins, fallbackToNetwork})`, which appends the redirect
  // copy plugin.
  let make = (~cacheName, ~plugins, ~fallbackToNetwork) => {
    let strategy = Strategy.make(~cacheName, ~plugins, ~handle=(strategy, request, handler) =>
      handle(strategy, ~fallbackToNetwork=fallbackToNetwork !== false, request, handler)
    )
    strategy.plugins->Array.push(copyRedirectedCacheableResponsesPlugin)
    strategy
  }
}

type installResult = {updatedURLs: array<string>, notUpdatedURLs: array<string>}
type cleanupResult = {deletedURLs: array<string>}

// `string | Request`, as matchPrecache takes it.
type requestInfo = Url(string) | Request(Request.t)

module PrecacheController = {
  type t = {
    urlsToCacheKeys: Map.t<string, string>,
    urlsToCacheModes: Map.t<string, string>,
    cacheKeysToIntegrities: Map.t<string, string>,
    strategy: strategy,
  }

  let getCacheKeyForURL = (urlsToCacheKeys, url) =>
    urlsToCacheKeys->Map.get(Url.makeWithBase(url, Location.href)->Url.href)

  // `new PrecacheController({cacheName, plugins, fallbackToNetwork = true})`: a PrecacheStrategy
  // whose plugins end with this controller's cache-key plugin.
  let make = (~cacheName, ~plugins, ~fallbackToNetwork=true) => {
    let urlsToCacheKeys = Map.make()
    let cacheKeyPlugin = precacheCacheKeyPlugin(~getCacheKeyForURL=url =>
      getCacheKeyForURL(urlsToCacheKeys, url)
    )
    {
      urlsToCacheKeys,
      urlsToCacheModes: Map.make(),
      cacheKeysToIntegrities: Map.make(),
      strategy: PrecacheStrategy.make(
        ~cacheName,
        ~plugins=[...plugins, cacheKeyPlugin],
        ~fallbackToNetwork,
      ),
    }
  }

  // `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added.
  // The revisionless warning goes to the console after each entry, as in the production build.
  let addToCacheList = (controller, entries: array<precacheEntry>) => {
    let urlsToWarnAbout = []
    entries->Array.forEach(entry => {
      switch entry {
      | String(url) => urlsToWarnAbout->Array.push(Some(JSON.String(url)))
      | Object(fields) if fields->Dict.get("revision")->Option.isNone =>
        urlsToWarnAbout->Array.push(fields->Dict.get("url"))
      | _ => ()
      }
      let (cacheKey, url) = createCacheKey(entry)
      let cacheMode = switch entry {
      | Object(fields) if truthy(fields->Dict.get("revision")) => "reload"
      | _ => "default"
      }
      switch controller.urlsToCacheKeys->Map.get(url) {
      | Some(firstEntry) if firstEntry !== cacheKey =>
        WorkboxError.throw(
          "add-to-cache-list-conflicting-entries",
          ~details=dict{"firstEntry": firstEntry->toUnknown, "secondEntry": cacheKey->toUnknown},
        )
      | _ => ()
      }
      switch entry {
      | Object(fields) =>
        switch fields->stringField("integrity") {
        | Some(integrity) if integrity !== "" =>
          switch controller.cacheKeysToIntegrities->Map.get(cacheKey) {
          | Some(known) if known !== integrity =>
            WorkboxError.throw(
              "add-to-cache-list-conflicting-integrities",
              ~details=dict{"url": url->toUnknown},
            )
          | _ => ()
          }
          controller.cacheKeysToIntegrities->Map.set(cacheKey, integrity)
        | _ => ()
        }
      | _ => ()
      }
      controller.urlsToCacheKeys->Map.set(url, cacheKey)
      controller.urlsToCacheModes->Map.set(url, cacheMode)
      if urlsToWarnAbout->Array.length > 0 {
        Console.warn(
          `Workbox is precaching URLs without revision info: ${urlsToWarnAbout->Array.joinUnsafe(
              ", ",
            )}
This is generally NOT safe. Learn more at https://bit.ly/wb-precache`,
        )
      }
    })
  }

  // `install(event)`: each URL in turn through the strategy, under a new install report plugin,
  // with the event's lifetime extended until it is done.
  let install = (controller, event) =>
    waitUntil(event, async () => {
      let installReportPlugin = PrecacheInstallReportPlugin.make()
      controller.strategy.plugins->Array.push(installReportPlugin.plugin)
      // The manifest map is iterated live, as `for…of` iterates it.
      let entries = controller.urlsToCacheKeys->Map.entries
      let rec loop = async () =>
        switch entries->Iterator.next {
        | {done: false, value: Some((url, cacheKey))} =>
          let integrity = controller.cacheKeysToIntegrities->Map.get(cacheKey)
          let cacheMode = controller.urlsToCacheModes->Map.get(url)
          let request = Request.make(
            url,
            {?integrity, cache: ?cacheMode, credentials: "same-origin"},
          )
          let (responseDone, handlerDone) = Strategy.handleAll(
            controller.strategy,
            {params: {cacheKey: cacheKey}, request, event},
          )
          let _ = await Promise.all2((responseDone, handlerDone))
          await loop()
        | _ => ()
        }
      await loop()
      {
        updatedURLs: installReportPlugin.updatedURLs,
        notUpdatedURLs: installReportPlugin.notUpdatedURLs,
      }
    })

  // `activate(event)`: deletes every request in the precache whose URL is no current cache key.
  let activate = (controller, event) =>
    waitUntil(event, async () => {
      let cache = await CacheStorage.openCache(controller.strategy.cacheName)
      let currentlyCachedRequests = await cache->Cache.keys
      let expectedCacheKeys = Set.fromIterator(controller.urlsToCacheKeys->Map.values)
      let deletedURLs = []
      let rec loop = async index =>
        switch currentlyCachedRequests[index] {
        | Some(request) =>
          if !(expectedCacheKeys->Set.has(request->Request.url)) {
            let _ = await cache->Cache.delete(request)
            deletedURLs->Array.push(request->Request.url)
          }
          await loop(index + 1)
        | None => ()
        }
      await loop(0)
      {deletedURLs: deletedURLs}
    })

  let getURLsToCacheKeys = controller => controller.urlsToCacheKeys

  let getCachedURLs = controller => controller.urlsToCacheKeys->Map.keys->Array.fromIterator

  // `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations.
  let getCacheKeyForURL = (controller, url) => getCacheKeyForURL(controller.urlsToCacheKeys, url)

  let getIntegrityForCacheKey = (controller, cacheKey) =>
    controller.cacheKeysToIntegrities->Map.get(cacheKey)

  // `matchPrecache(request)`: opens the precache only for a URL in the manifest.
  let matchPrecache = async (controller, request) => {
    let url = switch request {
    | Request(request) => request->Request.url
    | Url(url) => url
    }
    switch controller->getCacheKeyForURL(url) {
    | Some(cacheKey) =>
      let cache = await CacheStorage.openCache(controller.strategy.cacheName)
      await cache->Cache.matchUrl(cacheKey)
    | None => None
    }
  }

  @val @scope("Object") external assignParams: (params, option<params>) => params = "assign"

  // `createHandlerBoundToURL(url)`: a handler callback that answers any request with this
  // precached URL's entry.
  let createHandlerBoundToURL = (controller, url) =>
    switch controller->getCacheKeyForURL(url) {
    | None => WorkboxError.throw("non-precached-url", ~details=dict{"url": url->toUnknown})
    | Some(cacheKey) =>
      Routing.Callback(
        options =>
          Strategy.handle(
            controller.strategy,
            {
              ...options,
              request: Request.fromUrl(url),
              params: assignParams({cacheKey: cacheKey}, options.params),
            },
          ),
      )
    }
}

// `PrecacheFallbackPlugin({fallbackURL, precacheController})`: a failed strategy answers with a
// precached URL, when it is there.
let precacheFallbackPlugin = (~fallbackURL, ~precacheController): plugin => {
  handlerDidError: _ => precacheController->PrecacheController.matchPrecache(Url(fallbackURL)),
}

// The PrecacheRoute options, as `generateURLVariations` destructures them: an absent member takes
// the default, and null disables `directoryIndex` and `cleanURLs`.
type urlManipulationParam = {url: Url.t}
type urlManipulation = urlManipulationParam => array<Url.t>
type precacheRouteOptions = {
  directoryIndex?: Null.t<string>,
  ignoreURLParametersMatching?: array<RegExp.t>,
  cleanURLs?: Null.t<bool>,
  urlManipulation?: urlManipulation,
}

// `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself.
let removeIgnoredSearchParams = (urlObject, ignoreURLParametersMatching) => {
  let searchParams = urlObject->Url.searchParams
  searchParams
  ->SearchParams.keys
  ->Array.fromIterator
  ->Array.forEach(paramName =>
    if ignoreURLParametersMatching->Array.some(regExp => regExp->RegExp.test(paramName)) {
      searchParams->SearchParams.delete(paramName)
    }
  )
  urlObject
}

// `generateURLVariations(url, options)`, consumed lazily: each variation goes to `visit` in
// order until it answers true, and later variations (and `urlManipulation`) are not computed.
let generateURLVariations = (url, options: precacheRouteOptions, visit) => {
  let ignoreURLParametersMatching =
    options.ignoreURLParametersMatching->Option.getOr([
      RegExp.fromString("^utm_"),
      RegExp.fromString("^fbclid$"),
    ])
  let directoryIndex = switch options.directoryIndex {
  | None => Some("index.html")
  | Some(directoryIndex) => directoryIndex->Null.toOption->Option.filter(index => index !== "")
  }
  let cleanURLs = switch options.cleanURLs {
  | None => true
  | Some(cleanURLs) => cleanURLs->Null.toOption->Option.getOr(false)
  }
  let urlObject = Url.makeWithBase(url, Location.href)
  urlObject->Url.setHash("")
  visit(
    urlObject->Url.href,
  ) || // The URL object itself loses its ignored parameters; urlManipulation receives it so.

  {
    let urlWithoutIgnoredParams = removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)
    visit(urlWithoutIgnoredParams->Url.href) ||
    switch directoryIndex {
    | Some(directoryIndex) if urlWithoutIgnoredParams->Url.pathname->String.endsWith("/") =>
      let directoryURL = Url.make(urlWithoutIgnoredParams->Url.href)
      directoryURL->Url.setPathname(directoryURL->Url.pathname ++ directoryIndex)
      visit(directoryURL->Url.href)
    | _ => false
    } ||
    cleanURLs && {
      let cleanURL = Url.make(urlWithoutIgnoredParams->Url.href)
      cleanURL->Url.setPathname(cleanURL->Url.pathname ++ ".html")
      visit(cleanURL->Url.href)
    } ||
    switch options.urlManipulation {
    | Some(urlManipulation) =>
      urlManipulation({url: urlObject})->Array.some(urlToAttempt => visit(urlToAttempt->Url.href))
    | None => false
    }
  }
}

// `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that
// matches the first variation of the request URL in the manifest, with `{cacheKey, integrity}`.
let precacheRoute = (controller: PrecacheController.t, ~options: precacheRouteOptions={}) => {
  let match = ({request}: Routing.matchOptions) => {
    let urlsToCacheKeys = controller->PrecacheController.getURLsToCacheKeys
    let found = ref(None)
    let _ = generateURLVariations(request->Request.url, options, possibleURL =>
      switch urlsToCacheKeys->Map.get(possibleURL) {
      | Some(cacheKey) if cacheKey !== "" =>
        found :=
          Some({
            cacheKey,
            integrity: ?(controller->PrecacheController.getIntegrityForCacheKey(cacheKey)),
          })
        true
      | _ => false
      }
    )
    found.contents
  }
  Routing.Route.make(~match, ~handler=Routing.Strategy(controller.strategy))
}
