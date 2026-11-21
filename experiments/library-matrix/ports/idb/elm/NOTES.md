# idb 8.0.4: Elm port

This port is written by hand. It replaces the earlier port, whose JS host ran the whole workload of the old flattened contract. It is a faithful port of idb 8.0.4 (`build/index.js`), the subset in `libraries/idb/CONTRACT.md`, and of what `libraries/idb/reference.mjs` does with it, to Elm 0.19 (elm/core 1.0.5, elm/json 1.1.3). All of idb and of the reference adapter is Elm. The JS host executes the contract's IndexedDB primitives ("Elm: the I/O boundary") and nothing else.

| File | What it is |
| --- | --- |
| `src/Io.elm` | the `Io` type: a computation that calls host primitives (`Call`), waits for a promise (`Await`), settles one (`Settle`), listens to host events (`Listen`, `Unlisten`) and logs; with `andThen`, `map` and `sequence` |
| `src/Idb.elm` | idb on `Io`: `promisified` (`promisifyRequest`'s listeners), `wrap`, `unwrap`, `cacheDone` (`cacheDonePromiseForTransaction`), the wrapped `Database`, `Transaction`, `ObjectStore`, `Index` and `Cursor`, `store` (the `tx.store` trap), the cursor moves, `Shortcut` and `shortcut` (database-extras' `getMethod`), `Iterator` (the async iterator and its intercepted moves), `openDB` and `deleteDB` |
| `src/Script.elm` | the corpus input as Elm types, with `Json.Decode` |
| `src/Session.elm` | the reference adapter: the script interpreter, the dump and the cleanup |
| `src/Main.elm` | the program: ports, and the runtime that runs `Io` fibers against the host |
| `js/index.js` | the JS host: the primitives, and `run` |

## Boundary

The host passes `{ input, uuid }` as flags, where `uuid` comes from `crypto.randomUUID()`. Elm decodes the script with `Json.Decode`. Names become Elm strings. Keys, values, versions, counts, directions, modes, options and host objects stay `Json.Value`s, which Elm sends back unchanged; under `--optimize` they are the JS values themselves. A field that may be absent is a `Maybe`, and an absent argument is left out of the host call, so it is never null (`null-arguments`). Numbers are not narrowed, and a deep value is never walked. The output `{ log, databases }` is built with `Json.Encode` and sent with `finish`.

Commands are `{ op, id, ...fields }` on the `command` port. The host answers each command synchronously, before it returns, on `receive`: with `{ kind: 'value', id, result? }` (no `result` key for undefined) or `{ kind: 'threw', id, name }`. Events follow as `{ kind, id, ... }` with the command's id, exactly as the contract's table lists them. The host decides no outcome. It creates no promise for Elm and builds no part of the output; a DOMStringList becomes an array, as the table says.

The host differs from the table in three places:

- **`close` carries an id, and the host answers it.** Elm delivers the port commands of one `Cmd.batch` in reverse order: elm/core's `_Platform_insert` conses each one onto the manager's list. So the program never batches commands; each one goes out alone, from the update that handled the previous answer. A `close` that is not answered would leave nothing to continue from. The `close` step, the dump and the cleanup each close a connection and then open or delete a database, which must come after the close. The host's general rule, "every command that carries an id is answered synchronously", covers it. The answer is `value { id }`, since `db.close()` returns undefined.
- **A `start` message.** The host subscribes to `command` only after `Elm.Main.init` returns, so it sends `{ kind: 'start' }` once it has. Before that, a command from `init` would be lost.
- **The method lists are enforced.** Each primitive accepts only the methods, properties or factories the table lists. Any other is a fault of the port and rejects the call.

## The runtime

`Main.run` steps one `Io` fiber at a time:

- `Call` sends the command and waits for its synchronous answer, so at most one command is in flight.
- `Await` either continues at once with a settled promise or parks the fiber.
- `Settle` stores the outcome (the first one wins, as with a JS promise) and queues the parked fibers behind the running one, as promise reactions queue microtasks.
- A host event starts its listener as a new fiber.

