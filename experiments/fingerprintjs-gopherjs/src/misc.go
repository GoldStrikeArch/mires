package main

import "github.com/gopherjs/gopherjs/js"

// errorToObject converts an error to a plain object that JSON.stringify can serialise:
// {name, message, stack: lines, ...ownEnumerableProperties}. Own properties are copied
// the way object spread copies them, so an own `name` keeps its leading position.
func errorToObject(err *js.Object) *js.Object {
	result := object("name", err.Get("name"), "message", err.Get("message"))
	if stack := err.Get("stack"); isNullish(stack) {
		result.Set("stack", js.Undefined)
	} else {
		result.Set("stack", stack.Call("split", "\n"))
	}

	defineProperty := js.Global.Get("Reflect").Get("defineProperty")
	ownKeys := keys(err)
	for i := 0; i < ownKeys.Length(); i++ {
		key := ownKeys.Index(i)
		defineProperty.Invoke(result, key, object(
			"value", getKey(err, key), "writable", true, "enumerable", true, "configurable", true))
	}
	return result
}

func isFunctionNative(fn *js.Object) bool {
	return regExpTest(`^function\s.*?\{\s*\[native code]\s*}$`, "", js.Global.Call("String", fn))
}
