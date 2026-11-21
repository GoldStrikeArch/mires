# workbox 7.4.1: Rust port (mires)

This is a hand-written port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses, to `no_std` Rust compiled by mires. It covers the subset in `libraries/workbox/CONTRACT.md`. It replaces the earlier flattened-scenario port (B9): the controller, strategy, handler, plugin and router layers are Workbox's own.

| File | What it is |
| --- | --- |
| `src/workbox_core.rs` | `Error` (a `WorkboxError` or a thrown host value) with the production message, `Details`, `ExtendableEvent`, `Task` (an eager, shareable promise), `waitUntil`, `copyResponse`, `cacheMatchIgnoreParams`, `Params`, `HandlerOptions` |
| `src/strategies.rs` | the `Plugin` trait (optional callbacks and `defines`), `PluginState`, `Strategy` (`handle`, `handleAll`, `_getResponse`, `_awaitComplete`), `StrategyHandler`, the `CacheFirst`, `CacheOnly`, `NetworkFirst` and `NetworkOnly` strategies, and `cacheOkAndOpaquePlugin` |
| `src/precaching.rs` | `createCacheKey`, `PrecacheController`, `PrecacheStrategy`, `PrecacheRoute` with `generateURLVariations` and `removeIgnoredSearchParams`, and the cache-key, install-report, fallback, default-cacheability and redirect-copy plugins |
| `src/routing.rs` | `Handler` (what `normalizeHandler` returns), `Route`, `Router` |
| `src/host.rs` | `js_bindings!` and typed handles: `Request`, `Response`, `Url`, `RegExp`, `Cache`, `caches.*`, `fetch`, `timeout`, `Deferred` |
| `src/scenario.rs` | the adapter: the step interpreter, the network, trace, keep/drop, fail and fallback fixtures, the stand-in and native events, the output records and the cleanup |
| `src/lib.rs` | `matrix_run` and the panic handler |

## Boundary

`matrix_run` returns a promise of the script's output. The adapter reads each step's host object into Rust values: strings, `ManifestEntry`, `PrecacheRouteOptions` and plugin and handler specs. Revisions stay host values because the host prints a number revision with ToString. Workbox's logic runs on Rust types over typed handles to the host's CacheStorage, Fetch, Request, Response, URL and RegExp. The output is built as host JSON, in the contract's key order.

## Structure and idiom

- **Plugins** are a trait of twelve optional callbacks. Each returns a boxed future, `Hook<'a, T>`, that borrows the plugin and its arguments. `defines(callback)` is `name in plugin`.
  - `StrategyHandler::callbacks` is `iterateCallbacks`. It reads the strategy's live plugin list at every step and holds no borrow while a callback runs, so a plugin added mid-handler is reached without state, as in JavaScript.
  - The state is one host object per plugin per handler, like Workbox's `MapLikeObject`.
- **Strategies** are one `Strategy` struct. Its `Kind` enum gives the subclass's `_handle`: `CacheFirst`, `CacheOnly`, `NetworkFirst`, `NetworkOnly`, or `Precache(PrecacheStrategy)` with `fallbackToNetwork` and the class's two static plugins. `Strategy::new` does NetworkFirst's constructor work: it unshifts the 200-or-opaque plugin unless a plugin has a cacheWillUpdate.
- **Promises** are `Task<T>`.
  - A task runs its future through `future_to_promise`, whose first poll is synchronous, like an async function. Its typed result sits beside the host promise.
  - The promise settles as the result does, which is what `event.waitUntil` sees. Any number of awaiters read the result.
  - `handleAll` starts `_getResponse` and `_awaitComplete` at once. `handle` drops the second task, which keeps running, as Workbox's dropped promise does.
