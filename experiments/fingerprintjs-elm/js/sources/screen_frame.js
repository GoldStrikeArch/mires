import { replaceNaN, round, toFloat } from '../data.js'
import {
  exitFullscreen,
  getFullscreenElement,
  isGecko,
  isGecko143OrNewer,
  isSafariWebKit,
  isWebKit,
  isWebKit616OrNewer,
} from '../browser.js'

export const screenFrameCheckInterval = 2500
const roundingPrecision = 10

let screenFrameBackup
let screenFrameSizeTimeoutId

function watchScreenFrame() {
  if (screenFrameSizeTimeoutId !== undefined) return
  const checkScreenFrame = () => {
    const frameSize = getCurrentScreenFrame()
    if (isFrameSizeNull(frameSize)) {
      screenFrameSizeTimeoutId = setTimeout(checkScreenFrame, screenFrameCheckInterval)
    } else {
      screenFrameBackup = frameSize
      screenFrameSizeTimeoutId = undefined
    }
  }
  checkScreenFrame()
}

export function getUnstableScreenFrame() {
  watchScreenFrame()
  return async () => {
    let frameSize = getCurrentScreenFrame()
    if (isFrameSizeNull(frameSize)) {
      if (screenFrameBackup) return [...screenFrameBackup]
      if (getFullscreenElement()) {
        await exitFullscreen()
        frameSize = getCurrentScreenFrame()
      }
    }
    if (!isFrameSizeNull(frameSize)) {
      screenFrameBackup = frameSize
    }
    return frameSize
  }
}

export function getScreenFrame() {
  const isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
  const isFirefox143OrAbove = isGecko() && isGecko143OrNewer()
  if (isSafari17OrAbove || isFirefox143OrAbove) {
    return () => Promise.resolve(undefined)
  }
  const screenFrameGetter = getUnstableScreenFrame()
  return async () => {
    const frameSize = await screenFrameGetter()
    const processSize = (sideSize) => (sideSize === null ? null : round(sideSize, roundingPrecision))
    return [processSize(frameSize[0]), processSize(frameSize[1]), processSize(frameSize[2]), processSize(frameSize[3])]
  }
}

function getCurrentScreenFrame() {
  const s = screen
  return [
    replaceNaN(toFloat(s.availTop), null),
    replaceNaN(toFloat(s.width) - toFloat(s.availWidth) - replaceNaN(toFloat(s.availLeft), 0), null),
    replaceNaN(toFloat(s.height) - toFloat(s.availHeight) - replaceNaN(toFloat(s.availTop), 0), null),
    replaceNaN(toFloat(s.availLeft), null),
  ]
}

function isFrameSizeNull(frameSize) {
  for (let i = 0; i < 4; ++i) {
    if (frameSize[i]) return false
  }
  return true
}
