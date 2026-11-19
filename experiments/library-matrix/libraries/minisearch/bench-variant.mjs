// Bench variant for minisearch (registry `bench.variant`; README "Adding a library", step 5).
// Adopted from the verifier's module (build/library-matrix-verify-minisearch/libraries/minisearch/
// bench-variant.mjs), unchanged in behaviour.
//
// The k-th variant (k >= 1) applies a k-seeded permutation of the ASCII letters (one permutation,
// applied to upper and lower case alike) to every document string value and every query string,
// and adds k * 10,000 to every numeric id. `i` and `I` are fixed points: `İ`.toLowerCase() yields
// an ASCII `i`, so permuting `i` would change which terms share prefixes.
//
// (a) Content changes throughout: every title, text, stored string and query changes with k, so
//     no cache keyed on a document, a field or a query can hit.
// (b) The work is the same: a bijection on code units that maps letters to letters keeps every
//     token boundary (letters are never separators), every case relation (both cases permuted
//     alike), every equality and prefix relation between terms, every edit distance, every field
//     length and the insertion order of everything, so the radix tree has the same shape and
//     every score is the same double; only the strings and ids in the output change.
// (c) Deterministic, and inside the parity domain: ids stay below 2^31 for k < 214,000.
//
// Only the bench cases' operations (`addAll`, `add`, `search`, `autoSuggest`, `stats`) are
// handled; anything else is an error rather than an unvaried operation.
function mulberry32(seed) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let value = state
    value = Math.imul(value ^ (value >>> 15), value | 1)
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296
  }
}

function permutation(k) {
  const random = mulberry32(0x9e3779b9 ^ k)
  const letters = 'abcdefghjklmnopqrstuvwxyz'.split('')
  for (let index = letters.length - 1; index > 0; index--) {
    const other = Math.floor(random() * (index + 1))
    ;[letters[index], letters[other]] = [letters[other], letters[index]]
  }
  const map = new Map([['i', 'i'], ['I', 'I']])
  'abcdefghjklmnopqrstuvwxyz'.split('').forEach((letter, index) => {
    map.set(letter, letters[index])
    map.set(letter.toUpperCase(), letters[index].toUpperCase())
  })
  return (text) => {
    let out = ''
    for (let index = 0; index < text.length; index++) out += map.get(text[index]) ?? text[index]
    return out
  }
}

export function variant(input, k) {
  const swap = permutation(k)
  const value = (field) => (typeof field === 'string' ? swap(field) : field)
  const document = (doc) => {
    const out = {}
    for (const key of Object.keys(doc)) out[key] = key === 'id' ? (typeof doc.id === 'number' ? doc.id + k * 10000 : swap(doc.id)) : value(doc[key])
    return out
  }
  const query = (q) => (typeof q === 'string' ? swap(q) : q.wildcard ? q : { ...q, queries: q.queries.map(query) })
  const operations = input.operations.map((operation) => {
    switch (operation.op) {
      case 'addAll': return { ...operation, documents: operation.documents.map(document) }
      case 'add': return { ...operation, document: document(operation.document) }
      case 'search': return { ...operation, query: query(operation.query) }
      case 'autoSuggest': return { ...operation, query: swap(operation.query) }
      case 'stats': return operation
      default: throw new Error(`bench variant: unexpected operation ${operation.op}`)
    }
  })
  return { options: input.options, operations }
}
