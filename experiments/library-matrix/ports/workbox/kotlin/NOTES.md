# workbox 7.4.1: Kotlin/JS port

This port is written by hand. It replaces the earlier flattened-scenario port (B9), which was partly generated. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles that `matrix-entry.mjs` links), for the subset in `libraries/workbox/CONTRACT.md`, and of what `libraries/workbox/reference.mjs` does with it. It targets Kotlin 2.4.10 on the harness's Gradle production lane. It uses no `js("…")` and no handwritten JS. The controller, strategy, handler, plugin and router layers are Workbox's own.

| File | What it is |
| --- | --- |
| `src/WorkboxCore.kt` | `WorkboxError` with the production message, `ExtendableEvent`, `waitUntil(event, asyncFn)`, `timeout`, `Deferred`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/Strategies.kt` | the `WorkboxPlugin` interface and its callback params, `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, `CacheFirst`, `CacheOnly`, `NetworkFirst`, `NetworkOnly`, `cacheOkAndOpaquePlugin` |
| `src/Precaching.kt` | `PrecacheEntry`, `createCacheKey`, `PrecacheController`, `PrecacheStrategy` and its two static plugins, the cache-key, install-report and fallback plugins, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams` |
| `src/Routing.kt` | `Route` and `Router` |
| `src/Host.kt` | external declarations for CacheStorage, Cache, fetch, Request, Response, Headers, URL and the JS globals; `await`, `launch`, `HostError` and the `instanceof Error` guard |
| `src/Main.kt` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup; `run`, the module's only export (`@JsExport`) |

## Boundary

The corpus input is a JS object typed by external interfaces and read in place. Manifest entries become `PrecacheEntry` values; a revision stays the caller's JS value, because its truthiness decides and the host prints a number revision with ToString (`searchParams.set`). Workbox runs on Kotlin classes and suspend functions over typed host objects. The output is built from JS objects (`json(...)`) and arrays in the contract's key order, and `run` returns a promise.

## Structure and idiom

- **Plugins** are the `WorkboxPlugin` interface: twelve nullable suspend-function properties, `null` by default. A plugin defines a callback by overriding its property, so `hasCallback` is `callback(plugin) != null`. Each callback takes a param class named after Workbox's (`CacheWillUpdateParam`, …), with `state`.
- **`iterateCallbacks`** is a `sequence { }` generator, as Workbox's is a generator. It reads `strategy.plugins` by index at every step, so a plugin added while a handler runs is reached, without state. The plugin state is a `MutableMap` per plugin present when the handler was created, as Workbox's `MapLikeObject`.
- **Strategies** subclass the abstract `Strategy` and override `_handle`. The module-level plugins are singletons, as in Workbox: `CacheOkAndOpaquePlugin` is an `object`, and PrecacheStrategy's two static plugins live in its companion object, compared by identity (`===`).
- **Promises.** Workbox's async functions are suspend functions. Its eager promises are host `Promise`s started by `launch`, which runs a coroutine up to its first suspension before it returns, as an async function call does. `handleAll` launches `_getResponse` and `_awaitComplete` at once. `handle` drops the second, which keeps running. `await` resumes the coroutine in the promise's continuation. There is no kotlinx dependency.
- **Errors.** Workbox's own error is `WorkboxError`, whose `name` is the code. Everything else is the JS value that was thrown. A JS Error is a Kotlin `Throwable` at run time, and `jsName` and `jsMessage` read its JS `name` and `message`. Kotlin can throw only a `Throwable`, so a thrown non-Error (the fail fixture's `value`) is a `HostError` holding the value. `isError` is Workbox's `instanceof Error` guard: true for everything but a `HostError`. It is applied at each guard the contract lists.
- **Route matches** answer `Any?`, as Workbox's `match` answers any value: `null` or `false` for no match, `true` for a match without params, or the `Params` (`{cacheKey, integrity}`).
- **`generateURLVariations`** is a `sequence { }` generator consumed with `firstNotNullOfOrNull`, so a later variation and `urlManipulation` run only when the earlier ones missed.
- **RegExp** is `kotlin.js.RegExp`, the host's, without flags. Kotlin's `Regex` would compile its own flags.
- **Maps** are `LinkedHashMap`, whose insertion order and keep-the-place-on-re-set match JavaScript's `Map`.

## Deviations from the npm source

None changes a result in the parity domain.

- **Nested errors in WorkboxError details.** `JSON.stringify` of a nested WorkboxError prints its own enumerable `name` and `details`, and of a native Error `{}`. A Kotlin exception has other own properties, so the details hold that printed form (`asDetail`) instead of the error itself. Only the message is observable.
- **A plugin callback that does not suspend** returns at once, where awaiting a JS async function takes a microtask. The subset observes no count of microtasks: within a handler the trace order follows the code, and the only timer is cachePut's `timeout(0)`, which every microtask-level step precedes either way.
- **`matchPrecache`** is two overloads (`String`, `Request`) instead of the `instanceof Request` test.
- **`install` iterates a snapshot of the URL map.** Workbox iterates the live `Map`, but steps run one at a time, so no entry can be added during an install.
- **`cache-put-with-no-response` cannot happen**: `cachePut` takes a non-null `Response`.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **`NetworkOnly`** awaits `fetch` directly: `Promise.race` of one promise settles as it does, and the timeout error is unreachable without `networkTimeoutSeconds`. `NetworkFirst` keeps its `Promise.race([a])`.
- **`HandlerCallbackOptions` has no `url`**: no handler in the subset reads it.
- **The revisionless warning** goes to `console.warn` with Workbox's text after each entry, as the production build does.

## Module state

None. The module-level plugins (`CacheOkAndOpaquePlugin` and PrecacheStrategy's two static plugins) hold no state, as in Workbox. Each `run` builds its own scenario.

## Performance accommodations

None.
