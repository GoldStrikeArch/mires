#!/usr/bin/env node
// Writes corpus.json for the minisearch library: every case is built here, deterministically (no
// clock, no Math.random: bench text comes from mulberry32 with fixed seeds), so the file is a pure
// function of this script. CONTRACT.md lists what each case exercises.
//
//   node libraries/minisearch/generate-corpus.mjs            # writes corpus.json
//   node libraries/minisearch/generate-corpus.mjs --verify   # also runs the reference on every case
//                                                            # and checks the parity-domain rules
//
// --verify checks, on the Node reference: no score is NaN or infinite (byScore sorting is only
// defined on comparable numbers), every recorded error is one of MiniSearch's own messages (engine
// messages differ between browsers), and it prints each bench case's time per call.

import { writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

// ---------------------------------------------------------------------------------------------
// Deterministic pseudo-random text for the bench cases.

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

const ONSETS = ['', 'b', 'c', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'm', 'n', 'p', 'r', 's', 't', 'v', 'w', 'z',
  'br', 'cl', 'cr', 'dr', 'fl', 'gr', 'pl', 'pr', 'sh', 'st', 'th', 'tr', 'ch', 'qu', 'sp']
const VOWELS = ['a', 'e', 'i', 'o', 'u', 'ai', 'ea', 'ou', 'io', 'y']
const CODAS = ['', '', '', 'n', 'r', 's', 't', 'l', 'm', 'nd', 'st', 'ng', 'ck', 'x']
// Non-ASCII words mixed into the bench text at a low rate, so the bench exercises the Unicode
// paths of every port too (tokenizer, lower-casing, UTF-16 radix keys).
const FOREIGN = ['Straße', 'café', 'naïve', 'Ελλάδα', 'ΟΔΟΣ', 'Москва', 'привет', '東京', '日本語', 'İstanbul',
  'smörgåsbord', 'Ærøskøbing', 'Zürich', 'München', 'Kraków', 'ﬁnance', '😀', '🚀launch', 'naïveté', 'résumé']

function vocabulary(random, size) {
  const words = new Set()
  while (words.size < size) {
    const syllables = 1 + Math.floor(random() * 3.2)
    let word = ''
    for (let index = 0; index < syllables; index++) {
      word += ONSETS[Math.floor(random() * ONSETS.length)] + VOWELS[Math.floor(random() * VOWELS.length)]
      if (index === syllables - 1 || random() < 0.3) word += CODAS[Math.floor(random() * CODAS.length)]
    }
    if (word.length >= 2) words.add(word)
  }
  return [...words]
}

// Zipf-like sampler over a word list (rank r has weight 1 / (r + 2) ** s).
function zipf(random, words, s) {
  const cumulative = []
  let total = 0
  for (let rank = 0; rank < words.length; rank++) {
    total += 1 / (rank + 2) ** s
    cumulative.push(total)
  }
  return () => {
    const target = random() * total
    let low = 0
    let high = cumulative.length - 1
    while (low < high) {
      const middle = (low + high) >> 1
      if (cumulative[middle] < target) low = middle + 1
      else high = middle
    }
    return words[low]
  }
}

function capitalise(word) {
  return word.charAt(0).toUpperCase() + word.slice(1)
}

function sentence(random, pick, words) {
  const out = []
  for (let index = 0; index < words; index++) {
    let word = random() < 0.025 ? FOREIGN[Math.floor(random() * FOREIGN.length)] : pick()
    if (index === 0) word = capitalise(word)
    if (random() < 0.02) word = String(1900 + Math.floor(random() * 125))
    out.push(word)
    if (index < words - 1 && random() < 0.08) out[out.length - 1] += random() < 0.5 ? ',' : ';'
  }
  return `${out.join(' ')}${random() < 0.85 ? '.' : random() < 0.5 ? '!' : '?'}`
}

const CATEGORIES = ['fiction', 'history', 'science', 'poetry', 'travel', 'cooking', 'philosophy', 'art']

function benchDocuments(seed, count, { textSentences = [2, 5] } = {}) {
  const random = mulberry32(seed)
  const words = vocabulary(random, 3000)
  const pick = zipf(random, words, 1.07)
  const documents = []
  for (let id = 1; id <= count; id++) {
    const sentences = textSentences[0] + Math.floor(random() * (textSentences[1] - textSentences[0] + 1))
    const text = []
    for (let index = 0; index < sentences; index++) text.push(sentence(random, pick, 6 + Math.floor(random() * 12)))
    documents.push({
      id,
      title: sentence(random, pick, 2 + Math.floor(random() * 6)).slice(0, -1),
      text: text.join(' '),
      category: CATEGORIES[Math.floor(random() * CATEGORIES.length)],
      year: 1950 + Math.floor(random() * 75),
    })
  }
  return { documents, words, pick, random }
}

// ---------------------------------------------------------------------------------------------
// A small library of real-looking documents shared by many cases.

const BOOKS = [
  { id: 1, title: 'Moby Dick', text: 'Call me Ishmael. Some years ago, never mind how long precisely, having little or no money in my purse...', category: 'fiction', year: 1851 },
  { id: 2, title: 'Zen and the Art of Motorcycle Maintenance', text: 'I can see by my watch, without taking my hand from the left grip of the cycle...', category: 'fiction', year: 1974 },
  { id: 3, title: 'Neuromancer', text: 'The sky above the port was the color of television, tuned to a dead channel.', category: 'fiction', year: 1984 },
  { id: 4, title: 'Zen and the Art of Archery', text: 'At first sight it must seem intolerably degrading for Zen, the art of archery...', category: 'non-fiction', year: 1948 },
  { id: 5, title: 'The Art of Computer Programming', text: 'The process of preparing programs for a digital computer is especially attractive...', category: 'non-fiction', year: 1968 },
  { id: 6, title: 'Motorcycle Diaries', text: 'This is not a story of heroic feats; it is a glimpse of two lives running parallel...', category: 'travel', year: 1993 },
  { id: 7, title: 'The Motorcyclist\'s Handbook', text: 'Motorcycles, motorbikes and motor scooters: maintenance, tuning, repair.', category: 'non-fiction', year: 2001 },
  { id: 8, title: 'Archery for Beginners', text: 'Archers draw the bow; the archer\'s art is patience. Arch, archer, archery.', category: 'non-fiction', year: 2003 },
  { id: 9, title: 'Art and Artists', text: 'Artists and artisans: the art of the artist, artful and artless art.', category: 'art', year: 1999 },
  { id: 10, title: 'A Tale of Two Cities', text: 'It was the best of times, it was the worst of times, it was the age of wisdom...', category: 'fiction', year: 1859 },
  { id: 11, title: 'Kate and the Cat', text: 'Kate, Katie and Cate met a cat, a kit and a kitten on a crate near the gate.', category: 'fiction', year: 2010 },
  { id: 12, title: 'Skating on Thin Ice', text: 'Skate, skater, skaters and skating: the state of the slate in late winter.', category: 'travel', year: 2015 },
]

const README_DOCS = BOOKS.slice(0, 4)
const book = (id) => ({ ...BOOKS.find((item) => item.id === id) })

// ---------------------------------------------------------------------------------------------
// The cases.

const cases = []
function add(id, note, input) {
  cases.push({ id, note, input })
}

const basic = { fields: ['title', 'text'], storeFields: ['title', 'category'] }

add('empty-index', 'every operation on an index with no documents', {
  options: { fields: ['title', 'text'] },
  operations: [
    { op: 'search', query: 'anything' },
    { op: 'search', query: 'anything', options: { prefix: true, fuzzy: 2 } },
    { op: 'search', query: { wildcard: true } },
    { op: 'autoSuggest', query: 'any' },
    { op: 'has', id: 1 },
    { op: 'getStoredFields', id: 1 },
    { op: 'discard', id: 1 },
    { op: 'remove', document: { id: 1, title: 'x' } },
    { op: 'vacuum' },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('readme-examples', 'the examples of the MiniSearch README and API docs, on its four sample documents', {
  options: basic,
  operations: [
    { op: 'addAll', documents: README_DOCS },
    { op: 'search', query: 'zen art motorcycle' },
    { op: 'search', query: 'zen', options: { fields: ['title'] } },
    { op: 'search', query: 'zen', options: { boost: { title: 2 } } },
    { op: 'search', query: 'moto neuro', options: { prefix: true } },
    { op: 'search', query: 'ismael', options: { fuzzy: 0.2 } },
    { op: 'search', query: 'ismael mob', options: { prefix: true, fuzzy: 0.2 } },
    { op: 'search', query: 'motorcycle art', options: { combineWith: 'AND' } },
    { op: 'search', query: 'motorcycle art', options: { filter: { field: 'category', in: ['fiction'] } } },
    { op: 'search', query: { wildcard: true } },
    { op: 'search', query: { wildcard: true }, options: { filter: { field: 'category', in: ['fiction'] } } },
    { op: 'search', query: { combineWith: 'AND', queries: ['zen', { combineWith: 'OR', queries: ['motorcycle', 'archery'] }] } },
    { op: 'search', query: { combineWith: 'AND_NOT', queries: [{ combineWith: 'OR', queries: ['zen', 'sky'] }, 'archery', 'motorcycle'] } },
    { op: 'autoSuggest', query: 'neuro' },
    { op: 'autoSuggest', query: 'zen ar' },
    { op: 'autoSuggest', query: 'neromancer', options: { fuzzy: 0.2 } },
    { op: 'autoSuggest', query: 'zen ar', options: { filter: { field: 'category', in: ['fiction'] } } },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('empty-and-separator-queries', 'queries that tokenize to nothing, to empty strings only, or to a tab (not a separator)', {
  options: basic,
  operations: [
    { op: 'addAll', documents: README_DOCS },
    { op: 'search', query: '' },
    { op: 'search', query: ' ' },
    { op: 'search', query: '...!?' },
    { op: 'search', query: '\n\r 　' },
    { op: 'search', query: '\t' },
    { op: 'search', query: ',zen,' },
    { op: 'search', query: '', options: { prefix: true, fuzzy: 1 } },
    { op: 'autoSuggest', query: '' },
    { op: 'autoSuggest', query: ' ' },
    { op: 'search', query: { queries: [] } },
    { op: 'search', query: { queries: ['', ' ', 'zen'] } },
    { op: 'search', query: { combineWith: 'AND', queries: ['', 'zen'] } },
  ],
})

add('combine-operators', 'OR, AND and AND_NOT in any letter case, invalid operators, and when they are or are not reached', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 9) },
    { op: 'search', query: 'zen art archery', options: { combineWith: 'OR' } },
    { op: 'search', query: 'zen art archery', options: { combineWith: 'or' } },
    { op: 'search', query: 'zen art archery', options: { combineWith: 'Or' } },
    { op: 'search', query: 'zen art archery', options: { combineWith: 'AND' } },
    { op: 'search', query: 'art zen archery', options: { combineWith: 'and' } },
    { op: 'search', query: 'art zen', options: { combineWith: 'AND_NOT' } },
    { op: 'search', query: 'art zen motorcycle', options: { combineWith: 'and_not' } },
    { op: 'search', query: 'zen art', options: { combineWith: 'And_Not' } },
    { op: 'search', query: 'nothing art', options: { combineWith: 'AND_NOT' } },
    { op: 'search', query: 'art nothing', options: { combineWith: 'AND' } },
    { op: 'search', query: 'zen sky', options: { combineWith: 'AND' } },
    { op: 'search', query: 'zen', options: { combineWith: 'XOR' } },
    { op: 'search', query: 'zen art', options: { combineWith: 'XOR' } },
    { op: 'search', query: '', options: { combineWith: 'XOR' } },
    { op: 'search', query: { combineWith: 'XOR', queries: [] } },
    { op: 'search', query: { combineWith: 'XOR', queries: ['zen'] } },
    { op: 'search', query: { queries: ['zen art'], combineWith: 'NAND' } },
    { op: 'autoSuggest', query: 'zen ar', options: { combineWith: 'OR' } },
    { op: 'autoSuggest', query: 'zen ar', options: { combineWith: 'NOPE' } },
  ],
})

add('duplicate-query-terms', 'repeated query terms are scored once per occurrence but counted once for quality', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 9) },
    { op: 'search', query: 'art' },
    { op: 'search', query: 'art art' },
    { op: 'search', query: 'Art ART art' },
    { op: 'search', query: 'art art zen', options: { combineWith: 'AND' } },
    { op: 'search', query: 'art art', options: { combineWith: 'AND_NOT' } },
    { op: 'search', query: 'ar ar', options: { prefix: true } },
    { op: 'search', query: 'archer archer', options: { fuzzy: 1 } },
    { op: 'autoSuggest', query: 'art art' },
  ],
})

