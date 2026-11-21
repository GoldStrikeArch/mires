// The Web APIs Workbox runs on, as Scala.js facades: CacheStorage and Cache, fetch, Request,
// Response and Headers, URL and URLSearchParams, and the JS globals the adapter reads. A JS
// exception one of them throws, or a rejection, reaches Scala as `js.JavaScriptException` holding
// the JS value.

package workbox

import scala.concurrent.{ExecutionContext, Future}
import scala.scalajs.concurrent.JSExecutionContext
import scala.scalajs.js
import scala.scalajs.js.annotation.{JSGlobal, JSGlobalScope, JSName}
import scala.util.control.NonFatal

/** Workbox's continuations run as promise jobs, as an async function's do. */
given ExecutionContext = JSExecutionContext.queue

@js.native
@JSGlobal
class Headers(init: Headers) extends js.Object

@js.native
@JSGlobal
class Request(input: Request | String, init: js.Object = js.native) extends js.Object:
  val url: String = js.native
  val method: String = js.native
  val mode: String = js.native
  val integrity: String = js.native
  val headers: Headers = js.native
  @JSName("clone") def copy(): Request = js.native

@js.native
@JSGlobal
class Response(body: js.Any, init: js.Object) extends js.Object:
  val status: Int = js.native
  val statusText: String = js.native
  val url: String = js.native
  val redirected: Boolean = js.native
  @JSName("type") val responseType: String = js.native
  val headers: Headers = js.native
  @JSName("clone") def copy(): Response = js.native
  def text(): js.Promise[String] = js.native
  def blob(): js.Promise[js.Any] = js.native

@js.native
trait URLSearchParams extends js.Object:
  def keys(): js.Iterable[String] = js.native
  def set(name: String, value: js.Any): Unit = js.native
  def delete(name: String): Unit = js.native

@js.native
@JSGlobal
class URL(url: String, base: String = js.native) extends js.Object:
  val href: String = js.native
  val origin: String = js.native
  val protocol: String = js.native
  var pathname: String = js.native
  var hash: String = js.native
  val searchParams: URLSearchParams = js.native

@js.native
trait Cache extends js.Object:
  @JSName("match") def matching(request: Request | String): js.Promise[js.UndefOr[Response]] = js.native
  def put(request: Request, response: Response): js.Promise[Unit] = js.native
  def delete(request: Request): js.Promise[Boolean] = js.native
  def keys(): js.Promise[js.Array[Request]] = js.native
  def keys(request: Request, options: js.Object): js.Promise[js.Array[Request]] = js.native

@js.native
trait CacheStorage extends js.Object:
  def open(cacheName: String): js.Promise[Cache] = js.native
  @JSName("match") def matching(request: Request, options: js.Object): js.Promise[js.UndefOr[Response]] = js.native
  def keys(): js.Promise[js.Array[String]] = js.native
  def delete(cacheName: String): js.Promise[Boolean] = js.native

@js.native
trait Location extends js.Object:
  val href: String = js.native
  val origin: String = js.native

@js.native
trait Crypto extends js.Object:
  def randomUUID(): String = js.native

@js.native
trait Console extends js.Object:
  def warn(message: String): Unit = js.native

@js.native
@JSGlobalScope
object Globals extends js.Object:
  val caches: CacheStorage = js.native
  val location: Location = js.native
  val crypto: Crypto = js.native
  val console: Console = js.native
  def fetch(request: Request): js.Promise[Response] = js.native

/** The members of `new Request(input, init)` the subset uses; an absent one stays out of the
  * dictionary.
  */
def requestInit(
    method: Option[String] = None,
    mode: Option[String] = None,
    integrity: Option[String] = None,
    cache: Option[String] = None,
    credentials: Option[String] = None,
    headers: Option[Headers] = None,
): js.Object =
  val init = js.Dictionary.empty[js.Any]
  method.foreach(init("method") = _)
  mode.foreach(init("mode") = _)
  integrity.foreach(init("integrity") = _)
  cache.foreach(init("cache") = _)
  credentials.foreach(init("credentials") = _)
  headers.foreach(init("headers") = _)
  init.asInstanceOf[js.Object]

/** Runs `body`, the body of an async function: a synchronous throw rejects the result, as it
  * rejects an async function's promise, instead of escaping to the caller.
  */
inline def async[T](inline body: Future[T]): Future[T] =
  try body
  catch case NonFatal(error) => Future.failed(error)

/** `error instanceof Error`, the guard Workbox applies before it keeps or reports a caught value:
  * Workbox's own errors are Errors, and a JS value is one if it is an instance of the page's Error.
  */
extension (error: Throwable)
  def isError: Boolean = error match
    case js.JavaScriptException(value) => value.isInstanceOf[js.Error]
    case _ => true

  /** The JS `name` and `message` of a thrown Error. */
  def jsName: String = error match
    case workboxError: WorkboxError => workboxError.name
    case js.JavaScriptException(value) => value.asInstanceOf[js.Dynamic].name.asInstanceOf[String]
    case other => other.getClass.getSimpleName

  def jsMessage: js.Any = error match
    case js.JavaScriptException(value) => value.asInstanceOf[js.Dynamic].message
    case other => other.getMessage

/** JavaScript truthiness of a JSON value. */
def isTruthy(value: Any): Boolean = value match
  case null => false
  case text: String => text.nonEmpty
  case number: Double => number != 0 && !number.isNaN
  case flag: Boolean => flag
  case _ => !js.isUndefined(value)
