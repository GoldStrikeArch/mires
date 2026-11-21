# d3-hierarchy 3.1.3: Go port (GopherJS)

A hand-written, faithful port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to Go 1.21,
compiled by GopherJS 1.21.0 (`gopherjs build -m`). It follows
`node_modules/d3-hierarchy/dist/d3-hierarchy.mjs` function by function, with d3's loops, explicit
stacks, order of operations and error checks. It replaced a geometry module that a Python
generator had emitted from a shared model (2026-11-21).

| File | d3 source |
| --- | --- |
| `src/hierarchy.go` | `Node`, `hierarchy`, `computeHeight`, `eachBefore`, `eachAfter`, the iterator, `sum`, `sort` |
| `src/stratify.go` | `stratify` with the default `id` and `parentId`, and `.path` (`normalize`, `parentof`, `slash`) |
| `src/tree.go` | `tree` (`TreeNode`, `treeRoot`, `firstWalk`, `secondWalk`, `apportion`, …) and `cluster` |
| `src/treemap.go` | `treemap`, `treemapDice`, `treemapSlice`, `treemapSliceDice`, `treemapSquarify`, `treemapBinary`, `roundNode`, `partition` |
| `src/pack.go` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/main.go` | the boundary: what `libraries/d3-hierarchy/reference.mjs` does; publishes `run` on `js.Module.exports` |

## Boundary

`run(input)` reads the corpus object through `*js.Object`. A node keeps its datum (`*js.Object`,
nil for an imputed ancestor), as d3's `node.data`. Stratify's ids become Go strings, which are
UTF-8 (`.String()`); an id of nested data is returned as the host string it is
(`node.data.Get("id")`), as the reference's `n.data.id`. The result is a slice of rows of strings,
`nil` and numbers, which GopherJS converts to JS arrays. An error is a panic, which GopherJS throws
to the caller.

## Numeric and string semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN.
- **Math.max, Math.min, Math.sqrt.** Go's `math.Max` and `math.Min` order -0 below +0 and return
  NaN for a finite/NaN pair, but GopherJS compiles Go's stdlib checks verbatim (`IsInf` before
  `IsNaN`), so `math.Max(+Inf, NaN) == +Inf` and `math.Min(-Inf, NaN) == -Inf` where JavaScript's
  `Math.max` and `Math.min` return `NaN`. In d3's uses (`enclosesWeak`, `place`, pack sizes,
  squarify ratios), such an infinity/NaN pair cannot change a result (`enclosesWeak`'s `dr` is
  already NaN, and squarify cannot reach it after a finite `minRatio`). Squarify keeps d3's
  explicit comparisons for its running minimum and maximum.
- **Math.round.** `roundNode` calls the host's `Math.round` through `js.Global`: it rounds halves
  up and returns -0 for [-0.5, 0), which `math.Round` (halves away from zero) does not.
- **Integers.** GopherJS's `int` is 32-bit; depths, heights and indices fit. binary.js's
  `k + hi >>> 1` is `int(uint(k+hi) >> 1)`, as `sort.Search` writes it. The LCG stays in
  `float64`s, as in d3: `1664525 * s + 1013904223` is below 2^53, and `math.Mod` is JavaScript's
  `%`.
- **Sort.** `sort.SliceStable`, with `children[j].value-children[i].value < 0` as d3's comparator:
  a NaN difference compares neither way, as Array.prototype.sort counts it as 0.
- **Strings.** Stratify's ids are UTF-8. `normalize`, `parentof` and `slash` look only at `/` and
  `\`, which are single bytes that never occur inside a multi-byte sequence, so byte indices give
  d3's results. A lone surrogate is outside the domain (CONTRACT.md, "Domain").

## Deviations from the npm source

- Only the contract's options exist: the default `children`, `id` and `parentId` accessors, the
  default separation, a constant padding, the three tilers and no `pack.radius`.
- d3's `treemapDice` and `treemapSlice` take a parent, or a squarified row object
  `{value, dice, children}`. Here they take the children and their value: the row object exists
  only for `treemapResquarify`, outside the contract, so squarify does not collect its rows.
- Stratify marks an imputed path ancestor with nil data from the start; d3 marks it with its
  `imputed` object and replaces that by null before returning. Every row has data, so the test
  `root.data === imputed` is the same. `ambiguous` is a node created per call. An id that is
  undefined, null or empty reads as `""`, which names nothing, as d3's `id != null && (id += "")`
  does; `Node.id` is `""` where d3 leaves `node.id` undefined.
- d3 parents the root to `preroot` (depth -1) while it sets depths; here the root has no parent
  and gets depth 0, the same values.
- The treemap's `paddingStack` is a slice per call, sized by the root's height; d3 keeps it in the
  layout's closure and resets it to `[0]` after each call.
- enclose.js keeps references to the front chain's circles; `packEncloseRandom` takes copies
  (`circle` values). The circles do not move while their enclosure is computed.

## Module state

`phi` (squarify's default ratio), as in d3's module. It does not change.

## Performance accommodations

GopherJS compiles every caller of a function value or interface method as a resumable state
machine (README, "Per-language notes"). So the hot paths call no function values:

- `eachBefore`, `eachAfter` and `breadthFirst` return the nodes in d3's visiting order, and the
  layouts loop over them, instead of taking a callback. No d3 callback adds or removes a child;
  the one that reorders children (`sort`) gives the same result in either visiting order, and the
  output's walk comes after it.
- The tiling method is a `tile` constant whose `layOut` method switches on it, where d3 passes the
  tiling function.
- `sum` and `sortByValue` have the reference's accessor (`d?.value ?? 0`) and comparator
  (`b.value - a.value`) built in. `sort.SliceStable`'s `less` is the one function value left; it
  makes only `sortByValue` and `run` resumable.