add('field-boosts-and-fields', 'boost (0 counts as 1, negative, fractional), field restriction, unknown, repeated and reordered fields', {
  options: { fields: ['title', 'text', 'category'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'art fiction' },
    { op: 'search', query: 'art fiction', options: { boost: { title: 3, text: 0.5 } } },
    { op: 'search', query: 'art fiction', options: { boost: { title: 0, category: 2 } } },
    { op: 'search', query: 'art fiction', options: { boost: { text: -1 } } },
    { op: 'search', query: 'art fiction', options: { boost: { nope: 5 } } },
    { op: 'search', query: 'art fiction', options: { fields: ['text'] } },
    { op: 'search', query: 'art fiction', options: { fields: [] } },
    { op: 'search', query: 'art fiction', options: { fields: ['nope'] } },
    { op: 'search', query: 'art fiction', options: { fields: ['title', 'title'] } },
    { op: 'search', query: 'art the of', options: { fields: ['text', 'title'] } },
    { op: 'search', query: 'art the of', options: { fields: ['title', 'text'] } },
    { op: 'search', query: 'art the of', options: { fields: ['category', 'text', 'nope', 'title'], boost: { category: 1.5 } } },
  ],
})

add('prefix-search', 'prefix matches, their weight by extra length, the exact term counted once, and custom prefix weights', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'mot', options: { prefix: true } },
    { op: 'search', query: 'motor', options: { prefix: true } },
    { op: 'search', query: 'motorcycle', options: { prefix: true } },
    { op: 'search', query: 'art', options: { prefix: true } },
    { op: 'search', query: 'ar', options: { prefix: true, weights: { prefix: 0.9 } } },
    { op: 'search', query: 'a', options: { prefix: true } },
    { op: 'search', query: 'sk sl', options: { prefix: true, combineWith: 'AND' } },
    { op: 'search', query: 'arch', options: { prefix: false } },
    { op: 'search', query: 'xyz', options: { prefix: true } },
  ],
})

add('fuzzy-integer', 'integer fuzzy distances 0 to 3: Levenshtein over UTF-16 code units, the weight by distance', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'kate', options: { fuzzy: 0 } },
    { op: 'search', query: 'kate', options: { fuzzy: 1 } },
    { op: 'search', query: 'kate', options: { fuzzy: 2 } },
    { op: 'search', query: 'kate', options: { fuzzy: 3 } },
    { op: 'search', query: 'skate', options: { fuzzy: 2 } },
    { op: 'search', query: 'kitten', options: { fuzzy: 3, maxFuzzy: 1 } },
    { op: 'search', query: 'gate state', options: { fuzzy: 1 } },
    { op: 'search', query: 'xq', options: { fuzzy: 2 } },
    { op: 'search', query: 'a', options: { fuzzy: 1 } },
  ],
})

add('fuzzy-fractional', 'fractional fuzzy: Math.round(length * fuzzy) with ties rounded up, capped by maxFuzzy; fuzzy: true is 0.2', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'skatez', options: { fuzzy: true } },
    { op: 'search', query: 'skatez', options: { fuzzy: 0.2 } },
    { op: 'search', query: 'kates', options: { fuzzy: 0.3 } },
    { op: 'search', query: 'kates', options: { fuzzy: 0.5 } },
    { op: 'search', query: 'katie', options: { fuzzy: 0.5, maxFuzzy: 2 } },
    { op: 'search', query: 'ka', options: { fuzzy: 0.25 } },
    { op: 'search', query: 'kittenskat', options: { fuzzy: 0.25 } },
    { op: 'search', query: 'kittenskat', options: { fuzzy: 0.99 } },
    { op: 'search', query: 'kittenskat', options: { fuzzy: 0.99, maxFuzzy: 0 } },
    { op: 'search', query: 'motorcyclez', options: { fuzzy: 0.34 } },
    { op: 'search', query: 'sk', options: { fuzzy: 0.2 } },
  ],
})

