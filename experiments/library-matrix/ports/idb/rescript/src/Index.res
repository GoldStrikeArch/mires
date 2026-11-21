// Entry of the ReScript port: plays a case's script through the port's idb (`Idb`), as
// libraries/idb/reference.mjs drives npm idb, then dumps the databases it left, and closes and
// deletes every database it touched, also after a failure.
//
// The boundary: `Script.decode` reads the input once into ReScript types. Keys, values and options
// stay JSON values. The log is an array of JSON entries, and the result a record, which is the
// `{ log, databases }` object itself.

open IndexedDb

type output = {log: array<JSON.t>, databases: array<JSON.t>}

/** A failure that the step where it happened has already logged. */
exception Failed

/** An input outside the contract that only shows while the script runs: a connection no earlier
    step opened, `store: null` in a transaction with several stores, a transaction step outside a
    transaction. It is never logged; `run` rejects with the TypeError after cleanup, as the
    reference does. */
exception OutOfContract(JsError.t)

let outOfContract = message => throw(OutOfContract(JsError.TypeError.make(message)))

@val @scope("crypto") external randomUUID: unit => string = "randomUUID"

type session = {
  prefix: string,
  log: array<JSON.t>,
  connections: Map.t<string, database>,
  opened: array<database>,
  attempted: array<string>,
  mutable live: array<string>,
}

let remember = (names, name) =>
  if !(names->Array.includes(name)) {
    names->Array.push(name)
  }

let connection = (session, conn) =>
  switch session.connections->Map.get(conn) {
  | Some(db) => db
  | None => outOfContract("no connection named " ++ conn)
  }

let inside = tx =>
  switch tx {
  | Some(tx) => tx
  | None => outOfContract("a transaction step outside a transaction")
  }

// --- The log ---

let push = (session, entry) => session.log->Array.push(JSON.Array(entry))

/** `[label, "ok"]` for an undefined result, `[label, "ok", value]` otherwise. */
let ok = (session, label, value) =>
  session->push(
    switch value {
    | None => [JSON.String(label), String("ok")]
    | Some(value) => [String(label), String("ok"), value]
    },
  )

let error = (session, label, error) =>
  session->push([JSON.String(label), String("error"), String(error->DOMException.name)])

/** Logs a failure where it happens; the `Failed` that propagates says it is logged. */
let fail = (session, label, failure) => {
  session->error(label, failure)
  throw(Failed)
}

/** A synchronous call; a host exception is logged with `label` and ends the step. */
let checkNow = (session, label, call) =>
  switch call() {
  | value => value
  | exception JsExn(failure) => session->fail(label, failure)
  }

/** A call that returns a promise: its synchronous exception or its rejection is logged. */
let check = async (session, label, call) =>
  switch await call() {
  | value => value
  | exception JsExn(failure) => session->fail(label, failure)
  }

/** One call that is awaited, then logged. */
let attempt = async (session, label, call) => session->ok(label, await session->check(label, call))

let version = blocked =>
  switch blocked->Null.toOption {
  | Some(version) => JSON.Number(version)
  | None => JSON.Null
  }

let names = list => JSON.Array(list->Array.fromArrayLike->Array.map(name => JSON.String(name)))

// --- Sources and arguments ---

/** `tx.objectStore(store)`, or `tx.store`, then `.index(index)`. */
let source = (tx, {store, index}: Script.target) => {
  let store = switch store {
  | Some(name) => tx->Idb.Transaction.objectStore(name)
  | None =>
    switch tx->Idb.Transaction.store {
    | Some(store) => store
    | None => outOfContract("store: null in a transaction with several stores")
    }
  }
  switch index {
  | Some(name) => store->ObjectStore.index(name)->StoreIndex.asSource
  | None => store->ObjectStore.asSource
  }
}

