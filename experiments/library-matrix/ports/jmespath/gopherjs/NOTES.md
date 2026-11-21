# jmespath 0.16.0: GopherJS port

This port is written by hand. The earlier port was a Go transliteration of the shared core that `build/library-matrix/dev-jmespath/native/emit.py` rendered for Dart, Kotlin and Scala.js. It used the same shims (`tokensBuffer`, `addToken`, `codeAt`, `strLen`, `part`, `slash`). This one is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to Go, compiled by GopherJS 1.21.0 with Go 1.21.13. It follows the npm source: Lexer, Parser, TreeInterpreter and Runtime. It uses error values, not panics.

| File | What it is |
| --- | --- |
| `src/lexer.go` | token types, jmespath's `bindingPower` table (a map with no entry for Colon and Literal), the lexer over UTF-16 code units |
| `src/parser.go` | the Pratt parser: `expression`, `nud`, `led` and the `parse*` helpers |
| `src/interpreter.go` | `visit`, `strictDeepEqual`, `isFalse`, `objValues`, `computeSliceParams` |
| `src/functions.go` | the jmespath types, `typeName`, the signature table, `validateArgs`, the 26 functions |
| `src/host.go` | the JS values jmespath works on and the host functions it calls |
| `src/errors.go` | the reference adapter's error categories |
| `src/main.go` | the boundary; `run` is published on `js.Module.exports` for the builder's facade |

## Boundary

The input is a `*js.Object` read in place. The document stays the JS value it is, because jmespath returns parts of it and compares them by identity. Each expression stays a JS string. Each result is a plain object: `{ value }`, or `{ error }` with the category that `reference.mjs`'s `category()` would give. Stack exhaustion is not a category: it propagates out of `run`, as it does out of the reference adapter (`stackExhausted` in `host.go` recognises the engine's RangeError or InternalError in the recovered `*js.Error`, and `query` and `parseJSON` panic with it again).

## Interop decisions

- **Values are `*js.Object`.** jmespath relies on JS semantics for its values, so the port keeps them:
  - `Object.keys` order;
  - `===` in `contains`;
  - JSON;
  - the `__proto__` setter on assignment (`Reflect.set` runs the same `[[Set]]`).

  AST nodes are JS objects in jmespath's property order, because `compile` returns them, `&expr` passes them around as values, and the interpreter tags them in place. GopherJS reads JS null as a nil `*js.Object` and undefined as `js.Undefined`. Comparing two `*js.Object`s compares the JS values with `===`.
- **Text never becomes a Go string.** GopherJS's conversion of a JS string to a Go string mishandles lone surrogates (README "Per-language notes"). So the lexer reads the expression's code units with `charCodeAt`, cuts tokens with the host's `slice`, and keeps every name and string a JS string. Only node types, comparator and function names (ASCII) are read as Go strings.
- **Host functions.**
  - `JSON.parse` (its SyntaxError recovered into a syntax error) and `JSON.stringify`;
  - `parseInt`;
  - `Number()` and `String()`;
  - `Object.keys`;
  - `Array.isArray`;
  - `Reflect.get`/`Reflect.set` for `object[key]` with a JS key;
  - `localeCompare`, `indexOf`, `lastIndexOf`, `split`/`reverse`/`join`;
  - `Array.prototype.sort`.

  Go's `math.Max`/`Min`/`Abs`/`Ceil`/`Floor` agree with `Math.*` on NaN and on signed zeros.
- **JS relational operators.** Go has no JS `<`, so `order` is the abstract relational comparison for these values. Arrays and objects become their `String()`, two strings compare by code unit (`charCodeAt`), anything else compares as numbers (`Number()`), and NaN compares as nothing. `Object.prototype.toString` gives the type tags, as in jmespath's `_getTypeName`.
- **`sort_by` sorts the indices with the engine's `Array.prototype.sort`,** with jmespath's comparator and its NaN behaviour. jmespath's first error aborts its sort. Here the comparator keeps that error, answers 0 to the calls the engine makes after it, and `sortBy` returns the error and discards the order.

## Deviations from the npm source

None changes a result in the parity domain.

- **jmespath's engine TypeErrors are explicit errors here.** jmespath reaches a TypeError, which its adapter files as "syntax", when it visits a missing node, reads `left.name` of a missing node, or sets `jmespathType` on one. The port returns a syntax error at those points. A panic out of a host call is recovered in `query` as "syntax", as `category()` does.
- **A lone `=` fails in the lexer.** jmespath pushes `undefined` as a token there and fails when the parser reaches it. The category is the same.
- **Error messages differ.** They are not compared.
- **`reverse` of a string is `split("").reverse().join("")`.** jmespath concatenates code units in a loop; the result is the same string.
- **`merge`'s `for (key in current)` is written per type.** A string gives its indices and `charAt`, an object or array gives `Object.keys` and values, and anything else gives nothing.

## Performance accommodations

**The function table holds only signatures; `call` dispatches the bodies with a `switch`.** GopherJS compiles every call through a function value as a resumable state machine (`$restore`), and with it every function that makes one, `visit` and the parser included. A table of closures, a `then(child, err, build)` parser helper and the slice-bound closure each turned `visit` and the parser into state machines. Their heavier frames cut the nesting depth: `x[][]…` failed below the contract's bound of 500 levels. The emit now has no state machines.
