# Library contract: `workbox`

| Field | Value |
| --- | --- |
| Package | `workbox-precaching@7.4.1` with the `workbox-core`, `workbox-routing` and `workbox-strategies` 7.4.1 modules it imports. The entry is `matrix-entry.mjs`, which `tools/prepare-references.mjs` builds from the four published production bundles (`build/workbox-*.prod.js`). |
| Reference adapter | `reference.mjs`: `run(input)` runs one script of Workbox API calls ("Steps") with the fixture plugins ("Fixtures"). It resolves with the canonical output ("Output"). |
| API subset | Precaching, routing and four runtime strategies, with the full plugin lifecycle ("API subset"). Ports implement these layers themselves on the host's CacheStorage, Fetch, Request, Response and URL. |
| Input | `{steps: [step, …]}`. Service-worker mode adds `cachePrefix`, `event` and `request` ("Service-worker mode"). |
| Output form | `json`, async; key order is significant |
| Normalisation | none. The output is location-independent by construction ("Output"). |
| Parity environments | chrome, firefox, safari. Safari is unavailable on this machine, so `expected.json` was captured in Chrome and confirmed identical in Firefox; Safari was never captured. |
| Corpus | `corpus.json`, 58 cases, written by `generate-corpus.mjs` |
| Bench cases | `bench-cache`, baseIterations 1, variant `bench-variant.mjs` ("Bench") |
| Caching | Nothing persists across calls. Workbox memoises `canConstructResponseFromBodyStream` (always false in 7.4.1) at module level; ports may hard-code false. Each call creates its caches under a fresh prefix and deletes them before it settles. |
| Files | none. The two network fixtures are fetched at run time from the test origin. |

The previous contract (until 2026-11-20) defined a flattened scenario API that ports reimplemented directly on CacheStorage. The B9 review recorded the divergences of that design. This contract makes ports implement Workbox's own structure, which is `PrecacheController` → `PrecacheStrategy` → `Strategy`/`StrategyHandler` → plugins, behind `Router` and `Route`. "Changes from the previous contract" maps the B9 findings to cases.

## API subset

Ports implement the following production behaviour. The development-build asserts and logging are not part of it.

**workbox-core**
- `WorkboxError(code, details)` is an `Error`. Its own `name` is the code and its own `details` is the details object. The production message is `code + ' :: ' + JSON.stringify([details])`. That gives `code :: [null]` without details, as for `unregister-route-route-not-registered`. `JSON.stringify` of a WorkboxError nested in details prints its own enumerable properties: `{"name":…,"details":…}`. A native error prints `{}`.
- `copyResponse(response)`:
  1. Throw `cross-origin-copy-response {origin}` unless the response URL's origin is `location.origin`. A URL-less response has origin `null`.
  2. Clone the response.
  3. Build `new Response(await clone.blob(), {headers: new Headers(clone.headers), status, statusText})`. `canConstructResponseFromBodyStream()` always returns false in 7.4.1, so the body is always a Blob.
- `cacheMatchIgnoreParams(cache, request, ['__WB_REVISION__'])`: `cache.match(request)` when stripping changes nothing. Otherwise, the first key from `cache.keys(request, {ignoreSearch: true})` that strips to the same URL.
- `timeout(0)` is one timer task (`setTimeout`). `waitUntil(event, fn)` passes `fn()`'s promise to `event.waitUntil` and returns it. `Deferred` is a promise with its resolver.

**workbox-routing**
- `new Route(match, handler, method = 'GET')`, with `route.setCatchHandler(handler)`. `normalizeHandler` turns an object with `handle` into itself and a function into `{handle: fn}`.
- `Router`: `registerRoute`, `unregisterRoute`, `setDefaultHandler(handler, method = 'GET')`, `setCatchHandler` and `handleRequest({request, event})`.
  - `unregisterRoute` throws `unregister-route-but-not-found-with-method {method}` when the router has no list for the method, and `unregister-route-route-not-registered` (no details) when the route is not in it.
  - `handleRequest`, step by step:
    1. Parse `new URL(request.url, location.href)`.
    2. Return `undefined` unless `protocol.startsWith('http')`.
    3. Compute `sameOrigin`.
    4. `findMatchingRoute` takes the first route registered for `request.method` whose `match({url, sameOrigin, request, event})` is truthy. A match of `[]`, `{}` or a boolean gives `params` undefined.
    5. Without a route, use the default handler for the method. Without either, return `undefined`.
    6. Call `handler.handle({url, request, event, params})`.
    7. A synchronous throw from `handle` becomes a rejected promise.
    8. Only when the result is a Promise and the route or the router has a catch handler (both read now), chain `.catch`:
       - The route's catch handler runs first. If it throws an `Error`, that error replaces the original; a thrown non-Error leaves the original.
       - Then the global catch handler runs, called without `params`. It is read when the rejection arrives.
       - Otherwise the error is rethrown.

       In the subset every handler returns a Promise.

