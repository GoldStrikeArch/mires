# jmespath 0.16.0: Dart port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-jmespath/native/emit.py` generated from a shared core. That port converted the document to Dart `Map`/`List` values, used an `Object()` sentinel for undefined, and rebuilt the output with `defineProperty`. This one is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to Dart 3.14 through dart2js (`-O2`). It follows the npm source class by class and method by method: Lexer, Parser, TreeInterpreter and Runtime.

| File | What it is |
| --- | --- |
| `src/lexer.dart` | `TokenType` (TOK_* names and jmespath's `bindingPower` table), `Token`, `Lexer` |
| `src/parser.dart` | `Parser`: `expression`, `nud`, `led` and the `_parse*` helpers |
| `src/interpreter.dart` | `TreeInterpreter.visit`, `strictDeepEqual`, `isFalse`, `objValues`, `computeSliceParams` |
| `src/runtime.dart` | `JmesType`, `typeName` (`_getTypeName`), the function table, `_validateArgs`, the 26 functions |
| `src/host.dart` | the JS values jmespath works on: the `Node` extension type and the host functions it calls |
| `src/errors.dart` | the reference adapter's error categories |
| `src/main.dart` | the boundary; `main()` publishes `run` for the builder's facade |

## Boundary

The input is the JS object itself, seen through `dart:js_interop` extension types, with no copy. The document stays the JS value it is (`JSAny?`), because jmespath returns parts of it and compares them by identity. Each expression is a Dart string, which on the web is the same JS string. Each result is a plain `JSObject`: `{ value }`, or `{ error }` with the category that `reference.mjs`'s `category()` would give.

Only jmespath's own errors become categories: `JmesPathException`, among them `JSON.parse`'s SyntaxError, which the lexer's `_parseJson` turns into a syntax error. There is no catch-all. Anything else thrown inside a query propagates out of `run` and fails the check, as it does out of the reference adapter. That includes stack exhaustion, which dart2js sees as a `StackOverflowError`, and a fault in the port, so a port fault cannot pass for a syntax error.

## Interop decisions

- **Values are `JSAny?`.** jmespath relies on JS semantics for its values, so the port keeps them:
  - `Object.keys` order;
  - `===` in `contains`;
  - the coercing relational operators;
  - JSON;
  - the `__proto__` setter on assignment.

  AST nodes are `JSObject`s built in jmespath's property order and read through `extension type Node`, because `compile` returns them, `&expr` passes them around as values, and the interpreter tags them in place.
- **`undefined` versus null.** dart2js reads both as Dart `null`, so the port tells them apart with `isUndefined`/`isNull` from `dart:js_interop`. It writes undefined through the global `undefined` (`@JS('undefined')`). The interop getter `children` is non-nullable and rejects `undefined`, so the interpreter reads it only for node types that have children.
- **Host functions, all through `@JS` externals.**
  - `JSON.parse`/`JSON.stringify`;
  - `parseInt`;
  - `Number()` and `String()`;
  - `Object.keys`;
  - `Array.isArray`;
  - `localeCompare`;
  - `Array.prototype.sort`, with and without a comparator.

  The relational operators and `===` are `JSAnyOperatorExtension` (`greaterThan`, `strictEquals`, …), which are the JS operators. dart2js's `math.max`/`min` are `Math.max`/`Math.min`, and `abs`/`ceilToDouble`/`floorToDouble` are `Math.*`.
- **Arrays are `JSArray`.** `operator []` past the end is JS's undefined. Results are Dart lists made into JS arrays with `.toJS`.
- **`bindingPower` is `int?`.** Colon and Literal have no entry, as in jmespath.
- **`sort_by` sorts the indices with the engine's `Array.prototype.sort`.** jmespath sorts `[index, element]` pairs. The comparator is jmespath's, including its type checks and its NaN behaviour, and its first error propagates through the engine's sort. Dart's `List.sort` is its own algorithm and would decide inconsistent comparisons differently.
- **Strings.** Dart's `String.indexOf` refuses a negative start, so `ends_with` clamps it to 0 as JS does. Reverse goes code unit by code unit. The literal trim uses an explicit ECMAScript whitespace set, because Dart's `trimLeft` also strips U+0085.

## Deviations from the npm source

None changes a result in the parity domain.

- **jmespath's engine TypeErrors are explicit errors here.** jmespath reaches a TypeError, which its adapter files as "syntax", when it visits a missing node, reads `left.name` of a missing node, or sets `jmespathType` on one. The port raises a syntax `JmesPathException` at those points.
- **A lone `=` fails in the lexer.** jmespath pushes `undefined` as a token there and fails when the parser reaches it. The category is the same.
- **Error messages differ.** They are not compared.
- **`merge`'s `for (key in current)` is written per type.** A string gives its indices and code units, an object or array gives `Object.keys` and values, and anything else gives nothing. dart2js would reject reading a property of a JS primitive through `JSObject`.

## Performance accommodations

None.
