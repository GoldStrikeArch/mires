// The host APIs Ky drives: Fetch (Headers, Request, Response), AbortController and AbortSignal,
// TextDecoder and the JS globals Ky calls, wrapped as Go types over *js.Object. Ky is a thin layer
// over these, so their semantics (method and URL normalisation, header validation, body encoding,
// encoding labels) are the engine's own. A host call that throws returns the thrown value as a
// *js.Error; awaiting a promise blocks the calling goroutine until the promise settles.

package main

import "github.com/gopherjs/gopherjs/js"

var (
	jsArray        = js.Global.Get("Array")
	jsObject       = js.Global.Get("Object")
	jsNumber       = js.Global.Get("Number")
	jsMap          = js.Global.Get("Map")
	jsSet          = js.Global.Get("Set")
	jsReflect      = js.Global.Get("Reflect")
	objectToString = jsObject.Get("prototype").Get("toString")
)

// typeTag is Object.prototype.toString.call(v): "[object Number]", "[object String]" and so on,
// which tells the JS types apart where typeof would.
func typeTag(v *js.Object) string { return objectToString.Call("call", v).String() }

// isUndefined: GopherJS reads a JS undefined as js.Undefined, and null as a nil *js.Object.
func isUndefined(v *js.Object) bool { return v == js.Undefined }

func isInteger(v *js.Object) bool { return jsNumber.Call("isInteger", v).Bool() }
func isArray(v *js.Object) bool   { return jsArray.Call("isArray", v).Bool() }

