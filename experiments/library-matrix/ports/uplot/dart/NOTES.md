# uPlot 1.6.32: Dart port

A hand-written, faithful port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), compiled by dart2js
(`dart compile js -O2`). It follows uPlot function by function: `roundDec`, `fixFloat`,
`incrRound`, `closestIdx`, `calcSize`, `getHPos`/`getVPos`, `posToVal`, the index window of
`setScales`, `paths.linear` with `_drawAcc`, `findGaps` and `clipGaps`, and `drawPath` with
`strokeFill`. It replaces a port that a Python generator had emitted from a shared core and later
scripts had patched by string replacement.

| File | What it is |
| --- | --- |
| `src/uplot.dart` | uPlot's slice: the rounding helpers, `Scale`, `Plot` (the plot box), `Chart` (conversions and index lookups), `linear`, `drawPath` |
| `src/dom.dart` | `dart:js_interop` extension types for the document, the canvas, its 2D context and `Path2D`, with `devicePixelRatio` and JS `Math.round` |
| `src/main.dart` | the boundary, doing what the adapter does per frame; `main()` publishes `run` for the comparator facade |

## Boundary

- **Input.** Extension types (`Input`, `Frame`) view the JS input. Each frame's arrays become Dart
  lists: `List<double>` for the size, ranges, x values and probes, and `List<double?>` for a series,
  where JS null is Dart null.
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  the chart is set up from it, the series are drawn, and the probes run through `valToPos`,
  `posToVal`, `valToIdx` and `posToIdx` with the adapter's `precise` (`Math.round(x * 1e6) || 0`,
  so −0 and NaN become 0).
- **Output.** Vertices and gaps are `(double, double)` records, converted with `toJS` into JS
  arrays with the traces and probes; the canvas width and height are read back from the canvas.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`; `finally` removes it, as the adapter's `finally` removes its host.

## Representation and numeric semantics

- **Math.round.** `jsRound` is the host's `Math.round` (halves toward +∞); Dart's `round()`
  rounds them away from zero.
- **The remainder.** `width % 2` is `remainder(2)`: Dart's `%` is the Euclidean modulo.
- **fixFloat.** On the web a non-integral `double`'s `toString()` is the JS string. `_regex6` is a
  `RegExp` with uPlot's pattern, which dart2js compiles to a host `RegExp`; `firstMatch` gives the
  first match, the one `fixFloat` reads from `String.prototype.match`. `Number.isInteger` is
  `isFinite && floorToDouble() == v`, and `Number.EPSILON` is a constant.
- **The plot offset.** `valToPosX`/`valToPosY` keep uPlot's `off + dim * pct`; the offset is
  `incrRound(0 * pxRatio, 0.5)`, +0, so an x value of −0 gives pixel 0, as in uPlot.
- **closestIdx.** JS `lo || 0` and `hi || arr.length - 1` are default 0 parameters and "a zero `hi`
  is the last index". On an empty array uPlot's final comparison reads `undefined` and fails, so it
  returns `hi`, −1; the port tests `isNotEmpty` before reading.
- **Accumulator.** uPlot's decimation keeps `inY`, `outY`, `minY` and `maxY` and tests
  `minY != null`; the port keeps them in an `_Accumulator?`, which is null exactly when uPlot's
  `minY` is.

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

`_regex6`, as uPlot's own is module-level. `firstMatch` keeps no state between calls.

## Performance accommodations

None.
