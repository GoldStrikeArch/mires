import {
  componentsToDebugString,
  hashComponents,
  load,
  murmurX64Hash128,
  prepareForSources,
} from './agent.js'
import {
  getFullscreenElement,
  isAndroid,
  isChromium,
  isDesktopWebKit,
  isEdgeHTML,
  isGecko,
  isSamsungInternet,
  isTrident,
  isWebKit,
} from './browser.js'
import { withIframe } from './dom.js'
import { loadSources, transformSource } from './entropy.js'
import { sources } from './sources/index.js'
import { getUnstableAudioFingerprint } from './sources/audio.js'
import { getUnstableCanvasFingerprint } from './sources/canvas.js'
import { getUnstableHardwareConcurrency } from './sources/simple.js'
import { getUnstableScreenFrame } from './sources/screen_frame.js'
import { getUnstableScreenResolution } from './sources/simple.js'
import { getWebGLContext } from './sources/webgl.js'

export {
  componentsToDebugString,
  getFullscreenElement,
  getUnstableAudioFingerprint,
  getUnstableCanvasFingerprint,
  getUnstableHardwareConcurrency,
  getUnstableScreenFrame,
  getUnstableScreenResolution,
  getWebGLContext,
  hashComponents,
  isAndroid,
  isChromium,
  isDesktopWebKit,
  isEdgeHTML,
  isGecko,
  isSamsungInternet,
  isTrident,
  isWebKit,
  load,
  loadSources,
  murmurX64Hash128,
  prepareForSources,
  sources,
  transformSource,
  withIframe,
}

export default { load, hashComponents, componentsToDebugString }
