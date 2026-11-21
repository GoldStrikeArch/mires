// The adapter (CONTRACT.md "Steps", "Fixtures" and "Output"): runs one script of Workbox API calls
// against this port, with the fixture plugins and the stand-in events, and builds the canonical
// output. The scenario's caches are deleted on every path once it has reported, or failed.
open Host
open Strategies

let fixture = "experiments/library-matrix/libraries/workbox/"
// Well-formed SRI metadata that matches no fixture, so the fetch fails integrity.
let wrongIntegrity = "sha256-" ++ String.repeat("A", 43) ++ "="

// The corpus's step objects, read in place.
type pluginSpec = {
  kind: string,
  label?: string,
  on?: string,
  value?: JSON.t,
  url?: string,
  controller?: string,
}
type handlerSpec = {
  strategy?: string,
  cacheName?: string,
  plugins?: array<pluginSpec>,
  precache?: string,
  bound?: string,
  controller?: string,
  respond?: string,
  status?: int,
}
type optionsSpec = {
  directoryIndex?: Null.t<string>,
  cleanURLs?: Null.t<bool>,
  ignoreURLParametersMatching?: array<string>,
  urlManipulation?: array<string>,
}
type step = {
  op: string,
  id?: string,
  cacheName?: string,
  plugins?: array<pluginSpec>,
  fallbackToNetwork?: bool,
  controller?: string,
  entries?: array<Precaching.precacheEntry>,
  url?: string,
  request?: bool,
  router?: string,
  options?: optionsSpec,
  prefix?: string,
  crossOrigin?: bool,
  handler?: handlerSpec,
  method?: string,
  mode?: string,
  integrity?: string,
  route?: string,
  source?: string,
}
type input = {
  steps: array<step>,
  cachePrefix?: string,
  event?: Event.t,
  request?: Request.t,
}

// The stand-in ExtendableEvent of one event-driven step.
type standInEvent = {@as("type") type_: string, waitUntil: promise<unknown> => unit}
external asEvent: standInEvent => Event.t = "%identity"

// The trace plugin's member of its state.
@get external log: state => option<array<JSON.t>> = "log"
@set external setLog: (state, array<JSON.t>) => unit = "log"

// A response as a followed redirect leaves it: it and every clone of it report redirected.
type property<'a> = {value: 'a}
type redirectedProperties = {redirected: property<bool>, clone: property<unit => Response.t>}
@get external cloneMethod: Response.t => unit => Response.t = "clone"
@send external bind: (unit => Response.t, Response.t) => unit => Response.t = "bind"
@val @scope("Object")
external defineProperties: (Response.t, redirectedProperties) => Response.t = "defineProperties"

let rec redirected = response => {
  let clone = response->cloneMethod->bind(response)
  defineProperties(response, {redirected: {value: true}, clone: {value: () => redirected(clone())}})
}

external json: 'a => JSON.t = "%identity"

let getOrFail = (map, key) =>
  map->Map.get(key)->Option.getOrThrow(~message="the script names no " ++ key)

