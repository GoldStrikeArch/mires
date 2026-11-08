import {
  isChromium,
  isChromium122OrNewer,
  isDesktopWebKit,
  isSamsungInternet,
  isSafariWebKit,
  isWebKit,
  isWebKit606OrNewer,
  isWebKit616OrNewer,
  isAndroid,
} from '../browser.js'
import { isPromise, suppressUnhandledRejectionWarning } from '../async.js'

export function getAudioFingerprint() {
  if (doesBrowserPerformAntifingerprinting()) return -4
  return getUnstableAudioFingerprint()
}

export function getUnstableAudioFingerprint() {
  const w = window
  const AudioContext = w.OfflineAudioContext || w.webkitOfflineAudioContext
  if (!AudioContext) return -2
  if (doesBrowserSuspendAudioContext()) return -1

  const hashFromIndex = 4500
  const hashToIndex = 5000
  const context = new AudioContext(1, hashToIndex, 44100)

  const oscillator = context.createOscillator()
  oscillator.type = 'triangle'
  oscillator.frequency.value = 10000

  const compressor = context.createDynamicsCompressor()
  compressor.threshold.value = -50
  compressor.knee.value = 40
  compressor.ratio.value = 12
  compressor.attack.value = 0
  compressor.release.value = 0.25

  oscillator.connect(compressor)
  compressor.connect(context.destination)
  oscillator.start(0)

  const [renderPromise, finishRendering] = startRenderingAudio(context)
  const fingerprintPromise = suppressUnhandledRejectionWarning(
    renderPromise.then(
      (buffer) => getHash(buffer.getChannelData(0).subarray(hashFromIndex)),
      (error) => {
        if (error.name === 'timeout' || error.name === 'suspended') return -3
        throw error
      },
    ),
  )

  return () => {
    finishRendering()
    return fingerprintPromise
  }
}

function doesBrowserSuspendAudioContext() {
  return isWebKit() && !isDesktopWebKit() && !isWebKit606OrNewer()
}

function doesBrowserPerformAntifingerprinting() {
  return (
    (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) ||
    (isChromium() && isSamsungInternet() && isChromium122OrNewer())
  )
}

function startRenderingAudio(context) {
  const renderTryMaxCount = 3
  const renderRetryDelay = 500
  const runningMaxAwaitTime = 500
  const runningSufficientTime = 5000
  let finalize = () => undefined

  const resultPromise = new Promise((resolve, reject) => {
    let isFinalized = false
    let renderTryCount = 0
    let startedRunningAt = 0

    context.oncomplete = (event) => resolve(event.renderedBuffer)

    const startRunningTimeout = () => {
      setTimeout(
        () => reject(makeInnerError('timeout')),
        Math.min(runningMaxAwaitTime, startedRunningAt + runningSufficientTime - Date.now()),
      )
    }

    const tryRender = () => {
      try {
        const renderingPromise = context.startRendering()
        if (isPromise(renderingPromise)) {
          suppressUnhandledRejectionWarning(renderingPromise)
        }
        switch (context.state) {
          case 'running':
            startedRunningAt = Date.now()
            if (isFinalized) startRunningTimeout()
            break
          case 'suspended':
            if (!document.hidden) renderTryCount++
            if (isFinalized && renderTryCount >= renderTryMaxCount) {
              reject(makeInnerError('suspended'))
            } else {
              setTimeout(tryRender, renderRetryDelay)
            }
            break
        }
      } catch (error) {
        reject(error)
      }
    }

    tryRender()

    finalize = () => {
      if (!isFinalized) {
        isFinalized = true
        if (startedRunningAt > 0) startRunningTimeout()
      }
    }
  })

  return [resultPromise, finalize]
}

function getHash(signal) {
  let hash = 0
  for (let i = 0; i < signal.length; ++i) hash += Math.abs(signal[i])
  return hash
}

function makeInnerError(name) {
  const error = new Error(name)
  error.name = name
  return error
}

export function getAudioContextBaseLatency() {
  const isAllowedPlatform = isAndroid() || isWebKit()
  if (!isAllowedPlatform) return -2
  if (!window.AudioContext) return -1
  const latency = new AudioContext().baseLatency
  if (latency === null || latency === undefined) return -1
  if (!isFinite(latency)) return -3
  return latency
}
