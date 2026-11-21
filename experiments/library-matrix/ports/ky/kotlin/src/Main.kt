// Entry of the Kotlin/JS port: what libraries/ky/reference.mjs does, on the port's Ky. Each request
// case gets its own abort controller, transport and hooks, which record the trace.
//
// The boundary: the corpus input is a JS object typed by external interfaces and read in place.
// The option values Ky validates stay the JS values they are (`Any?`, undefined where omitted)
// until Ky's own checks read them. Each result is the JS array [trace, outcome]. `run` is a native
// ES module export through @JsExport.

import ky.AbortController
import ky.HTTPError
import ky.Hooks
import ky.JsError
import ky.Ky
import ky.Options
import ky.Request
import ky.Response
import ky.RetryOptions
import ky.Shortcut
import ky.await
import ky.hostCall
import ky.launch
import ky.name
import ky.typeError
import kotlin.js.Promise
import kotlin.js.json
import kotlin.math.min
import kotlin.math.pow

external interface Reply {
    val kind: String
    val status: Any?
    val body: String?
    val headers: Any?
}

external interface Case {
    val url: String
    val method: String?
    val headers: Any?
    val json: Any?
    val read: String
    val replies: Array<Reply>
    val limit: Any?
    val backoffLimit: Any?
    val maxRetryAfter: Any?
    val timeout: Any?
    val retryOnTimeout: Any?
    val throwHttpErrors: Any?
    val abort: String
    val methods: Any?
    val statusCodes: Any?
}

external interface Input {
    val cases: Array<Case>
}

/** What the adapter aborts its controller with. */
private fun cancel(controller: AbortController) {
    val reason = JsError("cancelled").asDynamic()
    reason.name = "AbortError"
    controller.abort(reason)
}

private suspend fun requestCase(case: Case): Array<Any?> {
    val trace = mutableListOf<Array<Any?>>()
    val controller = AbortController()
    if (case.abort == "before") cancel(controller)
    var sent = 0
    // The injected transport: it records what it was sent and answers with the next reply.
    val fetch = { request: Request ->
        launch {
            val body = request.text().await()
            val headers = request.headers
            trace += arrayOf(
                "fetch", request.method, request.url, headers.get("accept"), headers.get("content-type"),
                headers.get("x-matrix"), body,
            )
            if (case.abort == "fetch") cancel(controller)
            val signal = request.signal
            signal.throwIfAborted()
            val reply = case.replies[min(sent++, case.replies.size - 1)]
            when (reply.kind) {
                "network" -> throw typeError("Failed to fetch")
                "stall" -> Promise<Response> { _, reject ->
                    signal.addEventListener("abort", { reject(signal.reason) }, json("once" to true))
                }.await()
                else -> hostCall { Response(reply.body, json("status" to reply.status, "headers" to reply.headers)) }
            }
        }
    }
    val delay = { attempt: Int ->
        (300 * 2.0.pow(attempt - 1)).also { trace += arrayOf<Any?>("backoff", attempt, it) }
    }
    val hooks = Hooks(
        init = listOf { trace += arrayOf<Any?>("init") },
        beforeRequest = listOf { request, retryCount ->
            trace += arrayOf<Any?>("beforeRequest", retryCount)
            request.headers.set("x-matrix", "initial")
        },
        afterResponse = listOf { _, response, retryCount ->
            trace += arrayOf<Any?>("afterResponse", retryCount, response.status)
        },
        beforeRetry = listOf { request, error, retryCount ->
            trace += arrayOf<Any?>("beforeRetry", retryCount, error.name)
            request.headers.set("x-matrix", "retry-$retryCount")
            if (case.abort == "retry") cancel(controller)
        },
        beforeError = listOf { _, error, retryCount ->
            trace += arrayOf<Any?>("beforeError", retryCount, error.name)
        },
    )
    val options = Options(
        method = case.method,
        headers = case.headers,
        json = case.json,
        retry = RetryOptions(
            limit = case.limit,
            methods = case.methods,
            statusCodes = case.statusCodes,
            maxRetryAfter = case.maxRetryAfter,
            backoffLimit = case.backoffLimit,
            retryOnTimeout = case.retryOnTimeout,
            delay = delay,
        ),
        timeout = case.timeout,
        throwHttpErrors = case.throwHttpErrors,
        hooks = hooks,
        fetch = fetch,
        signal = controller.signal,
    )
    val outcome: Array<Any?> = try {
        arrayOf("ok", Ky.request(case.url, options, if (case.read == "json") Shortcut.Json else Shortcut.Text))
    } catch (error: HTTPError) {
        arrayOf("error", "HTTPError", error.response.status, error.data)
    } catch (error: Throwable) {
        arrayOf("error", error.name, null, null)
    }
    return arrayOf(trace.toTypedArray(), outcome)
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Promise<Array<Array<Any?>>> = launch {
    input.cases.map { requestCase(it) }.toTypedArray()
}
