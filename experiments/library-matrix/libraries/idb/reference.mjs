import { openDB, deleteDB } from '../../node_modules/idb/build/index.js'

export async function run(input) {
  const name = 'mires-matrix-idb-' + crypto.randomUUID()
  let db
  try {
    db = await openDB(name, 1, {upgrade(database) {
      const store = database.createObjectStore('rows', {keyPath:'id'})
      store.createIndex('byGroup', 'group')
    }})
    const transactions = []
    for (const batch of input.batches) {
      const tx = db.transaction('rows', 'readwrite')
      const writes = batch.rows.map(row => tx.store[batch.add ? 'add' : 'put'](row))
      // All requests and the cached completion promise receive rejection handlers
      // before the browser dispatches any request or transaction events.
      const settled = Promise.allSettled([...writes, tx.done])
      if (batch.abort) tx.abort()
      const outcomes = await settled
      transactions.push(outcomes.map((outcome, i) => outcome.status === 'rejected'
        ? '!' + outcome.reason.name : i === writes.length ? 'done' : outcome.value))
    }
    const tx = db.transaction('rows')
    const store = tx.store
    const index = store.index('byGroup')
    const reads = Promise.all(input.keys.map(key => store.get(key)))
    const indexed = Promise.all(input.groups.map(group => index.get(group)))
    const source = input.index ? index : store
    const range = input.only === undefined ? null : IDBKeyRange.only(input.only)
    const cursorValues = []
    const done = tx.done
    let cursor = await source.openCursor(range, input.reverse ? 'prev' : 'next')
    while (cursor) {
      cursorValues.push([cursor.key, cursor.primaryKey, cursor.value.value])
      cursor = input.step === 1 ? await cursor.continue() : await cursor.advance(input.step)
    }
    const values = rows => rows.map(row => row === undefined ? null : row.value)
    const output = [transactions, values(await reads), values(await indexed), cursorValues]
    await done
    return output
  } finally {
    db?.close()
    await deleteDB(name)
  }
}
