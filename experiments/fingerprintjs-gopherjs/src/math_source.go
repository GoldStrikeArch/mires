package main

import "github.com/gopherjs/gopherjs/js"

// getMathFingerprint evaluates the engine's own Math functions, whose last digits differ
// between implementations. The arguments are empirical.
// https://gitlab.torproject.org/legacy/trac/-/issues/13018
// https://bugzilla.mozilla.org/show_bug.cgi?id=531915
func getMathFingerprint() *js.Object {
	m := js.Global.Get("Math")

	// Native operations; a missing one returns 0
	native := func(name string) func(float64) float64 {
		fn := m.Get(name)
		if !fn.Bool() {
			return func(float64) float64 { return 0 }
		}
		return func(value float64) float64 { return fn.Invoke(value).Float() }
	}
	acos, acosh, asin, asinh := native("acos"), native("acosh"), native("asin"), native("asinh")
	atanh, atan, sin, sinh := native("atanh"), native("atan"), native("sin"), native("sinh")
	cos, cosh, tan, tanh := native("cos"), native("cosh"), native("tan"), native("tanh")
	exp, expm1, log1p := native("exp"), native("expm1"), native("log1p")

	// Operation polyfills, on the host's elementary functions
	call := func(name string, args ...any) float64 { return m.Call(name, args...).Float() }
	powPI := func(value float64) float64 { return call("pow", m.Get("PI"), value) }
	acoshPf := func(value float64) float64 { return call("log", value+call("sqrt", value*value-1)) }
	asinhPf := func(value float64) float64 { return call("log", value+call("sqrt", value*value+1)) }
	atanhPf := func(value float64) float64 { return call("log", (1+value)/(1-value)) / 2 }
	sinhPf := func(value float64) float64 { return call("exp", value) - 1/call("exp", value)/2 }
	coshPf := func(value float64) float64 { return (call("exp", value) + 1/call("exp", value)) / 2 }
	expm1Pf := func(value float64) float64 { return call("exp", value) - 1 }
	tanhPf := func(value float64) float64 { return (call("exp", 2*value) - 1) / (call("exp", 2*value) + 1) }
	log1pPf := func(value float64) float64 { return call("log", 1+value) }

	return object(
		"acos", acos(0.123124234234234242),
		"acosh", acosh(1e308),
		"acoshPf", acoshPf(1e154), // 1e308 will not work for the polyfill
		"asin", asin(0.123124234234234242),
		"asinh", asinh(1),
		"asinhPf", asinhPf(1),
		"atanh", atanh(0.5),
		"atanhPf", atanhPf(0.5),
		"atan", atan(0.5),
		"sin", sin(-1e300),
		"sinh", sinh(1),
		"sinhPf", sinhPf(1),
		"cos", cos(10.000000000123),
		"cosh", cosh(1),
		"coshPf", coshPf(1),
		"tan", tan(-1e300),
		"tanh", tanh(1),
		"tanhPf", tanhPf(1),
		"exp", exp(1),
		"expm1", expm1(1),
		"expm1Pf", expm1Pf(1),
		"log1p", log1p(10),
		"log1pPf", log1pPf(10),
		"powPI", powPI(-100),
	)
}
