# MiniSearch 7.2.0: Go port (GopherJS)

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` to Go 1.21, compiled by
GopherJS 1.21.0 (`gopherjs build -m`, the harness's production setting). It follows
`minisearch/dist/es/index.js` method by method, with the `github.com/gopherjs/gopherjs/js` package
only at the boundary and for the host functions the contract allows. There is no embedded JS, no
handwritten `.js`, no package-level variable, no goroutine, and no 64-bit integer in the port's
code (`math.Round`, once per fuzzy query term, uses emulated `uint64` inside). It reaches exact
parity, float scores included, on all 58 corpus cases in Node 26.8.1, Chrome 158 and Firefox 160,
for both `dist/index.min.js` and `dist/index.js`. Safari is unavailable on this machine.

| File | What it is |
| --- | --- |
| `src/searchable_map.go` | `SearchableMap`: the radix tree, with nodes as insertion-ordered entry slices (the `''` leaf among them). It holds `createPath`, `lookup`, `trackDown`, `delete` with `cleanup`/`merge`, `atPrefix`, the `TreeIterator` order and the Levenshtein matrix walk of `fuzzySearch` |
| `src/minisearch.go` | the `MiniSearch` type: values and ids, posting lists, add/remove/discard/replace, field lengths, vacuuming, the state getters, `toJSON`'s data |
| `src/search.go` | search and autoSuggest: query trees, `executeQuerySpec`, `termResults` with BM25+, the combinators, the `Object.keys` order of `match`, the stable score sort |
| `src/options.go` | option types and their spread-merge (`{ ...a, ...b }`) |
| `src/text.go` | UTF-16 text: code units in and out, the default tokenizer, `toLowerCase`, array-index keys, integer `toString` |
| `src/boundary.go` | what `reference.mjs` does: the corpus input into Go values, the operation script, the result as JS objects |
| `src/main.go` | publishes `run` on `js.Module.exports` |

## Interop decisions

- **Text is `[]uint16`, never a Go string.** Go strings are UTF-8, and GopherJS's JS-to-Go string
  conversion mishandles lone surrogates: a lone high surrogate swallows the next unit, and a lone
  low one becomes U+FFFD (`docs/COMPARATORS.md`; `$internalize` in the prelude). Document values,
  queries, `combineWith`, the `boostTerm` terms, string ids, stored strings, radix labels, terms
  and messages are all code units:
  - they are read with `s.Call("charCodeAt", i).Int()` per unit. That is 2.3 ms for bench-index's
    443K units in Node, and faster than going through `.String()`;
  - they are written with `String.fromCharCode.apply(null, units)`. GopherJS externalises a
    `[]uint16` as a `Uint16Array` view, so the copy happens in one host call (chunked at 8,192
    units).

  Names are Go strings: field names, stored-field names, `idField` and operation names. They are
  ASCII identifiers in the parity domain, so `.String()` is exact for them. `Error()` decodes a
  message to Go UTF-8 for display only; the boundary writes the code units.
- **Map keys for text.** A slice cannot be a Go map key. `key(units)` packs two bytes per code unit
  into a Go string, losslessly, where a map is needed: string ids, autoSuggest phrases, and the
  distinct-token set of long fields.
- **Tokenizer: native, with Go's `unicode` tables.** A code point is a separator if it is `\n`,
  `\r`, `unicode.IsPunct` or `unicode.Is(unicode.Z, ·)`. Surrogate pairs are decoded with
  `unicode/utf16`, and a lone surrogate is its own code point (category Cs, never a separator), as
  under the `u` flag. This is the contract's GopherJS guidance. Go 1.21's tables are Unicode 15.0,
  and the contract's `separators.json` is 17.0; they differ at exactly 14 code points:
  - U+1B4E, U+1B4F, U+1B7F;
  - U+10D6E, U+10ED0;
  - U+113D4, U+113D5, U+113D7, U+113D8;
  - U+11BE1;
  - U+16D6D–U+16D6F;
  - U+1E5FF.

  All 14 are punctuation added in Unicode 16.0 and 17.0. I checked with a native Go 1.21 program
  over all of U+0000–U+10FFFF, and a scan of `corpus.json` found none of them in any string. The
  bench variant permutes only ASCII letters. In Latin-1, `isSeparator` tests space and no-break
  space (Z there) and `unicode.IsPunct`'s flag table, without calling `unicode.Is` (see finding 1
  below).
- **`toLowerCase`: ASCII in Go, everything else on the host.** Go's `unicode.ToLower` maps one rune
  to one rune: it has no `İ` → `i` + U+0307 and no Final_Sigma. A term with any unit ≥ 0x80
  therefore goes through `String.prototype.toLowerCase`, which the contract allows: the units go
  out as a JS string, are lowered, and are read back. The host sees exactly the token MiniSearch
  lowers, so Final_Sigma gets its context right, lone surrogates included. An all-ASCII term is
  lowered in Go (A–Z to a–z, the whole of what the host would do), and an unchanged term is
  returned as is.
- **`math.Log` is `Math.log`.** GopherJS overrides `math` with natives
  (`compiler/natives/src/math/math.go`): `Log` is `math.Call("log", x).Float()` behind a NaN
  guard. I checked this in the emit and in each engine (Node and Chrome disagree on
  `Math.log(2.666666666666667)`, and both checks pass). The rest:
  - `math.Floor` is `Math.floor` and `math.Mod` is JS `%`;
  - `math.Round` is Go's exact bit-level implementation. It rounds halves away from zero, which is
    `Math.round` for the non-negative `term.length * fuzzy`;
  - `math.Trunc` is never used: it is wrong above 2^31 under GopherJS;
  - Go does not fuse `x*y + z` here, because GopherJS emits plain JS arithmetic;
  - the BM25+ expression is written in the contract's operation order.
- **typeof.** `js.Global.Call("$typeOf", v)` reaches the prelude's `typeof` helper, as the
  FingerprintJS GopherJS port does. `v == nil` compiles to `=== null` and `js.Undefined` to
  `undefined`. Numbers are read with `.Float()` (`$parseFloat` returns a number as is, `-0`
  included). Object keys (the `boost` option) come from `js.Keys`.
- **Output** is built as plain JS objects and arrays (`Object`/`Array` `New`, `Set`, `SetIndex`),
  in the adapter's key order. `Object.assign(result, storedFields)` and `toJSON`'s integer keys come
  free from the host objects: a stored `id` replaces the `id` in place, and short ids and field ids
  set as keys are listed ascending. `JSON.parse(JSON.stringify(·))` is modelled where it shows:
  non-finite numbers and holes become `null`, and `-0` becomes `0`.

## Data structures

- A **radix node** is `[]*entry` in insertion order. Setting a new key appends; deleting removes in
  place; a split or a merge appends the new edge and then deletes the old one, so it moves last.
  Iteration is newest first, and the fuzzy walk oldest first.
- **Walks under deletion.** Vacuuming and search delete terms while MiniSearch's iterator walks the
  tree. The iterator works on key snapshots, and only the term just visited is ever deleted, so
  the walk visits the tree as it was (CONTRACT.md, "The index"). `entries()` and `atPrefix()`
  therefore collect `(term, *termData)` first. The value pointers are the very objects MiniSearch's
  iterator would hand out, and merges move whole subtrees, not values.
- **`termResults`** removes the postings of discarded documents after it has scored the term,
  instead of while iterating the list. That is the same sequence of index changes, and nothing in
  between reads them. The exact term's removals happen before the prefix and fuzzy walks start, as
  in MiniSearch (`discard-exact-term-merge`). Fuzzy matches that are prefix matches are dropped by
  one test (they are exactly the terms extending the query term), which is what the prefix walk's
  `fuzzyMatches.delete` amounts to.
- **Posting lists** (`Map<shortId, freq>`) are slices sorted by short id, which is their insertion
  order (a document only ever adds postings under its own, newest short id).
- **Per short id.** `_documentIds`, `_fieldLength` and `_storedFields` are always deleted together,
  so one `[]*document` indexed by short id holds them (nil once removed or discarded).
- **Arrays with holes.** `_avgFieldLength` and a document's field lengths are `[]float64` with NaN
  for a hole. Every use MiniSearch makes of a hole treats it as NaN does: arithmetic gives NaN,
  `|| 0` gives 0, JSON gives `null`, and `forEach` skips it (only field lengths are iterated, and
  they are never genuinely NaN).
- **Ids.** `_idToShortId` is two Go maps, numbers and strings, because `1` and `"1"` are different
  keys. **Result maps** are an insertion-ordered slice plus a `map[int]*result` for lookups: AND
  builds a new one in b's order, and AND_NOT rebuilds a's without b's documents. `match` is an
  insertion-ordered slice reported in `Object.keys` order: array indices first, numerically, from
  `arrayIndex` (a `float64` accumulator, since `uint32` would overflow and `uint64` is emulated),
  and `Object.assign` replaces field lists. Go maps are used for lookups only and never iterated:
  their iteration order is randomised.
- **Options** are `SearchOptions` with pointer fields, nil meaning absent, and `over` is the
  spread. Defaults are MiniSearch's `defaultSearchOptions` and `defaultAutoSuggestOptions` (whose
  prefix is the "last term" function). A tree node merges only the call's and the nodes' own
  options. `x || default` on numbers is `or`, where 0 and NaN are falsy.
- **Vacuuming** in a synchronous run follows the contract. The first vacuum runs up to its first
  batch boundary (`math.Mod(i, batchSize) == 0`, which is JS `%`), and gives the dirt back only if
  the walk ends first. `isVacuuming` then stays true, and later requests never run.
- **Errors** are Go `error`s (`*Error`, with the message in UTF-16), returned up to the script,
  which records them. An input outside the parity domain panics, which GopherJS throws out of
  `run`.
- **No cross-call state.** There is no package-level variable: the module-state lint is 0. Each
  `run` builds its own index from its own input, and never writes or retains the input.

## GopherJS problems, and where they forced less idiomatic Go

The first version was idiomatic Go 1.21 with `slices.SortStableFunc`, `slices.IndexFunc`,
`ContainsFunc` and `DeleteFunc`, value structs and `strconv`. It passed all 58 cases at once, at
7.6x (bench-index) and 7.2x (bench-search) the reference's time in Node. The findings below took it
to the receipts' 1.7x to 2.8x. Each is in the code with a comment.

1. **Defect: `-m` makes `&slice[i]` of a struct element ~20x slower.** Repro:
   `build/library-matrix/gopherjs-bugs/minified-indexptr/` (`bash run.sh`): 2M `&s[i]` take
   10.7 ms in the plain emit and 241 ms in the `-m` emit. The prelude's `$indexPtr` creates a
   nested closure on every call. `-m` minifies the prelude with esbuild's `keepNames`, which wraps
   that closure in `Object.defineProperty(fn, "name", …)` each time. The port's `-m` emit was ~2x
   slower than its plain emit until the hot paths stopped taking such pointers:
   - posting lists are `[]*posting`, not `[]posting`;
   - stored fields, boosts, filter values and subqueries are pointer slices;
   - `isSeparator` skips `unicode.Is` in Latin-1.

   The standard library pays the cost too: `unicode.is16` takes `&ranges[i]`, so every non-Latin-1
   character still costs a few of these calls (about 5% of the `-m` bench-index time, from 2.5% of
   its words).
2. **A call through a function value makes the caller resumable.** Repro:
   `build/library-matrix/gopherjs-bugs/funcvalue-blocking/`. GopherJS assumes that any call through
   a func parameter, variable, struct field or interface method may block, and it compiles such
   callers, and all their callers, into `$restore` state machines. The first version had 77, `run`
   included. So the port has:
   - no `slices.*Func`, no `sort.Interface` and no closures;
   - its own stable merge sort (`scoreOrder`, used for both hits and suggestions) and binary search
     (`find`);
   - plain loops where `IndexFunc`, `ContainsFunc` and `DeleteFunc` would be idiomatic;
   - the operation script as an `operation` struct and a `switch` instead of a table of closures.

   This alone did not change the speed much (nothing blocks; the state machines run straight
   through), but it cut the emit from 239 KB to 186 KB: `slices` dropped from 47 KB to 4.6 KB.
3. **Struct values are copied field by field.** Every assignment, argument or `range` element of a
   struct type is a `$clone`. The first version's option structs (`option[T]` generics with a "set"
   flag, nested `Weights`/`BM25` values) were deep-copied at every merge and call. So:
   - options are pointer fields (a common Go idiom for optional values anyway);
   - the per-term scoring context and the query specs are passed by pointer;
   - the hot paths take `*Value`.
4. **Every `append` and every subslice allocates a slice header**, even within capacity.
   `mergeSort` therefore passes bounds instead of subslicing. Counting the tokens first, as
   `strings.Split` does, was measured and gained nothing, so the tokenizer appends.
5. **A struct-typed map key is hashed by building a string from its fields** (with regex escapes),
   on every lookup. Ids are therefore two maps (`map[float64]int`, `map[string]int`), not one map
   keyed by a struct.
6. **`strconv` costs ~53 KB of emit** (its float tables; `FormatInt` alone took a test program
   from 52 KB to 105 KB), and its integer path is emulated `int64`. `numberText` writes the safe
   integers MiniSearch stringifies, with `math.Mod`.
7. **`new Set(tokens).size`** with a Go `map[string]struct{}` costs a key allocation and a
   `String.fromCharCode` per token: 17 ms for bench-index's 3,600 fields. Pairwise comparison
   takes 4.4 ms (most tokens differ in length) but is quadratic, so `distinctCount` compares a
   field of up to 128 tokens pairwise and uses the set above that.

Known and contract-listed, and not hit here: `math.Trunc` (not used), lone surrogates in Go
strings (never made), random map order (no map is iterated), and blocking from JS (no goroutine,
channel or sleep). No GopherJS miscompilation was found. `go vet` and `gofmt` are clean.

## Size

`dist/index.min.js` is 134,177 bytes (35,615 gzip-9, 30,804 brotli-11), against the GopherJS floor
of 41,652 (13,739 gzip-9); the figures are in `build/library-matrix/sizes.json`. Of the 170 KB `-m`
emit (`dist/meta.json` attribution), the parts are:

| Part | Bytes |
| --- | ---: |
| prelude | 41.8 K |
| the port | 96.4 K |
| `unicode` (the P and Z tables, `IsPunct`'s flag table) | 11.5 K |
| `github.com/gopherjs/gopherjs/js` | 4.8 K |
| `runtime` | 4.8 K |
| `slices` | 4.6 K |
| `unicode/utf16` | 2.7 K |
| `math` | 2.6 K |
| `math/bits` and `cmp` | 0.9 K |

GopherJS's statement-per-line emit (bounds checks on every index, `$clone`s, typed slice headers)
makes the port's own code large.

## Performance

Measured by `tools/bench.mjs` (triplet design, 24 samples, the bench variant; receipts under
`build/library-matrix/bench/minisearch/`). The 1-minute load average was 4.8 to 6.5 on 10 cores.
These numbers are not a claim: read each ratio next to its A/A.

| Environment | bench-index port/js | A/A | bench-search port/js | A/A |
| --- | --- | --- | --- | --- |
| Node 26.8.1 | 2.84 | 1.02 | 2.69 | 0.98 |
| Chrome 158 | 2.32 | 0.98 | 2.70 | 1.01 |
| Firefox 160 | 1.72 | 1.03 | 2.03 | 0.97 |

Where the time goes, from Node CPU profiles of bench-index:
- about a quarter is slice growth and slice headers (`$growSlice`, `$internalAppend`, the slice
  constructor), mostly posting-list and token appends;
- 15% is `createPath`'s scans of node entries, each index bounds-checked;
- 7% is garbage collection;
- the rest is spread over tokenizing, lower-casing and the boundary. The boundary is a few
  percent: `charCodeAt` in, `fromCharCode` out.
