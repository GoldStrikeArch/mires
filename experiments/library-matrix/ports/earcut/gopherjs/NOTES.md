# earcut 3.2.4: GopherJS port

A port of `earcut.js` 3.2.4 to Go, compiled by GopherJS. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

| File | What it is |
| --- | --- |
| `src/earcut.go` | `node` and `clipper`: rings, `filterPoints`, the clipping loop, the predicates, Morton order |
| `src/holes.go` | hole elimination: the block index, `findHoleBridge`, the hole sort |
| `src/refine.go` | `refine` and `deviation` |
| `src/main.go` | the boundary and the `op` dispatch; `run` is published on `js.Module.exports` |

## Boundary

`run` reads the input as a `*js.Object` and never writes it. Coordinates are copied into a `[]float64` and indices into a `[]int`, and results are new JS arrays. For `flatten` the result is a plain `Object` with `vertices`, `holes` and `dimensions` set in that order. `flatten` pushes `point.Index(d)`, the JS element itself, so a point shorter than the first contributes JS `undefined` for each missing component, as npm's `p[d]` does. An absent `dim` is 2, an absent or null `holeIndices` is no holes, and an absent `refine` is false.

## Module state

None. Each triangulation owns a `clipper`, which holds npm's module-level `filteredOut` (`filtered`), `indexActive` and block index, and the triangles. The Morton sort and `refine` allocate their buffers per call.

## Deviations from the npm source

None changes a result in the parity domain.

- **`nil` for npm's `null`.** `linkedList` returns nil for an empty ring, `findBridge` for no bridge, and `prevZ`/`nextZ` end the z-order list with nil.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. `node.steiner` is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **The block index is a slice of `*block`.** Each block holds its head, its stop and its bounding box. npm keeps a flat `Float64Array` and two node arrays of a precomputed maximum size. Blocks are appended in the same order and scanned in append order.
- **`isEar` and `isEarHashed` are one function.** It branches on `invSize == 0` as `earcutLinked` does, and runs npm's two loops unchanged.
- **Hole order.** `sortHoles` is a stable merge sort with `holeLess`, which is `compareXYSlope(a, b) < 0` written as comparisons. For finite coordinates and the slopes they give (±Infinity included), `x - y < 0` is exactly `x < y`, and a NaN slope compares as neither, as in npm.
- **ToInt32 in `zOrder`.** The Go specification leaves an out-of-range float-to-integer conversion implementation-defined. GopherJS lowers `int32(v)` to `v >> 0`, which is ECMAScript ToInt32 (NaN and ±Infinity become 0, other values wrap modulo 2^32). The port relies on that lowering. `refine`'s hash multiplies `uint32`s, which wrap as Go defines and as npm's `Math.imul` does.
- **`refine`'s hash table** encodes a slot as 0 (empty), -1 (consumed) or half-edge + 1, so a fresh `make([]int, size)` is all empty. npm uses `hStamp`/`gen` for empty slots and -1 for consumed ones. `pending` (a `[]bool`) stands for `edgeStamp`.
- **`min` and `max`.** The outer bounding box folds Go's `min` and `max`, where npm compares with `<` and `>`. Go's builtins follow JS's `Math.min`/`Math.max` for signed zeros and NaN. With finite coordinates the two forms differ only in which zero they keep when -0 and +0 meet. No result depends on that: the box feeds only comparisons, the span, and `(x - minX) * invSize`, whose ToInt32 is 0 for either zero.
- **Errors.** An unknown `op` or an empty `flatten` input panics with a Go string, where npm throws a TypeError. An empty hole slice, which CONTRACT.md excludes, panics on a nil ring. The harness compares only that both threw.

## Performance accommodations

- **`earTriangle.blocked` has a pointer receiver.** With a value receiver, GopherJS copies all seven fields of the candidate triangle for every vertex the ear test scans.
