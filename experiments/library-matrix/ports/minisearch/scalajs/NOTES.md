# MiniSearch 7.2.0: Scala.js port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` in Scala 3.8.4 for Scala.js
1.22.0, built by `languages/scalajs/build.mjs` (`fullLinkJS`, ES modules, common Terser). It has the
same algorithms and data structures as `minisearch/dist/es/index.js`, method by method, and reaches
exact parity, doubles included, on all 58 corpus cases in Node, Chrome and Firefox.

| File | What it is |
| --- | --- |
| `src/SearchableMap.scala` | `SearchableMap`: the radix tree (nodes as `mutable.LinkedHashMap`, the `""` leaf among their keys), `createPath`, `trackDown`/`lookup`, `remove`/`cleanup`/`merge`, `atPrefix`, MiniSearch's `TreeIterator` with its key snapshots, and the Levenshtein walk of `fuzzySearch` |
| `src/MiniSearch.scala` | the `MiniSearch` class: add, remove, discard, replace, field lengths, vacuuming, search (option merging, query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/Options.scala` | the option types and their spread merge (`a ++ b` is `{ ...a, ...b }`), MiniSearch's defaults |
| `src/Host.scala` | the tokenizer's host `RegExp` split, the port's only interop besides the boundary |
| `src/Main.scala` | the boundary: the port of `reference.mjs` (facade traits in, `js.Dictionary`/`js.Array` out) |

## Interop decisions

- **Strings are JS strings.** A Scala.js `String` is the host string, so `length`, `charAt`,
  `substring` and `startsWith` work on UTF-16 code units, as MiniSearch's radix tree, prefix weights
  and edit distances do, and lone surrogates pass through untouched. Nothing is converted.
- **Tokenizer: the host `RegExp`, through `js.RegExp` and `jsSplit`.** The contract's Scala.js
  guidance suggests `java.util.regex` (`split(re, -1)`), which translates `\p{Z}` and `\p{P}` only
  when the linker targets ES2018. The builder links for Scala.js's default ES version (ES2015), and
  there the pattern is refused at run time (repro below). So `Host.scala` holds MiniSearch's
  module-level `/[\n\r\p{Z}\p{P}]+/u` as a `js.RegExp` and calls the host `split` through Scala.js's
  standard facade (`JSStringOps.jsSplit`), which the contract allows ("A port may call the host").
  The result is the host array, seen as a Scala `IndexedSeq` (`js.WrappedArray`, no copy). A
  `//> using jsEsVersionStr es2018` directive in a source file makes `java.util.regex` work
  (checked: `split(re, -1)` then keeps the leading and trailing empty tokens). The port does not set
  one, because it would change the production variant the builder defines.
- **`toLowerCase()` is the host's.** Scala.js compiles `String.toLowerCase()` (no `Locale`) to JS
  `toLowerCase` (checked in the linked output), which has the full Unicode mappings and
  Final_Sigma.
- **`math.log` is `Math.log`** (checked in the linked output). It has to be the host's, since the
  engines disagree in the last bit (CONTRACT.md, "Floating point").
- **`Double.toString` is JS `Number#toString`** in Scala.js, so a numeric field value is indexed
  as `fieldValue.toString()` would index it (a safe integer as its plain digits).
- **The boundary.** `run(input: Input)` reads the input through `@js.native` facade traits, with
  no copy. Each operation's arguments become Scala values as the script runs, the way
  `reference.mjs` maps JSON onto the MiniSearch API: options and queries become `SearchOptions` and
  `Query`; the declarative `filter`, `boostDocument` and `boostTerm` become the closures they
  describe; ids become `DocId`. Documents stay host objects. `MiniSearch[D]` is generic in the
  document type, as MiniSearch itself is, and reads fields through its `extractField` option. The
  boundary passes `document[name]` (an `@JSBracketAccess` facade), each value read into a `Value`.
  The result is built as plain JS objects and arrays in the adapter's key order. In particular, a
  hit's stored fields are set after `match`, so a stored `id` replaces the id in place, as
  `Object.assign` does.

## Data structures and how they follow MiniSearch

- **Every JS `Map` is a `mutable.LinkedHashMap`**: radix nodes, posting lists, `fieldsData`,
  `_documentIds`, `_fieldLength`, `_storedFields`, result maps, `match`, fuzzy results and
  autoSuggest groups. They need insertion order and JS `Map`'s update rules: a new key is appended,
  `-=` leaves the others in place, and updating a key keeps its position. Replacing an edge (a
  split, a merge) is a set then a delete, so the new edge moves last. `_idToShortId` is only ever
  looked up, so it is a `mutable.HashMap`. Arrays with holes (`fieldLength` per document,
  `avgFieldLength`) are `ArrayBuffer[Option[_]]`, where a hole reads as `undefined` would: `NaN`
  in arithmetic, `0` under `|| 0`, `null` in `toJSON`.
- **Ids** are `type DocId = Int | String` (the contract's ids are strings and 32-bit integers), so
  `1` and `"1"` are different keys, as they are in a JS `Map`. An `Int` or `String` is a JS value
  in Scala.js, so ids cross the boundary unchanged.
- **Mutation during iteration.** MiniSearch deletes terms while a `TreeIterator` walks the tree
  (vacuuming, and search's cleanup of discarded postings). The port's `TreeIterator` is
  MiniSearch's own: it keeps a path of `(node, key snapshot)` pairs and pops each snapshot from its
  end. So the walk is exactly MiniSearch's, merges and detached nodes included, and terms are not
  collected first. Map loops that delete entries iterate over snapshots in Scala, because a Scala
  iterator is not specified to survive the deletion of its current entry. The vacuum's field and
  posting loops snapshot with `toArray`. Only the entry being visited is ever deleted, so this
  visits the same entries as MiniSearch's live JS iterators. In `termResults`, the hot path, the
  postings of discarded documents are collected during the posting-list loop. `removeTerm` then
  runs on them right after the loop, in the same order. The loop reads nothing that those removals
  change: `matchingFields` is decremented at the same point, and later documents read only their
  own postings. The exact term's cleanup still runs before `atPrefix` and `fuzzyGet` read the tree.
- **Vacuuming in a synchronous run** follows the contract. The first vacuum runs up to its first
  batch boundary. `isVacuuming` (`_currentVacuum != null`) then stays true, and every later
  request is deferred, which in a synchronous run means never.
- **Sorting** is `sortWith(a.score > b.score)`, Scala's stable sort with the IEEE comparison of
  `(a, b) => b.score - a.score`. `sortBy(-_.score)` would not do: `Ordering[Double]` puts `-0.0`
  before `0.0`, which JS treats as equal.
- **`Object.keys` order** (a hit's `terms` and `match` pairs, and `Object.assign` of `match`
  objects): `MiniSearch.ownKeys` puts array-index keys first, in numeric order, and the rest in
  insertion order.
- **The fuzzy matrix** is an `Array[Byte]` read back unsigned (`& 0xff`), which is MiniSearch's
  `Uint8Array`: every store is reduced modulo 256. Rows past the matrix stop the walk, which finds
  the same keys (CONTRACT.md, "Fuzzy").
- **Errors** are `MiniSearchError` with MiniSearch's messages. The boundary records them and the
  script goes on with the index as MiniSearch left it. Any other exception (input outside the
  parity domain, an unknown operation) propagates: the port fails fast instead of recording it.
- **No cross-call state.** `Main.run` builds a new `MiniSearch` from its own input on every call.
  Nothing is cached between calls or keyed on input identity, and the input is never written.

## Lint columns

- Embedded JS: 0. Eval references: 0.
- Module state: the heuristic flags 13 lines, all instance fields of classes that `run` creates
  for each call. None is in an `object`. The heuristic matches any `var`, or any `val` holding a
  `mutable.` collection, indented by at most two spaces, so it cannot tell a class body from an
  `object` body.
  - `MiniSearch.scala`, 12 lines: the fields of `final class MiniSearch[D]`, which are MiniSearch's
    own instance state (`autoVacuum`, `_index`, `_documentCount`, `_documentIds`, `_idToShortId`,
    `_fieldIds`, `_fieldLength`, `_avgFieldLength`, `_nextId`, `_storedFields`, `_dirtCount`, and
    `_currentVacuum` as `currentVacuum`).
  - `SearchableMap.scala`, 1 line: `knownSize`, MiniSearch's `_size` memo of one tree. The tree
    belongs to its `MiniSearch` instance, and every change to it resets the memo.

  The `object`s hold only immutable values: MiniSearch's defaults, the operator names and the
  combinator table, and the tokenizer's `RegExp`. That `RegExp` is a module-level constant in MiniSearch too, and `split`
  does not use its `lastIndex`.

## Verification beyond the harness

- **Differential fuzzing.** 3,300 random operation scripts inside the parity domain were run on the
  reference and the final artifact in Node, and every canonical output was identical. The scripts used 40 to
  150 operations over a vocabulary built for shared prefixes, numeric and array-index terms,
  surrogates, Σ/İ case mapping and `Object.prototype` names. They mixed add, addAll, remove of
  changed documents, discard, discardAll, replace, vacuum with small batches, autoVacuum with low
  thresholds, search with random options and nested trees, autoSuggest, toJSON, stats,
  duplicate ids and invalid operators.
- **Mutation check.** The fuzzer and the corpus both catch a port whose `TreeIterator` visits the
  keys of every node below the root oldest first: 38 of 58 corpus cases and 222 of 300 fuzz
  scripts fail.

## Toolchain and language problems

1. **`java.util.regex` with `\p{…}` fails at run time under the builder's settings.** This is not
   a compiler bug: Scala.js documents that Unicode character families need ES2018 `RegExp`
   features. But it contradicts the contract's Scala.js guidance as written, which assumes an
   ES2018 target. Minimal repro (`project.scala` as the builder writes it, plus):

   ```text
   object Main:
     private val separators = java.util.regex.Pattern.compile("[\\n\\r\\p{Z}\\p{P}]+")
     @JSExportTopLevel("run") def run(text: String) = separators.split(text, -1).mkString("|")
   ```

   It links without a warning and the module imports, but the first `run` throws while `Main`'s
   initialiser compiles the pattern:
   `java.util.regex.PatternSyntaxException: Unicode character family is not supported because it
   requires RegExp features of ECMAScript 2018. … near index 9`. The port uses `js.RegExp` instead
   (above).
2. **No compiler problems.** The code compiles without warnings, and rebuilds are byte-identical.
   Scala 3 union types (`Int | String`, `Boolean | Double`, `Boolean | TermPredicate`) and their
   type tests work as expected under Scala.js.

## Where the language forced a less idiomatic choice

- The tokenizer calls the host through `js.RegExp` instead of `java.util.regex` (problem 1).
- `Double.toString` relies on Scala.js's documented JS formatting. On the JVM it would print
  `1851.0`.
- The fuzzy matrix emulates a `Uint8Array` with `Array[Byte]` and `& 0xff`. An `Array[Int]` would
  be the plain Scala choice, and inside the parity domain it would give the same results.
- Numeric ids are `Int`. A number outside the 32-bit range is outside the parity domain and is
  rejected with an exception.

## Size and speed

- **Size.** `dist/index.min.js` is about 306 KB, and the readable linked output is 850 KB. The exact
  figures are in `build/library-matrix/sizes.json` and `MATRIX.md`. By class in the linked output,
  the port's own code is about 145 KB (17%). Most of the rest is the Scala collections library.
  Its shared default methods pull in classes the port hardly uses (its data structures are all
  mutable): `immutable.Vector` and its builders about 130 KB, `ArraySeq` 45 KB, immutable
  `Map`/CHAMP 55 KB, plus `LazyList`, views and `RTLong`. Two experiments made almost no difference, so the port
  keeps its original code. Replacing the immutable `Map` uses (the combinator table, `boost`'s
  `toMap`) saved 0.5%, and copying host arrays into `ArrayBuffer`s instead of `toSeq` saved 0.1%.
  For scale, a ten-line program that counts words in a `mutable.LinkedHashMap` and prints them with
  `ArrayBuffer.sortWith` and `mkString` links to 130 KB minified with the same settings. The
  README's note on Scala.js applies: the pilot is 193 KB, the floor 11 KB.
- **Speed.** The harness's first run (24 samples, the bench variant, triplet design, load average
  6 to 9 on 10 cores) measured port/js p50 as follows. The receipts under
  `build/library-matrix/bench/minisearch/` are authoritative.

  | Environment | `bench-index` port/js (A/A) | `bench-search` port/js (A/A) |
  | --- | --- | --- |
  | Node | 2.07 (1.00) | 1.56 (0.87) |
  | Chrome | 1.76 (1.00) | 1.45 (0.92) |
  | Firefox | 1.54 (1.02) | 2.13 (1.00) |

  By the Node profile, the time goes to:
  - the garbage collector (16–20%);
  - radix-node key scans (`keysIterator.find`);
  - `String.hashCode`, a Scala.js loop recomputed on every `String` key lookup (a JS `Map` hashes
    natively);
  - generic hashing of boxed keys.

  None of these can be avoided without leaving the standard collections.
