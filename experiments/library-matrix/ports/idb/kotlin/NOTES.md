# idb 8.0.4: Kotlin/JS port

This port is written by hand. It replaces the earlier port, which implemented the old flattened contract and was partly generated from a shared core (`build/library-matrix/dev-*`). It is a faithful port of idb 8.0.4 (`build/index.js`), the subset in `libraries/idb/CONTRACT.md`, and of what `libraries/idb/reference.mjs` does with it. It targets Kotlin 2.4.10 on the harness's Gradle production lane. It uses no `js("…")` and no hand-written JS.

| File | What it is |
| --- | --- |
| `src/Idb.kt` | idb: `promisify` (`promisifyRequest`), `wrap`, `unwrap`, `cacheDone` (`cacheDonePromiseForTransaction`), the wrapped `Database`, `Transaction`, `ObjectStore`, `Index` and `Cursor`, `Transaction.store` (the `tx.store` trap), the cursor moves (`wrapFunction`'s cursor advance methods), `Shortcut` and `Database.shortcut` (database-extras' `getMethod`), `CursorIterator` and `IteratedCursor` (the async iterator and its proxied cursor), `openDB` and `deleteDB` |
| `src/Host.kt` | external declarations for IndexedDB and DOM events, the JS globals the port calls, and `await` and `launch`, which bridge promises and coroutines |
| `src/Main.kt` | the reference adapter (`Session`: the script interpreter, the dump and the cleanup) and `run`, the module's only export (`@JsExport`) |

## Boundary

The corpus input is a JS object typed by external interfaces (`Input`, `Step` and one interface per step kind) and read in place. Keys, values, versions, counts, directions, modes and options stay the JS values they are (`Any?`) and reach IndexedDB unchanged. An absent field reads as undefined and is passed on as it is, so an absent argument is never null (`null-arguments`). Request arguments are the input's argument array, spread into variadic external methods, as idb's wrapped methods pass `...args` on. So a missing argument is not passed, numbers are not narrowed, and a deep value is never walked. The result is a plain object `{ log, databases }` of JS arrays (`kotlin.js.json`), and `run` returns a `Promise`.

## The library layer

- **`wrap` and `cacheDone`** are host promises (`kotlin.js.Promise`) settled by idb's own listener logic: two listeners added, both removed after the first event. `promisify` takes the result mapping idb's `wrap(request.result)` makes: the value itself, a `Cursor` or null for a cursor request, a `Database` for an open.
- **`Transaction`** creates `done` in its constructor, so `done` exists from the moment the transaction is wrapped (`done-after-complete`). It listens to `error` as well as `abort`. A rejection is `tx.error`, or a new `DOMException('AbortError', 'AbortError')` when that is null.
- **`Transaction.store`** tests the second store name, as idb's trap does, before it calls `objectStore`.
- **Wrapped methods** call the host method at once. A host exception is thrown synchronously, before any promise exists. `Source.request(method, …)` covers the nine request methods. `Method` carries database-extras' read/write split, and `Method.call` is the one `when` that names each host method.
- **Cursor moves** (`advance`, `continueTo` for `continue`, `continuePrimaryKey`) call the host method, then promisify `cursor.request` again.
- **`openDB`** adds the `upgradeneeded` and `blocked` listeners on the request, and attaches `blocking` in a `then` on the open promise, as idb does. That reaction runs before the caller's continuation, because JS runs promise reactions in registration order.
- **`Database.shortcut`** is getMethod's async method as a coroutine (`launch`). It makes a transaction per call, takes `tx.store` and then the index for the `FromIndex` forms, and returns `Promise.all([request, tx.done])` for a write, or `[request]` for a read, at index 0.
- **`CursorIterator.next`** is the generator's step. The first call opens `openCursor(...args)`; a later one awaits the move recorded by the `IteratedCursor` (`advanced`, the generator's `advanceResults` entry) or calls `continue()`.

## Microtasks

The interpreter's steps are suspend functions. `Promise.await` suspends with `suspendCoroutine` and resumes from the promise's `then` callback, and `run` starts the coroutine with `startCoroutine` on `EmptyCoroutineContext`. No kotlinx dispatcher is involved, so every continuation runs as a promise job, inside the event dispatch that settled the promise. A transaction is therefore still active when the next request is issued (`request-after-error`, `cursor-many`, `done-after-complete`).

## Errors and cleanup

A host exception is a JS error, which Kotlin/JS catches as a `Throwable`; its `name` is read with `asDynamic().name`. Each step logs its own failure and throws `Failed`, which the nearest step with `catch: true` stops (`perform`). `Session.run` dumps the live databases in a `try`, and its `finally` closes every connection and deletes every database the script named, also after a failure. A failure in the dump or the cleanup rejects the call, as in the reference.

## Deviations from npm idb (no observable difference in the contract)

- **Classes, not Proxies.** Each wrapped object is a class over its host object, with the subset's members only. There is no `transformCache`, so `wrap(x) === wrap(x)` does not hold; the contract does not observe identity. The one cache effect it does observe is kept: `done` is created when the transaction is wrapped.
- **`tx.store` on an empty scope.** `objectStoreNames.item(0)!!` stands for idb's `objectStore(objectStoreNames[0])`. A transaction's scope is never empty, because `transaction([])` throws.
- **`iterate`** is a class with a suspending `next`, since Kotlin has no async generators. After a step that threw, or after the end, `next` returns null, as a finished generator does.
- **Shortcut arguments.** `shortcut(shortcut, storeName, indexName, …)` takes the index name as its own parameter, where getMethod `shift`s it off `args`.

## Inputs outside the contract

An unknown step, request method, upgrade step, shortcut or cursor action is an `OutOfContract`. So is a connection the script never opened, `store: null` in a transaction with several stores, a transaction step outside a transaction, and a `call` whose `index` does not match its method's `FromIndex` form. It is never logged, it passes every `catch`, the cleanup runs, and `run` rejects with a TypeError, as the reference does. The reference rejects the same inputs except two. It calls a `call` with a mismatched `index` with that name as the first query argument. And it logs an unknown cursor action as `[name, "error", "TypeError"]`, the error of calling `undefined`.

## Module state

`UPGRADES` and `QUERY_FIRST`, two constant sets of method names. Nothing else is kept between calls.

## Performance accommodations

None.
