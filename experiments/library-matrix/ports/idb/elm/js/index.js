import Elm from 'elm-program'

function request(req) {
  return new Promise((resolve,reject) => {
    const clear = () => { req.onsuccess = req.onerror = null }
    req.onsuccess = () => { clear(); resolve(req.result) }
    req.onerror = () => { clear(); reject(req.error) }
  })
}

function transactionDone(tx) {
  return new Promise((resolve,reject) => {
    const clear = () => { tx.oncomplete = tx.onabort = tx.onerror = null }
    tx.oncomplete = () => { clear(); resolve() }
    tx.onabort = tx.onerror = () => {
      const error = tx.error ?? new DOMException('AbortError','AbortError')
      clear()
      reject(error)
    }
  })
}

async function write(db,batch) {
  const tx = db.transaction('rows','readwrite')
  const done = transactionDone(tx)
  const store = tx.objectStore('rows')
  const writes = batch.rows.map(row => request(batch.add ? store.add(row) : store.put(row)))
  const settled = Promise.allSettled([...writes,done])
  if (batch.abort) tx.abort()
  return (await settled).map(r => r.status === 'fulfilled' ? r.value ?? 'done' : '!' + r.reason.name)
}

async function read(db,input) {
  const tx = db.transaction('rows')
  const done = transactionDone(tx)
  const store = tx.objectStore('rows')
  const index = store.index('byGroup')
  const reads = Promise.all(input.keys.map(key => request(store.get(key))))
  const indexed = Promise.all(input.groups.map(group => request(index.get(group))))
  const range = input.only === undefined ? null : IDBKeyRange.only(input.only)
  const req = (input.index ? index : store).openCursor(range,input.reverse ? 'prev' : 'next')
  const cursors = []
  for (let cursor = await request(req); cursor; cursor = await request(req)) {
    cursors.push([cursor.key,cursor.primaryKey,cursor.value.value])
    if (input.step === 1) cursor.continue()
    else cursor.advance(input.step)
  }
  const [a,b] = await Promise.all([reads,indexed,done])
  return [a.map(row => row?.value ?? null),b.map(row => row?.value ?? null),cursors]
}

export async function run(input) {
  const name = 'mires-matrix-idb-' + crypto.randomUUID()
  const app = Elm.Main.init({flags:input})
  let db, dispatch, finish
  try {
    return await new Promise((resolve,reject) => {
      finish = resolve
      dispatch = async ({op,data}) => {
        try {
          let value
          if (op === 'open') {
            const req = indexedDB.open(name,1)
            req.onupgradeneeded = () => req.result.createObjectStore('rows',{keyPath:'id'}).createIndex('byGroup','group')
            db = await request(req)
            value = null
          } else if (op === 'write') value = await write(db,data)
          else value = await read(db,data)
          app.ports.reply.send(value)
        } catch (error) { reject(error) }
      }
      app.ports.effect.subscribe(dispatch)
      app.ports.output.subscribe(finish)
      app.ports.start.send(null)
    })
  } finally {
    app.ports.effect.unsubscribe(dispatch)
    app.ports.output.unsubscribe(finish)
    db?.close()
    await request(indexedDB.deleteDatabase(name))
  }
}
