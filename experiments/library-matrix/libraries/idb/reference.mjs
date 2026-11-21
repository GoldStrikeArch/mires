import { openDB, deleteDB, wrap, unwrap } from '../../node_modules/idb/build/index.js'

// A case is a script of idb calls (CONTRACT.md, "Input"). Each call works on databases of its own,
// named by a fresh prefix, and closes and deletes them before it settles, also after a failure.
export async function run(input) {
  const prefix = 'mires-matrix-idb-' + crypto.randomUUID() + '-'
  const log = []
  const connections = new Map()
  const opened = []
  const attempted = []
  const live = []
  const remember = (list, name) => { if (!list.includes(name)) list.push(name) }

  // Logs a failure where it happens; the Failed that propagates says it is logged. An input
  // outside the contract is never logged: it rejects the call after cleanup, as any other fault does.
  function failed(label, error) {
    if (error instanceof OutOfContract) return error
    log.push([label, 'error', error.name])
    return new Failed()
  }

  function connection(conn) {
    if (!connections.has(conn)) throw new OutOfContract('no connection named ' + conn)
    return connections.get(conn)
  }

  // One call: logs [label, 'ok'], [label, 'ok', shown value] or [label, 'error', name].
  async function attempt(label, action, show = (value) => value) {
    let value
    try {
      value = await action()
    } catch (error) {
      throw failed(label, error)
    }
    const shown = show(value)
    log.push(shown === undefined ? [label, 'ok'] : [label, 'ok', shown])
    return value
  }

  async function perform(step, tx) {
    try {
      await execute(step, tx)
    } catch (error) {
      if (!(error instanceof Failed) || !step.catch) throw error
    }
  }

  async function execute(step, tx) {
    switch (step.op) {
      case 'openDB': return open(step)
      case 'close': return attempt('close', () => connection(step.conn).close())
      case 'deleteDB': return remove(step)
      case 'tx': return transaction(step)
      case 'call': {
        if (!SHORTCUTS.has(step.method)) throw new OutOfContract('not a database shortcut: ' + step.method)
        const names = step.index === null ? [step.store] : [step.store, step.index]
        return attempt(step.method, () => connection(step.conn)[step.method](...names, ...args(step.method.replace(/FromIndex$/, ''), step.args)))
      }
      case 'batch': return batch(step, tx)
      case 'openCursor':
      case 'openKeyCursor': return walk(step, tx)
      case 'iterate': return iterate(step, tx)
      case 'abort': return attempt('abort', () => tx.abort())
      case 'done': return attempt('done', () => tx.done)
      case 'store': return attempt('store', () => tx.store?.name)
      case 'wrap':
        if (!REQUESTS.has(step.method)) throw new OutOfContract('not a request method: ' + step.method)
        return attempt('wrap', () => wrap(unwrap(source(tx, step))[step.method](...args(step.method, step.args))))
      default:
        if (!REQUESTS.has(step.op)) throw new OutOfContract('not a step: ' + step.op)
        return attempt(step.op, () => source(tx, step)[step.op](...args(step.op, step.args)))
    }
  }

  async function open(step) {
    for (const op of step.upgrade ?? []) if (!UPGRADES.has(op.op)) throw new OutOfContract('not an upgrade step: ' + op.op)
    remember(attempted, step.db)
    const closing = typeof step.blocked === 'object' ? connection(step.blocked.close) : null
    const callbacks = {}
    if (step.upgrade) {
      callbacks.upgrade = (db, oldVersion, newVersion, transaction) => {
        log.push(['upgrade', oldVersion, newVersion])
        for (const op of step.upgrade) upgrade(op, db, transaction)
      }
    }
    if (step.blocked) {
      callbacks.blocked = (currentVersion, blockedVersion) => {
        log.push(['blocked', currentVersion, blockedVersion])
        closing?.close()
      }
    }
    if (step.blocking) {
      callbacks.blocking = (currentVersion, blockedVersion, event) => {
        log.push(['blocking', step.conn, currentVersion, blockedVersion])
        if (step.blocking === 'close') event.target.close()
      }
    }
    const db = await attempt('openDB', () => openDB(prefix + step.db, step.version, callbacks),
      (db) => [db.version, [...db.objectStoreNames]])
    connections.set(step.conn, db)
    opened.push(db)
    remember(live, step.db)
  }

  async function remove(step) {
    remember(attempted, step.db)
    const closing = typeof step.blocked === 'object' ? connection(step.blocked.close) : null
    const options = step.blocked ? {
      blocked(currentVersion) {
        log.push(['blocked', currentVersion])
        closing?.close()
      },
    } : {}
    await attempt('deleteDB', () => deleteDB(prefix + step.db, options))
    if (live.includes(step.db)) live.splice(live.indexOf(step.db), 1)
  }

  // Runs synchronously inside the upgrade callback; a failure is logged and the callback goes on.
  function upgrade(op, db, transaction) {
    try {
      switch (op.op) {
        case 'createStore':
          log.push(['createStore', 'ok', db.createObjectStore(op.name, op.options).name])
          break
        case 'deleteStore':
          db.deleteObjectStore(op.name)
          log.push(['deleteStore', 'ok'])
          break
        case 'createIndex':
          log.push(['createIndex', 'ok', transaction.objectStore(op.store).createIndex(op.name, op.keyPath, op.options).name])
          break
        case 'deleteIndex':
          transaction.objectStore(op.store).deleteIndex(op.name)
          log.push(['deleteIndex', 'ok'])
          break
        case 'abort':
          transaction.abort()
          log.push(['abort', 'ok'])
          break
        default:
          // put or add: issued, not awaited. A failed write aborts the upgrade, which rejects the open.
          transaction.objectStore(op.store)[op.op](...op.args).catch(() => {})
      }
    } catch (error) {
      log.push([op.op, 'error', error.name])
    }
  }

  async function transaction(step) {
    let tx
    try {
      tx = connection(step.conn).transaction(step.stores, step.mode)
    } catch (error) {
      throw failed('tx', error)
    }
    for (const inner of step.steps) await perform(inner, tx)
  }

  // Issues every request before awaiting any, then logs each outcome in issue order.
  async function batch(step, tx) {
    for (const request of step.requests) if (!REQUESTS.has(request.op)) throw new OutOfContract('not a request method: ' + request.op)
    const pending = step.requests.map(async (request) => source(tx, request)[request.op](...args(request.op, request.args)))
    if (step.abort) await attempt('abort', () => tx.abort()).catch((error) => { if (!(error instanceof Failed)) throw error })
    const labels = step.requests.map((request) => request.op)
    if (step.done) {
      pending.push(tx.done)
      labels.push('done')
    }
    const outcomes = await Promise.allSettled(pending)
    for (const outcome of outcomes) if (outcome.reason instanceof OutOfContract) throw outcome.reason
    outcomes.forEach((outcome, i) => log.push(outcome.status === 'rejected' ? [labels[i], 'error', outcome.reason.name]
      : outcome.value === undefined ? [labels[i], 'ok'] : [labels[i], 'ok', outcome.value]))
  }

  // Moves with the promise-returning continue/advance/continuePrimaryKey; a position without a move ends the walk.
  async function walk(step, tx) {
    let cursor
    try {
      cursor = await source(tx, step)[step.op](...args(step.op, step.args))
    } catch (error) {
      throw failed(step.op, error)
    }
    for (let position = 0; cursor; position++) {
      log.push(step.op === 'openKeyCursor' ? ['cursor', cursor.key, cursor.primaryKey] : ['cursor', cursor.key, cursor.primaryKey, cursor.value])
      let moved = null
      for (const action of actions(step, position)) {
        if (action[0] === 'break') return
        moved = (await act(cursor, action)) ?? moved
      }
      if (!moved) return
      try {
        cursor = await moved.promise
      } catch (error) {
        throw failed(moved.label, error)
      }
    }
    log.push(['end'])
  }

  // The async iterator: a move only records the next step, and a position without one continues.
  async function iterate(step, tx) {
    let iterator
    try {
      iterator = source(tx, step).iterate(...args('iterate', step.args))
    } catch (error) {
      throw failed('iterate', error)
    }
    for (let position = 0; ; position++) {
      let next
      try {
        next = await iterator.next()
      } catch (error) {
        throw failed('iterate', error)
      }
      if (next.done) break
      const cursor = next.value
      log.push(['cursor', cursor.key, cursor.primaryKey, cursor.value])
      for (const action of actions(step, position)) {
        if (action[0] === 'break') return
        await act(cursor, action)
      }
    }
    log.push(['end'])
  }

  async function act(cursor, [name, ...values]) {
    switch (name) {
      case 'update':
        await attempt('update', () => cursor.update(values[0]))
        return null
      case 'delete':
        await attempt('delete', () => cursor.delete())
        return null
      default:
        try {
          return { label: name, promise: cursor[name](...values) }
        } catch (error) {
          throw failed(name, error)
        }
    }
  }

  async function dump(name) {
    const db = await openDB(prefix + name)
    try {
      const names = [...db.objectStoreNames]
      const stores = []
      if (names.length) {
        const tx = db.transaction(names)
        for (const storeName of names) {
          const store = tx.objectStore(storeName)
          const indexes = [...store.indexNames].map((indexName) => {
            const index = store.index(indexName)
            return [indexName, index.keyPath, index.unique, index.multiEntry]
          })
          const keys = await store.getAllKeys()
          const values = await store.getAll()
          stores.push([storeName, store.keyPath, store.autoIncrement, indexes, keys.map((key, i) => [key, values[i]])])
        }
        await tx.done
      }
      return [name, db.version, stores]
    } finally {
      db.close()
    }
  }

  try {
    try {
      for (const step of input.steps) await perform(step, null)
    } catch (error) {
      // A logged failure ends the script there; anything else rejects the call.
      if (!(error instanceof Failed)) throw error
    }
    const databases = []
    for (const name of live) databases.push(await dump(name))
    return { log, databases }
  } finally {
    for (const db of opened) db.close()
    for (const name of attempted) await deleteDB(prefix + name)
  }
}

