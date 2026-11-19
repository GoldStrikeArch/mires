# MiniSearch 7.2.0: Elm port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` to Elm 0.19.2: the same
algorithms and data structures as `minisearch/dist/es/index.js`, method by method, over a
persistent index. No kernel code, no `Debug`, no packages beyond `elm/core` and `elm/json`. The JS
host (`js/index.js`, 27 non-blank lines) is host I/O only. The port reaches exact parity (scores included) on
all 58 corpus cases in Node, Chrome and Firefox.

| File | What it is |
| --- | --- |
| `src/MiniSearch.elm` | the `MiniSearch` class: add/remove/discard/replace, field lengths, vacuuming, search (option spreading, query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON`; the option, query and result types |
| `src/SearchableMap.elm` | `SearchableMap`: the radix tree, `createPath`, `lookup`, `remove`/`cleanup`/`merge`, `atPrefix`, the `TreeIterator` order, the Levenshtein walk of `fuzzySearch` |
| `src/Text.elm`, `src/Separators.elm` | strings as UTF-16 code units: the default tokenizer, `toLowerCase`, the `Object.keys` order of `match`; the separator table |
| `src/OrderedDict.elm` | an insertion-ordered dictionary, the stand-in for a JS `Map` whose order is observable |
| `src/Main.elm` | the boundary: decodes the input as `reference.mjs` maps it, runs the script, encodes the result |
| `js/index.js` | the JS host: sends the input through the `request` port, returns what `response` answers |

## Decisions

- **Persistent, threaded index.** Every method that changes the index takes it and returns the new
  one. A method that throws in MiniSearch returns `Err` with MiniSearch's message, and, where
  MiniSearch throws part way, the index as it leaves it: `addAll`, `removeAll(documents)`,
  `discardAll`, `replace` (a discard that succeeded, then an `add` that failed) and `search` (its
  cleanup of discarded postings happens before an invalid operator is found) return
  `( index, Result String a )`. The logger is a log kept in the index, cleared per operation by the
  boundary.
- **Strings are JS strings, read as UTF-16 code units.** Elm's `String.length`, `slice`, `left`,
  `dropLeft`, `startsWith` and `++` are the JS ones, so radix labels, term lengths and every output
  string are exactly MiniSearch's, lone surrogates included. Code-unit *values* (edge lookup, the
  Levenshtein comparison) come from `Text.codeUnitAt`, which works around elm/core's decoding (see
  "Language problems", 1) and equals `charCodeAt` on all 65,536 code units (checked).
- **The tokenizer classifies natively** with `separators.json` (`Separators.elm`, 203 ranges, an
  ASCII bit set built from the same table, then a binary search), because `elm/regex` has no `u`
  flag. It scans code points with `String.foldl` and repairs elm/core's pairing of a high surrogate
  with whatever follows it (`Text.tokenize`); on 200,000 random strings of surrogate halves,
  pairs, astral separators and punctuation it equals `text.split(/[\n\r\p{Z}\p{P}]+/u)`.
- **`toLowerCase` is the host's** (`String.toLower` compiles to it), as the contract allows.
- **`Math.log` is the host's, through `logBase e x`**, which elm/core compiles to
  `Math.log(x) / Math.log(Math.E)` (the minified bundle keeps both calls; Terser does not fold
  `Math.log(Math.E)`). That is exactly `Math.log(x)` where `Math.log(Math.E)` is 1. Probed in each
  engine: `Math.log(Math.E) === 1` in Node 26.8.1, Chrome 158 and Firefox 160, and
  `Math.log(x) / Math.log(Math.E) === Math.log(x)` on all 80,601 BM25 arguments
  `1 + (N - n + 0.5) / (n + 0.5)` with N up to 400. An engine where `Math.log(Math.E)` is not 1
  would need `Math.log` from the host, which Elm can only reach through ports.
- **Radix nodes are insertion-ordered lists**, as a `Map`'s keys are: the leaf is one of the
  entries, a new key is appended, a deleted one removed in place, a replaced edge (a split, a
  merge) removed and the new one appended. Lookups scan from the oldest entry as MiniSearch's
  `for (const k of node.keys())` does, iteration (newest first) folds from the right, the fuzzy
  walk (oldest first) from the left. Each edge carries its label's first code unit (labels never
  change once made), so a lookup compares integers. `createPath` is one pass over a node's
  entries: follow, split, or append.
- **Walks see the tree as it was when they began**, because it is a value. That is what MiniSearch's
  iterator sees too (it snapshots each node's keys and only the term just visited is ever deleted),
  so the prefix walk, the fuzzy matches and the vacuum walk take their terms (and data) from the
  index at the moment MiniSearch starts them, and apply each term's changes in walk order: the
  exact term's cleanup (and the merge it may cause) before the prefix view and the fuzzy walk are
  taken, as in MiniSearch.
- **The Levenshtein matrix is a list per row.** MiniSearch's matrix row `i` only depends on row
  `i - 1`, and the cells outside a row's band are never written (they keep `maxDistance + 1`, and
  `i` in column 0), so a row is a pure function of the row above and the label unit; the walk
  carries the current row and stops at the last row of the matrix, as the contract allows.
- **Other structures.** Posting lists are `Dict Int Int` (their insertion order is ascending short
  id, `Dict`'s order); a term's fields are `Dict Int …` (their order is never observable);
  `_documentIds`, `_fieldLength`, `_avgFieldLength` and `_storedFields` are `Dict`s by short id or
  field id (holes are missing keys; `toJSON` writes them as `null`); `_idToShortId` is two `Dict`s,
  for string and numeric ids (`1` and `"1"` stay different documents). Result maps and the
  autoSuggest groups are `OrderedDict`s (AND builds a new one in the right operand's order). A
  result's `match` is an insertion-ordered list; `Object.keys` order (array indices first,
  numerically) is applied when the result is assembled, and `Object.assign` of two `match`
  objects replaces a field list in place.
- **Options are records of `Maybe`s**; `{ ...a, ...b }` is `merge`, key by key. `prefix` is a
  boolean or a function (autoSuggest's default `i === terms.length - 1`), `fuzzy` a boolean or a
  number, `autoVacuum` a flag or options. The declarative `filter`, `boostDocument` and
  `boostTerm` become functions in the boundary, as the reference adapter makes callbacks; the
  filter reads `result[field]` through `MiniSearch.resultField` (a stored `id` replaces the id).
- **Vacuuming in a synchronous run** follows the contract: the first vacuum runs up to its first
  batch boundary, gives the dirt count back only when its walk ends first, and the vacuum then
  stays current for the rest of the run, so later requests never start.
- **The boundary.** `Json.Decode` reads the input into the types above: documents as the values of
  the names MiniSearch reads (`idField`, `fields`, `storeFields`), absent names left out (`null`
  and absent stay different for stored fields). The result is built with `Json.Encode`, keys in
  the adapter's order. `Encode.object` assigns keys in order on a plain object, so a stored field
  named `id` replaces the hit's id in place, exactly as `Object.assign` does. `toJSON` numbers go
  out as `JSON.parse(JSON.stringify(...))` gives them back (non-finite is `null`). An input the
  decoders refuse (outside the parity domain) makes the host throw.
- **The JS host** is the pilot's: one `Platform.worker` instance, created on the first call, and
  the slot the synchronous answer lands in. Those are its two module-state lines (`let app`,
  `let response`); the worker's model is `()`, so nothing survives a call. Every `run` builds its
  index from its own input, and the input is never written.

## Language problems

No compiler bug was found. One elm/core defect and these elm/core and compiler behaviours forced
a less obvious choice:

1. **elm/core 1.0.5's string iteration assumes well-formed UTF-16**, and Elm strings are JS strings
   that may hold lone surrogates (the corpus has them). Minimal repro (`elm make --optimize`, run
   in Node):

   | Expression | Result |
   | --- | --- |
   | `String.foldl (\_ n -> n + 1) 0 "\u{D800}a"` | `1`: a high surrogate is taken with whatever unit follows it (also `uncons`, `map`, `filter`) |
   | `String.foldl (::) [] "a\u{D800}"` | the last `Char` is `"\uD800undefined"`: the kernel appends `string[length]` |
   | `Char.toCode (Char.fromCode 0xD800)` | `NaN` |
   | `String.toList "\u{DC00}"` | **never returns**: `_String_foldr`'s `while (i--)` steps past a leading low surrogate to `i = -1`, which is truthy, and loops forever (also `String.foldr`; `any` and `all` share the loop) |

   The port never calls `foldr`, `toList`, `any`, `all`, `uncons`, `map` or `filter` on strings.
   The tokenizer folds with `String.foldl` after splitting off trailing high surrogates and repairs
   each multi-unit `Char` it is given; `Text.codeUnitAt` reads a high surrogate paired with U+DC00
   and takes the low half off again.
2. **Comparisons of two variables are generic.** Elm compiles `a == b` to `_Utils_eq` (which
   allocates an array on every call), and `a < b` and `min a b` to `_Utils_cmp`, unless one side is
   a literal. The two hottest loops (edge lookup, the Levenshtein row) compare a difference with a
   literal (`sameUnit`, `smaller`, `atMost` in `SearchableMap.elm`), which compiles to `===` / `<`.
3. **A function of more than nine parameters compiles to nested closures** (Elm has `F2`..`F9`
   only), one allocation per argument per call. The Levenshtein row fill keeps its constants in a
   record to stay at nine.
4. **`==` on a value holding a function crashes at run time.** Queries and options hold the
   callbacks, so the wildcard check and the `Maybe` callback options are pattern-matched, never
   compared.
5. **No natural logarithm** in elm/core: `logBase e` (see "Decisions").
6. **No shadowing**, so the decoders in `Main.elm` are named `…Decoder` next to the values they
   decode.
7. **`Dict.size` is O(n).** MiniSearch's `fieldIndex.size <= 1` before deleting a posting is "the
   posting list is empty after the deletion" in the port.

## Size and performance

`dist/index.min.js` is about 47 KB; the exact figures are in `build/library-matrix/sizes.json` and
`MATRIX.md`. The host is 1.1 KB of source (2% of the minified bundle, far from the "mostly JS"
half). By `dist/meta.json`, an empty `Platform.worker` compiled the same way is 6.5 KB minified;
the rest is the port and the parts of elm/core and elm/json it uses (`Dict`, `Set`, `Array`, the
JSON decoders and encoders).

The harness's run (24 samples, the bench variant, load average 4 to 6; receipts under
`build/library-matrix/bench/minisearch/`) put a call at 1.9 to 3.0 times the reference: port/js
3.03 / 2.82 in Node (bench-index / bench-search), 2.49 / 2.89 in Chrome, 1.92 / 2.71 in Firefox,
with A/A between 0.98 and 1.01. By CPU profiles in Node, the time goes to
path copying in the persistent tree and `Dict`s (`createPath` and the posting updates are half of
indexing), garbage collection (about 12%), and Elm's curried calls (`A2`/`A3`, about 20% with
inlining off). What was changed for it, each change ordinary Elm: node entries in insertion order
(MiniSearch's frequent edges, inserted early, are found first, as in its `Map` scan), integer
first units on edges, a single pass for `createPath`, and the Levenshtein row fill at nine
parameters with literal comparisons (locally, bench-search went from 45 ms to 26 ms a call and
bench-index from 87 ms to 65 ms).

## Verification beyond the corpus

Besides the harness: a differential fuzz against the reference adapter in Node (scratch scripts,
not part of the port) on 11,000 random scripts, half of them built from surrogate halves, astral
separators, Σ and İ (indexing, prefix and fuzzy search, discards, vacuum, removal, `toJSON`), half
over the whole subset (query trees, wildcards, boosts, filters, every combinator spelling, invalid
operators, replace, `removeAll` in its three forms, changed-document removal, automatic vacuuming
with conditions and batch sizes): all equal. That fuzz also met a MiniSearch defect outside the
synchronous domain: when a vacuum pauses at a batch boundary and the script then calls
`removeAll()`, the vacuum's continuation (on a timer, after `run` has returned) reads the
replaced index and rejects with a `TypeError`. No corpus case triggers it.
