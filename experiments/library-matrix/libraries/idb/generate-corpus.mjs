// Writes corpus.json: scripts of idb calls (CONTRACT.md, "Input"). Every optional field the
// contract defines a default for is written out, except where a case tests its absence.
import { writeFileSync } from 'node:fs'

const cases = []
function add(id, note, steps) {
  cases.push({ id, note, input: { steps } })
}

// Top-level steps.
const open = (db, version, upgrade, extra = {}) => ({ op: 'openDB', db, conn: extra.conn ?? db, ...(version === undefined ? {} : { version }),
  ...(upgrade === undefined ? {} : { upgrade }), ...(extra.blocking ? { blocking: extra.blocking } : {}),
  ...(extra.blocked ? { blocked: extra.blocked } : {}), catch: extra.catch ?? false })
const close = (conn) => ({ op: 'close', conn, catch: false })
const drop = (db, extra = {}) => ({ op: 'deleteDB', db, ...(extra.blocked ? { blocked: extra.blocked } : {}), catch: extra.catch ?? false })
const tx = (conn, stores, mode, steps, extra = {}) => ({ op: 'tx', conn, stores, mode, steps, catch: extra.catch ?? false })
const call = (conn, method, store, index, args, extra = {}) => ({ op: 'call', conn, method, store, index, args, catch: extra.catch ?? false })

// Transaction steps. `on` names the target: null for tx.store, 'name' or ['store', 'index'].
const target = (on) => Array.isArray(on) ? { store: on[0], index: on[1] } : { store: on, index: null }
const request = (op, on, args, extra = {}) => ({ op, ...target(on), args, catch: extra.catch ?? false })
const get = (on, ...args) => request('get', on, args)
const put = (on, ...args) => request('put', on, args)
const addRow = (on, ...args) => request('add', on, args)
const caught = (step) => ({ ...step, catch: true })
const batch = (requests, extra = {}) => ({ op: 'batch', requests, abort: extra.abort ?? false, done: extra.done ?? false })
const walk = (op, on, args, positions, rest, extra = {}) => ({ op, ...target(on), args, positions, rest, catch: extra.catch ?? false })
const cursor = (on, args, positions = [], rest = [['continue']], extra) => walk('openCursor', on, args, positions, rest, extra)
const keyCursor = (on, args, positions = [], rest = [['continue']], extra) => walk('openKeyCursor', on, args, positions, rest, extra)
const iterate = (on, args, positions = [], rest = [], extra) => walk('iterate', on, args, positions, rest, extra)
const simple = (op, extra = {}) => ({ op, catch: extra.catch ?? false })
const raw = (on, method, args, extra = {}) => ({ op: 'wrap', ...target(on), method, args, catch: extra.catch ?? false })

// Upgrade steps.
const createStore = (name, options) => ({ op: 'createStore', name, ...(options === undefined ? {} : { options }) })
const deleteStore = (name) => ({ op: 'deleteStore', name })
const createIndex = (store, name, keyPath, options) => ({ op: 'createIndex', store, name, keyPath, ...(options === undefined ? {} : { options }) })
const deleteIndex = (store, name) => ({ op: 'deleteIndex', store, name })
const upgradeWrite = (op, store, ...args) => ({ op, store, args })

const row = (id, group, value) => ({ id, group, value })
const rows = [row(3, 'b', 'three'), row(1, 'a', 'one'), row(2, 'a', 'two'), row(-1, 'a', 'negative')]
const schema = [createStore('rows', { keyPath: 'id' }), createIndex('rows', 'byGroup', 'group')]
const main = (extra) => open('main', 1, schema, extra)
const fill = (records = rows) => tx('main', 'rows', 'readwrite', [batch(records.map((r) => put(null, r)), { done: true })])
const read = (steps) => tx('main', 'rows', 'readonly', steps)
const write = (steps) => tx('main', 'rows', 'readwrite', steps)

// --- Database lifecycle and versioning ---

add('open-empty-script', 'No calls: nothing to dump and nothing to clean up.', [])
add('open-default-version', 'openDB without a version creates version 1 and runs upgrade(0, 1).',
  [open('main', undefined, schema), read([simple('store')])])
add('open-without-upgrade', 'A new database opened without an upgrade callback has no stores.',
  [open('main', 2), open('main', undefined, undefined, { conn: 'again' })])
