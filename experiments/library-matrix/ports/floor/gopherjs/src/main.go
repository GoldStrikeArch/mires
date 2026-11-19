// Floor port (GopherJS): a JS string to a Go UTF-8 string and back, published on js.Module.exports
// (the comparator's one-export runtime-floor program, with the matrix boundary).
package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", func(input *js.Object) string {
		return input.Get("text").String()
	})
}
