// Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
// utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md. Ky's async
// methods are Futures; what Ky throws, they fail with.

package ky

import scala.collection.mutable
import scala.concurrent.{ExecutionContext, Future, Promise}
import scala.scalajs.concurrent.JSExecutionContext
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.timers
import scala.util.{Failure, Success}

/** The errors/ modules: Ky's own errors. Everything else a call can fail with (a TypeError, RangeError,
  * SyntaxError or abort reason, from a host API, the transport or Ky's option checks) is the JS
  * error that was thrown, in a `js.JavaScriptException`.
  */
sealed abstract class KyError(val errorName: String) extends Exception(errorName)
final case class HTTPError(response: Response, request: Request, data: Option[Any]) extends KyError("HTTPError")
final case class NetworkError(request: Request, cause: Any) extends KyError("NetworkError")
final case class TimeoutError(request: Request) extends KyError("TimeoutError")

object KyError:
  /** `error.name`, as the hooks and the caller read it. */
  def name(error: Throwable): String = error match
    case error: KyError => error.errorName
    case js.JavaScriptException(value) => value.asInstanceOf[js.Dynamic].name.asInstanceOf[js.UndefOr[String]].getOrElse("Error")
    case _ => "Error"

final case class Hooks(
    init: List[() => Unit],
    beforeRequest: List[(Request, Int) => Unit],
    afterResponse: List[(Request, Response, Int) => Unit],
    beforeRetry: List[(Request, Throwable, Int) => Unit],
    beforeError: List[(Request, Throwable, Int) => Unit]
)

/** The caller's retry options, as the JS values given (None where undefined); Ky's
  * `normalizeRetryOptions` validates them.
  */
final case class RetryOptions(
    limit: Option[Any],
    methods: Option[Any],
    statusCodes: Option[Any],
    maxRetryAfter: Option[Any],
    backoffLimit: Option[Any],
    retryOnTimeout: Option[Any],
    delay: Int => Double
)

final case class Options(
    method: Option[String],
    /** A plain object of header names to values. */
    headers: Option[js.Object],
    json: Option[Any],
    retry: RetryOptions,
    timeout: Option[Any],
    throwHttpErrors: Option[Any],
    hooks: Hooks,
    fetch: Request => Future[Response],
    signal: Option[AbortSignal]
)

/** The body shortcuts, and the media type each one advertises. */
enum Shortcut(val mimeType: String):
  case Json extends Shortcut("application/json")
  case Text extends Shortcut("text/*")

private final case class Retry(
    limit: Double,
    methods: List[String],
    statusCodes: List[Double],
    afterStatusCodes: List[Double],
    maxRetryAfter: Double,
    backoffLimit: Double,
    delay: Int => Double,
    retryOnTimeout: Boolean
)

