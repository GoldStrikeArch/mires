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
type scale = {low: float, high: float, dim: float, requested: float, vertical: bool}
let elt = Array.getUnsafe
let scale = (range, dim, vertical) => {low: elt(range, 0), high: elt(range, 1), dim: round(dim *. 2.) /. 2., requested: dim, vertical}
let pixel = (s, v) => {
  let pct = (v -. s.low) /. (s.high -. s.low)
  s.dim *. (s.vertical ? 1. -. pct : pct)
}
let valueAt = (s, p) => {
  let q = s.vertical ? s.requested -. p : p
  s.low +. (s.high -. s.low) *. (q /. s.requested)
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
let precise = v => {
  let x = round(v *. 1000000.)
  x == 0. ? 0. : x
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
  save(ctx)
  try {
    let width = round(pixelRatio *. 1000.) /. 1000.
    let offset = Float.mod(width, 2.) /. 2.
    lineWidth(ctx, width)
    lineJoin(ctx, "round")
    lineCap(ctx, "butt")
    translate(ctx, offset, offset)
    let bounds = newPath()
    rect(bounds, 0. -. width /. 2., 0. -. width /. 2., x.dim +. width, y.dim +. width)
    clip(ctx, bounds)
    if Array.length(gaps) > 0 {
      let mask = newPath()
      let last = ref(0.)
      Array.forEach(gaps, g => {
        let a = elt(g, 0)
        let b = elt(g, 1)
        if b > a {
          if a > last.contents {
            rect(mask, last.contents, 0., a -. last.contents, y.dim)
          }
          last := b
        }
      })
      if x.dim > last.contents {
        rect(mask, last.contents, -5., x.dim -. last.contents, y.dim +. 10.)
      }
      clip(ctx, mask)
    }
    stroke(ctx, path)
  } catch {
  | e =>
    restore(ctx)
    throw(e)
  }
  restore(ctx)
  [cast(points), span ? cast(Nullable.null) : cast(gaps)]
}
let drawFrame = (canvas, ctx, f, span) => {
  let w = elt(f.size, 0)
  let h = elt(f.size, 1)
  width(canvas, round(w))
  height(canvas, round(h))
  setColor(ctx, "#172839")
  let x = scale(f.xRange, w, false)
  let y = scale(f.yRange, h, true)
  let xs = elt(f.data, 0)
  let paths = Array.fromInitializer(~length=Array.length(f.data) - 1, i =>
    render(ctx, x, y, xs, elt(f.data, i + 1), span)
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
    input.frames->Array.map(f => drawFrame(canvas, ctx, f, input.spanGaps))
  } catch {
  | e =>
    remove(canvas)
    throw(e)
  }
  remove(canvas)
  result
}
