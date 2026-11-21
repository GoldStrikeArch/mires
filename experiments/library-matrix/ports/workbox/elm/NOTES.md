# workbox 7.4.1: Elm port

This port is written by hand. It replaces the earlier port (B9), whose JS host held the precaching, routing and strategy logic and whose Elm only picked a route. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles that `matrix-entry.mjs` links), for the subset in `libraries/workbox/CONTRACT.md`, and of what `libraries/workbox/reference.mjs` does with it. It targets Elm 0.19 with elm/core and elm/json only. Every layer is Elm, and so is the adapter. The JS host executes only the contract's primitives (CONTRACT.md "Elm: the I/O boundary").

| File | What it is |
| --- | --- |
| `src/Io.elm` | `Io w a`, the computation Workbox is written in: host primitives (answered at once, or with a promise), eager promises with any number of awaiters, `timeout(0)`, updates of the world, `try`/`catch`/`finally`; `Thrown` and the `instanceof Error` guard; the production WorkboxError message |
| `src/Host.elm` | the contract's primitives as `Io` actions, over handles: CacheStorage, Cache, fetch, Request, Response, URL, RegExp, the environment, the native event, `markRedirected` |
| `src/World.elm` | the heap: what JavaScript Workbox keeps in mutable objects (plugin lists, handler state, controller maps, routers, route catch handlers, install reports, stand-in events) and the fixtures' shared state; the types the layers pass between them (`Plugin`, the callback params, `Event`, `Strategy`, `StrategyHandler`, `Route`, `RouteHandler`, `Params`) |
| `src/WorkboxCore.elm` | `WorkboxError`, `waitUntil(event, asyncFn)`, `timeout`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/Strategies.elm` | `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler` (`fetch`, `fetchAndCachePut`, `cacheMatch`, `cachePut`, `getCacheKey`, `iterateCallbacks`, `waitUntil`, `doneWaiting`, `destroy`), `CacheFirst`, `CacheOnly`, `NetworkFirst`, `NetworkOnly`, `cacheOkAndOpaquePlugin` |
| `src/Precaching.elm` | `PrecacheEntry`, `createCacheKey`, `PrecacheController`, `PrecacheStrategy` and its two static plugins, the cache-key, install-report and fallback plugins, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams` |
| `src/Routing.elm` | `Route` and `Router` |
| `src/Scenario.elm` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup |
| `src/Main.elm` | the program and its runtime, which runs the adapter's `Io` against the host |
| `js/index.js` | the JS host |

## Boundary

The host sends the input through `receive`. Elm reads each step with elm/json, in place. Workbox and the adapter run as `Io` fibers. Each primitive is one `command`; the host answers it at once through `receive`: with what it gave or threw, or, for a primitive that answers with a promise, that it has started, and then with the promise's outcome. Host objects stay in the host, behind handles. The output goes back through `command` as `exit`.

## The host

`js/index.js` performs only the contract's primitives, each of them one call, and holds no Workbox logic: no URL variation, cache-key, plugin-dispatch or ordering code. A Request, Response or URL reply carries the properties Workbox reads (`url`, `method`, `mode`, `integrity`; `status`, `statusText`, `url`, `redirected`, `type`; `href`, `origin`, `protocol`, `pathname`), so a read is not a round trip. A URL write answers the URL as it is after the write. A thrown or rejected value travels as `{name, message, isError, text}`, where `isError` is `instanceof Error` and `text` is `String(value)`.

- **Timing.** A synchronous primitive answers before the port call returns. A promise's outcome is sent in that promise's continuation, never through a timer. The only timer is `timeout(0)`, which is Elm's `Process.sleep 0`.
- **Size.** The host is about 130 lines, far under half of the bundle, so the row is not "mostly JS".
- **RegExp.** `RegExp(source).test(name)` is the contract's primitive, as `newRegExp` (at the `precacheRoute` step, where a bad source throws its SyntaxError, as in the reference) and `regExpTest`. elm/regex, which wraps the same JS RegExp, is not in the toolchain's package cache.

## The runtime

`Main` runs one fiber at a time, as JavaScript runs one job at a time, with a queue of ready fibers as JavaScript keeps its microtask queue.