add('open-upgrade-sequence', 'Version 1 then 2: upgrade(1, 2) adds a store, drops an index and writes in the upgrade transaction.', [
  main(), fill(), close('main'),
  open('main', 2, [createStore('notes', { autoIncrement: true }), deleteIndex('rows', 'byGroup'),
    createIndex('rows', 'byValue', 'value', { unique: true }), upgradeWrite('put', 'notes', 'first note'),
    upgradeWrite('add', 'rows', row(9, 'z', 'nine'))]),
  read([request('getAll', ['rows', 'byValue'], [])]),
])
add('open-version-errors', 'A lower version rejects with VersionError; 0, -1 and 2^53 throw TypeError synchronously.', [
  open('main', 3, schema), close('main'),
  open('main', 2, schema, { catch: true, conn: 'low' }), open('main', 0, schema, { catch: true, conn: 'zero' }),
  open('main', -1, schema, { catch: true, conn: 'negative' }), open('main', 2 ** 53, schema, { catch: true, conn: 'huge' }),
  open('main', 3, undefined, { conn: 'same' }),
])
add('open-version-wide', 'Versions past 2^32 and up to 2^53 - 1, and a fraction, which IndexedDB truncates.', [
  open('wide', 2 ** 32 + 1, [createStore('s')]), close('wide'),
  open('wide', 2 ** 53 - 1, [deleteStore('s')]), close('wide'),
  open('fraction', 2.75, []),
])
add('upgrade-errors', 'Failing schema calls inside upgrade are logged and the upgrade continues.', [
  open('main', 1, [createStore('rows', { keyPath: 'id' }), createStore('rows'), createStore('auto', { keyPath: ['a', 'b'], autoIncrement: true }),
    createStore('empty-auto', { keyPath: '', autoIncrement: true }), createStore('bad-path', { keyPath: 'a b' }),
    createIndex('missing', 'x', 'x'), createIndex('rows', 'tags', ['a', 'b'], { multiEntry: true }),
    createIndex('rows', 'bad', 'not valid'), createIndex('rows', 'byGroup', 'group'), createIndex('rows', 'byGroup', 'other'),
    deleteStore('missing'), deleteIndex('rows', 'missing'), createStore('', { keyPath: 'id' })]),
  tx('main', ['', 'rows'], 'readonly', [simple('store')]),
])
add('upgrade-abort-new', 'Aborting the first upgrade rejects the open with AbortError; a second open starts again at 0.', [
  open('main', 1, [createStore('rows'), simple('abort')], { catch: true }),
  open('main', 1, schema),
])
add('upgrade-abort-existing', 'Aborting an upgrade of an existing database keeps version 1 and its data.', [
  main(), fill(), close('main'),
  open('main', 2, [deleteStore('rows'), createStore('other'), simple('abort')], { catch: true, conn: 'v2' }),
  open('main', undefined, undefined, { conn: 'again' }), tx('again', 'rows', 'readonly', [request('count', null, [])]),
])
add('upgrade-write-fails', 'A failing add in the upgrade transaction aborts the upgrade and rejects the open.', [
  open('main', 1, [createStore('rows', { keyPath: 'id' }), upgradeWrite('add', 'rows', row(1, 'a', 'x')),
    upgradeWrite('add', 'rows', row(1, 'a', 'y')), upgradeWrite('put', 'rows', { group: 'no key' })], { catch: true }),
])
add('blocking-closes', 'A connection whose blocking callback closes it lets the upgrade proceed; it is then unusable.', [
  main({ blocking: 'close' }), fill(),
  open('main', 2, [createStore('more')], { conn: 'next' }),
  tx('main', 'rows', 'readonly', [], { catch: true }),
  tx('next', 'rows', 'readonly', [request('count', null, [])]),
])
add('blocked-closes-other', 'blocking only logs, so blocked fires; its callback closes the old connection.', [
  main({ blocking: 'log' }),
  open('main', 2, [createStore('more')], { conn: 'next', blocked: { close: 'main' } }),
])
add('blocked-no-blocking-callback', 'Without a blocking callback the old connection gets no call; blocked closes it.', [
  main(), open('main', 5, [], { conn: 'next', blocked: { close: 'main' } }),
])
add('delete-blocked', 'deleteDB with an open connection: blocking(1, null), then blocked(1), then the delete resolves.', [
  main({ blocking: 'log' }), fill(), drop('main', { blocked: { close: 'main' } }),
])
add('delete-blocking-closes', 'deleteDB with a connection that closes itself in blocking; the database is gone afterwards.', [
  main({ blocking: 'close' }), fill(), drop('main', { blocked: 'log' }),
  open('main', undefined, [], { conn: 'fresh' }),
])
add('close-reopen', 'Closing and reopening without a version keeps the schema and data; no upgrade runs.', [
  main(), fill(), close('main'), close('main'), open('main', undefined, schema, { conn: 'again' }),
  tx('again', 'rows', 'readonly', [request('getAllKeys', null, [])]),
])
add('two-databases', 'Two databases and a deleted third: only live databases are dumped.', [
  main(), open('other', 4, [createStore('kv')]), tx('other', 'kv', 'readwrite', [put(null, 'value', 'key'), simple('done')]),
  open('gone', 1, [createStore('x')]), close('gone'), drop('gone'), fill(),
])

