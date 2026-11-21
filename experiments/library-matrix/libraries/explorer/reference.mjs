import { run as store } from '../idb/reference.mjs'
import { run as search } from '../minisearch/reference.mjs'
import { run as render } from '../marked/reference.mjs'
import { run as compress } from '../fflate/reference.mjs'
import { run as plot } from '../uplot/reference.mjs'

// One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
// transaction, then walk the store with a cursor in primary-key order.
function persist(rows) {
  return {
    steps: [
      { op: 'openDB', db: 'explorer', conn: 'explorer', version: 1, upgrade: [
        { op: 'createStore', name: 'rows', options: { keyPath: 'id' } },
        { op: 'createIndex', store: 'rows', name: 'byGroup', keyPath: 'group' }
      ], catch: false },
      { op: 'tx', conn: 'explorer', stores: 'rows', mode: 'readwrite', steps: [
        { op: 'batch', requests: rows.map((row) => ({ op: 'put', store: null, index: null, args: [row], catch: false })), abort: false, done: true }
      ], catch: false },
      { op: 'tx', conn: 'explorer', stores: 'rows', mode: 'readonly', steps: [
        { op: 'openCursor', store: null, index: null, args: [], positions: [], rest: [['continue']], catch: false }
      ], catch: false }
    ]
  }
}

// Read the records back from the database before indexing or archiving them.
export async function run({ records, query, chart }) {
  const rows = records.map((doc) => ({ id: doc.id, group: 'documents', value: JSON.stringify(doc) }))
  const database = await store(persist(rows))
  const stored = database.log.filter((entry) => entry[0] === 'cursor').map((entry) => entry[3].value)
  const documents = stored.map((text) => JSON.parse(text))
  const index = search({
    options: { fields: ['title', 'body'], storeFields: ['body'] },
    operations: [
      { op: 'addAll', documents },
      { op: 'search', query }
    ]
  })
  const hits = index.results[1].hits
  const html = render({ op: 'parse', markdown: hits[0]?.body ?? '' })
  const bytes = compress({ op: 'strToU8', text: stored.join('\n') })
  const archive = compress({ op: 'gzipSync', data: { bytes }, options: { level: 6, mtime: 0 } })
  return [hits.map((hit) => hit.id), html, Array.from(archive), bytes.length, plot(chart)]
}
