# Elm diff port

Elm with a JS host (`js/index.js`). The host only relays `{ input, undefined }` and the response through ports, passing JavaScript's real `undefined` as an opaque value so Elm emits optional patch fields directly without a sentinel or post-pass walk. Strings are JS strings, and Elm's `String` functions count UTF-16 units, as jsdiff does. `Utf16.toList` splits a string at code-unit boundaries where an isolated surrogate would confuse Elm's character folds.

- `Differ.elm`: the Myers search, the token policies and word.js's whitespace cleanup.
- `Patch.elm`: create.js, parse.js, apply.js, reverse.js and line-endings.js. Since the 2026-11-20 review it follows parse.js and apply.js step by step: `parseIndex`, `metadata` (with its early return at a second diff header), `gitNames` (parseGitDiffHeader), `quoted` (parseQuotedFileName), `fileHeader`, `parseHunk`, `apply` (applyStructuredPatch), `fit` (applyHunk) and `step` (distanceIterator).
- `Main.elm`: the boundary.

Elm has no regex in `elm/core`. A function named after each of parse.js's patterns matches it with the same semantics:

- `.` in `(.*)` stops at `\r`, U+2028 and U+2029 (`restOfLine`). `\d` is ASCII (`mode`, `digits`). `\s` is tested with `String.trim`, which is the host's `trim`. `\w` is ASCII.
- `hgPrefix` and `indexHeader` reproduce the greedy repetition of the Mercurial and `Index:` patterns, and its backtracking.
- `hunkRange` finds the leftmost match of the range pattern anywhere in the `@@` line. A line it does not match gives NaN starts (`0 / 0`).

Hunk ranges are `Float`s, JavaScript numbers (`String.toFloat`), and so are the positions `apply` computes from them, so a NaN start fits only insert-only hunks (and copies no source lines), while any hunk with a removed or context line returns `false`, as in npm.

## Stack safety

Every loop over lines, hunks, files and octal bytes is a self tail call, which Elm compiles to a loop, so parsing does not recurse once per hunk or file (the review's D7 finding). Checked against npm with 10,000 files, 20,000 hunks, a 50,000-line hunk, 30,000-line applications (shifted, failing and CRLF) and 5,000 octal runs in one name. The corpus has 3,000 files, 5,000 hunks and a 20,000-line hunk.

`diffJson`'s `canonical` visits the document with an explicit stack of open arrays and objects in a loop, where npm's `canonicalize` recurses once per nesting level. The recursive version it replaced overflowed below the contract's bound of 1,000 levels on a cold call: about 650 nested objects and 950 nested arrays in Node, before the JIT had optimised it. Only the host's `JSON.stringify` now recurses, as in npm, and a cold call handles 4,500 levels of either kind in Node.

## Deviations from the npm source

- **canonicalize** is a loop over an explicit stack rather than recursion (see "Stack safety"). It visits members in the same order and assigns the sorted keys to a plain object, as npm does. It fails for a truthy `toJSON` member, which npm calls and JSON cannot make callable, so `run` throws.
- **Runs instead of change objects.** `Differ.elm` and `Patch.create` restate base.js, word.js, line.js and create.js over runs (a kind and a count), in immutable lists and arrays.
- **Metadata.** A parsed patch's metadata is a list of fields in the order jsdiff first assigns them; `put` replaces a field in place, as assignment does.
- **UTF-8.** `decodeUtf8` is TextDecoder written out: each maximal ill-formed subpart becomes U+FFFD and a leading BOM is consumed. Elm has no host TextDecoder without a port round trip.
- **Options.** Only the contract's options exist (CONTRACT.md, "Options"). `fit` allows no errors (fuzz 0). diffArrays compares the elements' JSON text, which for JSON scalars is `===`; under `ignoreCase`, elements are strings (CONTRACT.md) and compare by `String.toLower` (JS `toLowerCase()`).
- **reversePatch** copies the patch's own JSON fields, as `Object.assign` does, so missing hunk fields stay `undefined`. It tests `isGit` and `isCopy` for JavaScript truthiness (`truthy`), as npm does.

## Module state

None (the host keeps the Elm worker and its response slot between calls).

## Performance accommodations

None.
