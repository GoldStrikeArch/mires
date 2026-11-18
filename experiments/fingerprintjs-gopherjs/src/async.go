package main

import "github.com/gopherjs/gopherjs/js"

// The observable asynchronous lifecycle is built on JavaScript promises, MessageChannel
// and timers, never on goroutines: a Go function called from JavaScript cannot block,
// and the GopherJS scheduler would not reproduce the oracle's task and microtask order.
//
// An oracle `async function` is ported as async(body): body runs synchronously, as the
// start of an async function does, and receives the resolve and reject functions of the
// returned promise. `await value` becomes await(value, reject, continuation), which
// settles in the same microtask as the oracle's continuation would run.

func newPromise(executor func(resolve, reject *js.Object)) *js.Object {
	return js.Global.Get("Promise").New(executor)
}

func promiseResolve(value any) *js.Object { return js.Global.Get("Promise").Call("resolve", value) }

// async runs body as the body of an async function. An exception thrown synchronously
// rejects the returned promise, like an exception before the first await.
func async(body func(resolve, reject *js.Object)) *js.Object { return newPromise(body) }

// await continues with next once value settles: a fulfilment value is passed to next,
// a rejection rejects through reject, and so does an exception thrown by next.
func await(value any, reject *js.Object, next func(result *js.Object)) {
	promiseResolve(value).Call("then", func(result *js.Object) {
		guard(reject, func() { next(result) })
	}, reject)
}

// awaitSettled is await inside a try/catch: a rejection goes to onRejected.
func awaitSettled(value any, reject *js.Object, onFulfilled, onRejected func(*js.Object)) {
	promiseResolve(value).Call("then", func(result *js.Object) {
		guard(reject, func() { onFulfilled(result) })
	}, func(err *js.Object) {
		guard(reject, func() { onRejected(err) })
	})
}

// guard routes a JavaScript exception thrown by fn to reject.
func guard(reject *js.Object, fn func()) {
	if exception, threw := tryJS(fn); threw {
		reject.Invoke(exception)
	}
}

func wait(durationMs any) *js.Object {
	return newPromise(func(resolve, _ *js.Object) {
		js.Global.Call("setTimeout", resolve, durationMs)
	})
}

// releaseEventLoop allows asynchronous actions and microtasks to happen. A MessageChannel
// is used instead of setTimeout, which browsers throttle.
func releaseEventLoop() *js.Object {
	return newPromise(func(resolve, _ *js.Object) {
		channel := js.Global.Get("MessageChannel").New()
		channel.Get("port1").Set("onmessage", func() { resolve.Invoke() })
		channel.Get("port2").Call("postMessage", nil)
	})
}

// requestIdleCallbackIfAvailable waits for an idle callback, or for the smaller timeout
// where requestIdleCallback is missing. fallbackTimeout is used as given, like the oracle's.
func requestIdleCallbackIfAvailable(fallbackTimeout *js.Object, deadlineTimeout float64) *js.Object {
	w := window()
	requestIdleCallback := w.Get("requestIdleCallback")
	if requestIdleCallback.Bool() {
		// The function loses its binding to window here, and globalThis isn't always
		// window, so it is called on window explicitly.
		return newPromise(func(resolve, _ *js.Object) {
			requestIdleCallback.Call("call", w, func() { resolve.Invoke() }, object("timeout", deadlineTimeout))
		})
	}
	return wait(mathCall("min", fallbackTimeout, deadlineTimeout))
}

func isPromise(value *js.Object) bool {
	return value.Bool() && isFunction(value.Get("then"))
}

// awaitIfAsync calls a maybe asynchronous action without creating microtasks when the
// action is synchronous, and reports the outcome to callback in both cases.
func awaitIfAsync(action func() *js.Object, callback func(ok bool, value *js.Object)) {
	exception, threw := tryJS(func() {
		returnedValue := action()
		if isPromise(returnedValue) {
			returnedValue.Call("then",
				func(result *js.Object) { callback(true, result) },
				func(err *js.Object) { callback(false, err) })
		} else {
			callback(true, returnedValue)
		}
	})
	if threw {
		callback(false, exception)
	}
}

// mapWithBreaks maps items while letting the event loop run every loopReleaseInterval
// milliseconds of Date.now() time. The callbacks for the items before the first break run
// synchronously within the call. The results are complete once done fulfils.
func mapWithBreaks[T, U any](items []T, callback func(item T, index int) U, loopReleaseInterval float64) (results []U, done *js.Object) {
	results = make([]U, len(items))
	done = async(func(resolve, reject *js.Object) {
		lastLoopReleaseTime := dateNow()
		var loop func(start int)
		loop = func(start int) {
			for i := start; i < len(items); i++ {
				results[i] = callback(items[i], i)
				now := dateNow()
				if now >= lastLoopReleaseTime+loopReleaseInterval {
					lastLoopReleaseTime = now
					next := i + 1
					await(releaseEventLoop(), reject, func(*js.Object) { loop(next) })
					return
				}
			}
			resolve.Invoke()
		}
		loop(0)
	})
	return results, done
}

// suppressUnhandledRejectionWarning keeps the promise from reporting an unhandled
// rejection while still passing the rejection on. It returns the promise.
func suppressUnhandledRejectionWarning(promise *js.Object) *js.Object {
	promise.Call("then", js.Undefined, ignore)
	return promise
}

func ignore() {}
