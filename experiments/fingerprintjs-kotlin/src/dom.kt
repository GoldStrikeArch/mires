internal fun withIframe(action: dynamic, initialHtml: dynamic, pollInterval: dynamic): dynamic {
    val html = if (isUndefined(initialHtml)) jsUndefined else initialHtml
    val domPollInterval = if (isUndefined(pollInterval)) 50.0 else jsNumber(pollInterval)
    val d = jsDocument()

    fun waitForBody(): dynamic {
        return if (isTruthy(d.body)) {
            jsPromiseResolve(jsUndefined)
        } else {
            wait(domPollInterval).then({ waitForBody() })
        }
    }

    return waitForBody().then({
        val iframe = d.createElement("iframe")

        fun cleanup() {
            val parent = iframe.parentNode
            if (isTruthy(parent)) {
                parent.removeChild(iframe)
            }
        }

        val loaded = jsPromise { resolve, reject ->
            var isComplete = false
            val done = {
                if (!isComplete) {
                    isComplete = true
                    resolve(jsUndefined)
                }
            }
            val fail = { error: dynamic ->
                if (!isComplete) {
                    isComplete = true
                    reject(error)
                }
            }
            iframe.onload = { done() }
            iframe.onerror = fail
            val style = iframe.style
            style.setProperty("display", "block", "important")
            style.position = "absolute"
            style.top = "0"
            style.left = "0"
            style.visibility = "hidden"
            if (isTruthy(html) && jsIn("srcdoc", iframe)) {
                iframe.srcdoc = html
            } else {
                iframe.src = "about:blank"
            }
            d.body.appendChild(iframe)

            fun checkReadyState() {
                if (isComplete) return
                val contentWindow = iframe.contentWindow
                val ready = isTruthy(contentWindow) &&
                    isTruthy(contentWindow.document) &&
                    contentWindow.document.readyState == "complete"
                if (ready) {
                    done()
                } else {
                    jsSetTimeout({ checkReadyState() }, 10.0)
                }
            }
            checkReadyState()
        }

        fun waitForIframeBody(): dynamic {
            val contentWindow = iframe.contentWindow
            val hasBody = isTruthy(contentWindow) &&
                isTruthy(contentWindow.document) &&
                isTruthy(contentWindow.document.body)
            return if (hasBody) {
                jsPromiseResolve(jsUndefined)
            } else {
                wait(domPollInterval).then({ waitForIframeBody() })
            }
        }

        loaded.then({ waitForIframeBody() }).then({
            val invoked = jsTry {
                js("action(iframe, iframe.contentWindow)")
            }
            if (!jsTryOk(invoked)) {
                cleanup()
                jsThrow(invoked.error)
            }
            val value = invoked.value
            if (isPromise(value)) {
                return@then value.then({ inner: dynamic ->
                    cleanup()
                    inner
                }, { error: dynamic ->
                    cleanup()
                    jsThrow(error)
                })
            }
            cleanup()
            value
        }, { error: dynamic ->
            cleanup()
            jsThrow(error)
        })
    })
}

internal fun selectorToElement(selector: String): dynamic {
    val parsed = parseSimpleCssSelector(selector)
    val element = jsDocument().createElement(parsed.first ?: "div")
    val attributes = parsed.second
    val names = jsKeys(attributes)
    val length = jsLength(names)
    var i = 0
    while (i < length) {
        val name = jsString(jsIndex(names, i))
        val value = jsJoin(jsGet(attributes, name), " ")
        if (name == "style") {
            addStyleString(element.style, value)
        } else {
            element.setAttribute(name, value)
        }
        i += 1
    }
    return element
}

internal fun addStyleString(style: dynamic, source: String) {
    val properties = jsSplit(source, ";")
    val length = jsLength(properties)
    var i = 0
    while (i < length) {
        val property = jsIndex(properties, i)
        val match = js("( /^\\s*([\\w-]+)\\s*:\\s*(.+?)(\\s*!([\\w-]+))?\\s*$/.exec(property) )")
        if (isTruthy(match)) {
            val name = jsIndex(match, 1)
            val value = jsIndex(match, 2)
            val priority = jsIndex(match, 4)
            style.setProperty(name, value, if (isMissing(priority)) "" else priority)
        }
        i += 1
    }
}

internal fun isAnyParentCrossOrigin(): Boolean {
    var currentWindow: dynamic = jsWindow()
    while (true) {
        val parentWindow = currentWindow.parent
        if (!isTruthy(parentWindow) || js("parentWindow === currentWindow") as Boolean) {
            return false
        }
        val compared = jsTry {
            js("parentWindow.location.origin !== currentWindow.location.origin")
        }
        if (!jsTryOk(compared)) {
            val error = compared.error
            if (jsIsError(error) && error.name == "SecurityError") {
                return true
            }
            jsThrow(error)
        }
        if (isTruthy(compared.value)) {
            return true
        }
        currentWindow = parentWindow
    }
}