**workbox-strategies**
- `Strategy` takes `{cacheName, plugins}`. `handle(options)` is `handleAll(options)[0]`. `handleAll` creates a `StrategyHandler` and starts two promises at once:
  - `_getResponse`:
    1. Run `handlerWillStart`, outside the `try`: a failure there rejects directly.
    2. Inside the `try`, run `_handle` into `response`. A missing response, or one of type `error`, throws `no-response {url}`.
    3. In the `catch`: when the thrown value is an `Error`, run the `handlerDidError` callbacks, assigning each result to `response`, until one returns a response. Then rethrow only if `response` is falsy. A type-`error` response that no handlerDidError replaced is therefore returned, not rethrown.
    4. Run `handlerWillRespond`.
  - `_awaitComplete`:
    1. Await the response, ignoring its failure.
    2. Run `handlerDidRespond`, then `doneWaiting()`.
    3. If either throws an `Error`, keep it. A thrown non-Error is dropped.
    4. Run `handlerDidComplete({response, error})`, then `destroy()`.
    5. Rethrow the kept error.

  Both promises run even when nobody awaits the second.
- `StrategyHandler`:
  - The constructor copies the plugin list into a per-handler state map: each plugin present now gets a fresh `{}`. It then calls `event.waitUntil(deferred.promise)`.
  - `iterateCallbacks` and `hasCallback` read the strategy's live plugin list. A plugin added later is called with `state` undefined.
  - `fetch(input)`:
    1. Clone the original request when a `fetchDidFail` exists.
    2. Run each `requestWillFetch(request.clone())`. An `Error` thrown there becomes `plugin-error-request-will-fetch {thrownErrorMessage}`. A non-Error ends the loop silently.
    3. Clone the filtered request. In one `try`, call the global `fetch(request)` and run `fetchDidSucceed`.
    4. When that `try` throws (the fetch or a fetchDidSucceed callback), run `fetchDidFail` with one clone of the original and one of the filtered request, then rethrow.
    5. The two `mode === 'navigate'` branches (navigation preload, and dropping `fetchOptions`) are out of the subset. A page cannot construct a navigate request.
  - `fetchAndCachePut` returns the response and passes `cachePut(input, response.clone())` to `waitUntil`.
  - `cacheMatch(key)` calls `caches.match(cacheKey('read'), {cacheName})`, which opens no cache, then runs `cachedResponseWillBeUsed`. A falsy result becomes `undefined`.
  - `cachePut(key, response)`:
    1. `await timeout(0)`.
    2. Compute `cacheKey('write')`.
    3. Run `_ensureResponseSafeToCache`: each `cacheWillUpdate` in turn, stopping at the first falsy result. With no such plugin, only status 200 passes. A refused response returns false.
    4. `caches.open(cacheName)`.
    5. With a `cacheDidUpdate`, look up the old response by `cacheMatchIgnoreParams`.
    6. `cache.put` the effective request, with a clone of the response when a `cacheDidUpdate` exists and the response itself otherwise. If the put throws an `Error`, rethrow it (`QuotaExceededError` first runs the registered quota callbacks; none are registered here). Any other thrown value is ignored.
    7. Run each `cacheDidUpdate` with a fresh `newResponse.clone()`, then return true.
  - `getCacheKey(request, mode)` runs `cacheKeyWillBeUsed({mode, request, params})` once per `url | mode` per handler.
  - `doneWaiting` settles every extended lifetime in batches, including lifetimes added meanwhile. It throws the first rejection of a batch.
- `CacheFirst` tries `cacheMatch`, else `fetchAndCachePut`. It throws `no-response {url, error}`, and `error` is kept only when the caught value is an `Error`.
- `CacheOnly` tries `cacheMatch` and throws `no-response {url}`.
- `NetworkFirst`, without `networkTimeoutSeconds`:
  - The constructor unshifts `cacheOkAndOpaquePlugin` (status 200 or 0) unless a plugin defines `cacheWillUpdate`.
  - The network promise `a` is `fetchAndCachePut`. When that throws (the value is kept only if it is an `Error`), or when it answers nothing, `a` falls back to `cacheMatch`.
  - `_handle` passes `Promise.race([a])` to `handler.waitUntil`, and also passes `(async () => await race || await a)()` to it. It answers with that promise's value, or throws `no-response {url}`.
- `NetworkOnly` calls `fetch` and throws `no-response {url, error}`. `networkTimeoutSeconds` is 0.

**workbox-precaching**
- `new PrecacheController({cacheName, plugins, fallbackToNetwork = true})` builds a `PrecacheStrategy` whose plugins are `[...plugins, new PrecacheCacheKeyPlugin(controller)]`, then the strategy's redirect copy plugin.
- `addToCacheList(entries)` works entry by entry. Entries before a throwing one stay added. Each entry goes through these steps:
  1. Queue a revisionless warning: string entries, and objects whose `revision === undefined`.
  2. Compute `createCacheKey(entry)`.
  3. The cache mode is `'reload'` for a truthy revision, else `'default'`.
  4. Throw `add-to-cache-list-conflicting-entries {firstEntry, secondEntry}` when the URL already has a different key.
  5. With a truthy `integrity`, throw `add-to-cache-list-conflicting-integrities {url}` when the key already has a different integrity. Otherwise record it.
  6. Set the URL's key and cache mode. Map insertion order is kept, and re-setting a URL keeps its place.
  7. `console.warn` the accumulated list. The warning is part of the behaviour but is not compared.
