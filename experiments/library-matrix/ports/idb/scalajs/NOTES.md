# idb 8.0.4: Scala.js port

This port is written by hand. It replaces the earlier port, which implemented the old flattened contract and was partly generated from a shared core (`build/library-matrix/dev-*`). It is a faithful port of idb 8.0.4 (`build/index.js`), the subset in `libraries/idb/CONTRACT.md`, and of what `libraries/idb/reference.mjs` does with it, to Scala 3.8 and Scala.js 1.22.

| File | What it is |
| --- | --- |
| `src/Idb.scala` | idb: `promisify` (`promisifyRequest`), `wrap`, `unwrap`, `cacheDone` (`cacheDonePromiseForTransaction`), the wrapped `Database`, `Transaction`, `ObjectStore`, `Index` and `Cursor`, `Transaction.store` (the `tx.store` trap), the cursor moves (`wrapFunction`'s cursor advance methods), `Shortcut` and `Database.shortcut` (database-extras' `getMethod`), `CursorIterator` and `IteratedCursor` (the async iterator and its proxied cursor), `openDB` and `deleteDB` |
| `src/Host.scala` | `@js.native` facades for IndexedDB and DOM events, the JS globals the port calls, and `jsErrorName` |
| `src/Main.scala` | the reference adapter (`Session`: the script interpreter, the dump and the cleanup) and `run`, exported with `@JSExportTopLevel` |

## Boundary

The corpus input is the JS object itself, seen through `@js.native` facade traits (`Input`, `Step` and one trait per step kind), with no copy. Keys, values, versions, counts, directions, modes and options stay the JS values they are: `Any`, or `js.UndefOr[Any]` for a field that may be absent. They reach IndexedDB unchanged, so an absent field is undefined, never null (`null-arguments`). Request arguments are the input's argument array, passed to variadic facade methods (`args*`), as idb's wrapped methods pass `...args` on. So a missing argument is not passed, numbers are not narrowed, and a deep value is never walked. The result is a plain object `{ log, databases }` of JS arrays (`js.Dynamic.literal`), and `run` returns a `js.Promise`.

## The library layer

- **`promisify` and `cacheDone`** complete a `scala.concurrent.Promise` from idb's own listener logic. The listeners are added together and removed together after the first event (`Listeners`, idb's `unlisten`). `promisify` takes the result mapping idb's `wrap(request.result)` makes: the value itself, `Option[Cursor]` for a cursor request, a `Database` for an open.
- **`Transaction`** creates `done` in its constructor, so `done` exists from the moment the transaction is wrapped (`done-after-complete`). It listens to `error` as well as `abort`. A failure is `tx.error`, or a new `DOMException('AbortError', 'AbortError')` when that is null.
- **`Transaction.store`** tests the second store name, as idb's trap does, before it calls `objectStore`.
- **Wrapped methods** call the host method at once and return a `Future`. A host exception is thrown synchronously, as a `js.JavaScriptException`, before any `Future` exists. `Source.request(method, …)` covers the nine request methods. `Method` carries database-extras' read/write split, and `Method.call` is the one match that names each host method.
- **Cursor moves** (`advance`, `continue`, `continuePrimaryKey`) call the host method, then promisify `cursor.request` again.
- **`openDB`** adds the `upgradeneeded` and `blocked` listeners on the request. It attaches `blocking` in a `map` on the open's `Future` and returns that `Future`, so the listener is attached before the caller resumes. idb attaches it in a `then` that also runs before the caller's continuation. Scala does not order callbacks registered on one Future, so the attachment is chained instead.
- **`Database.shortcut`** is getMethod's async method. It makes a transaction per call, takes `tx.store` and then the index for the `FromIndex` forms, and issues the request now; a synchronous failure fails the `Future`. A write completes through `all(request, done)`, which is `Promise.all`: the request's value once both complete, or the first failure as it happens.
- **`CursorIterator.next`** is the generator's step. The first call opens `openCursor(...args)`; a later one waits for the move recorded by the `IteratedCursor` (`advanced`, the generator's `advanceResults` entry) or calls `continue()`. Like an async generator's `next()`, it never throws: a host exception fails the returned `Future`.

## Microtasks

Every Future callback runs on `JSExecutionContext.queue`, whose jobs are promise microtasks. A continuation after a request therefore runs inside the event dispatch that completed it, while the transaction is still active (`request-after-error`, `cursor-many`, `done-after-complete`). There are no timers. The interpreter's loops are recursive `flatMap` chains, which do not grow the stack, because each step runs in a new job.

## Errors and cleanup

A host exception reaches Scala as `js.JavaScriptException(value)`, and `jsErrorName` reads the JS `name` of that value. Each step logs its own failure and fails with `Failed`, which the nearest step with `catch: true` recovers (`perform`). `Session.run` ends with `transformWith`, as the reference's `finally`. It closes every connection and deletes every database the script named, after a success or a failure, and a cleanup failure replaces the result. A failure in the dump or the cleanup rejects the call.

## Deviations from npm idb (no observable difference in the contract)

- **Classes, not Proxies.** Each wrapped object is a class over its host object, with the subset's members only. There is no `transformCache`, so `wrap(x) === wrap(x)` does not hold; the contract does not observe identity. The one cache effect it does observe is kept: `done` is created when the transaction is wrapped.
- **Futures, not promises.** idb's requests are host promises. Here they are Futures completed from the same listeners, and a failure carries the JS error inside `js.JavaScriptException`.
- **`iterate`** is a class with a `next` that gives a `Future`, since Scala has no async generators. After a step that failed, or after the end, `next` gives `None`, as a finished generator does.
- **Shortcut arguments.** `shortcut(shortcut, storeName, indexName, …)` takes the index name as its own parameter, where getMethod `shift`s it off `args`.

## Inputs outside the contract

An unknown step, request method, upgrade step, shortcut or cursor action is an `OutOfContract`. So is a connection the script never opened, `store: null` in a transaction with several stores, a transaction step outside a transaction, and a `call` whose `index` does not match its method's `FromIndex` form. It is never logged, it passes every `catch`, the cleanup runs, and `run` rejects with a TypeError, as the reference does. The reference rejects the same inputs except two. It calls a `call` with a mismatched `index` with that name as the first query argument. And it logs an unknown cursor action as `[name, "error", "TypeError"]`, the error of calling `undefined`.

## Module state

`given ExecutionContext` (`JSExecutionContext.queue`), and `Upgrades` and `QueryFirst`, two constant sets of method names. Nothing else is kept between calls.

## Performance accommodations

None. The bundle is 288 KB after Terser; most of it is the Scala collections and `scala.concurrent` runtime (the ky Scala.js port, which uses the same, is 267 KB).