add('prefix-and-fuzzy', 'prefix and fuzzy together: a term matched by both is scored as a prefix match only', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'moto', options: { prefix: true, fuzzy: 2 } },
    { op: 'search', query: 'skat', options: { prefix: true, fuzzy: 1 } },
    { op: 'search', query: 'arche', options: { prefix: true, fuzzy: 0.4 } },
    { op: 'search', query: 'kat cat', options: { prefix: true, fuzzy: 1, combineWith: 'AND' } },
    { op: 'search', query: 'art', options: { prefix: true, fuzzy: 1, weights: { fuzzy: 0.8, prefix: 0.2 } } },
    { op: 'autoSuggest', query: 'kat', options: { fuzzy: 1 } },
  ],
})

add('weights-and-bm25', 'weights merged with the library defaults, BM25 parameters, and object options replaced wholesale', {
  options: { ...basic, searchOptions: { weights: { fuzzy: 0.1 }, bm25: { k: 2, b: 0.3, d: 1 } } },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'skate art', options: { fuzzy: 1, prefix: true } },
    { op: 'search', query: 'skate art', options: { fuzzy: 1, prefix: true, weights: { prefix: 0.5 } } },
    { op: 'search', query: 'skate art', options: { fuzzy: 1, prefix: true, weights: { fuzzy: 1, prefix: 1 } } },
    { op: 'search', query: 'skate art', options: { bm25: { k: 1.2, b: 0.7, d: 0.5 } } },
    { op: 'search', query: 'skate art', options: { bm25: { k: 0, b: 0, d: 0 } } },
    { op: 'search', query: 'skate art', options: { bm25: { k: 1.2, b: 1, d: 0 } } },
    { op: 'search', query: 'skate art', options: { bm25: { k: 3.5, b: 0.25, d: 2.75 } } },
    { op: 'autoSuggest', query: 'ska' },
  ],
})

add('constructor-search-options', 'searchOptions given to the constructor: defaults for every search, overridden per call; a query tree node ignores the constructor combineWith', {
  options: { ...basic, searchOptions: { prefix: true, fuzzy: 1, boost: { title: 2 }, combineWith: 'AND' } },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'art zen' },
    { op: 'search', query: 'art zen', options: { combineWith: 'OR' } },
    { op: 'search', query: 'art zen', options: { prefix: false, fuzzy: false } },
    { op: 'search', query: 'art zen', options: { boost: { text: 3 } } },
    { op: 'search', query: { queries: ['art', 'zen'] } },
    { op: 'search', query: { queries: ['art zen', 'sky'] } },
    { op: 'search', query: { queries: ['art zen', 'sky'] }, options: { combineWith: 'AND' } },
    { op: 'search', query: { combineWith: 'OR', queries: ['art zen', 'sky'] } },
    { op: 'autoSuggest', query: 'zen ar' },
  ],
})

add('filter-stored-fields', 'the declarative filter over stored fields (strings and numbers), in the constructor, and ignored inside a query tree', {
  options: { fields: ['title', 'text'], storeFields: ['category', 'year'] },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'art', options: { prefix: true, filter: { field: 'category', in: ['fiction', 'art'] } } },
    { op: 'search', query: 'art', options: { prefix: true, filter: { field: 'year', in: [1974, 1999, 2003] } } },
    { op: 'search', query: 'art', options: { prefix: true, filter: { field: 'year', in: ['1974'] } } },
    { op: 'search', query: 'art', options: { prefix: true, filter: { field: 'title', in: ['Art and Artists'] } } },
    { op: 'search', query: 'art', options: { prefix: true, filter: { field: 'category', in: [] } } },
    { op: 'search', query: { queries: ['art'], filter: { field: 'category', in: ['art'] } } },
    { op: 'search', query: { wildcard: true }, options: { filter: { field: 'year', in: [1851, 1859, 2015] } } },
    { op: 'autoSuggest', query: 'ar', options: { filter: { field: 'category', in: ['non-fiction'] } } },
  ],
})

add('filter-in-constructor', 'a filter in the constructor searchOptions applies to search and autoSuggest unless replaced', {
  options: { fields: ['title', 'text'], storeFields: ['category'], searchOptions: { filter: { field: 'category', in: ['fiction'] } } },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'art the' },
    { op: 'search', query: 'art the', options: { filter: { field: 'category', in: ['non-fiction'] } } },
    { op: 'autoSuggest', query: 'th' },
    { op: 'search', query: { wildcard: true } },
  ],
})

add('boost-document', 'boostDocument by id: factors multiply scores, 0 drops a document from a term (not from a wildcard), ids compare by type', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'art zen', options: { boostDocument: [[2, 3], [4, 0], [9, 0.5]] } },
    { op: 'search', query: 'art zen', options: { boostDocument: [['2', 5]] } },
    { op: 'search', query: 'art zen', options: { boostDocument: [[2, -1]] } },
    { op: 'search', query: 'art', options: { prefix: true, boostDocument: [[9, 2], [7, 1.5]] } },
    { op: 'search', query: { wildcard: true }, options: { boostDocument: [[3, 2], [5, 0], [1, 0.25]] } },
    { op: 'search', query: { combineWith: 'AND_NOT', queries: [{ wildcard: true }, 'art'] }, options: { boostDocument: [[3, 2]] } },
    { op: 'autoSuggest', query: 'ar', options: { boostDocument: [[8, 4]] } },
  ],
})

add('boost-term', 'boostTerm by processed query term, applied to that term and every term derived from it', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: 'Zen art', options: { boostTerm: [['zen', 2], ['art', 0.5]] } },
    { op: 'search', query: 'Zen art', options: { boostTerm: [['Zen', 2]] } },
    { op: 'search', query: 'ar mot', options: { prefix: true, fuzzy: 1, boostTerm: [['ar', 3]] } },
    { op: 'search', query: 'zen art', options: { boostTerm: [['zen', 0]] } },
  ],
})

add('query-trees', 'nested query trees: per-node options inherited by subqueries, wildcard subqueries, empty and single subquery lists', {
  options: basic,
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'search', query: { queries: ['zen', { queries: ['motorcycle', 'archery'], combineWith: 'OR' }], combineWith: 'AND' } },
    { op: 'search', query: { queries: ['art', { queries: ['moto', 'arch'], prefix: true }], combineWith: 'AND', boost: { title: 2 } } },
    { op: 'search', query: { queries: [{ queries: ['kate'], fuzzy: 2 }, { queries: ['skat'], prefix: true, fields: ['title'] }] } },
    { op: 'search', query: { queries: [{ queries: [{ queries: ['art'], prefix: true }, 'zen'], combineWith: 'and' }, 'sky'], combineWith: 'or' } },
    { op: 'search', query: { queries: [{ wildcard: true }, 'art'], combineWith: 'AND_NOT' } },
    { op: 'search', query: { queries: [{ wildcard: true }, 'art'], combineWith: 'OR' } },
    { op: 'search', query: { queries: ['art', { wildcard: true }], combineWith: 'AND' } },
    { op: 'search', query: { queries: [{ queries: [] }, 'art'], combineWith: 'OR' } },
    { op: 'search', query: { queries: [{ queries: [] }, 'art'], combineWith: 'AND' } },
    { op: 'search', query: { queries: ['art zen'], combineWith: 'AND' } },
    { op: 'search', query: { queries: ['art zen'] }, options: { combineWith: 'AND' } },
    { op: 'search', query: { queries: ['art', 'zen', 'sky', 'kate'], combineWith: 'AND_NOT', fuzzy: 1 } },
    { op: 'search', query: { queries: ['skating', 'skate'], weights: { fuzzy: 1 }, fuzzy: 2, bm25: { k: 1, b: 0.5, d: 0.25 } } },
  ],
})

