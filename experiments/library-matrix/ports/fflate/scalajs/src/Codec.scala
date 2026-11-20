// UTF-8 and Latin-1 string codecs (`strToU8`, `strFromU8`, and UTF-16LE encoding for the
// harness output contract). Uses the host WHATWG TextEncoder / TextDecoder facades, matching
// fflate 0.8.3's browser/Node path (single leading UTF-8 BOM strip, maximal-subpart U+FFFD).

package fflate

import scala.scalajs.js
import scala.scalajs.js.annotation.JSGlobal
import scala.scalajs.js.typedarray.Uint8Array

@js.native
@JSGlobal("TextEncoder")
private class JsTextEncoder extends js.Object:
  def encode(input: String): Uint8Array = js.native

@js.native
@JSGlobal("TextDecoder")
private class JsTextDecoder extends js.Object:
  def decode(input: Uint8Array): String = js.native

object Codec:
  // Stateless host codec instances matching fflate's module-level `te` / `td`.
  private val textEncoder = new JsTextEncoder()
  private val textDecoder = new JsTextDecoder()

  def strToBytes(str: String, latin1: Boolean): Array[Byte] =
    if latin1 then
      val len = str.length
      val out = new Array[Byte](len)
      var i = 0
      while i < len do
        out(i) = str.charAt(i).toByte
        i += 1
      out
    else Tables.u8ToBytes(textEncoder.encode(str))

  def strToU8(str: String, latin1: Boolean): Uint8Array =
    if latin1 then Tables.bytesToU8(strToBytes(str, true))
    else textEncoder.encode(str)

  def strFromU8(dat: Array[Byte], latin1: Boolean): String =
    if latin1 then
      val len = dat.length
      val chars = new Array[Char](len)
      var i = 0
      while i < len do
        chars(i) = (dat(i) & 0xff).toChar
        i += 1
      new String(chars)
    else textDecoder.decode(Tables.bytesToU8(dat))

  def stringToUtf16Le(text: String): Uint8Array =
    val len = text.length
    val out = new Array[Byte](len * 2)
    var i = 0
    while i < len do
      val u = text.charAt(i).toInt
      out(i * 2) = u.toByte
      out(i * 2 + 1) = (u >>> 8).toByte
      i += 1
    Tables.bytesToU8(out)
