# diff 9.0.0

The matrix uses the synchronous Myers differ and unified patch operations from the published `diff@9.0.0` ESM package. Each input selects one operation through `op`. Each call returns a fresh JSON value. Comparisons preserve the order of changes, flags, token counts and values, patch metadata, hunk ranges and contents, and every character of formatted patches. An unsuccessful patch application returns `false`. Invalid patch data may throw.

| Operation | Inputs | Result |
| --- | --- | --- |
| `diffChars`, `diffWords`, `diffLines` | String `before` and `after` | Array of change objects |
| `diffJson` | JSON documents in `before` and `after`, or strings containing already serialized documents | Array of line change objects |
| `diffArrays` | Arrays of JSON scalar values in `before` and `after` | Array of changes with array-valued contents |
| `structuredPatch`, `createTwoFilesPatch` | String `before`, `after`, `oldFileName`, `newFileName` and optional strings `oldHeader`, `newHeader` | Structured patch object or exact unified diff text |
| `parsePatch` | Unified diff text in `patch` | Array of file patch objects |
| `applyPatch` | Source string `before`, unified diff text, a structured patch or an array of them in `patch` | Patched text or `false` |
| `reversePatch` | Structured patch object or array in `patch` | Reversed object or reversed array |

## Options

The text operations accept `options.ignoreCase` and `options.oneChangePerToken`. Line tokenization also accepts `stripTrailingCr`, `ignoreWhitespace`, `newlineIsToken` and `ignoreNewlineAtEof`. `diffArrays` accepts `oneChangePerToken` and `ignoreCase`. With `ignoreCase`, npm compares two unequal elements by `toLowerCase()`, so the elements must be strings: for numbers, booleans and null that call throws, and arrays holding them are outside the domain under `ignoreCase`.

Patch creation accepts a nonnegative integer `context` with a default of four lines, and no other option. `context` is a JavaScript number: npm compares and slices with it as a double, so values past int32, up to `Number.MAX_SAFE_INTEGER` and beyond (whole-file context), keep one hunk. npm passes the whole options object on to `diffLines`, so `ignoreWhitespace`, `ignoreCase`, `stripTrailingCr` and `ignoreNewlineAtEof` would change which lines a patch keeps, and `newlineIsToken` throws; those are outside the domain. `applyPatch` takes no options: `fuzzFactor` (including npm's check that it is a nonnegative integer), `compareLine` and `autoConvertLineEndings: false` are outside the domain. Every port applies with npm's defaults: exact line comparison, zero fuzz and automatic line-ending conversion. Callbacks, `maxEditLength`, time limits, user-provided comparison functions, `diffWords`'s `ignoreWhitespace` (which selects `diffWordsWithSpace`) and Intl segmenters are outside the synchronous JSON boundary.

## Text and numbers

Text values preserve ECMAScript UTF-16 code units, including isolated surrogates and leading U+FEFF; positions, lengths and the longest-line rule of `diffJson` count UTF-16 units. `\s`, `trim` and the word tokenizer use the ECMAScript whitespace set. Git file names use the upstream byte rules. Formatting substitutes U+FFFD for isolated surrogates when encoding a quoted name (TextEncoder). Parsing decodes each consecutive run of octal escapes as UTF-8 with TextDecoder: each maximal ill-formed subpart becomes U+FFFD and a leading BOM of the run is consumed.

`parsePatch` matches header lines with jsdiff's regexes, so `.` stops at `\r`, U+2028 and U+2029, `\d` is ASCII, and the hunk range is the leftmost match of `@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@` anywhere in an `@@` line. Hunk ranges are JavaScript numbers: digits past 2^31 and 2^53 round as `Number()` rounds them, a run too long for a double is `Infinity`, and an `@@` line the range pattern does not match gives `NaN` starts, all as npm returns them.

## JSON documents

npm's `canonicalize` calls an object's `toJSON` member when it is truthy, at any depth. JSON cannot make that member callable, so for such a document `diffJson` throws, as npm does; a falsy `toJSON` (`false`, `0`, `""`, `null`) is an ordinary member.

## Domain bounds

- **JSON nesting.** `diffJson` documents nest at most 1,000 arrays or objects deep. npm's `canonicalize` and `JSON.stringify` recurse once per level. Measured on 2026-11-20, npm handles 4,784 nested objects and 5,155 nested arrays in Node, 3,844 in Chrome and 3,162 in Firefox (both inside a WebDriver script). The checker's `structuredClone` of each input is a tighter limit than npm's. The corpus checks the bound with arrays and with objects.
- **Hunk positions in `applyPatch`.** Applied hunks have an `oldStart` that is `NaN` or an integer below 2^31 in magnitude, and an integer `oldLines` of the same size, as `structuredPatch` writes them and `parsePatch` reads them. npm's search for a fitting position, and its padding when a hunk starts past the end of the source, are linear in the distance between the hunk's start and the source, so a larger start is impractical for any implementation (and from an `Infinity` start npm's search never ends). A `NaN` start comes from a header the range pattern does not match: if the hunk contains any removed or context line, it fits at no position and `applyPatch` returns `false`; if the hunk is insert-only (`+` lines only), it fits at `NaN`, copies no source lines, and returns the inserted lines, as npm does.
- **Structured patches.** `applyPatch` and `reversePatch` take structured patches whose `hunks` is an array of objects with a `lines` array of strings. `reversePatch` copies every other field as it is, as npm does, including missing hunk fields (which it writes as `undefined`). Git file names in reversed patches are strings or absent.

## Corpus

The patch corpus covers unified file headers (with tabs, quoted names, ECMAScript whitespace and empty names), ordinary and Git text patches, Git extended headers with carriage returns, U+2028, U+2029 and non-ASCII digits, quoted Git names split on UTF-16 units, `Index:` and Mercurial `diff -r` headers (names with spaces, tabs and the regex's backtracking), CRLF-terminated patches (Git mode, rename and index headers and CRLF hunk headers, through parse and apply), headings and text around hunk ranges, ranges past int32 and 2^53 and unmatched ranges (also through apply), empty ranges, multiple hunks and files, a second diff header that ends a file early, shifted context, out-of-order hunks, hunks past the end of the source and at line zero, missing final newlines, malformed hunk counts, invalid UTF-8 in octal escapes, 3,000-file, 5,000-hunk and 20,000-line patches, and `diffJson` at the nesting bound. The other cases cover `stripTrailingCr` with `\r\r\n`, `context` past int32 and at `Number.MAX_SAFE_INTEGER`, truthy and falsy `toJSON` members, `diffArrays` with `ignoreCase`, and `reversePatch` with truthy non-boolean `isGit` and `isCopy`.

The two timing cases use revisions of a 2,000-line file with approximately 5% changed lines. The per-call variant changes line contents on both sides while preserving edit distances and hunk boundaries.