// A failure the step where it happened has already logged.
class Failed {}
// An input outside the contract (CONTRACT.md, "Parity domain").
class OutOfContract extends TypeError {}

const READS = ['get', 'getKey', 'getAll', 'getAllKeys', 'count']
const REQUESTS = new Set([...READS, 'put', 'add', 'delete', 'clear'])
// database-extras' getMethod: every request method, and FromIndex forms of the read methods.
const SHORTCUTS = new Set([...REQUESTS, ...READS.map((method) => method + 'FromIndex')])
const UPGRADES = new Set(['createStore', 'deleteStore', 'createIndex', 'deleteIndex', 'put', 'add', 'abort'])
const QUERY_FIRST = new Set(['get', 'getKey', 'getAll', 'getAllKeys', 'count', 'delete', 'openCursor', 'openKeyCursor', 'iterate'])

function source(tx, step) {
  const store = step.store === null ? tx.store : tx.objectStore(step.store)
  if (store === undefined) throw new OutOfContract('store: null in a transaction with several stores')
  return step.index === null ? store : store.index(step.index)
}

function actions(step, position) {
  return position < step.positions.length ? step.positions[position] : step.rest
}

// The first argument of a query method is a key, or a key range written as an object.
function args(method, list) {
  return QUERY_FIRST.has(method) && list.length ? [query(list[0]), ...list.slice(1)] : list
}

function query(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return value
  if ('only' in value) return IDBKeyRange.only(value.only)
  if ('lower' in value && 'upper' in value) return IDBKeyRange.bound(value.lower, value.upper, value.lowerOpen, value.upperOpen)
  if ('lower' in value) return IDBKeyRange.lowerBound(value.lower, value.lowerOpen)
  return IDBKeyRange.upperBound(value.upper, value.upperOpen)
}