add('dump-order-reopen', 'Databases are dumped in the order they became live: one deleted and opened again moves to the end.', [
  open('a', 1, []), open('b', 1, []), close('a'), drop('a'), open('a', 1, []),
])
add('blocking-close-and-blocked', 'blocking closes the old connection at once, so the blocked callback is never called.', [
  main({ blocking: 'close' }), open('main', 2, [], { conn: 'next', blocked: { close: 'main' } }),
])

// --- Transactions and requests ---

add('store-methods', 'Each store method awaited in turn inside one readwrite transaction.', [
  main(), write([put(null, row(5, 'c', 'five')), addRow(null, row(6, 'c', 'six')), get(null, 5), request('getKey', null, [6]),
    request('getAll', null, []), request('getAllKeys', null, []), request('count', null, []), request('delete', null, [5]),
    request('count', null, []), get(null, 5), request('clear', null, []), request('count', null, []), simple('done')]),
])
add('index-methods', 'Index get, getKey, getAll, getAllKeys and count, with misses.', [
  main(), fill(), read([get(['rows', 'byGroup'], 'a'), request('getKey', ['rows', 'byGroup'], ['a']), get(['rows', 'byGroup'], 'missing'),
    request('getAll', ['rows', 'byGroup'], ['a']), request('getAllKeys', ['rows', 'byGroup'], ['a', 2]), request('count', ['rows', 'byGroup'], ['b']),
    request('getKey', ['rows', 'byGroup'], ['missing'])]),
])
add('misses-and-null', 'A miss resolves undefined, a stored null resolves null; null and range queries.', [
  open('main', 1, [createStore('kv')]),
  tx('main', 'kv', 'readwrite', [put(null, null, 1), put(null, 'one', 'x'), get(null, 1), get(null, 2), request('getKey', null, [2]),
    request('getAll', null, [null]), request('getAll', null, []), request('count', null, [null]), caught(get(null, null)),
    caught(request('delete', null, [null])), caught(request('getAll', null, [{ only: null }])),
    caught(request('count', null, [{ lower: null, upper: 5 }])), request('count', null, [{ upper: 5 }]),
    request('getAllKeys', null, [{ only: 1 }]), simple('done')]),
])
add('key-ranges', 'only, lowerBound, upperBound and bound with open ends; an empty bound throws DataError.', [
  main(), fill(), read([request('getAllKeys', null, [{ lower: 1 }]), request('getAllKeys', null, [{ lower: 1, lowerOpen: true }]),
    request('getAllKeys', null, [{ upper: 2, upperOpen: true }]), request('getAllKeys', null, [{ lower: -1, upper: 3, lowerOpen: true, upperOpen: true }]),
    request('getAllKeys', null, [{ lower: 2, upper: 2 }]), caught(request('getAllKeys', null, [{ lower: 3, upper: 1 }])),
    caught(request('getAllKeys', null, [{ lower: 2, upper: 2, lowerOpen: true }])), request('count', null, [{ only: 3 }]),
    request('getAll', ['rows', 'byGroup'], [{ lower: 'a', upper: 'b', upperOpen: true }])]),
])
add('getall-count-limits', 'getAll counts: 0 means no limit; 2^32 and 2^32 + 1 throw TypeError (they do not wrap).', [
  main(), fill(), read([request('getAll', null, [null, 2]), request('getAllKeys', null, [null, 0]), request('getAllKeys', null, [null, 1.9]),
    caught(request('getAllKeys', null, [null, 2 ** 32])), caught(request('getAllKeys', null, [null, 2 ** 32 + 1])),
    caught(request('getAll', null, [null, -1])), request('getAllKeys', ['rows', 'byGroup'], ['a', 2 ** 32 - 1])]),
])
add('null-arguments', 'An absent argument is undefined, never null: null as a key, a direction or a mode throws; continue(null) moves on as continue() does.', [
  open('main', 1, [createStore('kv')]),
  tx('main', 'kv', 'readwrite', [caught(put(null, 'value', null)), put(null, 'value', 1), put(null, 'other', 2), cursor(null, [null, null], [], [['continue']], { catch: true }),
    cursor(null, [], [[['continue', null]]], [], { catch: true }), simple('done')]),
  tx('main', 'kv', null, [simple('store')], { catch: true }),
])
add('batch-constraint', 'A duplicate add: ConstraintError, AbortError for the rest and for done.', [
  main(), fill([row(1, 'a', 'one')]),
  write([batch([addRow(null, row(3, 'b', 'three')), addRow(null, row(1, 'a', 'again')), put(null, row(2, 'a', 'two'))], { done: true })]),
  write([batch([addRow(null, row(-1, 'a', 'negative'))], { done: true })]),
])
add('batch-explicit-abort', 'Requests queued then tx.abort(): every request and done reject with AbortError.', [
  main(), fill([row(1, 'a', 'one')]),
  write([batch([put(null, row(1, 'x', 'overwritten')), put(null, row(4, 'x', 'four'))], { abort: true, done: true })]),
  read([request('getAll', null, [])]),
])
add('batch-sync-failure', 'A put without a key path value throws DataError synchronously; the other requests still run.', [
  main(), write([batch([put(null, row(1, 'a', 'one')), put(null, { group: 'g', value: 'no id' }), put(null, row(2, 'a', 'two')),
    put(null, row(3, 'a', 'three'), 3)], { done: true })]),
])
add('batch-empty', 'Empty transactions: one completes, one aborts.', [
  main(), write([batch([], { done: true })]), write([batch([], { abort: true, done: true })]),
])
add('uncaught-sync-failure', 'An uncaught DataError ends the script; the committed write is dumped and cleanup runs.', [
  main(), write([put(null, row(1, 'a', 'kept')), put(null, { group: 'g', value: 'no id' }), put(null, row(2, 'a', 'never'))]),
  fill(),
])
add('uncaught-request-failure', 'An uncaught ConstraintError ends the script; the transaction aborts.', [
  main(), fill([row(1, 'a', 'one')]), write([put(null, row(2, 'b', 'two')), addRow(null, row(1, 'a', 'dup')), put(null, row(3, 'c', 'never'))]),
])
add('request-after-error', 'A request issued while a caught error is still being dispatched is accepted, then aborted with the transaction.', [
  main(), fill([row(1, 'a', 'one')]), write([caught(addRow(null, row(1, 'a', 'dup'))), caught(put(null, row(5, 'e', 'five'))), caught(simple('done'))]),
  read([request('getAllKeys', null, [])]),
])
add('catch-scope', 'A failure goes to the nearest enclosing step with catch: a transaction step absorbs it and skips the rest of its body.', [
  main(), tx('main', 'rows', 'readonly', [get(null, null), request('count', null, [])], { catch: true }),
  read([request('count', null, [])]),
  write([put(null, row(1, 'a', 'one')), tx('main', 'rows', 'readonly', [get(null, null), get(null, 1)], { catch: true }),
    put(null, row(2, 'a', 'two')), simple('done')]),
])
add('readonly-write', 'Writing in a readonly transaction throws ReadOnlyError.', [
  main(), fill(), read([caught(put(null, row(7, 'a', 'seven'))), caught(request('clear', null, [])), request('count', null, [])]),
])
add('transaction-errors', 'Unknown store, empty scope and an invalid mode throw; a caught failure keeps the script going.', [
  main(), tx('main', 'missing', 'readonly', [], { catch: true }), tx('main', [], 'readonly', [], { catch: true }),
  tx('main', 'rows', 'versionchange', [], { catch: true }), read([caught(get('missing', 1)), caught(get(['rows', 'missing'], 1))]),
  tx('main', ['rows', 'rows'], 'readonly', [simple('store')]),
])
add('tx-store', 'tx.store is the only store, or undefined when the transaction has several.', [
  open('main', 1, [createStore('a'), createStore('b')]),
  tx('main', 'a', 'readwrite', [simple('store'), put(null, 1, 1)]), tx('main', ['b', 'a'], 'readonly', [simple('store'), get('a', 1)]),
  tx('main', ['b'], 'readonly', [simple('store')]),
])
add('store-finished', 'tx.store on a finished transaction: undefined with several stores, InvalidStateError with one.', [
  open('main', 1, [createStore('a'), createStore('b')]),
  tx('main', ['a', 'b'], 'readonly', [simple('done'), simple('store')]),
  tx('main', 'a', 'readonly', [simple('done'), caught(simple('store'))]),
])
add('batch-finished', 'A batch on a finished transaction: the abort and every request throw InvalidStateError; done stays resolved.', [
  main(), write([put(null, row(1, 'a', 'one')), simple('done'), batch([put(null, row(2, 'a', 'two')), get(null, 1)], { abort: true, done: true })]),
])
add('inactive-after-done', 'After tx.done resolves, tx.store throws InvalidStateError (the transaction has finished); done stays resolved.', [
  main(), write([put(null, row(1, 'a', 'one')), simple('done'), caught(get(null, 1)), simple('done'), caught(simple('abort'))]),
])
add('done-after-complete', 'A nested readwrite transaction waits for the outer one; the outer done is awaited after it completed.', [
  main(), write([put(null, row(1, 'a', 'outer')), write([put(null, row(2, 'a', 'inner')), simple('done')]), simple('done'),
    caught(get(null, 1))]),
])
add('nested-readonly', 'Two readonly transactions run together; the inner one reads while the outer is still active.', [
  main(), fill(), read([get(null, 1), read([get(null, 2), simple('done')]), simple('done')]),
])
add('abort-then-use', 'Explicit abort after an awaited put: done rejects, a second abort throws InvalidStateError.', [
  main(), fill([row(1, 'a', 'one')]), write([put(null, row(1, 'a', 'changed')), simple('abort'), caught(simple('done')), caught(simple('abort'))]),
  read([get(null, 1)]),
])
add('done-rejects-uncaught', 'An uncaught tx.done rejection ends the script after the abort.', [
  main(), write([put(null, row(1, 'a', 'one')), simple('abort'), simple('done'), put(null, row(2, 'a', 'never'))]),
])

