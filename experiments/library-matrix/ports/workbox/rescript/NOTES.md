# workbox 7.4.1: ReScript port

This port is written by hand. It replaces the earlier port of the flattened scenario contract, which a Python translator (`build/library-matrix/dev-wb-comparators/`) generated from a shared core. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles behind `matrix-entry.mjs`, the subset in `libraries/workbox/CONTRACT.md`), and of what `libraries/workbox/reference.mjs` does with them, to ReScript 12.3. The layers are Workbox's own: `PrecacheController` → `PrecacheStrategy` → `Strategy`/`StrategyHandler` → plugins, behind `Router` and `Route`.

| File | What it is |
| --- | --- |
| `src/WorkboxCore.res` | `WorkboxError` with the production message, `copyResponse`, `cacheMatchIgnoreParams`, `timeout`, `waitUntil`, `Deferred` |
| `src/Strategies.res` | the plugin record and its callback parameters, `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, `CacheFirst`, `CacheOnly`, `NetworkFirst` (with `cacheOkAndOpaquePlugin`) and `NetworkOnly` |
| `src/Routing.res` | the route handler (what `normalizeHandler` gives), `Route`, `Router` |
| `src/Precaching.res` | `createCacheKey`, `PrecacheController`, `PrecacheStrategy`, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams`, and the cache-key, install-report, fallback, default-cacheability and redirect-copy plugins |
| `src/Host.res` | bindings for CacheStorage and Cache, Fetch, URL and URLSearchParams, the ExtendableEvent, the location and crypto |
| `src/Scenario.res` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup |
| `src/Index.res`, `src/Index.resi` | `run`, the module's only export |

## Boundary

`run(input)` reads the script's step objects in place as ReScript records with optional fields. Manifest entries and the fail fixture's `value` stay `JSON.t`, as the caller wrote them: `createCacheKey` reports the entry itself, and the host prints a number revision with ToString. Workbox's logic runs on bindings to the host's CacheStorage, Fetch, Request, Response and URL. The output is built as `JSON.t` objects from ordered dicts, in the contract's key order, and `run` returns a promise.

## Structure and idiom

- **Async functions.** Workbox's async methods are ReScript `async` functions, compiled to JavaScript `async`/`await`, so each promise is the host's: eager, shareable, and continued in microtasks. `handleAll` starts `_getResponse` and `_awaitComplete` at once, and `handle` drops the second, which keeps running as Workbox's dropped promise does. `Promise.all`, `Promise.allSettled` and `Promise.race` are the Stdlib's bindings, used where Workbox uses them (install, `doneWaiting`, NetworkFirst and NetworkOnly).
- **Errors.** A thrown value stays the JavaScript value. Workbox's code throws with `JsExn.throw`, and each `catch` binds the value with `JsExn(error)`. A WorkboxError is a real `Error` with own `name` and `details`. The `instanceof Error` guards are `Error.prototype.isPrototypeOf`. The fail fixture's string therefore reaches every guard as the string it is.
- **Plugins** are a record of optional function fields, which compiles to an object without the absent members. `hasCallback` and `iterateCallbacks` test the field's presence, as Workbox tests `name in plugin`. Each callback receives a parameter record with its `state`. `iterateCallbacks` is a cursor that reads the strategy's live plugin array at every step, so a plugin added meanwhile is reached, with no state.
- **Plugin state** is a plain object per plugin per handler, as Workbox's `MapLikeObject` is. Each plugin binds its own members with `@get` and `@set` externals: the trace's `log` and the install report's `originalRequest`.
- **Loops that await** are local recursive `async` functions, since ReScript has no `break`: `runCallbacks` and `chainCallbacks` for the callback loops, and early exits where Workbox breaks.
- **Maps** are the Stdlib's `Map` (insertion-ordered, a set keeps the place), as in Workbox. `install` walks `Map.entries` with `Iterator.next`, live, as `for…of` does.
- **`generateURLVariations`** is lazy: each variation goes to a `visit` callback in order, chained with `||`, and the later ones (and `urlManipulation`) are not computed once one is in the manifest.

## Deviations from the npm source

None changes a result in the parity domain.

- **`instanceof WorkboxError` is a WeakSet test.** ReScript cannot declare a JavaScript class, so `WorkboxError.make` records each error it makes in a module-level `WeakSet`, and `isInstance` tests membership. The adapter's `failure` is the only reader.
- **The cache-key plugin holds the controller's `getCacheKeyForURL`, not the controller.** A record cannot refer to itself while it is built, and the controller's strategy holds the plugin. The plugin's lookup reads the controller's own map. Rust uses a `Weak` for the same reason.
- **Route parameters** are a record of the two members the subset uses, `cacheKey` and `integrity`. A match callback answers `None` for no match, or the parameters. A match that is a plain `true` answers `{}`, which `findMatchingRoute` turns into undefined, as it does a `{}` or a boolean.
- **The router's `.catch`** is an async function that awaits the handler's promise. It is attached at once and reads the global catch handler when the rejection arrives, as Workbox's callback does.
- **`cache-put-with-no-response` cannot happen**: `cachePut` takes a `Response` by type.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **The revisionless warning** goes to `console.warn` with Workbox's text, after each entry, as the production build does. It is not compared.

## Module state

The `WeakSet` of WorkboxErrors, and the shared plugin constants Workbox also has: `cacheOkAndOpaquePlugin` and PrecacheStrategy's `defaultPrecacheCacheabilityPlugin` and `copyRedirectedCacheableResponsesPlugin`. Nothing else outlives a call.

## Performance accommodations

None.
