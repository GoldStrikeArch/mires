# d3-hierarchy 3.1.3: Rust port (mires)

A hand-written port of the subset in `libraries/d3-hierarchy/CONTRACT.md` to `no_std` Rust,
compiled by mires. It keeps d3-hierarchy's algorithms (`node_modules/d3-hierarchy/dist/d3-hierarchy.mjs`)
over an arena of nodes. The review of 2026-11-20 found its JavaScript truthiness and `Math.max`
handling of NaN and -0 inexact; both were fixed on 2026-11-21.

| File | d3 source |
| --- | --- |
| `src/lib.rs` | the boundary, `hierarchy`, `stratify` with `.path`, `sum`, `sort`, the iterator and `cluster` |
| `src/tidy.rs` | `tree` (Buchheim et al.'s `firstWalk`, `apportion`, `executeShifts`, `secondWalk`) |
| `src/rectangles.rs` | `treemap`, `partition`, `treemapDice`/`treemapSlice`, `treemapSliceDice`, `treemapBinary`, `treemapSquarify`, `roundNode` |
| `src/circles.rs` | `pack`, `packSiblingsRandom` and its front chain, `packEncloseRandom`, `lcg`, `shuffle` |

## Boundary

The input is a `JsValue`, read field by field with `Reflect.get`. Ids are read into Rust strings
(`read::<&str>()`, UTF-8), so a lone surrogate becomes U+FFFD; that is outside the domain
(CONTRACT.md, "Domain"). The result is a `JsArray` of rows of host strings, `null` and numbers.
`#[js_export] fn matrix_run` is re-exported as `run`; a panic throws through
`mires_js::throw_value`.

## Numeric semantics

- **Truthiness.** d3 tests numbers as booleans: `value ? … : y1` in binary and squarify,
  squarify's `!sumValue`, pack's `if (d2)`, `parent.value && …` in dice and slice, and the `|| 0`
  coercions in `sum`, `radiusLeaf` and `packChildrenRandom`. `truthy` is ToBoolean of a number,
  false for 0, -0 and NaN. (These tested `!= 0`, so an aggregate of NaN gave NaN where d3 takes
  the other branch.)
- **Math.max and Math.min.** `js_max` and `js_min` return NaN for a NaN operand and order -0 below
  +0. `f64::max` and `f64::min` return the other operand for a NaN and either zero for a pair of
  zeros. Squarify's running minimum and maximum are d3's explicit comparisons.
- **Host math.** `Math.sqrt` and `Math.round` are bound from the host (`js_bindings!`); the
  rounding is JavaScript's, halves up and -0 for [-0.5, 0).
- **LCG.** `u32` wrapping arithmetic, which equals d3's `(1664525 * s + 1013904223) % 2^32` on
  doubles: the double product is exact below 2^53.
- **Sort.** `sort_by` is stable, with `partial_cmp` of the two values; a NaN pair is `Equal`, as
  Array.prototype.sort counts d3's NaN difference as 0.

## Deviations from the npm source

- Nodes live in a `Vec` and refer to each other by index; `NONE` (`usize::MAX`) is the absent
  parent or thread, and marks an ambiguous id in stratify's map.
- Only the contract's options exist (default accessors, the default separation, a constant
  padding, the three tilers, no `pack.radius`).
- One post-order pass computes each node's aggregate and its height (one more than its tallest
  child), and sorts its children, where d3 runs `computeHeight` from every node and sorts in a
  later pre-order pass. The values are the same: a node's sort reads only its children's final
  aggregates.
- The treemap's padding is `p / 2` below the root, which is what d3's `paddingStack` holds with a
  constant padding.
- Pack copies a node's children into `Circle` values, packs them and writes the positions back;
  enclose takes copies of the front chain. The circles do not move while either runs.
- Every failure throws the one message `invalid hierarchy`; d3's messages are recorded by the
  checker, not compared.

## Module state

None.

## Performance accommodations

None.
