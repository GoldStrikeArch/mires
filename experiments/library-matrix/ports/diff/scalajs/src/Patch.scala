// jsdiff 9.0.0 patch/create.js, parse.js, apply.js, reverse.js and line-endings.js.

package diff

import scala.annotation.tailrec
import scala.collection.mutable
import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.JSStringOps.*

/** A hunk. Its ranges are JavaScript numbers: a parsed header can exceed Int or be NaN. */
final class Hunk(var oldStart: Double, var oldLines: Double, var newStart: Double, var newLines: Double, val lines: ArrayBuffer[String]):
  def toJs: js.Object = js.Dynamic.literal(
    oldStart = oldStart,
    oldLines = oldLines,
    newStart = newStart,
    newLines = newLines,
    lines = js.Array(lines.toSeq*)
  )

/** A file patch. `properties` holds its metadata in the order jsdiff first assigns each property,
  * which is the key order of the object it returns; `hunks` stands for the hunks.
  */
final class FilePatch(val hunks: ArrayBuffer[Hunk] = ArrayBuffer.empty):
  val properties = mutable.LinkedHashMap.empty[String, Any]

  def get(key: String): Option[Any] = properties.get(key)
  def update(key: String, value: Any): Unit = properties(key) = value

  def toJs: js.Object =
    val out = js.Dynamic.literal()
    for (key, value) <- properties do
      out.updateDynamic(key)(if key == "hunks" then js.Array(hunks.map(_.toJs).toSeq*) else value.asInstanceOf[js.Any])
    out

// ---- structuredPatch and formatPatch ----

def structuredPatch(
    oldFileName: js.UndefOr[String],
    newFileName: js.UndefOr[String],
    oldStr: String,
    newStr: String,
    oldHeader: js.UndefOr[String],
    newHeader: js.UndefOr[String],
    context: Int
): FilePatch =
  final case class Entry(added: Boolean, removed: Boolean, lines: IndexedSeq[String])
  // Step 1: hunks of lines that keep their newlines; an empty entry at the end closes the last one.
  val diff = LineDiff.diff(oldStr, newStr, DiffOptions()).map(c => Entry(c.added, c.removed, splitLines(c.value))) :+
    Entry(added = false, removed = false, lines = IndexedSeq.empty)
  val hunks = ArrayBuffer.empty[Hunk]
  var oldRangeStart = 0
  var newRangeStart = 0
  var curRange = ArrayBuffer.empty[String]
  var oldLine = 1
  var newLine = 1
  for (current, i) <- diff.zipWithIndex do
    val lines = current.lines
    if current.added || current.removed then
      // If we have previous context, start with that.
      if oldRangeStart == 0 then
        oldRangeStart = oldLine
        newRangeStart = newLine
        if i > 0 then
          curRange = if context > 0 then diff(i - 1).lines.takeRight(context).map(" " + _).to(ArrayBuffer) else ArrayBuffer.empty
          oldRangeStart -= curRange.length
          newRangeStart -= curRange.length
      curRange ++= lines.map((if current.added then "+" else "-") + _)
      if current.added then newLine += lines.length else oldLine += lines.length
    else
      if oldRangeStart != 0 then
        if lines.length <= context * 2 && i < diff.length - 2 then
          // Overlapping: the context joins the next change.
          curRange ++= lines.map(" " + _)
        else
          val contextSize = math.min(lines.length, context)
          curRange ++= lines.take(contextSize).map(" " + _)
          hunks += Hunk(
            oldRangeStart.toDouble,
            (oldLine - oldRangeStart + contextSize).toDouble,
            newRangeStart.toDouble,
            (newLine - newRangeStart + contextSize).toDouble,
            curRange
          )
          oldRangeStart = 0
          newRangeStart = 0
          curRange = ArrayBuffer.empty
      oldLine += lines.length
      newLine += lines.length
  // Step 2: drop each line's newline, or mark that it has none.
  for hunk <- hunks do
    var i = 0
    while i < hunk.lines.length do
      if hunk.lines(i).endsWith("\n") then hunk.lines(i) = hunk.lines(i).dropRight(1)
      else
        hunk.lines.insert(i + 1, "\\ No newline at end of file")
        i += 1
      i += 1
  val patch = FilePatch(hunks)
  patch("oldFileName") = oldFileName
  patch("newFileName") = newFileName
  patch("oldHeader") = oldHeader
  patch("newHeader") = newHeader
  patch("hunks") = hunks
  patch

