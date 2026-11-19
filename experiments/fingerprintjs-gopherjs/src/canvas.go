package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

const (
	imageUnsupported = "unsupported"
	imageSkipped     = "skipped"
	imageUnstable    = "unstable"
)

// getCanvasFingerprint is the stabilised source. The canvas image is noised in private
// Safari 17+, and Firefox 120+ randomises canvas data in private browsing and strict ETP
// mode, so the images are skipped there.
func getCanvasFingerprint() *js.Object {
	return getUnstableCanvasFingerprint(doesBrowserPerformAntiFingerprintingOnCanvas())
}

// getUnstableCanvasFingerprint is the source without stabilisation.
func getUnstableCanvasFingerprint(skipImages bool) *js.Object {
	winding := false
	var geometry, text *js.Object

	canvas, context := makeCanvasContext()
	if !isCanvasSupported(canvas, context) {
		geometry, text = toJS(imageUnsupported), toJS(imageUnsupported)
	} else {
		winding = doesSupportWinding(context)
		if skipImages {
			geometry, text = toJS(imageSkipped), toJS(imageSkipped)
		} else {
			geometry, text = renderImages(canvas, context)
		}
	}

	return object("winding", winding, "geometry", geometry, "text", text)
}

func makeCanvasContext() (canvas, context *js.Object) {
	canvas = document().Call("createElement", "canvas")
	canvas.Set("width", 1)
	canvas.Set("height", 1)
	return canvas, canvas.Call("getContext", "2d")
}

func isCanvasSupported(canvas, context *js.Object) bool {
	return context.Bool() && canvas.Get("toDataURL").Bool()
}

// doesSupportWinding: https://github.com/Modernizr/Modernizr/blob/master/feature-detects/canvas/winding.js
func doesSupportWinding(context *js.Object) bool {
	context.Call("rect", 0, 0, 10, 10)
	context.Call("rect", 2, 2, 6, 6)
	return !context.Call("isPointInPath", 5, 5, "evenodd").Bool()
}

func renderImages(canvas, context *js.Object) (geometry, text *js.Object) {
	renderTextImage(canvas, context)
	textImage1 := canvasToString(canvas)
	textImage2 := canvasToString(canvas) // It's slightly faster to double-encode the text image

	// Some browsers add a noise to the canvas, which then is excluded from the fingerprint:
	// https://github.com/fingerprintjs/fingerprintjs/issues/791
	if textImage1 != textImage2 {
		return toJS(imageUnstable), toJS(imageUnstable)
	}

	// Text is unstable, so it is rendered into a separate image:
	// https://github.com/fingerprintjs/fingerprintjs/issues/583
	renderGeometryImage(canvas, context)
	return canvasToString(canvas), textImage1
}

func renderTextImage(canvas, context *js.Object) {
	// Resizing the canvas cleans it
	canvas.Set("width", 240)
	canvas.Set("height", 60)

	context.Set("textBaseline", "alphabetic")
	context.Set("fillStyle", "#f60")
	context.Call("fillRect", 100, 1, 62, 20)

	context.Set("fillStyle", "#069")
	// Explicit built-in fonts exclude the effect of font preferences, which are a separate
	// source. Newer emojis slow the rendering down 50-200 times, especially in Firefox, and
	// no text may follow the emoji (issue 574). The emoji is built from its code units so
	// that the script encoding can't change it (issue 66).
	printedText := "Cwm fjordbank gly " + js.Global.Get("String").Call("fromCharCode", 55357, 56835).String()
	context.Set("font", `11pt "Times New Roman"`)
	context.Call("fillText", printedText, 2, 15)
	context.Set("fillStyle", "rgba(102, 204, 0, 0.2)")
	context.Set("font", "18pt Arial")
	context.Call("fillText", printedText, 4, 45)
}

func renderGeometryImage(canvas, context *js.Object) {
	// Resizing the canvas cleans it
	canvas.Set("width", 122)
	canvas.Set("height", 110)

	// Canvas blending
	context.Set("globalCompositeOperation", "multiply")
	for _, circle := range []struct {
		color string
		x, y  float64
	}{{"#f2f", 40, 40}, {"#2ff", 80, 40}, {"#ff2", 60, 80}} {
		context.Set("fillStyle", circle.color)
		context.Call("beginPath")
		context.Call("arc", circle.x, circle.y, 40, 0, math.Pi*2, true)
		context.Call("closePath")
		context.Call("fill")
	}

	// Canvas winding
	context.Set("fillStyle", "#f9c")
	context.Call("arc", 60, 60, 60, 0, math.Pi*2, true)
	context.Call("arc", 60, 60, 20, 0, math.Pi*2, true)
	context.Call("fill", "evenodd")
}

func canvasToString(canvas *js.Object) *js.Object { return canvas.Call("toDataURL") }

// doesBrowserPerformAntiFingerprintingOnCanvas: Safari 17+ noises the canvas image in
// private mode; Firefox 120+ randomises canvas data in private browsing and strict ETP mode.
func doesBrowserPerformAntiFingerprintingOnCanvas() bool {
	isSafari17OrAbove := isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
	isFirefox120OrAbove := isGecko() && isGecko120OrNewer()
	return isSafari17OrAbove || isFirefox120OrAbove
}
