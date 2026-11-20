// Marked options subset defined by libraries/marked/CONTRACT.md.

package marked

final class MarkedOptions(
    val gfm: Boolean = true,
    val breaks: Boolean = false,
    val pedantic: Boolean = false,
)

object MarkedOptions:
  val defaults: MarkedOptions = new MarkedOptions()
