# d3-hierarchy 3.1.3: ReScript port

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to ReScript 12.
It follows `node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function by function, with d3's
loops, explicit stacks, order of operations and error checks. It replaced a geometry module that a
Python generator had emitted from a shared model (2026-11-21).

| File | d3 source |
| --- | --- |
| `src/Hierarchy.res` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter`, the iterator, `sum`, `sort`, and `stratify` with `.path` |
| `src/Tree.res` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/Treemap.res` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/Pack.res` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/Index.res` | the boundary: what `libraries/d3-hierarchy/reference.mjs` does |

## Boundary

The input record (`input`, and `datum` for nested data and stratify rows) is the corpus object
itself, with optional fields for what may be absent: no conversion. Strings are JS strings and
numbers are floats. A node keeps its datum, as d3's `node.data`, and the output reads the id the
way the reference does, `n.id ?? n.data.id`. The result is a JS array of rows of strings, `null`
and numbers (`cell` is an identity cast).

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN.
- **Math.** `Math.max`, `Math.min`, `Math.sqrt`, `Math.round` and `Float.mod` are the host's
  functions and `%`. Squarify keeps d3's explicit comparisons for its running minimum and maximum.
- **Integers.** Depths, heights and indices are `int`s; binary.js's `k + hi >>> 1` is
  `Int.shiftRightUnsigned(k + hi, 1)`. The LCG stays in floats, as in d3:
  `1664525 * s + 1013904223` is below 2^53.
- **Sort.** `Array.sort` is Array.prototype.sort, called with d3's comparator `b.value - a.value`.
- **Strings.** UTF-16 throughout; `normalize`, `parentof` and `slash` index code units with
  `String.charAt`, as d3 indexes them.

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`. The reference's
  accessors (`d?.value ?? 0`, `d.id`, `b.value - a.value`) are passed as functions.
- A leaf's `children` is the empty array, where d3 leaves the property undefined; d3 never has an
  empty children array, so "has children" is the same test.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here they take the children (a row's slice of them) and their value:
  the row object exists only for `treemapResquarify`, outside the contract, so squarify does not
  collect its rows.
- Stratify marks an imputed path ancestor with `None` data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `nodeByKey` maps to `Unique(node) | Ambiguous`, for d3's
  node or its `ambiguous` object. An undefined or empty id reads as `""`, which names nothing, as
  d3's `id != null && (id += "")` does.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- tree.js's extra parent of the root wraps `null`; here it wraps the root, which it never reads.
  `parent` is an option only because that extra parent has none.
- The treemap's `paddingStack` is an array per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.
- enclose.js keeps references to the front chain's circles; `packEncloseRandom` takes copies
  (`circle` records). The circles do not move while their enclosure is computed.
- ReScript has no `break` or labelled `continue`, so those loops are tail-recursive functions,
  which the compiler turns into loops: squarify's row search returns the row's value and end
  (the value with d3's add-then-subtract rounding), apportion walks the contours, the front-chain
  search returns the neighbour to drop (`Ahead` or `Behind`), and placing the next circle
  recurses, with the same circle again after an intersection, as `--i` does.

## Module state

`phi` (squarify's default ratio), as in d3's module. It does not change.

## Performance accommodations

None.
