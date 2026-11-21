# d3-hierarchy 3.1.3: Elm port

A hand-written port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to Elm 0.19, with
elm/core and elm/json only. It keeps d3-hierarchy's algorithms
(`node_modules/d3-hierarchy/dist/d3-hierarchy.mjs`) over an immutable arena of node records. The
review of 2026-11-20 found its comparisons wrong for NaN, its JavaScript truthiness inexact and
its nested-data decoder recursive; all three were fixed on 2026-11-21.

| File | d3 source |
| --- | --- |
| `src/Main.elm` | the boundary, `hierarchy`, `stratify` with `.path`, `sum`, `sort` and the iterator |
| `src/Arena.elm` | the node record and the arena's `get`, `alter` and pre- and post-order `walk` |
| `src/Tree.elm` | `tree` (Buchheim et al.'s walks and `apportion`) and `cluster` |
| `src/Rectangles.elm` | `treemap`, `partition`, `treemapDice`/`treemapSlice`, `treemapSliceDice`, `treemapBinary`, `treemapSquarify`, `roundNode` |
| `src/Circles.elm` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |
| `src/JsMath.elm` | JavaScript's ToBoolean of a number, `Math.max` and `Math.min` |
| `js/index.js` | the JS host: sends the input through the `request` port and returns the `response` |

## Boundary

The host passes the input object through a port. `Json.Decode` reads the options and the stratify
rows; nested `data` stays a raw `Json.Decode.Value`, and `nested` decodes one node at a time
(its id, value and raw children) while it builds the arena on an explicit stack, as d3's
`hierarchy()` does. No decoder recurses through the nesting, so depth is bounded by memory, not by
the JS stack. The result is encoded with `Json.Encode` and returned by the host; an `Err` becomes
a thrown `Error`. Strings are JS strings (UTF-16).

## Numeric semantics

- **Comparisons.** Elm compiles `a > b` and `a >= b` between two non-literal Floats to
  `_Utils_cmp(a, b) > 0` and `>= 0`, and `_Utils_cmp` answers "greater" when an operand is NaN, so
  both are true where JavaScript's are false. `<` and `<=` are false for NaN, as in JavaScript.
  So every Float comparison that can see NaN is written as `<` or `<=`: d3's `a > b` is `b < a`
  (squarify's `newRatio > minRatio`, binary's split direction, `enclosesWeak`, `place`,
  `intersects`, the tree's right-most node). A comparison with a literal compiles to JavaScript's
  own operator. Before this, NaN made squarify break rows early and pack loop forever where d3
  throws.
- **Math.max and Math.min.** `max` and `min` compare the same way and choose between -0 and +0
  by argument order; `JsMath.jsMax` and `jsMin` return NaN for a NaN operand and order -0 below
  +0. Squarify's running minimum and maximum are d3's explicit comparisons.
- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `JsMath.truthy` is ToBoolean of a
  number, false for 0, -0 and NaN.
- **Rounding and the LCG.** `round` is `Math.round`. The LCG is in Floats, as in d3, with `floor`
  for the modulus.
- **Sort.** `List.sortWith` sorts with the host's stable Array.prototype.sort; `compare` of the
  two values orders equal infinities as ties, as d3's NaN difference does.

## Deviations from the npm source

- Nodes are records in an `Array` and refer to each other by index; `-1` is the absent parent,
  thread or ancestor, and marks an ambiguous id in stratify's `Dict`. Elm has no mutation, so
  every layout step returns a new arena.
- Only the contract's options exist (default accessors, the default separation, a constant
  padding, the three tilers, no `pack.radius`).
- The sum pass also computes each node's height (one more than its tallest child) and sorts its
  children, where d3 runs `computeHeight` from every node and sorts in a later pre-order pass. The
  values are the same: a node's sort reads only its children's final aggregates.
- Pack works on `Circle` records copied from the nodes and writes them back; enclose shuffles a
  copy, as d3 does.
- d3's loops are tail-recursive functions, which Elm compiles to loops; binary's partition
  recurses as d3's does.
- Error messages differ from d3's; the checker records messages, it does not compare them.

## Module state

None (the host keeps the Elm program instance between calls).

## Performance accommodations

None.
