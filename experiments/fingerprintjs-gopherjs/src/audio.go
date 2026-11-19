package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

const (
	audioKnownForSuspending         = -1 // The browser always suspends the audio context
	audioNotSupported               = -2 // The browser doesn't support the audio context
	audioTimeout                    = -3 // An unexpected timeout has happened
	audioKnownForAntifingerprinting = -4 // The browser applies anti-fingerprinting measures
	innerErrorTimeout               = "timeout"
	innerErrorSuspended             = "suspended"
)

// getAudioFingerprint is the stabilised source. The audio signal is noised in private Safari
// 17, so audio fingerprinting is skipped there. A description of the technique:
// https://fingerprint.com/blog/audio-fingerprinting/
func getAudioFingerprint() any {
	if doesBrowserPerformAntifingerprintingOnAudio() {
		return audioKnownForAntifingerprinting
	}
	return getUnstableAudioFingerprint()
}

// getUnstableAudioFingerprint is the source without stabilisation: a number, or a function
// that finishes the rendering and returns a promise of the fingerprint.
func getUnstableAudioFingerprint() any {
	w := window()
	audioContext := or(w.Get("OfflineAudioContext"), w.Get("webkitOfflineAudioContext"))
	if !audioContext.Bool() {
		return audioNotSupported
	}

	// Some browsers keep the audio context suspended unless it is started in response to a
	// user action. They are old and unpopular, so the fingerprinting is skipped in them.
	if doesBrowserSuspendAudioContext() {
		return audioKnownForSuspending
	}

	const hashFromIndex, hashToIndex = 4500, 5000
	context := audioContext.New(1, hashToIndex, 44100)

	oscillator := context.Call("createOscillator")
	oscillator.Set("type", "triangle")
	oscillator.Get("frequency").Set("value", 10000)

	compressor := context.Call("createDynamicsCompressor")
	compressor.Get("threshold").Set("value", -50)
	compressor.Get("knee").Set("value", 40)
	compressor.Get("ratio").Set("value", 12)
	compressor.Get("attack").Set("value", 0)
	compressor.Get("release").Set("value", 0.25)

	oscillator.Call("connect", compressor)
	compressor.Call("connect", context.Get("destination"))
	oscillator.Call("start", 0)

	renderPromise, finishRendering := startRenderingAudio(context)
	// Suppresses the console error in case the fingerprint fails before it is requested
	fingerprintPromise := suppressUnhandledRejectionWarning(renderPromise.Call("then",
		func(buffer *js.Object) float64 {
			return getAudioHash(buffer.Call("getChannelData", 0).Call("subarray", hashFromIndex))
		},
		func(err *js.Object) any {
			if name := err.Get("name"); equals(name, innerErrorTimeout) || equals(name, innerErrorSuspended) {
				return audioTimeout
			}
			throw(err)
			return nil
		},
	))

	return func() *js.Object {
		finishRendering()
		return fingerprintPromise
	}
}

// doesBrowserSuspendAudioContext: mobile Safari 11 and older.
func doesBrowserSuspendAudioContext() bool {
	return isWebKit() && !isDesktopWebKit() && !isWebKit606OrNewer()
}

// doesBrowserPerformAntifingerprintingOnAudio: Safari 17+ and Samsung Internet 26+.
func doesBrowserPerformAntifingerprintingOnAudio() bool {
	return (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) ||
		(isChromium() && isSamsungInternet() && isChromium122OrNewer())
}

// startRenderingAudio starts rendering the context. Once finish is called, the rendering
// starts finishing: a running context times out, a suspended one stops retrying.
func startRenderingAudio(context *js.Object) (result *js.Object, finish func()) {
	const (
		renderTryMaxCount     = 3
		renderRetryDelay      = 500
		runningMaxAwaitTime   = 500
		runningSufficientTime = 5000
	)
	finalize := func() {}

	result = newPromise(func(resolve, reject *js.Object) {
		isFinalized := false
		renderTryCount := 0
		startedRunningAt := 0.0

		context.Set("oncomplete", func(event *js.Object) { resolve.Invoke(event.Get("renderedBuffer")) })

		startRunningTimeout := func() {
			js.Global.Call("setTimeout",
				func() { reject.Invoke(makeInnerError(innerErrorTimeout)) },
				math.Min(runningMaxAwaitTime, startedRunningAt+runningSufficientTime-dateNow()))
		}

		var tryRender func()
		tryRender = func() {
			if exception, threw := tryJS(func() {
				renderingPromise := context.Call("startRendering")
				// startRendering has a promise API and a callback API; it is checked to be a
				// promise just in case. Redundant retries after a successful rendering reject.
				if isPromise(renderingPromise) {
					suppressUnhandledRejectionWarning(renderingPromise)
				}

				switch state := context.Get("state"); {
				case equals(state, "running"):
					startedRunningAt = dateNow()
					if isFinalized {
						startRunningTimeout()
					}

				// Sometimes the context doesn't start after startRendering, e.g. in a background
				// tab on iPhone; retries usually help. Retries in the background don't count, as
				// a long fingerprint duration in the background isn't a problem.
				case equals(state, "suspended"):
					if !document().Get("hidden").Bool() {
						renderTryCount++
					}
					if isFinalized && renderTryCount >= renderTryMaxCount {
						reject.Invoke(makeInnerError(innerErrorSuspended))
					} else {
						js.Global.Call("setTimeout", tryRender, renderRetryDelay)
					}
				}
			}); threw {
				reject.Invoke(exception)
			}
		}

		tryRender()

		finalize = func() {
			if !isFinalized {
				isFinalized = true
				if startedRunningAt > 0 {
					startRunningTimeout()
				}
			}
		}
	})

	return result, func() { finalize() }
}

// getAudioHash sums the absolute samples of any array-like signal.
func getAudioHash(signal *js.Object) float64 {
	hash := 0.0
	for i := 0; i < signal.Length(); i++ {
		hash += math.Abs(toNumber(signal.Index(i)))
	}
	return hash
}

func makeInnerError(name string) *js.Object {
	err := newError(name)
	err.Set("name", name)
	return err
}
