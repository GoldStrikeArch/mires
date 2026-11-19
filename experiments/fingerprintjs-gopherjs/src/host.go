// Package main is a Go port of FingerprintJS 5.2.0, compiled to JavaScript by GopherJS.
//
// This file is the JavaScript interop layer. Every helper is built on the native
// github.com/gopherjs/gopherjs/js package and the GopherJS prelude; there is no
// embedded JavaScript source.
//
// Browser globals (window, document, navigator, screen) are read on every use and
// never during package initialisation, so importing the module in an environment
// without them touches none of them.
package main

import "github.com/gopherjs/gopherjs/js"

func window() *js.Object    { return browserGlobal("window") }
func document() *js.Object  { return browserGlobal("document") }
func navigator() *js.Object { return browserGlobal("navigator") }
func screen() *js.Object    { return browserGlobal("screen") }
func console() *js.Object   { return js.Global.Get("console") }

// browserGlobal reads a global as the oracle's bare identifier does: a missing global
// throws a ReferenceError rather than reading as undefined.
func browserGlobal(name string) *js.Object {
	if !has(js.Global, name) {
		throw(js.Global.Get("ReferenceError").New(name + " is not defined"))
	}
	return js.Global.Get(name)
}

// toJS externalises a Go value through the prelude's identity function $id.
func toJS(v any) *js.Object { return js.Global.Call("$id", v) }

// typeOf and instanceOf are the JavaScript operators, exposed by the GopherJS prelude
// as $typeOf and $instanceOf.
func typeOf(v *js.Object) string { return js.Global.Call("$typeOf", v).String() }

func instanceOf(v, constructor *js.Object) bool {
	return js.Global.Call("$instanceOf", v, constructor).Bool()
}

func isFunction(v *js.Object) bool { return typeOf(v) == "function" }
func isError(v *js.Object) bool    { return instanceOf(v, js.Global.Get("Error")) }
func isNullish(v *js.Object) bool  { return v == nil || v == js.Undefined }

// equals is value === s.
func equals(value *js.Object, s string) bool { return value == toJS(s) }

// has is the `in` operator.
func has(object *js.Object, key string) bool {
	return js.Global.Get("Reflect").Call("has", object, key).Bool()
}

// getKey and setKey access a property through a JavaScript key value, so keys that
// are not valid UTF-16 (lone surrogates) never round-trip through a Go string.
func getKey(object, key *js.Object) *js.Object {
	return js.Global.Get("Reflect").Call("get", object, key)
}

func setKey(object, key, value *js.Object) {
	js.Global.Get("Reflect").Call("set", object, key, value)
}

// keys is Object.keys, as a JavaScript array.
func keys(object *js.Object) *js.Object { return js.Global.Get("Object").Call("keys", object) }

func newObject() *js.Object { return js.Global.Get("Object").New() }

// object builds a plain object from alternating keys and values, in that order.
func object(entries ...any) *js.Object {
	o := newObject()
	for i := 0; i < len(entries); i += 2 {
		o.Set(entries[i].(string), entries[i+1])
	}
	return o
}

// array builds a JavaScript array (Array.of).
func array(items ...any) *js.Object { return js.Global.Get("Array").Call("of", items...) }

func stringArray(items []string) *js.Object {
	a := array()
	for _, item := range items {
		a.Call("push", item)
	}
	return a
}

// jsString is ToString as template literals apply it; String.prototype.concat
// applies the same conversion to its arguments.
func jsString(v any) string {
	return js.Global.Get("String").Get("prototype").Get("concat").Call("call", "", v).String()
}

// toNumber is ToNumber.
func toNumber(v *js.Object) float64 { return js.Global.Call("Number", v).Float() }

// utf8String converts a JavaScript string to the Go string holding its UTF-8 bytes,
// exactly as TextEncoder encodes it: unpaired surrogates become U+FFFD. GopherJS's own
// string conversion is exact for well-formed UTF-16 but mangles an unpaired high
// surrogate, so any non-ASCII string goes through the platform encoder.
func utf8String(v *js.Object) string {
	s := v.String()
	for i := 0; i < len(s); i++ {
		if s[i] >= 0x80 {
			return string(textEncode(v))
		}
	}
	return s
}

func textEncode(v *js.Object) []byte {
	return byteSlice(js.Global.Get("TextEncoder").New().Call("encode", v))
}

// byteSlice converts an array of bytes to Go. A same-realm Uint8Array becomes a []byte
// without copying; any other array-like is copied element by element.
func byteSlice(bytes *js.Object) []byte {
	if b, ok := bytes.Interface().([]byte); ok {
		return b
	}
	b := make([]byte, bytes.Length())
	for i := range b {
		b[i] = byte(bytes.Index(i).Int())
	}
	return b
}

func dateNow() float64 { return js.Global.Get("Date").Call("now").Float() }

func newRegExp(pattern, flags string) *js.Object {
	return js.Global.Get("RegExp").New(pattern, flags)
}

func regExpTest(pattern, flags string, input any) bool {
	return newRegExp(pattern, flags).Call("test", input).Bool()
}

func newError(message string) *js.Object { return js.Global.Get("Error").New(message) }

// mathCall calls a function of the host Math object. Several sources fingerprint the
// engine's own Math implementation, and Math.round has JavaScript rounding semantics.
func mathCall(name string, args ...any) float64 {
	return js.Global.Get("Math").Call(name, args...).Float()
}

// jsFunc wraps fn as a JavaScript function whose name and length match the oracle's,
// by redefining the two configurable function properties.
func jsFunc(name string, length int, fn func(this *js.Object, args []*js.Object) any) *js.Object {
	f := js.MakeFunc(fn)
	defineProperty := js.Global.Get("Object").Get("defineProperty")
	defineProperty.Invoke(f, "name", object("value", name))
	defineProperty.Invoke(f, "length", object("value", length))
	return f
}

// arg returns the i-th argument of a JavaScript call, or undefined when it was omitted.
func arg(args []*js.Object, i int) *js.Object {
	if i < len(args) {
		return args[i]
	}
	return js.Undefined
}

// tryJS runs fn and returns the JavaScript exception it threw, if any. GopherJS turns
// a JavaScript exception that unwinds through a deferred call into a *js.Error panic
// that holds the original thrown value. A thrown null is the exception: GopherJS uses
// `throw null` for its own unwinding and runs the deferred calls without a panic, so a
// null exception shows only as fn not completing.
func tryJS(fn func()) (exception *js.Object, threw bool) {
	completed := false
	defer func() {
		r := recover()
		switch {
		case r != nil:
			jsError, ok := r.(*js.Error)
			if !ok {
				panic(r)
			}
			exception, threw = jsError.Object, true
		case !completed:
			exception, threw = nil, true
		}
	}()
	fn()
	completed = true
	return nil, false
}

// throw rethrows a JavaScript value unchanged, through the prelude's $throw.
func throw(v *js.Object) { js.Global.Call("$throw", v) }
