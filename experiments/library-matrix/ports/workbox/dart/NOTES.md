# workbox 7.4.1: Dart port

This port is written by hand. It replaces the earlier flattened-scenario port (B9), which was partly generated. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles that `matrix-entry.mjs` links), for the subset in `libraries/workbox/CONTRACT.md`, and of what `libraries/workbox/reference.mjs` does with it. It targets Dart 3 compiled by dart2js. The controller, strategy, handler, plugin and router layers are Workbox's own.

| File | What it is |
| --- | --- |
| `src/workbox.dart` | the library: workbox-core, -strategies, -precaching and -routing as its parts |
| `src/workbox_core.dart` | `WorkboxError` with the production message, `ExtendableEvent`, `waitUntil(event, asyncFn)`, `timeout`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/strategies.dart` | the `WorkboxPlugin` class and its callback params, `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, `CacheFirst`, `CacheOnly`, `NetworkFirst`, `NetworkOnly`, `cacheOkAndOpaquePlugin` |
| `src/precaching.dart` | `PrecacheEntry`, `createCacheKey`, `PrecacheController`, `PrecacheStrategy` and its two static plugins, the cache-key, install-report and fallback plugins, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams` |
| `src/routing.dart` | `Route` and `Router` |
| `src/host.dart` | `dart:js_interop` extension types for CacheStorage, Cache, fetch, Request, Response, Headers, URL, RegExp and the JS globals; `hostCall` and the `instanceof Error` guard |
| `src/main.dart` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup; `main()` publishes `run` |

## Boundary

The corpus input is the JS object itself, seen through `dart:js_interop` extension types (no copy). Manifest entries become `PrecacheEntry` values; a revision stays the caller's `JSAny?`, because its truthiness decides and the host prints a number revision with ToString (`searchParams.set`). Workbox runs on Dart classes and `async` functions over extension types of the host objects. The output is built from JS objects and arrays in the contract's key order, and `run` returns a `JSPromise`. dart2js cannot emit ES module exports, so `main()` publishes `run` on `globalThis.__matrixExports` for the builder's facade.

## Structure and idiom

- **One library, four parts.** The npm packages are the parts of `workbox.dart`, so `_handle` stays library-private and a Strategy subclass in `precaching.dart` overrides it, as Workbox's subclasses override its protected `_handle`.
- **Plugins** are the `WorkboxPlugin` class: twelve nullable callback getters (`Callback<P, R>`, a function to a `Future`), `null` by default. A plugin defines a callback by overriding its getter, so `hasCallback` is `!= null`. Each callback takes a param class named after Workbox's (`CacheWillUpdateParam`, …), with `state`.
- **`iterateCallbacks`** is a `sync*` generator, as Workbox's is a generator. It reads `strategy.plugins` by index at every step, so a plugin added while a handler runs is reached, without state. The plugin state is a map per plugin present when the handler was created, in an identity map, as Workbox's `MapLikeObject` keyed by the plugin object.
- **Strategies** subclass the abstract `Strategy` and override `_handle`. The module-level plugins are singletons, as in Workbox: `_cacheOkAndOpaquePlugin` is a top-level final, and PrecacheStrategy's two static plugins are static finals compared with `identical`.
- **Promises.** Workbox's async functions are Dart `async` functions, which run synchronously up to their first `await`, and whose `Future`s are eager and may have several listeners. `handleAll` starts `_getResponse` and `_awaitComplete` at once, and `handle` drops the second, which keeps running. `Promise.race` is `Future.any`, `Promise.all` is `Future.wait` with `eagerError`, and `Promise.allSettled` is `Future.wait` over outcomes, whose first failure is taken in batch order.
- **Errors.** Workbox's own error is `WorkboxError`, whose `name` is the code. Everything else is the value that was thrown: a JS Error (`JSObject`), or the fail fixture's non-Error string. A promise's rejection and a value Dart throws itself keep their identity through `await` and `catch`. `isError` is Workbox's `instanceof Error` guard, applied at each guard the contract lists.
- **Route matches** answer `Object?`, as Workbox's `match` answers any value: `null` or `false` for no match, `true` for a match without params, or the `Params`.
- **`generateURLVariations`** is a `sync*` generator, so a later variation and `urlManipulation` run only when the earlier ones missed.
- **RegExp** is the host's `RegExp` (`HostRegExp`), without flags: a bad source throws its SyntaxError, where Dart's `RegExp` would throw a `FormatException`.

## Interop decisions

- **TypeErrors.** dart2js turns a JS TypeError caught by `catch` into a Dart error that has lost its name. Workbox and the adapter report the name, so:
  - `URL.from` parses with `URL.parse` and throws the host's own TypeError (`new TypeError(…)`) where `new URL` would throw one. The parser is the same.
  - Every `Request`, `Response` and `Headers` construction in both the adapter and Workbox (`install`, `createHandlerBoundToURL`, `_handleFetch`, `PrecacheCacheKeyPlugin`, `copyResponse`) runs inside a JS Promise executor (`hostCall`), whose rejection keeps the thrown `TypeError` or `RangeError` name (e.g., for credentialed URLs passed to `new Request` or invalid `{respond}` statuses).
- **Non-nullable externals.** dart2js checks the type of an external getter's value, so the optional step members are nullable.
- **Microtasks in a service worker.** dart2js schedules its microtasks with a MutationObserver, which a page has. A service worker has no `document`, so dart2js falls back to `setTimeout`. `verify-worker.mjs` compares the reports, responses and cache state only, and those match.
- **Unhandled failures.** A `Future` that fails with no listener yet is reported to the zone as uncaught, as a JS promise is reported as an unhandled rejection. Nothing reads either.

## Deviations from the npm source

None changes a result in the parity domain.

- **Nested errors in WorkboxError details.** `JSON.stringify` of a nested WorkboxError prints its own enumerable `name` and `details`, and of a native Error `{}`. A Dart exception is not a JS object of that shape, so the details hold that printed form (`asDetail`) instead of the error itself. Only the message is observable.
- **`install` iterates a snapshot of the URL map.** Workbox iterates the live `Map`, but steps run one at a time, so no entry can be added during an install.
- **`cache-put-with-no-response` cannot happen**: `cachePut` takes a non-null `Response`.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **`NetworkOnly`** awaits `fetch` directly: `Promise.race` of one promise settles as it does, and the timeout error is unreachable without `networkTimeoutSeconds`.
- **`HandlerCallbackOptions` has no `url`**: no handler in the subset reads it.
- **The revisionless warning** goes to `console.warn` with Workbox's text after each entry, as the production build does.

## Module state

None. The module-level plugins hold no state, as in Workbox. Each `run` builds its own scenario.

## Performance accommodations

None.
