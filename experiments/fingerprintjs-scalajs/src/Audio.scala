package fp

import scala.scalajs.js

object Audio {
  val KnownForSuspending = -1
  val NotSupported = -2
  val Timeout = -3
  val KnownForAntifingerprinting = -4

  private val InnerTimeout = "timeout"
  private val InnerSuspended = "suspended"

  def getAudioFingerprint(_options: js.Any): js.Any =
    if (doesBrowserPerformAntifingerprinting()) KnownForAntifingerprinting
    else getUnstableAudioFingerprint()

  def getUnstableAudioFingerprint(): js.Any = {
    val w = Host.window
    val AudioContext = {
      val a = w.OfflineAudioContext
      if (Host.jsTruthy(a.asInstanceOf[js.Any])) a else w.webkitOfflineAudioContext
    }
    if (!Host.jsTruthy(AudioContext.asInstanceOf[js.Any])) return NotSupported
    if (doesBrowserSuspendAudioContext()) return KnownForSuspending

    val hashFromIndex = 4500
    val hashToIndex = 5000
    val context = Host.jsNew(AudioContext)(1, hashToIndex, 44100)
    val oscillator = context.createOscillator()
    oscillator.`type` = "triangle"
    oscillator.frequency.value = 10000
    val compressor = context.createDynamicsCompressor()
    compressor.threshold.value = -50
    compressor.knee.value = 40
    compressor.ratio.value = 12
    compressor.attack.value = 0
    compressor.release.value = 0.25
    oscillator.connect(compressor)
    compressor.connect(context.destination)
    oscillator.start(0)

    val started = startRenderingAudio(context)
    val renderPromise = started._1
    val finishRendering = started._2
    val fingerprintPromise = Async.suppressUnhandledRejectionWarning(
      renderPromise
        .`then`[Double] { (buffer: js.Dynamic) =>
          val data = buffer.getChannelData(0).subarray(hashFromIndex)
          getHash(data)
        }
        .`asInstanceOf`[js.Promise[js.Any]]
        .`catch` { (error: Any) =>
          val jsError = error.asInstanceOf[js.Any]
          val name = Host.toJsString(Host.dyn(jsError).name.asInstanceOf[js.Any])
          if (name == InnerTimeout || name == InnerSuspended)
            js.Promise.resolve[js.Any](Timeout)
          else js.Promise.reject(jsError)
        },
    )

    val getter: js.Function0[js.Promise[js.Any]] = () => {
      finishRendering()
      fingerprintPromise.asInstanceOf[js.Promise[js.Any]]
    }
    getter
  }

  private def doesBrowserSuspendAudioContext(): Boolean =
    Browser.isWebKit() && !Browser.isDesktopWebKit() && !Browser.isWebKit606OrNewer()

  private def doesBrowserPerformAntifingerprinting(): Boolean =
    (Browser.isWebKit() && Browser.isWebKit616OrNewer() && Browser.isSafariWebKit()) ||
      (Browser.isChromium() && Browser.isSamsungInternet() && Browser.isChromium122OrNewer())

  private def startRenderingAudio(context: js.Dynamic): (js.Promise[js.Dynamic], () => Unit) = {
    val renderTryMaxCount = 3
    val renderRetryDelay = 500
    val runningMaxAwaitTime = 500
    val runningSufficientTime = 5000
    var finalize: () => Unit = () => ()

    val resultPromise = new js.Promise[js.Dynamic]((resolve, reject) => {
      var isFinalized = false
      var renderTryCount = 0
      var startedRunningAt = 0.0

      context.oncomplete = (event: js.Dynamic) => resolve(event.renderedBuffer)

      def startRunningTimeout(): Unit = {
        Host.g.setTimeout(
          () => reject(makeInnerError(InnerTimeout)),
          math.min(runningMaxAwaitTime, startedRunningAt + runningSufficientTime - Host.dateNow()),
        )
        ()
      }

      def tryRender(): Unit =
        try {
          val renderingPromise = context.startRendering()
          if (Async.isPromise(renderingPromise.asInstanceOf[js.Any]))
            Async.suppressUnhandledRejectionWarning(renderingPromise.asInstanceOf[js.Promise[js.Any]])
          val state = Host.toJsString(context.state.asInstanceOf[js.Any])
          state match {
            case "running" =>
              startedRunningAt = Host.dateNow()
              if (isFinalized) startRunningTimeout()
            case "suspended" =>
              if (!Host.jsTruthy(Host.document.hidden.asInstanceOf[js.Any])) renderTryCount += 1
              if (isFinalized && renderTryCount >= renderTryMaxCount)
                reject(makeInnerError(InnerSuspended))
              else Host.g.setTimeout(() => tryRender(), renderRetryDelay)
            case _ => ()
          }
        } catch {
          case js.JavaScriptException(error) => reject(error.asInstanceOf[js.Any])
          case error: Throwable              => reject(error.asInstanceOf[js.Any])
        }

      tryRender()
      finalize = () => {
        if (!isFinalized) {
          isFinalized = true
          if (startedRunningAt > 0) startRunningTimeout()
        }
      }
    })

    (resultPromise, () => finalize())
  }

  private def getHash(signal: js.Any): Double = {
    val arr = signal.asInstanceOf[js.typedarray.Float32Array]
    val length = arr.length
    var hash = 0.0
    var i = 0
    while (i < length) {
      hash += math.abs(arr(i).toDouble)
      i += 1
    }
    hash
  }

  private def makeInnerError(name: String): js.Error = {
    val error = new js.Error(name)
    Host.dyn(error).updateDynamic("name")(name)
    error
  }

  val source: js.Function1[js.Any, js.Any] = getAudioFingerprint
  val getUnstableAudioFingerprintFn: js.Function0[js.Any] = () => getUnstableAudioFingerprint()
}
