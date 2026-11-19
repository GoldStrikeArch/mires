package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const defaultDomPollInterval = 50

// awaitUntil awaits wait(pollInterval) until ready reports true and then calls next; like
// a `while (!ready()) await wait(pollInterval)` loop, it continues synchronously when
// ready already holds.
func awaitUntil(ready func() bool, pollInterval any, reject *js.Object, next func()) {
	if ready() {
		next()
		return
	}
	await(wait(pollInterval), reject, func(*js.Object) { awaitUntil(ready, pollInterval, reject, next) })
}

// withIframe creates an invisible iframe, calls action once the iframe has loaded and has a
// body, and removes the iframe when the action settles. The iframe lets the sources measure
// DOM sizes without being affected by the page's styles.
func withIframe(action func(iframe, iWindow *js.Object) *js.Object, initialHTML *js.Object, domPollInterval any) *js.Object {
	return async(func(resolve, reject *js.Object) {
		d := document()

		// document.body can be null while the page is loading
		awaitUntil(func() bool { return d.Get("body").Bool() }, domPollInterval, reject, func() {
			iframe := d.Call("createElement", "iframe")
			removeIframe := func() {
				if parent := iframe.Get("parentNode"); !isNullish(parent) {
					parent.Call("removeChild", iframe)
				}
			}
			// The finally block: the iframe is removed however the body below settles, and
			// an exception from the removal replaces the body's outcome.
			fail := js.MakeFunc(func(_ *js.Object, args []*js.Object) any {
				if exception, threw := tryJS(removeIframe); threw {
					reject.Invoke(exception)
				} else {
					reject.Invoke(arg(args, 0))
				}
				return js.Undefined
			})

			loaded := newPromise(func(resolveLoad, rejectLoad *js.Object) {
				isComplete := false
				complete := func() {
					isComplete = true
					resolveLoad.Invoke()
				}
				iframe.Set("onload", complete)
				iframe.Set("onerror", func(err *js.Object) {
					isComplete = true
					rejectLoad.Invoke(err)
				})

				style := iframe.Get("style")
				style.Call("setProperty", "display", "block", "important") // Required for browsers to calculate the layout
				style.Set("position", "absolute")
				style.Set("top", "0")
				style.Set("left", "0")
				style.Set("visibility", "hidden")
				if initialHTML.Bool() && has(iframe, "srcdoc") {
					iframe.Set("srcdoc", initialHTML)
				} else {
					iframe.Set("src", "about:blank")
				}
				d.Get("body").Call("appendChild", iframe)

				// WebKit in WeChat doesn't fire the iframe's onload, so the loading state is
				// also checked manually. Firefox may never reach readyState "complete" even
				// though the load event fires, hence the isComplete check.
				var checkReadyState func()
				checkReadyState = func() {
					if isComplete {
						return
					}
					if equals(contentWindowDocument(iframe).Get("readyState"), "complete") {
						complete()
					} else {
						js.Global.Call("setTimeout", checkReadyState, 10)
					}
				}
				checkReadyState()
			})

			await(loaded, fail, func(*js.Object) {
				bodyReady := func() bool { return contentWindowDocument(iframe).Get("body").Bool() }
				awaitUntil(bodyReady, domPollInterval, fail, func() {
					await(action(iframe, iframe.Get("contentWindow")), fail, func(result *js.Object) {
						// The finally block on success: one removal, whose exception rejects
						// directly rather than through fail, which would remove again.
						if exception, threw := tryJS(removeIframe); threw {
							reject.Invoke(exception)
							return
						}
						resolve.Invoke(result)
					})
				})
			})
		})
	})
}

// contentWindowDocument is iframe.contentWindow?.document, with an empty object standing
// in for a missing window or document so that the caller's property reads stay optional.
func contentWindowDocument(iframe *js.Object) *js.Object {
	contentWindow := iframe.Get("contentWindow")
	if isNullish(contentWindow) {
		return newObject()
	}
	d := contentWindow.Get("document")
	if isNullish(d) {
		return newObject()
	}
	return d
}

// selectorToElement creates a DOM element that matches a single-element CSS selector.
func selectorToElement(selector string) *js.Object {
	tag, attributes := parseSimpleCssSelector(selector)
	if tag == "" {
		tag = "div"
	}
	element := document().Call("createElement", tag)
	for _, name := range attributes.names {
		value := strings.Join(attributes.values[name], " ")
		// Changing the style attribute can cause a CSP error, so style.setProperty is used.
		if name == "style" {
			addStyleString(element.Get("style"), value)
		} else {
			element.Call("setAttribute", name, value)
		}
	}
	return element
}

// addStyleString adds CSS declarations from a string without triggering a CSP warning
// (unsafe-inline or unsafe-eval).
func addStyleString(style *js.Object, source string) {
	for _, property := range strings.Split(source, ";") {
		match := newRegExp(`^\s*([\w-]+)\s*:\s*(.+?)(\s*!([\w-]+))?\s*$`, "").Call("exec", property)
		if match != nil {
			priority := match.Index(4)
			if !priority.Bool() {
				priority = toJS("") // The last argument can't be undefined in IE11
			}
			style.Call("setProperty", match.Index(1), match.Index(2), priority)
		}
	}
}

// isAnyParentCrossOrigin reports whether the code runs in an iframe and the origin of any
// parent page differs from the current origin.
func isAnyParentCrossOrigin() bool {
	currentWindow := window()
	for {
		parentWindow := currentWindow.Get("parent")
		if !parentWindow.Bool() || parentWindow == currentWindow {
			return false // The top page is reached
		}

		var crossOrigin bool
		exception, threw := tryJS(func() {
			crossOrigin = parentWindow.Get("location").Get("origin") != currentWindow.Get("location").Get("origin")
		})
		if threw {
			// Reading location.origin of a cross-origin parent throws a SecurityError
			if isError(exception) && equals(exception.Get("name"), "SecurityError") {
				return true
			}
			throw(exception)
		}
		if crossOrigin {
			return true
		}

		currentWindow = parentWindow
	}
}
