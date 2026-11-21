# idb 8.0.4: GopherJS port

A hand-written port of idb 8.0.4 (`build/index.js`) to the contract in `libraries/idb/CONTRACT.md`, in Go 1.21 compiled by GopherJS 1.21.0. It implements idb's own layers and then plays a case's script through them, as `libraries/idb/reference.mjs` drives npm idb. It replaces an earlier port that implemented the old flattened contract and was partly generated (`build/library-matrix/dev-*`); nothing of its structure is reused.

| File | What it is |
| --- | --- |
| `src/idb.go` | idb: `wrap` (`promisifyRequest`), `cacheDone` (`cacheDonePromiseForTransaction`), `Transaction.Store` (the `tx.store` trap), the wrapped request methods on `Source` (which `ObjectStore` and `Index` embed), `Cursor`'s moves (`wrapFunction`), `Database.Shortcut` (database-extras' `getMethod`), `Iterate` and `IterCursor` (the async iterator and `cursorIteratorTraps`), `OpenDB` and `DeleteDB` |
| `src/host.go` | the IndexedDB, DOMException and Promise APIs idb drives, over `*js.Object` |
| `src/script.go` | the corpus input as Go types, and its decoder |
| `src/session.go` | the interpreter: steps, catch scopes, the log, the dump and the cleanup |
| `src/main.go` | `run`, published on `js.Module.exports` |

The interpreter calls only the idb layer, except for the raw request of the `wrap` step (`Source.Raw`, the unwrapped host method) and the properties idb forwards unchanged (`Name`, `KeyPath`, `Version`, `ObjectStoreNames`, `Key`, ...).

## Boundary

`decode` reads the input once into Go types: a `Step` holds one of the step structs behind the sealed `Op` interface, upgrade steps are behind `UpgradeStep`, and cursor actions are `Action{Name, Args}`. Anything outside the schema (an unknown step, request method, upgrade step or cursor action, a `FromIndex` form of a write method, a field of the wrong type) is a `contractError`, which `run` rejects with as a TypeError before any database is opened. The decoder panics with it internally and `decode` recovers it, as `encoding/json` does. Names become Go (UTF-8) strings; the corpus has only well-formed UTF-16. Keys, values, versions, counts, directions, modes and options stay `*js.Object` and reach IndexedDB as they are: a JS `null` is a nil `*js.Object`, and an absent value is `js.Undefined`, never nil, so an absent argument is never passed as `null`. A query method's first argument is the key itself or a `keyRange{factory, args}`, built when the request is issued. The log is a JS array, and `run` returns a host Promise of `{log, databases}`.

## Continuations

- **Continuation-passing on host promises**, as the contract prescribes for GopherJS: every asynchronous step takes `done func(error)`, and a promise's outcome reaches Go through `Promise.Then`, whose callbacks the host runs as microtasks, while an IndexedDB transaction is still active. There are no goroutines and no channels, so no callback ever blocks, and GopherJS's scheduler (which yields to `setTimeout` after a 4 ms pass) is never involved.
- **Outcomes.** `done` gets nil, `logged{}` (a failure the step already logged), or a `contractError` (an input outside the contract that only shows while the script runs: a connection no earlier step opened, `store: null` in a transaction with several stores, a transaction step outside a transaction), which no step logs. `perform` stops `logged{}` at a step with `Catch`. `run` cleans up, then rejects with the contract error's TypeError, as the reference does.
- **Host exceptions.** Every host call that can throw goes through `catch`, which recovers the `*js.Error` GopherJS raises for a JS exception and returns it. idb's synchronous throws are therefore `error` results, and a request is issued, with its promise, when the method is called. `await` logs a synchronous failure or a rejection with the step's label.
- **idb's own promises** are host Promises built as idb builds them: `wrap` and `cacheDone` register and later remove the same `js.MakeFunc` listeners. The shortcut is the host's `Promise.all([request, isWrite && tx.done])`, and a batch's outcomes come from the host's `Promise.allSettled`, where a synchronous failure is a rejected promise, as the reference's `async` issuing function makes it.

## Deviations from npm idb

None changes a result in the contract.

- **Typed handles, not Proxies.** Each wrapped object is a struct over its host object, and its methods are idb's wrapped methods. `ObjectStore` and `Index` embed `Source`, which holds the request methods and cursors they share. A wrapped transaction also holds its `Done` promise. `unwrap` is the host object of a store or index (`Source.Raw` calls its method), and is not a function of its own.
- **No wrap cache.** idb's `transformCache` makes `wrap(x) === wrap(x)`, which the contract does not observe. The one cache effect it does observe is kept: `Done` is created when the transaction is wrapped (`done-after-complete`).
- **`tx.store`** tests `objectStoreNames.length > 1` where idb tests `objectStoreNames[1]`. Store names are sorted and distinct, so a second name is never the empty string.
- **`iterate`** is a struct with a `Next` method that returns a Promise, since Go has no generators. It keeps the generator's states (`unopened`, `positioned`, `finished`) and idb's `advanceResults` entry (`advanced`). `Cursor()` is idb's proxied cursor, the same at every position: `IterCursor` embeds the cursor and overrides `Advance`, `Continue` and `ContinuePrimaryKey` so that they record the move.
- **`openDB`'s `blocking`** is attached in a `then` on the open promise, as in idb, and `IgnoreRejection` handles that chain's rejection.

## JS in the port

None: no `eval`, no `Function`. `kind` classifies an input value with `Object.prototype.toString`, since Go has no `typeof`.

## Module state

None. Each `run` builds its own session and its own database-name prefix from `crypto.randomUUID()`.

## Performance accommodations

None.