The comparator wrapper makes a port round trip synchronous. So an update that handles a `success`, `error`, `upgradeneeded`, `blocked` or `versionchange` message runs inside that event's dispatch. So does everything it starts, up to the next wait, while IndexedDB keeps the transaction active (`request-after-error`, `cursor-many`, `done-after-complete`). There are no timers and no `Task`s.

Every effect constructor holds the rest of the computation as a function, so `andThen` handles one constructor at a time. A long run of log entries or settlements does not deepen the stack. A 3000-request batch logs 3000 outcomes in a row, which an eager `andThen` overflowed. `sequence` collects items that are done at once in its loop.

## The library layer

- **`promisified`** is idb's pair of request listeners. The first `success` settles the request's promise with the result, or the first `error` with the error's name, and then the listener goes. Each `request` or `step` command is a promise of its own: a cursor request fires once per move, so each move promisifies it again. The host keeps its own listener pair per command, removed after the first event, as the contract prescribes.
- **`cacheDone`** runs when `transaction` answers, so `done` exists from the moment the transaction is wrapped (`done-after-complete`). The first `complete`, `error` or `abort` settles it. A failure is the event's `txError` (`tx.error?.name ?? null`, read by the host when the event fires), or AbortError when that is null (`batch-constraint`).
- **`store`** reads `objectStoreNames` and tests the second name, as idb's trap does, before it calls `objectStore`.
- **Wrapped methods** are host primitives. A host exception comes back at once, as `Err`, before any promise exists. `request`, `openCursor` and `openKeyCursor` are idb's `wrap(func.apply(unwrap(this), args))`. Cursor moves are `step` commands, whose promise gives the cursor, or `Nothing` at the end.
- **`openDB`** sends `open` and handles its events: `upgradeneeded` runs `upgrade`, `blocked` runs `blocked`, and `success` and `error` are promisified. Once the open has succeeded, it sends `listenVersionChange` for `blocking`, before the caller resumes, as idb's `then` does. One handler per command id serves all the open's events.
- **`shortcut`** is getMethod: a transaction per call, `readwrite` for a write, `tx.store`, then the index for the `FromIndex` forms. idb awaits `Promise.all([request, tx.done])`. A request always settles before its transaction, so awaiting the request and then `done` resolves and fails as that `Promise.all` does.
- **`Iterator`** is the generator as a value: each `next` and each intercepted move gives the iterator to use afterwards. The first `next` opens `openCursor(...args)`; a later one waits for the move the body recorded (`advanced`, the generator's `advanceResults` entry) or calls `continue()`.

## Deviations from npm idb (no observable difference in the contract)

- **Opaque values, not Proxies.** Each wrapped object is an opaque host value with the subset's functions. There is no `transformCache`, so wrapper identity does not hold; the contract does not observe it.
- **The versionchange transaction has no `done`.** The `open` primitive reports no events of that transaction, and nothing in the subset awaits it.
- **An absent key-range open flag is sent as `false`.** IndexedDB reads an undefined one as false, and a JSON message cannot carry undefined in the middle of an argument list.

## Inputs outside the contract

An input that does not decode rejects the call with a TypeError at once, before any database is opened: an unknown step, request method, upgrade step, shortcut or cursor action, or a `call` whose `index` does not match its method's `FromIndex` form. The reference rejects the first four with a TypeError too, after it has run the steps before them. It calls a `call` with a mismatched `index` with that name as the first query argument, and logs an unknown cursor action as `[name, "error", "TypeError"]`, the error of calling `undefined`. A connection the script never opened, `store: null` in a transaction with several stores, and a transaction step outside a transaction are `OutOfContract` when the step runs. They are never logged, they pass every `catch`, the cleanup runs, and the call rejects with a TypeError, as the reference does.

## Module state

None in Elm; each call has its own program instance. The host keeps five constant sets of primitive names.

## Performance accommodations

None. A cursor position reads `key`, `primaryKey` and `value` with three `read` commands, since the host returns property values one at a time.