add('stored-fields', 'storeFields including the id field under a custom idField, absent and null values, numbers; getStoredFields', {
  options: { fields: ['title'], storeFields: ['id', 'title', 'year', 'note', 'missing'], idField: 'key' },
  operations: [
    { op: 'addAll', documents: [
      { key: 'a', id: 100, title: 'Alpha beta', year: 2001, note: null },
      { key: 'b', title: 'Beta gamma', year: 0, note: 'second' },
      { key: 'c', id: 'C', title: 'Gamma delta' },
      { key: 7, id: 7, title: 'Delta alpha', note: '' },
    ] },
    { op: 'search', query: 'alpha beta gamma delta' },
    { op: 'search', query: { wildcard: true } },
    { op: 'getStoredFields', id: 'a' },
    { op: 'getStoredFields', id: 'b' },
    { op: 'getStoredFields', id: 7 },
    { op: 'getStoredFields', id: '7' },
    { op: 'search', query: 'alpha', options: { filter: { field: 'id', in: [100, 7] } } },
    { op: 'toJSON' },
  ],
})

add('no-store-fields', 'with no storeFields nothing is stored: getStoredFields is null even for a present document', {
  options: { fields: ['title', 'text'] },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 3) },
    { op: 'getStoredFields', id: 1 },
    { op: 'has', id: 1 },
    { op: 'search', query: 'sky moby' },
    { op: 'toJSON' },
  ],
})

add('id-types', 'numeric and string ids are distinct keys (1 is not "1"); zero, negative and Unicode ids', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: 'one number' },
      { id: '1', text: 'one string' },
      { id: 0, text: 'zero number' },
      { id: -42, text: 'negative number' },
      { id: 'Ünïcødé id 🚀', text: 'unicode string' },
      { id: '', text: 'empty string id' },
    ] },
    { op: 'search', query: 'one number string' },
    { op: 'has', id: 1 },
    { op: 'has', id: '1' },
    { op: 'has', id: 2 },
    { op: 'has', id: '' },
    { op: 'discard', id: '1' },
    { op: 'search', query: 'one' },
    { op: 'add', document: { id: '1', text: 'one again' } },
    { op: 'search', query: 'one' },
    { op: 'remove', document: { id: -42, text: 'negative number' } },
    { op: 'getStoredFields', id: 'Ünïcødé id 🚀' },
    { op: 'getStoredFields', id: -42 },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('document-errors', 'errors keep MiniSearch\'s messages and leave the state MiniSearch leaves (addAll and discardAll stop part way); removeAll(null) is an error, not removeAll()', {
  options: { fields: ['title', 'text'] },
  operations: [
    { op: 'add', document: { title: 'no id' } },
    { op: 'add', document: { id: null, title: 'null id' } },
    { op: 'addAll', documents: [book(1), book(2), book(1), book(3)] },
    { op: 'stats' },
    { op: 'add', document: book(2) },
    { op: 'remove', document: { id: 99, title: 'not there' } },
    { op: 'remove', document: { title: 'no id at all' } },
    { op: 'discard', id: 99 },
    { op: 'discard', id: '1' },
    { op: 'discardAll', ids: [1, 99, 2] },
    { op: 'stats' },
    { op: 'replace', document: { id: 98, title: 'replaces nothing' } },
    { op: 'replace', document: { id: 2, title: 'already discarded' } },
    { op: 'removeAll', documents: [{ id: 97, title: 'x' }] },
    { op: 'removeAll', documents: null },
    { op: 'search', query: 'moby zen sky' },
    { op: 'stats' },
  ],
})

add('remove-and-readd', 'remove() deletes postings immediately; average field lengths shrink and reset; re-adding gets a new short id', {
  options: { fields: ['title', 'text'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 6) },
    { op: 'search', query: 'art zen the', options: { prefix: true } },
    { op: 'remove', document: book(2) },
    { op: 'search', query: 'art zen the', options: { prefix: true } },
    { op: 'toJSON' },
    { op: 'add', document: { id: 2, title: 'Zen Motorcycle', text: 'Rewritten text: the art of the road.' } },
    { op: 'search', query: 'art zen the', options: { prefix: true } },
    { op: 'remove', document: book(1) },
    { op: 'remove', document: book(3) },
    { op: 'remove', document: book(4) },
    { op: 'remove', document: book(5) },
    { op: 'remove', document: book(6) },
    { op: 'remove', document: { id: 2, title: 'Zen Motorcycle', text: 'Rewritten text: the art of the road.' } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'add', document: book(9) },
    { op: 'search', query: 'art' },
    { op: 'toJSON' },
  ],
})

add('remove-changed-document', 'removing a document that changed since it was added logs version_conflict warnings and leaves stray postings', {
  options: { fields: ['title', 'text'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 5) },
    { op: 'remove', document: { id: 2, title: 'Zen and the Art of Bicycle Maintenance', text: 'I can see by my clock.' } },
    { op: 'search', query: 'motorcycle zen watch art', options: { prefix: true } },
    { op: 'search', query: 'motorcycle zen watch art', options: { prefix: true } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'removeAll', documents: [{ id: 3, title: 'Neuromancer', text: 'changed text entirely' }, { id: 4, title: 'Zen' }] },
    { op: 'stats' },
  ],
})

add('discard-cleanup-on-search', 'discarded documents are skipped by search, which removes (or decrements) their postings as it meets them', {
  options: { fields: ['title', 'text'], storeFields: ['title'], autoVacuum: false },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, title: 'apple banana', text: 'apple apple apple cherry' },
      { id: 2, title: 'apple', text: 'banana banana cherry date' },
      { id: 3, title: 'banana', text: 'apple cherry cherry' },
      { id: 4, title: 'apple cherry', text: 'date elder fig' },
      { id: 5, title: 'applesauce', text: 'apples and apple pie' },
      { id: 6, title: 'grape', text: 'grapefruit grapes grape' },
    ] },
    { op: 'search', query: 'apple cherry' },
    { op: 'discard', id: 1 },
    { op: 'discard', id: 3 },
    { op: 'search', query: 'apple cherry' },
    { op: 'search', query: 'apple cherry' },
    { op: 'search', query: 'apple cherry' },
    { op: 'search', query: 'apple cherry' },
    { op: 'stats' },
    { op: 'discard', id: 6 },
    { op: 'search', query: 'grape', options: { prefix: true, fuzzy: 1 } },
    { op: 'search', query: 'grap', options: { prefix: true, fuzzy: 2 } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'discard', id: 5 },
    { op: 'search', query: 'applesauce', options: { fuzzy: 2 } },
    { op: 'search', query: 'apple', options: { prefix: true, fuzzy: 3 } },
    { op: 'toJSON' },
  ],
})

add('discard-exact-term-merge', 'search deletes an emptied exact term before the fuzzy walk, so the radix merge it causes changes the order of the fuzzy matches', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'add', document: { id: 1, text: 'tea' } },
    { op: 'add', document: { id: 2, text: 'tex tez teal ten' } },
    { op: 'add', document: { id: 3, text: 'tez' } },
    { op: 'search', query: 'tea', options: { fuzzy: 1 } },
    { op: 'discard', id: 1 },
    { op: 'search', query: 'tea', options: { fuzzy: 1 } },
    { op: 'search', query: 'tea', options: { fuzzy: 1 } },
    { op: 'search', query: 'te', options: { prefix: true } },
    { op: 'add', document: { id: 4, text: 'tea teal' } },
    { op: 'search', query: 'tea', options: { fuzzy: 1, prefix: true } },
    { op: 'toJSON' },
  ],
})

add('vacuum-merge-order', 'a vacuum that deletes terms merges radix edges, which reorders later prefix and fuzzy matches', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'add', document: { id: 1, text: 'car' } },
    { op: 'add', document: { id: 2, text: 'cab' } },
    { op: 'add', document: { id: 3, text: 'cart cat cab' } },
    { op: 'add', document: { id: 4, text: 'can' } },
    { op: 'search', query: 'ca', options: { prefix: true } },
    { op: 'search', query: 'car', options: { fuzzy: 1 } },
    { op: 'discardAll', ids: [1, 4] },
    { op: 'vacuum' },
    { op: 'search', query: 'ca', options: { prefix: true } },
    { op: 'search', query: 'car', options: { fuzzy: 1 } },
    { op: 'toJSON' },
  ],
})

const smallDocs = (count, start = 1) => Array.from({ length: count }, (_, index) => ({
  id: start + index,
  text: `doc${start + index} common word${(start + index) % 5} shared${(start + index) % 3} tail${index}`,
}))

