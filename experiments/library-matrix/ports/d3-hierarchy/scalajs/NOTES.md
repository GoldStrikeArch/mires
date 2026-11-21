# d3-hierarchy 3.1.3: Scala.js port

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to Scala 3
(Scala.js 1.22). It follows `node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function by
function, with d3's loops, explicit stacks, order of operations and error checks. It replaced a
geometry module that a Python generator had emitted from a shared model (2026-11-21).

| File | d3 source |
| --- | --- |
| `src/Hierarchy.scala` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter` (on the `Parent` trait), the iterator, `sum`, `sort` |
| `src/Stratify.scala` | `stratify` with the default `id` and `parentId`, and `.path` (`normalize`, `parentof`, `slash`) |
| `src/Tree.scala` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/Treemap.scala` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/Pack.scala` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/Main.scala` | the boundary: what `libraries/d3-hierarchy/reference.mjs` does |

## Boundary

`run(input)` reads the corpus object in place through `@js.native` facades (`Input`, and `Datum`
for nested data and stratify rows): strings are JS strings, numbers are `Double`s, absent fields
are `js.UndefOr`. A node keeps its datum, as d3's `node.data`, and the output reads the id the way
the reference does, `n.id ?? n.data.id`. The result is a `js.Array` of rows of strings, `null` and
numbers, exported with `@JSExportTopLevel("run")`.

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN.
- **Math.max, Math.min, Math.sqrt.** `math.max` and `math.min` on `Double`s have Java's
  semantics, which are JavaScript's for NaN and -0. Squarify keeps d3's explicit comparisons for
  its running minimum and maximum.
- **Math.round.** `roundNode` calls the host's `js.Math.round`, which rounds halves up and returns
  -0 for [-0.5, 0); `math.round` returns a `Long` and `math.rint` rounds halves to even.
- **Integers.** Depths, heights and child numbers are `Int`s; `(k + hi) >>> 1` is binary.js's. The
  LCG stays in `Double`s, as in d3: `1664525 * s + 1013904223` is below 2^53.
- **Sort.** `sortInPlaceWith` is a stable merge sort, with `b.value - a.value < 0` as d3's
  comparator. A NaN difference compares neither way, as Array.prototype.sort counts it as 0.
- **Strings.** UTF-16 throughout; `normalize`, `parentof` and `slash` index code units as d3 does.

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`. The reference's
  accessors (`d?.value ?? 0`, `d.id`, `b.value - a.value`) are passed as functions.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here they take the children and their value: the row object exists
  only for `treemapResquarify`, outside the contract, so squarify does not collect its rows.
- Stratify marks an imputed path ancestor with `None` data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `ambiguous` is a node created per call. An id that is
  undefined or empty reads as `""`, which names nothing, as d3's `id != null && (id += "")` does.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- tree.js's extra parent of the root wraps `null`; here it wraps the root, which it never reads.
  `TreeNode.parent` starts as `null` and is set for every node before the walks; only the extra
  parent keeps it.
- The treemap's `paddingStack` is an array per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.
- Scala 3 has no do-while and no labelled `continue`: squarify's first `do … while` is unrolled,
  and packSiblings's search loop is `while { …; cond } do ()` with a `retry` flag in place of
  `continue pack`. `extendBasis` searches with `find` in d3's order, without non-local returns.

## Module state

`phi` (squarify's default ratio) and the LCG constants, as in d3's modules. Neither changes.

## Performance accommodations

None.
