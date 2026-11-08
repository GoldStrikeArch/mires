// JavaScript reference for the MiniSearch-shaped workload: the published `minisearch` library
// indexing the same `id|title|body` rows and answering the same query the Rust build answers.
// The import is a relative path to the package's published ESM entry (its `exports` "." import
// target), loaded the same way by Node and browsers.
// Like the Rust entry (src/minisearch.rs `search_packed`), each call parses the packed rows, builds
// a fresh index, runs the query and serialises `id:score` pairs.
//
// Configuration chosen to mirror the Rust port's index, not its scoring:
// - fields `title` and `body`, title boosted 3x (Rust weights title 3, body 1);
// - terms of one character are dropped and terms are lower-cased (Rust `tokenize` keeps only
//   terms longer than one byte and lower-cases ASCII);
// - the query is split on ASCII whitespace and a trailing `*` makes that term a prefix query, OR-ed
//   together (Rust `search_packed`).
// MiniSearch scores with BM25+ (prefix matches weighted 0.375, multiplied by the number of matching
// query terms); the Rust port sums integer weighted term frequencies. Scores and therefore ranking
// differ by design; only the matched document set is shared behaviour (see tools/timing-protocol.mjs).
import MiniSearch from '../../../node_modules/minisearch/dist/es/index.js'

function parseRows(packed) {
  const documents = []
  for (const row of packed.split('\n')) {
    const first = row.indexOf('|')
    if (first < 0) continue
    const second = row.indexOf('|', first + 1)
    if (second < 0) continue
    documents.push({ id: row.slice(0, first), title: row.slice(first + 1, second), body: row.slice(second + 1) })
  }
  return documents
}

function parseQuery(query) {
  const queries = []
  for (const word of query.split(/[\t\n\f\r ]+/)) {
    if (!word) continue
    const prefix = word.endsWith('*')
    queries.push({ queries: [prefix ? word.slice(0, -1) : word], prefix })
  }
  return { combineWith: 'OR', queries }
}

const processTerm = (term) => (term.length > 1 ? term.toLowerCase() : null)

export function run({ documents, query }) {
  const index = new MiniSearch({ fields: ['title', 'body'], processTerm, searchOptions: { boost: { title: 3 } } })
  index.addAll(parseRows(documents))
  return index.search(parseQuery(query)).map((result) => `${result.id}:${result.score}`).join(',')
}