add('discard-auto-vacuum', 'automatic vacuuming: triggered at 20 discards and dirt factor 0.1, run synchronously, then later requests deferred for the rest of the run', {
  options: { fields: ['text'] },
  operations: [
    { op: 'addAll', documents: smallDocs(40) },
    { op: 'search', query: 'common shared1' },
    ...Array.from({ length: 19 }, (_, index) => ({ op: 'discard', id: 2 * index + 1 })),
    { op: 'stats' },
    { op: 'discard', id: 39 },
    { op: 'stats' },
    { op: 'search', query: 'common shared1' },
    ...Array.from({ length: 21 }, (_, index) => ({ op: 'discard', id: 2 * index + 2 })).slice(0, 20),
    { op: 'stats' },
    { op: 'search', query: 'common', options: { prefix: true } },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('vacuum-batches', 'an explicit vacuum stops at its first batch boundary in a synchronous run, keeps dirtCount, and defers the next request', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'addAll', documents: smallDocs(12) },
    { op: 'discardAll', ids: [2, 4, 6, 8, 10] },
    { op: 'stats' },
    { op: 'vacuum', options: { batchSize: 7 } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'vacuum' },
    { op: 'stats' },
    { op: 'search', query: 'common shared0 shared1 shared2', options: { prefix: true } },
    { op: 'toJSON' },
  ],
})

add('vacuum-complete', 'an explicit vacuum on a small index completes synchronously and resets dirtCount; batchSize 0 means 1000', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'addAll', documents: smallDocs(10) },
    { op: 'discardAll', ids: [1, 5, 9] },
    { op: 'vacuum', options: { batchSize: 0 } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'discard', id: 2 },
    { op: 'vacuum' },
    { op: 'stats' },
  ],
})

add('vacuum-conditions', 'autoVacuum with custom conditions (2 discards, dirt factor 0.5) and batch size 4: no vacuum until the fourth discard (factor 4/7), which stops after its first 4 terms', {
  options: { fields: ['text'], autoVacuum: { minDirtCount: 2, minDirtFactor: 0.5, batchSize: 4 } },
  operations: [
    { op: 'addAll', documents: smallDocs(6) },
    { op: 'discard', id: 1 },
    { op: 'stats' },
    { op: 'discard', id: 2 },
    { op: 'stats' },
    { op: 'discard', id: 3 },
    { op: 'stats' },
    { op: 'discard', id: 4 },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('vacuum-conditions-defaults', 'autoVacuum with zero conditions falls back to 20 discards and factor 0.1; discardAll triggers one vacuum', {
  options: { fields: ['text'], autoVacuum: { minDirtCount: 0, minDirtFactor: 0 } },
  operations: [
    { op: 'addAll', documents: smallDocs(30) },
    { op: 'discardAll', ids: Array.from({ length: 19 }, (_, index) => index + 1) },
    { op: 'stats' },
    { op: 'discardAll', ids: [20, 21] },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('replace-documents', 'replace() is discard then add: the new version gets a new short id; a replace may vacuum before it adds', {
  options: { fields: ['title', 'text'], storeFields: ['title'], autoVacuum: { minDirtCount: 2, minDirtFactor: 0.01 } },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 6) },
    { op: 'replace', document: { id: 3, title: 'Neuromancer (revised)', text: 'The sky above the harbour was the color of static.' } },
    { op: 'stats' },
    { op: 'search', query: 'sky harbour port', options: { prefix: true } },
    { op: 'replace', document: { id: 4, title: 'Zen in the Art of Archery', text: 'Archery, revisited.' } },
    { op: 'stats' },
    { op: 'search', query: 'zen art archery' },
    { op: 'getStoredFields', id: 3 },
    { op: 'toJSON' },
  ],
})

add('remove-all', 'removeAll() resets the index but keeps dirtCount and the vacuum state; removeAll(documents) removes each', {
  options: { fields: ['title', 'text'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 5) },
    { op: 'discard', id: 1 },
    { op: 'removeAll', documents: [book(2), book(3)] },
    { op: 'stats' },
    { op: 'removeAll' },
    { op: 'stats' },
    { op: 'search', query: 'zen' },
    { op: 'addAll', documents: [book(4), book(1)] },
    { op: 'search', query: 'zen moby', options: { prefix: true } },
    { op: 'toJSON' },
  ],
})

add('auto-suggest', 'autoSuggest: AND with prefix on the last term by default, grouping by matched terms, averaged scores, option overrides', {
  options: { ...basic, autoSuggestOptions: { boost: { title: 2 } } },
  operations: [
    { op: 'addAll', documents: BOOKS },
    { op: 'autoSuggest', query: 'ar' },
    { op: 'autoSuggest', query: 'art ar' },
    { op: 'autoSuggest', query: 'zen ar' },
    { op: 'autoSuggest', query: 'mot' },
    { op: 'autoSuggest', query: 'mot', options: { prefix: false } },
    { op: 'autoSuggest', query: 'mot', options: { prefix: true, combineWith: 'OR' } },
    { op: 'autoSuggest', query: 'skatin', options: { fuzzy: 0.2 } },
    { op: 'autoSuggest', query: 'kat ca', options: { fuzzy: 1 } },
    { op: 'autoSuggest', query: 'the of', options: { boost: { text: 5 } } },
    { op: 'autoSuggest', query: 'zzz' },
  ],
})

add('tokenizer-ascii', 'ASCII separators (\\n \\r space and punctuation) against symbols and controls that are not separators; leading and trailing empty tokens in field lengths', {
  options: { fields: ['text', 'title'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: 'a+b c$d e<f>g h=i j^k l`m n|o p~q', title: 'symbols' },
      { id: 2, text: 'a!b"c#d%e&f\'g(h)i*j,k-l.m/n:o;p?q@r[s\\t]u_v{w}x', title: 'punctuation' },
      { id: 3, text: 'tab\there vt\u000bhere ff\u000chere nul\u0000here del\u007fhere', title: 'controls' },
      { id: 4, text: '  leading and trailing  ', title: ',,both,,' },
      { id: 5, text: '', title: '...' },
      { id: 6, text: 2024, title: -15 },
      { id: 7, text: 'Word word WORD wOrD', title: 'case' },
      { id: 8, text: 'line\nbreak\r\nwindows\rold mac', title: 'C++ & C#' },
      { id: 9, text: 0, title: 'zero' },
    ] },
    { op: 'search', query: 'a+b c$d p~q' },
    { op: 'search', query: 'a b c d' },
    { op: 'search', query: 'tab\there' },
    { op: 'search', query: 'tab here' },
    { op: 'search', query: 'leading trailing both' },
    { op: 'search', query: '2024 15 -15 0' },
    { op: 'search', query: 'word' },
    { op: 'search', query: 'c++ c' },
    { op: 'search', query: 'line break windows old mac' },
    { op: 'search', query: 'nul\u0000here' },
    { op: 'toJSON' },
  ],
})

add('tokenizer-unicode', 'Unicode separators (Zs, Zl, Zp and every P category) in many scripts, and format characters that are not separators', {
  options: { fields: ['text'], storeFields: ['lang'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 'fr', lang: 'fr', text: 'Café, naïve « déjà-vu » — l’été arrive… ¿Qué? ¡Sí!' },
      { id: 'de', lang: 'de', text: 'Straße STRASSE Grüße „Zitat“ Maß' },
      { id: 'ru', lang: 'ru', text: 'Привет, мир! Съешь же ещё этих мягких французских булок.' },
      { id: 'el', lang: 'el', text: 'Γειά σου Κόσμε· η ΟΔΟΣ είναι εδώ;' },
      { id: 'ar', lang: 'ar', text: 'مرحبا بالعالم، كيف حالك؟ هذا نص عربي.' },
      { id: 'he', lang: 'he', text: 'שלום עולם־חדש ״ציטוט״' },
      { id: 'hi', lang: 'hi', text: 'नमस्ते दुनिया। यह हिन्दी है॥' },
      { id: 'th', lang: 'th', text: 'สวัสดีชาวโลก๏ภาษาไทยฯลฯ' },
      { id: 'ja', lang: 'ja', text: '東京は日本の首都です。大阪、京都・奈良「古都」' },
      { id: 'zh', lang: 'zh', text: '你好，世界！中文：测试《书名》' },
      { id: 'ko', lang: 'ko', text: '안녕하세요 세계 한국어·테스트' },
      { id: 'ws', lang: 'ws', text: 'nbsp ogham en em thin hair narrow math ideo　ls ps end' },
      { id: 'fmt', lang: 'fmt', text: 'soft­hyphen zero​width word⁠joiner bom﻿mark zw‍j zwnj‌x' },
      { id: 'dash', lang: 'dash', text: 'en–dash em—dash two⸺em minus−sign hyphen‐x fig‒dash' },
    ] },
    { op: 'search', query: 'café naïve déjà vu été arrive qué sí' },
    { op: 'search', query: 'straße strasse maß zitat' },
    { op: 'search', query: 'привет мир булок' },
    { op: 'search', query: 'κόσμε οδος σου' },
    { op: 'search', query: 'بالعالم حالك' },
    { op: 'search', query: 'עולם חדש ציטוט' },
    { op: 'search', query: 'दुनिया हिन्दी' },
    { op: 'search', query: 'สวัสดีชาวโลก ภาษาไทย ลฯ' },
    { op: 'search', query: '東京は日本の首都です 大阪 京都 奈良 古都' },
    { op: 'search', query: '你好 世界 测试 书名' },
    { op: 'search', query: '안녕하세요 한국어 테스트' },
    { op: 'search', query: 'nbsp ogham en em thin hair narrow math ideo ls ps end' },
    { op: 'search', query: 'soft­hyphen soft hyphen zero​width zero width bom﻿mark zw‍j' },
    { op: 'search', query: 'en dash em two minus−sign hyphen fig' },
    { op: 'search', query: 'стр', options: { prefix: true } },
    { op: 'search', query: 'xyz', options: { prefix: true } },
    { op: 'autoSuggest', query: 'бул' },
    { op: 'toJSON' },
  ],
})

