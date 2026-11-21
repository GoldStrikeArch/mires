# uPlot 1.6.32

The workload constructs an aligned numeric multi-series line chart, replaces its data, resizes its canvas, applies fixed linear scales and destroys the chart. Each frame specifies a physical canvas size, increasing x and y scale bounds, nondecreasing x values, one or more aligned y arrays with nullable values, and coordinate probes. The series count stays constant within a call. The port draws the series on a real Canvas 2D context through Path2D, including pixel-column extrema preservation on dense inputs and gap clipping when spanGaps is false.

Each frame reports the canvas width and height, then each series' exact linear stroke vertices and gap intervals, then the coordinate probes. A probe reports x and y value-to-pixel and pixel-to-value conversions in millionths, the nearest x index to its value, and the nearest visible x index to its pixel. The npm oracle records the vertices from the published linear path builder. Inputs include the precise decimation threshold, repeated x values, leading, trailing and internal nulls, resized and replaced data, and multiple series. The benchmark draws 4,096 samples per series on a 256-pixel-wide canvas. Axes, labels, plugins, alternate scales and alternate path styles are outside this workload.

Empty aligned series have empty strokes and both nearest-index probes return `-1`. Replacing a populated frame with empty data clears its canvas, vertices and gaps. The adapter gives uPlot CSS dimensions, the physical size divided by `devicePixelRatio`, and uPlot multiplies them back. The bitmap is `Math.round` of that product. The plot bounds are `incrRound(product, 0.5)`, which applies uPlot's `fixFloat` and `roundDec` before it rounds to half pixels. `fixFloat` first snaps a decimal string with a run of six or more 9s or 0s, so a physical width of 32.24999999 gives a 32.5-pixel plot. Value-to-pixel conversions add the plot offset, which is 0, so an x value of −0 maps to pixel 0. Pixel-to-value probes divide the canvas position by `devicePixelRatio` and then by the CSS dimension. Benchmark calls translate the data, scale bounds and probe values together to preserve their pixel distribution.

The domain:

- **Sizes.** Physical sizes are finite and at least one pixel. `fixFloat`'s branch for exponent strings handles values below 1e-6, which no such size reaches; the ports leave it out.
- **Values.** Scale bounds are finite and increasing. Data values and probes are finite JSON numbers, including −0 (the corpus generator writes the literal, which `JSON.parse` keeps) and coordinates past int32 and past 2^53.
- **devicePixelRatio.** Any ratio the page reports, read once per call. The ports repeat the adapter's division and uPlot's multiplication, so parity does not depend on the ratio. `check.mjs` runs headless at ratio 1. The `dpr-canvas-roundtrip` and `dpr-probe-roundtrip` cases change the bitmap or a probe only at ratios such as 1.1, 1.75 and 2.625, where `size / ratio * ratio` misses the size. The 2026-11-20 review also ran the corpus for all nine ports at nine emulated ratios from 0.75 to 3 in Chrome (device-metrics emulation) and seven from 1.1 to 3 in Firefox (`layout.css.devPixelsPerPx`).

The ports implement the part of uPlot this chart reaches, with the chart's settings fixed:

- the x scale has orientation 0 and the y scale orientation 1, both with direction 1;
- the series has `pxAlign` 1, `alignGaps` 0 and width 1, with no fill, bands or points;
- no axes, cursor, legend or padding, so the plot box starts at the canvas origin.

Within that slice, the Rust, ReScript, OCaml and Elm ports scan each trimmed series for gaps in one inline pass instead of uPlot's `hasGap` flag and `findGaps`. With direction 1 and `alignGaps` 0, `findGaps` spans each run of nulls from the pixel of the value before it to the pixel of the value after it, and the inline scan computes the same intervals. Each port appends one canvas to `document.body` for the call, in place of uPlot's elements inside the adapter's host `div`; both leave the page in `finally`.

`expected.json` records the pinned package's outputs for the corpus. Run `node verify.mjs chrome rust` (or another browser and built language) in this directory to compare a port's painted pixels and results with the reference, check its painted-stroke witnesses and run 100 generated charts. `node verify.mjs chrome reference --capture` regenerates the fixture.
