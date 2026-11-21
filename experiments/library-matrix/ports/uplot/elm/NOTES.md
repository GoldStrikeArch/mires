# uPlot 1.6.32: Elm port

A hand-written Elm port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), for Elm 0.19.2 with a JS
host. Elm computes everything uPlot computes: `roundDec`, `fixFloat` and `incrRound` (calcSize's
plot box), `getHPos` and `getVPos`, `posToVal`, `closestIdx` with the index window of `setScales`,
`paths.linear` with its pixel-column decimation and gaps, `clipGaps`, and drawPath's settings.

| File | What it is |
| --- | --- |
| `src/Main.elm` | uPlot's slice and the boundary (what the adapter does per frame), with `Json.Decode` |
| `js/index.js` | the host: sends the input and `devicePixelRatio` to the `request` port, then applies the `response` to a real canvas |

## Boundary

- **Request.** The host sends `{ input, pxRatio: devicePixelRatio }`. Elm decodes each frame into a
  record; a series is a `List (Maybe Float)`.
- **Response.** Per frame: the canvas size, `Math.round` of the CSS size times the ratio; the
  frame's drawPath settings (`null` for a frame without data, which `drawSeries` does not draw):
  the translate (`null` when the half-pixel offset is 0), `strokeStyle`, `lineWidth`, `lineJoin`,
  `lineCap`, the bounds rectangle and whether to stroke (width > 0); and per series the vertices,
  the gaps and clipGaps' rectangles (`null` without gaps), then the probes with the adapter's
  `precise` (`Math.round(x * 1e6) || 0`, so −0 and NaN become 0).
- **Host (review b10).** The host only applies the response: it sets the canvas size, builds the
  stroke and clip paths from Elm's numbers, makes uPlot's canvas calls in uPlot's order (translate,
  style, `save`, the clips, `stroke`, `restore`, translate back) and returns the geometry and
  probes with the canvas size read back. The stroke width, offset, bounds, clip rectangles and
  canvas size, which the host used to compute, come from Elm.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`; `finally` removes it, as the adapter's `finally` removes its host.

## Numeric semantics

- **JS numbers.** Elm's `round` and `floor` are `Math.round` and `Math.floor` and `^` is
  `Math.pow`; `Int` is a JS number, so pixel positions past int32 stay exact. `Number.isInteger`
  is `not NaN, not infinite, and toFloat (floor v) == v`, and `epsilon` is `Number.EPSILON`.
- **fixFloat.** `String.fromFloat` is the JS string. Core Elm has no regex, so
  `decimalsBeforeRun` scans the digits after the point for uPlot's `/\.\d*?(?=9{6,}|0{6,})/`: the
  first position where six 9s or six 0s begin.
- **The remainder.** Elm has no float remainder: `width % 2` is `width - 2 * truncate (width / 2)`,
  which is exact for the widths `roundDec(devicePixelRatio, 3)` gives (below 2^31).
- **The plot offset.** `pixel` keeps uPlot's `off + dim * pct`; the offset is
  `incrRound 0.5 (0 * ratio)`, +0, so an x value of −0 gives pixel 0, as in uPlot (review b10).
  `valueAt` divides by the ratio and the CSS size, as `posToVal` does; the CSS offset it would
  subtract is 0.
- **closestIdx.** `nearest` returns −1 for an empty array, as uPlot's does (its final comparison
  reads `undefined`), and treats a zero `hi` as the last index (`hi || arr.length - 1`).

## The slice

The chart's settings are fixed (CONTRACT.md): the x scale has orientation 0 and the y scale
orientation 1, both with direction 1; the series has `pxAlign` 1, `alignGaps` 0, width 1 and the
adapter's stroke, with no fill, bands or points; there are no axes, padding, cursor or legend.

- **Decimation.** A fold over the trimmed `(x, Maybe y)` pairs. A `Bucket` holds a pixel column's
  first, last, lowest and highest values, which is uPlot's `inY`, `outY`, `minY` and `maxY` while
  `minY != null`. `min` and `max` replace uPlot's `if (outY < minY) … else if (outY > maxY)`; they
  can pick a different one of two equal values (0 and −0), whose pixels are the same.
- **Gaps.** A fold that records each run of nulls from the pixel of the previous value to the pixel
  of the next replaces `hasGap` and `findGaps`, which with direction 1 and `alignGaps` 0 give the
  same intervals.
- **devicePixelRatio** is read once per call (uPlot reads it when it loads and on a `dppxchange`).
- **Drawing.** uPlot draws at construction and after each frame's batch; the port draws each frame
  once, which leaves the same pixels. `setCtxStyle`'s `fillStyle` and `setLineDash([])` are not
  set: nothing is filled and `[]` is the default dash.

## Deviations from upstream

- **fixedDec.** `fixFloat` first returns a value of uPlot's `fixedDec` map unchanged. That lookup
  is left out: the review checked that the string path returns each of the map's 358 members
  unchanged too.
- **Exponent strings.** `fixFloat`'s branch for strings containing `e-` (values below 1e-6) is
  left out. Plot sizes of one pixel or more never reach it (CONTRACT.md).

## Module state

The host keeps the initialized Elm worker and the latest response between calls (`app`,
`response`). The worker's model is `()`, so nothing computed for one call reaches the next.

## Performance accommodations

None.
