// jsdiff 9.0.0 diff/word.js and the whitespace helpers of util/string.js.

package diff

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.JSStringOps.*

object WordDiff extends TextDiff:
  // Latin letters with diacritics and the spacing modifier letters count as word characters.
  private val ExtendedWordChars =
    "a-zA-Z0-9_\\u{AD}\\u{C0}-\\u{D6}\\u{D8}-\\u{F6}\\u{F8}-\\u{2C6}\\u{2C8}-\\u{2D7}\\u{2DE}-\\u{2FF}\\u{1E00}-\\u{1EFF}"
  // match() starts a global RegExp from lastIndex 0, and test() of a non-global one ignores it.
  private val TokenizeIncludingWhitespace = js.RegExp(s"[$ExtendedWordChars]+|\\s+|[^$ExtendedWordChars]", "ug")
  private val Whitespace = js.RegExp("\\s")

  private def isWhitespaceRun(part: String) = Whitespace.test(part)

  def tokenize(value: String, options: DiffOptions): IndexedSeq[String] =
    // Whitespace runs are joined to the neighbouring word or punctuation tokens.
    val parts = value.jsMatch(TokenizeIncludingWhitespace)
    val tokens = ArrayBuffer.empty[String]
    var prevPart: Option[String] = None
    if parts != null then
      for part <- parts do
        if isWhitespaceRun(part) then
          if prevPart.isEmpty then tokens += part else tokens += tokens.remove(tokens.length - 1) + part
        else
          prevPart match
            case Some(prev) if isWhitespaceRun(prev) =>
              if tokens.last == prev then tokens += tokens.remove(tokens.length - 1) + part else tokens += prev + part
            case _ => tokens += part
        prevPart = Some(part)
    tokens.toIndexedSeq

  override protected def tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
    if options.ignoreCase then left.toLowerCase.jsTrim == right.toLowerCase.jsTrim else left.jsTrim == right.jsTrim

  // Tokens that are joined appeared consecutively, so each but the first drops its leading whitespace.
  override protected def join(tokens: IndexedSeq[String]): String =
    tokens.zipWithIndex.map((token, i) => if i == 0 then token else token.substring(leadingWs(token).length)).mkString

  override protected def postProcess(changes: IndexedSeq[Change[String]], options: DiffOptions): IndexedSeq[Change[String]] =
    if !options.oneChangePerToken then
      var lastKeep: Option[Change[String]] = None
      // The insertion and deletion since the last kept change; there is at most one of each.
      var insertion: Option[Change[String]] = None
      var deletion: Option[Change[String]] = None
      for change <- changes do
        if change.added then insertion = Some(change)
        else if change.removed then deletion = Some(change)
        else
          if insertion.isDefined || deletion.isDefined then dedupeWhitespace(lastKeep, deletion, insertion, Some(change))
          lastKeep = Some(change)
          insertion = None
          deletion = None
      if insertion.isDefined || deletion.isDefined then dedupeWhitespace(lastKeep, deletion, insertion, None)
    changes

  /** dedupeWhitespaceInChangeObjects: whitespace that trails one change object and leads the next is
    * kept once, on the side where both texts have it.
    */
  private def dedupeWhitespace(
      startKeep: Option[Change[String]],
      deletion: Option[Change[String]],
      insertion: Option[Change[String]],
      endKeep: Option[Change[String]]
  ): Unit = (deletion, insertion) match
    case (Some(deletion), Some(insertion)) =>
      val oldWsPrefix = leadingWs(deletion.value)
      val oldWsSuffix = trailingWs(deletion.value)
      val newWsPrefix = leadingWs(insertion.value)
      val newWsSuffix = trailingWs(insertion.value)
      for startKeep <- startKeep do
        val commonWsPrefix = longestCommonPrefix(oldWsPrefix, newWsPrefix)
        startKeep.value = replaceSuffix(startKeep.value, newWsPrefix, commonWsPrefix)
        deletion.value = removePrefix(deletion.value, commonWsPrefix)
        insertion.value = removePrefix(insertion.value, commonWsPrefix)
      for endKeep <- endKeep do
        val commonWsSuffix = longestCommonSuffix(oldWsSuffix, newWsSuffix)
        endKeep.value = replacePrefix(endKeep.value, newWsSuffix, commonWsSuffix)
        deletion.value = removeSuffix(deletion.value, commonWsSuffix)
        insertion.value = removeSuffix(insertion.value, commonWsSuffix)
    case (None, Some(insertion)) =>
      // The whitespace is all from the new text: each object keeps its trailing whitespace.
      if startKeep.isDefined then insertion.value = insertion.value.substring(leadingWs(insertion.value).length)
      for endKeep <- endKeep do endKeep.value = endKeep.value.substring(leadingWs(endKeep.value).length)
    case (Some(deletion), None) =>
      (startKeep, endKeep) match
        case (Some(startKeep), Some(endKeep)) =>
          val newWsFull = leadingWs(endKeep.value)
          val delWsStart = leadingWs(deletion.value)
          val delWsEnd = trailingWs(deletion.value)
          // Whitespace right after startKeep in both texts goes to startKeep.
          val newWsStart = longestCommonPrefix(newWsFull, delWsStart)
          deletion.value = removePrefix(deletion.value, newWsStart)
          // Whitespace right before endKeep in both texts, not given to startKeep, goes to endKeep.
          val newWsEnd = longestCommonSuffix(removePrefix(newWsFull, newWsStart), delWsEnd)
          deletion.value = removeSuffix(deletion.value, newWsEnd)
          endKeep.value = replacePrefix(endKeep.value, newWsFull, newWsEnd)
          // Any whitespace from the new text not yet assigned goes to the start.
          startKeep.value = replaceSuffix(startKeep.value, newWsFull, newWsFull.substring(0, newWsFull.length - newWsEnd.length))
        case (None, Some(endKeep)) =>
          // At the start of the text: drop the deletion's trailing whitespace that endKeep repeats.
          deletion.value = removeSuffix(deletion.value, maximumOverlap(trailingWs(deletion.value), leadingWs(endKeep.value)))
        case (Some(startKeep), None) =>
          // At the end of the text: drop the deletion's leading whitespace that startKeep repeats.
          deletion.value = removePrefix(deletion.value, maximumOverlap(trailingWs(startKeep.value), leadingWs(deletion.value)))
        case (None, None) => ()
    case (None, None) => ()