- `createCacheKey(entry)`:
  - A falsy entry, or an object with a falsy `url`, throws `add-to-cache-list-unexpected-type {entry}`, where `entry` is the caller's value.
  - A string, or an entry with a falsy `revision`, is its own key: `new URL(url, location.href).href`.
  - Otherwise the key URL gets `searchParams.set('__WB_REVISION__', revision)`. The host converts a number with Number-to-String.
- `install(event)` runs under `waitUntil(event, …)`:
  1. Push a new `PrecacheInstallReportPlugin` onto the strategy's plugins. It is never removed.
  2. For each URL in order, build `new Request(url, {integrity, cache: mode, credentials: 'same-origin'})`.
  3. `await Promise.all(strategy.handleAll({params: {cacheKey}, request, event}))`.
  4. Return `{updatedURLs, notUpdatedURLs}`.
- `activate(event)` runs under `waitUntil`. It opens the precache, deletes every key whose URL is not a current cache key, in `cache.keys()` order, and returns `{deletedURLs}`.
- The lookup methods:
  - `getURLsToCacheKeys()` and `getCachedURLs()` (its keys).
  - `getCacheKeyForURL(url)` is the exact `new URL(url, location.href).href` lookup, without variations.
  - `getIntegrityForCacheKey`.
  - `matchPrecache(urlOrRequest)` opens the precache only when the URL is in the manifest.
  - `createHandlerBoundToURL(url)` throws `non-precached-url {url}`. Otherwise it returns a callback that sets `request = new Request(url)` and `params = {cacheKey, ...params}`, then calls `strategy.handle`.
- `PrecacheStrategy`:
  - `_handle(request, handler)` returns the `cacheMatch` hit, else calls `_handleInstall` under an `install` event, else `_handleFetch`.
  - `_handleInstall`:
    1. Run `_useDefaultCacheabilityPluginIfNeeded`.
    2. `fetch`.
    3. `cachePut(request, response.clone())`. A false result throws `bad-precaching-response {url, status}`.
  - `_handleFetch`:
    1. Without `fallbackToNetwork`, throw `missing-precache-entry {cacheName, url}`.
    2. Fetch `new Request(request, {integrity: mode !== 'no-cors' ? request.integrity || params.integrity : undefined})`.
    3. Repair the cache (default-plugin check, then `cachePut`) only when the manifest has an integrity, the request's integrity is empty or equal to it, and the mode is not `no-cors`.
  - `_useDefaultCacheabilityPluginIfNeeded` counts the plugins defining `cacheWillUpdate`, skipping the redirect copy plugin. With none, it pushes `defaultPrecacheCacheabilityPlugin` (status < 400). With more than one, it removes that default plugin if present.
  - `copyRedirectedCacheableResponsesPlugin.cacheWillUpdate` returns `copyResponse(response)` when `response.redirected`.
- `PrecacheRoute(controller, options)` is a `Route` to the controller's strategy. It matches the first variation of `request.url` in the manifest with `{cacheKey, integrity}`. `generateURLVariations` is lazy and yields these candidates in order:
  1. The URL without its hash.
  2. The URL after `removeIgnoredSearchParams` deletes every parameter matching one of `ignoreURLParametersMatching` (default `[/^utm_/, /^fbclid$/]`). This mutates the URL object, and `urlManipulation` later receives that mutated object.
  3. With `directoryIndex` (default `index.html`) and a pathname ending in `/`, the pathname plus the index.
  4. With `cleanURLs` (default true), the pathname plus `.html`.
  5. Each URL `urlManipulation({url})` returns.
- `PrecacheCacheKeyPlugin.cacheKeyWillBeUsed` uses `params?.cacheKey || controller.getCacheKeyForURL(request.url)`. When there is a key, it builds `new Request(key, {headers: request.headers})`.
- `PrecacheInstallReportPlugin`: `handlerWillStart` stores `state.originalRequest`. `cachedResponseWillBeUsed` under an `install` event pushes that request's URL to `notUpdatedURLs` on a hit and to `updatedURLs` on a miss.
- `PrecacheFallbackPlugin({fallbackURL, precacheController}).handlerDidError` returns `controller.matchPrecache(fallbackURL)`.

**Left out** (each a scope choice unless a reason is given)
- The default-controller module functions: `precache`, `precacheAndRoute`, `addRoute`, `addPlugins`, and the module-level `getCacheKeyForURL`, `matchPrecache` and `createHandlerBoundToURL`. They share a process-wide controller and router across calls. `addPlugins` is reachable as the `addPlugins` step on an explicit controller.
- `PrecacheController.precache(entries)`: it is `addToCacheList` plus global `install` and `activate` listeners on `self`.
- `cleanupOutdatedCaches`: it registers a global `activate` listener and reads `self.registration.scope`, which a page does not have.
- `cacheNames` defaults and `setCacheNameDetails`: module-level state. A cache name is always supplied, so `getPrecacheName(name)` and `getRuntimeName(name)` return it.
- `addFetchListener`, `addCacheListener` and `registerRoute(capture)`: they bind to the global `self` or to the default router.
- `NavigationRoute`: it matches only `navigate` requests, which a page cannot construct.
- `RegExpRoute` (scope choice): its params are the capture array, which no handler in the subset reads. The subset's match callbacks cover route matching.
- `StaleWhileRevalidate`: its `fetchAndCachePut` and `cacheMatch` start together, so the order of their callbacks depends on which host call settles first.
- `networkTimeoutSeconds` (`NetworkFirst`, `NetworkOnly`): it races a timer against the network.
- `fetchOptions` and `matchOptions` (scope choice): they are passed through to `fetch` and to `CacheStorage`/`Cache` (`{...matchOptions, cacheName}`, and the cacheDidUpdate lookup). They exercise the host's query options, not Workbox's logic.
- Quota callbacks: `registerQuotaErrorCallback` fills a module-level registry, and `QuotaExceededError` cannot be provoked.
- Navigation preload: a page cannot construct a `navigate` request.

