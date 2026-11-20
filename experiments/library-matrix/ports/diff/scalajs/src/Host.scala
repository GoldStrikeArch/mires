// The host facilities the port uses besides what Scala.js compiles to the host itself. Scala.js
// strings follow java.lang.String, whose trim() and split() differ from ECMAScript's: jsdiff trims
// ECMAScript white space and line terminators, and its regexes are host RegExps (js.RegExp here,
// as jsdiff writes them, rather than java.util.regex's translation). TextEncoder and TextDecoder
// are what jsdiff itself calls for quoted names: Java's UTF-8 codec writes '?' for an isolated
// surrogate and does not replace each maximal ill-formed subpart with U+FFFD.

package diff

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSGlobal, JSName}
import scala.scalajs.js.typedarray.Uint8Array

@js.native
private trait JsString extends js.Any:
  def trim(): String = js.native
  @JSName("match") def matchAll(regexp: js.RegExp): js.Array[String] = js.native

extension (s: String)
  /** `String.prototype.trim`. */
  def jsTrim: String = s.asInstanceOf[JsString].trim()

  /** `String.prototype.match` with a global RegExp: every match, or null when there is none. */
  def jsMatch(regexp: js.RegExp): js.Array[String] = s.asInstanceOf[JsString].matchAll(regexp)

/** The class `\s`: ECMAScript white space and line terminators, one UTF-16 unit at a time. */
def isJsWhitespace(c: Char): Boolean = c match
  case '\t' | '\n' | '\u000b' | '\u000c' | '\r' | ' ' | ' ' | ' ' | ' ' | ' ' | ' ' |
      ' ' | '　' | '﻿' =>
    true
  case _ => c >= ' ' && c <= ' '

@js.native
@JSGlobal
private class TextEncoder() extends js.Object:
  def encode(input: String): Uint8Array = js.native

@js.native
@JSGlobal
private class TextDecoder() extends js.Object:
  def decode(input: Uint8Array): String = js.native

/** TextEncoder's UTF-8, which writes U+FFFD for an isolated surrogate. */
def textEncode(s: String): IndexedSeq[Int] =
  val bytes = TextEncoder().encode(s)
  IndexedSeq.tabulate(bytes.length)(bytes(_).toInt)

/** TextDecoder's UTF-8: each maximal ill-formed subpart becomes U+FFFD and a leading BOM is consumed. */
def textDecode(bytes: collection.Seq[Int]): String =
  val array = Uint8Array(bytes.length)
  for (b, i) <- bytes.zipWithIndex do array(i) = (b & 0xff).toShort
  TextDecoder().decode(array)
