package main

import (
	"github.com/gopherjs/gopherjs/js"
	"math"
)

type V = *js.Object
type Scale struct {
	low, high, dim, requested float64
	vert                      bool
}

func scale(a, b, d float64, v bool) Scale { return Scale{a, b, rounded(d*2) / 2, d, v} }
func pixel(s Scale, v float64) float64 {
	pct := (v - s.low) / (s.high - s.low)
	if s.vert {
		pct = 1 - pct
	}
	return s.dim * pct
}
func valueAt(s Scale, v float64) float64 {
	q := v
	if s.vert {
		q = s.requested - v
	}
	return s.low + (s.high-s.low)*(q/s.requested)
}
func rounded(v float64) float64 {
	if v >= -0.5 && v <= 0 {
		return math.Copysign(0, v)
	}
	return math.Floor(v + 0.5)
}
func half(n int) int              { return n / 2 }
func real(i int) float64          { return float64(i) }
func num(v float64) V             { return js.Global.Get("Number").Invoke(v) }
func field(o V, key string) V     { return o.Get(key) }
func at(a V, i int) V             { return a.Index(i) }
func numberAt(a V, i int) float64 { return a.Index(i).Float() }
func size(a V) int                { return a.Length() }
func empty(v V) bool              { return v == nil }
func array() V                    { return js.Global.Get("Array").New() }
func pair(a, b V) V               { r := array(); r.Call("push", a, b); return r }
func gapResult(span bool, v V) V {
	if span {
		return nil
	}
	return v
}
func appendValue(a, v V)     { a.Call("push", v) }
func newPath() V             { return js.Global.Get("Path2D").New() }
func line(p V, x, y float64) { p.Call("lineTo", x, y) }
func resize(c, ctx V, w, h float64) {
	c.Set("width", w)
	c.Set("height", h)
	ctx.Set("strokeStyle", "#172839")
}
func draw(ctx, path, gaps V, w, h float64) {
	ctx.Call("save")
	defer ctx.Call("restore")
	width := rounded(js.Global.Get("devicePixelRatio").Float()*1000) / 1000
	offset := math.Mod(width, 2) / 2
	ctx.Set("lineWidth", width)
	ctx.Set("lineJoin", "round")
	ctx.Set("lineCap", "butt")
	ctx.Call("translate", offset, offset)
	bounds := newPath()
	bounds.Call("rect", -width/2, -width/2, w+width, h+width)
	ctx.Call("clip", bounds)
	if size(gaps) > 0 {
		mask := newPath()
		end := 0.0
		for i := 0; i < size(gaps); i++ {
			gap := at(gaps, i)
			a, b := numberAt(gap, 0), numberAt(gap, 1)
			if b > a {
				if a > end {
					mask.Call("rect", end, 0, a-end, h)
				}
				end = b
			}
		}
		if w > end {
			mask.Call("rect", end, -5, w-end, h+10)
		}
		ctx.Call("clip", mask)
	}
	ctx.Call("stroke", path)
}

