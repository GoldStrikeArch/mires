# ReScript diff port

A hand-written ReScript port of jsdiff 9.0.0 over native JS strings, arrays and objects. On 2026-11-20 it replaced a translation of the OCaml port, which used OCaml standard-library shims and UTF-8 bytes packed into JS strings. The files follow npm's modules:

- `Diff.res`: `diff/base.js`, with the character, line, JSON and array diffs;
- `Words.res`: `diff/word.js` and the whitespace helpers of `util/string.js`;
- `Patch.res`: `patch/create.js`, `parse.js`, `apply.js`, `reverse.js`, `line-endings.js` and `util/distance-iterator.js`;
- `Host.res`: the host calls below;
- `Index.res`: the boundary. `Index.resi` exports `run` only.

## Boundary

The input record is the corpus object itself, with no conversion. Strings are JS strings, and JSON documents are `JSON.t` values. Records are the JS objects jsdiff returns, with keys in its order: change objects, hunks and structured patches, the last with optional fields written as `undefined`. Parsed patches are `dict`s built in the order jsdiff first assigns each property. A patch that does not apply returns `false`.

## Host facilities

ReScript's standard library binds the host directly, so the port uses it for:

- JS regex literals, written as jsdiff writes them;
- `String.trim` and `toLowerCase`;
- `Array.from` for code points;
- `JSON.stringify`, `Object.keys`, `Object.assign` (`Dict.copy`), `delete` (`Dict.delete`) and `===`.

`Host.res` adds:

- TextEncoder and TextDecoder, which jsdiff uses for quoted names;
- `Boolean`, for JavaScript truthiness;
- `String.prototype.match` with a global RegExp;
- identity casts between the JSON input and the records that have its shape.

## Deviations from the npm source

- Only the contract's options exist (CONTRACT.md, "Options"): no callbacks, `maxEditLength`, timeouts, comparators or segmenters. `applyHunk` allows no errors: npm's `maxErrors` is 0 when `fuzzFactor` is 0, which removes its recursion. `canonicalize` has no cycle stack, `toJSON` call or replacer, which JSON documents never need.
- npm's diff classes become one `diff` function and a `policy` record: token equality, `join`, and diffJson's longest-token rule.
- The Myers search keeps the best path for each diagonal in an array offset by `maxEditLength + 1`, not in a sparse JS array with negative indices. Its diagonal limits start at `±maxEditLength`, beyond which no diagonal lies, in place of `±Infinity`.
- Loops that npm leaves early with `return` or `break` are self-recursive functions, which ReScript compiles to `while` loops.
- `diffArrays` compares with `===` and has no `ignoreCase` (CONTRACT.md).
- Parsed hunk ranges are floats. Positions in `applyPatch` are ints, within the bound in CONTRACT.md, "Domain bounds".

## Module state

The RegExp literals of `Words.res`, `Diff.res` and `Patch.res`, which match jsdiff's module-level regexes. Two are global: the word tokenizer, which `match` runs from `lastIndex` 0, and the dangling-comma pattern, which `replace` runs from `lastIndex` 0. So no call leaves state for the next.

## Performance accommodations

None. `canonicalize` recurses from `for` loops, as npm's does. Recursing through `Array.map` and `forEach` callbacks adds SpiderMonkey's self-hosted frames to every nesting level, and it overflowed Firefox's stack at the contract's 1,000 levels.