// --- Cursors ---

add('cursor-store', 'Store cursor with continue, ascending and descending.', [
  main(), fill(), read([cursor(null, []), cursor(null, [null, 'prev'])]),
])
add('cursor-index', 'Index cursor: key, primary key and value; unique directions skip duplicates.', [
  main(), fill(), read([cursor(['rows', 'byGroup'], []), cursor(['rows', 'byGroup'], [null, 'prev']),
    cursor(['rows', 'byGroup'], [null, 'nextunique']), cursor(['rows', 'byGroup'], [null, 'prevunique'])]),
])
add('cursor-advance', 'advance(2), advance(1.5) which IndexedDB truncates to 1, and advance past the end.', [
  main(), fill(), read([cursor(null, [], [], [['advance', 2]]), cursor(null, [], [[['advance', 1.5]]], [['advance', 10]])]),
])
add('cursor-continue-key', 'continue(key) jumps forward; a key behind the cursor throws DataError.', [
  main(), fill(), read([cursor(null, [], [[['continue', 2]]], [['continue']]),
    cursor(null, [null, 'prev'], [[['continue', 0]]], [['continue']]), cursor(null, [], [[['continue', 5]]]),
    cursor(null, [], [[['continue']], [['continue', -5]]], [['continue']], { catch: true }), cursor(null, [], [[['continue', -5]]], [], { catch: true })]),
])
add('cursor-continue-primary-key', 'continuePrimaryKey on an index cursor; on a store cursor it throws InvalidAccessError.', [
  main(), fill([...rows, row(4, 'a', 'four'), row(0, 'b', 'zero')]),
  read([cursor(['rows', 'byGroup'], [], [[['continuePrimaryKey', 'a', 2]]], [['continue']]),
    cursor(['rows', 'byGroup'], [null, 'prev'], [[['continuePrimaryKey', 'a', 2]]], [['continue']]),
    cursor(['rows', 'byGroup'], [], [[['continuePrimaryKey', 'a', 100]]], [['continue']]),
    cursor(null, [], [[['continuePrimaryKey', 1, 1]]], [], { catch: true })]),
])
add('cursor-update-delete', 'cursor.update and cursor.delete in a readwrite walk.', [
  main(), fill(), write([cursor(null, [], [[['update', row(-1, 'a', 'NEGATIVE')], ['continue']], [['delete'], ['continue']]],
    [['update', row(0, 'z', 'key changed')], ['continue']], { catch: true }), cursor(['rows', 'byGroup'], ['a'], [], [['delete'], ['continue']]), simple('done')]),
])
add('cursor-key-only', 'openKeyCursor yields keys without values; update on a key cursor throws InvalidStateError.', [
  main(), fill(), read([keyCursor(null, [null, 'prev']), keyCursor(['rows', 'byGroup'], ['a'])]),
  write([keyCursor(null, [], [[['update', row(1, 'a', 'x')]]], [], { catch: true })]),
])
add('cursor-update-readonly', 'cursor.update and delete in a readonly transaction throw ReadOnlyError.', [
  main(), fill(), read([cursor(null, [], [[['update', row(-1, 'a', 'x')]]], [], { catch: true }), cursor(null, [], [[['delete']]], [], { catch: true })]),
])
add('cursor-stop', 'A position without a move ends the walk; break ends it at once; a range with no match ends immediately.', [
  main(), fill(), read([cursor(null, [], [[['continue']], []]), cursor(null, [], [[['continue']], [['break'], ['continue']]]),
    cursor(null, [{ only: 42 }]), cursor(['rows', 'byGroup'], ['missing'])]),
])
add('cursor-double-move', 'Two moves at one position: the second throws InvalidStateError.', [
  main(), fill(), read([cursor(null, [], [[['continue'], ['advance', 1]]], [], { catch: true }), request('count', null, [])]),
])
add('cursor-bad-arguments', 'An invalid direction and a non-key continue throw TypeError and DataError.', [
  main(), fill(), read([caught(cursor(null, [null, 'sideways'])), cursor(null, [], [[['continue', { a: 1 }]]], [], { catch: true }),
    cursor(null, [], [[['advance', -1]]], [], { catch: true }), cursor(null, [], [[['advance', 2 ** 32 + 1]]], [], { catch: true })]),
])
add('walk-throw-then-action', 'A move that throws ends its position at once: the delete after it never runs, in a walk or in the iterator.', [
  main(), fill(), write([cursor(null, [], [[['continue', -5], ['delete']]], [], { catch: true }),
    iterate(null, [], [[['advance', 0], ['delete']]], [], { catch: true }), request('count', null, []), simple('done')]),
])
add('cursor-advance-zero-uncaught', 'advance(0) throws TypeError; uncaught, it ends the script after the open.', [
  main(), fill(), read([cursor(null, [], [], [['advance', 0]])]), fill([row(9, 'z', 'never')]),
])
add('cursor-only-null-uncaught', 'A cursor over only(null) throws DataError before any request; uncaught, the script ends.', [
  main(), fill([row(1, 'g', 'v1')]), read([cursor(null, [{ only: null }])]), fill([row(9, 'z', 'never')]),
])
add('cursor-many', 'A 3000-record index walk with continue and a store walk with advance(7).', (() => {
  const many = Array.from({ length: 3000 }, (_, i) => row(i, 'g' + (i % 5), 'v' + i))
  return [main(), fill(many), read([cursor(['rows', 'byGroup'], [{ lower: 'g3' }]), keyCursor(null, [], [], [['advance', 7]])])]
})())

