// Helper utilities ported directly from marked 18.1.1 (src/helpers.ts).

package marked

import scala.scalajs.js
import JsInterop.*

object Helpers:
  private val getEscapeReplacement: js.Function1[String, String] = (ch: String) =>
    ch.charAt(0) match
      case '&'  => "&amp;"
      case '<'  => "&lt;"
      case '>'  => "&gt;"
      case '"'  => "&quot;"
      case '\'' => "&#39;"
      case _    => ch

  def escapeHtmlEntities(html: String, encode: Boolean = false): String =
    if encode then
      if Rules.escapeTest.test(html) then html.jsReplaceReFn1(Rules.escapeReplace, getEscapeReplacement)
      else html
    else if Rules.escapeTestNoEncode.test(html) then
      html.jsReplaceReFn1(Rules.escapeReplaceNoEncode, getEscapeReplacement)
    else html

  private val decodeNcrReplacer: js.Function3[String, js.UndefOr[String], js.UndefOr[String], String] =
    (_ref: String, dec: js.UndefOr[String], hex: js.UndefOr[String]) =>
      if js.isUndefined(dec) && js.isUndefined(hex) then "&amp;"
      else
        val code =
          if js.isUndefined(dec) then parseHex(hex.asInstanceOf[String])
          else parseDec(dec.asInstanceOf[String])
        if code == 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) then "\uFFFD"
        else if code == 38 then "&amp;"
        else JsStringStatic.fromCodePoint(code)

  def decodeNumericCharacterReferences(text: String): String =
    if !Rules.numericCharacterReference.test(text) then text
    else text.jsReplaceReFn3(Rules.numericCharacterReferenceOrAmpersand, decodeNcrReplacer)

  def cleanUrl(href: String): String | Null =
    try js.URIUtils.encodeURI(href).jsReplaceRe(Rules.percentDecode, "%")
    catch case _: Throwable => null

  private val pipeReplacer: js.Function3[String, Int, String, String] =
    (_match: String, offset: Int, str: String) =>
      var escaped = false
      var curr = offset - 1
      while curr >= 0 && str.charAt(curr) == '\\' do
        escaped = !escaped
        curr -= 1
      if escaped then "|" else " |"

  def splitCells(tableRow: String, count: Int = 0): js.Array[String] =
    val row = tableRow.jsReplacePipeFn(Rules.findPipe, pipeReplacer)
    val cells = row.jsSplitRe(Rules.splitPipe)
    if cells.length > 0 && cells(0).jsTrim().isEmpty then cells.shift()
    if cells.length > 0 && cells(cells.length - 1).jsTrim().isEmpty then cells.pop()
    if count > 0 then
      if cells.length > count then cells.length = count
      else
        while cells.length < count do cells.push("")
    var i = 0
    val len = cells.length
    while i < len do
      cells(i) = cells(i).jsTrim().jsReplaceRe(Rules.slashPipe, "|")
      i += 1
    cells

  def rtrim(str: String, c: Char, invert: Boolean = false): String =
    val l = str.length
    if l == 0 then ""
    else
      var suffLen = 0
      var done = false
      while suffLen < l && !done do
        val currChar = str.charAt(l - suffLen - 1)
        if currChar == c && !invert then suffLen += 1
        else if currChar != c && invert then suffLen += 1
        else done = true
      str.substring(0, l - suffLen)

  def trimTrailingBlankLines(str: String): String =
    val lines = str.jsSplit("\n")
    var end = lines.length - 1
    while end >= 0 && Rules.blankLine.test(lines(end)) do end -= 1
    if lines.length - end <= 2 then str
    else lines.arrSlice(0, end + 1).join("\n")

  def normalizeLabel(label: String): String =
    label.jsTrim().jsToLowerCase().jsToUpperCase().jsToLowerCase()

  def findClosingBracket(str: String, open: Char, close: Char): Int =
    if str.indexOf(open) == -1 && str.indexOf(close) == -1 then -1
    else
      var level = 0
      val len = str.length
      var i = 0
      while i < len do
        val ch = str.charAt(i)
        if ch == '\\' then i += 1
        else if ch == open then level += 1
        else if ch == close then
          level -= 1
          if level < 0 then return i
        i += 1
      if level > 0 then -2 else -1

  private inline def spaces(n: Int): String =
    if n == 1 then " "
    else if n == 2 then "  "
    else if n == 3 then "   "
    else "    "

  def expandTabs(line: String, indent: Int = 0): String =
    var col = indent
    var expanded = ""
    val len = line.length
    var i = 0
    var chunkStart = 0
    while i < len do
      val c = line.charAt(i)
      if c == '\t' then
        if i > chunkStart then expanded += line.substring(chunkStart, i)
        val added = 4 - (col % 4)
        expanded += spaces(added)
        col += added
        i += 1
        chunkStart = i
      else if c >= 0xd800 && c <= 0xdbff && i + 1 < len then
        val lo = line.charAt(i + 1)
        if lo >= 0xdc00 && lo <= 0xdfff then
          col += 1
          i += 2
        else
          col += 1
          i += 1
      else
        col += 1
        i += 1
    if chunkStart == 0 then line
    else
      if len > chunkStart then expanded += line.substring(chunkStart, len)
      expanded
