package main

import (
	"math"
	"sort"
	"strings"
	"unicode/utf16"

	"github.com/gopherjs/gopherjs/js"
)

type value = *js.Object
type token struct {
	kind  string
	value value
}
type parser struct {
	tokens []token
	index  int
}

func (p *parser) tokenAt(i int) token {
	if i < 0 || i >= len(p.tokens) {
		return token{"EOF", js.Undefined}
	}
	return p.tokens[i]
}
func tokensBuffer() *[]token                  { return new([]token) }
func addToken(xs *[]token, k string, v value) { *xs = append(*xs, token{k, v}) }
func str(s string) value                      { return js.Global.Get("String").Invoke(s) }
func numeric(x float64) value                 { return js.Global.Get("Number").Invoke(x) }
func intValue(x int) value                    { return numeric(float64(x)) }
func boolValue(v bool) value                  { return js.Global.Get("Boolean").Invoke(v) }
func codeAt(s string, i int) int              { return int(s[i]) }
func strLen(s string) int                     { return len(s) }
func part(s string, i, j int) string          { return s[i:j] }
func slash() string                           { return "\\" }
func replaceFirst(s, a, b string) string      { return strings.Replace(s, a, b, 1) }
func decodeJson(s string) value               { return js.Global.Get("JSON").Call("parse", s) }
func tryJSON(s string) (v value, ok bool) {
	defer func() {
		if recover() != nil {
			ok = false
		}
	}()
	return decodeJson(s), true
}
func parseInteger(s string) value { return js.Global.Get("parseInt").Invoke(s, 10) }
func literal(input string) value {
	s := replaceFirst(strings.TrimLeft(input, " \t\n\r\f\v"), "\\`", "`")
	if strings.HasPrefix(s, "[") || strings.HasPrefix(s, "{") || strings.HasPrefix(s, "\"") || s == "true" || s == "false" || s == "null" {
		return decodeJson(s)
	}
	if len(s) > 0 && (s[0] == '-' || (s[0] >= '0' && s[0] <= '9')) {
		if x, ok := tryJSON(s); ok {
			return x
		}
	}
	return decodeJson("\"" + s + "\"")
}
func arr() value           { return js.Global.Get("Array").New() }
func obj() value           { return js.Global.Get("Object").New() }
func missing(v value) bool { return v == js.Undefined }
func isNil(v value) bool   { return v == nil }
func flag(v value) bool    { return v.Bool() }
func infinity(pos bool) value {
	if pos {
		return numeric(math.Inf(1))
	}
	return numeric(math.Inf(-1))
}
func nan(x float64) bool         { return math.IsNaN(x) }
func stringValue(v value) string { return js.Global.Get("String").Invoke(v).String() }
func toNumber(v value) float64   { return js.Global.Get("Number").Invoke(v).Float() }
func integer(v value) int        { return int(toNumber(v)) }
func same(a, b value) bool       { return a == b }
func clamp(i, lo, hi float64) float64 {
	if i < lo {
		return lo
	}
	if i > hi {
		return hi
	}
	return i
}
func field(v value, k string) value {
	if v == nil || v == js.Undefined {
		panic("syntax")
	}
	return v.Get(k)
}
func at(v value, i int) value          { return v.Index(i) }
func size(v value) int                 { return v.Length() }
func put(v value, k string, x value)   { v.Set(k, x) }
func push(a, x value)                  { a.Call("push", x) }
func setIndex(a value, i int, x value) { a.SetIndex(i, x) }
func keys(v value) value               { return js.Global.Get("Object").Call("keys", v) }

var typeTag = js.Global.Get("Object").Get("prototype").Get("toString")

func kind(v value) string {
	if v == nil {
		return "null"
	}
	if v == js.Undefined {
		return "undefined"
	}
	switch typeTag.Call("call", v).String() {
	case "[object Boolean]":
		return "boolean"
	case "[object String]":
		return "string"
	case "[object Number]":
		return "number"
	case "[object Array]":
		return "array"
	default:
		if v.Get("jmespathType") == str("Expref") {
			return "expref"
		}
		return "object"
	}
}
func compare(a, b value) int {
	ka, kb := kind(a), kind(b)
	if ka == "array" || ka == "object" || ka == "expref" {
		a = str(stringValue(a))
		ka = "string"
	}
	if kb == "array" || kb == "object" || kb == "expref" {
		b = str(stringValue(b))
		kb = "string"
	}
	if ka == "string" && kb == "string" {
		x, y := utf16.Encode([]rune(a.String())), utf16.Encode([]rune(b.String()))
		for i := 0; i < len(x) && i < len(y); i++ {
			if x[i] < y[i] {
				return -1
			}
			if x[i] > y[i] {
				return 1
			}
		}
		if len(x) < len(y) {
			return -1
		}
		if len(x) > len(y) {
			return 1
		}
		return 0
	}
	x, y := toNumber(a), toNumber(b)
	if x < y {
		return -1
	}
	if x > y {
		return 1
	}
	if x == y {
		return 0
	}
	return 2
}
func localeOrder(a, b value) int { return a.Call("localeCompare", b).Int() }
func jsonText(a value) value     { return js.Global.Get("JSON").Call("stringify", a) }
func sortByExpression(a, b value) value {
	type entry struct{ item, key value }
	entries := make([]entry, size(a))
	t := ""
	for i := range entries {
		item := at(a, i)
		key := evaluate(b, item)
		kt := kind(key)
		if i == 0 {
			t = kt
		}
		if (t != "number" && t != "string") || kt != t {
			panic("invalid-type")
		}
		entries[i] = entry{item, key}
	}
	sort.SliceStable(entries, func(i, j int) bool { return compare(entries[i].key, entries[j].key) < 0 })
	out := arr()
	for _, e := range entries {
		push(out, e.item)
	}
	return out
}
func primitiveFunction(name string, a, b value) value {
	switch name {
	case "abs":
		return numeric(math.Abs(toNumber(a)))
	case "ceil":
		return numeric(math.Ceil(toNumber(a)))
	case "floor":
		return numeric(math.Floor(toNumber(a)))
	case "contains":
		return boolValue(a.Call("indexOf", b).Int() >= 0)
	case "starts_with":
		return boolValue(a.Call("lastIndexOf", b).Int() == 0)
	case "ends_with":
		return boolValue(a.Call("indexOf", b, size(a)-size(b)).Int() >= 0)
	case "join":
		return b.Call("join", a)
	case "sort":
		return a.Call("slice").Call("sort")
	case "reverse":
		if kind(a) == "string" {
			return a.Call("split", "").Call("reverse").Call("join", "")
		}
		return a.Call("slice").Call("reverse")
	default:
		panic("unknown-function")
	}
}
func query(q, data value) (out value) {
	out = obj()
	defer func() {
		if e := recover(); e != nil {
			kind := "syntax"
			if k, ok := e.(string); ok {
				switch k {
				case "invalid-type", "invalid-arity", "unknown-function", "invalid-value":
					kind = k
				}
			}
			put(out, "error", str(kind))
		}
	}()
	p := parser{tokens: *lex(q.Get("expression").String())}
	ast := p.parse()
	if q.Get("op").String() == "compile" {
		put(out, "value", ast)
	} else {
		put(out, "value", evaluate(ast, data))
	}
	return out
}
func main() {
	js.Module.Get("exports").Set("run", func(input value) value {
		data := input.Get("data")
		queries := input.Get("queries")
		out := arr()
		for i := 0; i < size(queries); i++ {
			push(out, query(at(queries, i), data))
		}
		return out
	})
}
