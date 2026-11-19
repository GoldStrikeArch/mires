# MiniSearch 7.2.0: Dart port (dart2js)

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` in Dart 3 (language version
3.13, the builder's `sdk: ^3.13.0`), compiled by `languages/dart/build.mjs` (Dart SDK 3.14.0,
`dart compile js -O2 --no-source-maps`, the comparator facade, common Terser). It has the same
algorithms and data structures as `minisearch/dist/es/index.js`, method by method. It reaches exact
parity, doubles included, on all 58 corpus cases in Node, Chrome and Firefox, for both
`dist/index.min.js` and `dist/index.js` (`check.mjs --readable`). Safari is `unavailable`.

| File | What it is |
| --- | --- |
| `src/searchable_map.dart` | `SearchableMap`: the radix tree (nodes as insertion-ordered `Map`s, the `''` leaf among their keys), `createPath`, `trackDown`/`lookup`, `remove`/`cleanup`/`merge`, `atPrefix`, MiniSearch's `TreeIterator` with its key snapshots, and the Levenshtein walk of `fuzzySearch` over a `Uint8List` |
| `src/mini_search.dart` | the `MiniSearch` class: add, remove, discard, replace, field lengths, vacuuming, search (option merging, query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJson`; the default tokenizer and term processor |
| `src/options.dart` | the option types (a class, records, typedefs), MiniSearch's defaults, the spread merge, and `FieldValue` (what a document field holds) |
| `src/main.dart` | the boundary, the port of `reference.mjs`: `dart:js_interop` extension types in, `JSObject` / JS arrays out; `main()` publishes `run` |

## Interop decisions

- **Strings are JS strings.** On the web a Dart `String` is the host string, so `length`,
  `codeUnitAt`, `substring` and `startsWith` work on UTF-16 code units, as MiniSearch's radix tree,
  prefix weights and edit distances do. Lone surrogates pass through untouched in both directions
  (`JSString.toDart` and `String.toJS` are casts in dart2js). Nothing is converted.
- **Tokenizer: a Dart `RegExp`, which is the host's.** `RegExp(r'[\n\r\p{Z}\p{P}]+', unicode: true)`
  compiles to a JS `RegExp` with the `u` flag. dart2js compiles `String.split` on a `RegExp` without
  capture groups to the host's `String.prototype.split` (`js_string.dart`), so leading and trailing
  empty tokens and `''.split(...)` → `['']` are JS semantics exactly. It is the standard library's
  own API, not hand-written interop.
- **`toLowerCase()` is the host's** (`JSString.toLowerCase` is `#.toLowerCase()`): full Unicode
  mappings and Final_Sigma. **`dart:math` `log` is `Math.log`** (`math_patch.dart`). It has to be
  the host's, because the engines disagree in the last bit (CONTRACT.md, "Floating point").
- **Numbers.** Every MiniSearch number is a double, and every operation is written in the
  contract's order. `calcBM25Score` takes `double`s (`int.toDouble()` is free in dart2js). Field
  values and numeric ids are read with `JSNumber.toDartDouble`, which is lossless. Printing them
  (`fieldValue.toString()`, `` `${id}` `` in messages) uses dart2js's `num.toString`, which is
  JavaScript's number formatting. The one difference is `-0`, printed `-0.0`, which the parity
  domain excludes.
- **The boundary.** `run(Input input)` reads the input through extension types with `external`
  getters, with no copy. Each operation's arguments become Dart values as the script runs, the
  way `reference.mjs` maps JSON onto the MiniSearch API: `SearchOptions` and `Query`; the
  declarative `filter`, `boostDocument` and `boostTerm` become the closures they describe; ids
  become `String` or `double`. Documents stay `JSObject`s. `MiniSearch<D>` is generic in the
  document type, as MiniSearch is, and reads fields through its `extractField` option. The boundary
  passes `document[name]` (`dart:js_interop_unsafe`), with the value read into a `FieldValue?`:
  - `undefined` becomes `null`;
  - JSON `null` becomes `NullValue`;
  - a string becomes `TextValue`, a number `NumberValue`.

  The result is built with `JSObject()` (`{}`) and `[]=`, keys in the adapter's order; a hit's
  stored fields are set after `match`, so a stored `id` replaces the id in place as
  `Object.assign` does. `toJSON` is MiniSearch's `toJSON` as a Dart `toJson()` (JSON-compatible
  maps and lists). The boundary turns it into JS with `JSON.parse(JSON.stringify(...))`'s number
  rules: NaN and the infinities become null, and `-0` becomes 0.