func nearest(xs V, value float64, lower int, upper int) int {
	if size(xs) == 0 {
		return -1
	}
	lo := lower
	hi := upper
	if hi == 0 {
		hi = size(xs) - 1
	}
	for hi > lo+1 {
		mid := half(lo + hi)
		if numberAt(xs, mid) < value {
			lo = mid
		} else {
			hi = mid
		}
	}
	if value-numberAt(xs, lo) <= numberAt(xs, hi)-value {
		return lo
	}
	return hi
}
func point(stroke V, points V, x float64, y float64) {
	line(stroke, x, y)
	appendValue(points, pair(num(x), num(y)))
}
func finish(stroke V, points V, col float64, y Scale, low float64, high float64, first float64, last float64) {
	minY := rounded(pixel(y, low))
	maxY := rounded(pixel(y, high))
	firstY := rounded(pixel(y, first))
	lastY := rounded(pixel(y, last))
	if minY != maxY {
		if firstY != minY && lastY != minY {
			point(stroke, points, col, minY)
		}
		if firstY != maxY && lastY != maxY {
			point(stroke, points, col, maxY)
		}
		point(stroke, points, col, lastY)
	}
}
func render(ctx V, x Scale, y Scale, xs V, ys V, span bool) V {
	stroke := newPath()
	points := array()
	gaps := array()
	n := size(ys)
	first := 0
	for first < n && empty(at(ys, first)) {
		first += 1
	}
	if first < n {
		last := n - 1
		for last > first && empty(at(ys, last)) {
			last -= 1
		}
		dense := real(last-first) >= x.dim*4.0
		col := rounded(pixel(x, numberAt(xs, first)))
		limit := valueAt(x, col+1.0)
		has := false
		startY := 0.0
		endY := 0.0
		low := 0.0
		high := 0.0
		i := first
		for i <= last {
			xv := numberAt(xs, i)
			if dense && xv >= limit {
				next := rounded(pixel(x, xv))
				if next != col {
					if has {
						finish(stroke, points, col, y, low, high, startY, endY)
					}
					has = false
					col = next
					limit = valueAt(x, col+1.0)
				}
			}
			if !empty(at(ys, i)) {
				v := numberAt(ys, i)
				if !dense {
					point(stroke, points, rounded(pixel(x, xv)), rounded(pixel(y, v)))
				} else if !has {
					point(stroke, points, col, rounded(pixel(y, v)))
					has = true
					startY = v
					endY = v
					low = v
					high = v
				} else {
					endY = v
					if v < low {
						low = v
					} else if v > high {
						high = v
					}
				}
			}
			i += 1
		}
		if has {
			finish(stroke, points, col, y, low, high, startY, endY)
		}
		if !span {
			i = first
			for i <= last {
				if empty(at(ys, i)) {
					left := rounded(pixel(x, numberAt(xs, i-1)))
					for i < last && empty(at(ys, i)) {
						i += 1
					}
					right := rounded(pixel(x, numberAt(xs, i)))
					if right >= left {
						appendValue(gaps, pair(num(left), num(right)))
					}
				}
				i += 1
			}
		}
	}
	draw(ctx, stroke, gaps, x.dim, y.dim)
	return pair(points, gapResult(span, gaps))
}
func precise(v float64) V {
	n := rounded(v * 1000000.0)
	if n == 0.0 {
		return num(0.0)
	}
	return num(n)
}
func frame(canvas V, ctx V, f V, span bool) V {
	dims := field(f, "size")
	w := numberAt(dims, 0)
	h := numberAt(dims, 1)
	resize(canvas, ctx, rounded(w), rounded(h))
	xr := field(f, "xRange")
	yr := field(f, "yRange")
	x := scale(numberAt(xr, 0), numberAt(xr, 1), w, false)
	y := scale(numberAt(yr, 0), numberAt(yr, 1), h, true)
	data := field(f, "data")
	xs := at(data, 0)
	paths := array()
	i := 1
	for i < size(data) {
		appendValue(paths, render(ctx, x, y, xs, at(data, i), span))
		i += 1
	}
	lo := nearest(xs, x.low, 0, size(xs)-1)
	hi := nearest(xs, x.high, 0, size(xs)-1)
	if hi > lo+1 {
		if numberAt(xs, lo) < x.low {
			lo += 1
		}
		if numberAt(xs, hi) > x.high {
			hi -= 1
		}
	}
	probes := field(f, "probes")
	answers := array()
	i = 0
	for i < size(probes) {
		q := at(probes, i)
		xv := numberAt(q, 0)
		px := numberAt(q, 2)
		values := pair(precise(pixel(x, xv)), precise(pixel(y, numberAt(q, 1))))
		appendValue(values, precise(valueAt(x, px)))
		appendValue(values, precise(valueAt(y, numberAt(q, 3))))
		appendValue(values, num(real(nearest(xs, xv, 0, size(xs)-1))))
		appendValue(values, num(real(nearest(xs, valueAt(x, px), lo, hi))))
		appendValue(answers, values)
		i += 1
	}
	result := pair(field(canvas, "width"), field(canvas, "height"))
	appendValue(result, paths)
	appendValue(result, answers)
	return result
}
func run(input V) V {
	doc := js.Global.Get("document")
	c := doc.Call("createElement", "canvas")
	doc.Get("body").Call("appendChild", c)
	defer c.Call("remove")
	ctx := c.Call("getContext", "2d")
	out := array()
	frames := field(input, "frames")
	for i := 0; i < size(frames); i++ {
		appendValue(out, frame(c, ctx, at(frames, i), field(input, "spanGaps").Bool()))
	}
	return out
}
func main() { js.Module.Get("exports").Set("run", run) }
