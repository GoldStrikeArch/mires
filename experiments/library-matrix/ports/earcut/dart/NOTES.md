# earcut 3.2.4: Dart port (dart2js)

A port of `earcut.js` 3.2.4 to Dart 3, compiled by dart2js. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

| File | What it is |
| --- | --- |
| `src/earcut.dart` | `Vertex` and the `Earcut` class: triangulation, hole elimination with the block index, Morton order, `deviation` and `refine` |
| `src/main.dart` | the boundary: `dart:js_interop` extension types over the input, and the `op` dispatch |

## Boundary

`run` reads the input through `dart:js_interop` extension types and never writes it. Coordinates are copied into a `List<double>` and indices into a `List<int>`. Results are JS arrays of numbers. For `flatten` the result is a plain object, built in `vertices`, `holes`, `dimensions` order. `flatten` reads each component with `getProperty`, so a point shorter than the first contributes JS `undefined` for each missing component, as npm's `p[d]` does. `main()` publishes `run` on `globalThis.__matrixExports`, which the builder's facade reads.

## Module state

None. Every call builds a fresh `Earcut`. npm's module-level `filteredOut`, `indexActive` and block index (`blockBBox`, `numBlocks`, `blockHead`, `blockStop`) are its fields. The Morton sort's and `refine`'s scratch arrays are locals of the functions that use them. `refine` marks used hash slots with 1 in a fresh `hStamp`, where npm reuses one array and compares it with an incremented `gen`. `main()` writes `globalThis.__matrixExports` once, at load.

## Deviations from the npm source

None changes a result in the parity domain.

- **A sentinel vertex instead of `null`.** `Earcut.none`, a vertex with index -1, stands for npm's `null`. That covers an empty ring from `linkedList`, a missing bridge, and the ends of the z-order list (`prevZ`, `nextZ`). Inside the domain no triangle reaches it. For an empty hole slice, which CONTRACT.md excludes, it leaks into the triangles as -1.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. `Vertex.steiner` is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **Hole order.** Dart's `List.sort` is not stable. `sortHoles` therefore breaks `compareXYSlope` ties (a zero or NaN difference) by the holes' original positions. That is what the engine's stable `Array.prototype.sort` does for npm. `compareXYSlope` tests `a.x != b.x` and `a.y != b.y` before subtracting. For finite coordinates, a difference is zero exactly when the two values are equal.
- **JS integer semantics on dart2js.** Dart's `int` is a JS number on the web, and its bitwise operators give unsigned 32-bit results.
  - `zOrder` converts with `toInt32`, which is ECMAScript ToInt32. NaN and ±Infinity become 0 and values in range truncate. Other values reduce modulo 2^32 before `toSigned(32)`. The interleaved key is taken `toSigned(32)`, as JS's `|` gives it.
  - `refine`'s hash multiplies with `mul32`, a 16-bit split whose partial products stay below 2^53, where npm uses `Math.imul`.
- **Errors.** An unknown `op` throws an `ArgumentError`, and an empty `flatten` input throws a `StateError`, where npm throws TypeErrors. The harness compares only that both threw.

## Performance accommodations

None.
