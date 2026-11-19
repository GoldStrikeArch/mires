// Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
// libraries/pilot/CONTRACT.md. Scala strings are UTF-16, so code points come from codePointAt.

package pilot

import java.nio.charset.StandardCharsets
import scala.collection.mutable

final class Entry(val word: String, var count: Int, val length: Int)

object Pilot:
  private def isWordCodePoint(cp: Int): Boolean =
    cp >= 0x80 || (cp >= '0' && cp <= '9') || (cp >= 'A' && cp <= 'Z') || (cp >= 'a' && cp <= 'z')

  /** FNV-1a 32 over the UTF-8 bytes of `text`; Int arithmetic wraps at 32 bits. */
  def fnv1a(text: String): Int =
    text.getBytes(StandardCharsets.UTF_8).foldLeft(0x811c9dc5)((hash, byte) => (hash ^ (byte & 0xff)) * 0x01000193)

  def analyse(text: String, top: Int): String =
    val entries = mutable.LinkedHashMap.empty[String, Entry]
    var tokens = 0
    var codePoints = 0
    val word = new java.lang.StringBuilder
    var length = 0
    def flush(): Unit =
      if length > 0 then
        tokens += 1
        codePoints += length
        val key = word.toString
        entries.get(key) match
          case Some(entry) => entry.count += 1
          case None        => entries(key) = Entry(key, 1, length)
        word.setLength(0)
        length = 0

    var index = 0
    while index < text.length do
      val cp = text.codePointAt(index)
      if isWordCodePoint(cp) then
        // ASCII-only case folding.
        word.appendCodePoint(if cp >= 'A' && cp <= 'Z' then cp + 0x20 else cp)
        length += 1
      else flush()
      index += Character.charCount(cp)
    flush()

    // sortBy is stable; LinkedHashMap iterates in first-occurrence order.
    val ranked = entries.values.toVector.sortBy(entry => (-entry.count, -entry.length))
    val mean = if tokens == 0 then 0.0 else codePoints.toDouble / tokens
    val header = Vector(s"tokens $tokens", s"distinct ${entries.size}", f"mean $mean%.3f", f"fnv ${fnv1a(text)}%08x")
    val rows = ranked.take(top).map { entry =>
      val share = entry.count.toDouble / tokens
      f"${entry.word} ${entry.count} ${fnv1a(entry.word)}%08x $share%.3f"
    }
    (header ++ rows).mkString("\n")