// --- Async iteration ---

add('iterate-auto-continue', 'store.iterate with no moves continues automatically, in both directions.', [
  main(), fill(), read([iterate(null, []), iterate(null, [null, 'prev'])]),
])
add('iterate-moves', 'Moves in the loop body are recorded and awaited by the iterator instead of continue().', [
  main(), fill([...rows, row(4, 'b', 'four'), row(5, 'c', 'five')]),
  read([iterate(null, [], [[['advance', 2]], [['continue', 4]]]), iterate(['rows', 'byGroup'], [], [[['continuePrimaryKey', 'a', 2]]]),
    iterate(['rows', 'byGroup'], [{ lower: 'a', upper: 'b' }, 'prevunique'])]),
])
add('iterate-update-delete', 'cursor.update and cursor.delete inside the iterator body.', [
  main(), fill(), write([iterate(null, [], [[['update', row(-1, 'a', 'updated')]], [['delete']]]), simple('done')]),
])
add('iterate-break', 'Breaking out of the iterator; the transaction is still usable.', [
  main(), fill(), read([iterate(null, [], [[], [['break']]]), request('count', null, [])]),
])
add('iterate-errors', 'Two moves in one body, a bad range and an index iterator over an empty match.', [
  main(), fill(), read([iterate(null, [], [[['advance', 1], ['continue']]], [], { catch: true }),
    iterate(null, [{ only: null }], [], [], { catch: true }), iterate(['rows', 'byGroup'], ['missing']), iterate(['rows', 'missing'], [], [], [], { catch: true })]),
])
add('iterate-move-then-update', 'cursor.update after a move in the same body throws InvalidStateError; a break after a move ends the loop.', [
  main(), fill(), write([iterate(null, [], [[['continue'], ['update', row(1, 'a', 'x')]]], [], { catch: true }),
    iterate(null, [], [[['advance', 1], ['break']]]), request('count', null, []), simple('done')]),
])
add('iterate-past-end', 'An iterator whose first move passes the end, and an index iterator that jumps with continue(key).', [
  main(), fill(), read([iterate(null, [], [[['advance', 10]]]), iterate(['rows', 'byGroup'], [], [[['continue', 'b']]])]),
])

