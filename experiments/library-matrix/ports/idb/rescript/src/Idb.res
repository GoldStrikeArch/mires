// idb 8.0.4 (build/index.js) for ReScript: IndexedDB requests become promises, a transaction gets
// a `done` promise, cursors move by returning a promise of the next position, and `iterate` walks a
// store or an index.
//
// idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back. A
// port has no Proxies, so a wrapped object is the host object itself, and this module holds the
// methods idb changes: each calls the host method and wraps its result as idb's `get` trap would.
// A transaction is the one exception: it carries its `done` promise. Properties idb forwards
// unchanged (`name`, `keyPath`, `version`, ...) are read with the bindings in `IndexedDb`.

open IndexedDb

/** idb's `promisifyRequest`: settled once, with the request's result on `success` or its error
    on `error`, after which both listeners are removed. A cursor's request fires once per move,
    so each move wraps the same request again. */
let wrap = (request: request<'result>): promise<'result> =>
  Promise.make((resolve, reject) => {
    let rec success = () => {
      resolve(request->Request.result)
      unlisten()
    }
    and error = () => {
      reject(request->Request.error)
      unlisten()
    }
    and unlisten = () => {
      request->Request.removeEventListener(#success, success)
      request->Request.removeEventListener(#error, error)
    }
    request->Request.addEventListener(#success, success)
    request->Request.addEventListener(#error, error)
  })

/** The request methods idb promisifies, and database-extras' `readMethods` and `writeMethods`. */
type method =
  | @as("get") Get
  | @as("getKey") GetKey
  | @as("getAll") GetAll
  | @as("getAllKeys") GetAllKeys
  | @as("count") Count
  | @as("put") Put
  | @as("add") Add
  | @as("delete") Delete
  | @as("clear") Clear

let name = (method: method) => (method :> string)

let isWrite = method =>
  switch method {
  | Put | Add | Delete | Clear => true
  | Get | GetKey | GetAll | GetAllKeys | Count => false
  }

/** The unwrapped host call: it throws now, and returns the raw request. */
let call = (source, method, args) =>
  switch method {
  | Get => source->Source.get(args)
  | GetKey => source->Source.getKey(args)
  | GetAll => source->Source.getAll(args)
  | GetAllKeys => source->Source.getAllKeys(args)
  | Count => source->Source.count(args)
  | Put => source->Source.put(args)
  | Add => source->Source.add(args)
  | Delete => source->Source.delete(args)
  | Clear => source->Source.clear(args)
  }

/** A wrapped request method: the request is issued now, and a host exception is thrown now. */
let request = (source, method, args) => wrap(call(source, method, args))

let openCursor = (source, args) => wrap(source->Source.openCursor(args))
let openKeyCursor = (source, args) => wrap(source->Source.openKeyCursor(args))

module Transaction = {
  type t = {raw: transaction, done: promise<unit>}

  /** idb's `cacheDonePromiseForTransaction`: settled by the first `complete`, `error` or `abort`.
      A request error bubbles to the transaction before the transaction records it, so `done`
      then rejects with a new AbortError. */
  let cacheDone = (tx: transaction): promise<unit> =>
    Promise.make((resolve, reject) => {
      let rec complete = () => {
        resolve()
        unlisten()
      }
      and error = () => {
        reject(
          switch tx->IndexedDb.Transaction.error->Null.toOption {
          | Some(error) => error
          | None => DOMException.make("AbortError", "AbortError")
          },
        )
        unlisten()
      }
      and unlisten = () => {
        tx->IndexedDb.Transaction.removeEventListener(#complete, complete)
        tx->IndexedDb.Transaction.removeEventListener(#error, error)
        tx->IndexedDb.Transaction.removeEventListener(#abort, error)
      }
      tx->IndexedDb.Transaction.addEventListener(#complete, complete)
      tx->IndexedDb.Transaction.addEventListener(#error, error)
      tx->IndexedDb.Transaction.addEventListener(#abort, error)
    })

  /** idb creates `done` when it first wraps a transaction, so `done` also settles for a caller
      that only asks after the transaction finished. */
  let wrap = raw => {raw, done: cacheDone(raw)}

  let objectStore = (tx, name) => tx.raw->IndexedDb.Transaction.objectStore(name)
  let abort = tx => tx.raw->IndexedDb.Transaction.abort

  /** idb's `tx.store` trap, `objectStoreNames[1] ? undefined : objectStore(objectStoreNames[0])`.
      Store names are sorted and distinct, so a second name is never the empty string, and the
      test is the name count. On a finished transaction with one store, `objectStore` throws
      InvalidStateError. */
  let store = tx => {
    let names = tx.raw->IndexedDb.Transaction.objectStoreNames->Array.fromArrayLike
    names->Array.length > 1 ? None : Some(tx->objectStore(names->Array.getUnsafe(0)))
  }
}

module Database = {
  let transaction = (db, stores, mode) => Transaction.wrap(db->IndexedDb.Database.transaction(stores, mode))

  /** The database shortcuts (`db.get(store, …)`, `db.getAllFromIndex(store, index, …)`, …),
      database-extras' `getMethod`: one transaction on `tx.store` (or its index) per call, then
      `Promise.all([request, isWrite && tx.done])`. So a write resolves only after `done`, the
      request's error comes first, and both promises are handled. The method is async, so a
      synchronous failure rejects the call. */
  let shortcut = async (db, method, store, index, args) => {
    let write = isWrite(method)
    let tx = transaction(db, JSON.String(store), Some(JSON.String(write ? "readwrite" : "readonly")))
    let target = tx->Transaction.store->Option.getOrThrow
    let source = switch index {
    | Some(name) => target->ObjectStore.index(name)->StoreIndex.asSource
    | None => target->ObjectStore.asSource
    }
    let (result, ()) = await Promise.all2((request(source, method, args), write ? tx.done : Promise.resolve()))
    result
  }
}

/** idb's cursor advance methods (`wrapFunction`): the host move, then the cursor's request wrapped
    again. The promise resolves with the same cursor, or with null past the end. */
module Cursor = {
  let advance = (cursor, count) => {
    cursor->IndexedDb.Cursor.advance(count)
    wrap(cursor->IndexedDb.Cursor.request)
  }

  let continue = (cursor, key) => {
    cursor->IndexedDb.Cursor.continue(key)
    wrap(cursor->IndexedDb.Cursor.request)
  }

  let continuePrimaryKey = (cursor, key, primaryKey) => {
    cursor->IndexedDb.Cursor.continuePrimaryKey(key, primaryKey)
    wrap(cursor->IndexedDb.Cursor.request)
  }

  let update = (cursor, value) => wrap(cursor->IndexedDb.Cursor.update(value))
  let delete = cursor => wrap(cursor->IndexedDb.Cursor.delete)
}

/** idb's `iterate`, the async iterator of a store or an index. The first `next` opens
    `openCursor(...args)`. Each later one awaits the move the loop body made on the yielded cursor,
    or calls `continue()` when the body made none. */
module Iterate = {
  type state = Unopened | At(cursor) | Finished

  type t = {
    source: source,
    args: array<argument>,
    mutable state: state,
    mutable advanced: option<promise<Null.t<cursor>>>,
  }

  /** The cursor an iteration yields, idb's proxied cursor: its moves are intercepted. `advance`,
      `continue` and `continuePrimaryKey` below call the host move and record its promise for the
      next `next`, as idb's `cursorIteratorTraps` do. */
  type yielded = {iteration: t, cursor: cursor}

  let make = (source, args) => {source, args, state: Unopened, advanced: None}

  let next = async iteration => {
    let state = iteration.state
    iteration.state = Finished
    let cursor = switch state {
    | Unopened => await openCursor(iteration.source, iteration.args)
    | At(cursor) =>
      let moved = switch iteration.advanced {
      | Some(moved) => moved
      | None => Cursor.continue(cursor, None)
      }
      let next = await moved
      iteration.advanced = None
      next
    | Finished => Null.null
    }
    cursor
    ->Null.toOption
    ->Option.map(cursor => {
      iteration.state = At(cursor)
      {iteration, cursor}
    })
  }

  let advance = ({iteration, cursor}, count) => iteration.advanced = Some(Cursor.advance(cursor, count))
  let continue = ({iteration, cursor}, key) => iteration.advanced = Some(Cursor.continue(cursor, key))
  let continuePrimaryKey = ({iteration, cursor}, key, primaryKey) =>
    iteration.advanced = Some(Cursor.continuePrimaryKey(cursor, key, primaryKey))
}

/** openDB's `upgrade(db, oldVersion, newVersion, transaction)`. */
type upgrade = (database, float, Null.t<float>, Transaction.t) => unit

/** openDB's `blocked` and `blocking`: `(currentVersion, blockedVersion, event)`. */
type versionChange<'target> = (float, Null.t<float>, versionChangeEvent<'target>) => unit

/** idb's `openDB(name, version, { upgrade, blocked, blocking })`. An absent version opens the
    current version, or version 1 for a new database. An invalid version throws now. `blocking` is
    attached to the connection once it is open. */
let openDB = (name, version, ~upgrade: option<upgrade>=?, ~blocked=?, ~blocking=?) => {
  let request = Factory.indexedDB->Factory.openDatabase(name, version)
  let openPromise = wrap(request)
  upgrade->Option.forEach(upgrade =>
    request->OpenRequest.onUpgradeNeeded(event =>
      upgrade(
        request->Request.result,
        event.oldVersion,
        event.newVersion,
        Transaction.wrap(request->OpenRequest.transaction),
      )
    )
  )
  blocked->Option.forEach(blocked =>
    request->OpenRequest.onBlocked(event => blocked(event.oldVersion, event.newVersion, event))
  )
  openPromise
  ->Promise.then(db => {
    blocking->Option.forEach(blocking =>
      db->IndexedDb.Database.onVersionChange(event => blocking(event.oldVersion, event.newVersion, event))
    )
    Promise.resolve()
  })
  ->Promise.catch(_ => Promise.resolve())
  ->Promise.ignore
  openPromise
}

/** idb's `deleteDB(name, { blocked })`: `blocked(currentVersion, event)`; resolves undefined. */
let deleteDB = (name, ~blocked=?) => {
  let request = Factory.indexedDB->Factory.deleteDatabase(name)
  blocked->Option.forEach(blocked =>
    request->DeleteRequest.onBlocked(event => blocked(event.oldVersion, event))
  )
  wrap(request)->Promise.thenResolve(() => ())
}
