# GopherJS diff port

Go over GopherJS's `*js.Object` bindings. Text crosses the boundary as WTF-8 Go strings: `readText` encodes a JS string, isolated surrogates included, and `writeText` decodes it (`utf16.go`). Parsed and created patches keep their metadata in a host object built in jsdiff's key order.

- `diff.go`: the Myers search, the token policies and word.js's whitespace cleanup.
- `patch.go`: create.js, parse.js, apply.js, reverse.js and line-endings.js. Since the 2026-11-20 review it follows parse.js and apply.js step by step: `parseIndex` (with its early return at a second diff header), `gitNames` (parseGitDiffHeader), `quoted` (parseQuotedFileName), `parseFileHeader`, `parseHunk`, `apply` (applyStructuredPatch), `fit` (applyHunk) and `distance` (distanceIterator).
- `main.go`: the boundary.

The hunk range was a Go `regexp`, but RE2's `.` and `\s` differ from ECMAScript's. A function named after each of parse.js's patterns now matches it with ECMAScript semantics:

- `.` in `(.*)` stops at `\r`, U+2028 and U+2029 (`restOfLine`). `\d` is ASCII (`mode`, `digits`). `\s` and `trim` use the ECMAScript whitespace set (`space`). `\w` is ASCII.
- `hgPrefix` and `indexHeader` reproduce the greedy repetition of the Mercurial and `Index:` patterns, and its backtracking.
- `rangeIn` finds the leftmost match of the range pattern anywhere in the `@@` line. A line it does not match gives NaN starts.

Hunk ranges are `float64`s, JavaScript numbers (`strconv.ParseFloat` rounds as `Number()` does, and overflows to infinity). The positions `apply` computes from them are `float64`s too, so a NaN start fits nowhere, as in npm. Quoted names decode octal runs with the host TextDecoder, as jsdiff does.

## Deviations from the npm source

- **Bytes, not units.** Text is WTF-8 bytes. Where jsdiff counts UTF-16 units, the port does too: `units` for diffJson's longest-token rule, and `dropFirstUnit` and `dropLastUnit` for the one unit that parseGitDiffHeader skips beside a quoted name.
- **Runs instead of change objects.** `diff.go` and `create` restate base.js, word.js, line.js and create.js over runs (a kind and a count). The inner loop's diagonal bound is computed once per edit length. The diagonals that this adds after a path reaches the right edge can neither finish nor reach an edge, so the result is npm's.
- **Options.** Only the contract's options exist (CONTRACT.md, "Options"). `fit` allows no errors (fuzz 0). diffArrays compares the elements' JSON text, which for JSON scalars is `===`; under `ignoreCase` the text is lowercased, which for strings is `toLowerCase()` (the JSON text of a lowercased string is the lowercased JSON text).
- **reversePatch** copies the patch's own JS values, as `Object.assign` does, so missing hunk fields stay `undefined`.
- **canonicalize** assigns keys to a plain object, so a `"__proto__"` key is dropped, as in npm. It throws for a truthy `toJSON` member, which npm calls and JSON cannot make callable.
- **context.** `create` clamps patch creation's `context`, a JavaScript number, to the line count. That is exact: npm uses `context` only through `min`, `slice` and `<=`, where every value from the line count up acts alike.

## Module state

None.

## Performance accommodations

None.
