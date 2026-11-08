package fp

import scala.scalajs.js

object Sources {
  val sources: js.Dictionary[js.Function] = {
    val o = js.Dictionary.empty[js.Function]
    def put0(key: String, name: String, fn: js.Function1[js.Any, js.Any]): Unit =
      o(key) = Host.source0(name, fn)
    def put1(key: String, name: String, fn: js.Function1[js.Any, js.Any]): Unit =
      o(key) = Host.source1(name, fn)
    put0("userAgentData", "getUserAgentData", UserAgentData.getUserAgentData)
    put0("fonts", "getFonts", Fonts.getFonts)
    put0("domBlockers", "getDomBlockers", DomBlockers.getDomBlockers)
    put0("fontPreferences", "getFontPreferences", FontPreferences.getFontPreferences)
    put0("audio", "getAudioFingerprint", Audio.getAudioFingerprint)
    put0("screenFrame", "getScreenFrame", ScreenFrame.getScreenFrame)
    put0("canvas", "getCanvasFingerprint", Canvas.getCanvasFingerprint)
    put0("osCpu", "getOsCpu", SourcesRest.getOsCpu)
    put0("languages", "getLanguages", SourcesRest.getLanguages)
    put0("colorDepth", "getColorDepth", SourcesRest.getColorDepth)
    put0("deviceMemory", "getDeviceMemory", SourcesRest.getDeviceMemory)
    put0("screenResolution", "getScreenResolution", SourcesRest.getScreenResolution)
    put0("hardwareConcurrency", "getHardwareConcurrency", SourcesRest.getHardwareConcurrency)
    put0("timezone", "getTimezone", SourcesRest.getTimezone)
    put0("sessionStorage", "getSessionStorage", SourcesRest.getSessionStorage)
    put0("localStorage", "getLocalStorage", SourcesRest.getLocalStorage)
    put0("indexedDB", "getIndexedDB", SourcesRest.getIndexedDB)
    put0("openDatabase", "getOpenDatabase", SourcesRest.getOpenDatabase)
    put0("cpuClass", "getCpuClass", SourcesRest.getCpuClass)
    put0("platform", "getPlatform", SourcesRest.getPlatform)
    put0("plugins", "getPlugins", SourcesRest.getPlugins)
    put0("touchSupport", "getTouchSupport", SourcesRest.getTouchSupport)
    put0("vendor", "getVendor", SourcesRest.getVendor)
    put0("vendorFlavors", "getVendorFlavors", SourcesRest.getVendorFlavors)
    put0("cookiesEnabled", "areCookiesEnabled", SourcesRest.areCookiesEnabled)
    put0("colorGamut", "getColorGamut", SourcesRest.getColorGamut)
    put0("invertedColors", "areColorsInverted", SourcesRest.areColorsInverted)
    put0("forcedColors", "areColorsForced", SourcesRest.areColorsForced)
    put0("monochrome", "getMonochromeDepth", SourcesRest.getMonochromeDepth)
    put0("contrast", "getContrastPreference", SourcesRest.getContrastPreference)
    put0("reducedMotion", "isMotionReduced", SourcesRest.isMotionReduced)
    put0("reducedTransparency", "isTransparencyReduced", SourcesRest.isTransparencyReduced)
    put0("hdr", "isHDR", SourcesRest.isHDR)
    put0("math", "getMathFingerprint", SourcesRest.getMathFingerprint)
    put0("pdfViewerEnabled", "isPdfViewerEnabled", SourcesRest.isPdfViewerEnabled)
    put0("architecture", "getArchitecture", SourcesRest.getArchitecture)
    put0("applePay", "getApplePayState", SourcesRest.getApplePayState)
    put0("privateClickMeasurement", "getPrivateClickMeasurement", SourcesRest.getPrivateClickMeasurement)
    put0("audioBaseLatency", "getAudioContextBaseLatency", SourcesRest.getAudioContextBaseLatency)
    put0("dateTimeLocale", "getDateTimeLocale", SourcesRest.getDateTimeLocale)
    put1("webGlBasics", "getWebGlBasics", Webgl.getWebGlBasics)
    put1("webGlExtensions", "getWebGlExtensions", Webgl.getWebGlExtensions)
    o
  }

  def loadBuiltin(options: js.Any): js.Function0[js.Promise[js.Dynamic]] =
    Entropy.loadSources(sources, options, js.Array(), js.undefined)
}
