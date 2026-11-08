open Bindings
open Browser
open Async

let doesBrowserSuspendAudioContext = () => isWebKit() && !isDesktopWebKit() && !isWebKit606OrNewer()

let doesBrowserPerformAntifingerprinting = () =>
  isWebKit() && isWebKit616OrNewer() && isSafariWebKit() ||
    isChromium() && isSamsungInternet() && isChromium122OrNewer()

let makeInnerError = name => {
  let error = newError(name)
  set(error, "name", name)
  error
}

let getHash = (signal: 'a) => {
  let hash = ref(0.)
  let length = u8Length(signal)
  let rec loop = i =>
    if i >= length {
      hash.contents
    } else {
      hash := hash.contents +. mathAbs(asFloat(jsIndex(signal, i)))
      loop(i + 1)
    }
  loop(0)
}

let startRenderingAudio = context => {
  let renderTryMaxCount = 3
  let renderRetryDelay = 500
  let runningMaxAwaitTime = 500.
  let runningSufficientTime = 5000.
  let finalize = ref(() => ())
  let resultPromise = jsPromise((resolve, reject) => {
    let isFinalized = ref(false)
    let renderTryCount = ref(0)
    let startedRunningAt = ref(0.)
    set(context, "oncomplete", event => apply1(resolve, get(event, "renderedBuffer")))
    let startRunningTimeout = () => {
      setTimeout(() => apply1(reject, makeInnerError("timeout")), asInt(mathMin(runningMaxAwaitTime, startedRunningAt.contents +. runningSufficientTime -. dateNow())))->ignore
    }
    let rec tryRender = () => {
      let outcome = runAction(() => {
        let renderingPromise = call0(context, "startRendering")
        if isPromise(renderingPromise) {
          suppressUnhandledRejectionWarning(renderingPromise)->ignore
        }
        let state = asString(get(context, "state"))
        if state == "running" {
          startedRunningAt := dateNow()
          if isFinalized.contents {
            startRunningTimeout()
          }
        } else if state == "suspended" {
          if !asBool(get(document(), "hidden")) {
            renderTryCount := renderTryCount.contents + 1
          }
          if isFinalized.contents && renderTryCount.contents >= renderTryMaxCount {
            apply1(reject, makeInnerError("suspended"))->ignore
          } else {
            setTimeout(tryRender, renderRetryDelay)->ignore
          }
        }
      })
      if !asBool(getInt(outcome, 0)) {
        apply1(reject, getInt(outcome, 1))->ignore
      }
    }
    tryRender()
    finalize :=
      () =>
        if !isFinalized.contents {
          isFinalized := true
          if startedRunningAt.contents > 0. {
            startRunningTimeout()
          }
        }
  })
  (resultPromise, finalize)
}

let getUnstableAudioFingerprint = () => {
  let w = window()
  let audioContext = orElse(get(w, "OfflineAudioContext"), get(w, "webkitOfflineAudioContext"))
  if isFalsy(audioContext) {
    identity(-2)
  } else if doesBrowserSuspendAudioContext() {
    identity(-1)
  } else {
    let hashFromIndex = 4500
    let hashToIndex = 5000
    let context = construct3(audioContext, 1, hashToIndex, 44100)
    let oscillator = call0(context, "createOscillator")
    set(oscillator, "type", "triangle")
    set(get(oscillator, "frequency"), "value", 10000)
    let compressor = call0(context, "createDynamicsCompressor")
    set(get(compressor, "threshold"), "value", -50)
    set(get(compressor, "knee"), "value", 40)
    set(get(compressor, "ratio"), "value", 12)
    set(get(compressor, "attack"), "value", 0)
    set(get(compressor, "release"), "value", 0.25)
    call1(oscillator, "connect", compressor)->ignore
    call1(compressor, "connect", get(context, "destination"))->ignore
    call1(oscillator, "start", 0)->ignore
    let (renderPromise, finishRendering) = startRenderingAudio(context)
    let fingerprintPromise = suppressUnhandledRejectionWarning(
      then_(
        renderPromise,
        buffer => getHash(call1(call1(buffer, "getChannelData", 0), "subarray", hashFromIndex)),
        error =>
          if errorName(error) == "timeout" || errorName(error) == "suspended" {
            -3
          } else {
            throwJs(error)
          },
      ),
    )
    identity(() => {
      apply0(finishRendering.contents)->ignore
      fingerprintPromise
    })
  }
}

let getAudioFingerprint = () =>
  if doesBrowserPerformAntifingerprinting() {
    identity(-4)
  } else {
    getUnstableAudioFingerprint()
  }
