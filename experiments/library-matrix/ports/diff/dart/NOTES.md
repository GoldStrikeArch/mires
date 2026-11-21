# Dart diff port

A hand-written port of jsdiff 9.0.0. It replaced a transliteration on 2026-11-20 that the same generator produced as the Kotlin and Scala.js ports. The files follow npm's modules:

- `diff.dart`: `diff/base.js`, with the character, line, JSON and array diffs;
- `words.dart`: `diff/word.js` and the whitespace helpers of `util/string.js`;
- `patch.dart`: `patch/create.js`, `parse.js`, `apply.js`, `reverse.js`, `line-endings.js` and `util/distance-iterator.js`;
- `host.dart`: the host facilities below;
- `main.dart`: the boundary.

## Boundary

`run(input)` reads the corpus object in place through `dart:js_interop` extension types. Strings are JS strings: on the web a Dart `String` is one, UTF-16, with no conversion. JSON documents, array elements and the fields of structured patches stay JS values. Results are `JSObject`s and `List.toJS` arrays in jsdiff's key order, and `false` for a patch that does not apply. Parsed patches keep their metadata in an insertion-ordered `Map`, in the order jsdiff first assigns each property. `main()` publishes `run` on `globalThis.__matrixExports` for the generated facade.

## Host facilities

- **Regexes.** jsdiff's patterns are Dart `RegExp`s, which are host RegExps on the web. `unicode: true` is jsdiff's `u` flag on the word tokenizer.
- **Whitespace.** ECMAScript's `\s` and `trim` are written out in `host.dart`, because Dart's `String.trim` also strips U+0085.
- **UTF-8.** TextDecoder decodes octal escapes, as jsdiff does. It replaces each maximal ill-formed subpart with U+FFFD and consumes a leading BOM, and the port does not assume that `dart:convert`'s decoder does both. Quoting uses `utf8.encode`, which writes U+FFFD for an isolated surrogate, as TextEncoder does.
- **JS operations.** `JSON.stringify`, `Object.keys`, `Boolean` (for truthiness), `Reflect.deleteProperty`, the `undefined` value and `strictEquals` (`===`). `toLowerCase` compiles to the host's.

## Deviations from the npm source

- Only the contract's options exist (CONTRACT.md, "Options"): no callbacks, `maxEditLength`, timeouts, comparators or segmenters. `applyHunk` allows no errors: npm's `maxErrors` is 0 when `fuzzFactor` is 0, which removes its recursion. `canonicalize` has no cycle stack or replacer: JSON documents have no cycles, and the default replacer keeps every value. For a truthy `toJSON` member, which npm calls and JSON cannot make callable, it throws.
- The Myers search keeps the best path for each diagonal in a list offset by `maxEditLength + 1`, not in a sparse JS array with negative indices. Its diagonal limits start at `±maxEditLength`, beyond which no diagonal lies, in place of `±Infinity`.
- `diffArrays` compares with `===`, and under `ignoreCase` also by `toLowerCase()`, which the contract applies to string elements only.
- Hunk ranges and the positions `applyPatch` computes from them are `double`s, JavaScript numbers, so a NaN start fits nowhere, as in npm. So is patch creation's `context`, which npm compares and slices as a double.

## Module state

The `RegExp` constants of `words.dart`, `diff.dart` and `patch.dart`, which match jsdiff's module-level regexes. None changes after it is built.

## Performance accommodations

None.
