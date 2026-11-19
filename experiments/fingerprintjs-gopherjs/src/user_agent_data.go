package main

import "github.com/gopherjs/gopherjs/js"

// highEntropyNotAllowed is the highEntropyStatus when a Permissions Policy blocks
// getHighEntropyValues(); a stable string so that it contributes to the hash.
const highEntropyNotAllowed = "not_allowed"

// isGreaseBrand tells the intentionally fake GREASE brands ("Not_A Brand", "Not(A:Brand",
// " Not;A Brand", ...) that browsers add to prevent API ossification.
func isGreaseBrand(brand *js.Object) bool { return regExpTest("not", "i", brand) }

// getUserAgentData collects browser and OS identity data from the User-Agent Client Hints
// API, which only Chromium-based browsers provide.
func getUserAgentData() *js.Object {
	return async(func(resolve, reject *js.Object) {
		uaData := navigator().Get("userAgentData")
		if !uaData.Bool() {
			resolve.Invoke(js.Undefined)
			return
		}

		var filteredBrands []*js.Object
		allBrands := uaData.Get("brands")
		for i := 0; i < allBrands.Length(); i++ {
			if brand := allBrands.Index(i).Get("brand"); !isGreaseBrand(brand) {
				filteredBrands = append(filteredBrands, brand)
			}
		}

		brands := array()
		for _, brand := range filteredBrands {
			if len(filteredBrands) <= 1 || !equals(brand, "Chromium") {
				brands.Call("push", brand)
			}
		}

		result := object("brands", brands, "mobile", uaData.Get("mobile"), "platform", uaData.Get("platform"))

		if !uaData.Get("getHighEntropyValues").Bool() {
			resolve.Invoke(result)
			return
		}

		onRejected := func(err *js.Object) {
			if instanceOf(err, js.Global.Get("DOMException")) && equals(err.Get("name"), "NotAllowedError") {
				result.Set("highEntropyStatus", highEntropyNotAllowed)
				resolve.Invoke(result)
				return
			}
			throw(err)
		}
		var highEntropy *js.Object
		exception, threw := tryJS(func() {
			highEntropy = uaData.Call("getHighEntropyValues",
				array("architecture", "bitness", "model", "platformVersion"))
		})
		if threw {
			onRejected(exception)
			return
		}
		awaitSettled(highEntropy, reject, func(values *js.Object) {
			// The oracle's try block covers these reads too
			if exception, threw := tryJS(func() {
				result.Set("architecture", values.Get("architecture"))
				result.Set("bitness", values.Get("bitness"))
				result.Set("model", values.Get("model"))
				result.Set("platformVersion", values.Get("platformVersion"))
			}); threw {
				onRejected(exception)
				return
			}
			resolve.Invoke(result)
		}, onRejected)
	})
}
