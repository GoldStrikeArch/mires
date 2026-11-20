// Block and inline token representations, link definitions, inline queue entries,
// and per-call lexer state matching marked 18.1.1.

package marked

import scala.scalajs.js

sealed trait Token:
  var raw: String

final class SpaceToken(var raw: String) extends Token

final class CodeToken(
    var raw: String,
    val text: String,
    val lang: String | Null,
    val escaped: Boolean = false,
) extends Token

final class HeadingToken(
    var raw: String,
    val depth: Int,
    val text: String,
    val tokens: js.Array[Token],
) extends Token

final class HrToken(var raw: String) extends Token

final class BlockquoteToken(
    var raw: String,
    val tokens: js.Array[Token],
    val text: String,
) extends Token

final class ListItemToken(
    var raw: String,
    var task: Boolean,
    var checked: Boolean,
    var loose: Boolean,
    var text: String,
    var tokens: js.Array[Token],
) extends Token

final class ListToken(
    var raw: String,
    val ordered: Boolean,
    val start: Int,
    var loose: Boolean,
    val items: js.Array[ListItemToken],
) extends Token

final class CheckboxToken(var raw: String, val checked: Boolean) extends Token

final class HtmlToken(
    var raw: String,
    val text: String,
    val block: Boolean,
    val pre: Boolean,
) extends Token

final class DefToken(
    var raw: String,
    val tag: String,
    val href: String,
    val title: String | Null,
) extends Token

final class TableCell(
    val text: String,
    val tokens: js.Array[Token],
    val header: Boolean,
    val align: String | Null,
)

final class TableToken(
    var raw: String,
    val header: js.Array[TableCell],
    val align: js.Array[String | Null],
    val rows: js.Array[js.Array[TableCell]],
) extends Token

final class ParagraphToken(
    var raw: String,
    var text: String,
    val tokens: js.Array[Token],
) extends Token

final class TextToken(
    var raw: String,
    var text: String,
    val tokens: js.Array[Token] | Null,
    val escaped: Boolean,
) extends Token

final class EscapeToken(var raw: String, val text: String) extends Token

final class StrongToken(
    var raw: String,
    val text: String,
    val tokens: js.Array[Token],
) extends Token

final class EmToken(
    var raw: String,
    val text: String,
    val tokens: js.Array[Token],
) extends Token

final class CodespanToken(var raw: String, val text: String) extends Token

final class BrToken(var raw: String) extends Token

final class DelToken(
    var raw: String,
    val text: String,
    val tokens: js.Array[Token],
) extends Token

final class LinkToken(
    var raw: String,
    val href: String,
    val title: String | Null,
    val text: String,
    val tokens: js.Array[Token],
    val autolink: Boolean,
) extends Token

final class ImageToken(
    var raw: String,
    val href: String,
    val title: String | Null,
    val text: String,
    val tokens: js.Array[Token] | Null,
) extends Token

final class LinkDef(val href: String, val title: String | Null)

final class InlineQueueEntry(var src: String, val tokens: js.Array[Token])

final class LexerState(
    var inLink: Boolean = false,
    var inRawBlock: Boolean = false,
    var linkEmitted: Boolean = false,
    var linkParenPossible: Boolean = true,
    var top: Boolean = true,
)