/** The lines of `text`, each with its newline where it has one. */
private def splitLines(text: String): IndexedSeq[String] =
  val lines = text.jsSplit("\n").map(_ + "\n").toIndexedSeq
  if text.endsWith("\n") then lines.init else lines.init :+ lines.last.dropRight(1)

/** C-style quoting of a name, over its UTF-8 bytes, as Git and GNU diffutils write it. */
private def quoteFileNameIfNeeded(s: String): String =
  if !s.exists(c => c < ' ' || c > '~' || c == '"' || c == '\\') then s
  else
    val result = StringBuilder("\"")
    for b <- textEncode(s) do
      b match
        case 0x07 => result ++= "\\a"
        case 0x08 => result ++= "\\b"
        case 0x09 => result ++= "\\t"
        case 0x0a => result ++= "\\n"
        case 0x0b => result ++= "\\v"
        case 0x0c => result ++= "\\f"
        case 0x0d => result ++= "\\r"
        case 0x22 => result ++= "\\\""
        case 0x5c => result ++= "\\\\"
        case printable if printable >= 0x20 && printable <= 0x7e => result += printable.toChar
        case other =>
          val octal = Integer.toOctalString(other)
          result += '\\'
          result ++= "0" * (3 - octal.length) + octal
    result.append('"').toString

/** formatPatch with every header, for a patch from `structuredPatch`. */
def formatPatch(patch: FilePatch): String =
  val ret = ArrayBuffer.empty[String]
  val names = (patch.get("oldFileName"), patch.get("newFileName"))
  names match
    case (Some(oldName: String), Some(newName: String)) if oldName == newName => ret += s"Index: $oldName"
    case _ => ()
  ret += Underline
  def header(key: String) = patch.get(key) match
    case Some(value: String) if value.nonEmpty => "\t" + value
    case _ => ""
  names match
    case (Some(oldName: String), Some(newName: String)) =>
      ret += "--- " + quoteFileNameIfNeeded(oldName) + header("oldHeader")
      ret += "+++ " + quoteFileNameIfNeeded(newName) + header("newHeader")
    case _ => ()
  for hunk <- patch.hunks do
    // The start printed for an empty range is one lower.
    val oldStart = if hunk.oldLines == 0 then hunk.oldStart - 1 else hunk.oldStart
    val newStart = if hunk.newLines == 0 then hunk.newStart - 1 else hunk.newStart
    ret += s"@@ -${oldStart.toInt},${hunk.oldLines.toInt} +${newStart.toInt},${hunk.newLines.toInt} @@"
    ret ++= hunk.lines
  ret.mkString("", "\n", "\n")

// ---- parsePatch ----

private val Underline = "==================================================================="

// parse.js's line patterns, as the host RegExps it writes; none is global, so exec and test do not
// depend on lastIndex.
private val GitDiffHeader = js.RegExp("^diff --git ")
private val IndexHeader = js.RegExp("^Index:\\s")
private val HgHeader = js.RegExp("^diff(?: -r \\w+)+\\s")
private val HeaderPrefix = js.RegExp("^(?:Index:|diff(?: -r \\w+)+)\\s+")
private val FileHeader = js.RegExp("^(---|\\+\\+\\+)\\s")
private val FileHeaderPrefix = js.RegExp("^(---|\\+\\+\\+)\\s+")
private val HunkHeader = js.RegExp("^@@\\s")
private val ChunkHeader = js.RegExp("@@ -(\\d+)(?:,(\\d+))? \\+(\\d+)(?:,(\\d+))? @@")
private val HunkBodyLine = js.RegExp("^[+ -]")
private val RenameFrom = js.RegExp("^rename from (.*)")
private val RenameTo = js.RegExp("^rename to (.*)")
private val CopyFrom = js.RegExp("^copy from (.*)")
private val CopyTo = js.RegExp("^copy to (.*)")
private val NewFileMode = js.RegExp("^new file mode (\\d+)")
private val DeletedFileMode = js.RegExp("^deleted file mode (\\d+)")
private val OldMode = js.RegExp("^old mode (\\d+)")
private val NewMode = js.RegExp("^new mode (\\d+)")
private val BinaryFiles = js.RegExp("^Binary files ")