## Steps

`input.steps` runs in order. Each step yields one entry of `results`: its value, or an error record when the call throws or rejects. A failed step does not stop the script. Ids name the controllers, routers and routes a script creates. Cache names in steps are suffixes: the adapter uses `<prefix>-<name>`.

| `op` | Fields | Workbox call | Result |
| --- | --- | --- | --- |
| `network` | `source` | sets the network fixture's source | `null` |
| `controller` | `id`, `cacheName`, `plugins?`, `fallbackToNetwork?` | `new PrecacheController({cacheName, plugins: [network, ...plugins], fallbackToNetwork})` | `null` |
| `addToCacheList` | `controller`, `entries` | `controller.addToCacheList(entries)`. The entries are passed as given: strings, `null`, or `{url, revision?, integrity?}` with `revision` a string, number or `null` | `null` |
| `addPlugins` | `controller`, `plugins` | `controller.strategy.plugins.push(...plugins)` | `null` |
| `install` | `controller` | `controller.install(event)` | `{updatedURLs, notUpdatedURLs}` |
| `activate` | `controller` | `controller.activate(event)` | `{deletedURLs}` |
| `getURLsToCacheKeys` | `controller` | `[...controller.getURLsToCacheKeys()]` | `[[url, key], …]` |
| `getCachedURLs` | `controller` | `controller.getCachedURLs()` | `[url, …]` |
| `getCacheKeyForURL` | `controller`, `url` | `controller.getCacheKeyForURL(url)` | key or `null` |
| `matchPrecache` | `controller`, `url`, `request?` | `controller.matchPrecache(request ? new Request(url) : url)` | response record or `null` |
| `router` | `id` | `new Router()` | `null` |
| `precacheRoute` | `router`, `controller`, `options?`, `id?` | `router.registerRoute(new PrecacheRoute(controller, options))` | `null` |
| `route` | `router`, `prefix` or `crossOrigin: true`, `handler`, `method?`, `id?` | `router.registerRoute(new Route(match, handler, method))`. `match` is `relative(url.href).startsWith(prefix)`, or `!sameOrigin` | `null` |
| `defaultHandler` | `router`, `handler`, `method?` | `router.setDefaultHandler(handler, method)` | `null` |
| `catchHandler` | `router` or `route`, `handler` | `(route ?? router).setCatchHandler(handler)` | `null` |
| `unregisterRoute` | `router`, `route` | `router.unregisterRoute(route)` | `null` |
| `fetch` | `router`, `url`, `method?`, `mode?`, `integrity?` | `router.handleRequest({request: new Request(url, {method, mode, integrity}), event})`. Members that are absent are left out | response record, or `null` when no route handles it |
| `caches` | | the cache dump | as `caches` in the output |

The adapter builds the following arguments:

- **`plugins`** is a list of fixture plugin specs.
- **`handler`** is one of:
  - `{strategy: 'CacheFirst'|'CacheOnly'|'NetworkFirst'|'NetworkOnly', cacheName, plugins?}` creates that strategy with `plugins: [network, ...plugins]`. A NetworkFirst without a cacheWillUpdate plugin then unshifts its own.
  - `{precache: id}` is that controller's `strategy`.
  - `{bound: url, controller: id}` is `controller.createHandlerBoundToURL(url)`. It can throw, which makes the step an error.
  - `{respond: body, status}` is `async () => new Response(body, {status})`.
- **`options`** for `PrecacheRoute`:
  - `directoryIndex` and `cleanURLs` are passed as given. `null` disables them, and an absent member keeps the default.
  - `ignoreURLParametersMatching` is a list of RegExp sources, compiled with `new RegExp(source)` (JavaScript semantics, no flags).
  - `urlManipulation` is a list of suffixes. The adapter's callback returns, for each suffix, `new URL(url.href)` with `pathname += suffix`.

An event-driven step (`install`, `activate`, `fetch`) runs under a fresh stand-in event `{type, waitUntil(promise)}`. Its `type` is `install`, `activate` or `fetch`. The stand-in collects the promises. After the call settles, the step awaits them all (`allSettled`), in rounds, until no new ones arrive. It then appends `[type, count]` to `events`.

## Fixtures

**Network.** Every plugin list the adapter builds starts with the network plugin, so no fetch leaves the test origin. Its `requestWillFetch` replaces the request with `new Request(<origin>/experiments/library-matrix/libraries/workbox/<alpha|beta>.mjs, {integrity})`. The `integrity` member is the request's own, except for `broken`. Its `fetchDidSucceed` handles the `missing` and `redirected` sources. The `source` decides the rest:

