// Command runtime-floor is the one-export empty program that measures the GopherJS runtime
// floor: the prelude and the packages every GopherJS program links, with no FingerprintJS
// code. It exports load as a function returning 1, like the Scala.js floor.
package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("load", func() int { return 1 })
}