/** Builds a key range written as an object: a DataError here is the step's failure. */
let hostArgs = (args: array<Script.argument>) =>
  args->Array.map(argument =>
    switch argument {
    | Value(value) => IndexedDb.value(value)
    | Range(Only(key)) => range(KeyRange.only(key))
    | Range(LowerBound(lower, open_)) => range(KeyRange.lowerBound(lower, open_))
    | Range(UpperBound(upper, open_)) => range(KeyRange.upperBound(upper, open_))
    | Range(Bound(lower, upper, lowerOpen, upperOpen)) =>
      range(KeyRange.bound(lower, upper, lowerOpen, upperOpen))
    }
  )

/** `source[method](...args)`: the source is resolved first, then the arguments are built. */
let request = (tx, {method, target, args}: Script.request) => {
  let source = source(tx, target)
  Idb.request(source, method, hostArgs(args))
}

/** A call's outcome, its rejection included, followed from now on as `Promise.allSettled` does. */
let settle = async call =>
  switch await call() {
  | value => Ok(value)
  | exception JsExn(failure) => Error(failure)
  }

// --- Upgrades ---

/** Runs inside the upgrade callback: synchronous calls, each logged; a failure does not stop it. */
let upgradeStep = (session, op: Script.upgrade, db, tx) => {
  let logged = (label, call) =>
    switch call() {
    | value => session->ok(label, value)
    | exception JsExn(failure) => session->error(label, failure)
    }
  switch op {
  | CreateStore({name, options}) =>
    logged("createStore", () => Some(JSON.String(db->Database.createObjectStore(name, options)->ObjectStore.name)))
  | DeleteStore(name) =>
    logged("deleteStore", () => {
      db->Database.deleteObjectStore(name)
      None
    })
  | CreateIndex({store, name, keyPath, options}) =>
    logged("createIndex", () => Some(
      JSON.String(tx->Idb.Transaction.objectStore(store)->ObjectStore.createIndex(name, keyPath, options)->StoreIndex.name),
    ))
  | DeleteIndex({store, name}) =>
    logged("deleteIndex", () => {
      tx->Idb.Transaction.objectStore(store)->ObjectStore.deleteIndex(name)
      None
    })
  | Abort =>
    logged("abort", () => {
      tx->Idb.Transaction.abort
      None
    })
  | Write({method, store, args}) =>
    // Issued, not awaited: a failed write aborts the upgrade, which rejects the open.
    switch Idb.request(tx->Idb.Transaction.objectStore(store)->ObjectStore.asSource, method, args->Array.map(value)) {
    | written => written->Promise.catch(_ => Promise.resolve(None))->Promise.ignore
    | exception JsExn(failure) => session->error(Idb.name(method), failure)
    }
  }
}

// --- Cursor walks ---

let position = (session, kind, cursor) =>
  session->push(
    switch kind {
    | Script.KeyCursor => [JSON.String("cursor"), cursor->Cursor.key, cursor->Cursor.primaryKey]
    | Cursor | Iterate => [String("cursor"), cursor->Cursor.key, cursor->Cursor.primaryKey, cursor->Cursor.value]
    },
  )

let actionsAt = ({positions, rest}: Script.walk, at) => positions->Array.get(at)->Option.getOr(rest)

let write = async (session, cursor, action: Script.action) =>
  switch action {
  | Update(value) => await session->attempt("update", () => Idb.Cursor.update(cursor, value))
  | _ => await session->attempt("delete", () => Idb.Cursor.delete(cursor))
  }

/** A position's actions in a walk, one at a time. The move to await next, or `None` when the walk
    stops here: no move, or a `break`. */
