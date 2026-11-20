# jmespath 0.16.0: ReScript port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-jmes-ml/resgen.py` produced by rewriting an OCaml-syntax source into ReScript. It is a faithful port of `jmespath.js` 0.16.0 (the subset in `libraries/jmespath/CONTRACT.md`) to ReScript 12.3. It follows the npm source: Lexer, Parser, TreeInterpreter and Runtime.

| File | What it is |
| --- | --- |
| `src/Lexer.res` | the token type, jmespath's `bindingPower` table, `tokenize` and its `consume*` helpers |
| `src/Parser.res` | `expression`, `nud`, `led` and the `parse*` helpers |
| `src/Interpreter.res` | `visit` and `computeSliceParams` |
| `src/Runtime.res` | the jmespath types, `typeName` (`_getTypeName`), the function table, `validateArgs`, the 26 functions |
| `src/Values.res` | `strictDeepEqual`, `isFalse`, `objValues` |
| `src/Host.res` | the JS values jmespath works on: the abstract `value`, the node record view, the host bindings |
| `src/Errors.res` | the reference adapter's error categories |
| `src/Index.res`, `src/Index.resi` | the boundary; `run` is the module's only export |

## Boundary

The input record is the corpus JS object itself, so reading it converts nothing. The document stays the JS value it is, because jmespath returns parts of it and compares them by identity. Each result is a `dict`: `{ value }`, or `{ error }` with the category that `reference.mjs`'s `category()` would give.

## Interop decisions

- **Values are the abstract type `Host.value`.** jmespath relies on JS semantics for its values, so the port keeps them:
  - `Object.keys` order;
  - `===` in `contains`;
  - the coercing relational operators;
  - JSON;
  - the `__proto__` setter on assignment.

  Values are converted with `%identity` externals. AST nodes are JS object literals (`{"type": …, "name": …}`) in jmespath's property order, read through the record type `Host.node`. `compile` returns them, `&expr` passes them around as values, and the interpreter tags them in place (a mutable field). A parse result is `option<node>`: `None` is jmespath's undefined node, and arrays of them hold `undefined`.
- **Host functions, as `@val`/`@send` externals or the Stdlib bindings of the same functions.**
  - `JSON.parse`/`JSON.stringify`;
  - `parseInt`;
  - `Number()` and `String()`;
  - `Object.keys`;
  - `Array.isArray`;
  - `String.localeCompare`;
  - `Array.prototype.sort`, with and without a comparator;
  - `Math.*`.
- **JS relational operators.** ReScript compiles float comparisons to JS's `<` and `>`. A value cast to `float` (`%identity`) therefore compares exactly as jmespath's operands do, with JS's own coercion. `===` on the abstract type is JS `===`.
- **`bindingPower` is `option<int>`.** Colon and Literal have no entry, as in jmespath.
- **`sort_by` sorts the indices with the engine's `Array.prototype.sort`** (`Array.sort`). The comparator is jmespath's, including its type checks and its NaN behaviour, and its first error propagates through the engine's sort.
- **The function table is a `Map`.** Function bodies receive `visit` as an argument, because ReScript modules cannot be mutually recursive.

## Deviations from the npm source

None changes a result in the parity domain.

- **jmespath's engine TypeErrors are explicit errors here.** jmespath reaches a TypeError, which its adapter files as "syntax", when it visits a missing node, reads `left.name` of a missing node, or sets `jmespathType` on one. The port raises a syntax error at those points. `run` still files every other exception as "syntax", as `category()` does.
- **A lone `=` fails in the lexer.** jmespath pushes `undefined` as a token there and fails when the parser reaches it. The category is the same.
- **Error messages differ.** They are not compared.
- **`merge`'s `for (key in current)` is written per type.** A string gives its indices and code units, an object or array gives `Object.keys` and values, and anything else gives nothing.

## Performance accommodations

**The interpreter's recursive paths and `strictDeepEqual` loop with `for` and `while`, not `Array.forEach`, `Array.map` or `Array.every`.** A callback adds frames to every nesting level. In Firefox that cost the contract's nesting bound: `x[*][*]…` and `x == y` failed at 500 levels. With the loops, every construct passes at 800 in Firefox.
