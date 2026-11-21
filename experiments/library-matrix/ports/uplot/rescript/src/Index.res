type canvas
type ctx
type path
type document
type element
@val external doc: document = "document"
@get external body: document => element = "body"
@send external createElement: (document, string) => canvas = "createElement"
@send external appendChild: (element, canvas) => unit = "appendChild"
@send external remove: canvas => unit = "remove"
@send external context: (canvas, string) => ctx = "getContext"
@set external width: (canvas, float) => unit = "width"
@set external height: (canvas, float) => unit = "height"
@get external getWidth: canvas => float = "width"
@get external getHeight: canvas => float = "height"
@set external setColor: (ctx, string) => unit = "strokeStyle"
@new external newPath: unit => path = "Path2D"
@send external line: (path, float, float) => unit = "lineTo"
@send external rect: (path, float, float, float, float) => unit = "rect"
@val external pixelRatio: float = "devicePixelRatio"
@set external lineWidth: (ctx, float) => unit = "lineWidth"
@set external lineJoin: (ctx, string) => unit = "lineJoin"
@set external lineCap: (ctx, string) => unit = "lineCap"
@send external translate: (ctx, float, float) => unit = "translate"
@send external save: ctx => unit = "save"
@send external restore: ctx => unit = "restore"
@send external clip: (ctx, path) => unit = "clip"
@send external stroke: (ctx, path) => unit = "stroke"
@val external round: float => float = "Math.round"
@val external pow: (float, float) => float = "Math.pow"
@val external isInteger: float => bool = "Number.isInteger"
type value
external cast: 'a => value = "%identity"
type frame = {
  size: array<float>,
  xRange: array<float>,
  yRange: array<float>,
  data: array<array<Nullable.t<float>>>,
  probes: array<array<float>>,
}
type input = {spanGaps: bool, frames: array<frame>}
// uPlot's roundDec: rounds half away from zero at `dec` decimals.
let roundDec = (v, ~dec=0) =>
  if isInteger(v) {
    v
  } else {
    let p = pow(10., Int.toFloat(dec))
    round(v *. p *. (1. +. Float.Constants.epsilon)) /. p
  }
// uPlot's fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the
// point to the decimals before the run. uPlot first returns members of its fixedDec map unchanged;
// the string path returns every member unchanged too. Its branch for strings with "e-" (values
// below 1e-6) is not reached by plot sizes of 1 px or more.
let regex6 = %re("/\.\d*?(?=9{6,}|0{6,})/")
let fixFloat = v =>
  if isInteger(v) {
    v
  } else {
    switch regex6->RegExp.exec(Float.toString(v)) {
    | Some(m) => roundDec(v, ~dec=String.length(RegExp.Result.fullMatch(m)) - 1)
    | None => v
    }
  }