private def isGitDiffHeader(line: String) = GitDiffHeader.test(line)

/** A line that starts the next file: `diff --git`, `Index:` or Mercurial's `diff -r`. */
private def isDiffHeader(line: String) = isGitDiffHeader(line) || IndexHeader.test(line) || HgHeader.test(line)
private def isFileHeader(line: String) = FileHeader.test(line)
private def isHunkHeader(line: String) = HunkHeader.test(line)

/** The first capture of `regex` in `line`, if it matches. */
private def capture(regex: js.RegExp, line: String): Option[String] =
  Option(regex.exec(line)).flatMap(_(1).toOption)

private final case class QuotedName(fileName: String, rawLength: Int)

private def isOctal(s: String, i: Int) = i < s.length && s(i) >= '0' && s(i) <= '7'

/** A C-style quoted name as Git and GNU diff write it: the unescaped name and the length of the
  * quoted text, or None for an unterminated quote or an unknown escape.
  */
private def parseQuotedFileName(s: String): Option[QuotedName] =
  val result = StringBuilder()
  // Octal escapes are raw bytes: a run of them is decoded together as UTF-8. The index after the
  // run, or None when an escape in it is cut short.
  @tailrec def octets(j: Int, bytes: ArrayBuffer[Int]): Option[Int] =
    if !(j + 1 < s.length && s(j) == '\\' && isOctal(s, j + 1)) then
      result ++= textDecode(bytes)
      Some(j)
    else if isOctal(s, j + 2) && isOctal(s, j + 3) then octets(j + 4, bytes += Integer.parseInt(s.substring(j + 1, j + 4), 8))
    else None
  // The index after the escape whose backslash is at j, or None for an unknown escape.
  def escape(j: Int): Option[Int] =
    def add(c: Char) =
      result += c
      Some(j + 2)
    s(j + 1) match
      case 'a' => add('\u0007')
      case 'b' => add('\b')
      case 'f' => add('\f')
      case 'n' => add('\n')
      case 'r' => add('\r')
      case 't' => add('\t')
      case 'v' => add('\u000b')
      case '\\' => add('\\')
      case '"' => add('"')
      case digit if isOctal(s, j + 1) && isOctal(s, j + 2) && isOctal(s, j + 3) => octets(j, ArrayBuffer.empty)
      case _ => None
  @tailrec def scan(j: Int): Option[QuotedName] =
    if j >= s.length then None
    else if s(j) == '"' then Some(QuotedName(result.toString, j + 1))
    else if s(j) == '\\' && j + 1 < s.length then
      escape(j) match
        case Some(next) => scan(next)
        case None => None
    else
      result += s(j)
      scan(j + 1)
  if s.startsWith("\"") then scan(1) else None

private def unquoteIfQuoted(s: String): String =
  if s.startsWith("\"") then parseQuotedFileName(s).fold(s)(_.fileName) else s

/** The names in a `diff --git` line, or None when they cannot be told apart. */
private def parseGitDiffHeader(line: String): Option[(String, String)] =
  val rest = line.substring("diff --git ".length)
  if rest.startsWith("\"") then
    parseQuotedFileName(rest).flatMap: oldPath =>
      // Skip the one unit after the quote, assumed to be a space.
      val afterOld = rest.substring(math.min(oldPath.rawLength + 1, rest.length))
      if afterOld.startsWith("\"") then parseQuotedFileName(afterOld).map(newPath => (oldPath.fileName, newPath.fileName))
      else Some((oldPath.fileName, afterOld))
  else
    val quoteIdx = rest.indexOf('"')
    if quoteIdx > 0 then parseQuotedFileName(rest.substring(quoteIdx)).map(newPath => (rest.substring(0, quoteIdx - 1), newPath.fileName))
    else if rest.startsWith("a/") then
      // Split on the middle " b/", which is right whenever both names are the same.
      val splits = Iterator.iterate(rest.indexOf(" b/", 1))(at => rest.indexOf(" b/", at + 1)).takeWhile(_ >= 0).toIndexedSeq
      if splits.isEmpty then None
      else
        val mid = splits(splits.length / 2)
        Some((rest.substring(0, mid), rest.substring(mid + 1)))
    else None

