# jmespath 0.16.0: Scala.js port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-jmespath/native/emit.py` generated from a shared core. It is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to Scala 3.8.4 and Scala.js 1.22.0 (`fullLinkJS`, ES module). It follows the npm source class by class and method by method: Lexer, Parser, TreeInterpreter and Runtime.

| File | What it is |
| --- | --- |
| `src/Lexer.scala` | `TokenType` (with jmespath's `bindingPower` table), `Token`, `Lexer` |
| `src/Parser.scala` | `Parser`: `expression`, `nud`, `led` and the `_parse*` helpers |
| `src/Interpreter.scala` | `TreeInterpreter.visit`, `strictDeepEqual`, `isFalse`, `objValues`, `computeSliceParams` |
| `src/Runtime.scala` | `JmesType`, `typeName` (`_getTypeName`), the function table, `_validateArgs`, the 26 functions |
| `src/Host.scala` | the JS values jmespath works on: the `Node` facade and the host functions it calls |
| `src/Errors.scala` | the reference adapter's error categories |
| `src/Main.scala` | the boundary; `run` is exported with `@JSExportTopLevel` |

## Boundary

The input is the JS object itself, seen through `@js.native` facade traits, with no copy. The document stays the JS value it is (`js.Any`), because jmespath returns parts of it and compares them by identity. Each expression is a JS string. Each result is a `js.Dictionary`: `{ value }`, or `{ error }` with the category that `reference.mjs`'s `category()` would give.

Only jmespath's own errors become categories: `JmesPathException`, among them `JSON.parse`'s SyntaxError, which the lexer's `parseJsonToken` matches as `js.JavaScriptException(_: js.SyntaxError)` and turns into a syntax error. There is no catch-all. Anything else thrown inside a query propagates out of `run` and fails the check, as it does out of the reference adapter. That includes stack exhaustion, which Scala.js re-throws as the original JS error, and a fault in the port, so a port fault cannot pass for a syntax error.

## Interop decisions

- **Values are `js.Any`.** jmespath relies on JS semantics for its values, so the port keeps them:
  - `Object.keys` order;
  - `===` in `contains`;
  - the coercing relational operators;
  - JSON;
  - the `__proto__` setter on assignment.

  AST nodes are JS objects built in jmespath's property order (a `js.Dictionary` filled in order) and read through the `Node` facade. `compile` returns them, `&expr` passes them around as values, and the interpreter tags them in place.
- **Host functions, through Scala.js's own facades and one `@JSGlobalScope` object.**
  - `js.JSON`;
  - `parseInt`;
  - `Number()` and `String()`;
  - `js.Object.keys`;
  - `js.Array.isArray`;
  - `localeCompare` (`JSStringOps`);
  - `js.Array.sort`.

  `math.max`/`min`/`abs`/`ceil`/`floor` agree with `Math.*` on NaN and on signed zeros.
- **JS operators.** `js.Dynamic`'s `<`, `>`, `<=`, `>=` are JS's coercing operators. `js.special.strictEquals` is `===`; Scala's `eq` is `Object.is` in Scala.js, which differs at NaN and -0.
- **`bindingPower` is `Option[Int]`.** Colon and Literal have no entry, as in jmespath.
- **`sort_by` sorts the indices with the engine's `Array.prototype.sort`** (`js.Array.sort`). The comparator is jmespath's, including its type checks and its NaN behaviour, and its first error propagates through the engine's sort.
- **Strings.**
  - The first-only replacements (`` \` `` and `\'`) are written with `indexOf`, because Java's `replaceFirst` takes a regex, in which `\'` is a bare quote.
  - Reverse goes code unit by code unit, because `StringOps.reverse` keeps surrogate pairs.
  - The literal trim uses an explicit ECMAScript whitespace set: `Character.isWhitespace` includes U+001C–U+001F and lacks U+00A0, U+202F and U+FEFF.

## Deviations from the npm source

None changes a result in the parity domain.

- **jmespath's engine TypeErrors are explicit errors here.** jmespath reaches a TypeError, which its adapter files as "syntax", when it visits a missing node, reads `left.name` of a missing node, or sets `jmespathType` on one. The port raises a syntax `JmesPathException` at those points.
- **A lone `=` fails in the lexer.** jmespath pushes `undefined` as a token there and fails when the parser reaches it. The category is the same.
- **Error messages differ.** They are not compared.
- **`merge`'s `for (key in current)` is written per type.** A string gives its indices and code units, an object or array gives `Object.keys` and values, and anything else gives nothing.

## Performance accommodations

None. The Scala collections the port uses (`ArrayBuffer`, `Map`, `mutable.Set`, `Option`) make up most of the bundle's size.
