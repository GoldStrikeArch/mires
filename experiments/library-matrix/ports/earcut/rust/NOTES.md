# earcut 3.2.4: Rust port (mires)

A faithful port of the complete public API (`earcut`, `deviation`, `flatten`, `refine`) and
contract pipelines (`earcutRefine`, `batch`) defined in `libraries/earcut/CONTRACT.md`: the exact
algorithms, data structures, tie-breaks, and IEEE-754 `f64` operation order of
`node_modules/earcut/src/earcut.js`, in safe `no_std` Rust (`#![deny(unsafe_code)]`, no `js!{}`).
It reaches exact parity on the full corpus across Node, Chrome, and Firefox (both minified
and readable artifacts). The independent differential suite covers another 3,239 inputs and 100
benchmark variants.

| File | What it is |
| --- | --- |
| `src/lib.rs` | `#![no_std]` / `#![no_main]` / `#![deny(unsafe_code)]` crate root, fail-fast panic handler via `mires_js::throw_value`, and `#[js_export] pub fn matrix_run` |
| `src/earcut.rs` | Arena-backed circular doubly-linked `Node` list, `signed_area`, `linked_list`, `filter_points`, `eliminate_holes` with stable `compare_xy_slope` sort and `K = 16` append-only block-bbox index (`index_segment`, `grow_block`, `find_hole_bridge`), 30-bit Morton `z_order` + `index_curve` + `sort_nodes` (insertion sort for `n <= 32`, 4-pass 8-bit LSD radix sort for `n > 32`), 3-stage `earcut_linked` (`filter_points` fixpoint -> `cure_local_intersections` -> `split_earcut`), and exact geometry predicates |
| `src/refine.rs` | Constrained Delaunay Lawson edge-flipping `refine` (`orient`, `in_circle` with `1e-13 * s * s` issue #205 tolerance, power-of-two open-addressing edge hash with `0x9e3779b1` / `0x85ebca6b` `wrapping_mul`, fused stack seeding, `edge_stamp` dedup) |
| `src/util.rs` | `deviation` (exact `f64` area ratio and `EPSILON` zero-area threshold) |
| `src/boundary.rs` | `JsValue` input parsing (typed `get_f64`/`get_u32` index bindings, no input mutation), `flatten` with the first point’s stride and native JS array writes to preserve missing coordinates as `undefined`, pre-sized JS array writes, and `TypeError` on unknown `op` / empty `flatten` |

## Decisions

- **Exact `earcut 3.2.4` algorithms, not an alternative triangulator.** Every v3.2.4 detail matches
  `src/earcut.js`: the append-only `K = 16` block-bbox hole-bridge index scanned in append order
  (including `grow_block` on `remove_node` while `index_active` and lazy `liveBlockHead`/`liveBlockStop`
  advancement), single-vertex Steiner hole preservation (`node.steiner`), T-junction horizontal-edge
  bridges (`p.y == hy && p.next.y == hy && p.next.x > hx`), `sector_contains_sector`, advancing to
  `next` (not `next.next`) after ear removal, 4-pass LSD radix sort for `n > 32` / insertion sort
  for `n <= 32`, exact `f64` expression parenthesisation without FMA, and `refine`'s `1e-13 * s * s`
  cocircular tolerance margin.
- **Append-only node arena.** `Node`s live in a per-call `Vec<Node>` indexed by `u32` (`NONE = u32::MAX`
  for `prev_z`/`next_z`). Removed nodes are never recycled during a call, preserving their `prev`/`next`
  links so `nodes[nodes[p].prev].next != p` dead-node detection and forward scans in `find_hole_bridge`
  match JS object references identically in safe Rust.
- **Zero cross-call mutable state.** Unlike `earcut.js` (which reuses module-level scratch arrays),
  this port allocates per call with zero `static` mutable state (`0` module-state lint findings).
- **No `core::slice::sort` and no `core::fmt`.** `stable_sort_holes` uses a compact stable
  insertion + bottom-up merge sort, and panics/errors use static strings. Under mires one
  `core::slice::sort` instantiation is 13–15 KB of JavaScript: 14.9 KB in the minisearch Rust port
  and 13.8 KB in d3-hierarchy's, by the optimizer report of 2026-11-21.
- **Steiner flag.** npm keeps single-vertex holes in a module-level `Set` named `steiners`; the
  port uses the per-node `steiner` flag that `libraries/earcut/CONTRACT.md` describes and shows
  equivalent ("Vertex `Node`").
- **Out of domain: an empty hole slice** (`holeIndices: [4, 4, 7]`). `linked_list` returns `NONE`
  and `eliminate_holes` skips the hole, where npm throws a `TypeError`. The contract's
  strictly-increasing rule for `holeIndices` excludes this input.

## Performance accommodations (PERF-004)

The matrix builds client crates with `JSC_RUST_MIR_INLINE=off`, and under mires every `Vec` index
runs `$rt_unscale` and `$rt__blk_get`, whose one-entry block cache thrashes when a loop alternates
between two `Vec`s (`MIRES-BUGS.md`, `PERF-004`). Four shapes in the source follow from this. Each
site carries a `// PERF-004:` comment that points here, so the accommodation can be removed when
the cliff is fixed.

- **Hoisted slice borrows.** Hot loops (`earcut_linked`, `find_hole_bridge`, `index_curve`,
  `refine`) bind `&[Node]` / `&mut [Node]` / `&[f64]` slices once, outside the loop.
- **Two copies of `removeNode` and `filterPoints`.** npm has one `removeNode`, which calls
  `growBlock` when `indexActive` is set, and one `filterPoints`. The port has `remove_node_sl` and
  `filter_points_sl`, free functions over the borrowed node slice that never touch the block index,
  and `EarcutState::remove_node_indexed` and `filter_points_indexed`, which also grow the block
  bbox. `eliminate_hole`, the only code that runs while `index_active` is true, calls the
  `_indexed` pair. Every other caller runs while npm's flag is false and calls the `_sl` pair. The
  split therefore matches npm's flag test. It lets the hot callers work on one borrowed slice
  instead of alternating between `nodes` and `block_bbox`.
- **Macros for the predicates.** `min3`, `max3`, `point_in_triangle`, `area_sl` and
  `z_order_inline` in `earcut.rs`, and `next_he` in `refine.rs`, are `macro_rules!` rather than
  functions. With MIR inlining off, `#[inline]` does not guarantee that a call disappears; the
  macros expand inside the loops and read the slice those loops already hold.
- **Offset sentinels in `refine`.** npm's `he` uses -1 for a boundary edge and `hTable` uses -1 for
  a consumed slot (and `hStamp`/`gen` for empty slots). The port stores `he[e] = s + 1` for twin
  `s` (0 for boundary) and `h_table[h] = s + 2` for live edge `s` (1 for consumed, 0 for empty),
  so every buffer starts as `vec![0u32; n]`. A non-zero `vec![value; n]` goes through
  `alloc::vec::from_elem`, whose fill loop allocates a `{ buf, off }` object per element under mires.

## mires bugs and performance cliffs

- **Compiler bugs / refusals met:** none (`289/289` items compiled cleanly: `99` JIR, `190`
  non-ordinary fallback, `ordinaryFallback: 0`, ES2018 gate passed; exact parity across Node,
  Chrome, and Firefox for both `index.min.js` and `index.js`).
- **Performance cliff logged (`PERF-004` in `MIRES-BUGS.md`, alongside `PERF-002`):**
  - Before hoisting slice borrows out of `earcut_linked` / `find_hole_bridge` and switching
    `refine` to zero-based sentinels, V8 CPU profiling showed `27.6%` of runtime in `$rt_unscale`
    (`8.6%`) + `$rt__blk_get` (`6.6%`) + `Vec<Node>::index` (`6.2%`) + `$rt__retype` (`3.9%`) +
    `$rt_slice_from_parts` (`1.7%`) + `$rt_unscale_scalar` (`1.5%`) + `$rt__heap_block` (`1.2%`)
    due to 1-entry `_blkK` cache thrashing between `nodes` and `triangles`/`block_bbox`/`sort_arr`,
    plus `2.0%` in non-zero `vec![-2i32; size]` fill loops (~4.8–5.3× vs JS on Node).
  - After hoisting slice borrows and using zero-based sentinels, Node ratios improved to ~3.26×
    (`bench-earcut`) and ~2.67× (`bench-refine`), Chrome to ~2.73× / ~2.30×, and Firefox to
    ~1.73× / ~1.07×. Remaining Node overhead is concentrated in `is_ear_hashed_sl` (`46.6%` of
    CPU, where JIR re-evaluates `a.buf[a.off + g]` 4× per loop step on a 48×-over-allocated-then-shrunk
    `Vec<Node>` JS Array) and `PERF-002` zeroed-buffer double-init (`6.1%`) in `refine`.

## Size

`dist/index.min.js` is 67,060 B raw / 20,870 B gzip-9 / 17,568 B brotli-11 (exact figures in
`sizes.json` and `MATRIX.md`), with `0` embedded JS, `0` eval refs, `0` host JS, and `0` module
state. By Rollup attribution (`dist/meta.json`): `64,522 B` program (`matrix_earcut` + `alloc`/`core`
helpers) and `24,059 B` minimal `runtime/shim.js` before Terser. The `// PERF-004:` comments added
on 2026-11-21 moved source lines, and with them the panic locations in the output, by 4 B.
