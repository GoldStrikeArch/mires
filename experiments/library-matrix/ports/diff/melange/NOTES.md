# Melange diff port

The OCaml core is shared with the js_of_ocaml port. `differ.ml`, `patch.ml` and `program.ml` are byte-identical in `ports/diff/melange/src` and `ports/diff/jsoo/src`. Only `host.ml` (the compiler's interop) and the entry module (`index.ml` here) differ. Text crosses the boundary as WTF-8 byte strings: `Host.read` encodes a JS string, isolated surrogates included, and `Host.str` decodes it. The host also supplies `toLowerCase`, `JSON.stringify`, TextEncoder-compatible quoting input (`toWellFormed`), TextDecoder and JS objects built in key order.

- `differ.ml`: the Myers search, the five token policies and word.js's whitespace cleanup.
- `patch.ml`: create.js, parse.js, apply.js, reverse.js and line-endings.js. Since the 2026-11-20 review it follows parse.js and apply.js step by step: `parse_index` (with its early return at a second diff header), `git_names` (parseGitDiffHeader), `quoted` (parseQuotedFileName), `parse_file_header`, `parse_hunk`, `apply` (applyStructuredPatch), `fit` (applyHunk) and `distance_iterator`.
- `program.ml`: the boundary.

OCaml has no regex engine here. A function named after each of parse.js's patterns matches it with the same semantics:

- `.` in `(.*)` stops at `\r`, U+2028 and U+2029 (`rest_of_line`). `\d` is ASCII (`mode`, `digits`). `\s` and `trim` use the ECMAScript whitespace set (`Differ.space`). `\w` is ASCII.
- `hg_prefix` and `index_header` reproduce the greedy repetition of the Mercurial and `Index:` patterns, and its backtracking.
- `hunk_range` finds the leftmost match of the range pattern anywhere in the `@@` line. A line it does not match gives NaN starts.

Hunk ranges are floats, JavaScript numbers. `apply` converts them to ints, within the bound in CONTRACT.md, "Domain bounds". Every loop over lines, hunks, files and octal escapes is a self tail call, which Melange compiles to a loop.

## Deviations from the npm source

- **Bytes, not units.** Text is WTF-8 bytes. Where jsdiff counts UTF-16 units, the port does too: `Differ.units` for diffJson's longest-token rule, and `drop_first_unit` and `drop_last_unit` for the one unit that parseGitDiffHeader skips beside a quoted name.
- **Runs instead of change objects.** `differ.ml` and `Patch.create` restate base.js, word.js, line.js and create.js over runs (a kind and a count). The inner loop's diagonal bound is computed once per edit length. The diagonals that this adds after a path reaches the right edge can neither finish nor reach an edge, so the result is npm's.
- **Options.** Only the contract's options exist (CONTRACT.md, "Options"). `fit` allows no errors (fuzz 0). diffArrays compares the elements' JSON text, which for JSON scalars is `===`.
- **reversePatch** copies the patch's own JS values, as `Object.assign` does, so missing hunk fields stay `undefined`.
- **canonicalize** assigns keys to a plain object, so a `"__proto__"` key is dropped, as in npm.
- **Layout.** `differ.ml` keeps the dense layout it was written in. `patch.ml` and `program.ml` were rewritten in the 2026-11-20 review.

## Module state

None.

## Performance accommodations

None.
