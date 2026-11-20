import { run as store } from '../idb/reference.mjs'
import { run as search } from '../minisearch/reference.mjs'
import { run as render } from '../marked/reference.mjs'
import { run as compress } from '../fflate/reference.mjs'
import { run as plot } from '../uplot/reference.mjs'

// Read the records back from the database before indexing or archiving them.
export async function run({ records, query, chart }) {
  const rows = records.map((doc) => ({ id: doc.id, group: 'documents', value: JSON.stringify(doc) }))
  const database = await store({ batches: [{ rows }], keys: [], groups: [], step: 1 })
  const stored = database[3].map((cursor) => cursor[2])
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
