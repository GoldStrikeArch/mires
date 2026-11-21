# earcut 3.2.4: Melange port

A port of `earcut.js` 3.2.4 to OCaml 5.3, compiled by Melange 7.0.1. It covers the API and the pipelines in `libraries/earcut/CONTRACT.md`. It follows the npm source function by function: the same linked rings, `K = 16` block index, Morton sort, three-stage `earcutLinked` and Lawson `refine`, with the same floating-point expression order.

**Shared source.** `src/geometry.ml`, `src/rings.ml`, `src/clip.ml`, `src/refine.ml`, `src/flatten.ml` and `src/vector.ml` are byte-identical in the Melange and js_of_ocaml ports (README "Per-language notes"). Only the entry module differs, as the per-compiler interop: `src/index.ml` here. See `ports/earcut/jsoo/NOTES.md` for the other side.

| File | What it is |
| --- | --- |
| `src/geometry.ml` | `node`, the predicates, `split_polygon`, `signed_area`, `deviation` |
| `src/rings.ml` | `linked_list`, `remove`, `filter`, the block index, `find_bridge`, hole elimination |
| `src/clip.ml` | Morton order (`z_order`, `sort_nodes`, `index_curve`), `is_ear`, `cure`, the clipping loop, `earcut` |
| `src/refine.ml` | `refine` |
| `src/flatten.ml` | `flatten` |
| `src/vector.ml` | a growable array, the per-call storage for triangles, blocks and the Morton sort |
| `src/index.ml` | the boundary and the `op` dispatch |

## Boundary

Melange arrays are JS arrays, so `index.ml` reads the input's own arrays in place through `[@mel.get_index]` and never writes them. `refine` copies `triangles` with `Array.copy` first. Results are returned as they are (`%identity`). For `flatten` the result is built by a `[@mel.obj]` external with `vertices`, `holes` and `dimensions` in that order. `flatten` pushes `~missing` (`Js.undefined`) for each component a point shorter than the first lacks, as npm's `p[d]` gives `undefined`. An absent `dim` is 2, and a null or absent `holeIndices` is no holes (`Js.Nullable.toOption`). An absent `refine` in a batch item is `undefined`, which the compiled `if` treats as false.

## Module state

None. Each `earcut` call creates its own `state` record, which holds npm's module-level `filteredOut` (`filtered`) and the block index (`blocks`). The Morton sort and `refine` allocate their buffers per call.

## Deviations from the npm source

None changes a result in the parity domain.

- **Options for npm's `null`.** `linked_list` returns `node option`, `find_bridge` returns `None` for no bridge, and `prev_z`/`next_z` are `node option`.
- **The Steiner flag.** npm keeps single-vertex holes in the module-level `steiners` set. The `steiner` field is set at the same point instead. CONTRACT.md ("Vertex `Node`") shows the two are equivalent.
- **The block index is a `Vector` of `block` records.** Each block holds its head, its stop and its bounding box, where npm keeps a flat `Float64Array` and two node arrays. Blocks are appended in the same order and scanned in append order. `st.blocks` is `Some` exactly while npm's `indexActive` is true, so `remove` grows a block exactly when npm's does.
- **`find_bridge`'s first scan answers with an option.** npm returns from inside its loop at two points: a hole vertex that touches the ring, and an intersection exactly at `hx`. `find_in_block` walks a block's live edges until its visit gives a node, and `ray` walks the blocks until one does, so the bridge is returned as `Some` at the same point and the remaining blocks are not visited. The second scan visits every live edge (`scan_block`).
- **`is_ear` covers npm's `isEar` and `isEarHashed`.** It branches on `inv = 0.` as `earcutLinked` does, and runs npm's two loops unchanged.
- **Hole order.** `Array.stable_sort` with `hole_order` keeps npm's order. That is the engine's stable `Array.prototype.sort` over `compareXYSlope`, with a zero or NaN difference as a tie.
- **JS integer semantics.** OCaml's `int` is 32 bits on both compilers.
  - `int_of_float` compiles to `x | 0`, which is ECMAScript ToInt32. NaN and ±Infinity become 0, and other values wrap modulo 2^32. `z_order` relies on that, where native OCaml leaves an out-of-range conversion unspecified.
  - `lsl`, `land` and `lor` are JS's 32-bit operators, and `lsr` is `>>>`.
  - `refine`'s hash multiplies `int`s, which wrap as npm's `Math.imul` does.
- **`min` and `max`.** OCaml's polymorphic `min` and `max` stand for npm's `Math.min` and `Math.max`. That covers the triangle box in `is_ear`, `on_segment`, `intersects_polygon` and `deviation`. They also fold the outer box, where npm compares with `<` and `>`. With finite coordinates they differ from JS only in which zero they return when -0 and +0 meet: OCaml returns the first argument. No result depends on that. The boxes feed only comparisons, spans, and `(x - minX) * invSize`, whose ToInt32 is 0 for either zero.
- **`refine`'s hash table** holds -2 for an empty slot, -1 for a consumed one, and otherwise the half-edge. npm uses `hStamp`/`gen` for empty slots. `queued` (a `bool array`) stands for `edgeStamp`.
- **Errors.** An unknown `op` and an empty `flatten` input raise `Invalid_argument`, where npm throws a TypeError. An empty hole slice, which CONTRACT.md excludes, raises `Invalid_argument "empty hole"`. The harness compares only that both threw.

## Performance accommodations

None.
