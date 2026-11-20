# jmespath 0.16.0: Elm port

This port is written by hand. It replaces the earlier Elm port, whose identity machinery `build/library-matrix/dev-jmespath-elmgo/identity.py` had patched in with string replacements: the `Box` wrapper, `raw`/`tag`, the `RefEq` step, the `identityResult` port and the host's identity loop. This one is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to Elm 0.19.2 with a JS host. It follows the npm source: Lexer, Parser, TreeInterpreter and Runtime.

| File | What it is |
| --- | --- |
| `src/Values.elm` | `V`, the JS values jmespath sees, with identity tokens; JS semantics on them; the boundary encoding |
| `src/Parser.elm` | the Lexer and the Pratt parser (`bindingPower`, `nud`, `led`, the `parse*` helpers) |
| `src/Eval.elm` | `Step`, `traverse`, the interpreter (`visit`) and the Runtime (types, signatures, the 26 functions) |
| `src/Main.elm` | the queries in order, and the port protocol with the host |
| `js/index.js` | the host: the boundary encoding, `localeCompare` and `===`, no jmespath logic |

## Boundary

The host sends `{ data, queries }`. The document is a flat postfix list of instructions: a primitive, or `{ array: n, token }` / `{ object: keys, token }` to build from the values before it. Both sides write and read such lists with loops, so no nesting depth recurses at the boundary. Each array and object carries itself as its token. The result array comes back the same way, with `{ undefined: true }` for undefined, and the host rebuilds it with `Object.fromEntries`, so a `__proto__` key stays an own property.

## Values

- **`V` is `Undefined | Null | Flag | Num | Str | Arr Token (List V) | Obj Token (List (String, V))`.** Objects keep JS key order. Decoded objects come in `for…in` order. Built objects follow assignment semantics (`put`: an existing key keeps its place, and a `__proto__` key is dropped, as the prototype setter ignores a primitive), and `sortFields` then puts array-index keys first.
- **Identity.** jmespath's `contains` compares arrays and objects with `===`, and Elm values have no identity. Every array and object therefore carries a `Token`: the JS object it was decoded from (the input, or what `JSON.parse` made of a literal), or a fresh empty JS array when the parser or the evaluator builds it. `&expr` returns the referenced node with its own token, so it keeps one identity however often it is evaluated. Only the host can compare tokens, which is why `contains` with an array or object asks the host once per call.
- **JS semantics in Elm.**
  - `jsString` is `String()`: Elm's `String.fromFloat` is `number + ''`.
  - `jsNumber` is `Number()`. It uses JS `trim` (Elm's `String.trim`), the `0x`/`0o`/`0b` prefixes in either case, rounded once to the nearest double past 2^53, and else Elm's `String.toFloat`, which is JS `+` after refusing whitespace and x/b/o.
  - `relation` is the abstract relational comparison: arrays and objects through `String()`, two strings by code unit (Elm's string `compare` is JS `<`), anything else as numbers, NaN as no order.
  - `strictDeepEqual` and `isFalse` are jmespath's.
  - `jsonText` is `JSON.stringify` (keys and strings through `Json.Encode`).

## Evaluation

- **`Step`** is `Done`, `Failed`, `Collate` (ask the host for `a.localeCompare(b)`, for max and min of strings) or `Identical` (ask whether a token is `===` to any of some others, for contains). The host answers through ports, and the continuation resumes.
- **`traverse` loops (a tail call) while each step is done.** A projection, filter, `map` or `sort_by` over 100,000 records costs no stack (J6). The parser's operator loop and the lexer are tail calls too.
- **Faithful details.**
  - Filter projections run every condition, then the projection.
  - `max`/`min` of numbers fold JS `Math.max`/`Math.min`; NaN wins, and of +0 and -0 `max` prefers +0 and `min` -0.
  - `abs` is `Math.abs` (-0 is +0).
  - `type()` of undefined is undefined.
  - `bindingPower` has no Colon or Literal entry.
  - Slices compute in JS numbers, NaN included, and read `value[i]` past the end as undefined.
- **`sort_by`** sorts with Elm's `List.sortWith`, which is the engine's `Array.prototype.sort`. It uses jmespath's comparator, the index breaking ties, so with NaN keys the engine decides the order, as it does for jmespath.

## Deviations from the npm source

- **`sort_by` checks keys in element order.** jmespath checks keys inside its comparator, in the order the engine compares them, and Elm cannot raise from inside the comparator. The reported error differs only when one key raises invalid-value (a zero-step slice of an array) and another has the wrong type. CONTRACT.md excludes that case.
- **`&expr` values are snapshots.** jmespath tags the referenced node in place. An expression reference taken as a value and evaluated again later (through `@`) tags its descendants, which jmespath then shows in output serialised afterwards. Here the earlier value is unchanged. CONTRACT.md excludes this case.
- **jmespath's engine TypeErrors are `Failed "syntax"` at the same points.** These are visiting a missing node, `left.name` of a missing node, and tagging one. A lone `=` fails in the lexer, where jmespath pushes `undefined` and fails later; the category is the same. Error messages are not produced.
- **Elm quirks the code works around.**
  - Elm's `>` and `>=` call NaN greater than everything, so slice comparisons use only `<` and `<=`, swapping operands where JS has `>`.
  - `x == 0` compiles to `!x`, which NaN passes, so the zero-step check also tests `isNaN`.

## Performance accommodations

- **The boundary is flat postfix lists both ways.** This is for depth (README "Per-language notes"): nested `Json.Decode` and `Json.Encode` recursion would cost the contract's nesting bound.
- **Slices index an `Array` made once per slice.** A `List` index is linear.
