// Entry of the Scala.js port: what libraries/ky/reference.mjs does, on the port's Ky. Each request
// case gets its own abort controller, transport and hooks, which record the trace.
//
// The boundary: the corpus input is the JS object itself, seen through @js.native facade traits
// (no copy). The option values Ky validates stay the JS values they are (`js.UndefOr[Any]`)
// until Ky's own checks read them. Each result is the JS array [trace, outcome]. `run` is a native
// ES module export.

package ky

import scala.collection.mutable
import scala.concurrent.{ExecutionContext, Future, Promise}
import scala.scalajs.concurrent.JSExecutionContext
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Reply extends js.Object:
  val kind: String = js.native
  val status: js.Any = js.native
  val body: Any = js.native
  val headers: js.Any = js.native

@js.native
trait Case extends js.Object:
  val url: String = js.native
  val method: js.UndefOr[String] = js.native
  val headers: js.UndefOr[js.Object] = js.native
  val json: js.UndefOr[Any] = js.native
  val read: String = js.native
  val replies: js.Array[Reply] = js.native
  val limit: js.UndefOr[Any] = js.native
  val backoffLimit: js.UndefOr[Any] = js.native
  val maxRetryAfter: js.UndefOr[Any] = js.native
  val timeout: js.UndefOr[Any] = js.native
  val retryOnTimeout: js.UndefOr[Any] = js.native
  val throwHttpErrors: js.UndefOr[Any] = js.native
  val abort: String = js.native
  val methods: js.UndefOr[Any] = js.native
  val statusCodes: js.UndefOr[Any] = js.native

@js.native
trait Input extends js.Object:
  val cases: js.Array[Case] = js.native

object Main:
  private given ExecutionContext = JSExecutionContext.queue

  /** What the adapter aborts its controller with. */
  private def cancel(controller: AbortController): Unit =
    val reason = js.Error("cancelled")
    reason.asInstanceOf[js.Dynamic].name = "AbortError"
    controller.abort(reason)

  private def requestCase(c: Case): Future[js.Array[Any]] =
    val trace = mutable.ArrayBuffer.empty[js.Array[Any]]
    val controller = AbortController()
    if c.abort == "before" then cancel(controller)
    var sent = 0
    // The injected transport: it records what it was sent and answers with the next reply.
    def fetch(request: Request): Future[Response] = request.text().toFuture.flatMap { body =>
      val headers = request.headers
      trace += js.Array(
        "fetch", request.method, request.url, headers.get("accept"), headers.get("content-type"),
        headers.get("x-matrix"), body
      )
      if c.abort == "fetch" then cancel(controller)
      val signal = request.signal
      signal.throwIfAborted()
      val reply = c.replies(sent.min(c.replies.length - 1))
      sent += 1
      reply.kind match
        case "network" => Future.failed(js.JavaScriptException(js.TypeError("Failed to fetch")))
        case "stall" =>
          val stalled = Promise[Response]()
          signal.addEventListener(
            "abort",
            () => stalled.failure(js.JavaScriptException(signal.reason)),
            js.Dynamic.literal(once = true)
          )
          stalled.future
        case _ => Future.successful(Response(reply.body, js.Dynamic.literal(status = reply.status, headers = reply.headers)))
    }
    def delay(attempt: Int): Double =
      val milliseconds = 300 * Math.pow(2, attempt - 1)
      trace += js.Array("backoff", attempt, milliseconds)
      milliseconds
    val hooks = Hooks(
      init = List(() => trace += js.Array("init")),
      beforeRequest = List { (request, retryCount) =>
        trace += js.Array("beforeRequest", retryCount)
        request.headers.set("x-matrix", "initial")
      },
      afterResponse = List((_, response, retryCount) => trace += js.Array("afterResponse", retryCount, response.status)),
      beforeRetry = List { (request, error, retryCount) =>
        trace += js.Array("beforeRetry", retryCount, KyError.name(error))
        request.headers.set("x-matrix", s"retry-$retryCount")
        if c.abort == "retry" then cancel(controller)
      },
      beforeError = List((_, error, retryCount) => trace += js.Array("beforeError", retryCount, KyError.name(error)))
    )
    val options = Options(
      method = c.method.toOption,
      headers = c.headers.toOption,
      json = c.json.toOption,
      retry = RetryOptions(
        limit = c.limit.toOption,
        methods = c.methods.toOption,
        statusCodes = c.statusCodes.toOption,
        maxRetryAfter = c.maxRetryAfter.toOption,
        backoffLimit = c.backoffLimit.toOption,
        retryOnTimeout = c.retryOnTimeout.toOption,
        delay = delay
      ),
      timeout = c.timeout.toOption,
      throwHttpErrors = c.throwHttpErrors.toOption,
      hooks = hooks,
      fetch = fetch,
      signal = Some(controller.signal)
    )
    Ky.request(c.url, options, if c.read == "json" then Shortcut.Json else Shortcut.Text)
      .map(result => js.Array[Any]("ok", result))
      .recover {
        case HTTPError(response, _, data) => js.Array("error", "HTTPError", response.status, data.getOrElse(null))
        case error => js.Array("error", KyError.name(error), null, null)
      }
      .map(outcome => js.Array(trace.toJSArray, outcome))

  @JSExportTopLevel("run")
  def run(input: Input): js.Promise[js.Array[js.Array[Any]]] =
    input.cases
      .foldLeft(Future.successful(Vector.empty[js.Array[Any]])) { (results, c) =>
        results.flatMap(done => requestCase(c).map(done :+ _))
      }
      .map(_.toJSArray)
      .toJSPromise
