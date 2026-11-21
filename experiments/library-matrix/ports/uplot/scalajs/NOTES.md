# uPlot 1.6.32: Scala.js port

A hand-written, faithful port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), for Scala 3.8.4 and
Scala.js 1.22.0 (`fullLinkJS`, ES modules). It follows uPlot function by function: `roundDec`,
`fixFloat`, `incrRound`, `closestIdx`, `calcSize`, `getHPos`/`getVPos`, `posToVal`, the index
window of `setScales`, `paths.linear` with `_drawAcc`, `findGaps` and `clipGaps`, and `drawPath`
with `strokeFill`. It replaces a port that a Python generator had emitted from a shared core and
later scripts had patched by string replacement.

| File | What it is |
| --- | --- |
| `src/UPlot.scala` | uPlot's slice: the rounding helpers, `Scale`, `Plot` (the plot box), `Chart` (conversions and index lookups), `linear`, `drawPath` |
| `src/Dom.scala` | `@js.native` facades for the document, the canvas, its 2D context, `Path2D` and `devicePixelRatio` |
| `src/Main.scala` | the boundary, doing what the adapter does per frame; `run` is exported with `@JSExportTopLevel` |

## Boundary

- **Input.** `@js.native` facade traits (`Input`, `Frame`) view the JS input with no copy. A series
  is a `js.Array[Double | Null]`, matched with `case y: Double` and `case null`. The x array is the
  same array cast to `js.Array[Double]` (x values are never null).
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  the chart is set up from it, the series are drawn, and the probes run through `valToPos`,
  `posToVal`, `valToIdx` and `posToIdx` with the adapter's `precise` (`Math.round(x * 1e6) || 0`,
  so −0 and NaN become 0).
- **Output.** Vertices, gaps, traces and probes are built as `js.Array`s; the canvas width and
  height are read back from the canvas.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`; `finally` removes it, as the adapter's `finally` removes its host.

## Representation and numeric semantics

- **Math.round.** `js.Math.round` is the host's (halves toward +∞); `math.round` would go through
  `Long`.
- **fixFloat.** `Double.toString` is the JS string on Scala.js. `regex6` is a `js.RegExp` with
  uPlot's pattern rather than a `java.util.regex` translation. uPlot's is global and read with
  `String.prototype.match`, which returns every match; `fixFloat` uses the first, which a
  non-global `exec` returns. Strings containing `e-` round the mantissa at that precision and
  reattach the exponent, as upstream does. `Number.isInteger` is `isFinite && math.floor(v) == v`,
  and `Number.EPSILON` is a constant.
- **Scale range guard (`< 1e-16`).** `ScaleState` carries `Option[Scale]` bounds for `x` and `y`
  across frames within a call, mirroring `setData` → `autoScaleX` (including `rangeNum(v, v, 0.1,
  true)` for flat x data), explicit `setScale('x'/'y')`, and `_commit` → `setScales`. Unset scales
  default to `Scale(0.0, 0.0)`, matching JS arithmetic on `null`.
- **The plot offset.** `valToPosX`/`valToPosY` keep uPlot's `off + dim * pct`; the offset is
  `incrRound(0 * pxRatio, 0.5)`, +0, so an x value of −0 gives pixel 0, as in uPlot.
- **closestIdx.** JS `lo || 0` and `hi || arr.length - 1` are default 0 parameters and "a zero `hi`
  is the last index". On an empty array uPlot's final comparison reads `undefined` and fails, so it
  returns `hi`, −1; the port tests `nonEmpty` before reading.
- **Accumulator.** uPlot's decimation keeps `inY`, `outY`, `minY` and `maxY` and tests
  `minY != null`; the port keeps them in an `Option[Accumulator]`, which is `None` exactly when
  uPlot's `minY` is null. Gaps and their clip are `Option`s too, `None` with `spanGaps`.

## The slice

The chart's settings are fixed (CONTRACT.md): the x scale has orientation 0 and the y scale
orientation 1, both with direction 1; the series has `pxAlign` 1, `alignGaps` 0, width 1 and the
adapter's stroke, with no fill, bands or points; there are no axes, padding, cursor or legend.

- **findGaps** runs with direction 1 and `alignGaps` 0, so each run of nulls spans from the pixel of
  the value before it to the pixel of the value after it. Both exist, because the range is trimmed
  to values, so uPlot's fallback to the run's own end pixels is not ported.
- **devicePixelRatio** is read once per call (uPlot reads it when it loads and on a `dppxchange`).
- **Drawing.** uPlot draws when it is constructed and again after each frame's batch; the port
  draws each frame once, which leaves the same pixels. As in `drawSeries`, nothing is drawn for a
  frame without data, and each series reports the adapter's empty trace. `setCtxStyle`'s
  `fillStyle` and `setLineDash([])` are not set: nothing is filled and `[]` is the default dash.

## Deviations from upstream

- **fixedDec.** `fixFloat` first returns a value of uPlot's `fixedDec` map unchanged. That lookup
  is left out: the review checked that the string path returns each of the map's 358 members
  unchanged too.
- **All-null series.** `nonNullIdxs` gives −1 and −1, and uPlot's loop then reads index −1, finds
  `undefined` and draws nothing. The port skips the loop.
- **drawnAtX.** uPlot never assigns it, so its final `drawnAtX != accX` test always passes and is
  left out.

## Module state

`regex6`, as uPlot's own is module-level; it is not global, so `exec` keeps no `lastIndex`. The
other top-level values are constants.

## Performance accommodations

None.