/** parsePatch: the unified diff text as file patches. */
def parsePatch(uniDiff: String): IndexedSeq[FilePatch] = PatchParser(uniDiff.jsSplit("\n").toIndexedSeq).parse()

private final class PatchParser(diffstr: IndexedSeq[String]):
  private var i = 0
  private val list = ArrayBuffer.empty[FilePatch]

  def parse(): IndexedSeq[FilePatch] =
    while i < diffstr.length do parseIndex()
    list.toIndexedSeq

  private def parseIndex(): Unit =
    val index = FilePatch()
    index("hunks") = index.hunks
    list += index
    // The diff metadata, up to the file or hunk headers; false when a second diff header, which
    // belongs to the next file, ends this one.
    @tailrec def metadata(seenDiffHeader: Boolean): Boolean =
      if i >= diffstr.length then true
      else
        val line = diffstr(i)
        if isFileHeader(line) || isHunkHeader(line) then true
        else if isGitDiffHeader(line) then
          if seenDiffHeader then false
          else
            index("isGit") = true
            for (oldName, newName) <- parseGitDiffHeader(line) do
              index("oldFileName") = oldName
              index("newFileName") = newName
            i += 1
            while i < diffstr.length && !isFileHeader(diffstr(i)) && !isHunkHeader(diffstr(i)) && !isDiffHeader(diffstr(i)) do
              extendedHeader(index, diffstr(i))
              i += 1
            metadata(seenDiffHeader = true)
        else if isDiffHeader(line) then
          if seenDiffHeader then false
          else
            // The index is the rest of an `Index:` or Mercurial header, trimmed.
            for headerMatch <- Option(HeaderPrefix.exec(line)) do
              index("index") = line.substring(headerMatch(0).get.length).jsTrim
            i += 1
            metadata(seenDiffHeader = true)
        else
          i += 1
          metadata(seenDiffHeader)
    if metadata(seenDiffHeader = false) then
      parseFileHeader(index)
      parseFileHeader(index)
      // One file header without the other is a malformed patch.
      (index.get("oldFileName"), index.get("newFileName")) match
        case (Some(name), None) => throw IllegalArgumentException(s"""Missing "+++ ..." file header for $name""")
        case (None, Some(name)) => throw IllegalArgumentException(s"""Missing "--- ..." file header for $name""")
        case _ => ()
      while i < diffstr.length && !isDiffHeader(diffstr(i)) && !isFileHeader(diffstr(i)) && !diffstr(i).startsWith(Underline) do
        // Blank lines and other content between or after hunks are skipped.
        if isHunkHeader(diffstr(i)) then index.hunks += parseHunk() else i += 1

  /** Git extended headers: each pattern is tried, and at most one matches. */
  private def extendedHeader(index: FilePatch, extLine: String): Unit =
    for name <- capture(RenameFrom, extLine) do
      index("oldFileName") = "a/" + unquoteIfQuoted(name)
      index("isRename") = true
    for name <- capture(RenameTo, extLine) do
      index("newFileName") = "b/" + unquoteIfQuoted(name)
      index("isRename") = true
    for name <- capture(CopyFrom, extLine) do
      index("oldFileName") = "a/" + unquoteIfQuoted(name)
      index("isCopy") = true
    for name <- capture(CopyTo, extLine) do
      index("newFileName") = "b/" + unquoteIfQuoted(name)
      index("isCopy") = true
    for mode <- capture(NewFileMode, extLine) do
      index("isCreate") = true
      index("newMode") = mode
    for mode <- capture(DeletedFileMode, extLine) do
      index("isDelete") = true
      index("oldMode") = mode
    for mode <- capture(OldMode, extLine) do index("oldMode") = mode
    for mode <- capture(NewMode, extLine) do index("newMode") = mode
    if BinaryFiles.test(extLine) then index("isBinary") = true

  /** The `---` or `+++` line at the cursor, if there is one. */
  private def parseFileHeader(index: FilePatch): Unit =
    if i < diffstr.length then
      for prefix <- capture(FileHeaderPrefix, diffstr(i)) do
        val data = diffstr(i).substring(3).jsTrim.jsSplit("\t")
        val header = (if data.length > 1 then data(1) else "").jsTrim
        val fileName = if data(0).startsWith("\"") then unquoteIfQuoted(data(0)) else data(0).replace("\\\\", "\\")
        val side = if prefix == "---" then "old" else "new"
        index(side + "FileName") = fileName
        index(side + "Header") = header
        i += 1

  /** The hunk at the cursor. A header the range pattern does not match gives NaN starts. */
  private def parseHunk(): Hunk =
    val chunkHeaderIndex = i
    val range = Option(ChunkHeader.exec(diffstr(i)))
    i += 1
    def number(group: Int) = range.flatMap(_(group).toOption).map(_.toDouble)
    val hunk = Hunk(
      number(1).getOrElse(Double.NaN),
      number(2).getOrElse(1.0),
      number(3).getOrElse(Double.NaN),
      number(4).getOrElse(1.0),
      ArrayBuffer.empty
    )
    // Unified diff quirk: the start of an empty range is one lower.
    if hunk.oldLines == 0 then hunk.oldStart += 1
    if hunk.newLines == 0 then hunk.newStart += 1
    var addCount = 0
    var removeCount = 0
    while i < diffstr.length && (removeCount < hunk.oldLines || addCount < hunk.newLines || diffstr(i).startsWith("\\")) do
      val line = diffstr(i)
      // An empty line is context, except the last one (whose `[0]` is undefined).
      val operation = if line.isEmpty && i != diffstr.length - 1 then Some(' ') else line.headOption
      operation match
        case Some('+') => addCount += 1
        case Some('-') => removeCount += 1
        case Some(' ') =>
          addCount += 1
          removeCount += 1
        case Some('\\') => ()
        case _ => throw IllegalArgumentException(s"Hunk at line ${chunkHeaderIndex + 1} contained invalid line $line")
      hunk.lines += line
      i += 1
    // The empty block count case.
    if addCount == 0 && hunk.newLines == 1 then hunk.newLines = 0
    if removeCount == 0 && hunk.oldLines == 1 then hunk.oldLines = 0
    require(addCount == hunk.newLines, s"Added line count did not match for hunk at line ${chunkHeaderIndex + 1}")
    require(removeCount == hunk.oldLines, s"Removed line count did not match for hunk at line ${chunkHeaderIndex + 1}")
    // A body line right after the counted ones means the counts were wrong, unless it is a file header.
    for next <- diffstr.lift(i) if next.nonEmpty && HunkBodyLine.test(next) && !isFileHeader(next) do
      throw IllegalArgumentException(s"Hunk at line ${chunkHeaderIndex + 1} has more lines than expected")
    hunk

