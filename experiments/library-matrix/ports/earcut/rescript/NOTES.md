# earcut 3.2.4: ReScript port

A port of `earcut.js` 3.2.4 to ReScript 12.3. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

| File | What it is |
| --- | --- |
| `src/Geometry.res` | `node`, the predicates, `splitPolygon`, `signedArea`, `deviation` |
| `src/Rings.res` | `linkedList`, `remove`, `filter`, the block index, `findBridge`, hole elimination |
| `src/Clip.res` | Morton order (`zOrder`, `sortNodes`, `indexCurve`), `isEar`, `cure`, the clipping loop, `earcut` |
| `src/Refine.res` | `refine` |
| `src/Flatten.res` | `flatten` |
| `src/Index.res`, `src/Index.resi` | the boundary and the `op` dispatch; the interface exports only `run` |

## Boundary

The input records are the corpus JS object itself, so reading them converts nothing, and the input is never written. `refine` copies `triangles` with `Array.copy` first. Results are the port's own arrays and numbers, returned as they are (`%identity`). For `flatten` the result is an object literal with `vertices`, `holes` and `dimensions` in that order. `flatten` pushes `point[d]`, an `option<float>` that is `undefined` at run time when the point is shorter than the first, as npm's `p[d]` is. An absent `dim` is 2, a null or absent `holeIndices` is no holes, and an absent `refine` is false.

## Module state

None. Each `earcut` call creates its own `state` record, which holds npm's module-level `filteredOut` (`filtered`) and the block index (`blocks`). The Morton sort and `refine` allocate their buffers per call.

## Deviations from the npm source

None changes a result in the parity domain.

- **Options for npm's `null`.** `linkedList` returns `option<node>`, `findBridge` returns `None` for no bridge, and `prevZ`/`nextZ` are `option<node>`.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. The `steiner` field is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **The block index is an array of `block` records.** Each block holds its head, its stop and its bounding box, where npm keeps a flat `Float64Array` and two node arrays. Blocks are appended in the same order and scanned in append order. `st.blocks` is `Some` exactly while npm's `indexActive` is true, so `remove` grows a block exactly when npm's does.
- **`findBridge`'s first scan answers with an option.** npm returns from inside its loop at two points: a hole vertex that touches the ring, and an intersection exactly at `hx`. `findInBlock` walks a block's live edges until its visit gives a node, and `ray` walks the blocks until one does, so the bridge is returned as `Some` at the same point and the remaining blocks are not visited. The second scan visits every live edge (`scanBlock`).
- **`isEar` covers npm's `isEar` and `isEarHashed`.** It branches on `inv == 0.` as `earcutLinked` does, and runs npm's two loops unchanged.
- **Hole order.** The queue is sorted with `Array.sort`, the engine's stable `Array.prototype.sort`, as npm sorts it. `holeOrder` maps a zero or NaN `compareXYSlope` difference to 0.
- **JS integer semantics.** ReScript's `int` is a 32-bit JS number. `Int.fromFloat` compiles to `x | 0`, which is ECMAScript ToInt32: NaN and ±Infinity become 0, and other values wrap modulo 2^32. `zOrder` relies on that. The shifts and masks are JS's 32-bit operators. `refine`'s hash uses `Math.Int.imul`, as npm does.
- **The outer bounding box folds `Math.min` and `Math.max`**, where npm compares with `<` and `>`. With finite coordinates the two forms differ only in which zero they keep when -0 and +0 meet. No result depends on that. The box feeds only comparisons, the span, and `(x - minX) * invSize`, whose ToInt32 is 0 for either zero.
- **`refine`'s hash table** holds -2 for an empty slot, -1 for a consumed one, and otherwise the half-edge. npm uses `hStamp`/`gen` for empty slots. `queued` (an `array<bool>`) stands for `edgeStamp`.
- **Errors.** An unknown `op` and an empty `flatten` input throw a JS `TypeError`, as npm does. An empty hole slice, which CONTRACT.md excludes, throws a `TypeError` ("empty hole") at the point where npm throws one reading `list.next`.

## Performance accommodations

None.
