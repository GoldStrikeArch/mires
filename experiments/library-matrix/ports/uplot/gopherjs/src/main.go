// The boundary: what libraries/uplot/reference.mjs does, on the port's uPlot slice. Each frame is
// read through *js.Object into Go values (a JSON null y value is a nil *float64, as encoding/json
// decodes it), and the result is built as js.S, which the js package externalizes to JS arrays.
package main

import "github.com/gopherjs/gopherjs/js"

// jsString is the host's String(n), whose digits fixFloat inspects.
func jsString(n float64) string { return js.Global.Call("String", n).String() }

// frame is one input frame in Go values.
type frame struct {
	size, xRange, yRange [2]float64
	xs                   []float64
	series               [][]*float64
	probes               [][4]float64
}

func floats(v *js.Object) []float64 {
	out := make([]float64, v.Length())
	for i := range out {
		out[i] = v.Index(i).Float()
	}
	return out
}

func pair(v *js.Object) [2]float64 { return [2]float64{v.Index(0).Float(), v.Index(1).Float()} }

// optionalFloats reads an aligned y array; GopherJS gives a nil *js.Object for JSON null.
func optionalFloats(v *js.Object) []*float64 {
	out := make([]*float64, v.Length())
	for i := range out {
		if item := v.Index(i); item != nil {
			y := item.Float()
			out[i] = &y
		}
	}
	return out
}

func readFrame(f *js.Object) frame {
	data, probes := f.Get("data"), f.Get("probes")
	fr := frame{size: pair(f.Get("size")), xRange: pair(f.Get("xRange")), yRange: pair(f.Get("yRange")),
		xs: floats(data.Index(0)), probes: make([][4]float64, probes.Length())}
	for i := 1; i < data.Length(); i++ {
		fr.series = append(fr.series, optionalFloats(data.Index(i)))
	}
	for i := range fr.probes {
		q := probes.Index(i)
		fr.probes[i] = [4]float64{q.Index(0).Float(), q.Index(1).Float(), q.Index(2).Float(), q.Index(3).Float()}
	}
	return fr
}

func pairs(points [][2]float64) js.S {
	out := make(js.S, len(points))
	for i, p := range points {
		out[i] = js.S{p[0], p[1]}
	}
	return out
}

// precise is the adapter's Math.round(x * 1e6) || 0, which maps -0 and NaN to 0.
func precise(v float64) float64 {
	n := round(v * 1e6)
	if n == 0 || n != n {
		return 0
	}
	return n
}

// drawFrame is one frame: setSize, setData and setScale in a batch, then the recorded paths and
// the probes.
func drawFrame(canvas, ctx *js.Object, spanGaps bool, f frame, pxRatio float64) js.S {
	// The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
	widthCss, heightCss := f.size[0]/pxRatio, f.size[1]/pxRatio
	p := newPlot(pxRatio, widthCss, heightCss)
	canvas.Set("width", round(widthCss*pxRatio))
	canvas.Set("height", round(heightCss*pxRatio))

	u := newChart(p, scale{f.xRange[0], f.xRange[1]}, scale{f.yRange[0], f.yRange[1]}, f.xs)
	traces := make(js.S, len(f.series))
	if len(f.xs) > 0 {
		paths := make([]seriesPaths, len(f.series))
		for i, ys := range f.series {
			paths[i] = u.linear(ys, spanGaps)
		}
		for i, sp := range paths {
			drawPath(ctx, p, sp)
			var gaps interface{}
			if sp.gaps != nil {
				gaps = pairs(sp.gaps)
			}
			traces[i] = js.S{pairs(sp.stroke.vertices), gaps}
		}
	} else {
		// drawSeries runs only with data; the adapter then reports an empty trace for each series.
		for i := range traces {
			var gaps interface{}
			if !spanGaps {
				gaps = js.S{}
			}
			traces[i] = js.S{js.S{}, gaps}
		}
	}

	probes := make(js.S, len(f.probes))
	for i, q := range f.probes {
		x, y, px, py := q[0], q[1], q[2], q[3]
		probes[i] = js.S{
			precise(u.valToPosX(x)), precise(u.valToPosY(y)),
			precise(u.posToValX(px)), precise(u.posToValY(py)),
			u.valToIdx(x), u.posToIdx(px),
		}
	}
	return js.S{canvas.Get("width"), canvas.Get("height"), traces, probes}
}

func run(input *js.Object) js.S {
	pxRatio := js.Global.Get("devicePixelRatio").Float()
	spanGaps := input.Get("spanGaps").Bool()
	doc := js.Global.Get("document")
	// The canvas stands in for uPlot's elements in the adapter's host div; both leave the page.
	canvas := doc.Call("createElement", "canvas")
	doc.Get("body").Call("appendChild", canvas)
	defer canvas.Call("remove")
	ctx := canvas.Call("getContext", "2d")
	frames := input.Get("frames")
	out := make(js.S, frames.Length())
	for i := range out {
		out[i] = drawFrame(canvas, ctx, spanGaps, readFrame(frames.Index(i)), pxRatio)
	}
	return out
}

func main() { js.Module.Get("exports").Set("run", run) }
