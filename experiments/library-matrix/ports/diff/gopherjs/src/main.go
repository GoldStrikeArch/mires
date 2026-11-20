package main

import (
	"github.com/gopherjs/gopherjs/js"
)

func arr(n int) *js.Object      { return js.Global.Get("Array").New(n) }
func object() *js.Object        { return js.Global.Get("Object").New() }
func isArray(v *js.Object) bool { return js.Global.Get("Array").Call("isArray", v).Bool() }
func lowercase(s string) string {
	return readText(writeText(s).Call("toLowerCase"))
}
func readOpts(v *js.Object) options {
	if v == js.Undefined || v == nil {
		return options{}
	}
	return options{ignoreCase: v.Get("ignoreCase").Bool(), individual: v.Get("oneChangePerToken").Bool(), stripCR: v.Get("stripTrailingCr").Bool(), ignoreWS: v.Get("ignoreWhitespace").Bool(), newlineToken: v.Get("newlineIsToken").Bool(), ignoreEOF: v.Get("ignoreNewlineAtEof").Bool()}
}
func canonical(v *js.Object) *js.Object {
	if v == nil {
		return nil
	}
	if isArray(v) {
		a := arr(v.Length())
		for i := 0; i < v.Length(); i++ {
			a.SetIndex(i, canonical(v.Index(i)))
		}
		return a
	}
	if js.Global.Get("Object").Get("prototype").Get("toString").Call("call", v).String() != "[object Object]" {
		return v
	}
	a := js.Global.Get("Object").Call("create", nil)
	ks := js.Global.Get("Object").Call("keys", v).Call("sort")
	reflect := js.Global.Get("Reflect")
	for i := 0; i < ks.Length(); i++ {
		k := ks.Index(i)
		reflect.Call("set", a, k, canonical(reflect.Call("get", v, k)))
	}
	return a
}
func doc(v *js.Object, op string) string {
	if op == "diffJson" && (v == nil || js.Global.Get("Object").Get("prototype").Get("toString").Call("call", v).String() != "[object String]") {
		return readText(js.Global.Get("JSON").Call("stringify", canonical(v), nil, 2))
	}
	return readText(v)
}
func writeChange(kind, count int, v interface{}) *js.Object {
	x := object()
	x.Set("count", count)
	x.Set("added", kind == add)
	x.Set("removed", kind == remove)
	x.Set("value", v)
	return x
}
func arrayKeys(v *js.Object) []string {
	out := make([]string, v.Length())
	for i := range out {
		out[i] = js.Global.Get("JSON").Call("stringify", v.Index(i)).String()
	}
	return out
}
func run(input *js.Object) interface{} {
	op := input.Get("op").String()
	a, b := input.Get("before"), input.Get("after")
	arg := input.Get("patch")
	opts := input.Get("options")
	o := readOpts(opts)
	switch op {
	case "diffArrays":
		spans := myers(arrayKeys(a), arrayKeys(b), o.individual)
		all := arr(len(spans))
		old, new := 0, 0
		for i, r := range spans {
			v := arr(r.count)
			for j := 0; j < r.count; j++ {
				if r.kind == remove {
					v.SetIndex(j, a.Index(old+j))
				} else {
					v.SetIndex(j, b.Index(new+j))
				}
			}
			all.SetIndex(i, writeChange(r.kind, r.count, v))
			if r.kind != remove {
				new += r.count
			}
			if r.kind != add {
				old += r.count
			}
		}
		return all
	case "diffChars", "diffWords", "diffLines", "diffJson":
		diff := changes(op, tokens(op, doc(a, op), o), tokens(op, doc(b, op), o), o)
		all := arr(len(diff))
		for i, c := range diff {
			all.SetIndex(i, writeChange(c.kind, c.count, writeText(c.value)))
		}
		return all
	case "createTwoFilesPatch", "structuredPatch":
		ctx := 4
		if opts != js.Undefined && opts.Get("context") != js.Undefined {
			ctx = opts.Get("context").Int()
		}
		p := create(input, ctx)
		if op == "structuredPatch" {
			return p.write(false)
		}
		return writeText(format(p))
	case "parsePatch":
		ps := parse(readText(arg))
		out := arr(len(ps))
		for i, p := range ps {
			out.SetIndex(i, p.write(false))
		}
		return out
	case "reversePatch":
		if isArray(arg) {
			n := arg.Length()
			out := arr(n)
			for i := 0; i < n; i++ {
				out.SetIndex(n-i-1, reverse(arg.Index(i)))
			}
			return out
		}
		return reverse(arg)
	case "applyPatch":
		var p patch
		if isArray(arg) {
			if arg.Length() != 1 {
				panic("multiple file patches")
			}
			p = readPatch(arg.Index(0))
		} else if arg.Get("hunks") != js.Undefined {
			p = readPatch(arg)
		} else {
			ps := parse(readText(arg))
			if len(ps) != 1 {
				panic("multiple file patches")
			}
			p = ps[0]
		}
		return apply(readText(a), p)
	default:
		panic("unknown diff operation")
	}
}
func main() { js.Module.Get("exports").Set("run", run) }
