# uPlot 1.6.32: ReScript port

A hand-written port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), for ReScript 12.3.0. It
covers `roundDec`, `fixFloat` and `incrRound` (calcSize's plot box), `getHPos` and `getVPos`,
`posToVal`, `closestIdx` with the index window of `setScales`, `paths.linear` with its
pixel-column decimation and gaps, and `drawPath`.

| File | What it is |
| --- | --- |
| `src/Index.res` | the externals, uPlot's slice and the boundary; `run` is the module's export |

## Boundary

- **Input.** Record types (`input`, `frame`) are the JS input itself, with no conversion; a series
  is `array<Nullable.t<float>>`.
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  each `scale` holds uPlot's plot offset and size (`incrRound(css * ratio, 0.5)`) with the CSS size,
  and the probes use the adapter's `precise` (`Math.round(x * 1e6) || 0`, so −0 and NaN become 0).
- **Output.** JS arrays; the canvas width and height are read back from the canvas.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`. A `try`/`catch` that removes it and rethrows emulates the
  adapter's `finally`.

## Numeric semantics

- **Host numbers.** `Math.round` (halves toward +∞), `Math.floor`, `Math.ceil`, `Math.pow`,
  `Math.log10`, `Math.abs`, `Math.min`, `Math.max`, `Number.isInteger` and `Number` are externals;
  `Float.Constants.epsilon` is `Number.EPSILON`.
- **fixFloat.** `Float.toString` is JS `toString()`. `regex6` is uPlot's pattern as a `%re`
  literal; `RegExp.exec` gives the first match, the one `fixFloat` reads from
  `String.prototype.match`. Strings containing `e-` round the mantissa at that precision and
  reattach the exponent, as upstream does (reached by `rangeNum` on tiny flat x data).
- **Scale range guard (`< 1e-16`).** `scaleState` carries `x` and `y` bounds across frames within a
  call, mirroring `setData` → `autoScaleX` (including `rangeNum(v, v, 0.1, true)` for flat x data),
  explicit `setScale('x'/'y')`, and `_commit` → `setScales`. Unset scales default to `(0., 0.)`,
  matching JS arithmetic on `null`. `rangeAccepted` checks `Float.isNaN(d) || d >= 1e-16` because
  ReScript rewrites `!(a < b)` to `a >= b`.
- **The plot offset.** `pixel` keeps uPlot's `off + dim * pct`; the offset is
  `incrRound(0 * ratio, 0.5)`, +0, so an x value of −0 gives pixel 0, as in uPlot (review b10).
  `valueAt` divides by the ratio and the CSS size, as `posToVal` does; the CSS offset it would
  subtract is 0.
- **closestIdx.** `nearest` returns −1 for an empty array, as uPlot's does (its final comparison
  reads `undefined`), and treats a zero `hi` as the last index (`hi || arr.length - 1`).

## The slice

The chart's settings are fixed (CONTRACT.md): the x scale has orientation 0 and the y scale
orientation 1, both with direction 1; the series has `pxAlign` 1, `alignGaps` 0, width 1 and the
adapter's stroke, with no fill, bands or points; there are no axes, padding, cursor or legend.

- **Decimation.** A `bucket` holds a pixel column's first, last, lowest and highest values, which
  is uPlot's `inY`, `outY`, `minY` and `maxY` while `minY != null`; `option<bucket>` is `None`
  exactly when `minY` is null. The column advances when `Float.isNaN(limit.contents) || xv >= limit.contents`
  (explicitly testing `isNaN` because ReScript rewrites `!(xv < limit)` to `xv >= limit`, matching
  uPlot's `!(xVal < nextAccXVal)` on `NaN` limits from overflowing scales) and its pixel differs.
- **Gaps.** An inline scan of the trimmed series replaces `hasGap` and `findGaps`. With direction 1
  and `alignGaps` 0, `findGaps` spans each run of nulls from the pixel of the value before it to the
  pixel of the value after it, and so does the scan.
- **devicePixelRatio** is read once per call (uPlot reads it when it loads and on a `dppxchange`).
- **Drawing.** Each series is drawn right after its path is built, with uPlot's order of
  operations: the half-pixel translate when the offset is positive, the bounds and gap clips inside
  `save`/`restore`, the stroke when the width is positive, and the translate back. uPlot draws at
  construction and after each frame's batch; the port draws each frame once, which leaves the same
  pixels. As in `drawSeries`, nothing is drawn for a frame without data. `setCtxStyle`'s
  `fillStyle` and `setLineDash([])` are not set: nothing is filled and `[]` is the default dash.

## Deviations from upstream

- **fixedDec.** `fixFloat` first returns a value of uPlot's `fixedDec` map unchanged. That lookup
  is left out: the review checked that the string path returns each of the map's 358 members
  unchanged too.

## Module state

`regex6`, as uPlot's own is module-level; it is not global, so `exec` keeps no `lastIndex`.

## Performance accommodations

None.
