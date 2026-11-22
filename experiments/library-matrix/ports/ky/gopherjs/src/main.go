// Package main is Ky 2.2.0 in Go, compiled to JavaScript by GopherJS: a port of the subset that
// libraries/ky/CONTRACT.md defines, and of what libraries/ky/reference.mjs does with it.
//
//   - ky.go: Ky's request, retry loop, option checks, error data and errors;
//   - retrytiming.go: core/retry-timing.js, the server-provided retry delays;
//   - host.go: the Fetch, abort, decoding and JS globals Ky drives;
//   - this file: the reference adapter (each case's transport, hooks and trace) and the export.
//
// The boundary: each corpus case is a *js.Object read in place, field by field. The option values
// Ky validates stay the JS values they are until Ky's own checks read them, and each result is the
// JS array [trace, outcome]. run returns a Promise that a goroutine settles; it is published on
// js.Module.exports, which the builder's facade binds and re-exports.
package main

import (
	"strconv"

	"github.com/gopherjs/gopherjs/js"
)

// cancel aborts the adapter's controller, as reference.mjs does.
func cancel(controller AbortController) {
	reason := js.Global.Get("Error").New("cancelled")
	reason.Set("name", "AbortError")
	controller.AbortWith(reason)
}

func requestCase(c *js.Object) *js.Object {
	trace := jsArray.New()
	record := func(entry ...interface{}) { trace.Call("push", newArray(entry...)) }
	abort := c.Get("abort").String()
	controller := newAbortController()
	if abort == "before" {
		cancel(controller)
	}
	replies := elements(c.Get("replies"))
	sent := 0
	// The injected transport: it records what it was sent and answers with the next reply.
	fetch := func(request Request) (Response, error) {
		body, err := request.Text()
		if err != nil {
			return Response{}, err
		}
		headers := request.Headers()
		record("fetch", request.Method(), request.URL(), headers.GetRaw("accept"),
			headers.GetRaw("content-type"), headers.GetRaw("x-matrix"), body)
		if abort == "fetch" {
			cancel(controller)
		}
		signal := request.Signal()
		if err := signal.ThrowIfAborted(); err != nil {
			return Response{}, err
		}
		reply := replies[min(sent, len(replies)-1)]
		sent++
		switch reply.Get("kind").String() {
		case "network":
			return Response{}, newError("TypeError", "Failed to fetch")
		case "stall":
			aborted := make(chan *js.Object, 1)
			signal.OnAbort(func() { aborted <- signal.Reason() })
			return Response{}, &js.Error{Object: <-aborted}
		}
		return newResponse(reply.Get("body"), reply.Get("status"), reply.Get("headers"))
	}
	delay := func(attempt int) float64 {
		milliseconds := 300 * js.Global.Get("Math").Call("pow", 2, attempt-1).Float()
		record("backoff", attempt, milliseconds)
		return milliseconds
	}
	hooks := Hooks{
		Init: []func(){func() { record("init") }},
		BeforeRequest: []func(Request, int){func(request Request, retryCount int) {
			record("beforeRequest", retryCount)
			request.Headers().Set("x-matrix", "initial")
		}},
		AfterResponse: []func(Request, Response, int){func(_ Request, response Response, retryCount int) {
			record("afterResponse", retryCount, response.Status())
		}},
		BeforeRetry: []func(Request, error, int){func(request Request, err error, retryCount int) {
			record("beforeRetry", retryCount, errorName(err))
			request.Headers().Set("x-matrix", "retry-"+strconv.Itoa(retryCount))
			if abort == "retry" {
				cancel(controller)
			}
		}},
		BeforeError: []func(Request, error, int){func(_ Request, err error, retryCount int) {
			record("beforeError", retryCount, errorName(err))
		}},
	}
	signal := controller.Signal()
	options := Options{
		Method:  c.Get("method"),
		Headers: c.Get("headers"),
		JSON:    c.Get("json"),
		Retry: RetryOptions{
			Limit:          c.Get("limit"),
			Methods:        c.Get("methods"),
			StatusCodes:    c.Get("statusCodes"),
			MaxRetryAfter:  c.Get("maxRetryAfter"),
			BackoffLimit:   c.Get("backoffLimit"),
			RetryOnTimeout: c.Get("retryOnTimeout"),
			Delay:          delay,
		},
		Timeout:         c.Get("timeout"),
		ThrowHTTPErrors: c.Get("throwHttpErrors"),
		Hooks:           hooks,
		Fetch:           fetch,
		Signal:          &signal,
	}
	shortcut := TextShortcut
	if c.Get("read").String() == "json" {
		shortcut = JSONShortcut
	}
	var outcome *js.Object
	switch result, err := send(c.Get("url"), options, shortcut); err := err.(type) {
	case nil:
		outcome = newArray("ok", result)
	case *HTTPError:
		outcome = newArray("error", "HTTPError", err.Response.Status(), err.Data)
	default:
		outcome = newArray("error", errorName(err), nil, nil)
	}
	return newArray(trace, outcome)
}

func run(input *js.Object) *js.Object {
	return js.Global.Get("Promise").New(func(resolve, reject *js.Object) {
		go func() {
			results := jsArray.New()
			for _, c := range elements(input.Get("cases")) {
				results.Call("push", requestCase(c))
			}
			resolve.Invoke(results)
		}()
	})
}

func main() { js.Module.Get("exports").Set("run", run) }
