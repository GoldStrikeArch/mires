# idb 8.0.4: Dart port

This port is written by hand. It replaces the earlier port, which implemented the old flattened contract and was partly generated from a shared core (`build/library-matrix/dev-*`). It is a faithful port of idb 8.0.4 (`build/index.js`), the subset in `libraries/idb/CONTRACT.md`, and of what `libraries/idb/reference.mjs` does with it, to Dart 3 compiled by dart2js.

| File | What it is |
| --- | --- |
| `src/idb.dart` | idb: `_promisify` (`promisifyRequest`), `wrap`, `unwrap`, `_cacheDone` (`cacheDonePromiseForTransaction`), the wrapped `Database`, `Transaction`, `ObjectStore`, `Index` and `Cursor`, `Transaction.store` (the `tx.store` trap), the cursor moves (`wrapFunction`'s cursor advance methods), `Shortcut` and `Database.shortcut` (database-extras' `getMethod`), `CursorIterator` and `IteratedCursor` (the async iterator and its proxied cursor), `openDB` and `deleteDB` |
| `src/host.dart` | `dart:js_interop` extension types for IndexedDB and DOM events, the JS globals the port calls, and `jsErrorName` |
| `src/main.dart` | the reference adapter (`_Session`: the script interpreter, the dump and the cleanup) and `run`, which `main()` publishes on `globalThis.__matrixExports` for the builder's facade |

## Boundary

The corpus input is the JS object itself, seen through extension types (`Input`, `Step` and one type per step kind), with no copy. Keys, values, versions, counts, directions, modes and options stay `JSAny?` host values and reach IndexedDB unchanged. On the web, JS null and undefined are both Dart null, but dart2js keeps the JS value itself, so a field read from the input is passed on as the value it is. An absent argument is never sent as null (`null-arguments`):

- Request and cursor-move arguments are lists of the arguments present (`List<JSAny?>`), called with `callMethodVarArgs`, as idb's wrapped methods pass `...args` on.
- `openDB` without a version calls `open(name)` with no version.

Numbers are not narrowed, and a deep value is never walked. The result is a plain JS object `{ log, databases }` of JS arrays, and `run` returns a `JSPromise`.

## The library layer

- **`_promisify` and `_cacheDone`** complete a `Completer` from idb's own listener logic. The listeners are added together and removed together after the first event (`_Listeners`, idb's `unlisten`). `_promisify` takes the result mapping idb's `wrap(request.result)` makes: the value itself, a `Cursor` or null for a cursor request, a `Database` for an open.
- **`Transaction`** creates `done` in its constructor, so `done` exists from the moment the transaction is wrapped (`done-after-complete`). It listens to `error` as well as `abort`. A failure is `tx.error`, or a new `DOMException('AbortError', 'AbortError')` when that is null.
- **`Transaction.store`** tests the second store name, as idb's trap does, before it calls `objectStore`.
- **Wrapped methods** call the host method at once and return a `Future`. A host exception is thrown synchronously, before any Future exists. `Source.request(method, …)` covers the nine request methods. `Method` carries database-extras' read/write split, and its enum names are the host method names.
- **Cursor moves** (`advance`, `continueTo` for `continue`, `continuePrimaryKey`) call the host method, then promisify `cursor.request` again.
- **`openDB`** adds the `upgradeneeded` and `blocked` listeners on the request. It attaches `blocking` in a `then` on the open's Future and returns that Future, so the listener is attached before the caller resumes, as idb's `then` is.
- **`Database.shortcut`** is getMethod's async method: a transaction per call, `tx.store` and then the index for the `FromIndex` forms. A write waits on `Future.wait([request, tx.done], eagerError: true)`, which is `Promise.all`: it fails with the first error as it happens and handles both Futures.
- **`CursorIterator`** is the generator, in Dart's async iteration protocol (`StreamIterator`'s `moveNext` and `current`). The first `moveNext` opens `openCursor(...args)`; a later one waits for the move recorded by the `IteratedCursor` (`_advanced`, the generator's `advanceResults` entry) or calls `continue()`. An `async*` stream pushes its values, while idb's iterator moves only when the loop asks for the next step, after the body has recorded its move; the explicit iterator keeps that pull order.

## Microtasks

The interpreter is `async`/`await` code. dart2js runs a continuation as a microtask, scheduled with a MutationObserver on `self.document`, which the facade leaves real in a page. Every continuation after a request therefore runs inside the event dispatch that completed it, while the transaction is still active (`request-after-error`, `cursor-many`, `done-after-complete`). There are no timers.

## Errors and cleanup

**Error names.** dart2js hands a DOMException that a Dart `catch` receives over as the JS object it is. It rewrites a JS TypeError into one of its own errors, which drop the JS name: `NoSuchMethodError`, `NullError` or an unknown-type error. A RangeError becomes an `ArgumentError`. IndexedDB throws only DOMExceptions, and the TypeErrors of its argument conversions (an invalid version, count, direction or mode, `advance(0)`). So `jsErrorName` reads `name` from a JS object, and names the dart2js errors `TypeError` and `RangeError`. A rejection, and a JS value Dart throws itself, keep their identity.

Each step logs its own failure and throws `_Failed`, which the nearest step with `catch: true` stops (`_perform`). `_Session.run` dumps the live databases in a `try`, and its `finally` closes every connection and deletes every database the script named, also after a failure. A failure in the dump or the cleanup rejects the call.

## Deviations from npm idb (no observable difference in the contract)

- **Classes, not Proxies.** Each wrapped object is a class over its host object, with the subset's members only. There is no `transformCache`, so `wrap(x) === wrap(x)` does not hold; the contract does not observe identity. The one cache effect it does observe is kept: `done` is created when the transaction is wrapped.
- **Futures, not promises.** idb's requests are host promises. Here they are Futures completed from the same listeners.
- **`done` is marked with `Future.ignore()`.** idb leaves a `done` nobody awaits unhandled, which JS only reports as an unhandled rejection; Dart would report it as an uncaught error. A caller that awaits `done` still gets its error. The batch settles each request's Future as soon as it is issued, as `Promise.allSettled` does, so none is left unhandled either.
- **`openDB` takes a null version as absent** and leaves it out of the host call. Dart null stands for both null and undefined here, and a caller with no version can only pass null. A null version is outside the schema (versions are numbers); IndexedDB would throw a TypeError for it.
- **The dump names its transaction `readonly`.** The reference's `db.transaction(names)` gets that mode by default. `Database.transaction` passes its mode as given, so that a `tx` step's null mode still throws (`null-arguments`), and therefore has no "absent" to fall back on.
- **Shortcut arguments.** `shortcut(shortcut, storeName, indexName, …)` takes the index name as its own parameter, where getMethod `shift`s it off `args`.

## Inputs outside the contract

An unknown step, request method, upgrade step, shortcut or cursor action is an `_OutOfContract`. So is a connection the script never opened, `store: null` in a transaction with several stores, a transaction step outside a transaction, and a `call` whose `index` does not match its method's `FromIndex` form. It is never logged, it passes every `catch`, the cleanup runs, and `run` rejects with a TypeError, as the reference does. The reference rejects the same inputs except two. It calls a `call` with a mismatched `index` with that name as the first query argument. And it logs an unknown cursor action as `[name, "error", "TypeError"]`, the error of calling `undefined`.

## Module state

`_upgrades` and `_queryFirst`, two constant sets of method names. Nothing else is kept between calls.

## Performance accommodations

None.
