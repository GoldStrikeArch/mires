package fp

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

/** Public FingerprintJS 5.2.0 namespace. Export order is alphabetical. */
object Exports {
  @JSExportTopLevel("componentsToDebugString")
  val componentsToDebugString: js.Function1[js.Any, String] = Agent.componentsToDebugStringFn

  @JSExportTopLevel("default")
  val default: js.Dynamic = js.Dynamic.literal(
    load = Agent.loadFn,
    hashComponents = Agent.hashComponentsFn,
    componentsToDebugString = Agent.componentsToDebugStringFn,
  )

  @JSExportTopLevel("getFullscreenElement")
  val getFullscreenElement: js.Function0[js.Any] = Browser.getFullscreenElementFn

  @JSExportTopLevel("getUnstableAudioFingerprint")
  val getUnstableAudioFingerprint: js.Function0[js.Any] = Audio.getUnstableAudioFingerprintFn

  @JSExportTopLevel("getUnstableCanvasFingerprint")
  val getUnstableCanvasFingerprint: js.Function1[js.UndefOr[Boolean], js.Dynamic] =
    Canvas.getUnstableCanvasFingerprintFn

  @JSExportTopLevel("getUnstableHardwareConcurrency")
  val getUnstableHardwareConcurrency: js.Function0[js.Any] = SourcesRest.getUnstableHardwareConcurrencyFn

  @JSExportTopLevel("getUnstableScreenFrame")
  val getUnstableScreenFrame: js.Function0[js.Function0[js.Promise[js.Array[js.Any]]]] =
    ScreenFrame.getUnstableScreenFrameFn

  @JSExportTopLevel("getUnstableScreenResolution")
  val getUnstableScreenResolution: js.Function0[js.Array[js.Any]] =
    SourcesRest.getUnstableScreenResolutionFn

  @JSExportTopLevel("getWebGLContext")
  val getWebGLContext: js.Function1[js.Any, js.Any] = Webgl.getWebGLContextFn

  @JSExportTopLevel("hashComponents")
  val hashComponents: js.Function1[js.Dynamic, String] = Agent.hashComponentsFn

  @JSExportTopLevel("isAndroid")
  val isAndroid: js.Function0[Boolean] = Browser.isAndroidFn

  @JSExportTopLevel("isChromium")
  val isChromium: js.Function0[Boolean] = Browser.isChromiumFn

  @JSExportTopLevel("isDesktopWebKit")
  val isDesktopWebKit: js.Function0[Boolean] = Browser.isDesktopWebKitFn

  @JSExportTopLevel("isEdgeHTML")
  val isEdgeHTML: js.Function0[Boolean] = Browser.isEdgeHTMLFn

  @JSExportTopLevel("isGecko")
  val isGecko: js.Function0[Boolean] = Browser.isGeckoFn

  @JSExportTopLevel("isSamsungInternet")
  val isSamsungInternet: js.Function0[Boolean] = Browser.isSamsungInternetFn

  @JSExportTopLevel("isTrident")
  val isTrident: js.Function0[Boolean] = Browser.isTridentFn

  @JSExportTopLevel("isWebKit")
  val isWebKit: js.Function0[Boolean] = Browser.isWebKitFn

  @JSExportTopLevel("load")
  val load: js.Function1[js.UndefOr[js.Dynamic], js.Promise[js.Dynamic]] = Agent.loadFn

  @JSExportTopLevel("loadSources")
  val loadSources: js.Function4[
    js.Dictionary[js.Function],
    js.Any,
    js.Array[js.Any],
    js.UndefOr[Double],
    js.Function0[js.Promise[js.Dynamic]],
  ] = Entropy.loadSourcesFn

  @JSExportTopLevel("murmurX64Hash128")
  val murmurX64Hash128: js.Function2[String, js.UndefOr[Double], String] = Hashing.murmurX64Hash128

  @JSExportTopLevel("prepareForSources")
  val prepareForSources: js.Function1[js.UndefOr[Double], js.Promise[Unit]] = Agent.prepareForSourcesFn

  @JSExportTopLevel("sources")
  val sources: js.Dictionary[js.Function] = Sources.sources

  @JSExportTopLevel("transformSource")
  val transformSource: js.Function2[js.Function, js.Function1[js.Any, js.Any], js.Function] =
    Entropy.transformSourceFn

  @JSExportTopLevel("withIframe")
  val withIframe: js.Function3[
    js.Function2[js.Dynamic, js.Dynamic, js.Any],
    js.UndefOr[String],
    js.UndefOr[Double],
    js.Promise[js.Any],
  ] = Dom.withIframeFn
}