object Ky:
  /** Ky's continuations run as microtasks, like the promise jobs of its async functions. */
  private given ExecutionContext = JSExecutionContext.queue

  // core/constants.js and utils/normalize.js
  private val requestMethods = List("get", "post", "put", "patch", "head", "delete", "query")
  private val caseInsensitiveMethods = requestMethods ++ List("options", "trace")
  private val retryMethods = List("get", "put", "head", "delete", "options", "trace", "query")
  private val retryStatusCodes = List(408.0, 413.0, 429.0, 500.0, 502.0, 503.0, 504.0)
  private val retryAfterStatusCodes = List(413.0, 429.0, 503.0)
  /** The maximum value of a 32bit int (see issue #117) */
  private val MaxSafeTimeout = 2_147_483_647.0
  private val MaxErrorResponseBodySize = 10 * 1024 * 1024

  private def typeError(message: String) = js.JavaScriptException(js.TypeError(message))

  // Scala.js's toLowerCase and toUpperCase are JS's.
  private def normalizeRequestMethod(input: String) =
    if requestMethods.contains(input.toLowerCase) then input.toUpperCase else input

  /** Common methods for retry matching are case-insensitive; custom ones keep their case. */
  private def normalizeRetryMethod(method: String) =
    val lowercase = method.toLowerCase
    if caseInsensitiveMethods.contains(lowercase) then lowercase else method

  private def nonNegative(value: Any): Option[Double] = value match
    case number: Double if number >= 0 => Some(number)
    case _ => None

  /** The defaults for undefined values, and Ky's checks, in Ky's order. */
  private def normalizeRetryOptions(retry: RetryOptions): Retry =
    val limit = retry.limit match
      case None => 2.0
      case Some(limit: Double) if JsNumber.isInteger(limit) && limit >= 0 => limit
      case Some(_) => throw typeError("`retry.limit` must be a finite, non-negative integer")
    // Ky's own test, `Array.isArray`.
    def array(key: String, value: Option[Any]): Option[List[Any]] = value.map { values =>
      if js.Array.isArray(values.asInstanceOf[js.Any]) then values.asInstanceOf[js.Array[Any]].toList
      else throw js.JavaScriptException(js.Error(s"retry.$key must be an array"))
    }
    val methods = array("methods", retry.methods)
    val statusCodes = array("statusCodes", retry.statusCodes)
    // A mistyped entry only ever fails to match, so it is reported instead.
    if methods.exists(_.exists(!_.isInstanceOf[String])) then
      throw typeError("`retry.methods` must only contain strings")
    if statusCodes.exists(_.exists(!JsNumber.isInteger(_))) then
      throw typeError("`retry.statusCodes` must only contain numbers")
    def limitOption(key: String, value: Option[Any]) = value match
      case None => Double.PositiveInfinity
      case Some(value) =>
        nonNegative(value).getOrElse(throw typeError(s"`retry.$key` must be a non-negative number or `Infinity`"))
    val maxRetryAfter = limitOption("maxRetryAfter", retry.maxRetryAfter)
    val backoffLimit = limitOption("backoffLimit", retry.backoffLimit)
    val retryOnTimeout = retry.retryOnTimeout match
      case None => false
      case Some(flag: Boolean) => flag
      case Some(_) => throw typeError("`retry.retryOnTimeout` must be a boolean")
    Retry(
      limit = limit,
      methods = methods.fold(retryMethods)(_.map(_.asInstanceOf[String])).map(normalizeRetryMethod),
      statusCodes = statusCodes.fold(retryStatusCodes)(_.map(_.asInstanceOf[Double])),
      afterStatusCodes = retryAfterStatusCodes,
      maxRetryAfter = maxRetryAfter,
      backoffLimit = backoffLimit,
      delay = retry.delay,
      retryOnTimeout = retryOnTimeout
    )

  /** Milliseconds, or None for `false`. */
  private def validateTimeoutOption(value: Any): Option[Double] = value match
    case flag: Boolean if !flag => None
    case _ =>
      nonNegative(value) match
        // Infinity lands here too, since it is not a usable delay either.
        case Some(milliseconds) if milliseconds > MaxSafeTimeout =>
          throw js.JavaScriptException(js.RangeError("The `timeout` option cannot be greater than 2147483647"))
        case Some(milliseconds) => Some(milliseconds)
        case None => throw typeError("The `timeout` option must be a non-negative number or `false`")

  /** Option merging (deepMerge, mergeHeaderContainers) keys the headers by their lowercased name,
    * the last spelling of a name winning at the place of its first.
    */
  private def lowercaseHeaders(headers: js.Object): mutable.LinkedHashMap[String, Any] =
    val merged = mutable.LinkedHashMap.empty[String, Any]
    for name <- js.Object.keys(headers) do merged(name.toLowerCase) = headers.asInstanceOf[js.Dynamic].selectDynamic(name)
    merged

  /** mergeHeaders: each header set on a fresh Headers, which validates it. */
  private def mergeHeaders(headers: collection.Map[String, Any]) =
    val result = Headers()
    for (name, value) <- headers do result.set(name, value)
    result

  private val networkErrorMessages = Set(
    "network error",
    "NetworkError when attempting to fetch resource.",
    "The Internet connection appears to be offline.",
    "Network request failed",
    "fetch failed",
    "terminated",
    " A network error occurred.",
    "Network connection lost"
  )

  private val objectToString = js.Dynamic.global.Object.prototype.selectDynamic("toString")

  /** Ky's `isError`: an Error, or an Error of another realm by its internal brand. */
  private def isError(value: Any): Boolean =
    value.isInstanceOf[js.Error] || objectToString.call(value.asInstanceOf[js.Any]).asInstanceOf[String] == "[object Error]"

  /** utils/is-network-error.js, inlined by Ky from is-network-error 1.3.2. */
  private def isRawNetworkError(value: Any): Boolean =
    if !isError(value) then false
    else
      val error = value.asInstanceOf[js.Dynamic]
      if (error.name: Any) != "TypeError" then false
      else (error.message: Any) match
        case message: String =>
          def wrapped(prefix: String) = message.startsWith(prefix) && message.endsWith(")")
          if message == "Load failed" || wrapped("Load failed (") then
            // Safari 17+ has generic message but no stack for network errors
            js.isUndefined(error.stack) || js.Object.hasProperty(error.asInstanceOf[js.Object], "__sentry_captured__")
          else
            message.startsWith("error sending request for url") || message == "Failed to fetch" ||
            wrapped("Failed to fetch (") || networkErrorMessages.contains(message)
        case _ => false

  /** JS whitespace: what String.prototype.trim removes and `\s` matches. Java's trim removes
    * everything up to U+0020 instead.
    */
  private def isJsWhitespace(char: Char) = char match
    case '\t' | '\n' | '\u000b' | '\u000c' | '\r' | ' ' | '\u00a0' | '\u1680' | '\u2028' | '\u2029' | '\u202f' |
        '\u205f' | '\u3000' | '\ufeff' =>
      true
    case _ => char >= '\u2000' && char <= '\u200a'

  private def jsTrim(text: String) = text.dropWhile(isJsWhitespace).reverse.dropWhile(isJsWhitespace).reverse

  /** Matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`, written out rather than
    * matched with a pattern, as Ky does.
    */
  private def isJsonContentType(contentType: String): Boolean =
    val mimeType = jsTrim(contentType.takeWhile(_ != ';')).toLowerCase
    if !mimeType.endsWith("json") then false
    else
      mimeType.lift(mimeType.length - 5) match
        case Some('/') => true
        // The separator is not the slash, so any slash in the type comes before it.
        case Some('.' | '+' | '-') => mimeType.contains('/')
        case _ => false

  private val charsetPattern = js.RegExp(""";\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))""", "i")

  private def createTextDecoder(contentType: String): TextDecoder =
    val charset = Option(charsetPattern.exec(contentType)).flatMap(groups => groups(1).toOption.orElse(groups(2).toOption))
    charset
      .filter(_.nonEmpty)
      .flatMap(label =>
        try Some(TextDecoder(label))
        catch case js.JavaScriptException(_) => None
      )
      .getOrElse(TextDecoder())

  /** An error body as text, or None when it cannot be read or is larger than
    * maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259).
    */
  private def readResponseText(response: Response): Future[Option[String]] =
    if response.body == null then response.text().toFuture.map(Some(_)).recover { case _ => None }
    else
      val contentType = Option(response.headers.get("content-type")).getOrElse("")
      val decoder = if isJsonContentType(contentType) then TextDecoder() else createTextDecoder(contentType)
      response.arrayBuffer().toFuture
        .map(bytes => Option.when(bytes.byteLength <= MaxErrorResponseBodySize)(decoder.decode(bytes)))
        .recover { case _ => None }

  /** #getResponseData: the parsed JSON or the text of an error body, or None when it is empty,
    * unreadable or unparsable.
    */
  private def getResponseData(response: Response): Future[Option[Any]] =
    readResponseText(response).map {
      case None | Some("") => None
      case Some(text) if !isJsonContentType(Option(response.headers.get("content-type")).getOrElse("")) => Some(text)
      case Some(text) =>
        try Some(js.JSON.parse(text))
        catch case js.JavaScriptException(_) => None
    }

  private def createManagedSignal(user: Option[AbortSignal], controller: AbortController) = user match
    case Some(user) => AbortSignal.any(js.Array(user, controller.signal))
    case None => controller.signal

  private def withSignal(request: Request, signal: AbortSignal) = Request(request, js.Dynamic.literal(signal = signal))

  private def create(input: String, options: Options): Ky =
    val headers = options.headers.fold(mutable.LinkedHashMap.empty[String, Any])(lowercaseHeaders)
    val requestHeaders = mergeHeaders(headers)
    val method = normalizeRequestMethod(options.method.getOrElse("GET"))
    val retry = normalizeRetryOptions(options.retry)
    // Defaults only replace undefined, so a null reaches the validation and is reported.
    val throwHttpErrors = options.throwHttpErrors match
      case None => true
      case Some(flag: Boolean) => flag
      case Some(_) => throw typeError("The `throwHttpErrors` option must be a boolean or a function")
    val timeout = validateTimeoutOption(options.timeout.getOrElse(10_000.0))
    val abortController = AbortController()
    val signal = createManagedSignal(options.signal, abortController)
    // A content-type from options.headers counts as user-provided.
    val body = options.json.map(json => js.JSON.stringify(json.asInstanceOf[js.Any]))
    if body.isDefined && !headers.contains("content-type") then requestHeaders.set("content-type", "application/json")
    val init = js.Dynamic.literal(method = method, headers = requestHeaders, signal = signal)
    body.foreach(body => init.body = body)
    Ky(Request(input, init), abortController, signal, options.signal, retry, timeout, throwHttpErrors, options.hooks, options.fetch)

  /** `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body read. A
    * construction error fails without running the beforeError hooks, and so does a body that is
    * not JSON.
    */
  def request(input: String, options: Options, shortcut: Shortcut): Future[Any] =
    Future.fromTry(scala.util.Try {
      options.hooks.init.foreach(_())
      create(input, options)
    }).flatMap { ky =>
      // Before dispatch, ky.request is the request that will be sent.
      val headers = ky.request.headers
      headers.set("accept", Option(headers.get("accept")).filter(_.nonEmpty).getOrElse(shortcut.mimeType))
      ky.run()
        .recoverWith { case error => Future.failed(ky.processedError(error)) }
        .flatMap(_.text().toFuture)
        .map(text =>
          shortcut match
            case Shortcut.Text => text
            case Shortcut.Json => js.JSON.parse(text)
        )
    }

