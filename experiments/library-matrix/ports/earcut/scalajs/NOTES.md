# earcut 3.2.4: Scala.js port

A port of `earcut.js` 3.2.4 to Scala 3.8, compiled by Scala.js 1.22. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

| File | What it is |
| --- | --- |
| `src/Earcut.scala` | `Vertex` and the `Earcut` class: triangulation, hole elimination with the block index, Morton order, `deviation` and `refine` |
| `src/Main.scala` | the boundary: `@js.native` facade traits over the input, `flatten`, and the `op` dispatch in the top-level export `run` |

## Boundary

The input is read in place through `@js.native` facade traits: `data`, `coords`, `triangles` and `holeIndices` are the input's own `js.Array`s and are never written. `refine` copies `triangles` with `slice` first. The triangles are built in a `js.Array[Int]`, which is returned as it is. For `flatten` the result is a `js.Dynamic.literal` with `vertices`, `holes` and `dimensions` in that order. `flatten` reads each component from a `js.Array[js.UndefOr[Double]]`, so a point shorter than the first contributes JS `undefined` for each missing component, as npm's `p[d]` does. A null or absent `holeIndices` is no holes.

## Module state

None. Every call builds a fresh `Earcut`. npm's module-level `filteredOut`, `indexActive` and block index (`blockBBox`, `numBlocks`, `blockHead`, `blockStop`) are its fields. The Morton sort's and `refine`'s scratch arrays are locals of the functions that use them. `refine` marks used hash slots with 1 in a fresh `hStamp`, where npm reuses one array and compares it with an incremented `gen`.

## Deviations from the npm source

None changes a result in the parity domain.

- **A sentinel vertex instead of `null`.** `Earcut.none`, a vertex with index -1, stands for npm's `null`. That covers an empty ring from `linkedList`, a missing bridge, and the ends of the z-order list (`prevZ`, `nextZ`). Inside the domain no triangle reaches it. For an empty hole slice, which CONTRACT.md excludes, it leaks into the triangles as -1.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. `Vertex.steiner` is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **Hole order.** The queue is a `js.Array` sorted with the engine's own stable `Array.prototype.sort`, as npm sorts it. The comparator maps a zero or NaN `compareXYSlope` difference to 0. `compareXYSlope` tests `a.x != b.x` and `a.y != b.y` before subtracting. For finite coordinates, a difference is zero exactly when the two values are equal.
- **ToInt32 in `zOrder`.** Scala's `Double.toInt` saturates outside the `Int` range. `toInt32` therefore truncates only in range, and reduces other values modulo 2^32 by hand, which is ECMAScript ToInt32 (NaN and ±Infinity become 0). `refine`'s hash multiplies `Int`s, which wrap at 32 bits as npm's `Math.imul` does.
- **Errors.** An unknown `op` throws a JS `TypeError`, as npm does. An empty `flatten` input reads a property of `undefined` (`js.Array` indexing is unchecked) and throws a TypeError, as npm does.

## Performance accommodations

None.
