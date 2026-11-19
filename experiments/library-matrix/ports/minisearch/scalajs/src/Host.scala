// The one host function the port calls besides what Scala.js itself compiles to the host
// (`toLowerCase()` is JS `toLowerCase`, `math.log` is `Math.log`): MiniSearch's tokenizer, which
// is `text.split(SPACE_OR_PUNCTUATION)` with a module-level RegExp. CONTRACT.md ("Tokenizer") lets
// a port call it, with the `u` flag. It goes through Scala.js's own facades (js.RegExp, jsSplit):
// java.util.regex translates \p{Z} and \p{P} only when linking for ES2018, and the builder links
// for Scala.js's default, ES2015 (NOTES.md).

package minisearch

import scala.scalajs.js
import scala.scalajs.js.JSStringOps.*

private[minisearch] object Host:
  /** MiniSearch's SPACE_OR_PUNCTUATION: a run of Unicode spaces, newlines or punctuation. As in
    * MiniSearch, one RegExp for the module: `split` does not use its `lastIndex`.
    */
  private val SpaceOrPunctuation = js.RegExp("[\\n\\r\\p{Z}\\p{P}]+", "u")

  /** `text.split(SPACE_OR_PUNCTUATION)`: leading and trailing separators leave empty tokens, and the
    * empty text is one empty token. The tokens are the host array, seen as a Scala sequence.
    */
  def splitOnSpaceOrPunctuation(text: String): collection.IndexedSeq[String] = text.jsSplit(SpaceOrPunctuation)
