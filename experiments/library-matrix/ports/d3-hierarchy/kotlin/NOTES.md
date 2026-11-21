# d3-hierarchy 3.1.3: Kotlin/JS port

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md`. It follows
`node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function by function, with d3's loops, explicit
stacks, order of operations and error checks. It replaced a geometry module that a Python
generator had emitted from a shared model (2026-11-21).

| File | d3 source |
| --- | --- |
| `src/Hierarchy.kt` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter`, the iterator, `sum`, `sort` |
| `src/Stratify.kt` | `stratify` with the default `id` and `parentId`, and `.path` (`normalize`, `parentof`, `slash`) |
| `src/Tree.kt` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/Treemap.kt` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/Pack.kt` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/Main.kt` | the boundary: what `libraries/d3-hierarchy/reference.mjs` does |

## Boundary

`run(input)` reads the corpus object in place through external interfaces (`Input`, and `Datum`
for nested data and stratify rows): strings are JS strings, numbers are `Double`s. A node keeps its
datum, as d3's `node.data`, and the output reads the id the way the reference does,
`n.id ?? n.data.id`. The result is an `Array<Array<Any?>>`, a JS array of rows of strings, `null`
and numbers. `run` is a native ES module export through `@JsExport`.

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN.
- **Math.max, Math.min, Math.sqrt.** `kotlin.math.max`, `min` and `sqrt` compile to the host's
  `Math` functions, so NaN and -0 behave as in d3. Squarify keeps d3's explicit comparisons for
  its running minimum and maximum.
- **Math.round.** `roundNode` calls the host's `Math.round` through an `external object`
  (`@JsName("Math")`): it rounds halves up and returns -0 for [-0.5, 0), which `roundToInt` and
  `kotlin.math.round` (half-even) do not.
- **Integers.** Depths, heights and child numbers are `Int`s. `(k + hi) ushr 1` is binary.js's
  `k + hi >>> 1`. The LCG stays in `Double`s, as in d3: `1664525 * s + 1013904223` is below 2^53.
- **Sort.** `sortWith` is stable (Kotlin/JS uses the host's stable sort), and the comparator
  returns the sign of d3's `b.value - a.value`, counting NaN as 0 as Array.prototype.sort does.
- **Strings.** UTF-16 throughout; `normalize`, `parentof` and `slash` index code units as d3 does.

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`. The reference's
  accessors (`d?.value ?? 0`, `d.id`, `b.value - a.value`) are passed as lambdas.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here they take the children and their value: the row object exists
  only for `treemapResquarify`, outside the contract, so squarify does not collect its rows.
- Stratify marks an imputed path ancestor with null data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `ambiguous` is a node created per call.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- tree.js's extra parent of the root wraps `null`; here it wraps the root, which it never reads, so
  `TreeNode.node` is not nullable. Its `parent` is `lateinit`.
- The treemap's `paddingStack` is an array per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.

## Module state

`phi` (squarify's default ratio) and the LCG constants, as in d3's modules. Neither changes.

## Performance accommodations

None.
