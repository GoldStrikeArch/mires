package fp

import scala.scalajs.js

object Async {
  def waitMs[T](durationMs: Double, resolveWith: T): js.Promise[T] =
    new js.Promise[T]((resolve, _) => {
      Host.g.setTimeout(() => resolve(resolveWith), durationMs)
      ()
    })

  def wait(durationMs: Double): js.Promise[Unit] =
    waitMs(durationMs, ())

  def releaseEventLoop(): js.Promise[Unit] =
    new js.Promise[Unit]((resolve, _) => {
      val channel = Host.jsNew(Host.g.MessageChannel)()
      channel.port1.onmessage = (_: js.Any) => resolve(())
      channel.port2.postMessage(null)
      ()
    })

  def requestIdleCallbackIfAvailable(fallbackTimeout: Double, deadlineTimeout: Double = Double.PositiveInfinity): js.Promise[Unit] = {
    val w = Host.window
    val requestIdleCallback = w.selectDynamic("requestIdleCallback")
    if (Host.jsTruthy(requestIdleCallback)) {
      new js.Promise[Unit]((resolve, _) => {
        Host.dyn(requestIdleCallback).call(
          w,
          () => resolve(()),
          js.Dynamic.literal(timeout = deadlineTimeout),
        )
        ()
      })
    } else wait(math.min(fallbackTimeout, deadlineTimeout))
  }

  def isPromise(value: js.Any): Boolean =
    Host.jsTruthy(value) && Host.isJsFunction(Host.get(value, "then"))

  def awaitIfAsync(action: js.Function0[js.Any], callback: (Boolean, js.Any) => Unit): Unit =
    try {
      val returnedValue = action()
      if (isPromise(returnedValue)) {
        val p = returnedValue.asInstanceOf[js.Dynamic]
        p.`then`(
          (result: js.Any) => callback(true, result),
          (error: js.Any) => callback(false, error),
        )
        ()
      } else callback(true, returnedValue)
    } catch {
      case js.JavaScriptException(error) => callback(false, error.asInstanceOf[js.Any])
      case error: Throwable              => callback(false, error.asInstanceOf[js.Any])
    }

  def mapWithBreaks[T, U](
      items: js.Array[T],
      callback: (T, Int) => U,
      loopReleaseInterval: Double = 16,
  ): js.Promise[js.Array[U]] = {
    val results = new js.Array[U](items.length)
    var lastLoopReleaseTime = Host.dateNow()

    def step(i: Int): js.Promise[js.Array[U]] =
      if (i >= items.length) js.Promise.resolve[js.Array[U]](results)
      else {
        results(i) = callback(items(i), i)
        val now = Host.dateNow()
        if (now >= lastLoopReleaseTime + loopReleaseInterval) {
          lastLoopReleaseTime = now
          releaseEventLoop().`then`(_ => step(i + 1))
        } else step(i + 1)
      }

    step(0)
  }

  def suppressUnhandledRejectionWarning[T <: js.Thenable[?]](promise: T): T = {
    Host.dyn(promise).`then`(js.undefined, (_: js.Any) => js.undefined)
    promise
  }
}
