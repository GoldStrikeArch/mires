// Boundary between JavaScript corpus inputs and the Go marked implementation, mirroring
// libraries/marked/reference.mjs.
package main

import "github.com/gopherjs/gopherjs/js"

func readOptions(obj *js.Object) *Options {
	opts := defaultOptions()
	if obj == js.Undefined || obj == nil {
		return opts
	}
	if v := obj.Get("gfm"); v != js.Undefined {
		opts.gfm = v.Bool()
	}
	if v := obj.Get("breaks"); v != js.Undefined {
		opts.breaks = v.Bool()
	}
	if v := obj.Get("pedantic"); v != js.Undefined {
		opts.pedantic = v.Bool()
	}
	return opts
}

// run is the module's export: executes parse or parseInline and returns the rendered HTML string.
func run(input *js.Object) *js.Object {
	opts := readOptions(input.Get("options"))
	op := input.Get("op").String()
	md := input.Get("markdown")
	if md == js.Undefined || md == nil {
		panic("marked(): input parameter is undefined or null")
	}
	switch op {
	case "parse":
		return parse(lex(md, opts), opts)
	case "parseInline":
		return parseInline(lexInline(md, opts), opts)
	default:
		panic("unknown operation " + op)
	}
}
