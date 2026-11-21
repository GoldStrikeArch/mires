# workbox 7.4.1: Scala.js port

This port is written by hand. It replaces the earlier flattened-scenario port (B9), which was partly generated. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles that `matrix-entry.mjs` links), for the subset in `libraries/workbox/CONTRACT.md`, and of what `libraries/workbox/reference.mjs` does with it. It targets Scala 3.8.4 and Scala.js 1.22.0. The controller, strategy, handler, plugin and router layers are Workbox's own.

| File | What it is |
| --- | --- |
| `src/WorkboxCore.scala` | `WorkboxError` with the production message, `ExtendableEvent`, `waitUntil(event, asyncFn)`, `timeout`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/Strategies.scala` | the `WorkboxPlugin` trait and its callback params, `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, `CacheFirst`, `CacheOnly`, `NetworkFirst`, `NetworkOnly`, `cacheOkAndOpaquePlugin` |
| `src/Precaching.scala` | `PrecacheEntry`, `createCacheKey`, `PrecacheController`, `PrecacheStrategy` and its two static plugins, the cache-key, install-report and fallback plugins, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams` |
| `src/Routing.scala` | `Route` and `Router` |
| `src/Host.scala` | `@js.native` facades for CacheStorage, Cache, fetch, Request, Response, Headers, URL and the JS globals; the execution context, `async`, `requestInit`, the `instanceof Error` guard |
| `src/Main.scala` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup; `run`, the module's only export (`@JSExportTopLevel`) |

## Boundary

The corpus input is the JS object itself, seen through `@js.native` facade traits (no copy). Manifest entries become `PrecacheEntry` values; a revision stays the caller's JS value, because its truthiness decides and the host prints a number revision with ToString (`searchParams.set`). Workbox runs on Scala classes over facades of the host objects. The output is built from JS objects (`js.Dynamic.literal`) and `js.Array`s in the contract's key order, and `run` returns a `js.Promise` converted from a `Future`.

## Structure and idiom

- **Plugins** are the `WorkboxPlugin` trait: twelve callbacks of type `Option[P => Future[R]]`, `None` by default. A plugin defines a callback by overriding it with `Some`, so `hasCallback` is `isDefined`. Each callback takes a case-class param named after Workbox's (`CacheWillUpdateParam`, …), with `state`.
- **`iterateCallbacks`** is an `Iterator` over the indices of `strategy.plugins`, which it reads at every step, so a plugin added while a handler runs is reached, without state. The loops over it (`runCallbacks`, `foldCallbacks`) await each callback before they ask for the next. The plugin state is a `mutable.Map` per plugin present when the handler was created, looked up by identity (`eq`), as Workbox's `MapLikeObject`.
- **Strategies** subclass the abstract `Strategy` and override `_handle`. The module-level plugins are singletons, as in Workbox: `CacheOkAndOpaquePlugin` is an `object`, and PrecacheStrategy's two static plugins live in its companion object, compared with `eq`.
- **Promises.** Workbox's async functions are methods answering a `Future` on `JSExecutionContext.queue`, whose callbacks run as promise jobs. A `Future` is eager and has any number of listeners, as a JS promise does. An async function's synchronous prefix runs at the call, and `async { … }` turns a synchronous throw in it into a failed `Future`, as an async function rejects. Loops of awaits are recursive `flatMap`s, and the `for` comprehensions read as the npm source's `await` sequences. Host promises cross with `toFuture`, and the native event gets `toJSPromise`.
- **Errors.** Workbox's own error is `WorkboxError`, whose `name` is the code. A JS value that was thrown reaches Scala as `js.JavaScriptException` holding it, and the fail fixture throws one, holding a `js.Error` or the non-Error string. `isError` is Workbox's `instanceof Error` guard: the held value `isInstanceOf[js.Error]`, and true for Workbox's own errors. It is applied at each guard the contract lists.
- **Route matches** answer `Option[Params]`. A match without params (`true` or `{}` in Workbox) answers empty `Params`, which `findMatchingRoute` normalises to none, as Workbox normalises `{}` and booleans.
- **`generateURLVariations`** is a lazy `Iterator` whose `++` operands are evaluated only once the earlier variations are used up, so `urlManipulation` runs only when it is reached.
- **RegExp** is `js.RegExp`, the host's, without flags.
- **Maps** are `mutable.LinkedHashMap`, whose insertion order and keep-the-place-on-re-set match JavaScript's `Map`.

## Deviations from the npm source

None changes a result in the parity domain.

- **Nested errors in WorkboxError details.** `JSON.stringify` of a nested WorkboxError prints its own enumerable `name` and `details`, and of a native Error `{}`. A Scala exception has other own properties, so the details hold that printed form (`asDetail`) instead of the error itself. Only the message is observable.
- **Microtask counts.** Every `flatMap` continuation is one promise job, so the number of microtasks between two steps differs from the npm bundle's. The subset observes none: within a handler the trace order follows the code, and the only timer is cachePut's `timeout(0)`, which every microtask-level step precedes either way.
- **`Promise.race([a])`** is `Future.firstCompletedOf`, and `Promise.allSettled` in `doneWaiting` and the stand-in event is `Future.sequence` over outcomes. The first failure of a batch is taken in batch order, as `find` on `allSettled` takes it.
- **`install` iterates a snapshot of the URL map.** Workbox iterates the live `Map`, but steps run one at a time, so no entry can be added during an install.
- **`cache-put-with-no-response` cannot happen**: `cachePut` always has a `Response`.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **`NetworkOnly`** awaits `fetch` directly: `Promise.race` of one promise settles as it does, and the timeout error is unreachable without `networkTimeoutSeconds`.
- **`HandlerCallbackOptions` has no `url`**: no handler in the subset reads it.
- **The revisionless warning** goes to `console.warn` with Workbox's text after each entry, as the production build does.

## Module state

None. The module-level plugins hold no state, as in Workbox. Each `run` builds its own scenario.

## Performance accommodations

None. The artifact is about 325 KB, most of it the Scala collections, `Future` and the Scala.js runtime.
