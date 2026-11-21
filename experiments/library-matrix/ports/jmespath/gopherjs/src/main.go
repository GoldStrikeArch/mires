// Package main is jmespath 0.16.0 in Go, compiled to JavaScript by GopherJS: a faithful port of the
// subset libraries/jmespath/CONTRACT.md defines.
//
//   - lexer.go: the Lexer, over the expression's UTF-16 code units;
//   - parser.go: the Pratt parser, building jmespath's AST as plain JS objects;
//   - interpreter.go: the TreeInterpreter, strictDeepEqual and isFalse;
//   - functions.go: the Runtime's function table and argument validation;
//   - host.go: the JS values and host functions jmespath relies on;
//   - errors.go: the error categories of the reference adapter.
//
// The boundary: the corpus input is a *js.Object read in place. The document stays the JS value it
// is (jmespath returns parts of it), each expression stays a JS string, and each result is a plain
// object {value} or {error} with the reference's error category. run is published on
// js.Module.exports, which the builder's facade binds and re-exports.
package main

import "github.com/gopherjs/gopherjs/js"

// search is jmespath's search: a fresh parse and interpretation per call.
func search(data, expression *js.Object) (*js.Object, error) {
	ast, err := compile(expression)
	if err != nil {
		return nil, err
	}
	return visit(ast, data)
}

// query runs one query as reference.mjs does. Only jmespath's errors become categories. Nothing
// recovers a panic (stack exhaustion, an exception from a host call, a fault in the port): it
// propagates out of run, as anything else thrown does out of the reference adapter.
func query(q, data *js.Object) *js.Object {
	var value *js.Object
	var err error
	if q.Get("op").String() == "compile" {
		value, err = compile(q.Get("expression"))
	} else {
		value, err = search(data, q.Get("expression"))
	}
	result := newObject()
	switch e := err.(type) {
	case nil:
		result.Set("value", value)
	case *Error:
		result.Set("error", e.Category)
	default:
		panic(err)
	}
	return result
}

func run(input *js.Object) *js.Object {
	data := input.Get("data")
	results := newArray()
	for _, q := range elements(input.Get("queries")) {
		results.Call("push", query(q, data))
	}
	return results
}

func main() {
	js.Module.Get("exports").Set("run", run)
}