// newArray is a JS array of the values, each converted as GopherJS converts a Go value.
func newArray(values ...interface{}) *js.Object {
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

// jsSetOf builds a JS Set containing the given strings.
func jsSetOf(values ...string) *js.Object {
	set := jsSet.New()
	for _, value := range values {
		set.Call("add", value)
	}
	return set
}

// toNumber is Number(text).
func toNumber(text string) float64 { return jsNumber.Invoke(text).Float() }

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

// newError is a JS Error of the named class: what Ky throws from its option checks.
func newError(class, message string) error {
	return &js.Error{Object: js.Global.Get(class).New(message)}
}

// await blocks the goroutine until the promise settles, and returns its value or its rejection
// reason.
func await(promise *js.Object) (*js.Object, error) {
	type outcome struct {
		value *js.Object
		err   error
	}
	settled := make(chan outcome, 1)
	promise.Call("then",
		func(value *js.Object) { settled <- outcome{value: value} },
		func(reason *js.Object) { settled <- outcome{err: &js.Error{Object: reason}} })
	result := <-settled
	return result.value, result.err
}

// setTimeout is the host's, with Ky's own millisecond value. GopherJS's time package adds a
// millisecond to every timer, which overflows setTimeout's 32-bit range at Ky's largest delay.
// The callback runs as a JS callback, so it must not block.
func setTimeout(callback func(), delay float64) *js.Object {
	return js.Global.Call("setTimeout", callback, delay)
}

func clearTimeout(timer *js.Object) { js.Global.Call("clearTimeout", timer) }

// sleep blocks the goroutine for the delay.
func sleep(delay float64) {
	elapsed := make(chan struct{})
	setTimeout(func() { close(elapsed) }, delay)
	<-elapsed
}

type AbortSignal struct{ object *js.Object }

// anySignal is AbortSignal.any: aborted as soon as one of the signals is, with that one's reason.
func anySignal(signals ...AbortSignal) AbortSignal {
	sources := jsArray.New()
	for _, signal := range signals {
		sources.Call("push", signal.object)
	}
	return AbortSignal{js.Global.Get("AbortSignal").Call("any", sources)}
}

func (s AbortSignal) Aborted() bool       { return s.object.Get("aborted").Bool() }
func (s AbortSignal) Reason() *js.Object  { return s.object.Get("reason") }
func (s AbortSignal) ThrowIfAborted() error { return catch(func() { s.object.Call("throwIfAborted") }) }

// OnAbort calls listener once, when the signal aborts. The listener runs as a JS callback, so it
// must not block.
func (s AbortSignal) OnAbort(listener func()) {
	options := jsObject.New()
	options.Set("once", true)
	s.object.Call("addEventListener", "abort", listener, options)
}

type AbortController struct{ object *js.Object }

func newAbortController() AbortController {
	return AbortController{js.Global.Get("AbortController").New()}
}

func (c AbortController) Signal() AbortSignal { return AbortSignal{c.object.Get("signal")} }

// Abort aborts with the platform's AbortError DOMException.
func (c AbortController) Abort() { c.object.Call("abort") }

func (c AbortController) AbortWith(reason *js.Object) { c.object.Call("abort", reason) }

type Headers struct{ object *js.Object }

func newHeaders() Headers { return Headers{js.Global.Get("Headers").New()} }

// Set validates the name and the value (a TypeError) and stores the value's ByteString
// conversion, so any JSON value is accepted as JS accepts it.
func (h Headers) Set(name, value interface{}) error {
	return catch(func() { h.object.Call("set", name, value) })
}

// GetRaw returns the header value as its raw JS string (*js.Object), or nil (JS null) when absent.
func (h Headers) GetRaw(name string) *js.Object {
	return h.object.Call("get", name)
}

func (h Headers) Get(name string) (string, bool) {
	value := h.object.Call("get", name)
	if value == nil {
		return "", false
	}
	return value.String(), true
}

type Request struct{ object *js.Object }

// newRequest is new Request(input, {method, headers, body, signal}), or the TypeError it throws.
// input, method and body stay raw *js.Object JS strings so GopherJS's UTF-16->UTF-8 .String()
// never mangles lone surrogates before Fetch's WebIDL USVString/ByteString handling.
func newRequest(input, method *js.Object, headers Headers, body *js.Object, signal AbortSignal) (Request, error) {
	init := jsObject.New()
	init.Set("method", method)
	init.Set("headers", headers.object)
	if body != nil {
		init.Set("body", body)
	}
	init.Set("signal", signal.object)
	var request Request
	err := catch(func() { request = Request{js.Global.Get("Request").New(input, init)} })
	return request, err
}

// WithSignal is new Request(request, {signal}): the same request, following another signal.
func (r Request) WithSignal(signal AbortSignal) (Request, error) {
	init := jsObject.New()
	init.Set("signal", signal.object)
	var request Request
	err := catch(func() { request = Request{js.Global.Get("Request").New(r.object, init)} })
	return request, err
}

func (r Request) Clone() (Request, error) {
	var clone Request
	err := catch(func() { clone = Request{r.object.Call("clone")} })
	return clone, err
}

func (r Request) Method() *js.Object  { return r.object.Get("method") }
func (r Request) URL() *js.Object     { return r.object.Get("url") }
func (r Request) Headers() Headers    { return Headers{r.object.Get("headers")} }
func (r Request) Signal() AbortSignal { return AbortSignal{r.object.Get("signal")} }

func (r Request) Text() (*js.Object, error) {
	return await(r.object.Call("text"))
}

type Response struct{ object *js.Object }

// newResponse is new Response(body, {status, headers}), or the RangeError or TypeError it throws.
func newResponse(body, status, headers *js.Object) (Response, error) {
	init := jsObject.New()
	init.Set("status", status)
	init.Set("headers", headers)
	var response Response
	err := catch(func() { response = Response{js.Global.Get("Response").New(body, init)} })
	return response, err
}

func (r Response) Status() int       { return r.object.Get("status").Int() }
func (r Response) OK() bool          { return r.object.Get("ok").Bool() }
func (r Response) Headers() Headers  { return Headers{r.object.Get("headers")} }
func (r Response) HasBody() bool     { return r.object.Get("body") != nil }

func (r Response) Text() (*js.Object, error) {
	return await(r.object.Call("text"))
}

// Bytes is the whole body, as an ArrayBuffer and its length.
func (r Response) Bytes() (*js.Object, int, error) {
	buffer, err := await(r.object.Call("arrayBuffer"))
	if err != nil {
		return nil, 0, err
	}
	return buffer, buffer.Get("byteLength").Int(), nil
}

type TextDecoder struct{ object *js.Object }

func utf8Decoder() TextDecoder { return TextDecoder{js.Global.Get("TextDecoder").New()} }

// newTextDecoder is a decoder for an Encoding Standard label, or the RangeError for an unknown one.
func newTextDecoder(label string) (TextDecoder, error) {
	var decoder TextDecoder
	err := catch(func() { decoder = TextDecoder{js.Global.Get("TextDecoder").New(label)} })
	return decoder, err
}

func (d TextDecoder) Decode(bytes *js.Object) *js.Object { return d.object.Call("decode", bytes) }
