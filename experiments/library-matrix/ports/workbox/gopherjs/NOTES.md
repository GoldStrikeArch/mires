# workbox 7.4.1: GopherJS port

This port is written by hand. It replaces the earlier port of the flattened scenario contract, which a Python translator (`build/library-matrix/dev-wb-comparators/`) generated from a shared core. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles behind `matrix-entry.mjs`, the subset in `libraries/workbox/CONTRACT.md`), and of what `libraries/workbox/reference.mjs` does with them, to Go 1.21 compiled by GopherJS 1.21. The layers are Workbox's own: `PrecacheController` → `PrecacheStrategy` → `Strategy`/`StrategyHandler` → plugins, behind `Router` and `Route`. It embeds no JS.

| File | What it is |
| --- | --- |
| `src/workbox_core.go` | `WorkboxError` with the production message, `ExtendableEvent`, `waitUntil`, `Params`, `HandlerOptions`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/strategies.go` | `Plugin`, `Strategy` (`Handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, CacheFirst, CacheOnly, NetworkFirst (with `cacheOkAndOpaquePlugin`) and NetworkOnly |
| `src/routing.go` | `Handler` (what `normalizeHandler` gives), `Route`, `Router` |
| `src/precaching.go` | `createCacheKey`, `PrecacheController`, PrecacheStrategy, `newPrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams`, and the cache-key, install-report, fallback, default-cacheability and redirect-copy plugins |
| `src/host.go` | `Task`, and Go types over URL, RegExp, Request, Response, Cache and CacheStorage; `catch` |
| `src/adapter.go` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup, and `run` |
| `src/main.go` | publishes `run` on `js.Module.exports` |

## Boundary

`run(input)` reads the script in place as `*js.Object`, member by member. Manifest entries, revisions, response bodies and the fail fixture's `value` stay `*js.Object`, as the caller wrote them. Workbox's logic runs on Go types over the host's CacheStorage, Cache, Request, Response and URL. The output is a JS object built in the contract's key order, and `run` returns a promise of it.

## Structure and idiom

- **Callbacks on host promises, no goroutines.** The contract asks for it: GopherJS's scheduler resumes a goroutine on a timer task, which would reorder the response path against `cachePut`'s `timeout(0)`. Each Workbox async function returns a `*Task[T]`: its typed outcome, and a host promise that fulfils once the outcome is set. `start(body)` runs the body synchronously up to its first wait, as an async function starts. `task.then(next)` continues in a reaction of that promise, a microtask, as the code after an `await` does. A task is eager and any number of awaiters read it, so `_awaitComplete` and the caller both wait on the response. A task's promise never rejects, since a rejection that nobody has awaited yet would be reported as unhandled. `lifetime()` gives the promise that rejects with the thrown value, for `event.waitUntil`. Loops that wait are local recursive closures.
- **Errors.** A WorkboxError is a `*WorkboxError` holding the JS Error this port made (own `name` and `details`, the production message). Anything a host API or a plugin threw is a `*js.Error` holding the value as it was thrown, which can be a string. `isError` is Workbox's `instanceof Error` guard, so the fail fixture's string reaches every guard as the string it is. A host call that throws is recovered by `catch`.
- **Plugins** are a struct of callback fields, nil where the plugin defines none. `defines(name)` is `name in plugin`. A plugin's identity is its pointer. `nextCallback` is one step of `iterateCallbacks`: it reads the strategy's live plugin slice, so a plugin added meanwhile is reached, with no state. `runCallbacks` and the generic `chainCallbacks` are the loops over it.
- **Plugin state** is a plain JS object per plugin per handler, as Workbox's `MapLikeObject` is: the trace keeps its `log` there and the install report its `originalRequest`, read back with Workbox's `instanceof Request` test.
- **The subclasses' `_handle`** is a function field of `Strategy`, which receives the strategy as `this`. `newPrecacheStrategy` closes it over `fallbackToNetwork`.
- **Optional values** are pointers: an optional `*Response`, `*Params`, `Params.Integrity`, a route method, and each `RequestInit` member (nil leaves it out, so an empty method still reaches the Request constructor). `MatchPrecache` takes a URL string or a `Request`, told apart by a type switch, as Workbox tests `instanceof Request`.
- **Maps.** The manifest is an `orderedMap`: insertion-ordered, and setting a known URL keeps its place, as a JS `Map` does. `Install` walks it by index, live, as `for…of` walks the `Map`. The other maps are Go maps, since their order is never seen.
- **`generateURLVariations`** is lazy: each variation goes to a `visit` callback in order, and the later ones (and `urlManipulation`) are not computed once one is in the manifest.
- **JS semantics.** URLs, search parameters and the ignore patterns are the host's (`URL`, `URLSearchParams`, `RegExp`). Parameter names stay JS strings between `keys()`, the pattern's `test` and `delete`. Go strings are UTF-8, and the strings that cross (serialised URLs, cache names, methods, messages) are well-formed, so the conversion is exact.

## Deviations from the npm source

None changes a result in the parity domain.

- **`Promise.all` in `install`** waits for the response, then the completion. The completion waits for the response before it can settle, so the first failure is the same one.
- **`doneWaiting`** waits for each lifetime of a batch in order and then fails with the first failure, which is what `Promise.allSettled` and `find` give.
- **`Promise.race` of one promise** is that task: NetworkFirst's raced network promise and NetworkOnly's fetch.
- **A synchronous throw from `handleRequest`** (the URL parse or a match callback) is the error result of `Router.HandleRequest`; the adapter treats it as Workbox's caller sees the throw.
- **A match callback** answers whether the route matches, and its parameters: nil for a match that is a plain `true`, which Workbox turns into undefined as it does a `{}` or a boolean.
- **The PrecacheRoute options** carry their defaults from `defaultPrecacheRouteOptions`, which the adapter overrides member by member when the script gives one. That is what `generateURLVariations`'s destructuring defaults do. A falsy `directoryIndex` is nil.
- **The handler options omit `url`, and the match options omit `event`.** No handler or matcher in the subset reads them.
- **`cache-put-with-no-response` cannot happen**: `cachePut` takes a `Response` by type.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **The revisionless warning** goes to `console.warn` with Workbox's text, after each entry, as the production build does. It is not compared.

## Module state

The shared plugin values Workbox also has: `cacheOkAndOpaquePlugin` and PrecacheStrategy's `defaultPrecacheCacheabilityPlugin` and `copyRedirectedCacheableResponsesPlugin`. Nothing else outlives a call.

## Performance accommodations

None.
