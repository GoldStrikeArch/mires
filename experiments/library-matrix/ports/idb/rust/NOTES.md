# Rust idb port

Hand-written port of idb 8.0.4 (`build/index.js`) to the contract in `libraries/idb/CONTRACT.md`.

## Layout

- `src/idb.rs` is the library:
  - `wrap` is idb's `promisifyRequest`;
  - `cache_done` is `cacheDonePromiseForTransaction`;
  - `Database`, `Transaction`, `ObjectStore`, `Index` and `Cursor` are the wrapped objects;
  - `Transaction::store` is the `tx.store` trap;
  - the `Cursor` moves are `wrapFunction`'s cursor advance methods;
  - `Database::shortcut` is `getMethod` from database-extras;
  - `Iterate` is the async iterator, with `IterCursor` as the proxied cursor whose moves are recorded;
  - `open_db` and `delete_db` are `openDB` and `deleteDB`.
- `src/host.rs` holds the typed bindings to IndexedDB, DOM events and `Promise`. Every IndexedDB call that can throw is `fallible`.
- `src/script.rs` decodes the corpus input into Rust types.
- `src/session.rs` plays the script, as `reference.mjs` drives npm idb.

## Boundary

The input is decoded once into enums and structs: steps, upgrade calls, cursor actions, targets and key-range descriptors. Store, index, connection and database names become Rust `String`s. Keys, values, versions, counts, directions, modes and IndexedDB options stay `JsValue` and reach IndexedDB unchanged. So `null` stays distinct from an absent field, numbers past 2^32 are not narrowed, and a deep value is never walked. Request and cursor methods forward their exact positional argument list through `Reflect.apply` (`host::call_method`) without padding omitted arguments with `undefined`, so missing required arguments throw `TypeError` as in idb. Only the query-first methods (`get`, `getKey`, `getAll`, `getAllKeys`, `count`, `delete`, `openCursor`, `openKeyCursor`, `iterate`) decode an object first argument as a key range; `put`, `add` and `clear` pass plain values. The log is a host array that the open callbacks append to. The result is a plain object, `{ log, databases }`.

## Errors and cleanup

A host exception never unwinds the future. A throwing host call returns `Err(DomError)` through `fallible` bindings, and each step logs its own failure and returns `Failed`. `session::run` then always dumps the live databases, closes every connection it opened and deletes every database it touched. This fixes the B9 finding, where an exception thrown by `IDBKeyRange.only`, `put` or `advance` skipped `close` and `deleteDatabase` and leaked a database. A failing dump or cleanup rejects the call, as the reference's `finally` would.

## Deviations from npm idb (no observable difference in the contract)

- **Typed handles, not Proxies.** Each wrapped object is a newtype over its host object. It exposes only the subset's methods and properties, each wrapping its result the way idb's `get` trap would.
- **No wrap cache.** idb caches wrappers in `transformCache`, which is what makes `wrap(x) === wrap(x)` hold. The contract does not observe identity. The one cache effect it does observe is kept: `Transaction::wrap` creates the `done` promise when the transaction is created, so `done` also settles for a caller that asks after the transaction finished (`done-after-complete`).
- **Listener removal.** idb removes its two listeners with `removeEventListener`. A Rust callback cannot name itself, so both listeners are registered with one `AbortController` signal, and `abort()` removes them together (`host::Listeners`).
- **Synchronous throws.** idb's `openDB`, `deleteDB` and wrapped methods throw synchronously. Here they return `Result`, and a request is issued at call time, not at the first poll.
- **`blocking`.** idb attaches the `versionchange` listener in `openPromise.then`. Here it is attached in the open future's continuation, which also runs before the caller resumes.
- **`tx.store`.** It tests `objectStoreNames.length > 1` where idb tests `objectStoreNames[1]`. Store names sort, so a second name is never the empty string.

## Promise handling

`Database::shortcut` follows `getMethod` literally. It takes `tx.store`, then the index for the `FromIndex` forms, and awaits the host `Promise.all([request, isWrite && tx.done])`. So `done`'s rejection is handled when the request fails first.

An upgrade write is issued and not awaited. Its promise is marked handled with `Request::ignore`, as the reference's `.catch(() => {})` does. `batch` passes every issued request promise (plus `tx.done` when `done: true`) to host `Promise.allSettled` synchronously before awaiting, matching `reference.mjs` so no promise rejects unhandled before its turn in the log. A transaction whose `done` nobody awaits leaves that promise unhandled, as idb does.

## Inputs outside the contract

`script.rs` panics on an unknown step, request method or upgrade step, and on a `FromIndex` form of a write method. `session.rs` panics (`expect`) on a connection the script never opened, and on `store: null` in a transaction with several stores. The panic handler throws, so `run` rejects, as the reference rejects these inputs with a TypeError.

The difference is cleanup. A decoding panic happens before any database is opened. A connection or store lookup can panic after databases were opened, and then they are left behind. The reference's `finally` would still delete them. The corpus has no such input.

## Size

The bundle is 166 KB after Terser. It is a script interpreter over the full contract subset, plus an async state machine per step kind. No performance accommodations.
