# Library contract: `idb`

| Field | Value |
| --- | --- |
| Package | `idb@8.0.4`, entry `build/index.js` |
| Reference adapter | `reference.mjs`: plays a case's script through `openDB`, `deleteDB`, `wrap`, `unwrap` and the wrapped database, transaction, store, index and cursor objects, then dumps and deletes its databases |
| API subset | see "The idb subset", which also lists what is left out and why |
| Input | `{ steps: Step[] }`, a script of idb calls ("Input") |
| Output form | `json`, async: `{ log: Entry[], databases: Dump[] }` ("Output"); key order significant |
| Normalisation | `plain-object-realm` (`normalise.mjs`, see "Normalisation") |
| Parity environments | chrome, firefox, safari (IndexedDB; Node has none). Safari has not been run: no WebDriver session was available, so WebKit's error names for the edge cases are unverified |
| Corpus | `corpus.json`, 78 cases |
| Bench cases | `bench-transactions`, baseIterations 1, variant `bench-variant.mjs` |
| Caching | idb keeps module-level WeakMaps keyed by IDB objects (wrappers, `done` promises, iterator cursors). A port may cache by host object the same way, never by input |
| Files | none |

Each call to `run` works on databases of its own. Their names are `mires-matrix-idb-` plus a fresh UUID, a `-`, and the script's logical name. Before it settles, the call closes every connection it opened and deletes every database it named, also after a failure. `verify.mjs` runs every case twice at once and checks that the browser's database inventory returns to where it started.

## The idb subset

A port implements these parts of idb itself, on top of the host's IndexedDB. Each port must have a library layer with this structure. The script interpreter calls that layer, never IndexedDB directly, except for the raw request in `wrap`.

1. **`wrap(request)`, idb's `promisifyRequest`.** It returns a promise (or the language's future) that is settled once:
   - on the request's `success`, with the result wrapped;
   - on its `error`, with `request.error`.

   Both listeners are removed after the first event. A cursor request fires once per move, so each move wraps the same request again.
2. **`unwrap(x)`.** It returns the host object behind a wrapped value.
3. **`tx.done`, idb's `cacheDonePromiseForTransaction`.** It is created when the transaction is created (wrapped), not when `done` is first read. It is settled by the first of three events:
   - `complete` resolves it with `undefined`;
   - `error` rejects it;
   - `abort` rejects it.

   A rejection uses `tx.error`, or a new `DOMException('AbortError', 'AbortError')` when that is null. A request error bubbles to the transaction before the transaction records it. So after a ConstraintError, `done` rejects with that new AbortError (`batch-constraint`).
4. **`tx.store`.** It is the only store in the transaction's scope, or undefined when there are several. idb tests `objectStoreNames[1]` before it calls `objectStore()`. So on a finished transaction, reading it throws InvalidStateError when the scope has one store, and gives undefined when it has several (`store-finished`).
5. **Wrapped methods.** Store and index methods return wrapped results: requests become promises, and `index()`, `objectStore()`, `createObjectStore()`, `createIndex()` and `transaction()` return wrapped objects. A host exception is thrown synchronously, before any promise exists.
6. **Cursor moves.** `continue(key?)`, `advance(count)` and `continuePrimaryKey(key, primaryKey)` call the host method, then return `wrap(cursor.request)`. That promise resolves with the same wrapped cursor, or with `null` past the end. `update(value)` and `delete()` return wrapped requests.
7. **`openDB(name, version, { upgrade, blocked, blocking })`.**
   - `upgrade(db, oldVersion, newVersion, transaction)` runs synchronously inside `upgradeneeded`, with the wrapped connection and the wrapped versionchange transaction.
   - `blocked(currentVersion, blockedVersion, event)` is called on the open request's `blocked`.
   - `blocking(currentVersion, blockedVersion, event)` is attached to the connection's `versionchange` only after the open succeeds.
   - An absent version opens the current version, or version 1 for a new database.
