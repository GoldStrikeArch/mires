package main

import "github.com/gopherjs/gopherjs/js"

func integers(v *js.Object) []int {
	if v == js.Undefined || v == nil {
		return nil
	}
	a := make([]int, v.Length())
	for i := range a {
		a[i] = v.Index(i).Int()
	}
	return a
}
func coordinates(v *js.Object) []float64 {
	a := make([]float64, v.Length())
	for i := range a {
		a[i] = v.Index(i).Float()
	}
	return a
}
func dimension(item *js.Object) int {
	if d := item.Get("dim"); d != js.Undefined {
		return d.Int()
	}
	return 2
}
func indices(a []int) *js.Object {
	out := js.Global.Get("Array").New(len(a))
	for i, v := range a {
		out.SetIndex(i, v)
	}
	return out
}
func polygon(item *js.Object, doRefine bool) *js.Object {
	dim := dimension(item)
	data := coordinates(item.Get("data"))
	t := triangulate(data, integers(item.Get("holeIndices")), dim)
	if doRefine {
		refine(t, data, dim)
	}
	return indices(t)
}
func flatten(rings *js.Object) *js.Object {
	if rings.Length() == 0 || rings.Index(0).Length() == 0 {
		panic("flatten: empty outer ring")
	}
	dim := rings.Index(0).Index(0).Length()
	vertices := js.Global.Get("Array").New()
	holes := js.Global.Get("Array").New()
	holeIndex, prevLen := 0, 0
	for i := 0; i < rings.Length(); i++ {
		ring := rings.Index(i)
		for j := 0; j < ring.Length(); j++ {
			point := ring.Index(j)
			for d := 0; d < dim; d++ {
				vertices.Call("push", point.Index(d))
			}
		}
		if prevLen != 0 {
			holeIndex += prevLen
			holes.Call("push", holeIndex)
		}
		prevLen = ring.Length()
	}
	out := js.Global.Get("Object").New()
	out.Set("vertices", vertices)
	out.Set("holes", holes)
	out.Set("dimensions", dim)
	return out
}
func run(input *js.Object) interface{} {
	switch input.Get("op").String() {
	case "earcut":
		return polygon(input, false)
	case "earcutRefine":
		return polygon(input, true)
	case "batch":
		items := input.Get("items")
		out := js.Global.Get("Array").New(items.Length())
		for i := 0; i < items.Length(); i++ {
			item := items.Index(i)
			out.SetIndex(i, polygon(item, item.Get("refine").Bool()))
		}
		return out
	case "flatten":
		return flatten(input.Get("rings"))
	case "refine":
		t := integers(input.Get("triangles"))
		refine(t, coordinates(input.Get("coords")), dimension(input))
		return indices(t)
	case "deviation":
		return deviation(coordinates(input.Get("data")), integers(input.Get("holeIndices")), dimension(input), integers(input.Get("triangles")))
	default:
		panic("unknown earcut operation")
	}
}
func main() { js.Module.Get("exports").Set("run", run) }
