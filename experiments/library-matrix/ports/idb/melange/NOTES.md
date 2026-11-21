# idb 8.0.4: Melange port

A hand-written port of idb 8.0.4 (`build/index.js`) to the contract in `libraries/idb/CONTRACT.md`, in OCaml compiled by Melange 7. It implements idb's own layers and then plays a case's script through them, as `libraries/idb/reference.mjs` drives npm idb. It replaces an earlier port that implemented the old flattened contract and was partly generated (`build/library-matrix/dev-*`); nothing of its structure is reused.

## Layout

The OCaml source is shared by the Melange and js_of_ocaml ports: `src/host.mli`, `src/idb.ml`, `src/script.ml` and `src/session.ml` are byte-identical in `ports/idb/melange/` and `ports/idb/jsoo/`. Each port has its own `src/host.ml`, which implements `host.mli` with its compiler's interop, and its own entry module.

| File | What it is |
| --- | --- |
| `src/idb.ml` | idb: `wrap` (`promisifyRequest`), `Transaction.cache_done` (`cacheDonePromiseForTransaction`) and `Transaction.store` (the `tx.store` trap), the wrapped request methods (`request`, `open_cursor`, `open_key_cursor`), `Cursor` (`wrapFunction`'s cursor moves), `Database.shortcut` (database-extras' `getMethod`), `Iterate` (the async iterator and `cursorIteratorTraps`), `open_db` and `delete_db` |
| `src/host.mli` | the IndexedDB, DOMException and Promise APIs idb drives, and the JS values a script carries |
| `src/script.ml` | the corpus input as OCaml types, and its decoder |
| `src/session.ml` | the interpreter: steps, catch scopes, the log, the dump and the cleanup; `run` |

The interpreter calls only `Idb`, except for the raw request of the `wrap` step (`Idb.call`, the unwrapped host method) and the properties idb forwards unchanged (`store_name`, `key_path`, `version`, `object_store_names`, `key`, ...), which it reads through `Host`.

## Boundary

`Script.decode` reads the input once, through `Host.classify` and `Host.member`, into variants and records: steps, upgrade steps, cursor actions, targets and key-range descriptors. Only the query-first methods (`get`, `getKey`, `getAll`, `getAllKeys`, `count`, `delete`, `openCursor`, `openKeyCursor`, `iterate`) decode an object first argument as a key range; `put`, `add` and `clear` pass plain values. It rejects anything outside the schema (an unknown step, request method, upgrade step or cursor action, a `FromIndex` form of a write method, a field of the wrong type) with a TypeError, before any database is opened. Names become OCaml strings. Keys, values, versions, counts, directions, modes and options stay `Host.value`s and reach IndexedDB as they are, so `null` stays a value and numbers are never narrowed. The log is built as a list of JS arrays, and the result is the plain object `{log, databases}`.

## Promises and failures

- **Host promises.** idb's promises are the host's: `wrap` and `cache_done` are `Host.promise` executors that add and later remove the same listener objects, as idb does. Every continuation is a `then` callback, so it runs as a microtask, while an IndexedDB transaction is still active.
- **A step's outcome** is `(unit, failed) result Host.promise`: `Error Failed` when the step failed, the failure already logged where it happened. `let*` chains outcomes and stops at `Error Failed`; `perform` stops it at a step with `catch`. `let*?` is `Result.bind`, for the synchronous host calls that issue a request (the source, then the key range, then the call).
- **Host exceptions.** Every host call that can throw returns `(_, Host.error) result`: idb's synchronous throws are `Error` values, and a request is issued, with its promise, when the method is called. `check_now` logs a synchronous failure and `check` also follows the promise, whose rejection is logged the same way. Each issued request of a batch is followed at once through `Host.settle`, so every outcome is handled from the start, as `Promise.allSettled` handles them in the reference.
- **Order of evaluation.** OCaml leaves the order of a function's arguments unspecified, so every call that has an effect is sequenced by `let`, and a batch issues its requests with `List.fold_left`.
- **Inputs outside the contract** that only show while the script runs (a connection no earlier step opened, `store: null` in a transaction with several stores, a transaction step outside a transaction) throw a JS TypeError (`Host.type_error`) outside any host-call wrapper. It rejects the promise chain, `run` cleans up, then rejects with it, as the reference does.
- **The dump and the cleanup** use plain promises (`let>` is `Host.bind`): a failure there rejects the call.

## Deviations from npm idb

None changes a result in the contract.

- **Typed handles, not Proxies.** A wrapped store, index, database or cursor is the host object itself, typed by `host.mli`, and `Idb` holds the methods idb changes. A wrapped transaction is a record `{ raw; done_ }`. `unwrap` is therefore the identity on the stores and indexes the `wrap` step unwraps, and is not a function of its own.
- **No wrap cache.** idb's `transformCache` makes `wrap(x) === wrap(x)`, which the contract does not observe. The one cache effect it does observe is kept: `done` is created when the transaction is wrapped (`done-after-complete`).
- **Synchronous throws** are `Error` results, not exceptions; the async shortcut turns one into a rejected promise, as idb's `async` method does.
- **`tx.store`** matches a one-name scope, where idb tests `objectStoreNames[1]`: store names are sorted and distinct, so a second name is never the empty string, and a transaction's scope is never empty.
- **`iterate`** is a record with a `next` function, since OCaml has no generators. It keeps the generator's states (`Unopened`, `At`, `Finished`) and idb's `advanceResults` entry (`advanced`). The value it yields, `{ iteration; cursor }`, is idb's proxied cursor: `Iterate.advance`, `continue` and `continue_primary_key` record the move, and everything else is read from the cursor.
- **`openDB`'s `blocking`** is attached in a `then` on the open promise, as in idb, and `Host.ignore_rejection` handles that chain's rejection.

## Module state

None. Each `run` builds its own session record and its own database-name prefix from `crypto.randomUUID()`.

## Performance accommodations

None.

## Melange interop (`src/host.ml`, `src/index.ml`)

- OCaml strings are JS strings and floats are JS numbers, so names and versions cross as they are.
- The IndexedDB objects are `[@@mel.send]` and `[@@mel.get]` externals. The request methods and the cursor openers are `[@@mel.variadic]`, so they get exactly the script's arguments. An absent optional argument is `Js.Undefined.fromOption None`, which is `undefined`, never `null`.
- A host call that can throw runs inside `catching`: `try … with Js.Exn.Error thrown`, which keeps the thrown DOMException or TypeError as it is.
- Promises are `Js.Promise.t` with the port's own `resolve`, `reject`, `Promise` constructor and `Promise.all` externals, so a rejection is the host's error itself, never an OCaml exception wrapping it. `settle` is `then` plus `catch`.
- The `%identity` externals (`string`, `number`, `bool`, `of_array`, `of_dict`, `of_exn`, `store_source`, `index_source`, `export`) are the typed coercions into `value`, `error`, `source` and the exported promise.
- `index.ml`, the entry, exports `run`, which returns the session's promise as the JS value it is.