// --- Database shortcuts ---

add('shortcuts-read', 'db.get, getKey, getAll, getAllKeys and count, and their FromIndex forms.', [
  main(), fill(), call('main', 'get', 'rows', null, [1]), call('main', 'get', 'rows', null, [99]), call('main', 'getKey', 'rows', null, [{ lower: 2 }]),
  call('main', 'getAll', 'rows', null, [null, 2]), call('main', 'getAllKeys', 'rows', null, []), call('main', 'count', 'rows', null, []),
  call('main', 'getFromIndex', 'rows', 'byGroup', ['a']), call('main', 'getKeyFromIndex', 'rows', 'byGroup', ['b']),
  call('main', 'getAllFromIndex', 'rows', 'byGroup', ['a', 2]), call('main', 'getAllKeysFromIndex', 'rows', 'byGroup', ['a']),
  call('main', 'countFromIndex', 'rows', 'byGroup', ['a']), call('main', 'getFromIndex', 'rows', 'byGroup', ['none']),
])
add('shortcuts-write', 'db.put, add, delete and clear wait for tx.done; the request error is reported first.', [
  main(), call('main', 'put', 'rows', null, [row(1, 'a', 'one')]), call('main', 'add', 'rows', null, [row(2, 'a', 'two')]),
  call('main', 'add', 'rows', null, [row(1, 'a', 'dup')], { catch: true }), call('main', 'delete', 'rows', null, [2]),
  call('main', 'delete', 'rows', null, [{ lower: 5 }]), call('main', 'count', 'rows', null, []),
  call('main', 'put', 'missing', null, [row(1, 'a', 'x')], { catch: true }), call('main', 'getFromIndex', 'rows', 'missing', ['a'], { catch: true }),
  call('main', 'put', 'rows', null, [{ value: 'no id' }], { catch: true }), call('main', 'get', 'rows', null, [null], { catch: true }),
  call('main', 'clear', 'rows', null, []), call('main', 'count', 'rows', null, []),
])
add('shortcut-write-commits', 'Close right after a shortcut write, then open version 2 with a blocked callback that is never called.', [
  main(), call('main', 'put', 'rows', null, [row(1, 'a', 'one')]), close('main'),
  open('main', 2, [], { conn: 'next', blocked: 'log' }), tx('next', 'rows', 'readonly', [request('count', null, [])]),
])
add('shortcut-errors', 'A shortcut whose key range throws rejects before any transaction; a failed write rejects with its own error.', [
  main(), fill(), call('main', 'count', 'rows', null, [{ only: null }], { catch: true }), call('main', 'add', 'rows', null, [row(1, 'a', 'dup')], { catch: true }),
  call('main', 'count', 'rows', null, []),
])
add('shortcut-inside-transaction', 'A shortcut inside an open transaction runs after it; the outer transaction then finishes.', [
  main(), fill(), write([put(null, row(8, 'h', 'eight')), call('main', 'count', 'rows', null, []), caught(get(null, 8))]),
])

