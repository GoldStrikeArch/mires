// jsdiff 9.0.0 diff/base.js with the character, line, JSON and array diffs.

package diff

import scala.scalajs.js
import scala.scalajs.js.JSStringOps.*

/** The synchronous options of the contract; patch creation passes none of them. */
final case class DiffOptions(
    ignoreCase: Boolean = false,
    oneChangePerToken: Boolean = false,
    stripTrailingCr: Boolean = false,
    ignoreWhitespace: Boolean = false,
    newlineIsToken: Boolean = false,
    ignoreNewlineAtEof: Boolean = false
)

/** A change object: `count` tokens kept, added or removed, and their joined `value`. */
final class Change[V](val count: Int, val added: Boolean, val removed: Boolean, var value: V)

/** An edit-history node; each path shares its predecessors with its neighbours. */
private final class Component(val count: Int, val added: Boolean, val removed: Boolean, val previous: Option[Component])

private final class Path(var oldPos: Int, var lastComponent: Option[Component])

/** base.js: Myers' O(ND) search over tokens of type `T`, joined into values of type `V`. */
abstract class Diff[T, V]:
  protected def useLongestToken: Boolean = false
  protected def removeEmpty(tokens: IndexedSeq[T]): IndexedSeq[T] = tokens
  protected def tokensEqual(left: T, right: T, options: DiffOptions): Boolean = left == right
  protected def join(tokens: IndexedSeq[T]): V
  protected def postProcess(changes: IndexedSeq[Change[V]], options: DiffOptions): IndexedSeq[Change[V]] = changes

  /** The longer of an old and a new token that compare equal (diffJson's dangling commas). */
  protected def longest(oldToken: T, newToken: T): T = newToken

  final def diffTokens(oldTokens: IndexedSeq[T], newTokens: IndexedSeq[T], options: DiffOptions): IndexedSeq[Change[V]] =
    val oldNonEmpty = removeEmpty(oldTokens)
    val newNonEmpty = removeEmpty(newTokens)
    postProcess(buildValues(search(oldNonEmpty, newNonEmpty, options), newNonEmpty, oldNonEmpty), options)

  private def search(oldTokens: IndexedSeq[T], newTokens: IndexedSeq[T], options: DiffOptions): Option[Component] =
    val newLen = newTokens.length
    val oldLen = oldTokens.length
    val maxEditLength = newLen + oldLen
    // The best path on each diagonal, which runs from -maxEditLength - 1 to maxEditLength + 1.
    val bestPath = new Array[Path](2 * maxEditLength + 3)
    def at(diagonal: Int) = diagonal + maxEditLength + 1
    val origin = Path(-1, None)
    bestPath(at(0)) = origin
    var newPos = extractCommon(origin, newTokens, oldTokens, 0, options)
    if origin.oldPos + 1 >= oldLen && newPos + 1 >= newLen then return origin.lastComponent
    // Once a path reaches the right or bottom edge, the diagonals beyond it are pointless.
    var minDiagonalToConsider = Int.MinValue
    var maxDiagonalToConsider = Int.MaxValue
    var editLength = 1
    while editLength <= maxEditLength do
      var diagonalPath = math.max(minDiagonalToConsider, -editLength)
      while diagonalPath <= math.min(maxDiagonalToConsider, editLength) do
        val removePath = bestPath(at(diagonalPath - 1))
        val addPath = bestPath(at(diagonalPath + 1))
        // No one else is going to use this value.
        if removePath != null then bestPath(at(diagonalPath - 1)) = null
        // The new position after an insertion must lie inside the new tokens.
        val canAdd = addPath != null && addPath.oldPos - diagonalPath >= 0 && addPath.oldPos - diagonalPath < newLen
        val canRemove = removePath != null && removePath.oldPos + 1 < oldLen
        if !canAdd && !canRemove then bestPath(at(diagonalPath)) = null
        else
          // Branch from the path that is furthest along the old tokens.
          val basePath =
            if !canRemove || (canAdd && removePath.oldPos < addPath.oldPos) then
              addToPath(addPath, added = true, removed = false, oldPosInc = 0, options)
            else addToPath(removePath, added = false, removed = true, oldPosInc = 1, options)
          newPos = extractCommon(basePath, newTokens, oldTokens, diagonalPath, options)
          if basePath.oldPos + 1 >= oldLen && newPos + 1 >= newLen then return basePath.lastComponent
          bestPath(at(diagonalPath)) = basePath
          if basePath.oldPos + 1 >= oldLen then maxDiagonalToConsider = math.min(maxDiagonalToConsider, diagonalPath - 1)
          if newPos + 1 >= newLen then minDiagonalToConsider = math.max(minDiagonalToConsider, diagonalPath + 1)
        diagonalPath += 2
      editLength += 1
    throw IllegalStateException("the edit graph has no path")

  private def addToPath(path: Path, added: Boolean, removed: Boolean, oldPosInc: Int, options: DiffOptions): Path =
    val component = path.lastComponent match
      case Some(last) if !options.oneChangePerToken && last.added == added && last.removed == removed =>
        Component(last.count + 1, added, removed, last.previous)
      case last => Component(1, added, removed, last)
    Path(path.oldPos + oldPosInc, Some(component))

  private def extractCommon(
      basePath: Path,
      newTokens: IndexedSeq[T],
      oldTokens: IndexedSeq[T],
      diagonalPath: Int,
      options: DiffOptions
  ): Int =
    var oldPos = basePath.oldPos
    var newPos = oldPos - diagonalPath
    var commonCount = 0
    while newPos + 1 < newTokens.length && oldPos + 1 < oldTokens.length &&
      tokensEqual(oldTokens(oldPos + 1), newTokens(newPos + 1), options)
    do
      newPos += 1
      oldPos += 1
      commonCount += 1
      if options.oneChangePerToken then
        basePath.lastComponent = Some(Component(1, added = false, removed = false, basePath.lastComponent))
    if commonCount > 0 && !options.oneChangePerToken then
      basePath.lastComponent = Some(Component(commonCount, added = false, removed = false, basePath.lastComponent))
    basePath.oldPos = oldPos
    newPos

  private def buildValues(last: Option[Component], newTokens: IndexedSeq[T], oldTokens: IndexedSeq[T]): IndexedSeq[Change[V]] =
    var components = List.empty[Component]
    var next = last
    while next.isDefined do
      components = next.get :: components
      next = next.get.previous
    var newPos = 0
    var oldPos = 0
    components.toIndexedSeq.map: component =>
      val tokens =
        if !component.removed then
          val kept = newTokens.slice(newPos, newPos + component.count)
          val value =
            if !component.added && useLongestToken then kept.indices.map(i => longest(oldTokens(oldPos + i), kept(i)))
            else kept
          newPos += component.count
          if !component.added then oldPos += component.count
          value
        else
          val removed = oldTokens.slice(oldPos, oldPos + component.count)
          oldPos += component.count
          removed
      Change(component.count, component.added, component.removed, join(tokens))

