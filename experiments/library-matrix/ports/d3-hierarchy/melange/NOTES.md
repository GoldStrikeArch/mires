# d3-hierarchy 3.1.3: Melange port

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to OCaml 5.3,
compiled by Melange 7. It follows `node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function
by function, with d3's loops, explicit stacks, order of operations and error checks. It replaced a
geometry module that a Python generator had emitted from a shared model (2026-11-21).

The OCaml core is shared with the js_of_ocaml port: `hierarchy.ml`, `tree.ml`, `treemap.ml`,
`pack.ml` and `program.ml` are byte-identical in `ports/d3-hierarchy/melange/src` and
`ports/d3-hierarchy/jsoo/src`. Only `host.ml` (the compiler's interop) and the entry module
(`index.ml` here) differ.

| File | d3 source |
| --- | --- |
| `src/hierarchy.ml` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter`, the iterator, `sum`, `sort`, and `stratify` with `.path` |
| `src/tree.ml` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/treemap.ml` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/pack.ml` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/program.ml` | what `libraries/d3-hierarchy/reference.mjs` does |
| `src/host.ml` | Melange's side of the boundary |
| `src/index.ml` | the module's `run` export |

## Boundary

`Host` reads the corpus object in place through `Js.t` object types (`input`, and `datum` for
nested data and stratify rows). Strings are JS strings and numbers are floats, with no conversion;
both `undefined` and `null` count as absent (`Js.Nullable.t`, matching d3's `!= null` and `??`),
with an absent string field reading as `""`. A node keeps its datum, as d3's `node.data`, and the
output reads the id through `Host.id_value` the way the reference does, `n.id ?? n.data.id`
(including `null` or `undefined`). The result is a JS array of rows of strings, `null`, `undefined`
and numbers.

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radius_leaf` and `pack_children_random`. `truthy` is ToBoolean of a
  number, false for 0, -0 and NaN.
- **Math.max, Math.min, Math.sqrt.** `Float.max` and `Float.min` return NaN for a NaN operand and
  order -0 below +0, as JavaScript's do (`Stdlib.max` does neither). Squarify keeps d3's explicit
  comparisons for its running minimum and maximum.
- **Math.round.** `Host.math_round` is the host's `Math.round` (`Js.Math.round`): it rounds halves
  up and returns -0 for [-0.5, 0), which `Float.round` (halves away from zero) does not.
- **Integers.** Melange's `int` is 32-bit; depths, heights and indices fit. binary.js's
  `k + hi >>> 1` is `(k + hi) lsr 1`. The LCG stays in floats, as in d3: `1664525 * s + 1013904223`
  is below 2^53, and `Float.rem` is JavaScript's `%`.
- **Sort.** `Array.stable_sort` (merge sort), with the sign of d3's `b.value - a.value` as the
  comparator, counting NaN as 0 as Array.prototype.sort does.
- **Strings.** Melange strings are JS strings, so `normalize`, `parentof` and `slash` index UTF-16
  code units as d3 does, and lone surrogates survive.

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`. The reference's
  accessors (`d?.value ?? 0`, `d.id`, `b.value - a.value`) are passed as functions.
- A leaf's `children` is the empty array, where d3 leaves the property undefined; d3 never has an
  empty children array, so "has children" is the same test.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here `dice` and `slice` take the children array, the row's bounds in
  it, and the row's value: the row object exists only for `treemapResquarify`, outside the
  contract, so squarify does not collect its rows.
- Stratify marks an imputed path ancestor with `None` data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `nodeByKey` maps to `Unique i | Ambiguous`, for d3's node or
  its `ambiguous` object. d3 appends each missing ancestor to the arrays it is iterating; a queue
  visits them in the same order. d3 pushes each child onto its parent; the children collect in
  per-node lists first.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- tree.js's extra parent of the root wraps `null`; here it wraps the root, which it never reads.
  `parent` is an option only because that extra parent has none.
- The treemap's `paddingStack` is an array per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.
- enclose.js keeps references to the front chain's circles; `pack_enclose_random` takes copies
  (`circle` records). The circles do not move while their enclosure is computed.
- d3's loops that `break` or `continue` an outer label are tail-recursive functions: the
  front-chain search returns the neighbour to drop (`` `Ahead `` or `` `Behind ``), and placing
  the next circle recurses, with the same circle again after an intersection, as `--i` does.

## Module state

`phi` (squarify's default ratio), as in d3's module. It does not change.

## Performance accommodations

None.
