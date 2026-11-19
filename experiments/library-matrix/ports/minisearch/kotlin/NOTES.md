# MiniSearch 7.2.0: Kotlin/JS port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md` to Kotlin 2.4.10 (the
harness's Gradle production lane: whole-program ES module, ES2015, production DCE and member
minimisation). It follows `minisearch/dist/es/index.js` method by method, with no `js("…")`, no
handwritten JS, and `dynamic` only at the boundary. It reaches exact parity on all 58 corpus cases
in Node, Chrome and Firefox, float scores included.

| File | What it is |
| --- | --- |
| `src/SearchableMap.kt` | `SearchableMap`: the radix tree (nodes are `LinkedHashMap`s), `createPath`, `lookup`, `trackDown`, `remove`/`cleanup`/`merge`, `atPrefix`, `fetch`, the `_size` cache, MiniSearch's `TreeIterator` (key snapshots, newest first) and the Levenshtein walk of `fuzzySearch` |
| `src/MiniSearch.kt` | the `MiniSearch` class: add/remove/discard/replace, field lengths, vacuuming, search (option spreading, query combinations, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/Options.kt` | option, query and result types: `Value`, `Document`, `SearchOptions` and its spread, the sealed unions (`Prefix`, `Fuzzy`, `AutoVacuum`, `Query`), `SearchResult`, `Suggestion`, `AsPlainObject`, `MiniSearchException` |
| `src/Main.kt` | the boundary, doing what `libraries/minisearch/reference.mjs` does; `run` is the module's only export (`@JsExport`) |

## Interop decisions

- **Strings are Kotlin strings, which are JS strings.** UTF-16 code units throughout, with no
  conversion. `String.get` is `charCodeAt`, `length` and `substring` count code units, so radix
  edges split inside surrogate pairs, and Levenshtein distances and prefix weights count units, as
  in MiniSearch. Lone surrogates are ordinary units.
- **The host functions the contract allows, all through the standard library:**
  - `Regex("[\\n\\r\\p{Z}\\p{P}]+")`. Kotlin/JS builds it as `new RegExp(pattern, toFlags(options,
    "gu"))`, so the `u` flag that `\p{…}` needs is there; checked in the emit. The ES2015 target
    only affects emitted syntax: the engines' `RegExp` supports `\p{…}` either way.
  - `text.split(regex)`. This is the stdlib's own split over `findAll`, not `String.prototype.split`.
    It keeps a leading and a trailing empty token, and gives `[""]` for an empty text. For this
    pattern (no captures, never an empty match) that is exactly JS `split`: both cut at the
    leftmost maximal run, on code-point boundaries under `u` (`tokenizer-ascii`, `field-lengths`).
  - `lowercase()` is `toLowerCase()` (Final_Sigma and full mappings come from the engine).
  - `kotlin.math.ln` is `Math.log`. `roundToInt()` is `Math.round` (ties up); `kotlin.math.round`
    would round ties to even, so `Math.round(term.length * fuzzy)` is `roundToInt()`.
- **MiniSearch's `Map`s are `LinkedHashMap`s.** That covers radix nodes, posting lists,
  `_documentIds`, `_fieldLength`, `_storedFields`, result maps, the fuzzy result map, suggestions
  and each result's `match`. Kotlin/JS's `LinkedHashMap` keeps JS `Map` order exactly: a new key is
  appended, removal is in place, and setting an existing key keeps its position. `_idToShortId`,
  whose order is never observed, is a `HashMap`. The arrays with holes (`_avgFieldLength`, each
  document's field lengths) are `MutableList<T?>`, with null as a hole:
  - `setGrowing` is `array[i] = v` past the end;
  - `forEachIndexed` with a null check is `forEach`, which skips holes;
  - a hole read in arithmetic is NaN;
  - in `toJSON`, holes and NaN become `null`.
- **A radix node is `RadixTree<T> : LinkedHashMap<String, RadixEntry<T>>`**, and an entry is either
  a child `RadixTree` or a `Leaf(value)`. A JS `Map` holds both under one key type; Kotlin needs one
  value type, hence the sealed entry. The leaf is still the `""` key, so its position among the
  node's keys is observable, as in MiniSearch. The node subclasses the map rather than wrapping
  it, so a node is one object, as in MiniSearch.
- **The iterator is MiniSearch's `TreeIterator`**: a path of (node, key snapshot) steps, keys popped
  from the end, and each child looked up when the walk reaches its key. Deleting terms and merging
  nodes during a walk therefore interleaves exactly as in MiniSearch (vacuuming, search's cleanup
  of discarded postings, `x-prefix-walk-merge`), without collecting terms first. This affects:
  - the exact term's cleanup before `atPrefix` and `fuzzyGet` (`discard-exact-term-merge`);
  - the prefix walk's `fuzzyMatches.delete(term)`;
  - the frequency decrement (`discard-cleanup-on-search`).

  All three are literal.
- **Live `Map` iteration under deletion.** A JS `Map` iterator survives deletions. Kotlin's
  iterators throw `ConcurrentModificationException` on their next step after the map changes,
  except through their own `remove()`. So:
  - `termResults` iterates a snapshot of the posting list's short ids (`keys.toList()`). This is
    exact: the only change during that loop is `removeTerm` of the current posting (deleted, or
    decremented in place), so a live iteration would visit the same ids.
  - The vacuum deletes postings and fields through the iterators' `remove()`.
  - `createPath` abandons its key iterator right after a split (`continue@outer`), as MiniSearch
    does.
- **The fuzzy matrix** is a `ByteArray` with every read masked `and 0xff`, which stores and reads
  exactly as MiniSearch's `Uint8Array` does. `const dist = matrix[i] = Math.min(…)` keeps the
  assigned value, not the stored byte. The walk stops at the end of the matrix, which the contract
  allows, and here it is also required: a Kotlin/JS array read past the end is `undefined`, and
  `undefined and 0xff` is 0, a false distance. MiniSearch's read of `undefined` never matches.
- **Options are classes with nullable properties**, null meaning "not given", and
  `SearchOptions.plus` is `{ ...a, ...b }`. MiniSearch puts its own defaults under the given
  options (`{ ...defaultOptions, ...options }`, `defaultSearchOptions`, `defaultAutoSuggestOptions`),
  so the boundary maps the JSON one to one. Unions are sealed interfaces:
  - `Prefix.Flag` / `Prefix.Predicate` keeps `typeof prefix === 'function'`, and autoSuggest's
    default prefix is a `Predicate`;
  - `Fuzzy.Flag` / `Fuzzy.Amount` keeps `fuzzy === true ? 0.2 : fuzzy`;
  - `AutoVacuum.Flag` / `AutoVacuumOptions` lets the constructor resolve `true` and absent to the
    defaults.

  JS `x || fallback` on numbers is `orIfFalsy` (null, 0 and NaN are falsy). The merged options
  always carry the library defaults, so `options.maxFuzzy!!` and `options.bm25!!` never fire.
- **Values and ids.** `Value` is `Text`, `Number` or `Null`, and an absent field is Kotlin null. So
  `== null` (`isNullish`) and `!== undefined` (stored fields, `x-filter-null-vs-absent`) stay
  distinct. `Value.Number` compares by SameValueZero, with `-0` hashed as `0`, as JS `Map` keys and
  `includes` compare: a data class would use `Double.equals`, which tells `-0` from `0`. As `Map`
  keys, `1` and `"1"` are different documents (`id-types`).
- **Sorting** uses `byScore`, the sign of `b.score - a.score`, as `Array.prototype.sort` reads a
  comparator. `compareByDescending { it.score }` would be wrong at `-0` versus `0`, because
  `Double.compareTo` orders them. `sortWith` is stable: Kotlin/JS uses the engine's sort when it is
  stable and a merge sort otherwise.
- **Numbers.** `Int` holds the integral counts (document and dirt counts, short ids, field ids,
  frequencies, lengths). Kotlin's `Int` arithmetic compiles to `| 0`, which is exact at these
  sizes. Every division is a `Double` one (`dirtCount.toDouble() / (…)`; the BM25+ operands
  promote where JS has doubles, in the contract's operation order). Option numbers (`batchSize`,
  `minDirtCount`, `maxFuzzy`) are `Double`, as in JS. No `Long` anywhere. The `Long` class in the
  bundle is the stdlib's: `EmptyList`, `EmptyMap` and `EmptySet` carry `serialVersionUID`
  constants.
- **Errors.** MiniSearch's are `MiniSearchException` (a Kotlin `Exception`, so a JS `Error`), and
  `run` catches only those, per operation. `discardAll`'s `try/finally` is Kotlin's. A port bug or
  an input outside the parity domain (a boolean document value, a missing id) throws out of `run`.
- **Vacuuming in a synchronous run** follows the contract. `performVacuuming` runs its terms up to
  the first batch boundary and gives the dirt back only when the walk ends first.
  `currentVacuum` (`isVacuuming`) then stays true for the rest of the run, so later requests are
  never started: MiniSearch chains them behind a promise that settles after `run` returns. No
  promise or timer is created.
- **The boundary.** The input is typed by `external interface`s and read in place:
  - unions are read with `jsTypeOf` and `unsafeCast`;
  - a document is read through `asDynamic()[name]` in an extension operator, the documented
    replacement for `@nativeGetter`, so documents stay host objects;
  - the keys of a `boost` object come from the host `Object.keys`, bound with an
    `@JsName("Object") external object`. That is the only external declaration besides the input
    interfaces.

  The output is built with `kotlin.js.json()` (plain objects) and `Array`s (`toTypedArray`), never
  `IntArray`/`DoubleArray`, which are typed arrays and would fail the canonical form. `toJSON`'s
  integer-keyed objects get their keys as strings on a plain object, so the engine lists the
  array-index keys in ascending order, exactly as `Object.fromEntries` does. `Object.keys(match)`
  (`objectKeys`: array indices first, numerically, then insertion order) gives a hit's `terms` and
  its `match` pairs. `Object.assign` of two `match` objects is `putAll`, which keeps the relative
  order of non-index keys (`numeric-terms`).
- **No cross-call state.** Each `run` builds its own index from its own input and never writes
  the input. The top-level `val`s are MiniSearch's module constants: the default options,
  `createMap`, and the `Regex`, whose `lastIndex` Kotlin sets before every `exec`.
  `SearchableMap.cachedSize` is MiniSearch's own `_size`. The Kotlin stdlib keeps lazily computed
  runtime flags (for example, whether the engine's sort is stable), none derived from input. The
  module-state lint is 0.

## Kotlin/JS problems and less idiomatic choices

No compiler bug was found. Every construct compiled to the JS one expects, and production builds
are byte-identical on rebuild, including from `--clean`. These Kotlin/JS behaviours shaped the code:

1. **Iterators fail fast where JS `Map` iterators are live** (`ConcurrentModificationException`);
   see "Live `Map` iteration" above. This is why there is a snapshot in `termResults`, and why the
   vacuum uses `iterator.remove()`.
2. **Callable references allocate.** `::createMap` compiles to `constructCallableReference(…)`
   on every evaluation (twice per indexed term). A non-capturing lambda compiles to a plain
   function reference, hence `private val createMap: () -> FieldTermData = { LinkedHashMap() }`.
   That is also MiniSearch's own form (`const createMap = () => new Map()`).
3. **`data class` equality is `Double.equals`** (`-0 != 0`), and `compareTo` orders `-0` before
   `0`. JS keys, `includes` and `sort` do not distinguish them, hence the hand-written
   `Value.Number.equals`/`hashCode` and the sign comparator.
4. **`kotlin.math.round` rounds ties to even**, unlike `Math.round`; use `roundToInt()`.
5. **Out-of-bounds array reads are unchecked** (`undefined`), which is why the fuzzy walk must
   stop at the end of the matrix (above).
6. **Unions need sealed types** (`Prefix`, `Fuzzy`, `AutoVacuum`, `Value`, `Query`, radix entries),
   where JS tests `typeof`. Each adds a small wrapper object.

## Size

`dist/index.min.js` is 71,755 bytes (23,521 gzip-9, 20,518 brotli-11; `build/library-matrix/sizes.json`).
About 39% of the 230 KB pre-Terser program is the port's own classes and functions. The rest is
the Kotlin/JS stdlib it reaches:

- `ArrayList`;
- `HashMap`/`LinkedHashMap`/`HashSet`/`LinkedHashSet` over `InternalHashMap`, with their key,
  value and entry views and iterators;
- `Regex` with `findAll`'s `Sequence` machinery and `MatchResult`;
- the boxed `Long` (above);
- exceptions, `StringBuilder`, sorting, and the console output behind `println`, which MiniSearch's
  default logger uses.

The family's runtime floor is 46 bytes.

## Performance

Measured by `tools/bench.mjs` (triplet design, 24 samples, the bench variant, load average 7.6 on
10 cores; receipts under `build/library-matrix/bench/minisearch/`). These numbers are not a claim:
read each ratio next to its A/A.

| Environment | bench-index port/js | A/A | bench-search port/js | A/A |
| --- | --- | --- | --- | --- |
| Node 26.8.1 | 2.82 | 1.10 | 2.01 | 0.95 |
| Chrome 158 | 2.40 | 1.00 | 2.16 | 1.02 |
| Firefox 160 | 1.26 | 1.02 | 1.52 | 1.00 |

Most of the gap is Kotlin/JS's collections, which are userland code where MiniSearch uses the
engine's `Map`. A Node CPU profile of `bench-index` (`dist/index.js`) puts `addTerm` (`fetch` and
`createPath` on the radix nodes, then the posting-list updates) at 61% of the time. About 31% is
self time in `InternalHashMap`'s `findKey`/`addKey`, the generic `hashCode`/`equals` dispatch
(a `typeof` switch per call; `equals` tests every string key for a callable reference) and
`getStringHashCode`, which hashes a string key char by char on every lookup, with no cached hash.
Garbage collection is 9%, `Regex.split` 10%, and `toSet` for the field lengths 6%. `bench-search`
spreads the same costs over `termResults`, `fuzzySearch` and the small `match` maps.

The port keeps MiniSearch's maps rather than replacing them with sorted lists or arrays. That
would measure a workaround, not Kotlin's standard collections. The one change made for speed
removes waste no Kotlin developer intends: the callable reference above.