- A fiber runs until it waits: for a host promise, a timer or a promise of its own.
- `spawn` runs the new fiber first, and its caller next, as an async function call runs up to its first `await` before the caller goes on.
- A settled promise queues its waiters behind the ready fibers, as promise reactions are queued. Awaiting a settled promise queues the continuation too.
- When a host promise settles, its fiber runs, and every fiber it makes ready runs after it, before the host gets control back.
- `Process.sleep 0` parks its fiber, and the runtime goes on with the ready fibers at once (a `Task.succeed` message after the sleep in the same batch). The timer wakes the fiber.
- Port commands are never batched, as Elm delivers a batch's port commands in reverse order.
- After `exit` the runtime keeps serving the fibers that still run. In service-worker mode a background `cachePut` outlives the call, as it outlives the reference's `run`.

Elm compiles the self-recursive `run` to a loop whose pattern variables every branch and iteration share. A closure made inside it and called later would see the variable as the last branch left it. The settling of a spawned fiber's promise is therefore built outside `run` (`settling`).

## Structure and idiom

- **Plugins** are `Plugin` values: a record of twelve `Maybe` callbacks, `Nothing` by default, with an id. The id is the plugin's identity for its state and for the static-plugin comparisons. Workbox's module-level plugins have fixed ids: `cacheOkAndOpaquePlugin` 1, `defaultPrecacheCacheabilityPlugin` 2, `copyRedirectedCacheableResponsesPlugin` 3. The fixture plugins are Elm values like Workbox's own.
- **`iterateCallbacks`** reads the strategy's plugin list from the world at every step (`nextCallback`), so a plugin added while a handler runs is reached, without state. A plugin's state is a `Dict String Value` per plugin present when the handler was created, as Workbox's `MapLikeObject`. The install report keeps the original Request in it, and the trace keeps its log id.
- **Strategies** are a record of the cache name, an id for the plugin list in the world, and the subclass's `_handle` as a function. PrecacheStrategy's `_handle` closes over `fallbackToNetwork`.
- **Promises** are eager and have any number of awaiters. A promise's value travels as JSON with the decoder that reads it back, so one runtime table holds the promises of every type, as a JS promise holds any value. `Promise.race([a])` is a fiber that awaits `a`. `Promise.allSettled` awaits each promise of a batch in turn and takes the first rejection in batch order.
- **Errors.** `Thrown` is a `WorkboxError` (name and details) or a host value as the host describes it. `Io.isError` is Workbox's `instanceof Error` guard, applied at each guard the contract lists. The fail fixture throws `new Error('fail')` as an Error named `Error`, or its value as a non-Error.
- **Mutable state** is the world, changed by `Io.modify`. A controller's `urlsToCacheKeys` keeps its insertion order in a list beside the `Dict`, since a `Dict` is ordered by key, and re-setting a URL keeps its place.
- **Strings** are JS strings: elm/core's `String.startsWith`, `dropLeft`, `replace` and `length` work on UTF-16 code units, as the reference's `startsWith`, `slice` and `split`/`join` do. `JSON.stringify` is `Encode.encode 0`.

## Deviations from the npm source

None changes a result in the parity domain.

- **`location`** is read once per call, with `location.href` and `location.origin`, and kept in the world. Workbox reads it at each use, but it does not change during a call.
- **Microtask counts.** A plugin callback is an Elm function, and calling one takes no microtask, where awaiting a JS async function takes one. The subset observes no count of microtasks: within a handler the trace order follows the code, and the only timer is cachePut's `timeout(0)`, which every microtask-level step precedes either way.
- **`install` iterates a snapshot of the URL map.** Workbox iterates the live `Map`, but steps run one at a time, so no entry can be added during an install.
- **`cache-put-with-no-response` cannot happen**: `cachePut` always has a `Response`.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **`NetworkOnly`** awaits `fetch` directly: `Promise.race` of one promise settles as it does, and the timeout error is unreachable without `networkTimeoutSeconds`.
- **`HandlerCallbackOptions` has no `url`**: no handler in the subset reads it.
- **The revisionless warning** goes to `console.warn` with Workbox's text after each entry, as the production build does.

## Module state

None in the port. Each call has its own Elm program instance and handle table. Elm's kernel keeps its scheduler and effect queues at module level, shared by the instances, as every Elm program does.

## Performance accommodations

One: a Request, Response or URL reply carries the properties Workbox reads, so a property read is not a host round trip (see "The host").
