// The browser APIs idb drives: IndexedDB (IDBFactory, IDBRequest, IDBDatabase, IDBTransaction,
// IDBObjectStore, IDBIndex, IDBCursor, IDBKeyRange and DOMStringList) with its events,
// DOMException and Promise, as Go types over *js.Object. Keys, values, versions, counts,
// directions, modes and options stay *js.Object and reach IndexedDB as they are: a JS null is a
// nil *js.Object, and an absent argument is js.Undefined, never nil. A host method that can throw
// returns what it threw as a *js.Error.

package main

import "github.com/gopherjs/gopherjs/js"

// catch runs a host call, and returns what it threw.
func catch(call func()) (err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			thrown, ok := recovered.(*js.Error)
			if !ok {
				panic(recovered)
			}
			err = thrown
		}
	}()
	call()
	return nil
}

// kind is Object.prototype.toString.call(value): "[object String]", "[object Null]" and so on.
func kind(value *js.Object) string {
	return js.Global.Get("Object").Get("prototype").Get("toString").Call("call", value).String()
}

// newArray is a JS array of the values, each converted as GopherJS converts a Go value.
func newArray(values ...any) *js.Object {
	array := js.Global.Get("Array").New()
	for _, value := range values {
		array.Call("push", value)
	}
	return array
}

// elements reads a JS array, or an array-like such as a DOMStringList, into a slice.
func elements(list *js.Object) []*js.Object {
	items := make([]*js.Object, list.Length())
	for i := range items {
		items[i] = list.Index(i)
	}
	return items
}

// names reads a DOMStringList.
func names(list *js.Object) []string {
	names := make([]string, list.Length())
	for i := range names {
		names[i] = list.Index(i).String()
	}
	return names
}

// errorName is a DOMException's or a TypeError's name.
func errorName(err *js.Error) string { return err.Get("name").String() }

func newDOMException(message, name string) *js.Object {
	return js.Global.Get("DOMException").New(message, name)
}

// Promise is a host Promise. Its callbacks run as microtasks, while an IndexedDB transaction is
// still active, and they must not block.
type Promise struct{ object *js.Object }

// newPromise is new Promise(executor).
func newPromise(executor func(resolve, reject func(value *js.Object))) Promise {
	return Promise{js.Global.Get("Promise").New(func(resolve, reject *js.Object) {
		executor(func(value *js.Object) { resolve.Invoke(value) }, func(reason *js.Object) { reject.Invoke(reason) })
	})}
}

func resolved(value *js.Object) Promise {
	return Promise{js.Global.Get("Promise").Call("resolve", value)}
}

// rejected is a Promise rejected with a host error.
func rejected(err error) Promise {
	return Promise{js.Global.Get("Promise").Call("reject", err.(*js.Error).Object)}
}

// promiseAll is Promise.all of promises and plain values: it follows every promise from now on,
// and rejects with the first rejection.
func promiseAll(values ...any) Promise {
	return Promise{js.Global.Get("Promise").Call("all", newArray(values...))}
}

// allSettled is Promise.allSettled: an array of {status, value} or {status, reason}.
func allSettled(promises []Promise) Promise {
	return Promise{js.Global.Get("Promise").Call("allSettled", promiseArray(promises))}
}

func promiseArray(promises []Promise) *js.Object {
	array := js.Global.Get("Array").New()
	for _, promise := range promises {
		array.Call("push", promise.object)
	}
	return array
}

// Then calls settled once the promise settles, with its value or its rejection.
func (p Promise) Then(settled func(value *js.Object, err *js.Error)) {
	p.object.Call("then",
		func(value *js.Object) { settled(value, nil) },
		func(reason *js.Object) { settled(nil, &js.Error{Object: reason}) })
}

// Map is p.then(f): a promise of f's result.
func (p Promise) Map(f func(value *js.Object) *js.Object) Promise {
	return Promise{p.object.Call("then", f)}
}

// IgnoreRejection is p.catch(() => {}).
func (p Promise) IgnoreRejection() { p.object.Call("catch", func() {}) }

// listener is a JS event listener that can be added and later removed.
func listener(handle func()) *js.Object {
	return js.MakeFunc(func(this *js.Object, arguments []*js.Object) any {
		handle()
		return nil
	})
}

// Request is an IDBRequest.
type Request struct{ object *js.Object }

func (r Request) Result() *js.Object { return r.object.Get("result") }
func (r Request) Error() *js.Object  { return r.object.Get("error") }

// onVersionChange listens to blocked, upgradeneeded or versionchange: oldVersion, newVersion
// (nil for a deletion) and the event.
func onVersionChange(target *js.Object, name string, handle func(oldVersion float64, newVersion *float64, event *js.Object)) {
	target.Call("addEventListener", name, func(event *js.Object) {
		var newVersion *float64
		if version := event.Get("newVersion"); version != nil {
			number := version.Float()
			newVersion = &number
		}
		handle(event.Get("oldVersion").Float(), newVersion, event)
	})
}

// openDatabase is indexedDB.open(name, version); an undefined version is absent.
func openDatabase(name string, version *js.Object) (request Request, err error) {
	err = catch(func() { request = Request{js.Global.Get("indexedDB").Call("open", name, version)} })
	return request, err
}

func deleteDatabase(name string) (request Request, err error) {
	err = catch(func() { request = Request{js.Global.Get("indexedDB").Call("deleteDatabase", name)} })
	return request, err
}

// keyRange is IDBKeyRange[factory](...args), as the script writes it: only, lowerBound,
// upperBound or bound, an absent bound or flag undefined.
type keyRange struct {
	factory string
	args    []any
}

func (r keyRange) build() (built *js.Object, err error) {
	err = catch(func() { built = js.Global.Get("IDBKeyRange").Call(r.factory, r.args...) })
	return built, err
}

// call calls a host method, and returns what it threw.
func call(target *js.Object, method string, args ...any) (result *js.Object, err error) {
	err = catch(func() { result = target.Call(method, args...) })
	return result, err
}
