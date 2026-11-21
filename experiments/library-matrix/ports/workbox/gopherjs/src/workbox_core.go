// workbox-core 7.4.1: WorkboxError and its production messages, copyResponse,
// cacheMatchIgnoreParams, timeout and waitUntil, the ExtendableEvent a strategy runs under, and the
// options a route handler receives.

package main

import "github.com/gopherjs/gopherjs/js"

// WorkboxError is a `new WorkboxError(code, details)` of the production build: a JS Error whose
// own name is the code and own details the details object. Any other thrown value, from a host
// API or a plugin, is a *js.Error, which holds the value as it was thrown.
type WorkboxError struct{ object *js.Object }

func (e *WorkboxError) Error() string { return e.object.Get("message").String() }

// newWorkboxError: the production messageGenerator prints the argument list as JSON, so absent
// details (nil) print as [null], and a nested error prints its own enumerable properties.
func newWorkboxError(code string, details *js.Object) error {
	if details == nil {
		details = js.Undefined
	}
	object := js.Global.Get("Error").New(code + " :: " + stringify(newArray(details)))
	object.Set("name", code)
	object.Set("details", details)
	return &WorkboxError{object}
}

// jsValue is the JS value of a thrown error, as a plugin, an event and an error's details see it.
func jsValue(err error) *js.Object {
	switch err := err.(type) {
	case *WorkboxError:
		return err.object
	case *js.Error:
		return err.Object
	}
	return js.Global.Get("Error").New(err.Error())
}

// isError is `error instanceof Error`: Workbox tests it before it keeps or reports a caught value,
// because a plugin may throw anything.
func isError(err error) bool {
	_, workbox := err.(*WorkboxError)
	return workbox || instanceOfError(jsValue(err))
}

// kept is the error Workbox keeps from a catch: only an Error.
func kept(err error) error {
	if err != nil && isError(err) {
		return err
	}
	return nil
}

// ExtendableEvent is the event a strategy, install or activate runs under: a native event, or the
// stand-in a page supplies.
type ExtendableEvent interface {
	Type() string
	WaitUntil(promise *js.Object)
}

// waitUntil is waitUntil(event, asyncFn): the started task extends the event's lifetime, rejecting
// where Workbox's promise would, and is returned.
func waitUntil[T any](event ExtendableEvent, asyncFn func() *Task[T]) *Task[T] {
	returnPromise := asyncFn()
	event.WaitUntil(returnPromise.lifetime())
	return returnPromise
}

// Params are the route parameters a handler receives: a PrecacheRoute match ({cacheKey,
// integrity}), or the {cacheKey} that install and createHandlerBoundToURL pass. Integrity is nil
// when it is absent.
type Params struct {
	CacheKey  string
	Integrity *string
}

// HandlerOptions is what a route handler receives (HandlerCallbackOptions). No handler in the
// subset reads its url member, so it is left out.
type HandlerOptions struct {
	Request Request
	Event   ExtendableEvent
	Params  *Params
}

// copyResponse refuses a response that is not from this origin (a URL-less one has origin null),
// then builds a new Response from a clone's headers, status, status text and body.
// canConstructResponseFromBodyStream() always answers false in 7.4.1, so the body is a Blob.
func copyResponse(response Response) *Task[Response] {
	return start(func(settle func(Response, error)) {
		var origin *string
		if url := response.URL(); url != "" {
			parsed, err := parseURL(url, "")
			if err != nil {
				settle(Response{}, err)
				return
			}
			value := parsed.Origin()
			origin = &value
		}
		if origin == nil || *origin != locationOrigin() {
			var details any
			if origin != nil {
				details = *origin
			}
			settle(Response{}, newWorkboxError("cross-origin-copy-response", newObject("origin", details)))
			return
		}
		clonedResponse := response.Clone()
		headers := js.Global.Get("Headers").New(clonedResponse.Headers())
		status, statusText := clonedResponse.Status(), clonedResponse.StatusText()
		clonedResponse.Blob().then(func(body *js.Object, err error) {
			if err != nil {
				settle(Response{}, err)
				return
			}
			copied, err := copiedResponse(body, headers, status, statusText)
			settle(copied, err)
		})
	})
}

func stripParams(fullURL string, ignoreParams []string) (string, error) {
	strippedURL, err := parseURL(fullURL, "")
	if err != nil {
		return "", err
	}
	for _, param := range ignoreParams {
		strippedURL.DeleteSearchParam(param)
	}
	return strippedURL.Href(), nil
}

// cacheMatchIgnoreParams is an ordinary match when stripping the parameters changes nothing
// (deleting re-serialises the query), else the first key that strips to the same URL.
func cacheMatchIgnoreParams(cache Cache, request Request, ignoreParams []string) *Task[*Response] {
	strippedRequestURL, err := stripParams(request.URL(), ignoreParams)
	if err != nil {
		return failed[*Response](err)
	}
	if request.URL() == strippedRequestURL {
		return cache.Match(request.object)
	}
	return start(func(settle func(*Response, error)) {
		cache.KeysIgnoringSearch(request).then(func(cacheKeys []Request, err error) {
			if err != nil {
				settle(nil, err)
				return
			}
			for _, cacheKey := range cacheKeys {
				stripped, err := stripParams(cacheKey.URL(), ignoreParams)
				if err != nil {
					settle(nil, err)
					return
				}
				if stripped == strippedRequestURL {
					cache.Match(cacheKey.object).then(settle)
					return
				}
			}
			settle(nil, nil)
		})
	})
}