let rec walkActions = async (session, cursor, actions, moved) =>
  switch actions {
  | list{} => moved
  | list{Script.Break, ..._} => None
  | list{(Update(_) | Delete) as action, ...rest} =>
    await session->write(cursor, action)
    await session->walkActions(cursor, rest, moved)
  | list{Continue(key), ...rest} =>
    let next = session->checkNow("continue", () => Idb.Cursor.continue(cursor, key))
    await session->walkActions(cursor, rest, Some(("continue", next)))
  | list{Advance(count), ...rest} =>
    let next = session->checkNow("advance", () => Idb.Cursor.advance(cursor, count))
    await session->walkActions(cursor, rest, Some(("advance", next)))
  | list{ContinuePrimaryKey(key, primaryKey), ...rest} =>
    let next = session->checkNow("continuePrimaryKey", () => Idb.Cursor.continuePrimaryKey(cursor, key, primaryKey))
    await session->walkActions(cursor, rest, Some(("continuePrimaryKey", next)))
  }

/** A walk with the promise-returning moves: each position's move is awaited after its actions. */
let rec walkFrom = async (session, walk: Script.walk, cursor, at) =>
  switch cursor->Null.toOption {
  | None => session->push([JSON.String("end")])
  | Some(cursor) =>
    session->position(walk.kind, cursor)
    switch await session->walkActions(cursor, walk->actionsAt(at), None) {
    | None => ()
    | Some((label, next)) => await session->walkFrom(walk, await session->check(label, () => next), at + 1)
    }
  }

/** A position's actions in the iterator. A move only records the next step; `false` on `break`. */
let rec iterateActions = async (session, yielded: Idb.Iterate.yielded, actions) =>
  switch actions {
  | list{} => true
  | list{Script.Break, ..._} => false
  | list{(Update(_) | Delete) as action, ...rest} =>
    await session->write(yielded.cursor, action)
    await session->iterateActions(yielded, rest)
  | list{Continue(key), ...rest} =>
    session->checkNow("continue", () => yielded->Idb.Iterate.continue(key))
    await session->iterateActions(yielded, rest)
  | list{Advance(count), ...rest} =>
    session->checkNow("advance", () => yielded->Idb.Iterate.advance(count))
    await session->iterateActions(yielded, rest)
  | list{ContinuePrimaryKey(key, primaryKey), ...rest} =>
    session->checkNow("continuePrimaryKey", () => yielded->Idb.Iterate.continuePrimaryKey(key, primaryKey))
    await session->iterateActions(yielded, rest)
  }

/** The async iterator: a position without a move continues. */
let rec iterateFrom = async (session, walk: Script.walk, iteration, at) =>
  switch await session->check("iterate", () => Idb.Iterate.next(iteration)) {
  | None => session->push([JSON.String("end")])
  | Some(yielded) =>
    session->position(walk.kind, yielded.cursor)
    if await session->iterateActions(yielded, walk->actionsAt(at)) {
      await session->iterateFrom(walk, iteration, at + 1)
    }
  }

let cursorWalk = async (session, tx, walk: Script.walk) =>
  switch walk.kind {
  | Iterate =>
    let iteration = session->checkNow("iterate", () => {
      let source = source(tx, walk.target)
      Idb.Iterate.make(source, hostArgs(walk.args))
    })
    await session->iterateFrom(walk, iteration, 0)
  | Cursor | KeyCursor =>
    let label = walk.kind == KeyCursor ? "openKeyCursor" : "openCursor"
    let cursor = await session->check(label, () => {
      let source = source(tx, walk.target)
      let args = hostArgs(walk.args)
      walk.kind == KeyCursor ? Idb.openKeyCursor(source, args) : Idb.openCursor(source, args)
    })
    await session->walkFrom(walk, cursor, 0)
  }

// --- Batches ---

/** Issues every request before awaiting any, then logs each outcome in issue order. Never fails. */
let batch = async (session, tx: Idb.Transaction.t, requests: array<Script.request>, abort, done) => {
  let pending = requests->Array.map(step => (step.method, settle(() => request(tx, step))))
  if abort {
    switch session->ok("abort", session->checkNow("abort", () => {
      tx->Idb.Transaction.abort
      None
    })) {
    | () => ()
    | exception Failed => ()
    }
  }
  let finished = done ? Some(settle(() => tx.done)) : None
  for i in 0 to pending->Array.length - 1 {
    let (method, outcome) = pending->Array.getUnsafe(i)
    switch await outcome {
    | Ok(value) => session->ok(Idb.name(method), value)
    | Error(failure) => session->error(Idb.name(method), failure)
    }
  }
  switch finished {
  | Some(outcome) =>
    switch await outcome {
    | Ok() => session->ok("done", None)
    | Error(failure) => session->error("done", failure)
    }
  | None => ()
  }
}

