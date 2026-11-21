# Melange diff port

The OCaml core is shared with the js_of_ocaml port. `differ.ml`, `patch.ml` and `program.ml` are byte-identical in `ports/diff/melange/src` and `ports/diff/jsoo/src`. Only `host.ml` (the compiler's interop) and the entry module (`index.ml` here) differ. Text crosses the boundary as WTF-8 byte strings: `Host.read` encodes a JS string, isolated surrogates included, and `Host.str` decodes it. The host also supplies `toLowerCase`, `JSON.stringify`, TextEncoder-compatible quoting input (`toWellFormed`), TextDecoder and JS objects built in key order.

- `differ.ml`: the Myers search, the five token policies and word.js's whitespace cleanup.
- `patch.ml`: create.js, parse.js, apply.js, reverse.js and line-endings.js. Since the 2026-11-20 review it follows parse.js and apply.js step by step: `parse_index` (with its early return at a second diff header), `git_names` (parseGitDiffHeader), `quoted` (parseQuotedFileName), `parse_file_header`, `parse_hunk`, `apply` (applyStructuredPatch), `fit` (applyHunk) and `distance_iterator`.
- `program.ml`: the boundary.

OCaml has no regex engine here. A function named after each of parse.js's patterns matches it with the same semantics:

- `.` in `(.*)` stops at `\r`, U+2028 and U+2029 (`rest_of_line`). `\d` is ASCII (`mode`, `digits`). `\s` and `trim` use the ECMAScript whitespace set (`Differ.space`). `\w` is ASCII.
- `hg_prefix` and `index_header` reproduce the greedy repetition of the Mercurial and `Index:` patterns, and its backtracking.
- `hunk_range` finds the leftmost match of the range pattern anywhere in the `@@` line. A line it does not match gives NaN starts.

Hunk ranges, and the positions `apply` computes from them, are floats, JavaScript numbers, so a NaN start fits nowhere, as in npm. Every loop over lines, hunks, files and octal escapes is a self tail call, which Melange compiles to a loop.

## Deviations from the npm source

- **Bytes, not units.** Text is WTF-8 bytes. Where jsdiff counts UTF-16 units, the port does too: `Differ.units` for diffJson's longest-token rule, and `drop_first_unit` and `drop_last_unit` for the one unit that parseGitDiffHeader skips beside a quoted name.
- **Runs instead of change objects.** `differ.ml` and `Patch.create` restate base.js, word.js, line.js and create.js over runs (a kind and a count). The inner loop's diagonal bound is computed once per edit length. The diagonals that this adds after a path reaches the right edge can neither finish nor reach an edge, so the result is npm's.
- **Options.** Only the contract's options exist (CONTRACT.md, "Options"). `fit` allows no errors (fuzz 0). diffArrays compares the elements' JSON text, which for JSON scalars is `===`; under `ignoreCase` the text is lowercased, which for strings is `toLowerCase()` (the JSON text of a lowercased string is the lowercased JSON text).
- **reversePatch** copies the patch's own JS values, as `Object.assign` does, so missing hunk fields stay `undefined`.
- **canonicalize** assigns keys to a plain object, so a `"__proto__"` key is dropped, as in npm. It throws for a truthy `toJSON` member, which npm calls and JSON cannot make callable.
- **context.** `Patch.create` clamps patch creation's `context`, a JavaScript number, to the line count. That is exact: npm uses `context` only through `min`, `slice` and `<=`, where every value from the line count up acts alike.
- **Layout.** `differ.ml` keeps the dense layout it was written in. `patch.ml` and `program.ml` were rewritten in the 2026-11-20 review.

## Module state

None.

## Performance accommodations

Slicing and splitting go through `Host.slice` and `Host.split_lines`. Melange's `String.sub` converts the whole source string to bytes on every call, which made parsing quadratic (a 40,000-line hunk took 5.7 s). So the Melange host uses the JS string's `slice` and `split`, and the js_of_ocaml host uses `String.sub` and `String.split_on_char`, which are linear there. A 40,000-line hunk now parses in about 10 ms in Melange and 17 ms in js_of_ocaml.