| `source` | Behaviour |
| --- | --- |
| `alpha` (initial), `beta` | That file. Each file holds `export default '<name>'\n`. |
| `offline` | `requestWillFetch` throws `new TypeError('offline')`. Workbox reports `plugin-error-request-will-fetch {"thrownErrorMessage":"offline"}`. |
| `broken` | alpha with SRI metadata that matches nothing (`sha256-AAAA…A=`). The real `fetch` rejects and `fetchDidFail` runs. |
| `missing` | alpha, then `fetchDidSucceed` returns `new Response('missing', {status: 404})`. |
| `redirected` | alpha, then `fetchDidSucceed` marks the response as a followed redirect leaves it. It defines own `redirected: true` and an own `clone` that returns a marked clone. |

**Plugins**, as `{kind, …}`:

- **`trace` (`label`)** defines every callback except `cacheWillUpdate`, so it never changes cacheability. `handlerWillStart` sets `state.log = []` and appends `[label, relative(request.url), state.log]` to `handlers`. Each later callback appends one record to its handler's log:
  - `["cacheKeyWillBeUsed", mode, url]` returns `request`.
  - `["cachedResponseWillBeUsed", cache, url, status|null]` returns `cachedResponse`.
  - `["requestWillFetch", url]` returns `request`.
  - `["fetchDidSucceed", url, status]` returns `response`.
  - `["fetchDidFail", originalUrl, url, errorName]`.
  - `["cacheDidUpdate", cache, url, oldStatus|null, newStatus]`.
  - `["handlerWillRespond", status]` returns `response`.
  - `["handlerDidRespond", status|null]`.
  - `["handlerDidComplete", status|null, errorName|null]`.
  - `["handlerDidError", errorName]` returns `undefined`.

  `url` is `relative(…)`, `cache` is the cache's suffix, and a status is `response.status`. `errorName` is `error.name` for an `Error`, and `String(value)` for a thrown non-Error. A callback records only into a log that its own `handlerWillStart` created (`state.log?.push`). A plugin listed before the trace can fail `handlerWillStart`, and the trace must not throw in the callbacks that follow.
- **`keep`** is `{cacheWillUpdate: ({response}) => response}`.
- **`drop`** is `{cacheWillUpdate: () => null}`.
- **`fail` (`on`, `value?`)** defines only the callback named `on`. That callback throws `new Error('fail')`, or, with `value`, the value itself: a non-Error string. The `value` form drives Workbox's `instanceof Error` guards. A port that treats every thrown value as an Error fails `fail-non-error`.
- **`fallback` (`url`, `controller`)** is `new PrecacheFallbackPlugin({fallbackURL: url, precacheController})`.

Because the log lives in the handler's plugin state, each handler's records stay in order. They do so even when Workbox runs two handlers at once (a failing handler's completion beside a catch handler's strategy). `handlers` lists the handlers in the order they started.

## Output

`{results, events, handlers, caches}`:

- `results` has one entry per step.
- `events` is the `[type, count]` list.
- `handlers` holds the traces.
- `caches` is the final dump. It lists every cache whose name starts with `<prefix>-`, in `caches.keys()` order, as `[suffix, [[relative(request.url), record], …]]`, with the entries in `cache.keys()` order.

The records are:

- **Response record:** `{status, url: relative(response.url), body: await response.text()}`.
- **Error record:**
  - A WorkboxError gives `{error: name, message}`. The message has every occurrence of the base and of `<prefix>-` removed.
  - Another `Error` gives `{error: name}`. Host messages differ between browsers and are not recorded.
  - A thrown non-Error gives `{thrown: String(value)}`.
- **`relative(url)`:**
  - The base is `new URL('./', location.href).href`. On the check page that is `<origin>/__load/<N>/`.
  - `relative` strips the base when the URL starts with it. Any other URL prints whole, for example `http://x/`, `data:,x`, or a cross-origin fixture target.
  - The corpus uses only URLs relative to the base, cross-origin URLs and non-http URLs. A same-origin URL outside the base would print the loopback port.

After the dump, the adapter deletes every `<prefix>-*` cache, in a `finally`. That runs on every path, including a failed step or a failed dump. `verify.mjs` checks that every call leaves `caches.keys()` as it found it.

**Determinism.** Steps run one at a time.

- Within a handler, the trace order follows from the code. The response path (`handlerWillRespond`, `handlerDidRespond`) runs in microtasks.
- A background `cachePut` starts with `timeout(0)`, a timer task. Its `cacheKeyWillBeUsed(write)` and `cacheDidUpdate` therefore come after `handlerDidRespond` and before `handlerDidComplete`.
- A port must not add a timer-task hop to the response path. A port must start every promise Workbox starts eagerly, including the `_awaitComplete` that `handle` drops.

## Realm

The checker and `verify.mjs` load the reference and the port into the page's own realm (`tools/lib/page.mjs`, harness H4). Host errors from `fetch`, `URL` and `Cache` are therefore `instanceof Error` in every browser, and Chrome and Firefox produce the same reference output. `expected.json` holds one section.

