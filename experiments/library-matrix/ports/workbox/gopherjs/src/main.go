// The port's one export (README, "Port convention"): run, published on js.Module.exports, which
// the builder's facade re-exports.

package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", run)
}
