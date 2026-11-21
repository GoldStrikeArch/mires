# Scala.js diff port

A hand-written port of jsdiff 9.0.0, in Scala 3. It replaced a generated transliteration on 2026-11-20. The files follow npm's modules:

- `Diff.scala`: `diff/base.js`, with the character, line, JSON and array diffs;
- `Words.scala`: `diff/word.js` and the whitespace helpers of `util/string.js`;
- `Patch.scala`: `patch/create.js`, `parse.js`, `apply.js`, `reverse.js`, `line-endings.js` and `util/distance-iterator.js`;
- `Host.scala`: the host facilities below;
- `Main.scala`: the boundary.

## Boundary

`run(input)` reads the corpus object in place through `@js.native` facade traits. Strings are JS strings. JSON documents, array elements and the fields of structured patches stay host values. Results are `js.Dynamic.literal` objects and `js.Array`s in jsdiff's key order, and `false` for a patch that does not apply. Parsed patches keep their metadata in a `mutable.LinkedHashMap`, in the order jsdiff first assigns each property.

## Host facilities

Scala.js strings follow `java.lang.String`, whose `trim()` and `split()` differ from ECMAScript's. The port uses the host where jsdiff depends on ECMAScript behaviour:

- **Regexes.** jsdiff's patterns are `js.RegExp`s, written as jsdiff writes them, rather than `java.util.regex` translations. `jsSplit`, `jsReplace` and `match` (a facade) are the host's string methods.
- **Whitespace.** `String.prototype.trim` (a facade) and the ECMAScript `\s` set.
- **UTF-8.** TextEncoder and TextDecoder, which jsdiff itself calls for quoted names. Java's UTF-8 codec writes `?` for an isolated surrogate, and it does not replace each maximal ill-formed subpart with U+FFFD (the review's D6 finding).
- **JS operations.** `JSON.stringify`, `Object.keys`, `js.special.strictEquals` (`===`), `js.special.delete` and JavaScript truthiness. `toLowerCase` compiles to the host's.
- **Arrays.** Arrays are recognised with `js.Array.isArray`, as jsdiff recognises them with `Array.isArray` and the `[object Array]` tag. A `js.Array` type test compiles to `instanceof Array`, which depends on the value's realm and fails for the inputs Firefox's WebDriver page passes.

## Deviations from the npm source

- Only the contract's options exist (CONTRACT.md, "Options"): no callbacks, `maxEditLength`, timeouts, comparators or segmenters. `applyHunk` allows no errors: npm's `maxErrors` is 0 when `fuzzFactor` is 0, which removes its recursion. `canonicalize` has no cycle stack or replacer: JSON documents have no cycles, and the default replacer keeps every value. For a truthy `toJSON` member, which npm calls and JSON cannot make callable, it throws.
- The Myers search keeps the best path for each diagonal in an array offset by `maxEditLength + 1`, not in a sparse JS array with negative indices. `Int.MinValue` and `Int.MaxValue` stand for the unbounded diagonal limits.
- `diffArrays` compares with `===`, and under `ignoreCase` also by `toLowerCase()`, which the contract applies to string elements only.
- Hunk ranges and the positions `applyPatch` computes from them are `Double`s, JavaScript numbers, so a NaN start fits nowhere, as in npm. So is patch creation's `context`, which npm compares and slices as a double.
- Early exits from loops use `while` loops and local `return`s, so no non-local return becomes an exception.

## Module state

The `js.RegExp` constants of `Words.scala`, `Diff.scala` and `Patch.scala`, and the `CharacterDiff`, `WordDiff`, `LineDiff`, `JsonDiff` and `ArrayDiff` objects. They match jsdiff's module-level regexes and diff instances. None of the regexes is global except two: the word tokenizer, which `match` runs from `lastIndex` 0, and the dangling-comma pattern, which `replace` runs from `lastIndex` 0. So no call leaves state for the next.

## Performance accommodations

None. `canonicalize` recurses from `while` loops, as npm's does from `for` loops, so each nesting level costs one stack frame. Firefox holds the contract's 1,000 levels.