An earlier capture (before e9a9f70) imported modules from the WebDriver script. Firefox evaluated them in Marionette's sandbox, and a second, Firefox-only section recorded that artefact. It is gone.

Non-Error throws are now covered by the `fail` fixture's `value` form. The guards ports must apply to the thrown value are:
- `_getResponse`;
- `_awaitComplete`;
- the `requestWillFetch` catch;
- the `CacheFirst`, `NetworkFirst` and `NetworkOnly` catches;
- the `cachePut` catch (reachable only through a host rejection);
- the router's route-catch fallback.

## Service-worker mode

`verify-worker.mjs` links the artifact into a real service worker (`worker-fixture.mjs`). There, `run` gets three extra input members:

- `cachePrefix`: the caches persist under this prefix, and the call deletes nothing.
- `event`: the native event. Every event-driven step uses it, the port calls its `waitUntil` directly, and nothing is recorded in `events`.
- `request`: the native request. A `fetch` step without `url` uses it.

The worker runs native install, activate and fetch events. The page checks:

- the install and activate reports;
- precached and runtime responses before and after the fixture goes offline;
- the worker's cache dump.

## Parity domain

Inputs outside the domain are these:

- Steps that name unknown ids, or that omit `cacheName`. The default cache names are out of the subset.
- Manifest `url` values that are not strings.
- Revisions that are booleans or objects. The domain revision is a string, a JSON number or `null`.
- `fail` on `handlerDidComplete`. `destroy()` would never run, so the event's lifetime would never settle.
- `fail` on `handlerDidRespond` in a strategy whose fetch path caches in the background (`CacheFirst`, `NetworkFirst`). `_awaitComplete` then skips `doneWaiting()`, so the event settles while that `cachePut` still runs, and later steps would see a host-timed cache. The fuzz found this: Chrome and Firefox resolved the race differently.
- Concurrent steps.
- Same-origin URLs outside the base.

## Elm: the I/O boundary

Elm implements every layer above in Elm: controller, strategies, handler lifecycle, plugins, router, URL-variation logic, the adapter's script interpreter and its output. The JS host executes only the primitive commands below and returns each result, or a thrown or rejected value, as a message.

The host keeps host objects (caches, requests, responses, URLs, native events) in a handle table and passes handles to Elm. A failure reaches Elm as `{name, message, isError}`, where `isError` is `value instanceof Error` in the host's realm. Elm needs it for Workbox's guards.

Allowed primitives:

| Group | Primitive |
| --- | --- |
| CacheStorage | `caches.open(name)` → cache, `caches.match(request, {cacheName})` → response or none, `caches.keys()` → names, `caches.delete(name)` → bool |
| Cache | `cache.match(request or url)` → response or none, `cache.put(request, response)`, `cache.delete(request)` → bool, `cache.keys()` → requests, `cache.keys(request, {ignoreSearch: true})` → requests |
| Fetch | `fetch(request)` → response |
| Request | `new Request(url or request, init)` with init members `method`, `mode`, `integrity`, `cache`, `credentials` and `headers` (a handle); `request.clone()`; reads of `url`, `method`, `mode`, `integrity` and `headers` |
| Response | `new Response(body, {status})`, `new Response(blob, {headers: new Headers(headers), status, statusText})`, `response.clone()`, `response.text()`, `response.blob()`, reads of `status`, `statusText`, `url`, `redirected`, `type` and `headers` |
| URL | `new URL(input, base?)` → url (handle) or TypeError; reads of `href`, `origin`, `protocol` and `pathname`; writes of `pathname` and `hash`; `[...searchParams.keys()]`, `searchParams.set(name, value)` (the host converts a number revision with ToString), `searchParams.delete(name)` |
| Environment | `location.href`, `location.origin`, `crypto.randomUUID()`, `setTimeout(0)`, which Elm's `Process.sleep 0` provides |
| Text | `JSON.stringify(value)` for WorkboxError messages; elm/json's `Encode.encode 0` is the same function. `RegExp(source).test(name)` for `ignoreURLParametersMatching`; `elm/regex` wraps the JS RegExp. `console.warn(message)` |
| Events | `event.type` and `event.waitUntil(promise)` on a native event (service-worker mode). The host gives Elm a lifetime token and resolves its promise when Elm reports the work settled. |
| Fixture | `markRedirected(response)` for the `redirected` source: own `redirected: true` and an own `clone` that marks its result |

Nothing else is allowed in the host: no Workbox logic, no URL variation or cache-key code, and no plugin dispatch. Elm decides every call, its order and its arguments. A row is labelled "mostly JS" when the host exceeds half the bundle.

## Per-language guidance

- **Structure.** Port the four packages as modules with Workbox's own names: `PrecacheController`, `PrecacheStrategy`, `Strategy`, `StrategyHandler`, `Router`, `Route`, the plugins and `WorkboxError`. Then add the adapter (script interpreter, fixtures, output) as a separate module. The plugin interface is a set of optional callbacks with a `defines(name)` test. The per-handler state is one mutable object per plugin.
- **Promises.** Workbox's promises are eager and may have several awaiters: `responseDone` is awaited by `_awaitComplete` and by the caller. Represent them as host promises, or as an eager shared task. A lazy future that is never polled loses `_awaitComplete`, and the stand-in event then waits forever.
- **Errors.** Test `instanceof Error` on the thrown host value at each guard (Rust: `JsValue::is_error`). A thrown value is any JavaScript value, not necessarily an Error.
- **Rust** is the reference implementation of this contract: `ports/workbox/rust/`, with its `NOTES.md`.
  - mires cannot lay out `Rc<dyn Trait>`, so plugins and events are `Rc<Box<dyn …>>`.
  - It has no `TypeId`, so plugin state is a host object.
  - A `Task` spawns a future with `future_to_promise` and keeps the typed result beside the promise.
