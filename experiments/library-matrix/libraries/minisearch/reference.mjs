// Reference adapter for MiniSearch 7.2.0: the published ESM entry, imported by a relative path (no
// bare specifier, no import map), so Node and the browsers load the same file.
//
// `run(input)` executes an operation script (CONTRACT.md, "Input") against one MiniSearch
// instance and returns its canonical result (CONTRACT.md, "Output"). The adapter maps JSON onto the
// package API and adds no behaviour of its own:
//
// - the three callback options a JSON corpus cannot spell (`filter`, `boostDocument`,
//   `boostTerm`) are given in a declarative form and turned into the callbacks they describe;
// - `{ "wildcard": true }` is `MiniSearch.wildcard`;
// - the `logger` option records the messages MiniSearch logs instead of printing them;
// - an operation that throws is recorded with MiniSearch's error message and the script goes on
//   with the instance in whatever state MiniSearch left it;
// - result objects are copied out key by key; `match` becomes a list of `[term, fields]` pairs in
//   `Object.keys` order, and `toJSON` is the value `JSON.parse(JSON.stringify(index))`.
import MiniSearch from '../../../../node_modules/minisearch/dist/es/index.js'

// filter: { field, in: [values] }  ->  (result) => values.includes(result[field])
// boostDocument: [[id, factor], ...]  ->  (id) => factor of the first pair whose id is `id`, else 1
// boostTerm: [[term, factor], ...]  ->  (term) => factor of the first pair whose term is `term`, else 1
function callbacks(json) {
  if (json === undefined) return undefined
  const options = { ...json }
  if (json.filter !== undefined) {
    const { field, in: values } = json.filter
    options.filter = (result) => values.includes(result[field])
  }
  if (json.boostDocument !== undefined) {
    const pairs = json.boostDocument
    options.boostDocument = (id) => {
      for (const [key, factor] of pairs) if (key === id) return factor
      return 1
    }
  }
  if (json.boostTerm !== undefined) {
    const pairs = json.boostTerm
    options.boostTerm = (term) => {
      for (const [key, factor] of pairs) if (key === term) return factor
      return 1
    }
  }
  return options
}

function query(json) {
  if (typeof json === 'string') return json
  if (json.wildcard === true) return MiniSearch.wildcard
  const { queries, ...options } = json
  return { ...callbacks(options), queries: queries.map(query) }
}

function hit(result) {
  const out = {}
  for (const key of Object.keys(result)) {
    out[key] = key === 'match' ? Object.keys(result.match).map((term) => [term, result.match[term]]) : result[key]
  }
  return out
}

export function run(input) {
  let warnings = []
  const { searchOptions, autoSuggestOptions, ...rest } = input.options
  const options = { ...rest, logger: (level, message) => { warnings.push(message) } }
  if (searchOptions !== undefined) options.searchOptions = callbacks(searchOptions)
  if (autoSuggestOptions !== undefined) options.autoSuggestOptions = callbacks(autoSuggestOptions)
  const index = new MiniSearch(options)

  const results = []
  for (const operation of input.operations) {
    const record = { op: operation.op }
    warnings = []
    try {
      switch (operation.op) {
        case 'add': index.add(operation.document); break
        case 'addAll': index.addAll(operation.documents); break
        case 'remove': index.remove(operation.document); break
        case 'removeAll':
          if (operation.documents === undefined) index.removeAll()
          else index.removeAll(operation.documents)
          break
        case 'discard': index.discard(operation.id); break
        case 'discardAll': index.discardAll(operation.ids); break
        case 'replace': index.replace(operation.document); break
        // The returned Promise settles after the synchronous run (CONTRACT.md, "Vacuuming").
        case 'vacuum': index.vacuum(operation.options); break
        case 'search':
          record.hits = index.search(query(operation.query), callbacks(operation.options)).map(hit)
          break
        case 'autoSuggest':
          record.suggestions = index.autoSuggest(operation.query, callbacks(operation.options))
            .map(({ suggestion, terms, score }) => ({ suggestion, terms, score }))
          break
        case 'has': record.value = index.has(operation.id); break
        case 'getStoredFields': {
          const stored = index.getStoredFields(operation.id)
          record.value = stored === undefined ? null : { ...stored }
          break
        }
        case 'stats':
          record.documentCount = index.documentCount
          record.termCount = index.termCount
          record.dirtCount = index.dirtCount
          record.dirtFactor = index.dirtFactor
          record.isVacuuming = index.isVacuuming
          break
        case 'toJSON': record.value = JSON.parse(JSON.stringify(index)); break
        default: throw new TypeError(`unknown operation ${operation.op}`)
      }
    } catch (error) {
      if (warnings.length) record.warnings = warnings
      record.error = error.message
      results.push(record)
      continue
    }
    if (warnings.length) record.warnings = warnings
    results.push(record)
  }
  return { results }
}