- **Type tests at the boundary** use `isA<JSString>()` / `isA<JSNumber>()` / `isA<JSBoolean>()`
  (`typeof`) and `isUndefined` / `isNull`. None of them depends on the realm the input object
  comes from. That is also why the port does not use `dartify()`: in dart2js it converts only
  objects whose prototype is the current realm's `Object.prototype` (`isJavaScriptSimpleObject` in
  `js_util_patch.dart`). The harness can hand Firefox an input from another realm (README,
  `rewritable`). The `boost` object's keys come from `Object.keys`, bound with
  `@JS('Object.keys') external`.
- **dart2js cannot emit ES module exports**: `main()` sets `globalThis.__matrixExports.run`
  (`globalContext`), which the builder's facade reads once and deletes.
- **Dart 3.14-only interop is avoided.** `ListOfStringToJSArray` (`List<String>.toJS`) and
  `JSArrayOfJSStringToList` are `@Since('3.14')`. The builder declares `sdk: ^3.13.0`, so string
  lists are converted element by element instead.

## Data structures and how they follow MiniSearch

- **Every JS `Map` is a Dart `Map`** (the default `{}` is a `LinkedHashMap`): radix nodes, posting
  lists, `fieldsData`, `_documentIds`, `_idToShortId`, `_fieldLength`, `_storedFields`, result
  maps, `match`, fuzzy results and autoSuggest groups. A `LinkedHashMap` has JS `Map`'s order
  rules:
  - a new key is appended;
  - `remove` leaves the others in place;
  - setting an existing key keeps its position.

  So replacing an edge (a split, a merge) is a set followed by a remove, and the new edge moves
  last, as in MiniSearch.

  Ids are `Object` keys (`String` or `double`), so `1` and `"1"` are different documents, as in a
  JS `Map`. Arrays with holes (`fieldLength` per document, `avgFieldLength`) are lists of nullable
  values. A hole reads as `undefined` would: NaN in arithmetic, 0 under `|| 0`, null in `toJson`.
- **Radix nodes.** A node maps keys to a sealed `_Entry`: a `_Node` under an edge label, or a
  `_Leaf<T>` under the leaf key `''`. The leaf has to be in the same ordered map, because its
  position among a node's keys is observable. `_Node` is not generic; only `SearchableMap<T>` and
  `_Leaf<T>` carry the value type (see "dart2js findings").