- **ReScript, Melange, js_of_ocaml, Scala.js, Kotlin, Dart.**
  - Bind the host APIs as externals or facades, and keep Workbox's logic in the language.
  - Use the language's own promise or async type over the host promise. Do not wrap the npm package.
  - Error guards must test `instanceof Error` on the thrown value: a caught exception is not necessarily an Error (`fail-non-error`).
- **GopherJS.** Await host promises with callbacks, as the comparator ports do, not with goroutines and channels. GopherJS's scheduler resumes a goroutine on a timer task, which reorders the response path against `cachePut`'s `timeout(0)`.
- **Elm.** See the boundary above. A command per primitive, with the script state in the model. The fixture plugins are Elm values like Workbox's own plugins.
  - The host sends each result in the continuation of the promise that settled, never through a timer.
  - The one timer is `timeout(0)`, which is Elm's `Process.sleep 0`.
  - A reply delayed by `setTimeout` would move the response path behind `cachePut`'s `timeout(0)` and reorder the traces.

## Corpus

| Case | Exercises |
| --- | --- |
| `keys-revisioned` | revisioned entries get __WB_REVISION__ appended through URLSearchParams (spaces, slashes, an existing query) |
| `keys-revisionless` | null, absent and empty revisions and string entries are their own cache keys |
| `keys-numeric-revision` | numeric revisions: 0 is falsy, the rest print with JavaScript Number-to-String, past int32 and past 2^53 |
| `keys-unicode-revision` | non-ASCII, astral and reserved characters in revisions and paths are percent-encoded by the host URL |
| `keys-url-forms` | hashes, queries, an existing revision parameter, padding, dot segments, cross-origin and non-http entries |
| `keys-duplicate` | repeated entries with the same key are accepted and keep their first position |
| `keys-conflict` | a conflicting revision throws after the earlier entries were added; later entries are not |
| `keys-conflict-revisionless` | a revisionless and a revisioned entry for one URL conflict, in both orders |
| `keys-integrity-conflict` | conflicting integrities for one cache key throw; a conflicting revision is reported before the integrity |
| `keys-unexpected-type` | null entries and entries without a URL throw add-to-cache-list-unexpected-type with the entry in the message |
| `keys-invalid-url` | URLs the host cannot parse throw a TypeError from addToCacheList and getCacheKeyForURL |
| `keys-lookup` | getCacheKeyForURL resolves against the location and looks up the exact URL, without route variations |
| `install-fresh` | install fetches every entry once; a second install finds them all cached |
| `install-upgrade` | a new version with a changed, a removed and a new entry; activate deletes outdated keys |
| `install-empty` | an empty precache installs nothing and opens no cache; activate opens (creates) the precache |
| `install-missing` | a 404 fails install with bad-precaching-response after the earlier entry was cached |
| `install-offline` | a requestWillFetch failure becomes plugin-error-request-will-fetch |
| `install-broken` | a fetch that rejects runs fetchDidFail and fails install with the host TypeError |
| `install-integrity` | manifest integrity reaches the fetch: the matching hash installs, the other fails |
| `install-keep-plugin` | a custom cacheWillUpdate replaces the default cacheability check, so a 404 is precached |
| `install-drop-plugin` | a cacheWillUpdate that refuses every response fails install with status 200 |
| `install-default-plugin-swap` | the default cacheability plugin is added on first use and removed when a custom one is added later |
| `install-redirected` | a redirected precache response is copied before caching (copyResponse), so the cached copy has no URL |
| `install-data-url` | a non-http entry: Cache.put rejects after the precache was opened (B9) |
| `install-cross-origin` | a cross-origin entry installs through the fixture and is served by the precache route (B9) |
| `install-trace` | the plugin lifecycle of install for a cached and an uncached entry, and the event lifetimes |
| `install-fail-did-respond` | a handlerDidRespond failure rejects install although the response was already cached |
| `install-shared-cache` | two versions share one precache: v2 reuses v1 entries and its activate removes v1-only keys |
| `install-wide` | a 160-entry manifest installs entry by entry and activates |
| `match-precache` | matchPrecache: unknown URLs open nothing, known but uninstalled URLs open the precache, a Request argument |
| `route-variations` | directory index, ignored utm_ and fbclid parameters, clean URLs, exact queries |
| `route-options` | PrecacheRoute options: no directory index, no clean URLs, custom ignore patterns with JavaScript RegExp semantics |
| `route-manipulation` | urlManipulation receives the URL with ignored parameters removed and its URLs are tried last |
| `route-protocol` | the router answers only http(s) URLs: data: and about: requests are not handled (B9) |
| `route-cross-origin` | cross-origin requests reach routes with sameOrigin false and print absolute (B9) |
| `route-methods` | routes are per method; the host Request normalises the six standard method names only |
| `route-precedence` | the first registered matching route wins, for the precache route and runtime routes alike |
| `route-default-handler` | default handlers per method answer requests no route matched |
| `route-catch-handlers` | a route catch handler runs first, then the global one; without either the error reaches the caller |
| `route-unregister` | unregisterRoute removes a route; unknown routes and methods throw their WorkboxError codes |
| `route-bound-handler` | createHandlerBoundToURL serves one precached URL for any request; an unknown URL throws non-precached-url |
| `route-precache-strategy` | the precache strategy behind a custom route has no params, so the cache key comes from getCacheKeyForURL |
| `precache-network-fallback` | a precached URL missing from the cache is fetched and, without integrity, not cached |
| `precache-integrity-repair` | with manifest integrity the network response repairs the precache, unless the request conflicts or is no-cors |
| `precache-no-fallback` | fallbackToNetwork false throws missing-precache-entry for an uninstalled entry |
| `precache-offline-miss` | an uncached precache entry while offline reaches the caller as plugin-error-request-will-fetch |
| `cache-first` | CacheFirst: a miss fetches and caches in the background, then hits; offline misses fail with no-response |
| `cache-first-status` | runtime strategies cache only status 200 by default; a custom cacheWillUpdate keeps a 404 |
| `cache-only` | CacheOnly: no-response without a cached entry; a CacheFirst with the same cache name fills it |
| `network-only` | NetworkOnly: every source, with the error nested in no-response |
| `fallback-plugin` | PrecacheFallbackPlugin answers a failed runtime strategy from the precache, or not at all when the fallback is absent |
| `fail-plugins` | a throwing plugin callback at each lifecycle point |
| `fail-non-error` | a plugin or route catch handler that throws a non-Error: each instanceof Error guard ignores it, drops it or lets it through unchanged |
| `network-first` | NetworkFirst without a timeout: the network, cached in the background by its unshifted 200-or-opaque plugin, then the cache when the fetch fails |
| `redirected-runtime` | runtime strategies cache a redirected response as it is; only the precache copies it |
| `error-after-install` | an invalid URL after a successful install throws, and the scenario still reports and deletes its caches (B9) |
| `error-sequence` | every step fails in turn after caches were opened; later steps still run |
| `bench-cache` | benchmark: install and activate 12 entries, then 24 fetches through precache, CacheFirst and NetworkOnly routes |

