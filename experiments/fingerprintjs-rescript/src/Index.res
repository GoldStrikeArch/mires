let componentsToDebugString = Agent.componentsToDebugString
let default = {
  "load": Agent.load,
  "hashComponents": Agent.hashComponents,
  "componentsToDebugString": Agent.componentsToDebugString,
}
let getFullscreenElement = Browser.getFullscreenElement
let getUnstableAudioFingerprint = Audio.getUnstableAudioFingerprint
let getUnstableCanvasFingerprint = Canvas.getUnstableCanvasFingerprint
let getUnstableHardwareConcurrency = HardwareConcurrency.getUnstableHardwareConcurrency
let getUnstableScreenFrame = ScreenFrame.getUnstableScreenFrame
let getUnstableScreenResolution = ScreenResolution.getUnstableScreenResolution
let getWebGLContext = Webgl.getWebGLContext
let hashComponents = Agent.hashComponents
let isAndroid = Browser.isAndroid
let isChromium = Browser.isChromium
let isDesktopWebKit = Browser.isDesktopWebKit
let isEdgeHTML = Browser.isEdgeHTML
let isGecko = Browser.isGecko
let isSamsungInternet = Browser.isSamsungInternet
let isTrident = Browser.isTrident
let isWebKit = Browser.isWebKit
let load = Agent.load
let loadSources = EntropySource.loadSources
let murmurX64Hash128 = Hashing.murmurX64Hash128
let prepareForSources = Agent.prepareForSources
let sources = Sources.sources
let transformSource = EntropySource.transformSource
let withIframe = FpDom.withIframe
