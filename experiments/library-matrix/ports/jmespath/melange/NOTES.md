# jmespath 0.16.0: Melange 7.0.1 port

A port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to OCaml 5.3, compiled by Melange 7.0.1. It is not a rewrite: the pre-merge review's findings were fixed in place.

**Shared source.** `src/parser.ml` and `src/eval.ml` are byte-identical in the Melange and js_of_ocaml ports (README "Per-language notes"). Only `src/host.ml` differs, as the per-compiler interop module. See `ports/jmespath/jsoo/NOTES.md` for the other side.

| File | What it is |
| --- | --- |
| `src/parser.ml` | the lexer over UTF-16 code units and the Pratt parser (`bindingPower`, `nud`, `led`) |
| `src/eval.ml` | `kind` (`_getTypeName`), `falsy`, `equal` (`strictDeepEqual`), `cmp` (JS `<`), the interpreter and the 26 functions |
| `src/host.ml` | the host values and functions, for this compiler |
| `src/program.ml` | the boundary: each query in order, `{ value }` or `{ error }` |

## Boundary

Native Pratt parsing, projection evaluation and typed built-ins operate on host JSON values. They keep UTF-16 string semantics and JavaScript object semantics. Each result is `{ value }`, or `{ error }` with the reference adapter's category. Any other exception inside a query becomes "syntax", as `category()` does. Stack exhaustion is the exception: `Host.stack_exhausted` recognises the engine's RangeError or InternalError, which Melange hands over as `Js.Exn.Error`, and `run` and `json_value` raise it again, as the reference adapter lets it propagate.

## Review fixes (2026-11-20)

- **J1.** Filter projections evaluate every condition first, then the projection.
- **J2.** `max`/`min` of numbers fold the host's `Math.max`/`Math.min` (`math_max`, `math_min` in `host.ml`). String `max`/`min` use `localeCompare` in jmespath's operand order.
- **J3.** `type()` of undefined (or of anything `_getTypeName` does not name) is undefined.
- **`strictDeepEqual`.** It compares the own keys of the first object against the second's values, so undefined equals missing, and expression references compare as objects.
- **Slices.** They use jmespath's `capSliceRange` and its loop direction (`step > 0`) in floats, so NaN steps and bounds behave as in JS. `Index` turns an undefined element into null.
- **`bindingPower` returns `int option`, with no entry for Colon and Literal.**
- **`sort_by`** sorts the indices with the engine's `Array.prototype.sort` (`sort_with` in `host.ml`) and jmespath's comparator. It records the first error, which jmespath would throw out of its sort, answers 0 after it, and the order is then discarded.

## Deviations from the npm source

- **jmespath's engine TypeErrors are syntax errors at the same points.** These are visiting a missing node, `left.name` of a missing node, and tagging one. A lone `=` fails in the lexer, where jmespath pushes `undefined` and fails later; the category is the same.
- **Error messages are not produced.** Only categories.