8. **`deleteDB(name, { blocked })`.** It resolves `undefined`; `blocked(currentVersion, event)`.
9. **Database shortcuts (database-extras' `getMethod`).**
   - The read methods are `get`, `getKey`, `getAll`, `getAllKeys` and `count`, plus their `…FromIndex(store, index, …)` forms.
   - The write methods are `put`, `add`, `delete` and `clear`.

   Each call opens one transaction, `readwrite` for a write and `readonly` otherwise, on `tx.store`, then on the index for the `FromIndex` forms. It resolves with the request's result. A write resolves only after `tx.done`, and a rejection is the request's error when the request failed. A synchronous failure, such as an unknown store, rejects the call. The corpus cannot tell whether a write waits for `done`: no case can make a commit fail. Reviewers check it in the source.
10. **`iterate(query, direction)` on a store or index (the async iterator).**
    - The first step opens `openCursor(query, direction)`.
    - Each later step awaits the move the loop body made on the yielded cursor. The body's `advance`, `continue` and `continuePrimaryKey` are intercepted: they call the host method but return nothing.
    - When the body made no move, the step calls `continue()`.

    Two moves in one body throw the host's InvalidStateError from the second.

Left out, each for one reason:

- **`openDB`'s `terminated`.** The browser calls it only when it closes a connection abnormally, which no script can trigger deterministically.
- **Wrapper identity.** `wrap(x) === wrap(x)` comes from idb's `transformCache`. It is not observable across a language boundary, and no port has Proxies.
- **`replaceTraps`.** A hook for idb's own add-on modules, not part of the application API.
- **`tx.commit()` and the `durability` option.** Host pass-through with no effect a script that awaits `done` can observe.
- **Iterating a cursor itself** (`for await (const c of cursor)`). It is the same `iterate` generator with a cursor as `this`, so one entry point is enough. `for await (const c of store)` is `store.iterate()`.

Every request must be issued when the method is called, not when its result is awaited. A batch issues all its requests before it awaits any. Every continuation must run as a microtask, because IndexedDB commits a transaction once control returns to the event loop with no request pending (see "Per-language guidance").

## Input

`steps` is a list of steps run in order, each awaited before the next. Fields marked optional may be absent; every other field is present. A step's `catch` (boolean, absent = false) decides what a failure does:

- **`catch: true`.** A failure of this step, or of a step inside it, stops here. The rest of this step's body is skipped, and the script continues after this step.
- **otherwise.** The failure goes to the nearest enclosing step with `catch: true` (`catch-scope`). If there is none, the script stops, and the call goes on to the dump and cleanup.

A failure is logged once, as an `error` entry, by the step where it happened.

### Top-level steps

| `op` | Fields | Call | Log |
| --- | --- | --- | --- |
| `openDB` | `db`, `conn`, `version`?, `upgrade`?: UpgradeStep[], `blocking`?: `"log"` / `"close"`, `blocked`?: `"log"` / `{ "close": conn }` | `openDB(prefix + db, version, callbacks)`, with a callback for each field present | callbacks as they run, then `["openDB", "ok", [db.version, objectStoreNames]]` |
| `close` | `conn` | `db.close()` | `["close", "ok"]` |
| `deleteDB` | `db`, `blocked`? | `deleteDB(prefix + db, { blocked })` | `["blocked", currentVersion]` if it runs, then `["deleteDB", "ok"]` |

A successful `openDB` registers the connection under `conn`, replacing an older one of that name. The callbacks:

- **`upgrade`.** Logs `["upgrade", oldVersion, newVersion]`, then runs its upgrade steps.
- **`blocked`.** Logs `["blocked", currentVersion, blockedVersion]`. With `{ close }`, it then closes that connection, resolved when the step starts.
- **`blocking`.** Logs `["blocking", conn, currentVersion, blockedVersion]`, where `blockedVersion` is null for a deletion. With `"close"`, it then closes its own connection (`event.target`).

`deleteDB`'s `blocked` takes the same two forms.

### Steps at any level

| `op` | Fields | Call | Log |
| --- | --- | --- | --- |
| `tx` | `conn`, `stores` (name or names), `mode`, `steps` | `db.transaction(stores, mode)`, then its steps inside it | `["tx", "error", name]` only if `transaction()` throws |
| `call` | `conn`, `method`, `store`, `index` (name or null), `args` | `db[method](store, [index,] ...args)`: a database shortcut. `method` is `get`, `getKey`, `getAll`, `getAllKeys`, `count`, `put`, `add`, `delete`, `clear`, `getFromIndex`, `getKeyFromIndex`, `getAllFromIndex`, `getAllKeysFromIndex` or `countFromIndex` | `[method, …outcome]` |

### Steps inside a transaction

| `op` | Fields | Call | Log |
| --- | --- | --- | --- |
| `get` `getKey` `getAll` `getAllKeys` `count` `put` `add` `delete` `clear` | `store`, `index`, `args` | `source[op](...args)`, awaited | `[op, …outcome]` |
| `batch` | `requests` (request steps), `abort`, `done` | issues every request in order, each call returning before the next is made; then `tx.abort()` if `abort`; then awaits all, plus `tx.done` if `done` | `["abort", …]` if `abort`, then one `[op, …outcome]` per request in issue order, then `["done", …]` if `done`. A batch never fails |
| `openCursor` `openKeyCursor` | `store`, `index`, `args`, `positions`, `rest` | opens a cursor and walks it with the promise-returning moves | see "Cursor walks" |
| `iterate` | `store`, `index`, `args`, `positions`, `rest` | `source.iterate(...args)`, walked with the iterator | see "Cursor walks" |
| `abort` | | `tx.abort()` | `["abort", …outcome]` |
| `done` | | `await tx.done` | `["done", …outcome]` |
| `store` | | `tx.store` | `["store", "ok", name]`, or `["store", "ok"]` when it is undefined |
| `wrap` | `store`, `index`, `method`, `args` | `await wrap(unwrap(source)[method](...args))`, where `method` is one of the nine request methods, `get` to `clear` | `["wrap", …outcome]` |

Any other `op`, `method` or upgrade step is outside the contract, and so are a `conn` that no earlier step opened and `store: null` in a transaction with several stores. The reference rejects such a call with a TypeError, after cleanup, and never logs it.

A nested `tx` or a `call` inside a transaction runs while the outer transaction is still open, so a nested `readwrite` transaction on the same store waits for the outer one to finish (`done-after-complete`).

The source of a request is `store` and `index`:

- `store` is a name, for `tx.objectStore(store)`, or `null`, for `tx.store`; `null` is only used in one-store transactions;
- `index` is a name, for `.index(index)`, or `null`.

Only the read methods take an index. The source is resolved for each step, and its errors (NotFoundError, InvalidStateError) are that step's failure.

### Upgrade steps

They run synchronously inside the upgrade callback, in order. A failure is logged and the callback goes on: an upgrade step never ends the script.

| `op` | Fields | Call | Log |
| --- | --- | --- | --- |
| `createStore` | `name`, `options`? | `db.createObjectStore(name, options)` | `["createStore", "ok", store.name]` |
| `deleteStore` | `name` | `db.deleteObjectStore(name)` | `["deleteStore", "ok"]` |
| `createIndex` | `store`, `name`, `keyPath`, `options`? | `transaction.objectStore(store).createIndex(name, keyPath, options)` | `["createIndex", "ok", index.name]` |
| `deleteIndex` | `store`, `name` | `transaction.objectStore(store).deleteIndex(name)` | `["deleteIndex", "ok"]` |
| `put` `add` | `store`, `args` | `transaction.objectStore(store)[op](...args)`, issued and not awaited | only a synchronous failure. A failing write aborts the upgrade, which rejects the `openDB` with AbortError |
| `abort` | | `transaction.abort()` | `["abort", "ok"]` |

### Arguments

`args` is a positional list, and a missing argument is not passed. For `get`, `getKey`, `getAll`, `getAllKeys`, `count`, `delete`, `openCursor`, `openKeyCursor` and `iterate`, and for the same arguments of the shortcuts, the first argument is a query:

- an array, a string, a number or `null` is passed as it is. So `getAll(null)` is unbounded, while `get(null)` and `delete(null)` throw DataError.
- an object is a key range, built before the call:

  | Object | Key range |
  | --- | --- |
  | `{ only }` | `IDBKeyRange.only(only)` |
  | `{ lower, upper, lowerOpen?, upperOpen? }` | `bound(lower, upper, lowerOpen, upperOpen)` |
  | `{ lower, lowerOpen? }` | `lowerBound(lower, lowerOpen)` |
  | `{ upper, upperOpen? }` | `upperBound(upper, upperOpen)` |

  Presence is tested with `in`, so `{ only: null }` builds `only(null)` and throws DataError. This is the B9 decision: `null` is a value, never "absent". An absent open flag is false.

The other arguments are passed as they are: `put(value, key?)`, a `getAll` count, a cursor direction, a transaction mode. Numbers are never narrowed, so `getAll(null, 2 ** 32)` throws TypeError where a port that wraps to 32 bits would return everything.

An absent argument is omitted or passed as `undefined`, never as `null`, because WebIDL treats only `undefined` as missing (`null-arguments`):

- `put(value, null)` throws DataError;
- `openCursor(query, null)` and `transaction(stores, null)` throw TypeError;
- Chrome and Firefox both treat `continue(null)` as `continue()`.

A port whose "nothing" becomes JS `null` must pass `undefined` instead: Kotlin/JS `null`, Dart `JSAny?` `null`, Scala.js `null` rather than `js.undefined`, GopherJS `nil`. The same holds for an absent `version`, `options` or `key`.

### Cursor walks

Each step's `positions[i]` lists the actions taken at the i-th cursor position. `rest` lists the actions for every position after them. Each position first logs one entry:

- `["cursor", key, primaryKey, value]`, or `["cursor", key, primaryKey]` for `openKeyCursor`.

The actions:

| Action | Walk (`openCursor`, `openKeyCursor`) | `iterate` |
| --- | --- | --- |
| `["continue"]`, `["continue", key]`, `["advance", n]`, `["continuePrimaryKey", key, primaryKey]` | calls the move; the walk awaits it after the position's last action | calls the intercepted move; the iterator awaits it |
| `["update", value]`, `["delete"]` | `await cursor.update(value)` or `cursor.delete()`, logged as `["update", …]` or `["delete", …]` | the same |
| `["break"]` | ends the walk at once | ends the loop at once |

Actions run one at a time, in order, and each call returns before the next is made. An action that fails ends the walk at once: the actions after it at that position do not run (`walk-throw-then-action`).

After a position's actions:

- A walk with no move stops.
- `iterate` with no move continues.

A cursor that reaches the end logs `["end"]`; `break` and a stopped walk log nothing more.

A failure logs `[label, "error", name]`, with these labels:

- opening the cursor: the step's `op`;
- a move, synchronous or awaited: the move's name;
- anything that the iterator's own step throws: `iterate`.

## Output

`{ log, databases }`.

An outcome is one of:

- `"ok"`, when the result is `undefined`;
- `"ok", value`;
- `"error", name`, the error's `name`; messages are not compared.

A miss (`undefined`) and a stored `null` therefore differ: `["get", "ok"]` against `["get", "ok", null]`. The log never contains `undefined`.

`databases` holds one dump per live database. The live databases form a list:

- a successful `openDB` appends its `db`, unless it is already in the list;
- a successful `deleteDB` removes it.

So a database that is deleted and opened again moves to the end (`dump-order-reopen`). A dump opens the database again without a version and reads it in one readonly transaction:

```
[db, version, [[storeName, keyPath, autoIncrement, [[indexName, keyPath, unique, multiEntry], ...], [[primaryKey, value], ...]], ...]]
```

Stores and indexes are in name order, records in key order: `getAllKeys()` zipped with `getAll()`. A failure in the dump or the cleanup rejects the call.

## Parity domain

The corpus stays inside this domain; inputs outside it are out of contract.

- **The schema.** Inputs follow the schema above:
  - every `conn` names a connection opened earlier in the script;
  - `store: null` is used only in one-store transactions;
  - index sources are used only with read methods;
  - booleans are booleans.
- **No deadlocks.** No script leaves an open blocked forever. Before every version change or deletion, the transactions on the old connection have finished: their block awaited `done`, or they were shortcuts.
- **JSON data.**
  - Values are JSON.
  - Keys are numbers, strings or arrays of keys.
  - Strings are well-formed UTF-16.
- **Nesting.** Values nest at most 1000 levels. The fixtures pass through `structuredClone` and WebDriver, and the checker's canonical serializer recurses.
- **What is compared.** Error names are compared, messages are not. Wrapper identity is not observable. Only the event order IndexedDB defines is: there are no microtask races between concurrent promises.

## Normalisation

`normalise.mjs` (`plain-object-realm`) copies plain records into the checking realm. Values, key order, `undefined` and -0 are kept. It is the jmespath rule.

Firefox gives a module imported from a WebDriver script the records it reads from IndexedDB, and `structuredClone`'s results, as objects of another realm. The json form's prototype check rejects those objects. So without the rule, the npm reference fails its own output check in Firefox on 67 of the 78 cases (`reference-error`). Chrome and an ordinary page are unaffected. The rule is applied to both sides.

The registry declares it as `matrix.json` `libraries.idb.normalisation`:

```json
{ "name": "plain-object-realm", "module": "libraries/idb/normalise.mjs", "description": "Copy cross-realm plain records (IndexedDB values read in a Firefox WebDriver-imported module) into the checking realm, preserving values and key order." }
```

## Per-language guidance

- **Microtasks only.** A continuation after a request must run before the event loop gets control back. Otherwise the transaction has committed, and the next request throws InvalidStateError or TransactionInactiveError. Cases `request-after-error`, `cursor-many`, `done-after-complete` and the nested cases catch a port that gets this wrong. The technique per family:

  | Family | Use | Avoid |
  | --- | --- | --- |
  | Rust | `mires_js::await_promise` and `future_to_promise`, which poll from microtasks | `yield_task` (a MessageChannel task) |
  | Kotlin/JS | `kotlin.coroutines`: `startCoroutine` with `EmptyCoroutineContext`, resumed from a `Promise.then` callback through `suspendCoroutine`, or `Dispatchers.Unconfined` | the `kotlinx.coroutines` `Default` and `Main` dispatchers, which go through `setTimeout` or `postMessage` |
  | Scala.js | `js.Promise` callbacks, or `Future`s on `JSExecutionContext.queue`, which runs on Promise microtasks | a macrotask executor, `setTimeout` |
  | Dart | `async`/`await`, `Completer` and `.toDart`: dart2js schedules microtasks with a MutationObserver on `self.document`, which the facade leaves real in a page | `Timer`, `Future.delayed` |
  | GopherJS | callbacks on host Promises (continuation-passing) | a goroutine that blocks on a channel and is resumed from a callback. GopherJS 1.21.0's `$runScheduled` (`compiler/prelude/goroutines.js`) runs it synchronously, but yields to `setTimeout` once a pass exceeds 4 ms, so the outcome would depend on load |
  | ReScript, Melange | native `async`/`await` or `Promise.then` (`Js.Promise`): they are JS promises | `setTimeout` |
  | js_of_ocaml | `Promise` bindings, or Lwt promises woken from host callbacks (Lwt wakes synchronously) | `Lwt_js.yield`, `Lwt_js.sleep` |
  | Elm | synchronous port round trips (below) | `Process.sleep`, a `Task` that waits on a timer |
- **Eager requests.** A request is issued at call time, and a batch issues all before awaiting any (`batch-explicit-abort`). In Rust this means no lazy `async fn` that calls IndexedDB on first poll.
- **`done` at creation.** Create `done` when the transaction is created (`done-after-complete`, `inactive-after-done`; a lazy `done` hangs). Listen to the transaction's `error` as well as `abort` (`batch-constraint`, `keys-compound`).
- **Catch host exceptions.** Every IndexedDB call that can throw is caught as a value and propagated to the step. This is the B9 Rust leak: an exception that unwinds past cleanup leaves a database behind, which `verify.mjs` detects. In Rust, use `fallible` bindings; everywhere else, try/catch around the host call.
- **Keep host data as host data.** Keep keys, values, versions, counts and options as host values, or convert them without loss:
  - `null` is not absent, and absent is not `null` (`misses-and-null`, `cursor-only-null-uncaught`, `null-arguments`);
  - numbers past 2^32 and 2^53 stay exact (`open-version-wide`, `keys-numbers`, `getall-count-limits`, `cursor-bad-arguments`);
  - a value 1000 levels deep must not overflow the stack (`values-deep-wide`).
- **`undefined` in the output.** A port that cannot produce `undefined` never needs to: the output has none. It still needs to know when a result was `undefined`.

### Elm: the I/O boundary

All of idb's logic and the whole script interpreter live in Elm:

- the request-to-result mapping;
- `done` and its AbortError fallback;
- `tx.store`;
- the shortcuts and their error order;
- the iterator's interception and auto-continue;
- `openDB`'s callbacks and the time `blocking` is attached;
- sequencing, catching, the log, the dump and the cleanup.

The JS host only executes the primitives below and reports their results and events as messages. Host objects travel as opaque `Json.Value`s that Elm stores and sends back unchanged. Elm chooses an `id` for each command whose result it awaits, and every reply carries that id.

Every command that carries an `id` is answered synchronously, before the host returns:

- `value { id, result? }` when the host call returned;
- `threw { id, name }` when it threw.

A command whose effects come later (`open`, `deleteDatabase`, `listenVersionChange`, `transaction`, `request`, `step`) then sends them as further messages with the same id. So Elm knows at once whether a request, a move or a schema call went through. It can make the position's next action, the batch's next request or the next upgrade step in the same synchronous turn, and stop at a call that threw.

The host sends one `{ kind: "start" }` message after it subscribes to the outgoing port, and Elm begins the script on it, because a command sent from `init` would be lost. The comparator wrapper makes port round trips synchronous, so an Elm reaction to a `success`, `error`, `upgradeneeded`, `blocked` or `versionchange` message runs inside that event's dispatch, while the transaction is still active. A result that is `undefined` is sent without the `result` key.

| Command | Host call | Messages |
| --- | --- | --- |
| `open { id, name, version? }` | `indexedDB.open(name, version)`, plus listeners for `upgradeneeded`, `blocked`, `success` and `error` | `value` or `threw`; later `upgradeneeded { id, db, transaction, oldVersion, newVersion }`, `blocked { id, oldVersion, newVersion }`, `success { id, result }`, `error { id, name }` |
| `deleteDatabase { id, name }` | `indexedDB.deleteDatabase(name)`, plus listeners for `blocked`, `success` and `error` | the same, without `upgradeneeded` |
| `listenVersionChange { id, db }` | `db.addEventListener('versionchange', …)` | `value`; later `versionchange { id, oldVersion, newVersion }`, each time |
| `close { id, db }` | `db.close()` | `value` (Elm delivers the commands of one `Cmd.batch` in reverse order, so it orders a close before the next open or delete by awaiting this answer) |
| `transaction { id, db, stores, mode }` | `db.transaction(stores, mode)`, plus listeners for `complete`, `error` and `abort` | `value` or `threw`; later `complete { id }`, `error { id, txError }`, `abort { id, txError }`, where `txError` is `tx.error?.name ?? null` when the event fires |
| `call { id, target, method, args }` | `target[method](...args)` for `objectStore`, `index`, `createObjectStore`, `deleteObjectStore`, `createIndex`, `deleteIndex` and `abort` | `value` or `threw` |
| `request { id, target, method, args }` | `target[method](...args)` for `get`, `getKey`, `getAll`, `getAllKeys`, `count`, `put`, `add`, `delete`, `clear`, `openCursor`, `openKeyCursor` and `update`, and a cursor's `delete`; then one `success`/`error` pair on the returned request, removed after the first event | `value` or `threw`; later `success { id, result }` or `error { id, name }` |
| `step { id, cursor, method, args }` | `cursor[method](...args)` for `continue`, `advance` and `continuePrimaryKey`; then one `success`/`error` pair on `cursor.request` | as `request` |
| `read { id, target, property }` | `target[property]` for `version`, `objectStoreNames`, `name`, `keyPath`, `autoIncrement`, `indexNames`, `unique`, `multiEntry`, `key`, `primaryKey` and `value`; a DOMStringList becomes an array | `value` |
| `keyRange { id, factory, args }` | `IDBKeyRange[factory](...args)` for `only`, `lowerBound`, `upperBound` and `bound` | `value` or `threw` |
| `finish { output }` | resolves `run` with `output` | none |
| `fail { name }` | rejects `run` with an `Error` whose `name` is `name`: a failed dump or cleanup | none |

The host passes the corpus input and the database-name prefix (from `crypto.randomUUID()`) as flags. It runs no idb or script logic of its own. It creates no promise for Elm, decides no outcome and builds no part of the output.

`request` and `step` keep one rule of idb's `promisifyRequest` in the host: one listener pair per command, removed after the first event. That is I/O mechanics. Elm still maps the event to a result.

## Corpus

Case classes:

- lifecycle and versioning, 19 cases: upgrades, version errors, versions past 2^32, upgrade aborts, `blocking` and `blocked`, `deleteDB`, the dump order;
- transactions and requests, 24 cases: the store and index methods, null and key-range queries, null arguments, batches with ConstraintError, abort and synchronous failures, `tx.store`, `done`, nested transactions, failure scoping, error order;
- cursors, 15 cases;
- the async iterator, 7 cases;
- database shortcuts, 5 cases;
- `wrap`/`unwrap`, 2 cases;
- keys and values, 5 cases: compound, string and numeric keys, key generators, deep and wide values;
- the bench case.

Error paths that fail after databases were opened, uncaught so the script ends and cleanup runs: `uncaught-sync-failure`, `uncaught-request-failure`, `done-rejects-uncaught`, `cursor-advance-zero-uncaught`, `cursor-only-null-uncaught`, `upgrade-write-fails`, `upgrade-abort-new` and `upgrade-abort-existing`.

- `open-empty-script`: No calls: nothing to dump and nothing to clean up.
- `open-default-version`: openDB without a version creates version 1 and runs upgrade(0, 1).
- `open-without-upgrade`: A new database opened without an upgrade callback has no stores.
- `open-upgrade-sequence`: Version 1 then 2: upgrade(1, 2) adds a store, drops an index and writes in the upgrade transaction.
- `open-version-errors`: A lower version rejects with VersionError; 0, -1 and 2^53 throw TypeError synchronously.
- `open-version-wide`: Versions past 2^32 and up to 2^53 - 1, and a fraction, which IndexedDB truncates.
- `upgrade-errors`: Failing schema calls inside upgrade are logged and the upgrade continues.
- `upgrade-abort-new`: Aborting the first upgrade rejects the open with AbortError; a second open starts again at 0.
- `upgrade-abort-existing`: Aborting an upgrade of an existing database keeps version 1 and its data.
- `upgrade-write-fails`: A failing add in the upgrade transaction aborts the upgrade and rejects the open.
- `blocking-closes`: A connection whose blocking callback closes it lets the upgrade proceed; it is then unusable.
- `blocked-closes-other`: blocking only logs, so blocked fires; its callback closes the old connection.
- `blocked-no-blocking-callback`: Without a blocking callback the old connection gets no call; blocked closes it.
- `delete-blocked`: deleteDB with an open connection: blocking(1, null), then blocked(1), then the delete resolves.
- `delete-blocking-closes`: deleteDB with a connection that closes itself in blocking; the database is gone afterwards.
- `close-reopen`: Closing and reopening without a version keeps the schema and data; no upgrade runs.
- `two-databases`: Two databases and a deleted third: only live databases are dumped.
- `dump-order-reopen`: Databases are dumped in the order they became live: one deleted and opened again moves to the end.
- `blocking-close-and-blocked`: blocking closes the old connection at once, so the blocked callback is never called.
- `store-methods`: Each store method awaited in turn inside one readwrite transaction.
- `index-methods`: Index get, getKey, getAll, getAllKeys and count, with misses.
- `misses-and-null`: A miss resolves undefined, a stored null resolves null; null and range queries.
- `key-ranges`: only, lowerBound, upperBound and bound with open ends; an empty bound throws DataError.
- `getall-count-limits`: getAll counts: 0 means no limit; 2^32 and 2^32 + 1 throw TypeError (they do not wrap).
- `null-arguments`: An absent argument is undefined, never null: null as a key, a direction or a mode throws; continue(null) moves on as continue() does.
- `batch-constraint`: A duplicate add: ConstraintError, AbortError for the rest and for done.
- `batch-explicit-abort`: Requests queued then tx.abort(): every request and done reject with AbortError.
- `batch-sync-failure`: A put without a key path value throws DataError synchronously; the other requests still run.
- `batch-empty`: Empty transactions: one completes, one aborts.
- `uncaught-sync-failure`: An uncaught DataError ends the script; the committed write is dumped and cleanup runs.
- `uncaught-request-failure`: An uncaught ConstraintError ends the script; the transaction aborts.
- `request-after-error`: A request issued while a caught error is still being dispatched is accepted, then aborted with the transaction.
- `catch-scope`: A failure goes to the nearest enclosing step with catch: a transaction step absorbs it and skips the rest of its body.
- `readonly-write`: Writing in a readonly transaction throws ReadOnlyError.
- `transaction-errors`: Unknown store, empty scope and an invalid mode throw; a caught failure keeps the script going.
- `tx-store`: tx.store is the only store, or undefined when the transaction has several.
- `store-finished`: tx.store on a finished transaction: undefined with several stores, InvalidStateError with one.
- `batch-finished`: A batch on a finished transaction: the abort and every request throw InvalidStateError; done stays resolved.
- `inactive-after-done`: After tx.done resolves, tx.store throws InvalidStateError (the transaction has finished); done stays resolved.
- `done-after-complete`: A nested readwrite transaction waits for the outer one; the outer done is awaited after it completed.
- `nested-readonly`: Two readonly transactions run together; the inner one reads while the outer is still active.
- `abort-then-use`: Explicit abort after an awaited put: done rejects, a second abort throws InvalidStateError.
- `done-rejects-uncaught`: An uncaught tx.done rejection ends the script after the abort.
- `cursor-store`: Store cursor with continue, ascending and descending.
- `cursor-index`: Index cursor: key, primary key and value; unique directions skip duplicates.
- `cursor-advance`: advance(2), advance(1.5) which IndexedDB truncates to 1, and advance past the end.
- `cursor-continue-key`: continue(key) jumps forward; a key behind the cursor throws DataError.
- `cursor-continue-primary-key`: continuePrimaryKey on an index cursor; on a store cursor it throws InvalidAccessError.
- `cursor-update-delete`: cursor.update and cursor.delete in a readwrite walk.
- `cursor-key-only`: openKeyCursor yields keys without values; update on a key cursor throws InvalidStateError.
- `cursor-update-readonly`: cursor.update and delete in a readonly transaction throw ReadOnlyError.
- `cursor-stop`: A position without a move ends the walk; break ends it at once; a range with no match ends immediately.
- `cursor-double-move`: Two moves at one position: the second throws InvalidStateError.
- `cursor-bad-arguments`: An invalid direction and a non-key continue throw TypeError and DataError.
- `walk-throw-then-action`: A move that throws ends its position at once: the delete after it never runs, in a walk or in the iterator.
- `cursor-advance-zero-uncaught`: advance(0) throws TypeError; uncaught, it ends the script after the open.
- `cursor-only-null-uncaught`: A cursor over only(null) throws DataError before any request; uncaught, the script ends.
- `cursor-many`: A 3000-record index walk with continue and a store walk with advance(7).
- `iterate-auto-continue`: store.iterate with no moves continues automatically, in both directions.
- `iterate-moves`: Moves in the loop body are recorded and awaited by the iterator instead of continue().
- `iterate-update-delete`: cursor.update and cursor.delete inside the iterator body.
- `iterate-break`: Breaking out of the iterator; the transaction is still usable.
- `iterate-errors`: Two moves in one body, a bad range and an index iterator over an empty match.
- `iterate-move-then-update`: cursor.update after a move in the same body throws InvalidStateError; a break after a move ends the loop.
- `iterate-past-end`: An iterator whose first move passes the end, and an index iterator that jumps with continue(key).
- `shortcuts-read`: db.get, getKey, getAll, getAllKeys and count, and their FromIndex forms.
- `shortcuts-write`: db.put, add, delete and clear wait for tx.done; the request error is reported first.
- `shortcut-write-commits`: Close right after a shortcut write, then open version 2 with a blocked callback that is never called.
- `shortcut-errors`: A shortcut whose key range throws rejects before any transaction; a failed write rejects with its own error.
- `shortcut-inside-transaction`: A shortcut inside an open transaction runs after it; the outer transaction then finishes.
- `wrap-unwrap`: wrap(unwrap(source).method(...)) promisifies raw IndexedDB requests.
- `wrap-count`: wrap with a second argument, and with a key range on an index.
- `keys-compound`: Array key paths, array keys, a multiEntry index and a unique index violation.
- `keys-strings`: String keys order by UTF-16 code units: an astral character sorts before U+E000.
- `keys-numbers`: Numeric keys past 2^31, 2^32 and 2^53, tiny and negative values; numbers sort before strings and arrays.
- `auto-increment`: Key generators: out-of-line and in-line keys, an explicit key moves the generator.
- `values-deep-wide`: A value nested 1000 levels deep and a 5000-element array round-trip.
- `bench-transactions`: Benchmark: 64 writes, 64 reads, index queries, an index walk with advance, an iterator and two shortcuts.

## Bench

`bench-transactions` is a representative workload over the subset, on one store with one index:

- 64 `put`s in two readwrite batches;
- one readonly transaction that runs a batch of 65 `get`s (one a miss) and 3 index `getAll`s, an index walk with `advance(3)`, and a store iterator over a key range;
- two shortcuts, `count` and `getAllFromIndex`.

Its keys, groups and values are strings such as `k07`, `g3` and `v12`. `bench-variant.mjs` appends `.k` to every one of them, which keeps their order and so the work per call. Every key, value and query therefore differs per call.

## Verifying the corpus

`node verify.mjs <chrome|firefox> <reference|language>` runs every case and two bench variants, twice at once, in one page. It compares the outputs with `expected.json`, one compact line per case, which Chrome and Firefox produce identically. It also checks that the database inventory returns to its initial state. `node verify.mjs chrome reference --capture` regenerates the fixture. `node generate-corpus.mjs` regenerates `corpus.json`.