// ---- applyPatch ----

/** applyPatch: the patched text, or None when a hunk does not fit. */
def applyPatch(source: String, patches: IndexedSeq[FilePatch]): Option[String] =
  require(patches.length <= 1, "applyPatch only works with a single input.")
  require(patches.nonEmpty, "applyPatch needs a patch")
  applyStructuredPatch(source, patches.head.hunks.toIndexedSeq)

private def hasOnlyWinLineEndings(s: String) =
  s.contains("\r\n") && !s.startsWith("\n") && !s.indices.exists(i => i > 0 && s(i) == '\n' && s(i - 1) != '\r')

private def hasOnlyUnixLineEndings(s: String) = !s.contains("\r\n") && s.contains("\n")

private def beforeMarker(lines: collection.IndexedSeq[String], i: Int) = lines.lift(i + 1).exists(_.startsWith("\\"))

private def isUnix(hunks: IndexedSeq[Hunk]) = !hunks.exists(_.lines.exists(line => !line.startsWith("\\") && line.endsWith("\r")))

private def isWin(hunks: IndexedSeq[Hunk]) =
  hunks.exists(_.lines.exists(_.endsWith("\r"))) &&
    hunks.forall: hunk =>
      hunk.lines.indices.forall: i =>
        hunk.lines(i).startsWith("\\") || hunk.lines(i).endsWith("\r") || beforeMarker(hunk.lines, i)

private def mapLines(hunks: IndexedSeq[Hunk])(transform: (collection.IndexedSeq[String], Int) => String) =
  hunks.map(hunk => Hunk(hunk.oldStart, hunk.oldLines, hunk.newStart, hunk.newLines, hunk.lines.indices.map(transform(hunk.lines, _)).to(ArrayBuffer)))

private def unixToWin(hunks: IndexedSeq[Hunk]) = mapLines(hunks): (lines, i) =>
  val line = lines(i)
  if line.startsWith("\\") || line.endsWith("\r") || beforeMarker(lines, i) then line else line + "\r"