add('astral-text', 'astral code points: emoji, ZWJ sequences, flags, math letters, CJK Extension B, Deseret case mapping, astral punctuation', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: '😀 rocket🚀 🚀🚀 👩‍💻 dev 🇺🇳 flag' },
      { id: 2, text: '𝒳𝒴 math 𝕬𝖑𝖕𝖍𝖆 𠜎𠜱 cjk' },
      { id: 3, text: '𐐀𐐁𐐂 deseret 𐐨𐐩𐐪 𐌰𐌱 gothic' },
      { id: 4, text: 'aegean\u{10100}divider ugarit\u{1039f}word cune\u{12470}iform' },
      { id: 5, text: '😁 😂 🙂 smile😀 😀grin' },
    ] },
    { op: 'search', query: '😀' },
    { op: 'search', query: '😀', options: { prefix: true } },
    { op: 'search', query: '😀', options: { fuzzy: 1 } },
    { op: 'search', query: '😀', options: { fuzzy: 2 } },
    { op: 'search', query: 'rocket🚀 🚀🚀 👩‍💻 🇺🇳' },
    { op: 'search', query: '𐐀𐐁𐐂' },
    { op: 'search', query: '𐐨', options: { prefix: true } },
    { op: 'search', query: '𝒳𝒴 𝕬𝖑𝖕𝖍𝖆 𠜎𠜱' },
    { op: 'search', query: 'aegean divider ugarit word cune iform' },
    { op: 'search', query: 'smile', options: { prefix: true, fuzzy: 2 } },
    { op: 'autoSuggest', query: '😀' },
    { op: 'toJSON' },
  ],
})

add('lone-surrogates', 'lone and reversed surrogates in documents and queries are ordinary code points: indexed, matched by prefix and fuzzy, returned exactly', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: 'high\ud83dalone low\ude00alone \ud83d \ude00' },
      { id: 2, text: 'reversed\ude00\ud83dpair pair😀ok' },
      { id: 3, text: 'edge\ud800, \udbff.end \udfff! start\ud800' },
      { id: 4, text: 'ΑΣ\ud800 ΑΣ\udc00Β \ud800ΑΣ' },
    ] },
    { op: 'search', query: 'high\ud83dalone' },
    { op: 'search', query: '\ud83d' },
    { op: 'search', query: '\ud83d', options: { prefix: true } },
    { op: 'search', query: '\ude00', options: { fuzzy: 1 } },
    { op: 'search', query: 'pair\ud83d', options: { prefix: true } },
    { op: 'search', query: 'reversed\ude00\ud83dpair' },
    { op: 'search', query: 'edge\ud800 \udbff \udfff start\ud800' },
    { op: 'search', query: 'ας\ud800 ας\udc00β \ud800ας' },
    { op: 'search', query: 'high', options: { prefix: true, fuzzy: 2 } },
    { op: 'autoSuggest', query: 'pair' },
    { op: 'toJSON' },
  ],
})

add('case-mapping', 'toLowerCase: final sigma in its contexts, İ to i plus combining dot (one code unit to two), ẞ, Ǆ/ǅ, Cherokee, Georgian, Kelvin and Ohm signs', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: 'ΟΔΟΣ ΣΑΣ Σ ΑΣ1 ΑΣ́ ΑΣ­Α ἈΣ ΜΆΣΚΑ' },
      { id: 2, text: 'İSTANBUL İzmir ISPARTA ıspanak DİYARBAKIR' },
      { id: 3, text: 'GROẞ Straẞe ǄEMAL ǅemal ǆemal Ⅻ Ⓐⓑ' },
      { id: 4, text: 'ᎠᎡᎢ ᏣᎳᎩ ᲐᲑᲒ ᲥᲐᲠᲗᲣᲚᲘ' },
      { id: 5, text: 'Kelvin Ωhm Ångstrom ＦＵＬＬ' },
    ] },
    { op: 'search', query: 'οδος σας σ ας1 ας́ ας­α ἀς μάσκα' },
    { op: 'search', query: 'ΟΔΟΣ ΣΑΣ' },
    { op: 'search', query: 'οδοσ' },
    { op: 'search', query: 'İstanbul' },
    { op: 'search', query: 'i', options: { prefix: true } },
    { op: 'search', query: 'istanbul', options: { fuzzy: 1 } },
    { op: 'search', query: 'ISPANAK ıspanak diyarbakir' },
    { op: 'search', query: 'groß straße ǆemal ⅻ ⓐⓑ' },
    { op: 'search', query: 'ꭰꭱꭲ ᏣᎳᎩ აბგ' },
    { op: 'search', query: 'kelvin ωhm ångstrom ｆｕｌｌ' },
    { op: 'toJSON' },
  ],
})

add('radix-structure', 'radix-tree shape: splits move the new intermediate node last, merges after removal move the merged edge last; prefix walks run newest first, fuzzy walks oldest first', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'add', document: { id: 1, text: 'team test toast' } },
    { op: 'add', document: { id: 2, text: 'tea te t teams' } },
    { op: 'add', document: { id: 3, text: 'tease tested toaster tram' } },
    { op: 'search', query: 't', options: { prefix: true } },
    { op: 'search', query: 'te', options: { prefix: true } },
    { op: 'search', query: 'tea', options: { prefix: true, fuzzy: 1 } },
    { op: 'search', query: 'tes', options: { fuzzy: 2 } },
    { op: 'autoSuggest', query: 'te' },
    { op: 'remove', document: { id: 2, text: 'tea te t teams' } },
    { op: 'search', query: 't', options: { prefix: true } },
    { op: 'search', query: 'tes', options: { fuzzy: 2 } },
    { op: 'add', document: { id: 4, text: 'tea teams te' } },
    { op: 'search', query: 't', options: { prefix: true } },
    { op: 'search', query: 'tea', options: { fuzzy: 2 } },
    { op: 'toJSON' },
  ],
})

add('surrogate-radix', 'emoji sharing a high surrogate split the radix tree inside the pair: a lone high surrogate is a prefix of them, and the iteration order (toJSON) is not a code-point tree\'s', {
  options: { fields: ['text'] },
  operations: [
    { op: 'add', document: { id: 1, text: '😀a xylo' } },
    { op: 'add', document: { id: 2, text: '😁b' } },
    { op: 'add', document: { id: 3, text: '😂c 🙂d' } },
    { op: 'search', query: '😀a', options: { prefix: true, fuzzy: 2 } },
    { op: 'search', query: '\ud83d', options: { prefix: true } },
    { op: 'search', query: '\ud83d', options: { prefix: true, fuzzy: 1 } },
    { op: 'autoSuggest', query: '\ud83d' },
    { op: 'toJSON' },
  ],
})

