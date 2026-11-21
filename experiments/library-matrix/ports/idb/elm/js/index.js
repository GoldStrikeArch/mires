// JS host of the Elm port. It executes the IndexedDB primitives of libraries/idb/CONTRACT.md
// ("Elm: the I/O boundary") and reports their results and events as messages; all of idb and of
// the reference adapter is Elm (src/Idb.elm, src/Session.elm).
//
// The program sends `{ op, id, ...fields }` through `command`. The host runs the primitive and
// answers synchronously, before it returns, with `{ kind: 'value', id, result? }` (no `result`
// key when the call gave undefined) or `{ kind: 'threw', id, name }`. A primitive with events
// (open, deleteDatabase, listenVersionChange, transaction, request, step) then sends them as
// `{ kind, id, ... }`, with the id of the command. Host objects travel in the messages as they
// are; the program sends them back unchanged. `finish` and `fail` settle the call.
//
// Each call has its own Elm program instance; the module keeps no state.
import Elm from 'elm-program'

const CALLS = new Set(['objectStore', 'index', 'createObjectStore', 'deleteObjectStore', 'createIndex', 'deleteIndex', 'abort'])
const REQUESTS = new Set(['get', 'getKey', 'getAll', 'getAllKeys', 'count', 'put', 'add', 'delete', 'clear', 'openCursor', 'openKeyCursor', 'update'])
const STEPS = new Set(['continue', 'advance', 'continuePrimaryKey'])
const PROPERTIES = new Set(['version', 'objectStoreNames', 'name', 'keyPath', 'autoIncrement', 'indexNames', 'unique', 'multiEntry', 'key', 'primaryKey', 'value'])
const KEY_RANGES = new Set(['only', 'lowerBound', 'upperBound', 'bound'])

/** A command the contract has no primitive for: a fault of the port, which rejects the call. */
class Refused extends Error {}

function allowed(names, name) {
  if (!names.has(name)) throw new Refused('no primitive for ' + name)
  return name
}

export function run(input) {
  return new Promise((resolve, reject) => {
    const app = Elm.Main.init({ flags: { input, uuid: crypto.randomUUID() } })
    const send = (message) => app.ports.receive.send(message)
    const withResult = (message, result) => (result === undefined ? message : { ...message, result })
    const versions = (kind, id) => (event) => send({ kind, id, oldVersion: event.oldVersion, newVersion: event.newVersion })

    // One success/error pair on a request, removed after the first event: promisifyRequest's
    // listener rule, kept here as I/O mechanics.
    function once(id, request) {
      const success = () => {
        remove()
        send(withResult({ kind: 'success', id }, request.result))
      }
      const error = () => {
        remove()
        send({ kind: 'error', id, name: request.error.name })
      }
      const remove = () => {
        request.removeEventListener('success', success)
        request.removeEventListener('error', error)
      }
      request.addEventListener('success', success)
      request.addEventListener('error', error)
    }

    const primitives = {
      open({ id, name, version }) {
        const request = indexedDB.open(name, version)
        request.addEventListener('upgradeneeded', (event) => send({
          kind: 'upgradeneeded', id, db: request.result, transaction: request.transaction, oldVersion: event.oldVersion, newVersion: event.newVersion,
        }))
        request.addEventListener('blocked', versions('blocked', id))
        request.addEventListener('success', () => send({ kind: 'success', id, result: request.result }))
        request.addEventListener('error', () => send({ kind: 'error', id, name: request.error.name }))
        return request
      },
      deleteDatabase({ id, name }) {
        const request = indexedDB.deleteDatabase(name)
        request.addEventListener('blocked', versions('blocked', id))
        request.addEventListener('success', () => send({ kind: 'success', id }))
        request.addEventListener('error', () => send({ kind: 'error', id, name: request.error.name }))
        return request
      },
      listenVersionChange({ id, db }) {
        db.addEventListener('versionchange', versions('versionchange', id))
      },
      close({ db }) {
        db.close()
      },
      transaction({ id, db, stores, mode }) {
        const tx = db.transaction(stores, mode)
        const txError = () => tx.error?.name ?? null
        tx.addEventListener('complete', () => send({ kind: 'complete', id }))
        tx.addEventListener('error', () => send({ kind: 'error', id, txError: txError() }))
        tx.addEventListener('abort', () => send({ kind: 'abort', id, txError: txError() }))
        return tx
      },
      call({ target, method, args }) {
        return target[allowed(CALLS, method)](...args)
      },
      request({ id, target, method, args }) {
        const request = target[allowed(REQUESTS, method)](...args)
        once(id, request)
        return request
      },
      step({ id, cursor, method, args }) {
        cursor[allowed(STEPS, method)](...args)
        once(id, cursor.request)
      },
      read({ target, property }) {
        const value = target[allowed(PROPERTIES, property)]
        return value instanceof DOMStringList ? Array.from(value) : value
      },
      keyRange({ factory, args }) {
        return IDBKeyRange[allowed(KEY_RANGES, factory)](...args)
      },
    }

    function execute(command) {
      const { op, id } = command
      if (op === 'finish' || op === 'fail') {
        app.ports.command.unsubscribe(execute)
        if (op === 'finish') resolve(command.output)
        else reject(Object.assign(new Error(command.name), { name: command.name }))
        return
      }
      if (!Object.hasOwn(primitives, op)) return fault(new Refused('no primitive ' + op))
      let result
      try {
        result = primitives[op](command)
      } catch (error) {
        if (error instanceof Refused) return fault(error)
        return send({ kind: 'threw', id, name: error.name })
      }
      send(withResult({ kind: 'value', id }, result))
    }

    function fault(error) {
      app.ports.command.unsubscribe(execute)
      reject(error)
    }

    app.ports.command.subscribe(execute)
    send({ kind: 'start' })
  })
}