// --- wrap and unwrap ---

add('wrap-unwrap', 'wrap(unwrap(source).method(...)) promisifies raw IndexedDB requests.', [
  main(), fill(), write([raw(null, 'count', []), raw(['rows', 'byGroup'], 'getAll', ['a']), raw(null, 'get', [99]),
    raw(null, 'put', [row(10, 'w', 'wrapped')]), raw(null, 'add', [row(1, 'a', 'dup')], { catch: true })]),
])
add('wrap-count', 'wrap with a second argument, and with a key range on an index.', [
  main(), fill(), read([raw(null, 'getAllKeys', [null, 2]), raw(['rows', 'byGroup'], 'count', [{ only: 'a' }])]),
])

// --- Keys and values ---

add('keys-compound', 'Array key paths, array keys, a multiEntry index and a unique index violation.', [
  open('main', 1, [createStore('pairs', { keyPath: ['group', 'id'] }), createIndex('pairs', 'tags', 'tags', { multiEntry: true }),
    createIndex('pairs', 'code', 'code', { unique: true })]),
  tx('main', 'pairs', 'readwrite', [batch([put(null, { group: 'b', id: 1, tags: ['x', 'y', 'x'], code: 1 }), put(null, { group: 'a', id: 2, tags: ['y'], code: 2 }),
    put(null, { group: 'a', id: [1, 'x'], tags: [], code: 3 })], { done: true })]),
  tx('main', 'pairs', 'readwrite', [batch([put(null, { group: 'd', id: 4, tags: 'z', code: 4 }), put(null, { group: 'c', id: 3, tags: 'z', code: 1 })], { done: true })]),
  tx('main', 'pairs', 'readonly', [get(null, ['a', 2]), request('getAllKeys', ['pairs', 'tags'], ['y']), keyCursor(['pairs', 'tags'], []),
    request('getAllKeys', null, [{ lower: ['a'], upper: ['b'] }])]),
])
add('keys-strings', 'String keys order by UTF-16 code units: an astral character sorts before U+E000.', [
  open('main', 1, [createStore('text'), createIndex('text', 'byWord', 'word')]),
  tx('main', 'text', 'readwrite', [batch(['', '😀', '', 'é', 'e', 'Z', 'a', ' ', '￿', 'ab', 'a\u0000']
    .map((key, i) => put(null, { word: key, i }, key)), { done: true })]),
  tx('main', 'text', 'readonly', [request('getAllKeys', null, []), request('getAllKeys', ['text', 'byWord'], [{ lower: '\uD83D\uDE00' }])]),
])
add('keys-numbers', 'Numeric keys past 2^31, 2^32 and 2^53, tiny and negative values; numbers sort before strings and arrays.', [
  open('main', 1, [createStore('n')]),
  tx('main', 'n', 'readwrite', [batch([2 ** 31, 2 ** 32 + 1, 2 ** 53 + 2, -(2 ** 53), 1e300, -1e-300, 5e-324, 0.1, -0.5, 'text', [0]]
    .map((key) => put(null, typeof key, key)), { done: true })]),
  tx('main', 'n', 'readonly', [request('getAllKeys', null, []), get(null, 2 ** 53 + 2), request('count', null, [{ lower: 2 ** 32 }])]),
])
add('auto-increment', 'Key generators: out-of-line and in-line keys, an explicit key moves the generator.', [
  open('main', 1, [createStore('log', { autoIncrement: true }), createStore('items', { keyPath: 'id', autoIncrement: true })]),
  tx('main', ['log', 'items'], 'readwrite', [put('log', 'first'), put('log', 'second'), put('log', 'tenth', 10), addRow('log', 'eleventh'),
    put('items', { name: 'a' }), put('items', { id: 7, name: 'b' }), put('items', { name: 'c' }), caught(put('items', { name: 'd' }, 3)), simple('done')]),
])
add('values-deep-wide', 'A value nested 1000 levels deep and a 5000-element array round-trip.', (() => {
  let deep = 'leaf'
  for (let i = 0; i < 1000; i++) deep = { d: deep, i }
  const wide = Array.from({ length: 5000 }, (_, i) => (i % 3 === 0 ? i : i % 3 === 1 ? 'x' + i : [i]))
  return [open('main', 1, [createStore('values')]),
    tx('main', 'values', 'readwrite', [put(null, deep, 'deep'), put(null, wide, 'wide'), request('getAllKeys', null, []), simple('done')])]
})())

