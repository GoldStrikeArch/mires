package fp

import scala.scalajs.js

object Dom {
  def withIframe[T](
      action: (js.Dynamic, js.Dynamic) => js.Any,
      initialHtml: js.UndefOr[String] = js.undefined,
      domPollInterval: Double = 50,
  ): js.Promise[T] = {
    val d = Host.document

    def waitForBody(): js.Promise[Unit] =
      if (Host.jsTruthy(d.body.asInstanceOf[js.Any])) js.Promise.resolve(())
      else Async.wait(domPollInterval).`then`(_ => waitForBody())

    waitForBody()
      .`then`(_ => attach(d, action, initialHtml, domPollInterval))
      .asInstanceOf[js.Promise[T]]
  }

  private def attach(
      d: js.Dynamic,
      action: (js.Dynamic, js.Dynamic) => js.Any,
      initialHtml: js.UndefOr[String],
      domPollInterval: Double,
  ): js.Promise[js.Any] = {
    val iframe = d.createElement("iframe").asInstanceOf[js.Dynamic]
    def cleanup(): Unit = {
      val parent = iframe.parentNode
      if (Host.jsTruthy(parent.asInstanceOf[js.Any])) parent.removeChild(iframe)
    }
    loadIframe(d, iframe, initialHtml)
      .`then`(_ => waitForIframeBody(iframe, domPollInterval))
      .`then` { _ =>
        val result = action(iframe, iframe.contentWindow.asInstanceOf[js.Dynamic])
        if (Async.isPromise(result)) result.asInstanceOf[js.Promise[js.Any]]
        else js.Promise.resolve(result)
      }
      .`asInstanceOf`[js.Promise[js.Any]]
      .`then`[js.Any](
        (value: js.Any) => {
          cleanup()
          value
        },
        (error: Any) => {
          cleanup()
          js.Promise.reject(error.asInstanceOf[js.Any])
        },
      )
  }

  private def loadIframe(d: js.Dynamic, iframe: js.Dynamic, initialHtml: js.UndefOr[String]): js.Promise[Unit] =
    new js.Promise[Unit]((resolve0, reject0) => {
      var isComplete = false
      def resolve(): Unit =
        if (!isComplete) {
          isComplete = true
          resolve0(())
        }
      def reject(error: js.Any): Unit =
        if (!isComplete) {
          isComplete = true
          reject0(error)
        }

      iframe.onload = () => resolve()
      iframe.onerror = (error: js.Any) => reject(error)
      val style = iframe.style
      style.setProperty("display", "block", "important")
      style.position = "absolute"
      style.top = "0"
      style.left = "0"
      style.visibility = "hidden"
      if (initialHtml.isDefined && Host.has(iframe, "srcdoc")) iframe.srcdoc = initialHtml.get
      else iframe.src = "about:blank"
      d.body.appendChild(iframe)

      def checkReadyState(): Unit =
        if (!isComplete) {
          val win = iframe.contentWindow.asInstanceOf[js.Any]
          val doc =
            if (Host.isUndef(win)) js.undefined.asInstanceOf[js.Any]
            else Host.dyn(win).document.asInstanceOf[js.Any]
          val ready =
            if (Host.isUndef(doc)) js.undefined.asInstanceOf[js.Any]
            else Host.dyn(doc).readyState.asInstanceOf[js.Any]
          if (Host.toJsString(ready) == "complete") resolve()
          else Host.g.setTimeout(() => checkReadyState(), 10)
        }
      checkReadyState()
    })

  private def waitForIframeBody(iframe: js.Dynamic, domPollInterval: Double): js.Promise[Unit] = {
    val win = iframe.contentWindow.asInstanceOf[js.Any]
    val doc =
      if (Host.isUndef(win)) js.undefined.asInstanceOf[js.Any]
      else Host.dyn(win).document.asInstanceOf[js.Any]
    val body =
      if (Host.isUndef(doc)) js.undefined.asInstanceOf[js.Any]
      else Host.dyn(doc).body.asInstanceOf[js.Any]
    if (Host.jsTruthy(body.asInstanceOf[js.Any])) js.Promise.resolve(())
    else Async.wait(domPollInterval).`then`(_ => waitForIframeBody(iframe, domPollInterval))
  }

  def selectorToElement(selector: String): js.Dynamic = {
    val (tag, attributes) = Data.parseSimpleCssSelector(selector)
    val element = Host.document.createElement(tag.getOrElse("div")).asInstanceOf[js.Dynamic]
    val names = js.Object.keys(attributes.asInstanceOf[js.Object])
    var i = 0
    while (i < names.length) {
      val name = names(i)
      val value = attributes(name).join(" ")
      if (name == "style") addStyleString(element.style.asInstanceOf[js.Dynamic], value)
      else element.setAttribute(name, value)
      i += 1
    }
    element
  }

  def addStyleString(style: js.Dynamic, source: String): Unit = {
    val parts = source.split(";")
    var i = 0
    while (i < parts.length) {
      val property = parts(i)
      val matched = new js.RegExp("""^\s*([\w-]+)\s*:\s*(.+?)(\s*!([\w-]+))?\s*$""").exec(property)
      if (matched != null) {
        val name = matched(1).getOrElse("")
        val value = matched(2).getOrElse("")
        val priority = matched(4).getOrElse("")
        style.setProperty(name, value, priority)
      }
      i += 1
    }
  }

  def isAnyParentCrossOrigin(): Boolean = {
    var currentWindow = Host.window
    while (true) {
      val parentWindow = currentWindow.parent
      if (Host.isUndef(parentWindow.asInstanceOf[js.Any]) || parentWindow.asInstanceOf[js.Any] == currentWindow.asInstanceOf[js.Any])
        return false
      try {
        if (
          Host.toJsString(parentWindow.location.origin.asInstanceOf[js.Any]) !=
            Host.toJsString(currentWindow.location.origin.asInstanceOf[js.Any])
        ) return true
      } catch {
        case js.JavaScriptException(error) =>
          val err = Host.dyn(error.asInstanceOf[js.Any])
          if (Host.toJsString(err.name.asInstanceOf[js.Any]) == "SecurityError") return true
          throw js.JavaScriptException(error)
        case error: Throwable => throw error
      }
      currentWindow = parentWindow.asInstanceOf[js.Dynamic]
    }
    false
  }

  val withIframeFn: js.Function3[js.Function2[js.Dynamic, js.Dynamic, js.Any], js.UndefOr[String], js.UndefOr[Double], js.Promise[js.Any]] =
    (action, initialHtml, interval) =>
      withIframe[js.Any](
        (iframe, iWindow) => action(iframe, iWindow),
        initialHtml,
        interval.getOrElse(50),
      )
}