let incrRound = (v, incr) => fixFloat(roundDec(fixFloat(v /. incr)) *. incr)
// One axis of the plot: the scale range, calcSize's offset and size in canvas pixels, and the CSS
// size uPlot was given (the adapter divides the physical size by pxRatio).
type scale = {
  low: float,
  high: float,
  off: float,
  dim: float,
  css: float,
  ratio: float,
  vertical: bool,
}
let elt = Array.getUnsafe
let scale = (range, css, ratio, vertical) => {
  low: elt(range, 0),
  high: elt(range, 1),
  off: incrRound(0. *. ratio, 0.5),
  dim: incrRound(css *. ratio, 0.5),
  css,
  ratio,
  vertical,
}
// getHPos / getVPos (dir 1) in canvas pixels, keeping uPlot's `off +`, which turns -0 into 0.
let pixel = (s, v) => {
  let pct = (v -. s.low) /. (s.high -. s.low)
  s.off +. s.dim *. (s.vertical ? 1. -. pct : pct)
}
// posToVal(p, key, true): the CSS position (the plot's CSS offset is 0), from the bottom for y.
let valueAt = (s, p) => {
  let pos = p /. s.ratio
  let q = s.vertical ? s.css -. pos : pos
  s.low +. (s.high -. s.low) *. (q /. s.css)
}
let number = (xs, i) => elt(xs, i)->Nullable.getOr(0.)
let nearest = (xs, v, lo, hi) => {
  let rec find = (a, b) =>
    if b <= a + 1 {
      v -. number(xs, a) <= number(xs, b) -. v ? a : b
    } else {
      let mid = (a + b) / 2
      number(xs, mid) < v ? find(mid, b) : find(a, mid)
    }
  Array.length(xs) == 0 ? -1 : find(lo, hi == 0 ? Array.length(xs) - 1 : hi)
}
// The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0.
let precise = v => {
  let x = round(v *. 1000000.)
  x == 0. || Float.isNaN(x) ? 0. : x
}
type bucket = {first: float, mutable last: float, mutable low: float, mutable high: float}
let render = (ctx, x, y, xs, ys, span) => {
  let path = newPath()
  let points = []
  let gaps = []
  let point = (px, py) => {
    line(path, px, py)
    Array.push(points, [px, py])
  }
  let roundY = v => round(pixel(y, v))
  let finish = (col, b) => {
    let lo = roundY(b.low)
    let hi = roundY(b.high)
    let first = roundY(b.first)
    let last = roundY(b.last)
    if lo != hi {
      if first != lo && last != lo {
        point(col, lo)
      }
      if first != hi && last != hi {
        point(col, hi)
      }
      point(col, last)
    }
  }
  let first = ref(0)
  let n = Array.length(ys)
  while first.contents < n && elt(ys, first.contents)->Nullable.isNullable {
    first := first.contents + 1
  }
  if first.contents < n {
    let last = ref(n - 1)
    while last.contents > first.contents && elt(ys, last.contents)->Nullable.isNullable {
      last := last.contents - 1
    }
    let dense = Int.toFloat(last.contents - first.contents) >= x.dim *. 4.
    let col = ref(round(pixel(x, number(xs, first.contents))))
    let limit = ref(valueAt(x, col.contents +. 1.))
    let bucket = ref(None)
    for i in first.contents to last.contents {
      let xv = number(xs, i)
      if dense && xv >= limit.contents {
        let next = round(pixel(x, xv))
        if next != col.contents {
          bucket.contents->Option.forEach(finish(col.contents, _))
          bucket := None
          col := next
          limit := valueAt(x, next +. 1.)
        }
      }
      switch elt(ys, i)->Nullable.toOption {
      | None => ()
      | Some(v) =>
        if !dense {
          point(round(pixel(x, xv)), roundY(v))
        } else {
          switch bucket.contents {
          | Some(b) => {
              b.last = v
              if v < b.low {
                b.low = v
              } else if v > b.high {
                b.high = v
              }
            }
          | None => {
              point(col.contents, roundY(v))
              bucket := Some({first: v, last: v, low: v, high: v})
            }
          }
        }
      }
    }
    bucket.contents->Option.forEach(finish(col.contents, _))
    if !span {
      let i = ref(first.contents)
      while i.contents <= last.contents {
        if elt(ys, i.contents)->Nullable.isNullable {
          let a = round(pixel(x, number(xs, i.contents - 1)))
          while i.contents < last.contents && elt(ys, i.contents)->Nullable.isNullable {
            i := i.contents + 1
          }
          let b = round(pixel(x, number(xs, i.contents)))
          if b >= a {
            Array.push(gaps, [a, b])
          }
        }
        i := i.contents + 1
      }
    }
  }
  // drawPath: the series width 1 in canvas pixels, the half-pixel translate, the bounds clip and
  // clipGaps' mask.
  let width = roundDec(x.ratio, ~dec=3)
  let offset = Float.mod(width, 2.) /. 2.
  if offset > 0. {
    translate(ctx, offset, offset)
  }
  let bounds = newPath()
  rect(bounds, x.off -. width /. 2., y.off -. width /. 2., x.dim +. width, y.dim +. width)
  setColor(ctx, "#172839")
  lineWidth(ctx, width)
  lineJoin(ctx, "round")
  lineCap(ctx, "butt")
  save(ctx)
  clip(ctx, bounds)
  if Array.length(gaps) > 0 {
    let mask = newPath()
    let last = ref(x.off)
    Array.forEach(gaps, g => {
      let a = elt(g, 0)
      let b = elt(g, 1)
      if b > a {
        if a -. last.contents > 0. {
          rect(mask, last.contents, y.off, a -. last.contents, y.off +. y.dim)
        }
        last := b
      }
    })
    let rest = x.off +. x.dim -. last.contents
    if rest > 0. {
      rect(mask, last.contents, y.off -. 5., rest, y.off +. y.dim +. 10.)
    }
    clip(ctx, mask)
  }
  if width > 0. {
    stroke(ctx, path)
  }
  restore(ctx)
  if offset > 0. {
    translate(ctx, -.offset, -.offset)
  }
  [cast(points), span ? cast(Nullable.null) : cast(gaps)]
}
let drawFrame = (canvas, ctx, f, span, ratio) => {
  // The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
  let w = elt(f.size, 0) /. ratio
  let h = elt(f.size, 1) /. ratio
  width(canvas, round(w *. ratio))
  height(canvas, round(h *. ratio))
  let x = scale(f.xRange, w, ratio, false)
  let y = scale(f.yRange, h, ratio, true)
  let xs = elt(f.data, 0)
  // drawSeries runs only with data; the adapter then reports an empty trace for each series.
  let paths = Array.fromInitializer(~length=Array.length(f.data) - 1, i =>
    Array.length(xs) > 0
      ? render(ctx, x, y, xs, elt(f.data, i + 1), span)
      : [cast([]), span ? cast(Nullable.null) : cast([])]
  )
  let endIndex = Array.length(xs) - 1
  let lo = ref(nearest(xs, x.low, 0, endIndex))
  let hi = ref(nearest(xs, x.high, 0, endIndex))
  if hi.contents > lo.contents + 1 {
    if number(xs, lo.contents) < x.low {
      lo := lo.contents + 1
    }
    if number(xs, hi.contents) > x.high {
      hi := hi.contents - 1
    }
  }
  let probes = f.probes->Array.map(q => {
    let xv = elt(q, 0)
    let px = elt(q, 2)
    [
      precise(pixel(x, xv)),
      precise(pixel(y, elt(q, 1))),
      precise(valueAt(x, px)),
      precise(valueAt(y, elt(q, 3))),
      Int.toFloat(nearest(xs, xv, 0, endIndex)),
      Int.toFloat(nearest(xs, valueAt(x, px), lo.contents, hi.contents)),
    ]
  })
  [cast(getWidth(canvas)), cast(getHeight(canvas)), cast(paths), cast(probes)]
}
let run = (input: input) => {
  let canvas = createElement(doc, "canvas")
  appendChild(body(doc), canvas)
  let result = try {
    let ctx = context(canvas, "2d")
    let ratio = pixelRatio
    input.frames->Array.map(f => drawFrame(canvas, ctx, f, input.spanGaps, ratio))
  } catch {
  | e =>
    remove(canvas)
    throw(e)
  }
  remove(canvas)
  result
}