// --- Benchmark ---

// Keys, groups and values carry k/g/v prefixes so bench-variant.mjs can rename them per call.
const benchRows = Array.from({ length: 64 }, (_, i) => row('k' + String((i * 29) % 64).padStart(2, '0'), 'g' + (i % 7), 'v' + String(i).padStart(2, '0')))
add('bench-transactions', 'Benchmark: 64 writes, 64 reads, index queries, an index walk with advance, an iterator and two shortcuts.', [
  main(),
  tx('main', 'rows', 'readwrite', [batch(benchRows.slice(0, 32).map((r) => put(null, r)), { done: true })]),
  tx('main', 'rows', 'readwrite', [batch(benchRows.slice(32).map((r) => put(null, r)), { done: true })]),
  read([batch([...benchRows.map((r) => get(null, r.id)), get(null, 'k99'), ...['g0', 'g3', 'g6'].map((g) => request('getAll', ['rows', 'byGroup'], [g]))], { done: false }),
    cursor(['rows', 'byGroup'], [], [], [['advance', 3]]), iterate(null, [{ lower: 'k16', upper: 'k47' }]), simple('done')]),
  call('main', 'count', 'rows', null, []), call('main', 'getAllFromIndex', 'rows', 'byGroup', ['g2']),
])

writeFileSync(new URL('corpus.json', import.meta.url), JSON.stringify({ schema: 1, library: 'idb', cases }) + '\n')
