# MiniSearch 7.2.0: ReScript port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` to ReScript 12.3.0: the same
algorithms and data structures as `minisearch/dist/es/index.js`, method by method, with no `%raw`,
no `Obj.magic` and no handwritten JS. It reaches exact parity (scores included) on all 58 corpus
cases in Node, Chrome and Firefox.

| File | What it is |
| --- | --- |
| `src/SearchableMap.res` | `SearchableMap`: the radix tree (nodes are JS `Map`s), `createPath`, `lookup`, `trackDown`, `remove`/`cleanup`/`merge`, `atPrefix`, `fetch`, the `TreeIterator` (key snapshots, newest first) and the Levenshtein walk of `fuzzySearch` on a `Uint8Array` |
| `src/MiniSearch.res` | the `MiniSearch` class: add/remove/discard/replace, field lengths, vacuuming, search (option spreading, query combinations, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/Index.res`, `src/Index.resi` | the boundary, doing what `libraries/minisearch/reference.mjs` does; `run` is the module's only export |

## Decisions

- **The same JS data structures, through ReScript's standard library.** ReScript compiles to JS
  and its `Map`, `Dict` and arrays are the JS ones, so the port keeps MiniSearch's structures
  instead of re-creating their ordering rules:
  - radix nodes, posting lists, `_documentIds`, `_idToShortId`, `_fieldLength`, `_storedFields`,
    result maps and the fuzzy result map are `Map.t`, insertion-ordered exactly as in MiniSearch;
  - `_fieldIds`, a result's `match` and the field boosts are `dict` (plain objects), so
    `Dict.keysToArray` is `Object.keys` and gives the array-index-first order of `match`
    (`numeric-terms`) natively. `Object.assign` of two `match` objects is `Dict.assign`;
  - per-document field lengths and the average field lengths are arrays with holes, as in
    MiniSearch: `Array.setUnsafe` past the end leaves holes, `Array.get` reads a hole as `None`,
    `Array.forEachWithIndex` (JS `forEach`) skips them, and `JSON.stringify` writes them as
    `null`.
- **A node is `Map.t<string, entry<'v>>` with `entry<'v> = Node(node<'v>) | Leaf('v)`.** A JS
  `Map` holds child nodes and the leaf value under different keys; ReScript needs one value type,
  so each entry is a variant (one `{TAG, _0}` object per edge and leaf). `@unboxed` cannot
  distinguish two `Map`-shaped payloads at run time. The leaf is still the `""` key, so its
  position among the node's keys stays observable, as in MiniSearch.
- **The iterator is MiniSearch's `TreeIterator`**: a path of `(node, keys snapshot)` frames, keys
  popped from the end, the child of a key looked up when the walk gets there. Deleting terms and
  merging nodes while it walks (vacuuming, search's cleanup of discarded postings, the
  `x-prefix-walk-merge` case) therefore behaves exactly as in MiniSearch, interleaved in the same
  order, without collecting terms first.
- **Host functions the contract allows**, all through typed externals or the standard library:
  `text.split(/[\n\r\p{Z}\p{P}]+/u)` (a ReScript 12 regex literal and an `@send external` typed
  `(string, RegExp.t) => array<string>`, because `String.splitByRegExp` is typed
  `array<option<string>>` for capture groups), `String.toLowerCase`, `Math.log`, `Math.round`,
  `Object.prototype.hasOwnProperty.call` (MiniSearch's `getOwnProperty`; `Dict.has` is the `in`
  operator, which sees `Object.prototype` members such as `constructor`), `Object.fromEntries`
  on a `Map` (typed `Map.t<int, 'a> => dict<'a>`), and in the boundary `JSON.stringify` /
  `JSON.parse` for `toJSON`, as the reference adapter does.
- **Options are records with optional fields**, and `{ ...a, ...b }` is `spread`, field by field
  with `Option.orElse`. Unions are `@unboxed` variants whose run-time value is the JS value:
  `prefix` (a boolean, or a function for autoSuggest's default, so `termToQuerySpec` keeps
  MiniSearch's `typeof options.prefix === 'function'` test), `fuzzy` (boolean or number),
  `autoVacuum` (boolean or options), `id` (string or number; as `Map` keys `1` and `"1"` stay
  different documents). The three callbacks (`filter`, `boostDocument`, `boostTerm`) are closures
  built by the boundary from the declarative JSON, as in the reference adapter.
- **The boundary converts nothing on input.** `input` is a record; operations are a
  `@tag("op")` variant with inline records, so `switch` compiles to `switch (operation.op)` on
  the input object itself; queries, ids and option unions are `@unboxed` variants, documents are
  `dict<JSON.t>`. The output is built as `JSON.t` with `JSON.Encode` (identity at run time, so
  string arrays such as `terms` are shared, not copied), each record as a `dict` in the key order
  of CONTRACT.md's "Output".
- **Errors** are a ReScript exception, `MiniSearchError(message)`, which `run` catches per
  operation to record `error`; anything else (a port bug, an input outside the parity domain) is
  not caught and fails the call. `discardAll`'s `try { ... } finally { ... }` is a
  `switch ... | exception error => (restore; throw(error))`, since ReScript has no `finally`.
- **Vacuuming in a synchronous run** follows the contract: `performVacuuming` runs its terms up to
  the first batch boundary, gives the dirt count back only when the walk ends first, and
  `currentVacuum` (`isVacuuming`) stays true for the rest of the run, so later requests are never
  started (MiniSearch chains them behind a promise that settles after `run` returns).
- **Fuzzy rows past the matrix.** MiniSearch reads a row past the end of its `Uint8Array` as
  `undefined`, which never prunes and never matches, and walks the subtree to no effect; the port
  stops at the end of the matrix, which the contract allows ("The index").
- **`SearchableMap` keeps MiniSearch's own `_size` cache** (reset by `fetch` and `delete`), the
  only cache MiniSearch keeps. Nothing else survives a call: every `run` builds its index from its
  own input, and the input is never written.
- **Out of the parity domain** the port fails fast instead of guessing: a document id that is not
  a string, a number or null, and an indexed field value that is an object or an array, `panic`.

## Language and toolchain problems

No compiler bug was found: every construct compiled to the JS one would expect. These ReScript
12.3.0 behaviours forced a less obvious choice, each documented in the source:

1. **`Iterator.find` was the port's biggest cost in V8.** ReScript 12's standard library binds the
   ES2025 iterator helpers, and `node->Map.keys->Iterator.find(k => ...)` is the natural way to
   write MiniSearch's `for (const k of node.keys()) if (...) return k`. It compiles to
   `node.keys().find(closure)`, which V8 cannot inline the way it inlines `for...of`: by a CPU
   profile, `createPath` and its predicate took 47% of `bench-index` (2.3 times the reference's
   `createPath`), and the harness's first bench run gave port/js 1.93 / 1.45 (Node,
   `bench-index` / `bench-search`) and 1.63 / 1.65 (Chrome), against 1.03 / 1.01 in Firefox. The
   key scans are now tail-recursive functions over `Iterator.next` (`edgeStarting`,
   `edgeExtending`, `edgeStartingWith`), which compile to a `while` loop over `keys.next()`, the
   same work as `for...of`; the next bench run gave 1.03 / 1.19 (Node), 1.12 / 1.06 (Chrome) and
   0.90 / 0.96 (Firefox), with A/A between 0.90 and 1.01 (both runs at load averages of 17 to 53,
   so read them as rough).
2. **`String.charCodeAt` returns `option<int>`** in ReScript 12, and `==` on options compiles to
   `Primitive_object.equal`, a structural comparison. Code-unit comparisons use
   `String.charCodeAtUnsafe` (JS `charCodeAt`, every index is in range).
3. **Nullary constructors of a `@tag` variant are strings at run time**, so `| Stats` would never
   match the input object `{ "op": "stats" }` (the generated `switch (operation.op)` is only
   reached for objects). `stats` and `toJSON` carry an empty inline record: `Stats({})`.
4. **An optional record field set to `None` is emitted as `key: undefined`**, which the harness's
   strict canonical JSON tells apart from an absent key. Output records are therefore `dict`s
   that only receive the keys an operation produces, not records with optional fields.
5. **No `break`, `continue` or early `return`.** MiniSearch's labelled `continue key` in
   `recurse` is a `within` flag on the `while` loop; its `for...of` loops with an early return are
   tail-recursive functions (compiled to `while (true)` loops).
6. **Generic options box.** `Some(value)` for a type variable goes through
   `Primitive_option.some` (the nested-option encoding), so `SearchableMap` is generic in its
   value but its hot paths (`fetch`, the iterator's value) match the `Leaf` entry directly.
7. **Smaller points.** `string` cannot be coerced to `JSON.t` (`JSON.Encode.string` and
   `JSON.Encode.stringArray`, which are `%identity`, do it); constructors that only occur in
   decoded input trigger warning 37, silenced once in `Index.res`; `String.sliceToEnd` is
   deprecated in 12.3 in favour of `String.slice(~start)`; `/` on `int` compiles to
   `Primitive_int.div` (a division-by-zero check), avoided in the fuzzy loop by comparing row
   offsets with the matrix length.

## Size

`dist/index.min.js` is about 19 KB. Before Terser (Rollup's rendered bytes, `dist/meta.json`),
2.5 KB of the bundle is `@rescript/runtime` (`Stdlib_Option`, `Primitive_option`,
`Primitive_exceptions` for the exception, `Stdlib_JsError`, `Stdlib.panic`, `Stdlib_Array.last`,
`Stdlib_Bool`); the rest is the port. The exact figures are in `build/library-matrix/sizes.json`
and `MATRIX.md`.
