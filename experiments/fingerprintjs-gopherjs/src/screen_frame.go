package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// frameSize holds the screen frame sides in CSS order: top, right, bottom, left. NaN stands
// for the oracle's null, which replaceNaN produces from exactly the NaN sides.
type frameSize [4]float64

const (
	screenFrameCheckInterval = 2500
	screenFrameRounding      = 10
)

var (
	// screenFrameBackup is the last non-null frame, as the very array the getter returned:
	// like the oracle's, it is shared with that caller.
	screenFrameBackup   *js.Object
	screenFrameWatching bool // a screen frame check is scheduled
)

func (f frameSize) toJS() *js.Object {
	sides := array()
	for _, side := range f {
		if math.IsNaN(side) {
			sides.Call("push", nil)
		} else {
			sides.Call("push", side)
		}
	}
	return sides
}

func (f frameSize) isNull() bool {
	for _, side := range f {
		if side != 0 && !math.IsNaN(side) {
			return false
		}
	}
	return true
}

// watchScreenFrame starts watching the screen frame size. When a non-zero size appears it is
// saved and the watch stops; the getter returns the saved size later when the current one
// is null. The frame turns null in some cases:
// https://github.com/fingerprintjs/fingerprintjs/issues/568
func watchScreenFrame() {
	if screenFrameWatching {
		return
	}
	var checkScreenFrame func()
	checkScreenFrame = func() {
		frame := getCurrentScreenFrame()
		if frame.isNull() {
			screenFrameWatching = true
			js.Global.Call("setTimeout", checkScreenFrame, screenFrameCheckInterval)
		} else {
			screenFrameBackup = frame.toJS()
			screenFrameWatching = false
		}
	}
	checkScreenFrame()
}

// getUnstableScreenFrame is the source without stabilisation.
func getUnstableScreenFrame() func() *js.Object {
	watchScreenFrame()

	return func() *js.Object {
		return async(func(resolve, reject *js.Object) {
			frame := getCurrentScreenFrame()
			finish := func() {
				result := frame.toJS()
				if !frame.isNull() {
					screenFrameBackup = result
				}
				resolve.Invoke(result)
			}

			if frame.isNull() {
				if screenFrameBackup != nil {
					resolve.Invoke(js.Global.Get("Array").Call("from", screenFrameBackup))
					return
				}
				if getFullscreenElement().Bool() {
					// Some browsers set the screen frame to zero when programmatic fullscreen
					// is on; a non-zero frame may come back after exiting the fullscreen.
					await(exitFullscreen(), reject, func(*js.Object) {
						frame = getCurrentScreenFrame()
						finish()
					})
					return
				}
			}
			finish()
		})
	}
}

// getScreenFrame is the stabilised source. The available screen resolution sometimes
// changes a bit (1900x1440 → 1900x1439, e.g. when the macOS Dock shrinks), which rounding
// mitigates. The frame is always 0 in private Safari 17 and spoofed in Firefox 143+ private
// browsing and strict ETP mode, so the source is skipped there.
func getScreenFrame() func() *js.Object {
	isSafari17OrAbove := isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
	isFirefox143OrAbove := isGecko() && isGecko143OrNewer()
	if isSafari17OrAbove || isFirefox143OrAbove {
		return func() *js.Object { return promiseResolve(js.Undefined) }
	}

	screenFrameGetter := getUnstableScreenFrame()

	return func() *js.Object {
		return async(func(resolve, reject *js.Object) {
			await(screenFrameGetter(), reject, func(frame *js.Object) {
				processSize := func(side *js.Object) *js.Object {
					if side == nil {
						return nil
					}
					return toJS(round(side.Float(), screenFrameRounding))
				}
				resolve.Invoke(array(
					processSize(frame.Index(0)), processSize(frame.Index(1)),
					processSize(frame.Index(2)), processSize(frame.Index(3)),
				))
			})
		})
	}
}

// getCurrentScreenFrame reads the frame. Some browsers return the screen resolution as
// strings, e.g. "1200", instead of numbers. IE and Edge ≤18 lack screen.availLeft and
// screen.availTop, which then count as 0 to keep the entropy of availWidth and availHeight.
func getCurrentScreenFrame() frameSize {
	s := screen()
	zeroIfNaN := func(f float64) float64 {
		if math.IsNaN(f) {
			return 0
		}
		return f
	}
	var frame frameSize
	frame[0] = toFloat(s.Get("availTop"))
	frame[1] = toFloat(s.Get("width")) - toFloat(s.Get("availWidth")) - zeroIfNaN(toFloat(s.Get("availLeft")))
	frame[2] = toFloat(s.Get("height")) - toFloat(s.Get("availHeight")) - zeroIfNaN(toFloat(s.Get("availTop")))
	frame[3] = toFloat(s.Get("availLeft"))
	return frame
}
