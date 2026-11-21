# uPlot 1.6.32: Go port (GopherJS)

A hand-written, faithful port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), for Go 1.21 compiled by
GopherJS 1.21.0 (`gopherjs build -m`). It follows uPlot function by function: `roundDec`,
`fixFloat`, `incrRound`, `closestIdx`, `calcSize`, `getHPos`/`getVPos`, `posToVal`, the index
window of `setScales`, `paths.linear` with `_drawAcc`, `findGaps` and `clipGaps`, and `drawPath`
with `strokeFill`. It replaces a port that a Python generator had emitted from a shared core and
later scripts had patched by string replacement.

| File | What it is |
| --- | --- |
| `src/uplot.go` | uPlot's slice: `round` (JS `Math.round`), the rounding helpers, `scale`, `plot` (the plot box), `chart` (conversions, index lookups, `linear`), `drawPath` |
| `src/main.go` | the boundary, doing what the adapter does per frame; publishes `run` on `js.Module.exports` |

## Boundary

- **Input.** Each frame is read through `*js.Object` into Go values: `[2]float64` for the size and
  ranges, `[]float64` for the x values, `[][4]float64` for the probes, and `[]*float64` for a
  series, where a JSON null is a nil pointer (GopherJS gives a nil `*js.Object` for null), as
  `encoding/json` would decode it.
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  the chart is set up from it, the series are drawn, and the probes run through `valToPos`,
  `posToVal`, `valToIdx` and `posToIdx` with the adapter's `precise` (`Math.round(x * 1e6) || 0`,
  so −0 and NaN become 0).
- **Output.** `js.S` values, which the `js` package externalizes to JS arrays; the canvas width
  and height are read back from the canvas.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`; `defer` removes it, as the adapter's `finally` removes its host.

## Representation and numeric semantics

- **Math.round (review b10).** `round` is JS `Math.round` in Go: `r := math.Floor(x)`, plus one
  when `x - r >= 0.5`, and −0 for a negative value that rounds to zero. `x - floor(x)` is exact
  for every float64, so 0.49999999999999994 rounds to 0 and odd integers in [2^52, 2^53) stay put.
  The former `floor(x + 0.5)` got both wrong. `math.Round` rounds halves away from zero.
- **fixFloat.** The JS string comes from the host's `String()`, as upstream builds it with a
  template literal (`jsString`, as in the big.js port). Go's `regexp` has no lookahead, so
  `decimalsBeforeRun` scans the digits after the point for uPlot's `/\.\d*?(?=9{6,}|0{6,})/`:
  the first position where six 9s or six 0s begin. `Number.isInteger` is
  `!IsInf && Floor(v) == v`, and `Number.EPSILON` is `0x1p-52`.
- **JS `%`.** `math.Mod`.
- **The plot offset.** `valToPosX`/`valToPosY` keep uPlot's `off + dim * pct`; the offset is
  `incrRound(0 * pxRatio, 0.5)`, +0, so an x value of −0 gives pixel 0, as in uPlot.
- **closestIdx.** JS `lo || 0` and `hi || arr.length - 1`: callers pass 0 for an absent bound, and
  a zero `hi` is the last index. On an empty array uPlot's final comparison reads `undefined` and
  fails, so it returns `hi`, −1; the port tests the length before reading.
- **Accumulator.** uPlot's decimation keeps `inY`, `outY`, `minY` and `maxY` and tests
  `minY != null`; the port keeps them in an `*accumulator`, which is nil exactly when uPlot's
  `minY` is null.
- **Integers.** Pixel positions stay `float64`; only indices are `int`, which is 32-bit here.

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
- **Exponent strings.** `fixFloat`'s branch for strings containing `e-` (values below 1e-6) is
  left out. Plot sizes of one pixel or more never reach it (CONTRACT.md).
- **All-null series.** `nonNullIdxs` gives −1 and −1, and uPlot's loop then reads index −1, finds
  `undefined` and draws nothing. The port skips the loop.
- **drawnAtX.** uPlot never assigns it, so its final `drawnAtX != accX` test always passes and is
  left out.

## Module state

None: the package-level names are constants and functions.

## Performance accommodations

`pixelForX`, `pixelForY` and `xForPixel`, closures in uPlot, are methods of `chart`
(`xForPixel` is `posToValX`). GopherJS compiles every call through a function value as a
resumable state machine (README, "Per-language notes"), and these run once per sample.