private final class Ky(
    var request: Request,
    private var abortController: AbortController,
    /** #options.signal: the user's signal and the abort controller's, combined. */
    private var signal: AbortSignal,
    private val userProvidedAbortSignal: Option[AbortSignal],
    private val retry: Retry,
    private val timeout: Option[Double],
    private val throwHttpErrors: Boolean,
    private val hooks: Hooks,
    /** The `fetch` option. */
    private val transport: Request => Future[Response]
):
  import Ky.given
  private var retryCount = 0
  private var originalRequest: Option[Request] = None

  /** #sentRequest: [[fetch]] replaces the request with the clone it prepares for a possible retry,
    * so the request that was actually sent is reported.
    */
  private def sentRequest = originalRequest.getOrElse(request)

  /** Chromium reports a user abort during a fetch as a network error; the abort reason is
    * surfaced instead.
    */
  private def throwIfAbortedByUser(): Unit = userProvidedAbortSignal.foreach(_.throwIfAborted())

  /** A hook runs inside #raceWithTotalTimeout(), which first throws a user abort. */
  private def runHooks[H](hooks: List[H])(call: H => Unit): Unit = hooks.foreach { hook =>
    throwIfAbortedByUser()
    call(hook)
  }

  def run(): Future[Response] =
    def settle(response: Response): Future[Response] =
      runHooks(hooks.afterResponse)(_(request, response, retryCount))
      if response.ok || !throwHttpErrors then Future.successful(response)
      else
        Ky.getResponseData(response).flatMap { data =>
          val error = HTTPError(response, sentRequest, data)
          throwIfAbortedByUser()
          retryFromError(error).flatMap(settle)
        }
    Future(runHooks(hooks.beforeRequest)(_(request, 0))).flatMap(_ => retryRequest()).flatMap(settle)

  private def retryRequest(): Future[Response] = fetch().recoverWith { case error => retryFromError(error) }

  private def retryFromError(error: Throwable): Future[Response] =
    Future {
      throwIfAbortedByUser()
      js.Math.min(calculateRetryDelay(error), Ky.MaxSafeTimeout)
    }.flatMap(sleep).flatMap { _ =>
      // Reset the abort controller if it was aborted (on a timeout retry), so hooks and the
      // retried request get a fresh signal.
      if abortController.signal.aborted then
        abortController = AbortController()
        signal = Ky.createManagedSignal(userProvidedAbortSignal, abortController)
        request = Ky.withSignal(request, signal)
      runHooks(hooks.beforeRetry)(_(request, error, retryCount + 1))
      retryCount += 1
      retryRequest()
    }

  private def sleep(milliseconds: Double): Future[Unit] =
    val done = Promise[Unit]()
    timers.setTimeout(milliseconds)(done.success(()))
    done.future

  private def fetch(): Future[Response] =
    // Cloning is done here to prepare in advance for retries, and skipped when they are disabled.
    val retryRequest = Option.when(retry.limit > 0)(Ky.withSignal(request.copy(), signal))
    val sent = request
    originalRequest = Some(sent)
    retryRequest.foreach(request = _)
    timeout.fold(transport(sent))(milliseconds => race(sent, milliseconds)).recoverWith { case error =>
      throwIfAbortedByUser()
      error match
        case js.JavaScriptException(cause) if Ky.isRawNetworkError(cause) => Future.failed(NetworkError(sent, cause))
        case error => Future.failed(error)
    }

  /** utils/timeout.js: the transport raced against a timer that aborts the controller and fails
    * with a TimeoutError. A response arriving after the timeout is discarded.
    */
  private def race(request: Request, milliseconds: Double): Future[Response] =
    val outcome = Promise[Response]()
    val timer = timers.setTimeout(milliseconds) {
      abortController.abort()
      outcome.tryFailure(TimeoutError(request))
    }
    transport(request).onComplete(outcome.tryComplete)
    outcome.future.andThen(_ => timers.clearTimeout(timer))

  /** #calculateDelay(): the retry.delay backoff, capped by retry.backoffLimit. */
  private def calculateDelay(): Double =
    val retryDelay = retry.delay(retryCount + 1)
    if !(retryDelay >= 0) then throw Ky.typeError("`retry.delay` must return a non-negative number or `Infinity`")
    js.Math.min(retry.backoffLimit, retryDelay)

  /** #calculateRetryDelay(): the wait before retrying the error, which is thrown again when it is
    * not retried.
    */
  private def calculateRetryDelay(error: Throwable): Double =
    if retryCount >= retry.limit then throw error
    if !retry.methods.contains(Ky.normalizeRetryMethod(request.method)) then throw error
    error match
      case _: TimeoutError => if retry.retryOnTimeout then calculateDelay() else throw error
      case HTTPError(response, _, _) =>
        val status = response.status.toDouble
        if !retry.statusCodes.contains(status) then throw error
        RetryTiming.getRetryTimingHeader(response.headers) match
          case Some(header) if retry.afterStatusCodes.contains(status) =>
            // Malformed retry timing headers should not disable retries; they only lose their
            // server-provided timing. No jitter applies to a server's timing.
            RetryTiming.calculateRetryTimingDelay(header).fold(calculateDelay())(js.Math.min(retry.maxRetryAfter, _))
          case _ if status == 413 => throw error
          case _ => calculateDelay()
      case _: NetworkError => calculateDelay()
      // Only known retriable error types are retried.
      case _ => throw error

  /** #throwProcessedError(): the beforeError hooks see the error, which then fails the call. */
  def processedError(error: Throwable): Throwable =
    hooks.beforeError.foreach(_(sentRequest, error, retryCount))
    error