// --- Steps ---

let rec perform = async (session, step: Script.step, tx) =>
  switch await session->execute(step.op, tx) {
  | () => ()
  | exception Failed if step.catches => ()
  }

and performAll = async (session, steps, tx) =>
  for i in 0 to steps->Array.length - 1 {
    await session->perform(steps->Array.getUnsafe(i), tx)
  }

and execute = async (session, op: Script.op, tx) =>
  switch op {
  | OpenDB(step) => await session->openDB(step)
  | Close(conn) =>
    session->connection(conn)->Database.close
    session->ok("close", None)
  | DeleteDB({db, blocked}) => await session->deleteDB(db, blocked)
  | Transaction({conn, stores, mode, steps}) =>
    let db = session->connection(conn)
    let tx = session->checkNow("tx", () => Idb.Database.transaction(db, stores, Some(mode)))
    await session->performAll(steps, Some(tx))
  | Shortcut({conn, label, method, store, index, args}) =>
    let db = session->connection(conn)
    await session->attempt(label, () => Idb.Database.shortcut(db, method, store, index, hostArgs(args)))
  | Request(step) => await session->attempt(Idb.name(step.method), () => request(inside(tx), step))
  | Batch({requests, abort, done}) => await session->batch(inside(tx), requests, abort, done)
  | Walk(walk) => await session->cursorWalk(inside(tx), walk)
  | AbortTransaction =>
    let tx = inside(tx)
    session->checkNow("abort", () => tx->Idb.Transaction.abort)
    session->ok("abort", None)
  | Done =>
    let tx = inside(tx)
    await session->check("done", () => tx.done)
    session->ok("done", None)
  | Store =>
    let tx = inside(tx)
    let store = session->checkNow("store", () => tx->Idb.Transaction.store)
    session->ok("store", store->Option.map(store => JSON.String(store->ObjectStore.name)))
  | Wrap({method, target, args}) =>
    let tx = inside(tx)
    // A wrapped store or index is the host object, so `unwrap(source)[method]` is the host method.
    await session->attempt("wrap", () => {
      let source = source(tx, target)
      Idb.wrap(Idb.call(source, method, hostArgs(args)))
    })
  }

and openDB = async (session, {db, conn, version: requested, upgrade: ops, blocking, blocked}: Script.openDB) => {
  session.attempted->remember(db)
  let closing = switch blocked {
  | Some(CloseOther(other)) => Some(session->connection(other))
  | Some(LogBlocked) | None => None
  }
  let upgrade = ops->Option.map(ops => (database, oldVersion, newVersion, tx) => {
    session->push([JSON.String("upgrade"), Number(oldVersion), version(newVersion)])
    ops->Array.forEach(op => session->upgradeStep(op, database, tx))
  })
  let blocked = blocked->Option.map(_ => (current, blocked, _) => {
    session->push([JSON.String("blocked"), Number(current), version(blocked)])
    closing->Option.forEach(Database.close)
  })
  let blocking = blocking->Option.map(blocking => (current, blocked, event: versionChangeEvent<database>) => {
    session->push([JSON.String("blocking"), String(conn), Number(current), version(blocked)])
    if blocking == Script.CloseSelf {
      event.target->Database.close
    }
  })
  let opened = await session->check("openDB", () =>
    Idb.openDB(session.prefix ++ db, requested, ~upgrade?, ~blocked?, ~blocking?)
  )
  session->ok("openDB", Some(JSON.Array([Number(opened->Database.version), names(opened->Database.objectStoreNames)])))
  session.connections->Map.set(conn, opened)
  session.opened->Array.push(opened)
  session.live->remember(db)
}