- **Errors** are `Error::Workbox` or `Error::Host`, and both carry the JavaScript value.
  - A WorkboxError is built as the production class builds it: an `Error` with own `name` and `details` and the message `code :: JSON.stringify([details])`. A nested error therefore stringifies as `{"name":…,"details":…}`.
  - The `instanceof Error` guards Workbox places before it keeps or reports a caught value are `Error::is_error`. A plugin may throw any value. `fail-non-error` throws a string, which every guard must treat apart.
  - The adapter prints a thrown non-Error as `{thrown}`, and names it by its string in traces.
- **Maps** are `mires_js::collections::HashMap`, a host `Map`. It keeps insertion order, and re-inserting a key keeps its place, as `_urlsToCacheKeys` needs.

## Deviations from the npm source

- **`Rc<Box<dyn Plugin>>` and `Rc<Box<dyn ExtendableEvent>>`, not `Rc<dyn …>`.** mires cannot lay out a struct with a `dyn` tail. `RcInner<dyn T>` is refused with "only a slice tail is", see `examples/core-tests/guard_rc_str.rs`. A plugin's identity, used for its state and for the static-plugin comparisons, is its `Rc`. This is not a defect: the backend documents the restriction.
- **Plugin state is a host object, not a typed Rust value.** mires has no `TypeId`, so there is no `dyn Any` downcast, and a host object is what Workbox has.
- **Route handlers are the `Handler` enum.** It is what `normalizeHandler` produces: `Strategy(Rc<Strategy>)` for an object with `handle`, `Callback(Box<dyn Fn>)` for a function. That covers `createHandlerBoundToURL` and the adapter's `respond`. No subset handler reads the `url` member of `HandlerOptions`, so the port leaves it out.
- **The two static PrecacheStrategy plugins are per strategy.** Each strategy's list holds only its own instances, so `===` against the class static and `Rc::ptr_eq` against the strategy's instance decide the same.
- **`generateURLVariations` is `find_variation`.** It searches lazily in the generator's order and stops at the first hit, as the route's `for…of` does, so `urlManipulation` runs only when it is reached.
- **`install` iterates a snapshot of the URL map.** JavaScript iterates the live `Map`, but steps run one at a time, so no entry can be added during an install.
- **The install report checks presence, not `instanceof Request`.** Only Requests are stored in `state.originalRequest`.
- **`cache-put-with-no-response` cannot happen.** `cachePut` takes a `Response` by type.
- **Quota callbacks.** The subset registers none, so the `QuotaExceededError` branch of `cachePut` rethrows directly.
- **The match callback's context omits `event`.** No matcher in the subset reads it.
- **`NetworkFirst` is a `Task` race of one.** `Promise.race([a])` settles as `a` does, so the race is `a`'s task itself. Each lifetime it passes to `handler.waitUntil` is a task that settles with it.
- **The router reads its global catch handler when the rejection arrives**, as Workbox reads `this._catchHandler` inside its `.catch`. It shares the handler cell with the pending request's task. The route's catch handler, and whether to chain at all, are read at request time, as in Workbox.
- **The cache-key plugin's controller is a `Weak`** (`Rc::new_cyclic`), because the controller owns the strategy that holds the plugin.
- **The revisionless warning** goes to `console.warn` with Workbox's text, after each entry, as the production build does.

## B9 fix

- **Cleanup when a host call throws** (`src/scenario.rs:391`). `Scenario::run` keeps the outcome of `report()`, which covers every step and the final dump. It then deletes the scenario's `<prefix>-*` caches on every path, as the reference's `finally` does, and only then returns the outcome.
  - A failing step is recorded and the script goes on.
  - A failure in the dump itself still reaches the cleanup.
  - In 17 cases, among them `error-after-install` (B9's failing input) and `error-sequence`, a step fails after caches were opened. `verify.mjs` checks the CacheStorage inventory around every call.

## Size

The production artifact is about 375 KB. The program renders to 413 KB before Terser, 387 KB of which is the port's own items. The bulk is mires' lowering of the async state machines: each `async fn` and boxed plugin future becomes a resumable JavaScript function, and `Task::spawn` and the drop glue add about 30 KB across their instances. There are no performance accommodations.