add('numeric-terms', 'terms that are array indices come first in match and terms, in numeric order (Object.keys); other digit strings keep insertion order', {
  options: { fields: ['text', 'title'], storeFields: ['title'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, title: 'numbers', text: 'zeta 2024 10 2 007 0 4294967294 4294967295 1e3 alpha 99999999999 01' },
      { id: 2, title: 'more', text: 'beta 10 delta 2 ٣ １２ 3.14 -7' },
    ] },
    { op: 'search', query: 'zeta 2024 10 2 007 0 4294967294 4294967295 1e3 alpha 99999999999 01' },
    { op: 'search', query: '1', options: { prefix: true } },
    { op: 'search', query: 'beta 10 delta 2 ٣ １２ 3 14 7' },
    { op: 'search', query: '2', options: { fuzzy: 1 } },
    { op: 'autoSuggest', query: '10 2' },
    { op: 'autoSuggest', query: 'beta 1' },
    { op: 'toJSON' },
  ],
})

add('object-property-terms', 'terms named like Object.prototype members (constructor, hasownproperty, tostring, proto) are ordinary terms everywhere', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: 'constructor prototype __proto__ toString' },
      { id: 2, text: 'hasOwnProperty valueOf constructor isPrototypeOf' },
      { id: 3, text: 'propertyIsEnumerable toLocaleString __defineGetter__ length' },
    ] },
    { op: 'search', query: 'constructor' },
    { op: 'search', query: 'constructor prototype proto tostring hasownproperty valueof' },
    { op: 'search', query: 'proto', options: { prefix: true } },
    { op: 'search', query: 'constructer', options: { fuzzy: 1 } },
    { op: 'autoSuggest', query: 'constructor pro' },
    { op: 'toJSON' },
  ],
})

add('score-ties', 'equal scores keep insertion order (a stable sort): identical documents, an AND, a prefix match and a wildcard (its autoSuggest makes a single group; x-suggest-score-ties covers tied suggestions)', {
  options: { fields: ['text'], storeFields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 'c', text: 'same words here' },
      { id: 'a', text: 'same words here' },
      { id: 'b', text: 'same words here' },
      { id: 'd', text: 'here words same' },
      { id: 'e', text: 'different' },
    ] },
    { op: 'search', query: 'same' },
    { op: 'search', query: 'here same', options: { combineWith: 'AND' } },
    { op: 'search', query: 'sam', options: { prefix: true } },
    { op: 'autoSuggest', query: 'wor' },
    { op: 'search', query: { wildcard: true } },
  ],
})

add('long-terms-fuzzy', 'long terms: fuzzy 0.2 capped at maxFuzzy 6, integer fuzzy beyond maxFuzzy, prefix weights of long extensions', {
  options: { fields: ['text'] },
  operations: [
    { op: 'addAll', documents: [
      { id: 1, text: `${'abcdefghij'.repeat(12)} short` },
      { id: 2, text: `${'abcdefghij'.repeat(11)}abcdefghiX ${'abcdefghij'.repeat(6)}` },
      { id: 3, text: `${'abcdefghij'.repeat(12)}klmnopqrstuvwxyz` },
    ] },
    { op: 'search', query: 'abcdefghij'.repeat(12), options: { fuzzy: 0.2 } },
    { op: 'search', query: 'abcdefghij'.repeat(12).slice(0, 117), options: { fuzzy: 8 } },
    { op: 'search', query: 'abcdefghij'.repeat(6), options: { prefix: true } },
    { op: 'search', query: 'abcdefghij'.repeat(6).slice(0, 55), options: { fuzzy: 0.2, maxFuzzy: 10 } },
  ],
})

add('field-lengths', 'documents missing fields or holding null or empty ones: field lengths count unique raw tokens, averages use the document count', {
  options: { fields: ['a', 'b', 'c'], storeFields: ['b'] },
  operations: [
    { op: 'add', document: { id: 1, c: 'only the third field' } },
    { op: 'add', document: { id: 2, a: 'first', b: null, c: '' } },
    { op: 'add', document: { id: 3, a: 'first and first again', b: 'second second' } },
    { op: 'add', document: { id: 4 } },
    { op: 'add', document: { id: 5, a: ', first,', b: 'Second SECOND second', c: 'third' } },
    { op: 'toJSON' },
    { op: 'search', query: 'first second third field' },
    { op: 'discard', id: 3 },
    { op: 'toJSON' },
    { op: 'search', query: 'first second third field' },
    { op: 'remove', document: { id: 5, a: ', first,', b: 'Second SECOND second', c: 'third' } },
    { op: 'remove', document: { id: 1, c: 'only the third field' } },
    { op: 'toJSON' },
  ],
})

add('has-and-stored', 'has and getStoredFields across add, discard, replace and remove', {
  options: { fields: ['title'], storeFields: ['title', 'year'] },
  operations: [
    { op: 'addAll', documents: BOOKS.slice(0, 4) },
    { op: 'has', id: 2 },
    { op: 'getStoredFields', id: 2 },
    { op: 'discard', id: 2 },
    { op: 'has', id: 2 },
    { op: 'getStoredFields', id: 2 },
    { op: 'replace', document: { id: 3, title: 'Count Zero', year: 1986 } },
    { op: 'getStoredFields', id: 3 },
    { op: 'remove', document: book(1) },
    { op: 'has', id: 1 },
    { op: 'getStoredFields', id: 1 },
    { op: 'has', id: 4 },
  ],
})

// ---------------------------------------------------------------------------------------------
// Coverage cases from the independent verification (build/library-matrix-verify-minisearch/extra/
// generate-extra.mjs, adopted verbatim). Each catches a plausible wrong port that the cases above
// let through (mutants/mutants.mjs there): autoSuggest tie order, AND's operand order, a stored
// null against an absent field in a filter, averageFieldLength holes, a constructor boostDocument
// on a wildcard, a vacuum boundary on the last term, the field-posting-missing warning, autoSuggest's
// prefix over a constructor prefix, and a prefix walk across a merge.

add('x-suggest-score-ties', 'autoSuggest: equal suggestion scores keep hit order (stable sort, no tie-break by text)', {
  options: { fields: ['text'] },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'abx' }, { id: 2, text: 'aby' }, { id: 3, text: 'abz' }, { id: 4, text: 'abw cd' }] },
    { op: 'autoSuggest', query: 'ab' },
    { op: 'autoSuggest', query: 'ab', options: { fuzzy: 1 } },
    { op: 'search', query: 'ab', options: { prefix: true } },
  ],
})

add('x-and-order-ties', 'AND builds the combined map in the right operand\'s order: visible through tied scores', {
  options: { fields: ['title', 'text'] },
  operations: [
    { op: 'addAll', documents: [{ id: 1, title: 'p q', text: 'x y' }, { id: 2, title: 'x q', text: 'p y' }] },
    { op: 'search', query: 'x y', options: { combineWith: 'AND' } },
    { op: 'search', query: 'x y' },
    { op: 'search', query: { combineWith: 'AND', queries: ['x', 'y'] } },
    { op: 'search', query: { combineWith: 'AND', queries: ['y', 'x'] } },
  ],
})

add('x-filter-null-vs-absent', 'filter in: [null] matches a stored null but not an absent stored field', {
  options: { fields: ['text'], storeFields: ['note'] },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'a', note: null }, { id: 2, text: 'a' }, { id: 3, text: 'a', note: 'x' }] },
    { op: 'search', query: 'a', options: { filter: { field: 'note', in: [null] } } },
    { op: 'search', query: { wildcard: true }, options: { filter: { field: 'note', in: [null, 'x'] } } },
    { op: 'getStoredFields', id: 2 },
  ],
})

add('x-average-field-length-holes', 'averageFieldLength holes are null in toJSON; a hole read by removeFieldLength is NaN (null), and `|| 0` restarts it', {
  options: { fields: ['a', 'b', 'c'] },
  operations: [
    { op: 'add', document: { id: 1, c: 'one two' } },
    { op: 'add', document: { id: 2, c: 'three' } },
    { op: 'toJSON' },
    { op: 'remove', document: { id: 1, a: 'extra', c: 'one two' } },
    { op: 'toJSON' },
    { op: 'add', document: { id: 3, a: 'alpha beta', b: 'gamma' } },
    { op: 'search', query: 'alpha gamma three' },
    { op: 'toJSON' },
  ],
})

