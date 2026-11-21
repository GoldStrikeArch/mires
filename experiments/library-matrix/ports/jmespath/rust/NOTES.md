# jmespath 0.16.0: Rust port (mires)

A port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to `no_std` Rust compiled by mires. It is not a rewrite. The pre-merge review's findings were fixed in place.

| File | What it is |
| --- | --- |
| `src/parser.rs` | the lexer over UTF-16 code units and the Pratt parser (`bindingPower`, `nud`, `led`) |
| `src/eval.rs` | `kind` (`_getTypeName`), `falsy`, `equal` (`strictDeepEqual`), `compare` (JS `<`), the interpreter |
| `src/functions.rs` | signatures, argument validation, the 26 functions, `sort_by` |
| `src/host.rs` | `js_bindings!` for the host functions jmespath relies on |
| `src/lib.rs` | the boundary (`matrix_run`) and the panic handler |

## Boundary

Rust drives host `JsValue`s throughout, and the AST and values are JS objects. JavaScript supplies the JSON primitives and the UTF-16 string operations. Each result is `{ value }`, or `{ error }` with the reference adapter's category.

Only jmespath's own errors become categories: the `Err` categories of the parser, interpreter and functions, and `JSON.parse`'s SyntaxError in the lexer (`json`, `is_syntax_error`). There is no catch-all. Anything else thrown inside a query propagates out of `matrix_run` and fails the check, as it does out of the reference adapter. That includes stack exhaustion, the panic handler's throw, and a host value that does not match its Rust type. A port fault therefore cannot pass for a syntax error.

## Review fixes (2026-11-20)

- **J1.** Filter projections evaluate every condition first, then the projection (`filter_projection`).
- **J2.** `max`/`min` of numbers fold the host's `Math.max`/`Math.min`, which have JS semantics for NaN and signed zeros (`math_max`, `math_min`). `f64::max` ignores NaN.
- **J3.** `type()` of undefined is undefined.
- **`strictDeepEqual`.** It compares the own keys of the first object against the second's values, so undefined equals missing, and expression references compare as objects.
- **Slices and indices compute in `f64`, as jmespath does in JS numbers.** This covers NaN tokens such as `[0:3:-]` and bounds past int32 and 2^53. `Index` turns an undefined element into null.
- **`bindingPower` is `Option<i32>`, with no entry for Colon and Literal.**
- **`sort_by`** sorts the indices with the engine's `Array.prototype.sort` and jmespath's comparator, which is a `mires_js::callback2` closure. It records the first error, which jmespath would throw out of its sort, answers 0 after it, and the order is then discarded.
- **Depth.** The interpreter's large arms (index, slice, projections, flatten, comparator, multiselect, function call) are separate functions. That keeps `eval`'s frame small enough for the contract's nesting bound of 500 levels: `x[][]…` failed at 327 levels before.

## Deviations from the npm source

- **jmespath's engine TypeErrors are syntax errors at the same points.** These are visiting a missing node, `left.name` of a missing node, and tagging one. A lone `=` fails in the lexer, where jmespath pushes `undefined` and fails later; the category is the same.
- **Error messages are not produced.** Only categories.