/** A diff over string tokens, which are dropped when empty and compared case-insensitively on request. */
abstract class TextDiff extends Diff[String, String]:
  def castInput(value: Any): String = value.asInstanceOf[String]
  def tokenize(value: String, options: DiffOptions): IndexedSeq[String]

  final def diff(oldValue: Any, newValue: Any, options: DiffOptions): IndexedSeq[Change[String]] =
    diffTokens(tokenize(castInput(oldValue), options), tokenize(castInput(newValue), options), options)

  override protected def removeEmpty(tokens: IndexedSeq[String]): IndexedSeq[String] = tokens.filter(_.nonEmpty)

  // Scala.js compiles toLowerCase to the host's.
  override protected def tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
    left == right || (options.ignoreCase && left.toLowerCase == right.toLowerCase)

  protected def join(tokens: IndexedSeq[String]): String = tokens.mkString

/** character.js: tokens are code points (`Array.from`), so a lone surrogate is one token. */
object CharacterDiff extends TextDiff:
  def tokenize(value: String, options: DiffOptions): IndexedSeq[String] = codePoints(value)

def codePoints(value: String): IndexedSeq[String] =
  val points = IndexedSeq.newBuilder[String]
  var i = 0
  while i < value.length do
    val width = if value(i).isHighSurrogate && i + 1 < value.length && value(i + 1).isLowSurrogate then 2 else 1
    points += value.substring(i, i + width)
    i += width
  points.result()

