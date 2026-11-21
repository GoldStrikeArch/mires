# idb 8.0.4: ReScript port

A hand-written port of idb 8.0.4 (`build/index.js`) to the contract in `libraries/idb/CONTRACT.md`, in ReScript 12.3. It implements idb's own layers and then plays a case's script through them, as `libraries/idb/reference.mjs` drives npm idb. It replaces an earlier port that implemented the old flattened contract and was partly generated (`build/library-matrix/dev-*`); nothing of its structure is reused.

| File | What it is |
| --- | --- |
| `src/Idb.res` | idb: `wrap` (`promisifyRequest`), `Transaction.cacheDone` (`cacheDonePromiseForTransaction`) and `Transaction.store` (the `tx.store` trap), the wrapped request methods (`request`, `openCursor`, `openKeyCursor`), `Cursor` (`wrapFunction`'s cursor moves), `Database.shortcut` (database-extras' `getMethod`), `Iterate` (the async iterator and `cursorIteratorTraps`), `openDB` and `deleteDB` |
| `src/IndexedDb.res` | typed bindings to IDBFactory, IDBRequest, IDBDatabase, IDBTransaction, IDBObjectStore, IDBIndex, IDBCursor, IDBKeyRange and DOMException, with their events |
| `src/Script.res` | the corpus input as ReScript types, and its decoder |
| `src/Index.res`, `src/Index.resi` | the interpreter: steps, catch scopes, the log, the dump and the cleanup; `run`, the module's only export |

The interpreter calls only `Idb`, except for the raw request of the `wrap` step (`Idb.call`, the unwrapped host method) and the properties idb forwards unchanged (`name`, `keyPath`, `version`, `objectStoreNames`, `key`, ...), which it reads with the `IndexedDb` bindings.

## Boundary

`Script.decode` reads the input (`JSON.t`) once, by pattern matching on JSON values, into variants and records: steps, upgrade steps, cursor actions (`list<action>`), targets and key-range descriptors. It rejects anything outside the schema (an unknown step, request method, upgrade step or cursor action, a `FromIndex` form of a write method, a field of the wrong type) with a TypeError, before any database is opened. Names become ReScript strings. Keys, values, versions, counts, directions, modes and options stay `JSON.t` and reach IndexedDB as they are, so `null` stays a value and numbers are never narrowed. The log is an `array<JSON.t>` that the open callbacks append to; the result is the record `{log, databases}`, which is the JS object itself.

## Host interop

- **Async.** Every step is a ReScript `async` function, and `await` is native JS `await`: each continuation is a promise microtask, which runs while an IndexedDB transaction is still active. `wrap` and `cacheDone` are `Promise.make` executors that register and later remove the same listener functions, as idb does.
- **Requests are issued at call time.** A wrapped method calls the host method at once and returns its promise. A batch issues every request before it awaits any. Each issued request is followed at once through `settle`, an async function that turns a rejection into `Error`, so every outcome is handled from the start, as `Promise.allSettled` handles them in the reference.
- **Host exceptions.** A host method throws its JS exception, caught as `JsExn` by `switch … | exception JsExn(error)` around the call (`checkNow` for a synchronous call, `check` for one that returns a promise: both its synchronous exception and its rejection). The step logs the error's `name` and throws `Failed`; `perform` stops `Failed` at a step with `catch`.
- **Arguments.** The request methods and the cursor openers are `@variadic` externals: they get exactly the script's arguments, the first one built into a key range when it is one. An absent optional argument (a version, an `options` object, a `continue` key, a mode for the dump's transaction) is `option` `None`, which ReScript passes as `undefined`, never `null`.
- **Unions.** A method argument is a JSON value or a key range: `IndexedDb.value` and `IndexedDb.range` are `%identity` externals into the abstract `argument` type. `ObjectStore.asSource` and `StoreIndex.asSource` are the same kind of upcast into `source`, the type both share their read methods on.
- **No early return.** ReScript has no `return` or `break`, so a walk's positions and a position's actions are async recursive functions over a `list<action>` (`walkActions`, `walkFrom`, `iterateActions`, `iterateFrom`). Each recursion runs after an `await`, so a 3000-position walk does not grow the stack.
- **Inputs outside the contract** that only show while the script runs (a connection no earlier step opened, `store: null` in a transaction with several stores, a transaction step outside a transaction) raise `OutOfContract`, which no step logs. `run` cleans up, then rejects with its TypeError, as the reference does.

## Deviations from npm idb

None changes a result in the contract.

- **Typed handles, not Proxies.** A wrapped store, index, database or cursor is the host object itself, typed by its binding module, and `Idb` holds the methods idb changes. A wrapped transaction is a record `{raw, done}`. `unwrap` is therefore the identity on the stores and indexes the `wrap` step unwraps, and is not a function of its own.
- **No wrap cache.** idb's `transformCache` makes `wrap(x) === wrap(x)`, which the contract does not observe. The one cache effect it does observe is kept: `done` is created when the transaction is wrapped (`done-after-complete`).
- **`tx.store`** tests `objectStoreNames.length > 1` where idb tests `objectStoreNames[1]`. Store names are sorted and distinct, so a second name is never the empty string.
- **`iterate`** is an object with an async `next`, since ReScript has no generators. It keeps the generator's states (`Unopened`, `At(cursor)`, `Finished`) and idb's `advanceResults` entry (`advanced`). The value it yields, `{iteration, cursor}`, is idb's proxied cursor: `Iterate.advance`, `continue` and `continuePrimaryKey` record the move, and everything else is read from the cursor.
- **`openDB`'s `blocking`** is attached in `openPromise.then`, as in idb, and `.catch` handles that chain's rejection.

## Module state

None. Each `run` builds its own session (log, connections, opened and attempted databases, the live list) and its own database-name prefix from `crypto.randomUUID()`.

## Performance accommodations

None.