private def winToUnix(hunks: IndexedSeq[Hunk]) = mapLines(hunks)((lines, i) => lines(i).stripSuffix("\r"))

/** distance-iterator.js: positions after `start`, alternating forward and backward, within the limits. */
private final class DistanceIterator(start: Int, minLine: Int, maxLine: Int):
  private var wantForward = true
  private var backwardExhausted = false
  private var forwardExhausted = false
  private var localOffset = 1

  @tailrec def next(): Option[Int] =
    val forward =
      if wantForward && !forwardExhausted then
        if backwardExhausted then localOffset += 1 else wantForward = false
        // Fit after the offset location, unless that is beyond the text.
        if start + localOffset <= maxLine then Some(start + localOffset)
        else
          forwardExhausted = true
          None
      else None
    if forward.isDefined then forward
    else if backwardExhausted then None
    else
      if !forwardExhausted then wantForward = true
      // Fit before the offset location, unless that is before the text.
      if minLine <= start - localOffset then
        val position = start - localOffset
        localOffset += 1
        Some(position)
      else
        backwardExhausted = true
        next()

private final case class HunkResult(patchedLines: IndexedSeq[String], oldLineLastI: Int)

private def applyStructuredPatch(source: String, patchHunks: IndexedSeq[Hunk]): Option[String] =
  val hunks =
    if hasOnlyWinLineEndings(source) && isUnix(patchHunks) then unixToWin(patchHunks)
    else if hasOnlyUnixLineEndings(source) && isWin(patchHunks) then winToUnix(patchHunks)
    else patchHunks
  val sourceLines = source.jsSplit("\n").to(ArrayBuffer)
  if hunks.isEmpty then return Some(source)
  // A change to the newline at the end of the file is in the last hunk.
  var prevLine = ""
  var removeEOFNL = false
  var addEOFNL = false
  for line <- hunks.last.lines do
    if line.startsWith("\\") then
      if prevLine.startsWith("+") then removeEOFNL = true else if prevLine.startsWith("-") then addEOFNL = true
    prevLine = line
  val endsWithNewline = sourceLines.last == ""
  val lines: Option[IndexedSeq[String]] =
    if removeEOFNL && addEOFNL then
      // The last line changes but has no newline on either side: the source must not have one.
      Option.when(!endsWithNewline)(sourceLines.toIndexedSeq)
    else if removeEOFNL then Option.when(endsWithNewline)(sourceLines.init.toIndexedSeq)
    else if addEOFNL then Option.when(!endsWithNewline)(sourceLines.toIndexedSeq :+ "")
    else Some(sourceLines.toIndexedSeq)
  lines.flatMap(applyHunks(_, hunks))

/** applyHunk with no errors allowed: every removed and context line must equal the source line at
  * its position. Trailing context is given back, so the next hunk may start inside it.
  */
private def applyHunk(lines: IndexedSeq[String], hunkLines: collection.IndexedSeq[String], position: Int): Option[HunkResult] =
  var toPos = position
  val patchedLines = ArrayBuffer.empty[String]
  var nConsecutiveOldContextLines = 0
  var k = 0
  while k < hunkLines.length do
    val hunkLine = hunkLines(k)
    val operation = hunkLine.headOption.getOrElse(' ')
    val content = hunkLine.drop(1)
    operation match
      case '-' =>
        if !lines.lift(toPos).contains(content) then return None
        toPos += 1
        nConsecutiveOldContextLines = 0
      case '+' =>
        patchedLines += content
        nConsecutiveOldContextLines = 0
      case ' ' =>
        nConsecutiveOldContextLines += 1
        if !lines.lift(toPos).contains(content) then return None
        patchedLines += content
        toPos += 1
      case _ => ()
    k += 1
  Some(HunkResult(patchedLines.dropRight(nConsecutiveOldContextLines).toIndexedSeq, toPos - nConsecutiveOldContextLines - 1))