/** leadingWs: the whitespace units at the start. */
def leadingWs(s: String): String = s.takeWhile(isJsWhitespace)

/** trailingWs: the whitespace units at the end. */
def trailingWs(s: String): String = s.substring(s.lastIndexWhere(c => !isJsWhitespace(c)) + 1)

private def longestCommonPrefix(a: String, b: String): String =
  var i = 0
  while i < a.length && i < b.length && a(i) == b(i) do i += 1
  a.substring(0, i)

private def longestCommonSuffix(a: String, b: String): String =
  var i = 0
  while i < a.length && i < b.length && a(a.length - 1 - i) == b(b.length - 1 - i) do i += 1
  a.substring(a.length - i)

private def replacePrefix(string: String, oldPrefix: String, newPrefix: String): String =
  require(string.startsWith(oldPrefix), "string doesn't start with prefix; this is a bug")
  newPrefix + string.substring(oldPrefix.length)

private def replaceSuffix(string: String, oldSuffix: String, newSuffix: String): String =
  require(string.endsWith(oldSuffix), "string doesn't end with suffix; this is a bug")
  string.substring(0, string.length - oldSuffix.length) + newSuffix

private def removePrefix(string: String, oldPrefix: String) = replacePrefix(string, oldPrefix, "")
private def removeSuffix(string: String, oldSuffix: String) = replaceSuffix(string, oldSuffix, "")

/** The longest suffix of `a` that is a prefix of `b`, by Knuth-Morris-Pratt (overlapCount). */
private def maximumOverlap(a: String, b: String): String =
  val startA = if a.length > b.length then a.length - b.length else 0
  val endB = math.min(a.length, b.length)
  val map = new Array[Int](endB)
  var k = 0
  for j <- 1 until endB do
    map(j) = if b(j) == b(k) then map(k) else k
    while k > 0 && b(j) != b(k) do k = map(k)
    if b(j) == b(k) then k += 1
  k = 0
  for i <- startA until a.length do
    while k > 0 && a(i) != b(k) do k = map(k)
    if a(i) == b(k) then k += 1
  b.substring(0, k)
