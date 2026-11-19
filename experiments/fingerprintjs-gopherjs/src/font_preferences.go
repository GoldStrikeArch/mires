package main

import (
	"math"
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// m and w take up the maximum width; there are a couple of ligatures too.
const fontPreferenceDefaultText = "mmMwWLliI0fiflO&1"

// fontPreset is a text block to measure. The keys are random but persistent words.
type fontPreset struct {
	key    string
	styles [][2]string // CSS property and value, in order
}

var fontPresets = []fontPreset{
	// The default font. User can change it in desktop Chrome, desktop Firefox, IE 11, Android
	// Chrome (but only when the size is ≥ than the default) and Android Firefox.
	{key: "default"},
	// OS font on macOS. User can change its size and weight. Applies after Safari restart.
	{key: "apple", styles: [][2]string{{"font", "-apple-system-body"}}},
	// User can change it in desktop Chrome and desktop Firefox.
	{key: "serif", styles: [][2]string{{"fontFamily", "serif"}}},
	// User can change it in desktop Chrome and desktop Firefox.
	{key: "sans", styles: [][2]string{{"fontFamily", "sans-serif"}}},
	// User can change it in desktop Chrome and desktop Firefox.
	{key: "mono", styles: [][2]string{{"fontFamily", "monospace"}}},
	// The smallest allowed font size. User can change it in desktop Chrome, desktop Firefox
	// and desktop Safari. The height can be 0 in Chrome on a retina display.
	{key: "min", styles: [][2]string{{"fontSize", "1px"}}},
	// Tells one OS from another in desktop Chrome.
	{key: "system", styles: [][2]string{{"fontFamily", "system-ui"}}},
}

// getFontPreferences measures the width of text samples. Heights aren't included because they
// give no extra entropy and are unstable. The result is very stable in IE 11, Edge 18 and
// Safari 14; it changes with the OS pixel density in Chromium 87, and "min" and "mono" (only
// on Windows) may change when the page is zoomed in Firefox 87.
func getFontPreferences() *js.Object {
	return withNaturalFonts(func(d, container, iframeWindow *js.Object) *js.Object {
		elements := make([]*js.Object, len(fontPresets))
		sizes := newObject()

		// Create all the elements first: alternating tree modification and layout reading is
		// very slow.
		for i, preset := range fontPresets {
			element := d.Call("createElement", "span")
			element.Set("textContent", fontPreferenceDefaultText)
			element.Get("style").Set("whiteSpace", "nowrap")
			for _, style := range preset.styles {
				element.Get("style").Set(style[0], style[1])
			}
			elements[i] = element
			container.Call("append", d.Call("createElement", "br"), element)
		}

		// Chromium 128+ applies the CSS zoom to the measurement, so multiplying by the
		// devicePixelRatio reverses it. Firefox, Safari and older Chrome measure raw values.
		shouldNormalizeMeasurement := isChromium() && isChromium128OrNewer()
		for i, preset := range fontPresets {
			rawWidth := elements[i].Call("getBoundingClientRect").Get("width")
			if shouldNormalizeMeasurement {
				sizes.Set(preset.key, normalizeFontMeasurement(rawWidth.Float()*iframeWindow.Get("devicePixelRatio").Float()))
			} else {
				sizes.Set(preset.key, rawWidth)
			}
		}

		return sizes
	})
}

// normalizeFontMeasurement floors a measurement to reduce floating-point errors: to whole
// numbers on Android, for stability across devices, and to 3 decimals elsewhere.
func normalizeFontMeasurement(value float64) float64 {
	decimals := 3.0
	if isAndroid() {
		decimals = 0
	}
	pow := mathCall("pow", 10, decimals)
	return math.Floor(value*pow) / pow
}

// withNaturalFonts creates a DOM environment with the most natural font available, including
// the Android OS font, with zoom-independent measurements. The content to measure mustn't be
// absolutely positioned.
//
// Android Chrome applies the system font size to text in an iframe when the iframe isn't
// display: none, the text isn't absolutely positioned, the text block is wide enough
// (2560px on some devices in portrait orientation for the biggest font size), there is
// enough text for a few lines, and the text has text-size-adjust: none (otherwise it scales
// in "Desktop site" mode). Android Firefox does when the iframe document has a
// width=device-width viewport meta, which only srcdoc can set, and the content loads before
// more content is added with JavaScript.
//
// Measured zoom effects: Safari 11-14 with zoom reset are 100% reliable with offsetWidth and
// getBoundingClientRect; Chrome 42-87 with zoom 1/devicePixelRatio are reliable with
// getBoundingClientRect but fluctuate 1px with offsetWidth; Firefox 52-84 widths are
// reliable; IE 11 and Edge 18 are reliable without zoom.
func withNaturalFonts(action func(d, container, iframeWindow *js.Object) *js.Object) *js.Object {
	const containerWidthPx = 4000
	return withIframe(func(_, iframeWindow *js.Object) *js.Object {
		iframeDocument := iframeWindow.Get("document")
		iframeBody := iframeDocument.Get("body")

		bodyStyle := iframeBody.Get("style")
		bodyStyle.Set("width", jsString(containerWidthPx)+"px")
		bodyStyle.Set("textSizeAdjust", "none")
		bodyStyle.Set("webkitTextSizeAdjust", "none")

		if isChromium() {
			iframeBody.Get("style").Set("zoom", jsString(1/iframeWindow.Get("devicePixelRatio").Float()))
		} else if isWebKit() {
			iframeBody.Get("style").Set("zoom", "reset")
		}

		linesOfText := iframeDocument.Call("createElement", "div")
		words := make([]string, containerWidthPx/20)
		for i := range words {
			words[i] = "word"
		}
		linesOfText.Set("textContent", strings.Join(words, " "))
		iframeBody.Call("appendChild", linesOfText)

		return action(iframeDocument, iframeBody, iframeWindow)
	}, toJS(`<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1">`), defaultDomPollInterval)
}
