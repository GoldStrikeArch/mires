// The host APIs Ky drives: Fetch (Headers, Request, Response), AbortController and AbortSignal and
// TextDecoder, as Scala.js facades. Ky is a thin layer over these, so their semantics (method and
// URL normalisation, header validation, body encoding, encoding labels) are the engine's own. A JS
// exception one of them throws reaches Scala as `js.JavaScriptException` holding the JS error.

package ky

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSGlobal, JSName}
import scala.scalajs.js.typedarray.ArrayBuffer

@js.native
@JSGlobal
class Headers() extends js.Object:
  /** Validates the name and the value (a TypeError) and stores the value's ByteString conversion,
    * so any JSON value is accepted as JS accepts it.
    */
  def set(name: String, value: Any): Unit = js.native
  def get(name: String): String = js.native

@js.native
@JSGlobal
class Request(input: js.Any, init: js.Object) extends js.Object:
  val method: String = js.native
  val url: String = js.native
  val headers: Headers = js.native
  val signal: AbortSignal = js.native
  @JSName("clone") def copy(): Request = js.native
  def text(): js.Promise[String] = js.native

@js.native
@JSGlobal
class Response(content: Any, init: js.Object) extends js.Object:
  val status: Int = js.native
  val ok: Boolean = js.native
  val headers: Headers = js.native
  val body: js.Any = js.native
  def text(): js.Promise[String] = js.native
  def arrayBuffer(): js.Promise[ArrayBuffer] = js.native

@js.native
@JSGlobal
class AbortController() extends js.Object:
  val signal: AbortSignal = js.native
  /** Without a reason, the platform's AbortError DOMException. */
  def abort(reason: js.Any = js.native): Unit = js.native

@js.native
@JSGlobal
class AbortSignal extends js.Object:
  val aborted: Boolean = js.native
  val reason: js.Any = js.native
  def throwIfAborted(): Unit = js.native
  def addEventListener(`type`: String, listener: js.Function0[Unit], options: js.Object): Unit = js.native

@js.native
@JSGlobal
object AbortSignal extends js.Object:
  /** Aborted as soon as one of the signals is, with that one's reason. */
  def any(signals: js.Array[AbortSignal]): AbortSignal = js.native

@js.native
@JSGlobal
class TextDecoder(label: String = js.native) extends js.Object:
  def decode(input: ArrayBuffer): String = js.native

@js.native
@JSGlobal("Number")
object JsNumber extends js.Object:
  def isInteger(value: Any): Boolean = js.native