/** line.js `tokenize`: lines with their terminators, or the terminators as separate tokens. */
def lineTokens(value: String, options: DiffOptions): IndexedSeq[String] =
  val text = if options.stripTrailingCr then value.replace("\r\n", "\n") else value
  // text.split(/(\n|\r\n)/): each \n is a separator, together with a \r before it.
  val linesAndNewlines = collection.mutable.ArrayBuffer.empty[String]
  var start = 0
  for i <- text.indices if text(i) == '\n' do
    val separator = if i > start && text(i - 1) == '\r' then i - 1 else i
    linesAndNewlines += text.substring(start, separator)
    linesAndNewlines += text.substring(separator, i + 1)
    start = i + 1
  linesAndNewlines += text.substring(start)
  // Ignore the final empty token that occurs if the string ends with a new line.
  if linesAndNewlines.last.isEmpty then linesAndNewlines.remove(linesAndNewlines.length - 1)
  val retLines = collection.mutable.ArrayBuffer.empty[String]
  for (line, i) <- linesAndNewlines.zipWithIndex do
    if i % 2 == 1 && !options.newlineIsToken then retLines(retLines.length - 1) += line
    else retLines += line
  retLines.toIndexedSeq

object LineDiff extends TextDiff:
  def tokenize(value: String, options: DiffOptions): IndexedSeq[String] = lineTokens(value, options)

  override protected def tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
    // With newlineIsToken, a newline token is not trimmed down to an empty line.
    def normalised(token: String) =
      if options.ignoreWhitespace then
        if !options.newlineIsToken || !token.contains('\n') then token.jsTrim else token
      else if options.ignoreNewlineAtEof && !options.newlineIsToken then token.stripSuffix("\n")
      else token
    super.tokensEqual(normalised(left), normalised(right), options)

/** json.js: a pretty-printed canonical document, diffed by lines. */
object JsonDiff extends TextDiff:
  override protected def useLongestToken: Boolean = true

  override def castInput(value: Any): String =
    if js.typeOf(value) == "string" then value.asInstanceOf[String]
    else js.JSON.stringify(canonicalize(value).asInstanceOf[js.Any], null, "  ")

  def tokenize(value: String, options: DiffOptions): IndexedSeq[String] = lineTokens(value, options)

  // value.replace(/,([\r\n])/g, '$1'); replace starts a global RegExp from lastIndex 0.
  private val DanglingComma = js.RegExp(",([\\r\\n])", "g")
  private def withoutDanglingComma(token: String) = token.jsReplace(DanglingComma, "$1")

  override protected def tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
    super.tokensEqual(withoutDanglingComma(left), withoutDanglingComma(right), options)

  override protected def longest(oldToken: String, newToken: String): String =
    if oldToken.length > newToken.length then oldToken else newToken

/** `canonicalize` for JSON documents: arrays in order, object keys sorted and assigned to a plain
  * object, so a "__proto__" key sets the prototype and is not serialized. JSON input has no cycles
  * and no `toJSON`, and the default replacer keeps every value. Arrays are recognised with
  * `Array.isArray`, which does not depend on the value's realm, as npm's `[object Array]` test does
  * not; and as in npm each level recurses from a loop, so a nesting level costs one stack frame.
  */
def canonicalize(value: Any): Any =
  if js.Array.isArray(value) then
    val array = value.asInstanceOf[js.Array[Any]]
    val canonical = new js.Array[Any](array.length)
    var i = 0
    while i < array.length do
      canonical(i) = canonicalize(array(i))
      i += 1
    canonical
  else if js.typeOf(value) == "object" && value != null then
    val canonical = js.Dynamic.literal()
    val dynamic = value.asInstanceOf[js.Dynamic]
    val keys = js.Object.keys(value.asInstanceOf[js.Object]).sorted
    var i = 0
    while i < keys.length do
      canonical.updateDynamic(keys(i))(canonicalize(dynamic.selectDynamic(keys(i))).asInstanceOf[js.Any])
      i += 1
    canonical
  else value

/** array.js: tokens are the JSON values themselves, compared with `===`. */
object ArrayDiff extends Diff[Any, js.Array[Any]]:
  override protected def tokensEqual(left: Any, right: Any, options: DiffOptions): Boolean =
    js.special.strictEquals(left, right)

  protected def join(tokens: IndexedSeq[Any]): js.Array[Any] = js.Array(tokens*)
