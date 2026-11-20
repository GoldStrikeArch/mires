// The boundary: what libraries/big-js/reference.mjs does. dp, rm and repeat take their defaults
// when undefined or null, as the adapter's ?? does; n and mode stay optional (*float64) and reach
// the methods as upstream's optional arguments.
package main

import (
	"math"
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// option is object[key] as a number, nil when it is undefined or null.
func option(object *js.Object, key string) *float64 {
	v := object.Get(key)
	if v == js.Undefined || v == nil {
		return nil
	}
	n := v.Float()
	return &n
}

func orDefault(n *float64, fallback float64) float64 {
	if n == nil {
		return fallback
	}
	return *n
}

// text is the Big constructor's conversion: a number goes through String(), except that -0
// becomes "-0".
func text(v *js.Object) string {
	switch value := v.Interface().(type) {
	case string:
		return value
	case float64:
		if value == 0 && math.Signbit(value) {
			return "-0"
		}
		return jsString(value)
	}
	return js.Global.Call("String", v).String()
}

func operate(c *js.Object) (string, error) {
	dp, rm := orDefault(option(c, "dp"), 20), orDefault(option(c, "rm"), 1)
	x, err := parse(text(c.Get("x")))
	if err != nil {
		return "", err
	}
	n, mode := option(c, "n"), orDefault(option(c, "mode"), rm)
	op := c.Get("op").String()
	var result *Big
	switch op {
	case "toString":
		return x.String(), nil
	case "toFixed":
		return x.toFixed(n, mode)
	case "toExponential":
		return x.toExponential(n, mode)
	case "toPrecision":
		return x.toPrecision(n, mode)
	case "sqrt":
		result, err = x.sqrt(dp, rm)
	case "pow":
		result, err = x.pow(n, dp, rm)
	case "round":
		result, err = x.round(n, mode)
	case "plus", "minus", "times", "div", "cmp":
		var y *Big
		if y, err = parse(text(c.Get("y"))); err != nil {
			return "", err
		}
		switch op {
		case "plus":
			return x.plus(y).String(), nil
		case "minus":
			return x.minus(y).String(), nil
		case "times":
			return x.times(y).String(), nil
		case "cmp":
			return jsString(float64(x.cmp(y))), nil
		}
		result, err = x.div(y, dp, rm)
	default:
		return "", bigError("unknown decimal operation")
	}
	if err != nil {
		return "", err
	}
	return result.String(), nil
}

func run(input *js.Object) string {
	cases := input.Get("cases")
	repeat := int(orDefault(option(input, "repeat"), 1))
	var outputs []string
	for k := 0; k < repeat; k++ {
		for i := 0; i < cases.Length(); i++ {
			result, err := operate(cases.Index(i))
			if err != nil {
				result = err.Error()
			}
			outputs = append(outputs, result)
		}
	}
	return strings.Join(outputs, "\n")
}

func main() { js.Module.Get("exports").Set("run", run) }
