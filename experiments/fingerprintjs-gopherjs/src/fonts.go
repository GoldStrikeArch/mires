package main

import "github.com/gopherjs/gopherjs/js"

// m and w take up the maximum width, and LLi separates otherwise matching fonts.
const fontTestString = "mmMwWLliI0O&1"

// Any font size works for the test; the larger the better.
const fontTextSize = "48px"

// A font is compared against the three default fonts; it is available when it differs from
// any of them.
var baseFonts = []string{"monospace", "sans-serif", "serif"}

var fontList = []string{
	// This is android-specific font from "Roboto" family
	"sans-serif-thin",
	"ARNO PRO",
	"Agency FB",
	"Arabic Typesetting",
	"Arial Unicode MS",
	"AvantGarde Bk BT",
	"BankGothic Md BT",
	"Batang",
	"Bitstream Vera Sans Mono",
	"Calibri",
	"Century",
	"Century Gothic",
	"Clarendon",
	"EUROSTILE",
	"Franklin Gothic",
	"Futura Bk BT",
	"Futura Md BT",
	"GOTHAM",
	"Gill Sans",
	"HELV",
	"Haettenschweiler",
	"Helvetica Neue",
	"Humanst521 BT",
	"Leelawadee",
	"Letter Gothic",
	"Levenim MT",
	"Lucida Bright",
	"Lucida Sans",
	"Menlo",
	"MS Mincho",
	"MS Outlook",
	"MS Reference Specialty",
	"MS UI Gothic",
	"MT Extra",
	"MYRIAD PRO",
	"Marlett",
	"Meiryo UI",
	"Microsoft Uighur",
	"Minion Pro",
	"Monotype Corsiva",
	"PMingLiU",
	"Pristina",
	"SCRIPTINA",
	"Segoe UI Light",
	"Serifa",
	"SimHei",
	"Small Fonts",
	"Staccato222 BT",
	"TRAJAN PRO",
	"Univers CE 55 Medium",
	"Vrinda",
	"ZWAdobeF",
}

// getFonts detects the available fonts by the size of text rendered with them. It runs in an
// iframe so that it neither affects the page nor is affected by the page's CSS (issues 592
// and 628). Kudos to http://www.lalit.org/lab/javascript-css-font-detect/
func getFonts() *js.Object {
	return withIframe(func(_, iWindow *js.Object) *js.Object {
		d := iWindow.Get("document")
		holder := d.Get("body")
		holder.Get("style").Set("fontSize", fontTextSize)

		// div to load spans for the default fonts and the fonts to detect
		spansContainer := d.Call("createElement", "div")
		spansContainer.Get("style").Call("setProperty", "visibility", "hidden", "important")

		// The sizes stay JavaScript values and are compared with ===
		defaultWidth := map[string]*js.Object{}
		defaultHeight := map[string]*js.Object{}

		// createSpan creates a span where the fonts will be loaded
		createSpan := func(fontFamily string) *js.Object {
			span := d.Call("createElement", "span")
			style := span.Get("style")
			style.Set("position", "absolute")
			style.Set("top", "0")
			style.Set("left", "0")
			style.Set("fontFamily", fontFamily)
			span.Set("textContent", fontTestString)
			spansContainer.Call("appendChild", span)
			return span
		}

		// A span for each font to detect, with each base font as its fallback
		createSpanWithFonts := func(fontToDetect, baseFont string) *js.Object {
			return createSpan("'" + fontToDetect + "'," + baseFont)
		}

		isFontAvailable := func(fontSpans []*js.Object) bool {
			for baseFontIndex, baseFont := range baseFonts {
				span := fontSpans[baseFontIndex]
				if span.Get("offsetWidth") != defaultWidth[baseFont] ||
					span.Get("offsetHeight") != defaultHeight[baseFont] {
					return true
				}
			}
			return false
		}

		// create spans for base fonts
		baseFontsSpans := make([]*js.Object, len(baseFonts))
		for i, baseFont := range baseFonts {
			baseFontsSpans[i] = createSpan(baseFont)
		}

		// create spans for fonts to detect
		fontsSpans := make(map[string][]*js.Object, len(fontList))
		for _, font := range fontList {
			spans := make([]*js.Object, len(baseFonts))
			for i, baseFont := range baseFonts {
				spans[i] = createSpanWithFonts(font, baseFont)
			}
			fontsSpans[font] = spans
		}

		// add all the spans to the DOM
		holder.Call("appendChild", spansContainer)

		// get the default width and height for the three base fonts
		for index, baseFont := range baseFonts {
			defaultWidth[baseFont] = baseFontsSpans[index].Get("offsetWidth")
			defaultHeight[baseFont] = baseFontsSpans[index].Get("offsetHeight")
		}

		// check available fonts
		var available []string
		for _, font := range fontList {
			if isFontAvailable(fontsSpans[font]) {
				available = append(available, font)
			}
		}
		return stringArray(available)
	}, js.Undefined, defaultDomPollInterval)
}
