package main

import (
	"github.com/gopherjs/gopherjs/js"
	"strings"
)

func integer(c *js.Object, key string, fallback int) int {
	v := c.Get(key)
	if v == js.Undefined {
		return fallback
	}
	n := v.Int()
	if float64(n) != v.Float() || n == -2000002 {
		return -2000001
	}
	return n
}

func operation(c *js.Object) (result string) {
	defer func() {
		if e := recover(); e != nil {
			if s, ok := e.(string); ok {
				result = s
			} else {
				panic(e)
			}
		}
	}()
	rm := integer(c, "rm", 1)
	return operate(c.Get("op").String(), js.Global.Get("String").Invoke(c.Get("x")).String(),
		js.Global.Get("String").Invoke(c.Get("y")).String(), integer(c, "n", -2000002),
		integer(c, "dp", 20), rm, integer(c, "mode", rm))
}
func run(input *js.Object) string {
	var lines []string
	rows := input.Get("cases")
	for k := 0; k < integer(input, "repeat", 1); k++ {
		for i := 0; i < rows.Length(); i++ {
			lines = append(lines, operation(rows.Index(i)))
		}
	}
	return strings.Join(lines, "\n")
}
func main() { js.Module.Get("exports").Set("run", run) }
