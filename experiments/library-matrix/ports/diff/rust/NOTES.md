# Rust diff port

The Myers search keeps the furthest old-file position on each diagonal and shares edit histories through a per-call arena. The separate tokenizers preserve Unicode code points, duplicated whitespace around word tokens, line endings, and typed array elements. Patch generation, parsing, reversal, exact hunk fitting and offset search run in Rust. The boundary uses JavaScript property access and JSON encoding for JSON input, and keeps the package's property insertion order and explicit `undefined` values on output.

## Patch parsing and application

Since the 2026-11-20 review, `patch.rs` follows parse.js and apply.js step by step:

- parse.js: `parse_index`, including its early return at a second diff header, then `git_names` (parseGitDiffHeader), `quoted` (parseQuotedFileName), `parse_file_header` and `parse_hunk`;
- apply.js: `apply` (applyStructuredPatch) with the line-ending conversion of line-endings.js, then `fit` (applyHunk) and `Distance` (distanceIterator).

The port has no regex engine. A function named after each of parse.js's patterns matches it with the same semantics:

- `.` in `(.*)` stops at `\r`, U+2028 and U+2029 (`rest_of_line`). `\d` is ASCII (`mode`, `digits`). `\s` and `trim` use the ECMAScript whitespace set (`diff::ws`). `\w` is ASCII.
- `hg_prefix` and `index_header` reproduce the greedy repetition of `/^diff(?: -r \w+)+\s/` and `/^(?:Index:|diff(?: -r \w+)+)\s+/`, and its backtracking.
- `hunk_range` finds the leftmost match of the range pattern anywhere in the `@@` line. A line it does not match gives NaN starts.

Hunk ranges, and the positions `apply` computes from them, are `f64`s, JavaScript numbers, so a NaN start fits only insert-only hunks (and copies no source lines), while any hunk with a removed or context line returns `false`, as in npm.

## Deviations from the npm source

- **Strings.** Text is Rust UTF-8, with an escape for isolated UTF-16 surrogates: U+E800 and a private-use payload (`utf16.rs`). Where jsdiff counts UTF-16 units, so does the port: `units` for diffJson's longest-token rule, and `drop_first_unit` and `drop_last_unit` for the one unit that parseGitDiffHeader skips beside a quoted name.
- **Runs instead of change objects.** The Myers search, the tokenizers and patch creation restate base.js, word.js, line.js and create.js over runs (a kind and a count). The inner loop's diagonal bound is computed once per edit length. The diagonals that this adds after a path reaches the right edge can neither finish nor reach an edge, so the result is npm's.
- **diffJson's dangling commas.** The rule removes `,\r` and `,\n` in two passes, which equals the single regex replacement.
- **Options.** Only the contract's options exist (CONTRACT.md, "Options"). `fit` allows no errors (fuzz 0). diffArrays compares the elements' JSON text, which for JSON scalars is `===`; under `ignoreCase`, elements are strings (CONTRACT.md) and compare by the host's `toLowerCase()`.
- **reversePatch** copies the patch's own JS values, as `Object.assign` does, so missing hunk fields stay `undefined`.
- **canonicalize** assigns keys to a plain object, so a `"__proto__"` key is dropped, as in npm. It throws for a truthy `toJSON` member, which npm calls and JSON cannot make callable.

## Compiler workarounds

Two previously tracked compiler defects recurred during the port.

- `WORKAROUND(MIRES-019)` in `src/patch.rs` compares a copied opening quote and queries the final source line directly. Promoted `Some(&b'"')` and `Some(&"")` comparisons produced incorrect results for quoted file names and EOF-newline removal. [Promoted-reference reproducer](../../../mires-bugs/MIRES-019-option-ref-const-decodes-none/)
- `WORKAROUND(MIRES-017)` in `src/patch.rs` decodes the validated UTF-8 prefix and emits a replacement character for each invalid sequence. The original `String::from_utf8_lossy` made a valid quoted name throw on the return path of `Utf8Chunks::next`. The generated switch assigned the successful result and fell through into its default `bad block 1` exception. [Structurizer reproducer](../../../mires-bugs/MIRES-017-switch-last-case-missing-break/)