and deleteDB = async (session, db, blocked) => {
  session.attempted->remember(db)
  let closing = switch blocked {
  | Some(Script.CloseOther(other)) => Some(session->connection(other))
  | Some(LogBlocked) | None => None
  }
  let blocked = blocked->Option.map(_ => (current, _) => {
    session->push([JSON.String("blocked"), Number(current)])
    closing->Option.forEach(Database.close)
  })
  await session->check("deleteDB", () => Idb.deleteDB(session.prefix ++ db, ~blocked?))
  session->ok("deleteDB", None)
  session.live = session.live->Array.filter(name => name != db)
}

// --- Dump and cleanup ---

/** `[storeName, keyPath, autoIncrement, indexes, records]` for each store, in one readonly
    transaction. */
let stores = async db => {
  let names = db->Database.objectStoreNames->Array.fromArrayLike
  let stores = []
  if names->Array.length > 0 {
    let tx = Idb.Database.transaction(db, JSON.Array(names->Array.map(name => JSON.String(name))), None)
    for i in 0 to names->Array.length - 1 {
      let name = names->Array.getUnsafe(i)
      let store = tx->Idb.Transaction.objectStore(name)
      let indexes = store->ObjectStore.indexNames->Array.fromArrayLike->Array.map(indexName => {
        let index = store->ObjectStore.index(indexName)
        JSON.Array([String(indexName), index->StoreIndex.keyPath, Boolean(index->StoreIndex.unique), Boolean(index->StoreIndex.multiEntry)])
      })
      let keys = await Idb.wrap(store->ObjectStore.getAllKeys)
      let values = await Idb.wrap(store->ObjectStore.getAll)
      let records = keys->Array.mapWithIndex((key, i) => JSON.Array([key, values->Array.getUnsafe(i)]))
      stores->Array.push(
        JSON.Array([String(name), store->ObjectStore.keyPath, Boolean(store->ObjectStore.autoIncrement), Array(indexes), Array(records)]),
      )
    }
    await tx.done
  }
  stores
}

/** `[db, version, stores]`, read through a connection of its own, closed also after a failure. */
let dump = async (session, name) => {
  let db = await Idb.openDB(session.prefix ++ name, None)
  let stores = switch await stores(db) {
  | stores => stores
  | exception failure =>
    db->Database.close
    throw(failure)
  }
  db->Database.close
  JSON.Array([String(name), Number(db->Database.version), Array(stores)])
}

let play = async (session, script) => {
  // A logged failure ends the script there; anything else rejects the call.
  switch await session->performAll(script, None) {
  | () => ()
  | exception Failed => ()
  }
  let databases = []
  for i in 0 to session.live->Array.length - 1 {
    databases->Array.push(await session->dump(session.live->Array.getUnsafe(i)))
  }
  {log: session.log, databases}
}

let cleanup = async session => {
  session.opened->Array.forEach(Database.close)
  for i in 0 to session.attempted->Array.length - 1 {
    await Idb.deleteDB(session.prefix ++ session.attempted->Array.getUnsafe(i))
  }
}

let run = async input => {
  let script = Script.decode(input)
  let session = {
    prefix: "mires-matrix-idb-" ++ randomUUID() ++ "-",
    log: [],
    connections: Map.make(),
    opened: [],
    attempted: [],
    live: [],
  }
  let played = switch await session->play(script) {
  | output => Ok(output)
  | exception failure => Error(failure)
  }
  // As the reference's `finally`: cleanup always runs, and its own failure replaces the result.
  await session->cleanup
  switch played {
  | Ok(output) => output
  | Error(OutOfContract(error)) => JsError.throw(error)
  | Error(JsExn(error)) => JsExn.throw(error)
  | Error(failure) => throw(failure)
  }
}