let run = async (input: input) => {
  let base = Url.makeWithBase("./", Location.href)->Url.href
  let owned = input.cachePrefix->Option.isNone
  let prefix = input.cachePrefix->Option.getOr("mires-workbox-" ++ randomUUID())
  let relative = url =>
    url->String.startsWith(base) ? url->String.slice(~start=base->String.length) : url
  let cacheName = name => prefix ++ "-" ++ name
  let cacheLabel = name =>
    name->String.startsWith(prefix ++ "-")
      ? name->String.slice(~start=prefix->String.length + 1)
      : name

  let source = ref("alpha")
  let controllers: Map.t<string, Precaching.PrecacheController.t> = Map.make()
  let routers: Map.t<string, Routing.Router.t> = Map.make()
  let routes: Map.t<string, Routing.Route.t> = Map.make()
  let results = []
  let events = []
  let handlers = []

  // Output records.
  // A thrown non-Error (the fail fixture's `value`) has no name; it prints as its string.
  let nameOf = error => isError(error) ? error->errorName : String.make(error)
  let failure = error =>
    if WorkboxCore.WorkboxError.isInstance(error) {
      let message =
        error->errorMessage->String.replaceAll(base, "")->String.replaceAll(prefix ++ "-", "")
      JSON.Object(dict{"error": JSON.String(error->errorName), "message": JSON.String(message)})
    } else if isError(error) {
      JSON.Object(dict{"error": JSON.String(error->errorName)})
    } else {
      JSON.Object(dict{"thrown": JSON.String(String.make(error))})
    }
  let record = async response => JSON.Object(
    dict{
      "status": json(response->Response.status),
      "url": JSON.String(relative(response->Response.url)),
      "body": JSON.String(await response->Response.text),
    },
  )
  let status = response =>
    switch response {
    | Some(response) => json(response->Response.status)
    | None => JSON.Null
    }
  let urls = urls => JSON.Array(urls->Array.map(url => JSON.String(relative(url))))
  let dump = async () => {
    let out = []
    let names = await CacheStorage.keys()
    let rec dumpCache = async index =>
      switch names[index] {
      | Some(name) =>
        if name->String.startsWith(prefix ++ "-") {
          let cache = await CacheStorage.openCache(name)
          let requests = await cache->Cache.keys
          let entries = []
          let rec dumpEntry = async index =>
            switch requests[index] {
            | Some(request) =>
              let response = switch await cache->Cache.matchRequest(request) {
              | Some(response) => response
              | None => JsError.TypeError.throwWithMessage("a cache entry without a response")
              }
              entries->Array.push(
                JSON.Array([JSON.String(relative(request->Request.url)), await record(response)]),
              )
              await dumpEntry(index + 1)
            | None => ()
            }
          await dumpEntry(0)
          out->Array.push(JSON.Array([JSON.String(cacheLabel(name)), JSON.Array(entries)]))
        }
        await dumpCache(index + 1)
      | None => ()
      }
    await dumpCache(0)
    JSON.Array(out)
  }

  // Fixture plugins (CONTRACT.md "Fixtures").
  let networkPlugin = (): plugin => {
    requestWillFetch: async ({request}) => {
      if source.contents === "offline" {
        JsError.TypeError.throwWithMessage("offline")
      }
      let file = source.contents === "beta" ? "beta" : "alpha"
      let target = Url.makeWithBase(fixture ++ file ++ ".mjs", Location.href)->Url.href
      let integrity = source.contents === "broken" ? wrongIntegrity : request->Request.integrity
      Request.make(target, {integrity: integrity})
    },
    fetchDidSucceed: async ({response}) =>
      switch source.contents {
      | "missing" => Response.make("missing", {status: 404})
      | "redirected" => redirected(response)
      | _ => response
      },
  }
  // Records go only into a log the plugin's own handlerWillStart created: a plugin before it can
  // fail that callback, and the trace must not throw in the callbacks that follow.
  let tracePlugin = (label): plugin => {
    let push = (state, entry) =>
      state->Option.flatMap(log)->Option.forEach(log => log->Array.push(JSON.Array(entry)))
    let url = request => JSON.String(relative(request->Request.url))
    {
      handlerWillStart: async ({request, state}) => {
        let log = []
        state->Option.forEach(state => state->setLog(log))
        handlers->Array.push(JSON.Array([JSON.String(label), url(request), JSON.Array(log)]))
      },
      cacheKeyWillBeUsed: async ({mode, request, state}) => {
        push(
          state,
          [JSON.String("cacheKeyWillBeUsed"), JSON.String((mode :> string)), url(request)],
        )
        request
      },
      cachedResponseWillBeUsed: async ({cacheName, request, cachedResponse, state}) => {
        push(
          state,
          [
            JSON.String("cachedResponseWillBeUsed"),
            JSON.String(cacheLabel(cacheName)),
            url(request),
            status(cachedResponse),
          ],
        )
        cachedResponse
      },
      requestWillFetch: async ({request, state}) => {
        push(state, [JSON.String("requestWillFetch"), url(request)])
        request
      },
      fetchDidSucceed: async ({request, response, state}) => {
        push(state, [JSON.String("fetchDidSucceed"), url(request), status(Some(response))])
        response
      },
      fetchDidFail: async ({originalRequest, request, error, state}) =>
        push(
          state,
          [
            JSON.String("fetchDidFail"),
            url(originalRequest),
            url(request),
            JSON.String(nameOf(error)),
          ],
        ),
      cacheDidUpdate: async ({cacheName, request, oldResponse, newResponse, state}) =>
        push(
          state,
          [
            JSON.String("cacheDidUpdate"),
            JSON.String(cacheLabel(cacheName)),
            url(request),
            status(oldResponse),
            status(Some(newResponse)),
          ],
        ),
      handlerWillRespond: async ({response, state}) => {
        push(state, [JSON.String("handlerWillRespond"), status(Some(response))])
        response
      },
      handlerDidRespond: async ({response, state}) =>
        push(state, [JSON.String("handlerDidRespond"), status(response)]),
      handlerDidComplete: async ({response, error, state}) =>
        push(
          state,
          [
            JSON.String("handlerDidComplete"),
            status(response),
            switch error {
            | Some(error) => JSON.String(nameOf(error))
            | None => JSON.Null
            },
          ],
        ),
      handlerDidError: async ({error, state}) => {
        push(state, [JSON.String("handlerDidError"), JSON.String(nameOf(error))])
        None
      },
    }
  }
  // `fail`: only the callback named `on`, which throws `new Error('fail')`, or `value` itself.
  let failPlugin = (on, value): plugin => {
    let fail = async _ =>
      switch value {
      | Some(value) => JsExn.throw(value)
      | None => JsError.throwWithMessage("fail")
      }
    switch on {
    | "handlerWillStart" => {handlerWillStart: fail}
    | "cacheKeyWillBeUsed" => {cacheKeyWillBeUsed: fail}
    | "cachedResponseWillBeUsed" => {cachedResponseWillBeUsed: fail}
    | "cacheWillUpdate" => {cacheWillUpdate: fail}
    | "cacheDidUpdate" => {cacheDidUpdate: fail}
    | "requestWillFetch" => {requestWillFetch: fail}
    | "fetchDidSucceed" => {fetchDidSucceed: fail}
    | "fetchDidFail" => {fetchDidFail: fail}
    | "handlerWillRespond" => {handlerWillRespond: fail}
    | "handlerDidRespond" => {handlerDidRespond: fail}
    | "handlerDidComplete" => {handlerDidComplete: fail}
    | "handlerDidError" => {handlerDidError: fail}
    | _ => JsError.TypeError.throwWithMessage("unknown callback " ++ on)
    }
  }
  let plugin = (spec: pluginSpec): plugin =>
    switch spec.kind {
    | "trace" => tracePlugin(spec.label->Option.getOr(""))
    | "keep" => {cacheWillUpdate: async ({response}) => Some(response)}
    | "drop" => {cacheWillUpdate: async _ => None}
    | "fail" => failPlugin(spec.on->Option.getOr(""), spec.value)
    | "fallback" =>
      Precaching.precacheFallbackPlugin(
        ~fallbackURL=spec.url->Option.getOr(""),
        ~precacheController=controllers->getOrFail(spec.controller->Option.getOr("")),
      )
    | kind => JsError.TypeError.throwWithMessage("unknown plugin " ++ kind)
    }
  let plugins = specs => [networkPlugin(), ...specs->Option.getOr([])->Array.map(plugin)]

  // Route handlers and options (CONTRACT.md "Steps").
  let handler = (spec: handlerSpec) =>
    switch (spec.strategy, spec.precache, spec.bound) {
    | (Some(name), _, _) =>
      let cacheName = cacheName(spec.cacheName->Option.getOr(""))
      let plugins = plugins(spec.plugins)
      Routing.Strategy(
        switch name {
        | "CacheFirst" => CacheFirst.make(~cacheName, ~plugins)
        | "CacheOnly" => CacheOnly.make(~cacheName, ~plugins)
        | "NetworkFirst" => NetworkFirst.make(~cacheName, ~plugins)
        | "NetworkOnly" => NetworkOnly.make(~cacheName, ~plugins)
        | name => JsError.TypeError.throwWithMessage("unknown strategy " ++ name)
        },
      )
    | (None, Some(id), _) => Routing.Strategy((controllers->getOrFail(id)).strategy)
    | (None, None, Some(url)) =>
      controllers
      ->getOrFail(spec.controller->Option.getOr(""))
      ->Precaching.PrecacheController.createHandlerBoundToURL(url)
    | (None, None, None) =>
      let body = spec.respond->Option.getOr("")
      Routing.Callback(async _ => Response.make(body, {status: ?spec.status}))
    }
  let routeOptions = (options: optionsSpec): Precaching.precacheRouteOptions => {
    directoryIndex: ?options.directoryIndex,
    cleanURLs: ?options.cleanURLs,
    ignoreURLParametersMatching: ?(
      options.ignoreURLParametersMatching->Option.map(sources =>
        sources->Array.map(source => RegExp.fromString(source))
      )
    ),
    urlManipulation: ?(
      options.urlManipulation->Option.map(suffixes =>
        ({url}: Precaching.urlManipulationParam) =>
          suffixes->Array.map(suffix => {
            let variant = Url.make(url->Url.href)
            variant->Url.setPathname(variant->Url.pathname ++ suffix)
            variant
          })
      )
    ),
  }
  let register = (step, route) => {
    step.id->Option.forEach(id => routes->Map.set(id, route))
    routers->getOrFail(step.router->Option.getOr(""))->Routing.Router.registerRoute(route)
    JSON.Null
  }

  // One event per event-driven step: a fresh stand-in event whose lifetime promises the step
  // awaits before it reports, or the host event the caller supplied (service-worker mode).
  let lifetime = async (type_, action) =>
    switch input.event {
    | Some(event) => await action(event)
    | None =>
      let pending = []
      let event = asEvent({type_, waitUntil: promise => pending->Array.push(promise)})
      let outcome = try Ok(await action(event)) catch {
      | JsExn(error) => Error(error)
      }
      let rec settle = async count =>
        if pending->Array.length > 0 {
          let batch = pending->Array.copy
          pending->Array.splice(~start=0, ~remove=batch->Array.length, ~insert=[])
          let _ = await Promise.allSettled(batch)
          await settle(count + batch->Array.length)
        } else {
          count
        }
      let count = await settle(0)
      events->Array.push(JSON.Array([JSON.String(type_), json(count)]))
      switch outcome {
      | Ok(value) => value
      | Error(error) => JsExn.throw(error)
      }
    }

  let controllerOf = id => controllers->getOrFail(id->Option.getOr(""))
  let routerOf = id => routers->getOrFail(id->Option.getOr(""))

  let runStep = async step =>
    switch step.op {
    | "network" =>
      source := step.source->Option.getOr("alpha")
      JSON.Null
    | "controller" =>
      let controller = Precaching.PrecacheController.make(
        ~cacheName=cacheName(step.cacheName->Option.getOr("")),
        ~plugins=plugins(step.plugins),
        ~fallbackToNetwork=?step.fallbackToNetwork,
      )
      controllers->Map.set(step.id->Option.getOr(""), controller)
      JSON.Null
    | "addToCacheList" =>
      controllerOf(step.controller)->Precaching.PrecacheController.addToCacheList(
        step.entries->Option.getOr([]),
      )
      JSON.Null
    | "addPlugins" =>
      let controller = controllerOf(step.controller)
      controller.strategy.plugins->Array.pushMany(step.plugins->Option.getOr([])->Array.map(plugin))
      JSON.Null
    | "install" =>
      let controller = controllerOf(step.controller)
      let {Precaching.updatedURLs: updatedURLs, notUpdatedURLs} = await lifetime("install", event =>
        controller->Precaching.PrecacheController.install(event)
      )
      JSON.Object(dict{"updatedURLs": urls(updatedURLs), "notUpdatedURLs": urls(notUpdatedURLs)})
    | "activate" =>
      let controller = controllerOf(step.controller)
      let {Precaching.deletedURLs: deletedURLs} = await lifetime("activate", event =>
        controller->Precaching.PrecacheController.activate(event)
      )
      JSON.Object(dict{"deletedURLs": urls(deletedURLs)})
    | "getCachedURLs" =>
      urls(controllerOf(step.controller)->Precaching.PrecacheController.getCachedURLs)
    | "getURLsToCacheKeys" =>
      JSON.Array(
        controllerOf(step.controller)
        ->Precaching.PrecacheController.getURLsToCacheKeys
        ->Map.entries
        ->Array.fromIterator
        ->Array.map(((url, key)) => JSON.Array([
          JSON.String(relative(url)),
          JSON.String(relative(key)),
        ])),
      )
    | "getCacheKeyForURL" =>
      switch controllerOf(step.controller)->Precaching.PrecacheController.getCacheKeyForURL(
        step.url->Option.getOr(""),
      ) {
      | Some(key) => JSON.String(relative(key))
      | None => JSON.Null
      }
    | "matchPrecache" =>
      let url = step.url->Option.getOr("")
      let request =
        step.request->Option.getOr(false)
          ? Precaching.Request(Request.fromUrl(url))
          : Precaching.Url(url)
      switch await controllerOf(step.controller)->Precaching.PrecacheController.matchPrecache(
        request,
      ) {
      | Some(response) => await record(response)
      | None => JSON.Null
      }
    | "router" =>
      routers->Map.set(step.id->Option.getOr(""), Routing.Router.make())
      JSON.Null
    | "precacheRoute" =>
      register(
        step,
        Precaching.precacheRoute(
          controllerOf(step.controller),
          ~options=?step.options->Option.map(routeOptions),
        ),
      )
    | "route" =>
      let match = if step.crossOrigin->Option.getOr(false) {
        ({sameOrigin}: Routing.matchOptions) => sameOrigin ? None : Some(({}: params))
      } else {
        let prefix = step.prefix->Option.getOr("")
        ({url}: Routing.matchOptions) =>
          relative(url->Url.href)->String.startsWith(prefix) ? Some(({}: params)) : None
      }
      let handler = handler(step.handler->Option.getOrThrow)
      register(step, Routing.Route.make(~match, ~handler, ~method=?step.method))
    | "defaultHandler" =>
      routerOf(step.router)->Routing.Router.setDefaultHandler(
        handler(step.handler->Option.getOrThrow),
        ~method=?step.method,
      )
      JSON.Null
    | "catchHandler" =>
      switch step.route {
      | None =>
        routerOf(step.router)->Routing.Router.setCatchHandler(
          handler(step.handler->Option.getOrThrow),
        )
      | Some(id) =>
        routes
        ->getOrFail(id)
        ->Routing.Route.setCatchHandler(handler(step.handler->Option.getOrThrow))
      }
      JSON.Null
    | "unregisterRoute" =>
      routerOf(step.router)->Routing.Router.unregisterRoute(
        routes->getOrFail(step.route->Option.getOr("")),
      )
      JSON.Null
    | "fetch" =>
      let request = switch step.url {
      | None => input.request->Option.getOrThrow
      | Some(url) =>
        Request.make(url, {method: ?step.method, mode: ?step.mode, integrity: ?step.integrity})
      }
      let router = routerOf(step.router)
      let response = await lifetime("fetch", async event =>
        switch router->Routing.Router.handleRequest(~request, ~event) {
        | Some(response) => Some(await response)
        | None => None
        }
      )
      switch response {
      | Some(response) => await record(response)
      | None => JSON.Null
      }
    | "caches" => await dump()
    | op => JsError.TypeError.throwWithMessage("unknown step " ++ op)
    }

  let report = async () => {
    let rec runSteps = async index =>
      switch input.steps[index] {
      | Some(step) =>
        let result = try await runStep(step) catch {
        | JsExn(error) => failure(error)
        }
        results->Array.push(result)
        await runSteps(index + 1)
      | None => ()
      }
    await runSteps(0)
    JSON.Object(
      dict{
        "results": JSON.Array(results),
        "events": JSON.Array(events),
        "handlers": JSON.Array(handlers),
        "caches": await dump(),
      },
    )
  }

  // As the reference's `finally`: the scenario's caches go whatever happened, then the outcome.
  let outcome = try Ok(await report()) catch {
  | JsExn(error) => Error(error)
  }
  if owned {
    let names = await CacheStorage.keys()
    let rec deleteCaches = async index =>
      switch names[index] {
      | Some(name) =>
        if name->String.startsWith(prefix ++ "-") {
          let _ = await CacheStorage.delete(name)
        }
        await deleteCaches(index + 1)
      | None => ()
      }
    await deleteCaches(0)
  }
  switch outcome {
  | Ok(output) => output
  | Error(error) => JsExn.throw(error)
  }
}
