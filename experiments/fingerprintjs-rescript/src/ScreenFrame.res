open Bindings
open Data
open Browser
open Async

let screenFrameCheckInterval = 2500
let roundingPrecision = 10.
let screenFrameBackup = ref(jsUndefined)
let screenFrameSizeTimeoutId = ref(jsUndefined)

let getCurrentScreenFrame = () => {
  let s = screen()
  [
    replaceNaN(toFloat(get(s, "availTop")), jsNull),
    replaceNaN(
      toFloat(get(s, "width")) -.
      toFloat(get(s, "availWidth")) -.
      asFloat(replaceNaN(toFloat(get(s, "availLeft")), 0)),
      jsNull,
    ),
    replaceNaN(
      toFloat(get(s, "height")) -.
      toFloat(get(s, "availHeight")) -.
      asFloat(replaceNaN(toFloat(get(s, "availTop")), 0)),
      jsNull,
    ),
    replaceNaN(toFloat(get(s, "availLeft")), jsNull),
  ]
}

let isFrameSizeNull = (frameSize: array<any>) => {
  let rec loop = i =>
    if i >= 4 {
      true
    } else if isTruthy(getInt(frameSize, i)) {
      false
    } else {
      loop(i + 1)
    }
  loop(0)
}

let watchScreenFrame = () => {
  if !isUndefined(screenFrameSizeTimeoutId.contents) {
    ()
  } else {
    let rec checkScreenFrame = () => {
      let frameSize = getCurrentScreenFrame()
      if isFrameSizeNull(frameSize) {
        screenFrameSizeTimeoutId := setTimeout(checkScreenFrame, screenFrameCheckInterval)
      } else {
        screenFrameBackup := identity(frameSize)
        screenFrameSizeTimeoutId := jsUndefined
      }
    }
    checkScreenFrame()
  }
}

let resetScreenFrameWatch = () => {
  if !isUndefined(screenFrameSizeTimeoutId.contents) {
    clearTimeout(screenFrameSizeTimeoutId.contents)
    screenFrameSizeTimeoutId := jsUndefined
  }
  screenFrameBackup := jsUndefined
}

let getUnstableScreenFrame = () => {
  watchScreenFrame()
  async () => {
    let frameSize = ref(getCurrentScreenFrame())
    if isFrameSizeNull(frameSize.contents) {
      if isTruthy(screenFrameBackup.contents) {
        asArray(screenFrameBackup.contents)->arraySlice(0)
      } else if isTruthy(getFullscreenElement()) {
        let _ = await asPromise(exitFullscreen())
        frameSize := getCurrentScreenFrame()
        if !isFrameSizeNull(frameSize.contents) {
          screenFrameBackup := identity(frameSize.contents)
        }
        frameSize.contents
      } else {
        if !isFrameSizeNull(frameSize.contents) {
          screenFrameBackup := identity(frameSize.contents)
        }
        frameSize.contents
      }
    } else {
      screenFrameBackup := identity(frameSize.contents)
      frameSize.contents
    }
  }
}

let getScreenFrame = () => {
  let isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
  let isFirefox143OrAbove = isGecko() && isGecko143OrNewer()
  if isSafari17OrAbove || isFirefox143OrAbove {
    () => Promise.resolve(jsUndefined)
  } else {
    let screenFrameGetter = getUnstableScreenFrame()
    async () => {
      let frameSize = await screenFrameGetter()
      let processSize = sideSize =>
        if strictEq(sideSize, jsNull) {
          jsNull
        } else {
          identity(round(asFloat(sideSize), ~base=roundingPrecision))
        }
      identity([
        processSize(getInt(asArray(frameSize), 0)),
        processSize(getInt(asArray(frameSize), 1)),
        processSize(getInt(asArray(frameSize), 2)),
        processSize(getInt(asArray(frameSize), 3)),
      ])
    }
  }
}
