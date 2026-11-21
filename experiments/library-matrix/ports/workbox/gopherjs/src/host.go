// The host APIs Workbox drives: CacheStorage and Cache, Fetch (Request, Response, Headers), URL and
// its search parameters, RegExp, timers and promises, as Go types over *js.Object. Workbox is a
// layer over these, so their semantics (URL parsing and serialisation, request and response
// construction, cache matching, integrity checks, regular expressions) are the engine's own. A
// host call that throws returns the thrown value as a *js.Error; an asynchronous host call answers
// with a Task.

package main

import "github.com/gopherjs/gopherjs/js"

var (
	jsArray   = js.Global.Get("Array")
	jsObject  = js.Global.Get("Object")
	jsPromise = js.Global.Get("Promise")
)

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

// isUndefined: GopherJS reads a JS undefined as js.Undefined, and null as a nil *js.Object.
func isUndefined(value *js.Object) bool { return value == js.Undefined }

// newObject is a plain object with the members in order, each converted as GopherJS converts a Go
// value.
func newObject(members ...any) *js.Object {
	object := jsObject.New()
	for i := 0; i < len(members); i += 2 {
		object.Set(members[i].(string), members[i+1])
	}
	return object
}

// newArray is a JS array of the values.
func newArray(values ...any) *js.Object {
	array := jsArray.New()
	for _, value := range values {
		array.Call("push", value)
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

func isArray(value *js.Object) bool { return jsArray.Call("isArray", value).Bool() }

// stringOf is String(value).
func stringOf(value *js.Object) string { return js.Global.Call("String", value).String() }

func stringify(value *js.Object) string {
	return js.Global.Get("JSON").Call("stringify", value).String()
}

func locationHref() string   { return js.Global.Get("location").Get("href").String() }
func locationOrigin() string { return js.Global.Get("location").Get("origin").String() }

// instanceOfError is `value instanceof Error`.
func instanceOfError(value *js.Object) bool {
	return js.Global.Get("Error").Get("prototype").Call("isPrototypeOf", value).Bool()
}

func newHostError(class, message string) error {
	return &js.Error{Object: js.Global.Get(class).New(message)}
}

// Task is a started asynchronous function: its outcome, and a host promise that fulfils once the
// outcome is set. Every continuation runs as a reaction of that promise, a microtask, as the
// code after an `await` does. The promise never rejects; lifetime gives the one that does.
type Task[T any] struct {
	promise *js.Object
	value   T
	err     error
}

// start runs body as the body of an async function: synchronously, up to its first wait. body
// settles the task once.
func start[T any](body func(settle func(T, error))) *Task[T] {
	task := &Task[T]{}
	task.promise = jsPromise.New(func(resolve *js.Object) {
		body(func(value T, err error) {
			task.value, task.err = value, err
			resolve.Invoke()
		})
	})
	return task
}

func resolved[T any](value T) *Task[T] {
	return start(func(settle func(T, error)) { settle(value, nil) })
}

func failed[T any](err error) *Task[T] {
	return start(func(settle func(T, error)) {
		var zero T
		settle(zero, err)
	})
}

// then continues with the task's outcome once it settles. A body that threw before it settled
// the task (a defect) continues with what it threw.
func (task *Task[T]) then(next func(T, error)) {
	task.promise.Call("then",
		func() { next(task.value, task.err) },
		func(reason *js.Object) {
			var zero T
			next(zero, &js.Error{Object: reason})
		})
}

// lifetime is the host promise event.waitUntil receives: it fulfils, or rejects with the thrown
// JS value, as the task does.
func (task *Task[T]) lifetime() *js.Object {
	return task.promise.Call("then", func() *js.Object {
		if task.err != nil {
			return jsPromise.Call("reject", jsValue(task.err))
		}
		return nil
	})
}

// fromHost is a host promise's outcome as a task: its value, or its rejection reason.
func fromHost[T any](promise *js.Object, convert func(*js.Object) T) *Task[T] {
	return start(func(settle func(T, error)) {
		promise.Call("then",
			func(value *js.Object) { settle(convert(value), nil) },
			func(reason *js.Object) {
				var zero T
				settle(zero, &js.Error{Object: reason})
			})
	})
}

// hostCall is fromHost for a host method that may also throw before it answers.
func hostCall[T any](call func() *js.Object, convert func(*js.Object) T) *Task[T] {
	var promise *js.Object
	if err := catch(func() { promise = call() }); err != nil {
		return failed[T](err)
	}
	return fromHost(promise, convert)
}

func asValue(value *js.Object) *js.Object { return value }
func asNothing(*js.Object) struct{}       { return struct{}{} }
func asBool(value *js.Object) bool        { return value.Bool() }

// allSettled is Promise.allSettled(promises), for its settling.
func allSettled(promises []*js.Object) *Task[struct{}] {
	return fromHost(jsPromise.Call("allSettled", newArray(toAny(promises)...)), asNothing)
}

func toAny(values []*js.Object) []any {
	items := make([]any, len(values))
	for i, value := range values {
		items[i] = value
	}
	return items
}

// deferred is a host promise with its resolver.
func deferred() (promise *js.Object, resolve func()) {
	promise = jsPromise.New(func(resolver *js.Object) { resolve = func() { resolver.Invoke(nil) } })
	return promise, resolve
}

// timeout is `new Promise(resolve => setTimeout(resolve, ms))`: one timer task.
func timeout(milliseconds int) *Task[struct{}] {
	return fromHost(jsPromise.New(func(resolve *js.Object) { js.Global.Call("setTimeout", resolve, milliseconds) }), asNothing)
}

// URL is a host URL.
type URL struct{ object *js.Object }

// parseURL is new URL(input, base), or the TypeError it throws; an empty base is absent.
func parseURL(input, base string) (url URL, err error) {
	err = catch(func() {
		if base == "" {
			url = URL{js.Global.Get("URL").New(input)}
		} else {
			url = URL{js.Global.Get("URL").New(input, base)}
		}
	})
	return url, err
}

func (u URL) Href() string             { return u.object.Get("href").String() }
func (u URL) Origin() string           { return u.object.Get("origin").String() }
func (u URL) Protocol() string         { return u.object.Get("protocol").String() }
func (u URL) Pathname() string         { return u.object.Get("pathname").String() }
func (u URL) SetPathname(path string)  { u.object.Set("pathname", path) }
func (u URL) SetHash(hash string)      { u.object.Set("hash", hash) }
func (u URL) Copy() URL                { return URL{js.Global.Get("URL").New(u.object.Get("href"))} }
func (u URL) searchParams() *js.Object { return u.object.Get("searchParams") }

// SearchParamNames is [...url.searchParams.keys()], as JS strings.
func (u URL) SearchParamNames() []*js.Object {
	return elements(jsArray.Call("from", u.searchParams().Call("keys")))
}

// SetSearchParam converts a number value with Number-to-String, as the host does.
func (u URL) SetSearchParam(name string, value *js.Object) { u.searchParams().Call("set", name, value) }

// DeleteSearchParam re-serialises the query.
func (u URL) DeleteSearchParam(name any) { u.searchParams().Call("delete", name) }

// RegExp is a host RegExp, with JavaScript semantics.
type RegExp struct{ object *js.Object }

func newRegExp(source string) (regexp RegExp, err error) {
	err = catch(func() { regexp = RegExp{js.Global.Get("RegExp").New(source)} })
	return regexp, err
}

func (r RegExp) Test(text *js.Object) bool { return r.object.Call("test", text).Bool() }

// RequestInit holds the members of a RequestInit; a nil member is left out.
type RequestInit struct {
	Method, Mode, Integrity, Cache, Credentials *string
	Headers                                     *js.Object
}

// some is a present optional string.
func some(text string) *string { return &text }

func (init RequestInit) object() *js.Object {
	object := jsObject.New()
	members := []struct {
		name  string
		value *string
	}{{"method", init.Method}, {"mode", init.Mode}, {"integrity", init.Integrity}, {"cache", init.Cache}, {"credentials", init.Credentials}}
	for _, member := range members {
		if member.value != nil {
			object.Set(member.name, *member.value)
		}
	}
	if init.Headers != nil {
		object.Set("headers", init.Headers)
	}
	return object
}

// Request is a host Request.
type Request struct{ object *js.Object }

// newRequest is new Request(input, init), or the TypeError it throws.
func newRequest(input any, init RequestInit) (request Request, err error) {
	err = catch(func() { request = Request{js.Global.Get("Request").New(input, init.object())} })
	return request, err
}

func (r Request) Clone() Request      { return Request{r.object.Call("clone")} }
func (r Request) URL() string         { return r.object.Get("url").String() }
func (r Request) Method() string      { return r.object.Get("method").String() }
func (r Request) Mode() string        { return r.object.Get("mode").String() }
func (r Request) Integrity() string   { return r.object.Get("integrity").String() }
func (r Request) Headers() *js.Object { return r.object.Get("headers") }

// asRequest is the value, when it is `instanceof Request`.
func asRequest(value *js.Object) (Request, bool) {
	if js.Global.Get("Request").Get("prototype").Call("isPrototypeOf", value).Bool() {
		return Request{value}, true
	}
	return Request{}, false
}

// Response is a host Response.
type Response struct{ object *js.Object }

// newResponse is new Response(body, {status}), or the RangeError/TypeError it throws.
func newResponse(body, status any) (response Response, err error) {
	err = catch(func() { response = Response{js.Global.Get("Response").New(body, newObject("status", status))} })
	return response, err
}

// copiedResponse is new Response(blob, {headers: new Headers(headers), status, statusText}).
func copiedResponse(blob, headers *js.Object, status int, statusText string) (response Response, err error) {
	err = catch(func() {
		response = Response{js.Global.Get("Response").New(blob, newObject("headers", headers, "status", status, "statusText", statusText))}
	})
	return response, err
}

func (r Response) Clone() Response     { return Response{r.object.Call("clone")} }
func (r Response) Status() int         { return r.object.Get("status").Int() }
func (r Response) StatusText() string  { return r.object.Get("statusText").String() }
func (r Response) URL() string         { return r.object.Get("url").String() }
func (r Response) Redirected() bool    { return r.object.Get("redirected").Bool() }
func (r Response) Type() string        { return r.object.Get("type").String() }
func (r Response) Headers() *js.Object { return r.object.Get("headers") }
func (r Response) Text() *Task[*js.Object] {
	return hostCall(func() *js.Object { return r.object.Call("text") }, asValue)
}
func (r Response) Blob() *Task[*js.Object] {
	return hostCall(func() *js.Object { return r.object.Call("blob") }, asValue)
}

func asResponse(value *js.Object) Response { return Response{value} }

// asFound is a match's answer: nil for undefined.
func asFound(value *js.Object) *Response {
	if isUndefined(value) {
		return nil
	}
	return &Response{value}
}

func asRequests(value *js.Object) []Request {
	var requests []Request
	for _, request := range elements(value) {
		requests = append(requests, Request{request})
	}
	return requests
}

func fetch(request Request) *Task[Response] {
	return hostCall(func() *js.Object { return js.Global.Call("fetch", request.object) }, asResponse)
}

// Cache is a host Cache.
type Cache struct{ object *js.Object }

func caches() *js.Object { return js.Global.Get("caches") }

func cachesOpen(name string) *Task[Cache] {
	return hostCall(func() *js.Object { return caches().Call("open", name) }, func(cache *js.Object) Cache { return Cache{cache} })
}

// cachesMatch is caches.match(request, {cacheName}), which opens no cache.
func cachesMatch(request Request, cacheName string) *Task[*Response] {
	return hostCall(func() *js.Object { return caches().Call("match", request.object, newObject("cacheName", cacheName)) }, asFound)
}

func cachesKeys() *Task[[]string] {
	return hostCall(func() *js.Object { return caches().Call("keys") }, func(names *js.Object) []string {
		var out []string
		for _, name := range elements(names) {
			out = append(out, name.String())
		}
		return out
	})
}

func cachesDelete(name string) *Task[bool] {
	return hostCall(func() *js.Object { return caches().Call("delete", name) }, asBool)
}

func (c Cache) Match(request any) *Task[*Response] {
	return hostCall(func() *js.Object { return c.object.Call("match", request) }, asFound)
}

func (c Cache) Put(request Request, response Response) *Task[struct{}] {
	return hostCall(func() *js.Object { return c.object.Call("put", request.object, response.object) }, asNothing)
}

func (c Cache) Delete(request Request) *Task[bool] {
	return hostCall(func() *js.Object { return c.object.Call("delete", request.object) }, asBool)
}

func (c Cache) Keys() *Task[[]Request] {
	return hostCall(func() *js.Object { return c.object.Call("keys") }, asRequests)
}

// KeysIgnoringSearch is cache.keys(request, {ignoreSearch: true}).
func (c Cache) KeysIgnoringSearch(request Request) *Task[[]Request] {
	return hostCall(func() *js.Object { return c.object.Call("keys", request.object, newObject("ignoreSearch", true)) }, asRequests)
}
