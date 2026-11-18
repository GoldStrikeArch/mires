package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// Functions that work with data only: no browser features, DOM or side effects.

// includes is Array.prototype.includes over an array-like, with === equality.
func includes(haystack, needle *js.Object) bool {
	for i, l := 0, haystack.Length(); i < l; i++ {
		if haystack.Index(i) == needle {
			return true
		}
	}
	return false
}

func excludes(haystack, needle *js.Object) bool { return !includes(haystack, needle) }

// toInt and toFloat are the host parseInt and parseFloat. Be careful, NaN can return.
func toInt(value *js.Object) float64   { return js.Global.Call("parseInt", value).Float() }
func toFloat(value *js.Object) float64 { return js.Global.Call("parseFloat", value).Float() }

// replaceNaN returns replacement instead of a NaN number.
func replaceNaN(value float64, replacement *js.Object) *js.Object {
	if math.IsNaN(value) {
		return replacement
	}
	return toJS(value)
}

func countTruthy(values ...bool) int {
	count := 0
	for _, value := range values {
		if value {
			count++
		}
	}
	return count
}

func mathRound(value float64) float64 { return mathCall("round", value) }

func round(value, base float64) float64 {
	if math.Abs(base) >= 1 {
		return mathRound(value/base) * base
	}
	// Multiplying by a small number loses precision (1234 * 0.0001 = 0.12340000000000001),
	// dividing by its inverse does not (1234 / (1 / 0.0001) = 0.1234).
	counterBase := 1 / base
	return mathRound(value*counterBase) / counterBase
}

// selectorAttributes keeps HTML attributes in first-seen order; an attribute can have
// several values.
type selectorAttributes struct {
	names  []string
	values map[string][]string
}

func (a *selectorAttributes) add(name, value string) {
	if _, seen := a.values[name]; !seen {
		a.names = append(a.names, name)
	}
	a.values[name] = append(a.values[name], value)
}

// parseSimpleCssSelector parses a single-element CSS selector (no operators like space, +
// or >) into a tag name, empty when there is none, and HTML attributes. The patterns are
// the oracle's, run by the host RegExp so that their JavaScript semantics are kept.
func parseSimpleCssSelector(selector string) (tag string, attributes *selectorAttributes) {
	errorMessage := "Unexpected syntax '" + selector + "'"
	tagMatch := newRegExp(`^\s*([a-z-]*)(.*)$`, "i").Call("exec", selector)
	tag = tagMatch.Index(1).String()
	attributes = &selectorAttributes{values: map[string][]string{}}
	partsRegex := newRegExp(`([.:#][\w-]+|\[.+?\])`, "gi")
	rest := tagMatch.Index(2)

	for {
		match := partsRegex.Call("exec", rest)
		if match == nil {
			break
		}
		part := match.Index(0).String()
		switch part[0] {
		case '.':
			attributes.add("class", part[1:])
		case '#':
			attributes.add("id", part[1:])
		case '[':
			attributeMatch := newRegExp(`^\[([\w-]+)([~|^$*]?=("(.*?)"|([\w-]+)))?(\s+[is])?\]$`, "").Call("exec", part)
			if attributeMatch == nil {
				throw(newError(errorMessage))
			}
			value := ""
			if quoted := attributeMatch.Index(4); !isNullish(quoted) {
				value = quoted.String()
			} else if bare := attributeMatch.Index(5); !isNullish(bare) {
				value = bare.String()
			}
			attributes.add(attributeMatch.Index(1).String(), value)
		default:
			throw(newError(errorMessage))
		}
	}

	return tag, attributes
}
