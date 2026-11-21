# workbox 7.4.1: Melange port

This port is written by hand. It replaces the earlier port of the flattened scenario contract, which a Python translator (`build/library-matrix/dev-wb-comparators/`) generated from a shared core. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles behind `matrix-entry.mjs`, the subset in `libraries/workbox/CONTRACT.md`), and of what `libraries/workbox/reference.mjs` does with them, to OCaml 5.3 compiled by Melange 7.0.1. The layers are Workbox's own: `PrecacheController` → `PrecacheStrategy` → `Strategy`/`StrategyHandler` → plugins, behind `Router` and `Route`.

**Shared source.** `src/host.mli`, `src/workbox_core.ml`, `src/strategies.ml`, `src/routing.ml`, `src/precaching.ml` and `src/adapter.ml` are byte-identical in the Melange and js_of_ocaml ports (README "Per-language notes"). Only `src/host.ml`, the per-compiler interop module, and the entry module differ. See `ports/workbox/jsoo/NOTES.md` for the other side.

| File | What it is |
| --- | --- |
| `src/workbox_core.ml` | the error type and `WorkboxError` with the production message, the promise operators, the event record, `waitUntil`, `copyResponse`, `cacheMatchIgnoreParams` |
| `src/strategies.ml` | the plugin record, `Strategy` (`handle`, `handle_all`, `_getResponse`, `_awaitComplete`), `Strategy_handler`, `Cache_first`, `Cache_only`, `Network_first` (with `cacheOkAndOpaquePlugin`) and `Network_only` |
| `src/routing.ml` | the route handler (what `normalizeHandler` gives), `Route`, `Router` |
| `src/precaching.ml` | `createCacheKey`, `Precache_controller`, `Precache_strategy`, `precache_route` with `generate_url_variations` and `remove_ignored_search_params`, and the cache-key, install-report, fallback, default-cacheability and redirect-copy plugins |
| `src/adapter.ml` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup |
| `src/host.mli` | the host APIs Workbox drives, as one interface for both compilers |
| `src/host.ml` | that interface for Melange: externals onto the JS values |
| `src/index.ml` | the entry module: `run` |

## Boundary

`run(input)` reads the script in place, member by member, through `Host`. Manifest entries and the fail fixture's `value` stay `Host.value`s, as the caller wrote them: `createCacheKey` reports the entry itself, and the host prints a number revision with ToString. Workbox's logic runs over `Host`'s abstract types for the host's CacheStorage, Cache, Request, Response, Headers and URL. The output is a JS object built through `Host`, in the contract's key order, and `run` returns a promise of it.

## Structure and idiom

- **Promises of results.** Workbox's async functions give `'a outcome = ('a, error) result Host.promise`. A call that Workbox would throw out of resolves with `Error error`, and no OCaml exception crosses a promise. `let*` binds a promise, `let*?` an outcome and `let*!` a result. Each outcome is a host promise: it starts when it is made, any number of awaiters read it, and every continuation is a microtask. `handle_all` starts `_getResponse` and `_awaitComplete` at once, and `handle` drops the second, which keeps running.
- **Errors.** `error` is `Workbox_error` of the JS Error this port made (own `name` and `details`, the production message) or `Thrown` of whatever a host API or a plugin threw, as the JS value. The `instanceof Error` guards are `is_error`, which is true for a `Workbox_error` and `Host.is_error` (`Error.prototype.isPrototypeOf`) otherwise. The fail fixture's string therefore reaches every guard as the string it is. A JS promise rejects only where Workbox's would be seen from outside: the lifetimes passed to `event.waitUntil` and the promise `run` returns (`Host.rejecting`).
- **Plugins** are a record of optional callbacks with labelled arguments, built from the empty `plugin` with `{ plugin with … }`. A plugin's identity is physical (`==`, `List.assq`). `iterateCallbacks` is a `Seq.t`, a lazy sequence like Workbox's generator: each step reads the strategy's live plugin list, so a plugin added meanwhile is reached, with no state. `run_callbacks` and `chain_callbacks` are the loops over it.
- **Plugin state** is a plain JS object per plugin per handler, as Workbox's `MapLikeObject` is. The trace keeps its `log` there and the install report its `originalRequest`, which `Host.as_request` reads back with Workbox's `instanceof Request` test.
- **The subclasses' `_handle`** is a function field of the strategy record, which receives the strategy as `this`. `Precache_strategy.make` closes it over `fallbackToNetwork`.
- **Maps** are association lists. `set_entry` replaces a known key in place and appends a new one, as a JS `Map` keeps insertion order. `install` walks the manifest by index, live, as `for…of` walks the `Map`.
- **`generateURLVariations`** is a `Seq.t` with the options' defaults as optional arguments, as the generator destructures them. The route takes the first variation in the manifest with `Seq.find_map`, so the later ones (and `urlManipulation`) are not computed.
- **Melange interop.** `Js.Promise`, `Js.Re` and externals with `mel.send`, `mel.get`, `mel.new` and `mel.scope`. OCaml strings are JS strings here. `Host.catching` wraps every throwing host constructor (`URL`, `RegExp`, `Request`, `Response`) and `Routing.handle` (converting synchronous handler throws into rejected outcomes per `CONTRACT.md`), and `Host.attempt` makes a host promise's rejection, or a synchronous throw, a value.

## Deviations from the npm source

None changes a result in the parity domain.

- **`Promise.all` in `install`** awaits the response, then the completion. The completion awaits the response before it can settle, so the first rejection is the same one.
- **`doneWaiting`** awaits each lifetime of a batch in order and then fails with the first failure, which is what `Promise.allSettled` and `find` give.
- **`Promise.race` of one promise** is that promise: NetworkFirst's raced network promise and NetworkOnly's fetch.
- **A synchronous throw from `handleRequest`** (the URL parse or a match callback) is an `Error` result of `Router.handle_request`; the adapter treats it as Workbox's caller sees the throw.
- **The cache-key plugin holds the controller's manifest lookup, not the controller.** A record cannot refer to itself while it is built, and the controller's strategy holds the plugin. The controller and the plugin share the manifest (`urls_to_cache_keys` is a `ref`). Rust uses a `Weak` for the same reason.
- **Route parameters** are a record of the two members the subset uses, `cache_key` and `integrity`. A match callback answers `None` for no match, or the parameters. A match that is a plain `true` answers empty parameters, which `find_matching_route` turns into undefined, as it does a `{}` or a boolean.
- **The handler options omit `url`, and the match options omit `event`.** No handler or matcher in the subset reads them.
- **`cache-put-with-no-response` cannot happen**: `cache_put` takes a response by type.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **The redirected fixture** (`Host.mark_redirected`) lives in the host module, because it redefines host properties with `Object.defineProperty`.
- **The revisionless warning** goes to `console.warn` with Workbox's text, after each entry, as the production build does. It is not compared.

## Module state

The shared plugin constants Workbox also has: `cache_ok_and_opaque_plugin` and PrecacheStrategy's `default_precache_cacheability_plugin` and `copy_redirected_cacheable_responses_plugin`. Nothing else outlives a call.

## Performance accommodations

None.
