// Entry of the GopherJS port. The boundary: the corpus input is a *js.Object; `text` is converted
// from a JS string to a Go (UTF-8) string and `top` to an int by the js package, and the Go string
// result is converted back to a JS string. `run` is published on js.Module.exports, which the
// builder's facade binds.

package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", func(input *js.Object) string {
		return analyse(input.Get("text").String(), input.Get("top").Int())
	})
}
