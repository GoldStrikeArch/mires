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
	// canonicalize() calls a truthy toJSON member, which JSON cannot make callable.
	if v.Get("toJSON").Bool() {
		panic(js.Global.Get("TypeError").New("obj.toJSON is not a function"))
	}
	// canonicalize() assigns the sorted keys to a plain object, so a
	// "__proto__" key sets its prototype and is not serialized.
	a := object()
	ks := js.Global.Get("Object").Call("keys", v).Call("sort")
	reflect := js.Global.Get("Reflect")
	for i := 0; i < ks.Length(); i++ {
		k := ks.Index(i)
		reflect.Call("set", a, k, canonical(reflect.Call("get", v, k)))
	}
	return a
}
func isString(v *js.Object) bool {
	return v != nil && js.Global.Get("Object").Get("prototype").Get("toString").Call("call", v).String() == "[object String]"
}
func doc(v *js.Object, op string) string {
	if op == "diffJson" && !isString(v) {
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
// arrayKeys are the elements' JSON texts, which compare as === does for JSON scalars. With
// ignoreCase, strings also compare lowercased (the JSON text of a lowercased string is the
// lowercased JSON text).
func arrayKeys(v *js.Object, ignoreCase bool) []string {
	out := make([]string, v.Length())
	for i := range out {
		key := js.Global.Get("JSON").Call("stringify", v.Index(i))
		if ignoreCase {
			key = key.Call("toLowerCase")
		}
		out[i] = key.String()
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
		spans := myers(arrayKeys(a, o.ignoreCase), arrayKeys(b, o.ignoreCase), o.individual)
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
		ctx := 4.0
		if opts != js.Undefined && opts.Get("context") != js.Undefined {
			ctx = opts.Get("context").Float()
		}
		p := create(input, ctx)
		if op == "structuredPatch" {
			return p.write()
		}
		return writeText(format(p))
	case "parsePatch":
		ps := parse(readText(arg))
		out := arr(len(ps))
		for i, p := range ps {
			out.SetIndex(i, p.write())
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
		var patches []patch
		switch {
		case isString(arg):
			patches = parse(readText(arg))
		case isArray(arg):
			for i := 0; i < arg.Length(); i++ {
				patches = append(patches, readPatch(arg.Index(i)))
			}
		default:
			patches = []patch{readPatch(arg)}
		}
		if len(patches) != 1 {
			panic("applyPatch only works with a single input")
		}
		return apply(readText(a), patches[0])
	default:
		panic("unknown diff operation")
	}
}
func main() { js.Module.Get("exports").Set("run", run) }
