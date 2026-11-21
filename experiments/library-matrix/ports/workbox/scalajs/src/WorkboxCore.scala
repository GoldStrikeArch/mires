// workbox-core: WorkboxError with its production messages, copyResponse, cacheMatchIgnoreParams,
// timeout, waitUntil, and the ExtendableEvent a strategy, install or activate runs under.

package workbox

import scala.concurrent.{Future, Promise}
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.timers

/** `new WorkboxError(code, details)` from the production build: the message is
  * `code :: JSON.stringify([details])`, so absent details print `[null]`. `name` is the code.
  */
final class WorkboxError(val name: String, val details: js.UndefOr[js.Object] = js.undefined)
    extends Exception(name + " :: " + js.JSON.stringify(js.Array(details)))

extension (error: Throwable)
  /** What `JSON.stringify` prints for an error nested in a WorkboxError's details: a
    * WorkboxError's own enumerable properties (`name`, `details`), and nothing for a native Error.
    */
  def asDetail: js.Object = error match
    case workboxError: WorkboxError => js.Dynamic.literal(name = workboxError.name, details = workboxError.details)
    case _ => js.Object()

/** The event a strategy, `install` or `activate` runs under: a native ExtendableEvent, or the
  * stand-in a page supplies.
  */
trait ExtendableEvent:
  def eventType: String
  def waitUntil(promise: Future[Any]): Unit

/** `waitUntil(event, asyncFn)`: the started call extends the event's lifetime and is returned. */
def waitUntil[T](event: ExtendableEvent)(asyncFn: => Future[T]): Future[T] =
  val returnPromise = async(asyncFn)
  event.waitUntil(returnPromise)
  returnPromise

/** `timeout(ms)`: one timer task. */
def timeout(ms: Int): Future[Unit] =
  val elapsed = Promise[Unit]()
  timers.setTimeout(ms.toDouble)(elapsed.success(()))
  elapsed.future

/** `copyResponse(response)`: refuses a response from another origin (or without a URL), then
  * builds a new Response from a clone's headers, status, status text and body. Workbox's
  * `canConstructResponseFromBodyStream` always answers false, so the body is read as a Blob.
  */
def copyResponse(response: Response): Future[Response] = async {
  val origin = Option.when(response.url.nonEmpty)(URL(response.url).origin)
  if !origin.contains(Globals.location.origin) then
    throw WorkboxError("cross-origin-copy-response", js.Dynamic.literal(origin = origin.orNull))
  val clonedResponse = response.copy()
  val responseInit = js.Dynamic.literal(
    headers = Headers(clonedResponse.headers),
    status = clonedResponse.status,
    statusText = clonedResponse.statusText,
  )
  clonedResponse.blob().toFuture.map(body => Response(body, responseInit))
}

private def stripParams(fullURL: String, ignoreParams: Seq[String]): String =
  val strippedURL = URL(fullURL)
  ignoreParams.foreach(strippedURL.searchParams.delete)
  strippedURL.href

/** `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when the request has
  * none of the parameters, else the first key that is the request once both are stripped.
  */
def cacheMatchIgnoreParams(cache: Cache, request: Request, ignoreParams: Seq[String]): Future[Option[Response]] = async {
  val strippedRequestURL = stripParams(request.url, ignoreParams)
  if request.url == strippedRequestURL then cache.matching(request).toFuture.map(_.toOption)
  else
    cache.keys(request, js.Dynamic.literal(ignoreSearch = true)).toFuture.flatMap { cacheKeys =>
      cacheKeys.find(cacheKey => strippedRequestURL == stripParams(cacheKey.url, ignoreParams)) match
        case Some(cacheKey) => cache.matching(cacheKey).toFuture.map(_.toOption)
        case None => Future.successful(None)
    }
}
