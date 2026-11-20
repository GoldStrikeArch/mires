// Zero-overhead @js.native facades for ECMAScript String, RegExp, and Object methods
// needed to match marked 18.1.1's exact UTF-16, ECMA-262 whitespace, Unicode case-folding,
// and RegExp semantics without pulling in scala.collection or java.util.regex.

package marked

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSBracketAccess, JSGlobal, JSName}

@js.native
trait LinkMap extends js.Object:
  @JSBracketAccess
  def apply(key: String): js.UndefOr[LinkDef] = js.native
  @JSBracketAccess
  def update(key: String, value: LinkDef): Unit = js.native

@js.native
trait JsArr[A] extends js.Any:
  def slice(start: Int): js.Array[A] = js.native
  def slice(start: Int, end: Int): js.Array[A] = js.native

@js.native
trait JsStr extends js.Any:
  def trim(): String = js.native
  def trimStart(): String = js.native
  def trimEnd(): String = js.native
  def slice(start: Int): String = js.native
  def slice(start: Int, end: Int): String = js.native
  @JSName("replace")
  def replaceStr(search: String, rep: String): String = js.native
  @JSName("replace")
  def replaceRe(re: js.RegExp, rep: String): String = js.native
  @JSName("replace")
  def replaceReFn1(re: js.RegExp, fn: js.Function1[String, String]): String = js.native
  @JSName("replace")
  def replaceReFn3(re: js.RegExp, fn: js.Function3[String, js.UndefOr[String], js.UndefOr[String], String]): String = js.native
  @JSName("replace")
  def replacePipeFn(re: js.RegExp, fn: js.Function3[String, Int, String, String]): String = js.native
  @JSName("split")
  def splitStr(sep: String): js.Array[String] = js.native
  @JSName("split")
  def splitRe(re: js.RegExp): js.Array[String] = js.native
  def search(re: js.RegExp): Int = js.native
  def repeat(count: Int): String = js.native
  def includes(sub: String): Boolean = js.native
  def endsWith(sub: String): Boolean = js.native
  def toLowerCase(): String = js.native
  def toUpperCase(): String = js.native

@js.native
@JSGlobal("String")
object JsStringStatic extends js.Object:
  def fromCodePoint(cp: Int): String = js.native

@js.native
@JSGlobal("Object")
object JsObjectStatic extends js.Object:
  def create(proto: Null): LinkMap = js.native
  def hasOwn(obj: js.Any, prop: String): Boolean = js.native

object JsInterop:
  extension (s: String)
    inline def jsTrim(): String = s.asInstanceOf[JsStr].trim()
    inline def jsTrimStart(): String = s.asInstanceOf[JsStr].trimStart()
    inline def jsTrimEnd(): String = s.asInstanceOf[JsStr].trimEnd()
    inline def jsSlice(start: Int): String = s.asInstanceOf[JsStr].slice(start)
    inline def jsSlice(start: Int, end: Int): String = s.asInstanceOf[JsStr].slice(start, end)
    inline def jsReplaceStr(search: String, rep: String): String = s.asInstanceOf[JsStr].replaceStr(search, rep)
    inline def jsReplaceRe(re: js.RegExp, rep: String): String = s.asInstanceOf[JsStr].replaceRe(re, rep)
    inline def jsReplaceReFn1(re: js.RegExp, fn: js.Function1[String, String]): String =
      s.asInstanceOf[JsStr].replaceReFn1(re, fn)
    inline def jsReplaceReFn3(
        re: js.RegExp,
        fn: js.Function3[String, js.UndefOr[String], js.UndefOr[String], String],
    ): String = s.asInstanceOf[JsStr].replaceReFn3(re, fn)
    inline def jsReplacePipeFn(re: js.RegExp, fn: js.Function3[String, Int, String, String]): String =
      s.asInstanceOf[JsStr].replacePipeFn(re, fn)
    inline def jsSplit(sep: String): js.Array[String] = s.asInstanceOf[JsStr].splitStr(sep)
    inline def jsSplitRe(re: js.RegExp): js.Array[String] = s.asInstanceOf[JsStr].splitRe(re)
    inline def jsSearch(re: js.RegExp): Int = s.asInstanceOf[JsStr].search(re)
    inline def jsRepeat(count: Int): String = s.asInstanceOf[JsStr].repeat(count)
    inline def jsIncludes(sub: String): Boolean = s.asInstanceOf[JsStr].includes(sub)
    inline def jsEndsWith(sub: String): Boolean = s.asInstanceOf[JsStr].endsWith(sub)
    inline def jsToLowerCase(): String = s.asInstanceOf[JsStr].toLowerCase()
    inline def jsToUpperCase(): String = s.asInstanceOf[JsStr].toUpperCase()

  extension [A](a: js.Array[A])
    inline def arrSlice(start: Int): js.Array[A] = a.asInstanceOf[JsArr[A]].slice(start)
    inline def arrSlice(start: Int, end: Int): js.Array[A] = a.asInstanceOf[JsArr[A]].slice(start, end)

  inline def undefStr(u: js.UndefOr[String]): String =
    if js.isUndefined(u) then "" else u.asInstanceOf[String]

  inline def undefStrOrNull(u: js.UndefOr[String]): String | Null =
    if js.isUndefined(u) then null else u.asInstanceOf[String]

  inline def isTruthy(u: js.UndefOr[String]): Boolean =
    !js.isUndefined(u) && u.asInstanceOf[String].length > 0

  inline def firstLineOf(s: String): String =
    val nl = s.indexOf('\n')
    if nl >= 0 then s.substring(0, nl) else s

  /** Counts Unicode code points (valid UTF-16 surrogate pairs count as 1), matching `[...s].length`. */
  def codePointLength(s: String): Int =
    val len = s.length
    var count = 0
    var i = 0
    while i < len do
      val c = s.charAt(i)
      if c >= 0xd800 && c <= 0xdbff && i + 1 < len then
        val lo = s.charAt(i + 1)
        if lo >= 0xdc00 && lo <= 0xdfff then i += 2
        else i += 1
      else i += 1
      count += 1
    count

  /** UTF-16 code-unit length of the first Unicode code point, matching `[...s][0].length`. */
  def firstCodePointUtf16Length(s: String): Int =
    val len = s.length
    if len == 0 then 0
    else
      val c = s.charAt(0)
      if c >= 0xd800 && c <= 0xdbff && len >= 2 then
        val lo = s.charAt(1)
        if lo >= 0xdc00 && lo <= 0xdfff then 2 else 1
      else 1

  /** Parses 1..9 ASCII decimal digits into a non-negative 32-bit Int. */
  def parseDec(s: String): Int =
    val len = s.length
    var acc = 0
    var i = 0
    while i < len do
      acc = acc * 10 + (s.charAt(i) - '0')
      i += 1
    acc

  /** Parses 1..6 ASCII hex digits into a non-negative 32-bit Int. */
  def parseHex(s: String): Int =
    val len = s.length
    var acc = 0
    var i = 0
    while i < len do
      val c = s.charAt(i)
      val d =
        if c >= '0' && c <= '9' then c - '0'
        else if c >= 'a' && c <= 'f' then c - 'a' + 10
        else c - 'A' + 10
      acc = (acc << 4) | d
      i += 1
    acc
