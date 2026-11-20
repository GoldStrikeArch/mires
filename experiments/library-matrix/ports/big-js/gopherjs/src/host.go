package main

import "github.com/gopherjs/gopherjs/js"

// The JS number conversions upstream relies on, from the host: String(n) for printing exponents
// and estimates, Number(s) for parsing them (NaN where s is not a number), and toExponential().
func jsString(n float64) string { return js.Global.Call("String", n).String() }

func jsNumber(s string) float64 { return js.Global.Call("Number", s).Float() }

func toExponential(n float64) string { return js.InternalObject(n).Call("toExponential").String() }