## Bench

`bench-cache` represents a service worker version at runtime:

1. Install and activate a 12-entry manifest (eight pages and four directory indexes).
2. Register a router with the precache route, a CacheFirst route and a NetworkOnly route.
3. Run 24 fetches: utm-tagged and clean-URL precache hits, directory indexes, CacheFirst misses then hits, and network requests.

No trace plugin is attached, but the plugin lifecycle still runs. `bench-variant.mjs` moves every URL and route prefix under `bench/` to a k-named root of fixed length, and appends the same k token to every revision. No URL, cache key or revision then repeats across calls, and the work per call stays the same. The registry entry (`bench.cases: ["bench-cache"]`) is unchanged.

## Verifiers

From this directory, after building a port:

| Command | What it does |
| --- | --- |
| `node generate-corpus.mjs` | Rewrites `corpus.json` deterministically. The only inputs are the fixture files, hashed for the integrity cases. |
| `node verify.mjs <chrome\|firefox> [reference\|<language>]` | Runs the corpus and four bench variants in a page. It checks the output against `expected.json`, that outputs are location-independent, that every call cleans up its caches, that at least ten cases fail a step after caches were opened, and the variant's properties. `--capture` (reference only) rewrites `expected.json`, which is the same for both browsers. |
| `node verify-worker.mjs <chrome\|firefox> [reference] [<language> …]` | The service-worker integration described above. |

## Changes from the previous contract

The B9 findings, and the new contract's answer to each:

| Finding | Now |
| --- | --- |
| Flattened scenario API; Router, Strategy and the plugin lifecycle fused into one function | ports implement the layers above. The trace plugin makes every lifecycle callback observable, in order, per handler. |
| Non-http precache entry served instead of `null` | `route-protocol` and `install-data-url` |
| `relative()` slicing or prefix-stripping unconditionally | `relative` is defined as a prefix strip. Cross-origin URLs print whole (`route-cross-origin`, `install-cross-origin`, `keys-url-forms`). |
| Numeric `revision` | `keys-numeric-revision` |
| Errors as strings; error values never compared | WorkboxError name and production message are compared; other Errors by name; thrown non-Errors by value. |
| Rust leaked caches when a host call threw | the scenario deletes its caches in a `finally` (Rust: `Scenario::run`). `error-after-install` and `error-sequence` cover it, and `verify.mjs` checks the inventory. |
| `event.waitUntil` never called in phase mode | `events` counts every lifetime the stand-in event receives, and service-worker mode passes the native event. |
| Integrity unreachable | `integrity` entries, `keys-integrity-conflict`, `install-integrity` and `precache-integrity-repair` |
| Behaviour without a cache name undefined | a cache name is always supplied; the defaults are out of the subset. |
