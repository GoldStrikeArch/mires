# jmespath 0.16.0: Kotlin/JS port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-jmespath/native/emit.py` generated from a shared core. It is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to Kotlin 2.4.10 on the harness's Gradle production lane. It follows the npm source class by class and method by method: Lexer, Parser, TreeInterpreter and Runtime. It uses no `js("…")` and no handwritten JS.

| File | What it is |
| --- | --- |
| `src/Lexer.kt` | `TokenType` (with jmespath's `bindingPower` table), `Token`, and `Lexer`, `_consume*` by `_consume*` |
| `src/Parser.kt` | `Parser`: `expression`, `nud`, `led` and the `_parse*` helpers |
| `src/Interpreter.kt` | `TreeInterpreter.visit`, `strictDeepEqual`, `isFalse`, `objValues`, `computeSliceParams` |
| `src/Runtime.kt` | `JmesType`, `typeName` (`_getTypeName`), the function table, `_validateArgs`, the 26 functions |
| `src/Host.kt` | the JS values jmespath works on: typed facades and the host functions it calls |
| `src/Errors.kt` | the reference adapter's error categories |
| `src/Main.kt` | the boundary; `run` is the module's only export (`@JsExport`) |

## Boundary

The input is an `external interface` read in place. The document stays the JS value it is, because jmespath returns parts of it and compares them by identity. Each result is a plain object built with `kotlin.js.json`: `{ value }`, or `{ error }` with the category that `reference.mjs`'s `category()` would give. Only jmespath's own errors become categories: `JmesPathException`, among them `JSON.parse`'s SyntaxError, which the lexer's `parseJsonToken` turns into a syntax error. There is no catch-all. Anything else thrown inside a query propagates out of `run` and fails the check, as it does out of the reference adapter. That includes stack exhaustion and a fault in the port (a `ClassCastException`, say), so a port fault cannot pass for a syntax error. `compile` is a fresh `Parser().parse`. `search` is a fresh parse and a fresh `TreeInterpreter`, as in jmespath.

## Interop decisions

- **Values are JS values.** jmespath relies on JS semantics for its values, so the port keeps them:
  - `Object.keys` order, including integer keys first;
  - `===` in `contains`;
  - the coercing relational operators;
  - JSON;
  - the `__proto__` setter on assignment.

  Values are `Any?` and are read through typed facades. AST nodes are JS objects built in jmespath's property order with `json(...)`, seen through `external interface Node`. They have to be: `compile` returns them, `&expr` hands a node to functions and to the caller, and the interpreter tags it in place (`jmespathType`).
- **Host functions.**
  - `JSON.parse`/`JSON.stringify` (kotlin.js.JSON);
  - `parseInt`;
  - `Number()` and `String()`;
  - `Object.keys`;
  - `Array.isArray`;
  - `localeCompare` (`asDynamic()`);
  - the default `Array.prototype.sort` (`asDynamic().sort()`, because Kotlin's `sort()` would order numbers numerically, not as strings).

  `kotlin.math.max`/`min`/`abs`/`ceil`/`floor` compile to `Math.*`.
- **JS operators.** `a.asDynamic() > b` compiles to JS `>`, with its coercion, for the comparators, `max_by`/`min_by` and `sort_by`. Kotlin's `===` on `Any?` compiles to JS `===`.
- **`undefined` stays distinct from null.** It comes from `isUndefined` (`jsTypeOf`) and `isNull` (`== null` and typeof object). Kotlin's `undefined` writes undefined where jmespath does: for `()`, for `a.` (`_parseDotRHS`), for an empty `max_by`, for `type()` of undefined, and for `value[i]` past the end.
- **`bindingPower` is `Int?`.** Colon and Literal have no entry, as in jmespath, so `rbp < undefined` and `undefined < 10` are both false.
- **`sort_by`** decorates with the index and sorts with `sortWith`. Kotlin/JS's `sortWith` calls the engine's `Array.prototype.sort` when that sort is stable, which it is in every target engine. So the comparator, its type checks and its NaN behaviour are jmespath's own, and its first error propagates through the engine's sort.
- **Strings are JS strings.** `lastIndexOf(prefix, length)` passes the start explicitly: Kotlin's default start is `lastIndex`, which differs from JS for an empty prefix. Reverse goes code unit by code unit, because `StringBuilder.reverse` keeps surrogate pairs together. The literal trim uses an explicit ECMAScript whitespace set: `Char.isWhitespace` includes U+001C–U+001F and U+0085 and lacks U+FEFF.

## Deviations from the npm source

None changes a result in the parity domain.

- **jmespath's engine TypeErrors are explicit errors here.** jmespath reaches a TypeError, which its adapter files as "syntax", when it visits a missing node, reads `left.name` of a missing node, or sets `jmespathType` on one. The port raises a syntax `JmesPathException` at those points.
- **A lone `=` fails in the lexer.** jmespath pushes `undefined` as a token there and fails when the parser reaches it. The category is the same.
- **Error messages differ.** They are not compared.
- **`merge`'s `for (key in current)` is `Object.keys(current)` and `current[key]`.** These are the same keys for strings, arrays, objects and primitives (for-in adds no inherited enumerable keys on these values). Null and undefined are skipped.

## Performance accommodations

None.
