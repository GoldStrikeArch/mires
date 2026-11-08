package fp

import scala.scalajs.js

object ScreenFrame {
  val screenFrameCheckInterval = 2500
  private val roundingPrecision = 10

  private var screenFrameBackup: js.UndefOr[js.Array[js.Any]] = js.undefined
  private var screenFrameSizeTimeoutId: js.UndefOr[Double] = js.undefined

  private def watchScreenFrame(): Unit =
    if (screenFrameSizeTimeoutId.isEmpty) {
      def checkScreenFrame(): Unit = {
        val frameSize = getCurrentScreenFrame()
        if (isFrameSizeNull(frameSize))
          screenFrameSizeTimeoutId = Host.g.setTimeout(() => checkScreenFrame(), screenFrameCheckInterval).asInstanceOf[Double]
        else {
          screenFrameBackup = frameSize
          screenFrameSizeTimeoutId = js.undefined
        }
      }
      checkScreenFrame()
    }

  def getUnstableScreenFrame(): js.Function0[js.Promise[js.Array[js.Any]]] = {
    watchScreenFrame()
    () => {
      var frameSize = getCurrentScreenFrame()
      def finish(): js.Array[js.Any] = {
        if (!isFrameSizeNull(frameSize)) screenFrameBackup = frameSize
        frameSize
      }
      if (isFrameSizeNull(frameSize)) {
        if (screenFrameBackup.isDefined) js.Promise.resolve(screenFrameBackup.get)
        else if (Host.jsTruthy(Browser.getFullscreenElement()))
          Browser.exitFullscreen().`then` { (_: Unit) =>
            frameSize = getCurrentScreenFrame()
            finish()
          }
        else js.Promise.resolve(finish())
      } else js.Promise.resolve(finish())
    }
  }

  def getScreenFrame(_options: js.Any): js.Function0[js.Promise[js.Any]] = {
    val isSafari17OrAbove = Browser.isWebKit() && Browser.isWebKit616OrNewer() && Browser.isSafariWebKit()
    val isFirefox143OrAbove = Browser.isGecko() && Browser.isGecko143OrNewer()
    if (isSafari17OrAbove || isFirefox143OrAbove)
      return () => js.Promise.resolve[js.Any](js.undefined)
    val screenFrameGetter = getUnstableScreenFrame()
    () =>
      screenFrameGetter().`then` { (frameSize: js.Array[js.Any]) =>
        def processSize(sideSize: js.Any): js.Any =
          if (sideSize == null || Host.isUndef(sideSize)) null
          else Data.round(Host.asNumber(sideSize), roundingPrecision)
        js.Array(processSize(frameSize(0)), processSize(frameSize(1)), processSize(frameSize(2)), processSize(frameSize(3)))
          .asInstanceOf[js.Any]
      }
  }

  private def getCurrentScreenFrame(): js.Array[js.Any] = {
    val s = Host.screen
    js.Array(
      Data.replaceNaN(Data.toFloat(s.availTop.asInstanceOf[js.Any]), null),
      Data.replaceNaN(
        Data.toFloat(s.width.asInstanceOf[js.Any]) - Data.toFloat(s.availWidth.asInstanceOf[js.Any]) -
          Host.asNumber(Data.replaceNaN(Data.toFloat(s.availLeft.asInstanceOf[js.Any]), 0)),
        null,
      ),
      Data.replaceNaN(
        Data.toFloat(s.height.asInstanceOf[js.Any]) - Data.toFloat(s.availHeight.asInstanceOf[js.Any]) -
          Host.asNumber(Data.replaceNaN(Data.toFloat(s.availTop.asInstanceOf[js.Any]), 0)),
        null,
      ),
      Data.replaceNaN(Data.toFloat(s.availLeft.asInstanceOf[js.Any]), null),
    )
  }

  private def isFrameSizeNull(frameSize: js.Array[js.Any]): Boolean = {
    var i = 0
    while (i < 4) {
      if (Host.jsTruthy(frameSize(i))) return false
      i += 1
    }
    true
  }

  val source: js.Function1[js.Any, js.Function0[js.Promise[js.Any]]] = getScreenFrame
  val getUnstableScreenFrameFn: js.Function0[js.Function0[js.Promise[js.Array[js.Any]]]] =
    () => getUnstableScreenFrame()
}
