# uPlot 1.6.32: Rust port (mires)

A hand-written port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), as `no_std` Rust compiled
by mires. It covers `roundDec`, `fixFloat` and `incrRound` (calcSize's plot box), `getHPos` and
`getVPos`, `posToVal`, `closestIdx` with the index window of `setScales`, `paths.linear` with its
pixel-column decimation and gaps, and `drawPath`.

| File | What it is |
| --- | --- |
| `src/lib.rs` | the host bindings, uPlot's slice and the boundary; `matrix_run` is the export |

## Boundary

- **Input.** Read in place through host bindings (`Reflect.get`, indexing, `length`); a JSON null
  y value is `is_null()`.
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  each `Scale` holds uPlot's plot offset and size (`incrRound(css * ratio, 0.5)`) with the CSS size,
  and the probes use the adapter's `precise` (`Math.round(x * 1e6) || 0`, so −0 and NaN become 0).
- **Output.** JS arrays built through the host; the canvas width and height are read back.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`. The adapter removes its host in `finally`; here `try_host` catches
  any throw, including a panic (the panic handler throws), the canvas is removed, and the exception
  is thrown again (review b10).

## Numeric semantics

- **Host numbers.** `Math.round` (halves toward +∞), `Math.pow`, `Number.isInteger` and `String()`
  are host bindings, so they are JS's by construction. `f64::EPSILON` is `Number.EPSILON`.
- **fixFloat.** The JS string comes from `String()`. With no regex crate, `decimals_before_run`
  scans the digits after the point for uPlot's `/\.\d*?(?=9{6,}|0{6,})/`: the first position where
  six 9s or six 0s begin.
- **The plot offset.** `Scale::pixel` keeps uPlot's `off + dim * pct`; the offset is
  `incrRound(0 * ratio, 0.5)`, +0, so an x value of −0 gives pixel 0, as in uPlot (review b10).
  `Scale::value` divides by the ratio and the CSS size, as `posToVal` does; the CSS offset it
  would subtract is 0.
- **closestIdx.** `nearest` returns −1 for an empty array, as uPlot's does (its final comparison
  reads `undefined`), and treats a zero `hi` as the last index (`hi || arr.length - 1`).

## The slice

The chart's settings are fixed (CONTRACT.md): the x scale has orientation 0 and the y scale
orientation 1, both with direction 1; the series has `pxAlign` 1, `alignGaps` 0, width 1 and the
adapter's stroke, with no fill, bands or points; there are no axes, padding, cursor or legend.

- **Decimation.** A `Bucket` holds a pixel column's first, last, lowest and highest values, which
  is uPlot's `inY`, `outY`, `minY` and `maxY` while `minY != null`; `Option<Bucket>` is `None`
  exactly when `minY` is null. The column advances when an x value reaches `limit`, uPlot's
  `nextAccXVal`, and its pixel differs.
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
- **Exponent strings.** `fixFloat`'s branch for strings containing `e-` (values below 1e-6) is
  left out. Plot sizes of one pixel or more never reach it (CONTRACT.md).

## Module state

None.

## Performance accommodations

None.
