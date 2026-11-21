# uPlot 1.6.32: Melange port

A hand-written port of the slice of `node_modules/uplot/dist/uPlot.esm.js` that
`libraries/uplot/reference.mjs` runs (`libraries/uplot/CONTRACT.md`), in OCaml compiled by
Melange 7.0.1 (`melange.emit`, ES6, the emitted stdlib bundled). It covers `roundDec`, `fixFloat`
and `incrRound` (calcSize's plot box), `getHPos` and `getVPos`, `posToVal`, `closestIdx` with the
index window of `setScales`, `paths.linear` with its pixel-column decimation and gaps, and
`drawPath`.

| File | What it is |
| --- | --- |
| `src/chart.ml` | uPlot's slice as the functor `Chart.Make (Host)`, over the JS values a `Host` provides |
| `src/index.ml` | the Melange `Host`, with externals for the input, the result and the canvas; `run` is the module's export |

**Shared source.** `chart.ml` is byte-identical to `ports/uplot/jsoo/src/chart.ml`, as the README
allows; only the `Host` module and the entry differ.

## Boundary

- **Input.** Read in place: the JS values sit behind the abstract `Host.v`, read with
  `mel.get_index` externals and `%identity` casts, and a JSON null y value is `== null`.
- **Frame.** As the adapter does: the CSS size is the physical size divided by `devicePixelRatio`,
  each `scale` holds uPlot's plot offset and size (`incrRound(css * ratio, 0.5)`) with the CSS size,
  and the probes use the adapter's `precise` (`Math.round(x * 1e6) || 0`, so −0 and NaN become 0).
- **Output.** JS arrays built with `Array` and `push`; the canvas width and height are read back.
- **Canvas.** One canvas, appended to `document.body` for the call, stands in for uPlot's elements
  in the adapter's host `div`. `run` removes it on return and on an exception, which it raises
  again, as the adapter's `finally` does. (`Fun.protect` would need
  `caml_restore_raw_backtrace`, which Melange does not polyfill, and would replace the error.)

## Numeric semantics

- **Host numbers.** `Math.round` (halves toward +∞), `Number.isInteger` and `String()` come from
  the host; `**` is `Math.pow`, `mod_float` is JS `%`, and `epsilon` is `Number.EPSILON`.
- **fixFloat.** The JS string comes from the host. `decimals_before_run` scans the digits after
  the point for uPlot's `/\.\d*?(?=9{6,}|0{6,})/`: the first position where six 9s or six 0s
  begin. A scan in the shared source, rather than each compiler's own `RegExp` binding, keeps
  `chart.ml` identical for both.
- **The plot offset.** `pixel` keeps uPlot's `off + dim * pct`; the offset is
  `incr_round (0 *. ratio) 0.5`, +0, so an x value of −0 gives pixel 0, as in uPlot (review b10).
  `value` divides by the ratio and the CSS size, as `posToVal` does; the CSS offset it would
  subtract is 0.
- **closestIdx.** `nearest` returns −1 for an empty array, as uPlot's does (its final comparison
  reads `undefined`), and treats a zero `hi` as the last index (`hi || arr.length - 1`).
- **NaN.** `precise` tests `x <> x`, which is true only for NaN.

## The slice

The chart's settings are fixed (CONTRACT.md): the x scale has orientation 0 and the y scale
orientation 1, both with direction 1; the series has `pxAlign` 1, `alignGaps` 0, width 1 and the
adapter's stroke, with no fill, bands or points; there are no axes, padding, cursor or legend.

- **Decimation.** A `bucket` holds a pixel column's first, last, lowest and highest values, which
  is uPlot's `inY`, `outY`, `minY` and `maxY` while `minY != null`; the `bucket option` is `None`
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
