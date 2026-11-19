# Library contract: `minisearch`

MiniSearch is an in-memory full-text search engine: a radix-tree inverted index with BM25+
scoring, prefix and fuzzy (Levenshtein) matching, query trees, auto-suggestion and document
discarding with background vacuuming. Ports implement the same algorithms and data structures (the
radix tree with its prefix and fuzzy walks, BM25+ in the same operation order, the same result
assembly) and must reproduce its output exactly, float scores included.

| Field | Value |
| --- | --- |
| Package | `minisearch@7.2.0`, entry `dist/es/index.js` (root `node_modules`) |
| Reference adapter | `reference.mjs`: one `new MiniSearch(options)` per call, then the operation script; see "Input" and "Output" |
| API subset | constructor (`fields`, `storeFields`, `idField`, `searchOptions`, `autoSuggestOptions`, `autoVacuum`); `add`, `addAll`, `remove`, `removeAll`, `discard`, `discardAll`, `replace`, `vacuum`, `search` (string, wildcard, query trees), `autoSuggest`, `has`, `getStoredFields`, `documentCount`, `termCount`, `dirtCount`, `dirtFactor`, `isVacuuming`, `toJSON`. Left out: `addAllAsync`, `loadJSON`/`loadJS`/`loadJSONAsync`, custom `tokenize`/`processTerm`/`extractField`/`stringifyField`, function-valued `prefix`/`fuzzy` (autoSuggest's default excepted) |
| Input | `{ options, operations }`, below |
| Output form | `json` (key order significant), synchronous: `{ results: [...] }`, one record per operation |
| Normalisation | none |
| Parity environments | node, chrome, firefox, safari (unavailable on this machine) |
| Corpus | `corpus.json`, 58 cases, written by `generate-corpus.mjs` (deterministic; `--verify` checks the domain) |
| Bench cases | `bench-index`, `bench-search`; `baseIterations` 1; variant `bench-variant.mjs` (a k-seeded ASCII letter permutation of every document string and query, ids + k × 10,000) |
| Shared data | `separators.json`: the tokenizer's separator set as code-point ranges, read off the host `RegExp` by the generator |

## Why this subset

It is what applications use MiniSearch for: index documents with stored fields, search with field
boosts, prefix and fuzzy matching, AND/OR/AND_NOT and nested query trees, filter results, suggest
completions, and keep the index current with `remove`, `discard` and `replace`. Every option of
`search` is in, except the callbacks a JSON corpus cannot carry; the three useful ones (`filter`,
`boostDocument`, `boostTerm`) are in through a small declarative form. Discarding is in with its
vacuuming, because its effects are visible in scores (`matchingFields`), in later iteration
orders (radix merges) and in `termCount`. `toJSON` is in as a probe of the whole internal state
(average field lengths, posting lists, tree order). Left out:

- the async entry points (`addAllAsync`, the batches of a vacuum after its first await, the
  promise `vacuum()` returns): a synchronous `run` cannot observe them (see "Vacuuming");
- deserialization (`loadJSON`, `loadJS`): a second, larger code path whose JSON shapes
  (`fieldLength` arrays with `null`, `parseInt` keys) change later behaviour; one library at a time;
- custom tokenizers and term processors: functions, and they would replace exactly the part (the
  default tokenizer and `toLowerCase`) that is hardest to port.

## Input

```text
{
  "options": {
    "fields":       [name, ...],                  required; field ids are positions in this list
    "storeFields":  [name, ...],                  optional, default []
    "idField":      name,                         optional, default "id"
    "searchOptions":      SearchOptions,          optional: defaults for every search
    "autoSuggestOptions": SearchOptions,          optional: defaults for every autoSuggest
    "autoVacuum":   true | false | { "batchSize"?, "minDirtCount"?, "minDirtFactor"? }   optional, default true
  },
  "operations": [ Operation, ... ]
}

Operation :=
  { "op": "add",         "document": Document }
  { "op": "addAll",      "documents": [Document, ...] }
  { "op": "remove",      "document": Document }
  { "op": "removeAll",   "documents": [Document, ...] }      removeAll(documents)
  { "op": "removeAll" }                                       removeAll()  (no argument)
  { "op": "removeAll",   "documents": null }                  removeAll(null): an error (see "Errors")
  { "op": "discard",     "id": Id }
  { "op": "discardAll",  "ids": [Id, ...] }
  { "op": "replace",     "document": Document }
  { "op": "vacuum",      "options"?: { "batchSize"? } }
  { "op": "search",      "query": Query, "options"?: SearchOptions }
  { "op": "autoSuggest", "query": string, "options"?: SearchOptions }
  { "op": "has",         "id": Id }
  { "op": "getStoredFields", "id": Id }
  { "op": "stats" }
  { "op": "toJSON" }

Query := string | { "wildcard": true } | { "queries": [Query, ...], ...SearchOptions }

SearchOptions (every key optional; a key present replaces the inherited value whole):
  "fields": [name, ...]          "boost": { name: number }      "prefix": boolean
  "fuzzy": boolean | number      "maxFuzzy": integer            "combineWith": string
  "weights": { "fuzzy"?: number, "prefix"?: number }            "bm25": { "k", "b", "d" }
  "filter":        { "field": name, "in": [scalar, ...] }   ->  (result) => values.includes(result[field])
  "boostDocument": [[Id, number], ...]                      ->  (id) => factor of the first pair whose id === id, else 1
  "boostTerm":     [[string, number], ...]                  ->  (term) => factor of the first pair whose term === term, else 1

Document := a JSON object; the values MiniSearch reads (idField, fields, storeFields) are
            strings, numbers (safe integers), null, or absent
Id       := a string or an integer
```

The adapter is the reference for this mapping: `wildcard` is `MiniSearch.wildcard` (top level or a
subquery), the three declarative options become the callbacks shown, and the adapter passes a
`logger` that records MiniSearch's warning messages. Option values are never `null`.

## Output

`{ "results": [record, ...] }`, one record per operation, keys in this order:

- `"op"`: the operation name;
- the payload, only when the operation returned:
  - `search`: `"hits": [hit, ...]`; a hit is MiniSearch's result object with `match` given as
    pairs: `{ "id", "score", "terms", "queryTerms", "match": [[term, [field, ...]], ...] }`
    followed by the stored fields as `Object.assign(result, storedFields)` adds them (a stored
    field named like an existing key replaces its value in place, which only `id` can be here);
  - `autoSuggest`: `"suggestions": [{ "suggestion", "terms", "score" }, ...]`;
  - `has`: `"value": boolean`; `getStoredFields`: `"value": { name: value, ... }` or `null`;
  - `stats`: `"documentCount"`, `"termCount"`, `"dirtCount"`, `"dirtFactor"`, `"isVacuuming"`;
  - `toJSON`: `"value": JSON.parse(JSON.stringify(index))` (holes and non-finite numbers are `null`);
- `"warnings": [message, ...]` when MiniSearch logged during the operation;
- `"error": message` when MiniSearch threw; the script then goes on with the index as MiniSearch
  left it (an `addAll` that fails part way keeps the documents added before the failure).

The other operations have no payload. Scores and `dirtFactor` are doubles and compared exactly.

## Behaviour

Everything below is MiniSearch 7.2.0's behaviour, restated so a port can be written from it; the
source (`node_modules/minisearch/dist/es/index.js`) is the authority where this is silent.

### Strings are UTF-16

Every string the index handles is a sequence of **UTF-16 code units**, as JavaScript strings are:
the radix tree compares and splits keys on code units (an edge can end between the two halves of a
surrogate pair), `term.length` in the prefix and fuzzy weights is in code units, the Levenshtein
distance counts code units (`😀` and `😁` are at distance 1, `😀` and `a` at distance 2), and
`Math.round(term.length * fuzzy)` uses the code-unit length. Lone surrogates are ordinary code
units and must survive input, indexing and output unchanged (the corpus has them).

**Languages whose strings are UTF-8 (Rust, Go through GopherJS, OCaml through js_of_ocaml) must
index UTF-16**: read the host string's code units (`charCodeAt`, or keep the host string) into a
`u16` sequence (Rust `Vec<u16>`, Go `[]uint16`, an OCaml int array or the JS string itself) and use
those as keys, labels and lengths. Converting to UTF-8 is wrong twice: lone surrogates are lost
(U+FFFD, or GopherJS's known boundary defect) and the tree's shape, the prefix weights and the
fuzzy distances of every non-ASCII term change.

### Tokenizer

`tokenize(text) = text.split(/[\n\r\p{Z}\p{P}]+/u)`. Decoded as code points (a well-formed pair
is one code point, a lone surrogate is one code point of category Cs), the text is cut at every
maximal run of **separators**: U+000A, U+000D, and the General_Category values Zs, Zl, Zp, Pc,
Pd, Ps, Pe, Pi, Pf, Po. Like `split`, the empty text is one empty token `[""]`, and a separator at
the start or end leaves an empty token there. Not separators, and therefore part of tokens:
TAB and every other control (Cc), the symbols `$ + < = > ^ \` | ~` and every S* character
(emoji included), format characters (soft hyphen, ZWSP, ZWJ, BOM), and lone surrogates.

A port **may call the host** (`String.prototype.split` with that `RegExp`, the `u` flag
required) or **classify natively**. A native table must agree with `separators.json` on the parity
domain. That file holds 877 code points in 203 ranges, read off Node 26.8.1 / ICU 78.3 / Unicode 17.0
by the generator. For ASCII alone it is 0x0A, 0x0D, 0x20–0x23, 0x25–0x2A, 0x2C–0x2F, 0x3A–0x3B,
0x3F–0x40, 0x5B–0x5D, 0x5F, 0x7B, 0x7D.

A language's standard Unicode tables of version 15.0 or later also qualify, such as Go 1.21's
`unicode.Is(unicode.Z/P, r)` (Unicode 15.0). They differ from `separators.json` only at 14 code points
that are new in Unicode 16 / 17, and those code points are **outside the parity domain**: no corpus
string contains them, and the bench variant only permutes ASCII letters. The 14 are U+1B4E, U+1B4F,
U+1B7F, U+10D6E, U+10ED0, U+113D4, U+113D5, U+113D7, U+113D8, U+11BE1, U+16D6D–U+16D6F, U+1E5FF. A
future corpus that adds one of them must re-open this rule. Every other corpus character classifies
and lower-cases the same from Unicode 13.0 to 17.0 (checked by the MiniSearch verifier).

The **field length** of a field (BM25's `fieldLength`) is the number of **distinct raw tokens**
(`new Set(tokens).size`): before lower-casing, case-sensitive, the empty token counted once.
`"Word word WORD"` has length 3 and gives the term `word` frequency 3; `", hello."` has tokens
`["", "hello", ""]` and length 2.

### `processTerm`: `toLowerCase`

Each token becomes `token.toLowerCase()`; an empty result is dropped (not indexed, not a query
term). This is Unicode default lower-casing with full mappings and the one context-sensitive rule,
Final_Sigma; no locale. The corpus exercises: `İ` (U+0130) becoming `i` + U+0307 (one code unit
becomes two, which changes term length and the tree), Σ as `ς` at the end of a word (`ΟΔΟΣ` to
`οδος`, `ΑΣ1`, `ΑΣ` + U+0301, `ΑΣ` before a lone surrogate) and as `σ` elsewhere (`Σ` alone,
`ΣΑΣ` → `σας`, `ΑΣ` U+00AD `Α`), `ẞ`, `Ǆ`/`ǅ`, Roman numerals, circled letters, Cherokee,
Georgian Mtavruli, the Kelvin and Ohm signs, fullwidth letters and Deseret.

A port may call the host `toLowerCase` (what most of these languages' standard lower-casing
compiles to) or lower-case natively with full Unicode mappings **and** Final_Sigma. Rust's
`str::to_lowercase` qualifies; a lone surrogate is neither cased nor case-ignorable, so applying it
to each well-formed run between lone surrogates is exact. Go's `strings.ToLower` and ASCII-only
lower-casing do not qualify. Same Unicode-version caveat as the tokenizer.

### Documents and ids

`extractField(document, name)` is `document[name]`. A field value `null` or absent is skipped
(not tokenized, no field length). A string is indexed as is; a number as `fieldValue.toString()`
(inside the domain: safe integers, so plain decimal digits, `-` for negatives, which the tokenizer
then splits off).

The id is `document[idField]`; `null` or absent is an error. Ids are keys of a `Map`, so they
compare by SameValueZero: `1` and `"1"` are different documents. Every document gets the next
**short id** (0, 1, 2, ...; `removeAll()` restarts at 0); short ids are never reused, and every
internal map keyed by short id therefore has its keys in ascending order.

Stored fields: when `storeFields` is non-empty, `add` stores `{ name: value }` for each stored
name whose value is not `undefined` (`null` is stored), in `storeFields` order. With no
`storeFields` nothing is stored and `getStoredFields` returns `undefined` (`null` in the output)
even for a present document.

### The index: `SearchableMap`

A radix tree. A **node** is an insertion-ordered map (a JavaScript `Map`) from keys to values: a
non-empty key is an **edge label** leading to a child node; the empty key `''` (`LEAF`) holds the
value of the term ending at this node. The value of a term is its `fieldsData`: field id to
posting list, a posting list being short id to term frequency. No two edges of a node share their
first code unit. **The insertion order of a node's keys, the leaf's position among them included,
is observable** and every operation below must keep it exactly as a JavaScript `Map` would:
setting a new key appends it, deleting removes it in place, setting an existing key keeps its
place.

- `fetch(term)`: `createPath`, then the node's leaf value, created (appended as `''`) if absent.
  `createPath(node, key)`: at each node, find the edge whose first unit is `key[pos]`; let `offset`
  be the length of the common prefix of the edge and the rest of the key. If the whole edge
  matches, descend. Otherwise **split**: a new intermediate node holding one edge (the edge's rest
  to its old child) is set under the shared part and the old edge deleted, so **the intermediate
  node moves to the end of its parent's keys**; continue from it. With no matching edge, append an
  edge holding the rest of the key to a new empty node.
- `get`/`has(term)`: walk edges that are prefixes of the rest of the key; the term exists when the
  walk consumes the key exactly at a node that has a leaf.
- `delete(term)` (`remove`): track down to the term's node, delete its leaf, then
  - if the node is now empty, **cleanup**: the parent deletes the edge to it; if the parent is now
    empty, cleanup continues upward; if the parent now holds exactly one entry and it is an edge,
    **merge** the parent into the grandparent;
  - if the node now holds exactly one entry (an edge), **merge** it into its parent.
  Merging node N, reached from P by label `k`, whose only entry is edge `k2` to child C: P sets
  `k + k2` to C and deletes `k`, so **the merged edge moves to the end of P's keys**. Neither step
  happens at the root.
- `atPrefix(prefix)`: track down; if the prefix ends at a node, that node; if it ends inside an
  edge whose label starts with the rest, a one-entry view `{ restOfLabel: child }`; otherwise
  empty.
- **Iteration** (`entries()`, `TreeIterator`, used by prefix search, vacuuming, `termCount` and
  `toJSON`): depth first from the view's node, **each node's keys from the most recently inserted
  to the oldest** (the iterator pops a snapshot of the keys from its end); a leaf yields the term
  spelled by the labels on the path.
- **Fuzzy** (`fuzzyGet(query, maxDistance)`): a depth-first walk over each node's keys **oldest
  first**, keeping a Levenshtein matrix of `(query.length + 1)` columns and
  `query.length + 1 + maxDistance` rows in a `Uint8Array` (every store reduced modulo 256)
  initialised to `maxDistance + 1`, with row 0 = 0, 1, 2, … and column 0 = 0, 1, 2, …. Each label
  unit fills the next row over the band `j` from `max(0, i - maxDistance - 1)` to
  `min(n - 1, i + maxDistance)` (exclusive), cell `(i, j+1)` = min(`(i-1, j)` + different,
  `(i-1, j+1)` + 1, `(i, j)` + 1); when the row's minimum (starting from its column-0 value)
  exceeds `maxDistance` the subtree is skipped. At a leaf, the last cell of the current row is the
  distance; the term is a match when it is `<= maxDistance`. Matches are returned in the order the
  walk meets them. Inside the parity domain no value reaches 256, and rows past the matrix (keys
  longer than `query.length + maxDistance`) never match, so a port may stop there.

Terms are only ever deleted right after they were visited, so a walk interleaved with deletions
visits exactly the terms of the tree as it was when the walk began (the iterator works on key
snapshots). A port may therefore collect a walk's terms first, provided each term's own changes
are applied in walk order.

### Adding, removing, discarding

- `add(document)`: error if no id or a duplicate id; take the next short id; store the stored
  fields; then for each field in `fields` order with a non-null value: tokenize, record the field
  length (`addFieldLength` with `count = documentCount - 1`), and add each non-empty processed
  term: posting `(field, shortId)` frequency + 1, created as 1 if absent (`addTerm`).
- `addAll(documents)`: `add` each in order; the first error ends it.
- `remove(document)`: error if no id or the id is not present. For each field with a non-null
  value of **the document as given** (it must not have changed since it was added, but nothing
  checks): tokenize, `removeFieldLength(count = documentCount)`, and `removeTerm` each non-empty
  processed term; then delete the stored fields, the id and the field lengths and decrement
  `documentCount`.
- `removeTerm(field, shortId, term)`: if the term is not in the index, warn and stop. If the field
  has no posting for the short id, warn. Else if its frequency is `<= 1`: delete the whole field
  from the term when its posting list holds one entry, else delete the short id; otherwise
  decrement the frequency. Finally, delete the term from the tree when it has no field left.
  The warning (logger level `warn`, code `version_conflict`) is
  `MiniSearch: document with ID ${id} has changed before removal: term "${term}" was not present in field "${fieldName}". Removing a document after it has changed can corrupt the index!`
  with `${id}` the id of that short id (`undefined` when there is none).
- `removeAll(documents)`: `remove` each in order. `removeAll()`: a new empty index, document
  count 0, empty id maps, field lengths, averages (`[]`) and stored fields, next short id 0;
  **`dirtCount` and the vacuum state are kept**.
- `discard(id)`: error if not present; delete the id, its short id from `_documentIds`, its stored
  fields; for each field length the document has (holes skipped), `removeFieldLength` with
  `count = documentCount`; delete its field lengths; decrement `documentCount`, increment
  `dirtCount`; then the automatic vacuum check. Its postings stay in the index.
- `discardAll(ids)`: `discard` each with automatic vacuuming off, then one automatic vacuum check;
  an error ends it before that check.
- `replace(document)`: `discard(document[idField])` (which may vacuum), then `add(document)`.
- `has(id)`, `getStoredFields(id)`: lookups through the id map.

Field lengths and their averages, in doubles, exactly in this order:

```text
addFieldLength(shortId, fieldId, count, length):
  fieldLength[shortId][fieldId] = length             (an array with holes for absent fields)
  average = avgFieldLength[fieldId] || 0
  avgFieldLength[fieldId] = (average * count + length) / (count + 1)
removeFieldLength(shortId, fieldId, count, length):
  if count === 1: avgFieldLength[fieldId] = 0; return
  avgFieldLength[fieldId] = (avgFieldLength[fieldId] * count - length) / (count - 1)
```

The average treats every document as having every field (a document without the field still
counts in `count`), and `avgFieldLength` is an array whose length is one past the highest field id
ever set (holes are `null` in `toJSON`).

### Vacuuming in a synchronous run

`performVacuuming` is an async function: it runs synchronously until its first `await`, and the
rest (and every promise callback) runs after `run` has returned, which nothing observes. So, inside
one synchronous run:

- **conditional vacuum** (`conditionalVacuum(options, conditions)`): if a vacuum is current (see
  below), the request is chained behind it, which in a synchronous run means **never**. Else, if
  conditions are given and not met, nothing. Else the vacuum becomes current and runs now.
- **conditions met**: `dirtCount >= (minDirtCount || 20) && dirtFactor >= (minDirtFactor || 0.1)`,
  where `dirtFactor = dirtCount / (1 + documentCount + dirtCount)`. A zero condition means the
  default.
- **automatic check** (after `discard`, once after `discardAll`): when `autoVacuum` is not
  `false`, a conditional vacuum with `batchSize` and the conditions from `autoVacuum` (`true` or
  absent: the defaults batchSize 1000, minDirtCount 20, minDirtFactor 0.1; an object is used as
  given, missing values falling back through `||`).
- **`vacuum(options)`**: a conditional vacuum without conditions.
- **the vacuum**: remember `dirtCount`; walk the index terms in iteration order; for each term,
  remove every posting whose short id is no longer present (a field whose posting list empties is
  removed, a term with no field left is deleted from the tree, with the merges that causes); after
  the i-th term (1-based), if `i % (batchSize || 1000) === 0`, **stop**: the rest happens after a
  timer, outside the run. Only if the walk ends without stopping is `dirtCount` reduced by the
  remembered value.
- **current**: once a vacuum has run, MiniSearch clears its "current" mark only in a later
  microtask, so **`isVacuuming` is `true` for the rest of the run and every later vacuum request
  is deferred** (and never observed).

### Search

**Option merging** is object spread at every level: a key present replaces the inherited value
whole (`weights: { prefix: 0.5 }` replaces a constructor `weights: { fuzzy: 0.1 }`, and the
missing `fuzzy` then comes from the library default 0.45, not from the constructor). The library
defaults are `combineWith: "or"`, `prefix: false`, `fuzzy: false`, `maxFuzzy: 6`, `boost: {}`,
`weights: { fuzzy: 0.45, prefix: 0.375 }`, `bm25: { k: 1.2, b: 0.7, d: 0.5 }`; the index's search
defaults are those under the constructor's `searchOptions`.

`search(query, options)`:

1. `executeQuery(query, options)` with the call's options (not merged with the index defaults):
   - wildcard: every present document in short-id order, score
     `boostDocument ? boostDocument(id) : 1` (no skipping of zero), no terms, empty `match`
     (with the index defaults merged in);
   - tree node: `nodeOptions = { ...options, ...node }`; execute each subquery with
     `nodeOptions` (in order, all of them, before combining), then combine with
     `nodeOptions.combineWith`. **Only the call's and the nodes' own options are seen here**, so
     a node without `combineWith` anywhere above it combines with OR even when the constructor's
     `searchOptions.combineWith` is `AND` (`constructor-search-options`);
   - string: `opts = { ...indexDefaults, ...options }`; `terms` = the processed, non-empty tokens
     of the query, duplicates kept; one query spec per term (`term`, `fuzzy = opts.fuzzy || false`,
     `prefix = opts.prefix === true`, or for autoSuggest's default `i === terms.length - 1`,
     `termBoost = boostTerm ? boostTerm(term) : 1`); execute each spec; combine with
     `opts.combineWith`.
2. For each raw result in the combined map's order: `quality = terms.length || 1` (the number of
   distinct matched query terms), `score = score * quality`, `terms = Object.keys(match)`,
   `queryTerms = terms`, then `Object.assign` the stored fields; keep it if there is no
   `filter` in `{ ...indexDefaults, ...options }` or `filter(result)` is true. A filter inside a
   tree node is ignored.
3. Sort by `(a, b) => b.score - a.score`, a **stable** sort: equal scores keep the combined map's
   order. (A top-level wildcard without `boostDocument` is not sorted; every score is 1, so a
   stable sort would not change it.)

**`combineResults(results, combineWith = "or")`**: no results is an empty map, whatever the
operator (so an invalid operator is only an error when there is something to combine). Otherwise
look the operator up in `combineWith.toLowerCase()` (`OR`, `And`, `and_not` all work); an unknown
one throws `Invalid combination operator: ${combineWith}` (the original spelling). Fold from the
left, starting with the first map:

- OR(a, b): for each document of b in b's order: absent from a → append b's entry; present →
  `score = a.score + b.score`, `match = Object.assign(a.match, b.match)`, add b's query terms
  missing from a's. Returns a.
- AND(a, b): a **new map in b's order** of the documents present in both, with a's query terms
  plus b's missing ones, `Object.assign(a.match, b.match)` and `score = a.score + b.score`.
- AND_NOT(a, b): delete from a every document of b.

`Object.assign` of two `match` objects keeps an existing term's position and takes the new
field list (it does not concatenate). Repeated query terms (`art art`) are therefore scored once
per occurrence and counted once for quality.

**`executeQuerySpec(spec, options)`**, with `opts = { ...indexDefaults, ...options }`:

1. `fieldBoosts`: for each field of `opts.fields || fields` (`fields: []` is not falsy: it
   matches no field, so nothing is found; a repeated field keeps its first
   position): `getOwnProperty(opts.boost, field) || 1` (a boost of 0 is 1; negative and
   fractional boosts stand). A field the index does not have never matches.
2. Score the exact term (`termResults` with weight 1).
3. If `spec.fuzzy`: `fuzzy = spec.fuzzy === true ? 0.2 : spec.fuzzy`; `maxDistance = fuzzy < 1 ?
   Math.min(maxFuzzy, Math.round(term.length * fuzzy)) : fuzzy` (`Math.round` rounds halves up: 5 ×
   0.5 is 3; integer fuzzy values are not capped by `maxFuzzy`); if non-zero, `fuzzyGet`.
   This happens **after** step 2's cleanup (below), which may have deleted the exact term and
   merged nodes, changing the fuzzy walk's order (`discard-exact-term-merge`).
4. If `spec.prefix`: for each term of `atPrefix(term)` in iteration order with
   `distance = term.length - query.length` non-zero: remove it from the fuzzy matches, then score
   it with weight `prefixWeight * t.length / (t.length + 0.3 * distance)`.
5. For each remaining fuzzy match in walk order with a non-zero distance: score it with weight
   `fuzzyWeight * t.length / (t.length + distance)`.

**`termResults(sourceTerm, derivedTerm, termWeight, …)`**: for each field of `fieldBoosts` in
order, with that field's posting list of the derived term: `matchingFields = postings.size`;
for each posting in order:

- **a discarded (or removed) document**: `removeTerm(field, shortId, derivedTerm)` (no warning on
  this path) and `matchingFields -= 1`, so **documents scored before it in the same list use the
  higher count**; note that `removeTerm` only **decrements** a frequency above 1, so a discarded
  document with frequency 3 is met by the next two searches too (`discard-cleanup-on-search`);
- `docBoost = boostDocument ? boostDocument(id) : 1`; a falsy `docBoost` (0) skips the document;
- `rawScore = calcBM25Score(termFreq, matchingFields, documentCount, fieldLength, avgFieldLength)`;
- `weighted = termWeight * termBoost * fieldBoost * docBoost * rawScore`, left to right;
- first time for this document: `{ score: weighted, terms: [sourceTerm], match: { [derivedTerm]: [field] } }`
  appended to the map; else `score += weighted`, add `sourceTerm` to its terms if missing, and
  push `field` onto `match[derivedTerm]` (create it if absent).

```text
calcBM25Score(termFreq, matchingCount, totalCount, fieldLength, avgFieldLength, { k, b, d }):
  invDocFreq = Math.log(1 + (totalCount - matchingCount + 0.5) / (matchingCount + 0.5))
  return invDocFreq * (d + termFreq * (k + 1) / (termFreq + k * (1 - b + b * fieldLength / avgFieldLength)))
```

Evaluate it exactly so: `a * b / c` is `(a * b) / c`, `1 - b + x` is `(1 - b) + x`, and no
fused multiply-add.

**`Object.keys(match)` order** (the hit's `terms`, its `match` pairs, and autoSuggest phrases):
keys that are **array indices** (the canonical decimal spelling of an integer from 0 to
2^32 − 2: `"0"`, `"10"`, `"4294967294"`; not `"007"`, `"01"`, `"4294967295"`, `"1e3"`, `"٣"`)
come first **in ascending numeric order**, then every other key in insertion order. A port that
keeps `match` as an insertion-ordered list must apply this order when it reports keys
(`numeric-terms`). Terms named like `Object.prototype` members (`constructor`, `tostring`,
`hasownproperty`) are ordinary keys (MiniSearch uses own-property checks); `__proto__` cannot be
a term (the tokenizer splits on `_`).

### autoSuggest

`autoSuggest(query, options)`: `opts = { ...autoSuggestDefaults, ...options }`, where the
autoSuggest defaults are `{ combineWith: "and", prefix: (term, i, terms) => i === terms.length - 1 }`
under the constructor's `autoSuggestOptions`; run `search(query, opts)`; group the hits by
`terms.join(' ')` (the matched **document** terms, in `Object.keys` order) in hit order, summing
scores and counting; each group becomes `{ suggestion, terms: the first hit's terms, score: sum / count }`;
stable-sort by score descending.

### State and serialization

- `stats`: `documentCount`; `termCount` (the number of terms in the tree); `dirtCount`;
  `dirtFactor`; `isVacuuming`.
- `toJSON` (`JSON.parse(JSON.stringify(index))`): `{ documentCount, nextId, documentIds:
  { shortId: id }, fieldIds: { name: fieldId }, fieldLength: { shortId: [length | null, ...] },
  averageFieldLength: [average | null, ...], storedFields: { shortId: { name: value } }, dirtCount,
  index: [[term, { fieldId: { shortId: frequency } }], ...], serializationVersion: 2 }`, with
  `index` in iteration order and every integer-keyed object in ascending key order (they are
  array-index keys).

### Errors

Exact messages, `${id}` written as a template literal writes it (an integer in decimal, a string
as itself):

| Message | When |
| --- | --- |
| `MiniSearch: document does not have ID field "${idField}"` | `add`, `remove` of a document whose id is null or absent |
| `MiniSearch: duplicate ID ${id}` | `add` of a present id |
| `MiniSearch: cannot remove document with ID ${id}: it is not in the index` | `remove` |
| `MiniSearch: cannot discard document with ID ${id}: it is not in the index` | `discard`, `discardAll`, `replace` |
| `Invalid combination operator: ${combineWith}` | `combineResults` with something to combine |
| `Expected documents to be present. Omit the argument to remove all documents.` | `removeAll` with an explicit `null` (or `undefined`) argument; `removeAll()` with no argument resets the index |

### Floating point

Every number is a double and every operation is the one written above, in that order: the
averages, BM25+, the weights, the sums `score += weighted` in scoring order, the combinators'
`a.score + b.score`, `score * quality`, autoSuggest's sum then division, `dirtFactor`.
**`Math.log` must be the host's `Math.log`.** MiniSearch calls the host's, and the hosts disagree
in the last bit, so the only exact definition is per environment: a port matches each engine's
reference only through that engine's `Math.log`. Evidence (the verifier's bit-exact probe,
`build/library-matrix-verify-minisearch/mathlog/`): on the 1,831 BM25 arguments
`1 + (N - n + 0.5) / (n + 0.5)` (N ≤ 60, plus 2.666666666666667), compared with a correctly
rounded logarithm (60-digit decimal), **Chrome 158's `Math.log` is correctly rounded on all of
them**, Node 26.8.1's is 1 ulp off on 150 and Firefox 160's on 161; Node and Chrome differ on 150,
Node and Firefox on 41, Chrome and Firefox on 161 (`Math.log(2.666666666666667)` is
`0.9808292530117264` in Node and Firefox, `0.9808292530117263` in Chrome). On the corpus this makes
**Chrome's reference output differ from Node's on 11 of the 58 cases, while Firefox's equals
Node's on all 58** (its 41 disagreements do not reach these cases). So a correctly rounded
logarithm would match Chrome and fail Node and Firefox, and an fdlibm-style library logarithm would
match Node at best and fail Chrome; the three-environment check catches both. In every language
here the standard `log` compiles to `Math.log` except where noted in "Guidance". `Math.round`
of the non-negative `term.length * fuzzy` rounds halves up (`floor(x)`, plus 1 when
`x - floor(x) >= 0.5`, is exact). Nothing is formatted: doubles go out as numbers.

## Parity domain

- Field names, `storeFields` names and `idField` are ASCII identifiers, distinct, not array
  indices, not `Object.prototype` member names, and `storeFields` holds none of `score`, `terms`,
  `queryTerms`, `match`.
- Document values MiniSearch reads are strings, `null`, or safe integers (indexed numbers go
  through `toString`, which is only plain decimal for integers below 1e21; `-0` is excluded).
- Ids are well-formed strings or integers in the signed 32-bit range, never `-0` (JSON can carry
  it; the reference returns `-0` while a port with integer ids returns `0`); field text, query text
  and stored strings may hold lone surrogates.
- `fuzzy` is a boolean, a number in (0, 1), or an integer from 1 to 20; `maxFuzzy` is an integer
  from 0 to 20; query terms used with fuzzy matching are at most 200 code units (so the
  `Uint8Array` never wraps).
- `bm25`, when given, has all of `k`, `b` and `d`; every option number is finite; `boost`,
  `boostDocument` and `boostTerm` factors are finite; no score is NaN or infinite (so the sort's
  comparator is consistent; `--verify` checks it).
- `combineWith` lower-cases to `or`, `and`, `and_not`, or to a string that is not an
  `Object.prototype` member name (those would be looked up as functions).
- `filter.field` is a stored field name or `id`; `filter.in` holds strings, numbers or `null`.
- Option values are never `null`; `replace` documents have ids.
- Characters in the corpus are classified and lower-cased the same way by Unicode 13.0 to 17.0. The 14 separator code points new in Unicode 16 / 17 (listed under "Tokenizer") are outside the domain.
- Exact equality holds per environment; the reference's own output differs between engines
  through `Math.log` (above).

## Bench cases and variants

`bench-index` indexes 1,200 generated documents (title, text, category, year; 2.5% non-ASCII
words from 20 languages' samples; Zipf-distributed vocabulary of 3,000 pseudo-words) and runs five
prefix searches; about 20–30 ms per call for the JS reference in Node. `bench-search` indexes 400
documents and runs 120 queries cycling through OR, prefix, fractional fuzzy, AND with prefix and
fuzzy, query trees with `AND_NOT` and boosts, and autoSuggest; about 8–10 ms per call. Both are
generated from fixed seeds by `generate-corpus.mjs`.

Neither case discards, so no vacuum runs (a vacuum over more than 1,000 terms would leave a
timer-driven continuation running in the reference between samples).

**Bench variant** (`bench-variant.mjs`, registry `bench.variant`; adopted from the independent
verification): the k-th variant applies a k-seeded permutation of the ASCII letters, the same to
upper and lower case, with `i`/`I` fixed (because `İ`.toLowerCase() yields an ASCII `i`), to every
document string value and every query string, and adds `k × 10,000` to every numeric id (below
2^31 for k < 214,000). Every title, text, stored string and query changes with k, so no cache keyed
on a document, a field or a query can hit (README rule (a)); and because the permutation maps
letters to letters one to one, it keeps every token boundary, case relation, term equality and
prefix relation, edit distance, field length and insertion order, so the work is the same and
every score is the same double (rule (b)); only strings and ids in the output change. Checked by
`build/library-matrix-verify-minisearch/extra/check-variant.mjs` for k in {1, 2, 3, 77, 5000,
100000}: scores and structure equal to the unvaried case, every title and text changed, port equal
to reference. Each variant's expected output is the reference's on the same variant.

A port must not cache anything across calls (MiniSearch does not), nor depend on the identity of
its input object, nor write to its input.

## Corpus

The `x-` cases come from the independent verification's mutation testing
(`build/library-matrix-verify-minisearch/mutants/mutants.mjs`: 80 plausible wrong ports made by
patching MiniSearch's own code, plus an identity control). Without them 10 mutants passed every
case; with them every mutant fails at least one case and only the identity control passes.

| Case | Ops | What it exercises |
| --- | --- | --- |
| `empty-index` | 11 | every operation on an index with no documents |
| `readme-examples` | 19 | the examples of the MiniSearch README and API docs, on its four sample documents |
| `empty-and-separator-queries` | 13 | queries that tokenize to nothing, to empty strings only, or to a tab (not a separator) |
| `combine-operators` | 20 | OR, AND and AND_NOT in any letter case, invalid operators, and when they are or are not reached |
| `duplicate-query-terms` | 9 | repeated query terms are scored once per occurrence but counted once for quality |
| `field-boosts-and-fields` | 13 | boost (0 counts as 1, negative, fractional), field restriction, unknown, repeated and reordered fields |
| `prefix-search` | 10 | prefix matches, their weight by extra length, the exact term counted once, and custom prefix weights |
| `fuzzy-integer` | 10 | integer fuzzy distances 0 to 3: Levenshtein over UTF-16 code units, the weight by distance |
| `fuzzy-fractional` | 12 | fractional fuzzy: Math.round(length * fuzzy) with ties rounded up, capped by maxFuzzy; fuzzy: true is 0.2 |
| `prefix-and-fuzzy` | 7 | prefix and fuzzy together: a term matched by both is scored as a prefix match only |
| `weights-and-bm25` | 9 | weights merged with the library defaults, BM25 parameters, and object options replaced wholesale |
| `constructor-search-options` | 10 | searchOptions given to the constructor: defaults for every search, overridden per call; a query tree node ignores the constructor combineWith |
| `filter-stored-fields` | 9 | the declarative filter over stored fields (strings and numbers), in the constructor, and ignored inside a query tree |
| `filter-in-constructor` | 5 | a filter in the constructor searchOptions applies to search and autoSuggest unless replaced |
| `boost-document` | 8 | boostDocument by id: factors multiply scores, 0 drops a document from a term (not from a wildcard), ids compare by type |
| `boost-term` | 5 | boostTerm by processed query term, applied to that term and every term derived from it |
| `query-trees` | 14 | nested query trees: per-node options inherited by subqueries, wildcard subqueries, empty and single subquery lists |
| `stored-fields` | 9 | storeFields including the id field under a custom idField, absent and null values, numbers; getStoredFields |
| `no-store-fields` | 5 | with no storeFields nothing is stored: getStoredFields is null even for a present document |
| `id-types` | 15 | numeric and string ids are distinct keys (1 is not "1"); zero, negative and Unicode ids |
| `document-errors` | 17 | errors keep MiniSearch's messages and leave the state MiniSearch leaves (addAll and discardAll stop part way); removeAll(null) is an error, not removeAll() |
| `remove-and-readd` | 18 | remove() deletes postings immediately; average field lengths shrink and reset; re-adding gets a new short id |
| `remove-changed-document` | 8 | removing a document that changed since it was added logs version_conflict warnings and leaves stray postings |
| `discard-cleanup-on-search` | 18 | discarded documents are skipped by search, which removes (or decrements) their postings as it meets them |
| `discard-exact-term-merge` | 11 | search deletes an emptied exact term before the fuzzy walk, so the radix merge it causes changes the order of the fuzzy matches |
| `vacuum-merge-order` | 11 | a vacuum that deletes terms merges radix edges, which reorders later prefix and fuzzy matches |
| `discard-auto-vacuum` | 49 | automatic vacuuming: triggered at 20 discards and dirt factor 0.1, run synchronously, then later requests deferred for the rest of the run |
| `vacuum-batches` | 10 | an explicit vacuum stops at its first batch boundary in a synchronous run, keeps dirtCount, and defers the next request |
| `vacuum-complete` | 8 | an explicit vacuum on a small index completes synchronously and resets dirtCount; batchSize 0 means 1000 |
| `vacuum-conditions` | 10 | autoVacuum with custom conditions (2 discards, dirt factor 0.5) and batch size 4: no vacuum until the fourth discard (factor 4/7), which stops after its first 4 terms |
| `vacuum-conditions-defaults` | 6 | autoVacuum with zero conditions falls back to 20 discards and factor 0.1; discardAll triggers one vacuum |
| `replace-documents` | 9 | replace() is discard then add: the new version gets a new short id; a replace may vacuum before it adds |
| `remove-all` | 10 | removeAll() resets the index but keeps dirtCount and the vacuum state; removeAll(documents) removes each |
| `auto-suggest` | 11 | autoSuggest: AND with prefix on the last term by default, grouping by matched terms, averaged scores, option overrides |
| `tokenizer-ascii` | 12 | ASCII separators (\n \r space and punctuation) against symbols and controls that are not separators; leading and trailing empty tokens in field lengths |
| `tokenizer-unicode` | 19 | Unicode separators (Zs, Zl, Zp and every P category) in many scripts, and format characters that are not separators |
| `astral-text` | 13 | astral code points: emoji, ZWJ sequences, flags, math letters, CJK Extension B, Deseret case mapping, astral punctuation |
| `lone-surrogates` | 12 | lone and reversed surrogates in documents and queries are ordinary code points: indexed, matched by prefix and fuzzy, returned exactly |
| `case-mapping` | 12 | toLowerCase: final sigma in its contexts, İ to i plus combining dot (one code unit to two), ẞ, Ǆ/ǅ, Cherokee, Georgian, Kelvin and Ohm signs |
| `radix-structure` | 15 | radix-tree shape: splits move the new intermediate node last, merges after removal move the merged edge last; prefix walks run newest first, fuzzy walks oldest first |
| `surrogate-radix` | 8 | emoji sharing a high surrogate split the radix tree inside the pair: a lone high surrogate is a prefix of them, and the iteration order (toJSON) is not a code-point tree's |
| `numeric-terms` | 8 | terms that are array indices come first in match and terms, in numeric order (Object.keys); other digit strings keep insertion order |
| `object-property-terms` | 7 | terms named like Object.prototype members (constructor, hasownproperty, tostring, proto) are ordinary terms everywhere |
| `score-ties` | 6 | equal scores keep insertion order (a stable sort): identical documents, an AND, a prefix match and a wildcard (its autoSuggest makes a single group; `x-suggest-score-ties` covers tied suggestions) |
| `long-terms-fuzzy` | 5 | long terms: fuzzy 0.2 capped at maxFuzzy 6, integer fuzzy beyond maxFuzzy, prefix weights of long extensions |
| `field-lengths` | 13 | documents missing fields or holding null or empty ones: field lengths count unique raw tokens, averages use the document count |
| `has-and-stored` | 12 | has and getStoredFields across add, discard, replace and remove |
| `x-suggest-score-ties` | 4 | autoSuggest: equal suggestion scores keep hit order (stable sort, no tie-break by text) |
| `x-and-order-ties` | 5 | AND builds the combined map in the right operand's order: visible through tied scores |
| `x-filter-null-vs-absent` | 4 | filter in: [null] matches a stored null but not an absent stored field |
| `x-average-field-length-holes` | 8 | averageFieldLength holes are null in toJSON; a hole read by removeFieldLength is NaN (null), and `\|\| 0` restarts it |
| `x-wildcard-constructor-boost` | 5 | a constructor boostDocument scores and sorts a top-level wildcard; a call boostDocument replaces it |
| `x-vacuum-boundary-on-last-term` | 6 | a vacuum whose first batch boundary falls on the last term has visited every term but has not finished: dirtCount kept |
| `x-remove-term-field-mismatch` | 5 | remove() of a changed document whose term exists in the index but not in that field for it: version_conflict warning, postings kept |
| `x-autosuggest-constructor-prefix` | 3 | autoSuggest's default prefix function overrides a constructor searchOptions.prefix: true |
| `x-prefix-walk-merge` | 8 | a prefix walk deletes a discarded term whose node then merges into its parent; the walk still visits the merged child (snapshot semantics), in search and in vacuum |
| `bench-index` | 7 | bench: index 1,200 generated documents (title, text, category, year; 2.5% non-ASCII words), then five searches |
| `bench-search` | 121 | bench: index 400 generated documents, then 120 queries (OR, prefix, fuzzy, AND with prefix and fuzzy, query trees, autoSuggest) |

## Guidance for ports

What every port needs, then per language. The Rust port (`ports/minisearch/rust/`, `NOTES.md`)
is a worked example of every point.

**Everyone.**

1. Strings as UTF-16 code units (see "Strings are UTF-16"); lengths in code units.
2. A node is an **ordered** map whose order you control: a list of entries (the leaf among them)
   with "append on new key, remove in place, set-then-delete on replace" works everywhere; a
   language's hash map without insertion order (Go `map`, Scala `HashMap`, Elm `Dict`) does not.
   Prefix walks go newest-first, fuzzy walks oldest-first.
3. Posting lists, `_documentIds`, `_fieldLength`, `_storedFields` may be vectors indexed or sorted
   by short id (their insertion order is ascending short id). Result maps (`Map<shortId, result>`)
   need insertion order plus delete (AND_NOT) and a fresh map in another map's order (AND).
4. `match`: an insertion-ordered list of `(term, fields)`; report it in `Object.keys` order (array
   indices first, numerically). `Object.assign` replaces field lists, it does not concatenate.
5. Use a **stable** sort with `b.score - a.score` semantics (`-0` equals `0`).
6. Reproduce the cleanup side effects of searching (the `matchingFields` decrement order, the
   frequency decrement, deleting an emptied exact term before the fuzzy walk) and of vacuuming
   (synchronous first batch, `isVacuuming` sticking) exactly.
7. `Math.log` from the host; no FMA; the operation order as written.
8. Build the output with keys in the order of "Output"; hits get the stored fields after `match`.
9. No caching across calls, no dependence on input identity, no writes to the input.

**Rust (mires).** `no_std` + `alloc`; `Vec<u16>` strings read with `charCodeAt`;
`mires_js::collections::HashMap` (a host `Map`) for insertion-ordered maps; `Math.log` through
`js_bindings!`. Avoid what this backend currently refuses or miscompiles (all reported under
`build/library-matrix/mires-bugs/`): `Vec::into_iter().map(..).collect()` that collects in place,
`<` on slices (use `cmp`), cloning tuples with non-`Copy` fields, `Box<[T]>::from(&[T])` (use
`into_boxed_slice`). `u128`/`u64` are BigInts; `Range::contains` allocates; each `sort_by`
instantiation is about 18 KB.

**ReScript / Melange.** JS strings are UTF-16 natively: `String.length`, `charCodeAt`, `slice`.
The host `RegExp` with the `u` flag (`%re("/[\\n\\r\\p{Z}\\p{P}]+/u")` / `Js.Re.fromStringWithFlags`)
and `Js.String.toLowerCase` are allowed and exact. Use `Js.Map`/`Map` from the runtime or arrays
for ordered nodes; `Js.Dict` is a plain object, whose integer-like keys reorder (exactly the
`Object.keys` rule you need for `match`, but wrong for node children and result maps). `Js.Math.log`.
Building output objects with `Js.Dict`/`Obj.magic` keeps key order, except integer keys.

**js_of_ocaml.** OCaml strings are bytes: keep `Js.js_string` values or convert to an
`int array` of code units (`charCodeAt`), never through `Js.to_string` (UTF-8, lossy for lone
surrogates). The host functions are reachable through jsoo's typed bindings:
`(Js.string s)##toLowerCase`, and `new%js Js.regExp_withFlags` with `"u"` for the split. jsoo's
`int` is 32-bit (frequencies and ids fit). `Float.log` compiles to `Math.log`. `Hashtbl` has no
order: use it for lookups only and keep order in lists or arrays.

**Scala.js.** Strings are UTF-16; `String.length`, `charAt`.

For the tokenizer, use the host RegExp: `js.RegExp("[\\n\\r\\p{Z}\\p{P}]+", "u")` with `jsSplit`, as
the accepted port does. `java.util.regex` supports `\p{Z}`/`\p{P}` only when linking for ES2018+. The
matrix builder links with Scala.js's default ES2015, the production setting the comparator also uses.
There the module imports, but the first call throws `PatternSyntaxException: Unicode character
family is not supported because it requires RegExp features of ECMAScript 2018`. Do not raise the
ES version in a port to get around it; that changes the family's production settings.

If you do use a Java-style `split`, remember it drops *trailing* empty tokens (use `split(re, -1)`).
`toLowerCase()` without a `Locale` is JS `toLowerCase` in Scala.js. `mutable.LinkedHashMap` keeps
insertion order; `math.log` is `Math.log`. Avoid `Long`. Sort with `sortWith(_.score > _.score)`:
`Ordering[Double]` orders `-0` and `0` differently from JS.

**Dart.** Strings are UTF-16 (`codeUnitAt`, `length`). `RegExp(r'[\n\r\p{Z}\p{P}]+', unicode: true)`
and `String.split` (Dart's `split` keeps leading and trailing empty strings like JS: verify on
`tokenizer-ascii`), `toLowerCase()` compiles to JS. `LinkedHashMap` (the default `Map`) keeps
insertion order; `dart:math` `log` is `Math.log`. `List.sort` is not stable: sort `(score, position)`
or use a merge sort. Integers are JS numbers on the web.

**Kotlin/JS.** Strings are UTF-16. With `Regex("[\\n\\r\\p{Z}\\p{P}]+")`, check that the
emitted `RegExp` carries the `u` flag (without it `\p` is not a class) and that `split` keeps the
leading and trailing empty tokens (`tokenizer-ascii` shows both). `lowercase()` is JS `toLowerCase`. `LinkedHashMap`
keeps order; `kotlin.math.ln` is `Math.log`; `sortedWith`/`sortWith` are stable.

From the accepted port:
- Kotlin's `Regex` hands the pattern to the host `RegExp` (`'gu'`), so `\p{…}` works under the ES2015
  target, unlike Scala.js's `java.util.regex`.
- `compareByDescending { it.score }` and data-class `compareTo` / `equals` treat `-0` and `0` as
  different. Sort by the sign of `b.score - a.score`, and compare number values like JS `Map` keys
  (SameValueZero).
- `kotlin.math.round` rounds ties to even; `roundToInt()` / `Math.round` does not.
- Kotlin iterators throw `ConcurrentModificationException` where JS `Map` iteration survives
  deletion. Iterate snapshots, or delete through the iterator's own `remove()`.
- Callable references (`::f`) allocate an object on every use; non-capturing lambdas do not.

**Elm.** `elm/regex` builds its `RegExp` without the `u` flag, so `\p{Z}`/`\p{P}` do not
work: classify natively with `separators.json`, or tokenize in the JS host (counted as host JS).
Elm strings are JS strings: `String.length` and `String.slice` count code units, but the
iterating functions mishandle lone surrogates (elm/core 1.0.5 kernel; found by the accepted port):
- `foldl`, `uncons`, `map` and `filter` pair a high surrogate with whatever unit follows it, and
  append `"undefined"` after a final one;
- `toList`, `foldr`, `any` and `all` mis-decode low surrogates, and **never return** on a string that
  starts with a lone low surrogate (`String.toList "\u{DC00}"`: the `while (i--)` loop steps to `i = -1`,
  which is truthy);
- `Char.toCode` of a lone high surrogate is `NaN`.

Lone surrogates therefore need code-unit access through `slice` / `length`, or through the host. `String.toLower` is JS `toLowerCase`. `Dict` is ordered by key, not
by insertion: nodes and result maps need association lists or an insertion-counter key. There is
no natural logarithm in elm/core: `logBase e x` is `Math.log(x) / Math.log(Math.E)`, exact only
where `Math.log(Math.E)` is exactly 1 (check it in each engine, or take `Math.log` from the
host). `List.sortWith` is stable.

**GopherJS.** Go strings are UTF-8 and GopherJS's JS-to-Go string conversion mishandles lone
surrogates (`docs/COMPARATORS.md`): read text as `[]uint16` through `*js.Object`
(`charCodeAt`) and build output strings with `String.fromCharCode` on the JS side of the boundary.
`unicode.Is(unicode.Z, r) || unicode.Is(unicode.P, r)` uses Go's Unicode tables (version of Go
1.21: Unicode 15.0; fine for this corpus) and adds their size; or call the host `RegExp` through
`js.Global`. `strings.ToLower`/`unicode.ToLower` have no Final_Sigma: call the host `toLowerCase`
or special-case Σ. `math.Log` in GopherJS: confirm it is bound to JS `Math.log` (a pure-Go `Log`
differs in the last bit); `map` iteration is random, so keep order in slices; `sort.SliceStable`.
