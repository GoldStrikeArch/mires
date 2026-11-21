# d3-hierarchy 3.1.3: Dart port

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md`, compiled by
dart2js. It follows `node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function by function, with
d3's loops, explicit stacks, order of operations and error checks. It replaced a geometry module
that a Python generator had emitted from a shared model (2026-11-21).

| File | d3 source |
| --- | --- |
| `src/hierarchy.dart` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter` (on the `Parent` class), the iterator, `sum`, `sort` |
| `src/stratify.dart` | `stratify` with the default `id` and `parentId`, and `.path` (`normalize`, `parentof`, `slash`) |
| `src/tree.dart` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/treemap.dart` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/pack.dart` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/main.dart` | the boundary: what `libraries/d3-hierarchy/reference.mjs` does |

## Boundary

`run(input)` reads the corpus object in place through `dart:js_interop` extension types (`Input`,
and `Datum` for nested data and stratify rows): strings are JS strings, numbers are `double`s and
an absent field is `null`. A node keeps its datum, as d3's `node.data`, and the output reads the
id the way the reference does, `n.id ?? n.data.id`. The result is a `JSArray` of rows of strings,
`null` and numbers. dart2js cannot emit ES modules, so `main()` publishes `run` on
`globalThis.__matrixExports` for the builder's facade.

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN.
- **Math.max, Math.min, Math.sqrt.** dart2js compiles `dart:math`'s `max`, `min` and `sqrt` to the
  host's `Math` functions, so NaN and -0 behave as in d3. Squarify keeps d3's explicit comparisons
  for its running minimum and maximum.
- **Math.round.** `roundNode` calls the host's `Math.round` (`@JS('Math.round')`), which rounds
  halves up and returns -0 for [-0.5, 0); `roundToDouble` rounds halves away from zero.
- **Integers.** On the web an `int` is a JS number. Depths, heights and child numbers stay small;
  `(k + hi) >>> 1` is binary.js's. The LCG is in `double`s, as in d3: `1664525 * s + 1013904223`
  is below 2^53, and `%` of positive doubles is JavaScript's.
- **Sort.** `List.sort` is not stable, so the comparator breaks ties by a child's original
  position, which is the order Array.prototype.sort keeps. It returns the sign of d3's
  `b.value - a.value`, counting NaN as a tie as Array.prototype.sort counts it as 0.
- **Strings.** UTF-16 throughout; `normalize`, `parentof` and `slash` index code units as d3 does.

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`. The reference's
  accessors (`d?.value ?? 0`, `d.id`, `b.value - a.value`) are passed as functions.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here they take the children and their value: the row object exists
  only for `treemapResquarify`, outside the contract, so squarify does not collect its rows.
- Stratify marks an imputed path ancestor with null data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `ambiguous` is a node created per call.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- tree.js's extra parent of the root wraps `null`; here it wraps the root, which it never reads, so
  `TreeNode.node` is not nullable. Its `parent` is `late`.
- The treemap's `paddingStack` is a list per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.

## Module state

`_phi` (squarify's default ratio) and the LCG constants, as in d3's modules. Neither changes.

## Performance accommodations

None.
