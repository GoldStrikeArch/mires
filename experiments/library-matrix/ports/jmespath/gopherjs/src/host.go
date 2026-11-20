// The JavaScript values jmespath works on, and the host functions it calls.
//
// jmespath 0.16.0 evaluates over plain JS values (the JSON document, objects it builds, and its own
// AST nodes, which `&expr` hands to functions and to the caller) and leans on JS semantics for them:
// Object.keys order, === identity in contains, JSON.parse/JSON.stringify, parseInt,
// Number(string), localeCompare and the default Array.prototype.sort. The port keeps those values
// as *js.Object and calls those functions on the host. Text stays in JS strings: GopherJS's
// conversion of a JS string to a Go string mishandles lone surrogates, so the lexer reads code
// units with charCodeAt and cuts substrings with the host's slice.

package main

import "github.com/gopherjs/gopherjs/js"

var (
	jsObject       = js.Global.Get("Object")
	jsArray        = js.Global.Get("Array")
	jsJSON         = js.Global.Get("JSON")
	jsReflect      = js.Global.Get("Reflect")
	objectToString = jsObject.Get("prototype").Get("toString")
)

// typeTag is Object.prototype.toString.call(v), which jmespath's isArray, isObject and
// _getTypeName read: "[object Number]", "[object Array]", "[object Null]" and so on.
func typeTag(v *js.Object) string { return objectToString.Call("call", v).String() }

func isArray(v *js.Object) bool  { return jsArray.Call("isArray", v).Bool() }
func isObject(v *js.Object) bool { return typeTag(v) == "[object Object]" }
func isString(v *js.Object) bool { return typeTag(v) == "[object String]" }

// isNull: GopherJS reads a JS null as a nil *js.Object, and undefined as js.Undefined.
func isNull(v *js.Object) bool      { return v == nil }
func isUndefined(v *js.Object) bool { return v == js.Undefined }

// strictEquals is ===: GopherJS compares *js.Object values by the JS values they hold.
func strictEquals(a, b *js.Object) bool { return a == b }

func jsString(s string) *js.Object       { return js.Global.Call("String", s) }
func jsNumber(x float64) *js.Object      { return js.InternalObject(x) }
func jsBool(b bool) *js.Object           { return js.InternalObject(b) }
func newObject() *js.Object              { return jsObject.New() }
func objectKeys(v *js.Object) *js.Object { return jsObject.Call("keys", v) }

func newArray(items ...*js.Object) *js.Object {
	array := jsArray.New()
	for _, item := range items {
		array.Call("push", item)
	}
	return array
}

// elements reads a JS array into a slice.
func elements(array *js.Object) []*js.Object {
	items := make([]*js.Object, array.Length())
	for i := range items {
		items[i] = array.Index(i)
	}
	return items
}

// property is object[key] for a JS key value, through the prototype chain as in JS.
func property(object, key *js.Object) *js.Object { return jsReflect.Call("get", object, key) }

// setProperty is object[key] = value: Reflect.set runs the same [[Set]] as an assignment, so a
// "__proto__" key reaches Object.prototype's setter.
func setProperty(object, key, value *js.Object) { jsReflect.Call("set", object, key, value) }

// elementAt is array[index] for a JS number index: an element, or undefined off the end, at NaN
// and at ±∞.
func elementAt(array *js.Object, index float64) *js.Object {
	if index >= 0 && index < float64(array.Length()) && index == float64(int(index)) {
		return array.Index(int(index))
	}
	return js.Undefined
}

// number is Number(v): the unary + that to_number applies to strings, and ToNumber in comparisons.
func number(v *js.Object) float64 { return js.Global.Call("Number", v).Float() }

// stringOf is String(v), the conversion String.prototype.indexOf applies to its argument and
// ToPrimitive gives arrays and objects in comparisons.
func stringOf(v *js.Object) *js.Object { return js.Global.Call("String", v) }

func parseInt(text *js.Object) *js.Object { return js.Global.Call("parseInt", text) }

func stringify(v *js.Object) *js.Object { return jsJSON.Call("stringify", v) }

// parseJSON is JSON.parse, whose SyntaxError is a jmespath syntax error.
func parseJSON(text *js.Object) (value *js.Object, err error) {
	defer func() {
		if recover() != nil {
			value, err = nil, syntaxError("invalid JSON")
		}
	}()
	return jsJSON.Call("parse", text), nil
}

// compareCodeUnits orders two JS strings as JS's < does: by UTF-16 code unit.
func compareCodeUnits(a, b *js.Object) int {
	la, lb := a.Length(), b.Length()
	for i := 0; i < la && i < lb; i++ {
		x, y := a.Call("charCodeAt", i).Int(), b.Call("charCodeAt", i).Int()
		if x != y {
			return x - y
		}
	}
	return la - lb
}

// order is JS's abstract relational comparison for these values, which jmespath's comparators,
// max_by, min_by and sort_by use through < and >: arrays and objects become their String() forms,
// two strings compare by code unit, and anything else compares as numbers. ok is false when a
// number is NaN, where every relational operator is false.
func order(a, b *js.Object) (cmp int, ok bool) {
	if isArray(a) || isObject(a) {
		a = stringOf(a)
	}
	if isArray(b) || isObject(b) {
		b = stringOf(b)
	}
	if isString(a) && isString(b) {
		return compareCodeUnits(a, b), true
	}
	x, y := number(a), number(b)
	switch {
	case x < y:
		return -1, true
	case x > y:
		return 1, true
	case x == y:
		return 0, true
	}
	return 0, false
}

func greater(a, b *js.Object) bool        { cmp, ok := order(a, b); return ok && cmp > 0 }
func greaterOrEqual(a, b *js.Object) bool { cmp, ok := order(a, b); return ok && cmp >= 0 }
func less(a, b *js.Object) bool           { cmp, ok := order(a, b); return ok && cmp < 0 }
func lessOrEqual(a, b *js.Object) bool    { cmp, ok := order(a, b); return ok && cmp <= 0 }