private def applyHunks(lines: IndexedSeq[String], hunks: IndexedSeq[Hunk]): Option[String] =
  val resultLines = ArrayBuffer.empty[String]
  // A position outside the source reads as undefined, which joins as an empty line.
  def copy(from: Int, until: Int) = for i <- from until until do resultLines += lines.lift(i).getOrElse("")
  var minLine = 0
  var prevHunkOffset = 0
  var k = 0
  while k < hunks.length do
    val hunk = hunks(k)
    val oldStart = hunk.oldStart.toInt
    val maxLine = lines.length - hunk.oldLines.toInt
    // The declared position first, then the positions around it.
    var toPos = oldStart + prevHunkOffset - 1
    val iterator = DistanceIterator(toPos, minLine, maxLine)
    var hunkResult = applyHunk(lines, hunk.lines, toPos)
    while hunkResult.isEmpty do
      iterator.next() match
        case Some(position) =>
          toPos = position
          hunkResult = applyHunk(lines, hunk.lines, toPos)
        case None => return None
    copy(minLine, toPos)
    resultLines ++= hunkResult.get.patchedLines
    // Later hunks may not fit over text this one patched.
    minLine = hunkResult.get.oldLineLastI + 1
    prevHunkOffset = toPos + 1 - oldStart
    k += 1
  copy(minLine, lines.length)
  Some(resultLines.mkString("\n"))

// ---- reversePatch, over the structured patch's own JSON values ----

private def truthy(value: js.Any): Boolean = js.DynamicImplicits.truthValue(value.asInstanceOf[js.Dynamic])

private def swapPrefix(fileName: js.Any): js.Any = (fileName: Any) match
  case name: String if name.startsWith("a/") => "b/" + name.substring(2)
  case name: String if name.startsWith("b/") => "a/" + name.substring(2)
  case _ => fileName

private def reverseLine(line: String) =
  if line.startsWith("-") then "+" + line.substring(1)
  else if line.startsWith("+") then "-" + line.substring(1)
  else line

/** reversePatch: a copy of each structured patch with its sides swapped, the list reversed. */
def reversePatch(structuredPatch: js.Any): js.Any =
  if js.Array.isArray(structuredPatch) then
    structuredPatch.asInstanceOf[js.Array[js.Any]].map(reversePatch).reverse
  else
    val patch = structuredPatch.asInstanceOf[js.Dynamic]
    val isGit = truthy(patch.isGit)
    def name(field: js.Any) = if isGit then swapPrefix(field) else field
    // Object.assign({}, patch, {...}): the patch's own keys first, then the new ones.
    val reversed = js.Dynamic.literal()
    for key <- js.Object.keys(patch.asInstanceOf[js.Object]) do reversed.updateDynamic(key)(patch.selectDynamic(key))
    reversed.oldFileName = name(patch.newFileName)
    reversed.oldHeader = patch.newHeader
    reversed.newFileName = name(patch.oldFileName)
    reversed.newHeader = patch.oldHeader
    reversed.oldMode = patch.newMode
    reversed.newMode = patch.oldMode
    reversed.isCreate = patch.isDelete
    reversed.isDelete = patch.isCreate
    reversed.hunks = patch.hunks.asInstanceOf[js.Array[js.Dynamic]].map: hunk =>
      js.Dynamic.literal(
        oldLines = hunk.newLines,
        oldStart = hunk.newStart,
        newLines = hunk.oldLines,
        newStart = hunk.oldStart,
        lines = hunk.lines.asInstanceOf[js.Array[String]].map(reverseLine)
      )
    if truthy(patch.isCopy) then
      // Reversing a copy deletes the copy destination; the hunks cannot be reversed into a deletion.
      reversed.newFileName = "/dev/null"
      reversed.newHeader = js.undefined
      reversed.isDelete = true
      for key <- Seq("isCreate", "isCopy", "isRename") do js.special.delete(reversed, key)
      reversed.hunks = js.Array()
    reversed

/** A structured patch from JSON, for applyPatch. */
def readFilePatch(value: js.Any): FilePatch =
  val hunks = value.asInstanceOf[js.Dynamic].hunks.asInstanceOf[js.Array[js.Dynamic]].map: hunk =>
    Hunk(
      hunk.oldStart.asInstanceOf[Double],
      hunk.oldLines.asInstanceOf[Double],
      hunk.newStart.asInstanceOf[Double],
      hunk.newLines.asInstanceOf[Double],
      hunk.lines.asInstanceOf[js.Array[String]].to(ArrayBuffer)
    )
  FilePatch(hunks.to(ArrayBuffer))