- **Mutation during iteration.** A Dart map iterator throws `ConcurrentModificationError` when its
  map changes, where a JS `Map` iterator survives the deletion of its current entry. So:
  - The tree walks interleaved with deletions (vacuuming, search's cleanup during the prefix walk)
    use the port of MiniSearch's own `TreeIterator`. It keeps a path of `(node, key snapshot)`
    pairs and pops each snapshot from its end, so the walk is exactly MiniSearch's, detached nodes
    and merges included.
  - The vacuum's field and posting loops iterate `toList()` snapshots. Only the visited entry is
    ever deleted, so they visit the same entries as MiniSearch's live iterators.
  - In `termResults`, the hot path, a posting list's discarded documents are collected during its
    loop, and `removeTerm` runs on them right after it, in the same order. The loop reads nothing
    those removals change: `matchingFields` is lowered at the same point, later postings are read
    only for their own documents, and the term can only lose its last field to them. The exact
    term's cleanup still runs before `atPrefix` and `fuzzyGet` read the tree
    (`discard-exact-term-merge`).
- **Vacuuming in a synchronous run** follows the contract. The first vacuum runs up to its first
  batch boundary. `isVacuuming` then stays true, and every later request is chained behind it,
  which in a synchronous run means never. Dart's `async` functions also start synchronously, so
  `performVacuuming` could be an `async` function with `await`s. The port does not do that, for
  two reasons. The continuation would run on timers after `run` returned, on an index nothing
  observes. And it would schedule dart2js's timer and microtask machinery in the checker's and the
  bench's realm. The contract leaves those asynchronous parts out.
- **Sorting.** `List.sort` is not stable in Dart (dart2js uses insertion sort up to 32 elements,
  dual-pivot quicksort above). `_sortedByScore` sorts positions with the comparator
  `sign(scores[b] - scores[a])`, then position. Equal scores, `-0` and `0` included, keep their
  order, exactly as JS's stable `sort(byScore)` does. Like JS, a NaN difference counts as equal.
  `double.compareTo` is not used: it orders `-0.0` before `0.0`. Each score is read once into a
  list, not on every comparison. This covers `search` and `autoSuggest` (`x-suggest-score-ties`,
  `score-ties`).
- **`Object.keys` order** (a hit's `terms` and `match` pairs, and `Object.assign` of `match`
  objects): `objectKeys` puts array-index keys first, in numeric order, and the rest in insertion
  order.
- **Option types.** `SearchOptions` is a class of nullable fields, and `merge` is the object
  spread. `weights`, `bm25`, `autoVacuum` and the vacuum options are records.
  - MiniSearch's `prefix: boolean | fn` is a `TermPredicate`. `true` and `false` are the constant
    predicates `everyTerm` and `noTerm`.
  - `fuzzy: boolean | number` is a `double`, with `false` as 0 and `true` as 0.2. That is what
    MiniSearch turns them into (`options.fuzzy || false`, then `fuzzy === true ? 0.2 : fuzzy`), so
    every search computes the same distance.
- **`removeAll()` and `removeAll(null)`.** MiniSearch tells them apart by `arguments.length`. Dart
  has no overloading, so the optional parameter's default is a private sentinel
  (`const _NoArgument()`): `removeAll()` resets the index, and an explicit `null` throws
  MiniSearch's error.
- **Errors** are `MiniSearchException`s with MiniSearch's messages. The boundary records them and
  the script goes on with the index as MiniSearch left it. An operation the adapter does not know
  is recorded the way the adapter records its `TypeError`. Anything else propagates, so the port
  fails fast.
- **No cross-call state.** `run` builds a new `MiniSearch` from its own input on every call.
  Nothing is cached between calls or keyed on input identity, and the input is never written.

## dart2js findings (no compiler bugs)

No miscompile or refusal was met. Every finding is a property of dart2js `-O2` that the port works
with.

1. **An `as` cast goes through the general runtime type check; an `is` test or type pattern is
   `instanceof`.** This holds even for a non-generic final class. Repro (`dart compile js -O2
   --no-minify`):

   ```dart
   sealed class E {}
   final class N extends E { final Map<String, E> entries = {}; }
   N asCast(N node, String k) => node.entries[k]! as N;            // type$.N._as(x): _generalAsCheckImplementation -> _isTestViaProperty
   N pattern(N node, String k) => switch (node.entries[k]) { final N c => c, _ => throw StateError('') }; // x instanceof A.N
   ```

   In the first version, `createPath`'s cast to the child node was about 11% of the port's
   indexing time. The port's node accesses are type patterns. Casts remain only where a generic
   leaf value is read (`_Leaf<T>`, once per value), where a pattern would cost the same reified
   check.
2. **Generic node classes cost reified type checks and type evaluation.** With `_Node<T>`, every
   cast and every `Map<String, _Entry<T>>` created in a generic function needed
   `_Universe_evalInEnvironment` and a type-argument check. The tree's structure is therefore
   non-generic, and only `SearchableMap<T>` and its leaves are typed.
3. **Implicit covariance checks stay at `-O2`.** Every `Map.[]=` checks its key and value against
   the map's type arguments (`_asInt` for `Map<int, int>`, `_Entry` for nodes). Together with
   dart2js's own `LinkedHashMap` (linked cells over `Object.create(null)` tables, not a JS `Map`),
   this is most of the remaining cost. It is language semantics (`-O3`'s `--omit-implicit-checks`
   would drop it, but that is not the production variant).
4. **`isUndefined` works after a `== null` test.** dart2js does not fold it, which was checked with
   a probe (`{a: null}`: `a` null, `b` undefined, both orders). The boundary still tests
   `isUndefined` first.
5. **`dartify()` depends on the realm** (see "Type tests at the boundary"). It was avoided, not met
   as a failure.

## Where the language forced a less idiomatic choice

- Snapshots (`toList()`) or deferred removals where MiniSearch deletes from a map it is iterating.
- A stable sort written by hand (positions plus a tie-break), since `List.sort` is not stable and
  `package:collection`'s `mergeSort` would add a pub dependency the builder fetches online.
- `HashSet<String>.of(tokens).length` for `new Set(tokens).size`: only the size is read, and
  dart2js's `LinkedHashSet` (what `toSet()` returns) allocates a linked cell per element.
- A sentinel default for `removeAll` (no overloading, no `arguments.length`).
- `FieldValue?` with `NullValue` to keep JSON `null` apart from `undefined`, which Dart does not
  have.
- Type patterns instead of casts for radix nodes, and a non-generic node class, for dart2js
  (findings 1 and 2). Both are ordinary Dart 3. The choice is made for the compiler, not the
  language.

## Lint columns

- Embedded JS: 0. Eval references: 0. Host JS: 0.
- Module state: 1. It is `final _spaceOrPunctuation = RegExp(...)` in `mini_search.dart`, the
  tokenizer's pattern: MiniSearch's own module-level constant `SPACE_OR_PUNCTUATION`. Dart has no
  `const` `RegExp`. It depends only on the pattern, never on input. dart2js initializes it lazily
  on first use, and the `RegExp` memoizes `_hasCapturesCache` for `split`. Neither is keyed on
  input or changes the output. Every other field is an instance field of a class that `run` creates
  for each call, and the other top-level declarations are `const` (defaults, operator names, the
  combinator table, the `removeAll` sentinel).

## Verification beyond the harness

- **Differential fuzzing.** A scratch fuzzer generated 3,000 random operation scripts inside the
  parity domain (two seeds, about 285,000 operations). It ran them on the reference and on the
  final dart2js output in Node, and every canonical output was identical. The scripts mixed:
  - add, addAll, remove of changed documents, discard, discardAll, replace, removeAll with and
    without documents and with null;
  - vacuum with batch sizes 0 to 100, and autoVacuum off, default, low thresholds and zero
    conditions;
  - search with random options and nested trees and wildcards, autoSuggest, has, getStoredFields,
    stats and toJSON;
  - a vocabulary built for shared prefixes, array-index and `Object.prototype` terms, surrogates
    and lone surrogates, and Σ/İ case mapping.
- **Mutation check.** A port whose `TreeIterator` visits every node's keys below the root oldest
  first fails 38 of 58 corpus cases and 264 of 300 fuzz scripts.
- **Reproducibility.** Two consecutive harness builds are byte-identical
  (`reproducibility.identical: true`).

## Size and speed

- **Size.** `dist/index.min.js` is 99,208 bytes (33,799 gzip-9, 29,377 brotli-11). dart2js emits
  112,079 bytes of program before Terser. The Dart floor (the identity program) is 38,726 bytes
  (13,118 gzip). So about 60 KB is the port, plus the runtime it reaches: `LinkedHashMap` /
  `HashSet`, `RegExp`, records and patterns, the RTI type system for generics, sorting, string
  buffers and interop. For scale, the JS reference bundle is 20,006 bytes (6,419 gzip). The exact
  figures are in `build/library-matrix/sizes.json` and `MATRIX.md`.
- **Speed.** The harness's run (24 samples, the bench variant, triplet design) measured port/js
  p50 as below. The receipts under `build/library-matrix/bench/minisearch/` are authoritative.

  | Environment | `bench-index` port/js (A/A) | `bench-search` port/js (A/A) |
  | --- | --- | --- |
  | Node 26 | 2.07 (1.00) | 1.60 (1.03) |
  | Chrome 158 | 1.85 (1.03) | 1.61 (1.02) |
  | Firefox 160 | 1.62 (0.95) | 2.14 (0.97) |

  By the Node profile, indexing time goes to dart2js's `LinkedHashMap` (cell allocation and
  `Object.create(null)` table writes, with the covariance checks above), then `createPath`'s key
  scans, then the field-length `HashSet`. The first version measured 2.55 and 1.89 in Node. Four
  ordinary changes brought it to the figures above:
  - iterate `keys` in `createPath`, as MiniSearch does, rather than `entries`, which allocates a
    `MapEntry` per key;
  - `HashSet` for distinct-token counts;
  - non-generic nodes with type patterns;
  - scores read once per sort.
