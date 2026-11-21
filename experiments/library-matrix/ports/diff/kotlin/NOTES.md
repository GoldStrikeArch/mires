# Kotlin/JS diff port

A hand-written port of jsdiff 9.0.0. It replaced a generated transliteration on 2026-11-20. The files follow npm's modules:

- `Diff.kt`: `diff/base.js`, with the character, line, JSON and array diffs;
- `Words.kt`: `diff/word.js` and the whitespace helpers of `util/string.js`;
- `Patch.kt`: `patch/create.js`, `parse.js`, `apply.js`, `reverse.js`, `line-endings.js` and `util/distance-iterator.js`;
- `Main.kt`: the boundary.

## Boundary

`run(input)` reads the corpus object in place through external interfaces. Strings are JS strings. JSON documents, array elements and the fields of structured patches stay host values. Results are `kotlin.js.json` objects and arrays in jsdiff's key order, and `false` for a patch that does not apply. Parsed patches keep their metadata in a `LinkedHashMap`, in the order jsdiff first assigns each property.

## Host facilities

- **Regexes.** parse.js's line patterns and word.js's tokenizer are `kotlin.text.Regex`, which Kotlin/JS compiles to a host `RegExp` with the `u` flag. jsdiff's tokenizer has `u` itself. For parse.js's patterns (`^rename from (.*)`, `(\d+)`, `\s`, the hunk range) the flag changes no match.
- **Whitespace.** JavaScript `trim` and the `\s` set are in `JsInterop.kt`. Kotlin's `trim()` and `isWhitespace()` use a different set: they include U+001C to U+001F and exclude U+FEFF.
- **JS globals.** `JSON.stringify`, `Object.keys`, `Array.isArray`, `Boolean` (for truthiness) and `Reflect.deleteProperty`. `lowercase()` compiles to the host's `toLowerCase`.
- **UTF-8.** `encodeToByteArray()` writes U+FFFD for an isolated surrogate, as TextEncoder does. `decodeToString()` writes U+FFFD per maximal ill-formed subpart, as TextDecoder does, and the port drops the leading BOM that TextDecoder consumes.

## Deviations from the npm source

- Only the contract's options exist (CONTRACT.md, "Options"): no callbacks, `maxEditLength`, timeouts, comparators or segmenters. `applyHunk` allows no errors: npm's `maxErrors` is 0 when `fuzzFactor` is 0, which removes its recursion. `canonicalize` has no cycle stack or replacer: JSON documents have no cycles, and the default replacer keeps every value. For a truthy `toJSON` member, which npm calls and JSON cannot make callable, it throws.
- The Myers search keeps the best path for each diagonal in an array offset by `maxEditLength + 1`, not in a sparse JS array with negative indices. `Int.MIN_VALUE` and `Int.MAX_VALUE` stand for the unbounded diagonal limits (`-Infinity`, `Infinity`).
- `diffArrays` compares with `===`, and under `ignoreCase` also by `toLowerCase()`, which the contract applies to string elements only.
- Hunk ranges and the positions `applyPatch` computes from them are `Double`s, JavaScript numbers, so a NaN start fits nowhere, as in npm. So is patch creation's `context`, which npm compares and slices as a double.

## Module state

The compiled `Regex` constants of `Words.kt` and `Patch.kt`, and the `CharacterDiff`, `WordDiff`, `LineDiff`, `JsonDiff` and `ArrayDiff` objects. They match jsdiff's module-level regexes and diff instances, and none changes after it is built.

## Performance accommodations

None.