add('x-wildcard-constructor-boost', 'a constructor boostDocument scores and sorts a top-level wildcard; a call boostDocument replaces it', {
  options: { fields: ['text'], storeFields: ['text'], searchOptions: { boostDocument: [[2, 3], [3, 0], [1, 0.5]] } },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'a b' }, { id: 2, text: 'a c' }, { id: 3, text: 'a d' }, { id: 4, text: 'a e' }] },
    { op: 'search', query: { wildcard: true } },
    { op: 'search', query: { wildcard: true }, options: { boostDocument: [[4, 2]] } },
    { op: 'search', query: 'a' },
    { op: 'search', query: { queries: [{ wildcard: true }] } },
  ],
})

add('x-vacuum-boundary-on-last-term', 'a vacuum whose first batch boundary falls on the last term has visited every term but has not finished: dirtCount kept', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'aa bb' }, { id: 2, text: 'cc' }, { id: 3, text: 'dd' }] },
    { op: 'discard', id: 1 },
    { op: 'stats' },
    { op: 'vacuum', options: { batchSize: 4 } },
    { op: 'stats' },
    { op: 'toJSON' },
  ],
})

add('x-remove-term-field-mismatch', 'remove() of a changed document whose term exists in the index but not in that field for it: version_conflict warning, postings kept', {
  options: { fields: ['title', 'text'] },
  operations: [
    { op: 'addAll', documents: [{ id: 1, title: 'alpha', text: 'beta' }, { id: 2, title: 'beta', text: 'alpha' }] },
    { op: 'remove', document: { id: 1, title: 'beta', text: 'alpha' } },
    { op: 'stats' },
    { op: 'toJSON' },
    { op: 'search', query: 'alpha beta' },
  ],
})

add('x-autosuggest-constructor-prefix', 'autoSuggest\'s default prefix function overrides a constructor searchOptions.prefix: true', {
  options: { fields: ['text'], searchOptions: { prefix: true } },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'zen zenith art' }, { id: 2, text: 'zen artist' }, { id: 3, text: 'zenith art' }] },
    { op: 'autoSuggest', query: 'zen ar' },
    { op: 'search', query: 'zen ar' },
  ],
})

add('x-prefix-walk-merge', 'a prefix walk deletes a discarded term whose node then merges into its parent; the walk still visits the merged child (snapshot semantics), in search and in vacuum', {
  options: { fields: ['text'], autoVacuum: false },
  operations: [
    { op: 'addAll', documents: [{ id: 1, text: 'teams teamster' }, { id: 2, text: 'team' }, { id: 3, text: 'teams' }, { id: 4, text: 'teal' }] },
    { op: 'discard', id: 2 },
    { op: 'search', query: 'tea', options: { prefix: true } },
    { op: 'toJSON' },
    { op: 'discard', id: 4 },
    { op: 'vacuum' },
    { op: 'search', query: 'te', options: { prefix: true } },
    { op: 'toJSON' },
  ],
})

// ---------------------------------------------------------------------------------------------
// Bench cases: realistic sizes, also part of the parity corpus.

{
  const { documents } = benchDocuments(0x5eed1, 1200)
  const queries = ['lorem', 'tha', 'brai', 'cou', 'nd']
  add('bench-index', 'bench: index 1,200 generated documents (title, text, category, year; 2.5% non-ASCII words), then five searches', {
    options: { fields: ['title', 'text', 'category'], storeFields: ['title', 'category', 'year'] },
    operations: [
      { op: 'addAll', documents },
      { op: 'stats' },
      ...queries.map((query) => ({ op: 'search', query, options: { prefix: true } })),
    ],
  })
}

{
  const { documents, words, random } = benchDocuments(0xbe7c4, 400, { textSentences: [1, 3] })
  const common = words.slice(0, 400)
  const pickWord = () => common[Math.floor(random() ** 1.6 * common.length)]
  const operations = [{ op: 'addAll', documents }]
  for (let index = 0; index < 120; index++) {
    const kind = index % 6
    const one = pickWord()
    const two = pickWord()
    if (kind === 0) operations.push({ op: 'search', query: `${one} ${two}` })
    else if (kind === 1) operations.push({ op: 'search', query: one.slice(0, Math.max(2, one.length - 2)), options: { prefix: true } })
    else if (kind === 2) operations.push({ op: 'search', query: `${one}${'xq'[index % 2]}`, options: { fuzzy: 0.2 } })
    else if (kind === 3) operations.push({ op: 'search', query: `${one} ${two}`, options: { combineWith: 'AND', prefix: true, fuzzy: 1 } })
    else if (kind === 4) operations.push({ op: 'search', query: { queries: [one, { queries: [two, pickWord()], combineWith: 'OR', prefix: true }], combineWith: 'AND_NOT' }, options: { boost: { title: 2 } } })
    else operations.push({ op: 'autoSuggest', query: `${one} ${two.slice(0, 2)}` })
  }
  add('bench-search', 'bench: index 400 generated documents, then 120 queries (OR, prefix, fuzzy, AND with prefix and fuzzy, query trees, autoSuggest)', {
    options: { fields: ['title', 'text'], storeFields: ['title', 'category'] },
    operations,
  })
}

const corpus = { schema: 1, library: 'minisearch', cases }
const path = resolve(here, 'corpus.json')
writeFileSync(path, `${JSON.stringify(corpus, null, 2)}\n`)
process.stdout.write(`wrote ${path}: ${cases.length} cases\n`)

// The default tokenizer's separator set as code point ranges, read off the host RegExp itself, for
// ports that classify natively (CONTRACT.md, "Tokenizer"). Lone surrogates are never separators.
{
  const separator = /^[\n\r\p{Z}\p{P}]$/u
  const ranges = []
  for (let point = 0; point <= 0x10ffff; point++) {
    if (point >= 0xd800 && point <= 0xdfff) {
      if (separator.test(String.fromCharCode(point))) throw new Error(`surrogate ${point.toString(16)} is a separator`)
      continue
    }
    if (!separator.test(String.fromCodePoint(point))) continue
    const last = ranges[ranges.length - 1]
    if (last && last[1] === point - 1) last[1] = point
    else ranges.push([point, point])
  }
  const table = {
    description: 'Code points matching /[\\n\\r\\p{Z}\\p{P}]/u (MiniSearch SPACE_OR_PUNCTUATION), as inclusive [first, last] ranges',
    unicode: process.versions.unicode,
    icu: process.versions.icu,
    node: process.version,
    codePoints: ranges.reduce((sum, [first, last]) => sum + last - first + 1, 0),
    ranges,
  }
  const tablePath = resolve(here, 'separators.json')
  writeFileSync(tablePath, `${JSON.stringify(table)}\n`)
  process.stdout.write(`wrote ${tablePath}: ${ranges.length} ranges, ${table.codePoints} code points (Unicode ${table.unicode})\n`)
}

if (process.argv.includes('--verify')) {
  const { run } = await import('./reference.mjs')
  const allowed = /^(MiniSearch: |Invalid combination operator: |Expected documents to be present\. )/
  let failures = 0
  const visit = (value, path, onNumber) => {
    if (typeof value === 'number') onNumber(value, path)
    else if (Array.isArray(value)) value.forEach((item, index) => visit(item, `${path}[${index}]`, onNumber))
    else if (value && typeof value === 'object') for (const key of Object.keys(value)) visit(value[key], `${path}.${key}`, onNumber)
  }
  for (const item of cases) {
    const output = run(structuredClone(item.input))
    output.results.forEach((result, index) => {
      if (result.error !== undefined && !allowed.test(result.error)) {
        failures++
        process.stdout.write(`${item.id}: operation ${index} threw a non-MiniSearch error: ${result.error}\n`)
      }
      visit(result, `${item.id}[${index}]`, (number, at) => {
        if (!Number.isFinite(number)) {
          failures++
          process.stdout.write(`${at}: non-finite number ${number}\n`)
        }
      })
    })
    if (item.id.startsWith('bench-')) {
      const start = performance.now()
      let calls = 0
      while (performance.now() - start < 1000) {
        run(structuredClone(item.input))
        calls++
      }
      process.stdout.write(`${item.id}: ${((performance.now() - start) / calls).toFixed(2)} ms per call (${calls} calls, input ${JSON.stringify(item.input).length} bytes)\n`)
    }
  }
  if (failures) process.exit(1)
}
