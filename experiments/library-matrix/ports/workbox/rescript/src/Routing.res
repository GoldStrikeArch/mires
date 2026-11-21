// workbox-routing 7.4.1: `Route` and `Router` (routes per method, the http(s) guard, matching
// with parameter normalisation, default handlers, route and global catch handlers).
open Host
open Strategies

// A route handler as `normalizeHandler` leaves it: an object with `handle` (a strategy), or a
// handler callback.
type handler =
  | Strategy(strategy)
  | Callback(handlerOptions => promise<Response.t>)

let handle = (handler, options) =>
  switch handler {
  | Strategy(strategy) => Strategy.handle(strategy, options)
  | Callback(callback) => callback(options)
  }

// What a match callback receives.
type matchOptions = {url: Url.t, sameOrigin: bool, request: Request.t, event: Event.t}

module Route = {
  // `match` answers None for no match, else the match's parameters; `{}` stands for a match
  // that is a plain `true`.
  type t = {
    handler: handler,
    match: matchOptions => option<params>,
    method: string,
    mutable catchHandler: option<handler>,
  }

  let make = (~match, ~handler, ~method="GET") => {handler, match, method, catchHandler: None}

  let setCatchHandler = (route, handler) => route.catchHandler = Some(handler)
}

module Router = {
  type t = {
    routes: Map.t<string, array<Route.t>>,
    defaultHandlerMap: Map.t<string, handler>,
    mutable catchHandler: option<handler>,
  }

  let make = () => {routes: Map.make(), defaultHandlerMap: Map.make(), catchHandler: None}

  // `findMatchingRoute`: the first route for the request's method that matches. Parameters that
  // are an empty object (a plain `true`) become undefined.
  let findMatchingRoute = (router, options: matchOptions) => {
    let routes = router.routes->Map.get(options.request->Request.method)->Option.getOr([])
    let rec find = index =>
      switch routes[index] {
      | None => None
      | Some(route) =>
        switch route.match(options) {
        | Some(params) =>
          let isEmpty = params.cacheKey->Option.isNone && params.integrity->Option.isNone
          Some((route, isEmpty ? None : Some(params)))
        | None => find(index + 1)
        }
      }
    find(0)
  }

  // `handleRequest({request, event})`: None when the request is not http(s) or nothing handles
  // it, else the response, behind the catch handlers when there are any.
  let handleRequest = (router, ~request, ~event) => {
    let url = Url.makeWithBase(request->Request.url, Location.href)
    if !(url->Url.protocol->String.startsWith("http")) {
      None
    } else {
      let sameOrigin = url->Url.origin === Location.origin
      let matched = router->findMatchingRoute({event, request, sameOrigin, url})
      let params = matched->Option.flatMap(((_, params)) => params)
      let route = matched->Option.map(((route, _)) => route)
      let handler = switch route {
      | Some(route) => Some(route.handler)
      | None => router.defaultHandlerMap->Map.get(request->Request.method)
      }
      handler->Option.map(handler => {
        let responsePromise = try handler->handle({url, request, event, ?params}) catch {
        | JsExn(error) => rejected(error)
        }
        let catchHandler = route->Option.flatMap(route => route.catchHandler)
        if router.catchHandler->Option.isSome || catchHandler->Option.isSome {
          (
            async () =>
              try await responsePromise catch {
              | JsExn(original) =>
                let error = ref(original)
                let caught = switch catchHandler {
                | Some(catchHandler) =>
                  try Some(await catchHandler->handle({url, request, event, ?params})) catch {
                  // An Error from the route's catch handler replaces the original.
                  | JsExn(catchErr) =>
                    if isError(catchErr) {
                      error := catchErr
                    }
                    None
                  }
                | None => None
                }
                switch (caught, router.catchHandler) {
                | (Some(response), _) => response
                // The global catch handler is the one set when the rejection arrives; it gets no
                // params.
                | (None, Some(globalHandler)) => await globalHandler->handle({url, request, event})
                | (None, None) => JsExn.throw(error.contents)
                }
              }
          )()
        } else {
          responsePromise
        }
      })
    }
  }

  let setDefaultHandler = (router, handler, ~method="GET") =>
    router.defaultHandlerMap->Map.set(method, handler)

  let setCatchHandler = (router, handler) => router.catchHandler = Some(handler)

  // `registerRoute(route)`: after the routes already registered for its method.
  let registerRoute = (router, route: Route.t) =>
    switch router.routes->Map.get(route.method) {
    | Some(routes) => routes->Array.push(route)
    | None => router.routes->Map.set(route.method, [route])
    }

  let unregisterRoute = (router, route: Route.t) => {
    switch router.routes->Map.get(route.method) {
    | None =>
      WorkboxCore.WorkboxError.throw(
        "unregister-route-but-not-found-with-method",
        ~details=dict{"method": route.method->WorkboxCore.toUnknown},
      )
    | Some(routes) =>
      switch routes->Array.indexOfOpt(route) {
      | Some(routeIndex) => routes->Array.removeInPlace(routeIndex)
      | None => WorkboxCore.WorkboxError.throw("unregister-route-route-not-registered")
      }
    }
  }
}
