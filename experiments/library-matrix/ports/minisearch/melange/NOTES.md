# MiniSearch 7.2.0: Melange port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md`, written as **one OCaml program
compiled by two compilers**: this directory (Melange 7.0.1) and `../jsoo/` (js_of_ocaml 6.4.1)
hold the same core, byte for byte, and differ only in the host module and the entry. It reaches
exact parity (doubles included) on all 58 corpus cases in Node, Chrome and Firefox.

| File | Shared with `../jsoo/` | What it is |
| --- | --- | --- |
| `src/host.mli` | identical | the interface to JavaScript the core sees: `Text` (JS strings), `Value` (reading the input), `Json` (building the result) |
| `src/collections.ml` | identical | `Vec` (growable array) and `Ordered` (an insertion-ordered map: a JS `Map`'s semantics over `Map.Make`) |
| `src/searchable_map.ml` | identical | `SearchableMap`: the radix tree, `createPath`, `lookup`, `trackDown`/`remove`/`cleanup`/`merge`, `atPrefix`, the `TreeIterator` order, the Levenshtein walk of `fuzzySearch` |
| `src/options.ml` | identical | option types, the library defaults and their spread-merge (`{ ...a, ...b }`) |
| `src/minisearch.ml` | identical | the `MiniSearch` class method by method: add/remove/discard/replace, field lengths, vacuuming, search (query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/script.ml` | identical | `run(input)`: decodes the operation script, runs it, builds the result records |
| `src/host.ml` | Melange only | `Host` with `[@@mel.send]` externals, `Js.Json`, `Js.Dict`, `Js.Re` |
| `src/index.ml` | Melange only | the entry: `let run input = Script.run input` (the ES module's export) |

`shasum -a 256 src/{host.mli,collections.ml,searchable_map.ml,options.ml,minisearch.ml,script.ml}`
prints the same six digests here and in `../jsoo/`.

## Interop decisions

- **Strings are JavaScript strings, everywhere the index sees them.** `Host.Text.t` is abstract to
  the core; here it is `string`, which in Melange *is* the JS string (UTF-16), so document text,
  terms, labels and string ids are never copied or converted. Lengths, `slice`s, the radix
  splits and the edit distances are on code units, and lone surrogates survive. The core uses only
  the JS string primitives MiniSearch itself uses: `length`, `charCodeAt`, `slice`, `+`, `===`,
  `toLowerCase`, `split(/[\n\r\p{Z}\p{P}]+/u)` and `String(x)` (for `fieldValue.toString()` and
  `${id}`). Code-unit comparisons (`startsWith`, the label scans, the key order of `Map.Make`) are
  OCaml loops over `charCodeAt`.
- **The tokenizer and `processTerm` are the host's**, as the contract allows and as MiniSearch
  itself does: `split` with the `u`-flag `RegExp` (a module constant, as `SPACE_OR_PUNCTUATION` is in
  MiniSearch; `split` neither reads nor writes `lastIndex`), `toLowerCase` per token.
- **`Math.log` is the host's**: the core calls `Float.log`, which Melange emits as `Math.log`.
- **Names are OCaml strings**: field names, stored field names, the id field, option and operation
  names (`Host.Value.name`, identity under Melange). The parity domain makes them ASCII identifiers.
- **The boundary** (`Host.Value`, used by `script.ml`): the input is `Js.Json.t`, read in place with
  `Js.Json.classify` and `obj[name]` (a `[@@mel.get_index]` external returning `Js.undefined`), never
  written. A document is read lazily, as MiniSearch's `extractField` is `document[name]`. The result
  is built with `Js.Json.object_ (Js.Dict.fromList ...)` and `Js.Json.array`, keys in the order of
  CONTRACT.md "Output".
- **Data structures.** Radix nodes are lists of entries (the `''` leaf among them) in insertion
  order; a split or merge removes the old label and appends the new one, as `set` then `delete` on a
  JS `Map` does. A posting list is two growable int arrays sorted by short id (a document only adds
  postings under its own, largest, short id, so ascending short id is the `Map`'s insertion order).
  `_documentIds`, `_fieldLength`, `_storedFields` are `Vec`s indexed by short id; field-length
  arrays keep their holes and their JS length (`toJSON` shows both). Result maps
  (`Map<shortId, result>`) are a table indexed by short id (dense: 0 to nextId - 1) plus the list of
  slots in insertion order, with delete (AND_NOT) and "a fresh map in b's order" (AND). `match` is a
  list in insertion order, reported in `Object.keys` order (array indices first, numerically).
  Fuzzy results and autoSuggest groups are `Ordered` maps keyed by string; the id map is a
  `Map.Make` over ids (`Float.compare` and code-unit order agree with SameValueZero).
- **Walks with deletions.** MiniSearch deletes terms while its `TreeIterator` walks the tree
  (vacuuming, and search's cleanup of discarded documents' postings during a prefix walk). The
  iterator snapshots each node's keys and only the term just visited is deleted, so it visits the
  terms of the tree as it was when the walk began (CONTRACT.md); the port collects a walk's
  `(term, data)` pairs first and applies each term's changes in walk order. In `termResults` the
  removals of one posting list are applied right after that list is scored (MiniSearch removes
  each while it iterates the list; nothing in between reads what they change). The exact term's
  removals happen before the prefix and fuzzy walks start, as in MiniSearch.
- **The fuzzy matrix** is an `int array`, every store reduced modulo 256 (a `Uint8Array` in
  MiniSearch), shared by all branches of the walk as MiniSearch's is. A label unit past the last
  row (where MiniSearch reads `undefined`, which never prunes and never matches) ends that branch.
- **Vacuuming in a synchronous run**: the first vacuum runs to its first batch boundary, and
  `isVacuuming` then stays true for the rest of the run (CONTRACT.md, "Vacuuming").
- **No cross-call state.** Every `run` builds a fresh index from its own input; the module-level
  values are constants (the `RegExp`, option defaults, `"or"`/`"and"`/`"and_not"`, `" "`). Errors
  MiniSearch throws are the OCaml exception `Minisearch.Error` with MiniSearch's message, caught per
  operation as the reference adapter does; an internal failure (input outside the domain) is an
  `Invalid_argument`, which escapes `run` as a JS exception.

## Language and compiler findings (Melange)

1. **`Hashtbl` costs 65.6 KB minified.** A Melange program with one `Hashtbl.Make` table bundles to
   65,598 bytes after Rollup and the common Terser pass (16,560 gzip); without it, 44 bytes; with one
   `Map.Make` instead, 13,492. The chain: `Hashtbl` imports `Random` (and through it `Digest`, MD5,
   `Int64`) and `Domain`; `Domain` imports `Fun`, whose module initialisation registers an exception
   printer (`Printexc.register_printer`), which keeps `Printexc`, `Printf` and `CamlinternalFormat`
   (92 KB before Terser) alive. The first build of this port used `Hashtbl` and was 102,560 bytes
   minified; the same code with `Map.Make` was 50,969. Both are standard OCaml collections, so the
   core uses `Map.Make` (and a table indexed by short id where keys are dense, below).
2. **Calls through stdlib higher-order functions pay Melange's curried-call check.** `List.find_map`,
   `List.iter` and functor arguments such as `Map.Make`'s `compare` call their closures through
   `Curry._1`/`_2` (an arity check per call). In the first build that was 20% of `bench-index`'s
   time, in the radix tree's scans. The hot loops (node scans, the walks, posting and `match`
   lookups) are therefore direct recursive functions, which is ordinary OCaml; and result maps,
   looked up once per scored posting, are an array indexed by short id instead of a `Map.Make`
   (whose curried `compare` made `bench-search` about 1.6x slower than the array, in a quick
   local timing on a loaded machine).
3. **Cross-module externals become functions.** `host.mli` has to declare the string primitives as
   `val`s (an `external` in a shared interface would fix Melange's primitive syntax), so each call
   from the core is a call of a small wrapper (`$$Text.unit_at` is `s.charCodeAt(i)`); V8 inlines most
   of them.
4. Options of an abstract type (`Text.t option`) are boxed through `Caml_option` (`valFromOption`),
   since the compiler cannot know the type is never `undefined`.

No Melange miscompilation was found.

## Less idiomatic than plain Melange code

Written alone for Melange, this port would use `string`, `Js.String` and `Js.Map` directly instead
of the abstract `Host` interface, which exists so that the same core also compiles with
js_of_ocaml, whose strings are bytes.

## Size and time

`dist/index.min.js` is 51,339 bytes (14,251 gzip-9, 12,700 brotli-11; the floor is 46 bytes). Of
the 134 KB Rollup renders before Terser, 79 KB is the port's own modules and 55 KB the emitted
stdlib (`Map`, `List`, `Array`, the curry and exception runtime; `meta.json`). The authoritative
figures are in `build/library-matrix/sizes.json`, the timings in the bench receipts under
`build/library-matrix/bench/minisearch/`.
