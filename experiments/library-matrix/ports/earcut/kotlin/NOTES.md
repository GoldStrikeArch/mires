# earcut 3.2.4: Kotlin/JS port

A port of `earcut.js` 3.2.4 to Kotlin, compiled by Kotlin/JS. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

| File | What it is |
| --- | --- |
| `src/Earcut.kt` | `Vertex` and the `Earcut` class: triangulation, hole elimination with the block index, Morton order, `deviation` and `refine` |
| `src/main.kt` | the boundary: external interfaces over the input, `flatten`, and the `op` dispatch in the `@JsExport` function `run` |

## Boundary

The input is read in place through `external interface`s: `data`, `coords`, `triangles` and `holeIndices` are the input's own JS arrays, typed `Array<Double>` and `Array<Int>`, and are never written. `refine` and `deviation` copy `triangles` with `toMutableList()` first. Results are JS arrays (`toTypedArray()`). For `flatten` the result is a plain `Object` with `vertices`, `holes` and `dimensions` assigned in that order. `flatten` adds `if (d < point.size) point[d] else undefined` for each component, so a point shorter than the first contributes JS `undefined` for each missing component, as npm's `p[d]` does. The bound is explicit, so the code does not rely on Kotlin/JS leaving `Array.get` unchecked.

## Module state

None. Every call builds a fresh `Earcut`. npm's module-level `filteredOut`, `indexActive` and block index (`blockBBox`, `numBlocks`, `blockHead`, `blockStop`) are its fields. The Morton sort's and `refine`'s scratch arrays are locals of the functions that use them. `refine` marks used hash slots with 1 in a fresh `hStamp`, where npm reuses one array and compares it with an incremented `gen`.

## Deviations from the npm source

None changes a result in the parity domain.

- **A sentinel vertex instead of `null`.** `Earcut.none`, a vertex with index -1, stands for npm's `null`. That covers an empty ring from `linkedList`, a missing bridge, and the ends of the z-order list (`prevZ`, `nextZ`). Inside the domain no triangle reaches it. For an empty hole slice, which CONTRACT.md excludes, it leaks into the triangles as -1.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. `Vertex.steiner` is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **Hole order.** `MutableList.sortWith` is stable, as the engine's `Array.prototype.sort` is for npm, and the comparator maps a zero or NaN `compareXYSlope` difference to 0. `compareXYSlope` tests `a.x != b.x` and `a.y != b.y` before subtracting. For finite coordinates, a difference is zero exactly when the two values are equal.
- **ToInt32 in `zOrder`.** Kotlin's `Double.toInt()` saturates outside the `Int` range. `toInt32` therefore truncates only in range, and reduces other values modulo 2^32 by hand, which is ECMAScript ToInt32 (NaN and ±Infinity become 0). `refine`'s hash multiplies `Int`s, which wrap at 32 bits as npm's `Math.imul` does.
- **Errors.** An unknown `op` throws an `IllegalArgumentException`, and an empty `flatten` input throws a `NoSuchElementException` from `first()`, where npm throws TypeErrors. The harness compares only that both threw.

## Performance accommodations

None.
