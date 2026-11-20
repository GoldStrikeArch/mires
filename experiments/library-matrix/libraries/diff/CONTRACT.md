# diff 9.0.0

The matrix uses the synchronous Myers differ and unified patch operations from the published `diff@9.0.0` ESM package. Each input selects one operation through `op`. Each call returns a fresh JSON value. Comparisons preserve the order of changes, flags, token counts and values, patch metadata, hunk ranges and contents, and every character of formatted patches. An unsuccessful patch application returns `false`. Invalid patch data may throw.

| Operation | Inputs | Result |
| --- | --- | --- |
| `diffChars`, `diffWords`, `diffLines` | String `before` and `after` | Array of change objects |
| `diffJson` | JSON documents in `before` and `after`, or strings containing already serialized documents | Array of line change objects |
| `diffArrays` | Arrays of JSON scalar values in `before` and `after` | Array of changes with array-valued contents |
| `structuredPatch`, `createTwoFilesPatch` | String `before`, `after`, `oldFileName`, `newFileName` and optional strings `oldHeader`, `newHeader` | Structured patch object or exact unified diff text |
| `parsePatch` | Unified diff text in `patch` | Array of file patch objects |
| `applyPatch` | Source string `before`, unified diff text or a structured patch in `patch` | Patched text or `false` |
| `reversePatch` | Structured patch object or array in `patch` | Reversed object or reversed array |

The text operations accept `options.ignoreCase` and `options.oneChangePerToken`. Line tokenization also accepts `stripTrailingCr`, `ignoreWhitespace`, `newlineIsToken` and `ignoreNewlineAtEof`. Patch creation accepts a nonnegative integer `context` with a default of four lines. The patch corpus covers unified file headers, ordinary and Git text patches, empty ranges, multiple hunks and files, shifted context, CRLF, missing final newlines, and malformed hunk counts. Patch application uses the default exact line comparison and zero fuzz. Callbacks, time limits, user-provided comparison functions and Intl segmenters are outside the synchronous JSON boundary.

Text values preserve ECMAScript UTF-16 code units, including isolated surrogates and leading U+FEFF. Git file names use the upstream byte rules. Formatting substitutes U+FFFD for isolated surrogates when encoding a quoted name. Parsing decodes each consecutive run of octal escapes as UTF-8, consuming a leading BOM for that run.

The two timing cases use revisions of a 2,000-line file with approximately 5% changed lines. The per-call variant changes line contents on both sides while preserving edit distances and hunk boundaries.
