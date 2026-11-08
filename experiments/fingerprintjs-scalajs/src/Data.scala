package fp

import scala.scalajs.js

object Data {
  def includes(haystack: js.Array[?], needle: js.Any): Boolean = {
    var i = 0
    val len = haystack.length
    while (i < len) {
      if (haystack(i).asInstanceOf[js.Any] == needle) return true
      i += 1
    }
    false
  }

  def excludes(haystack: js.Array[?], needle: js.Any): Boolean =
    !includes(haystack, needle)

  def toInt(value: js.Any): Double = Host.parseInt(value)

  def toFloat(value: js.Any): Double = Host.parseFloat(value)

  def replaceNaN(value: Double, replacement: js.Any): js.Any =
    if (value.isNaN) replacement else value

  def replaceNaNDouble(value: Double, replacement: Double): Double =
    if (value.isNaN) replacement else value

  def countTruthy(values: Seq[Boolean]): Int =
    values.count(identity)

  def countTruthyAny(values: js.Any*): Int =
    values.count(Host.jsTruthy)

  def round(value: Double, base: Double = 1): Double =
    if (math.abs(base) >= 1) math.round(value / base).toDouble * base
    else {
      val counterBase = 1 / base
      math.round(value * counterBase).toDouble / counterBase
    }

  def parseSimpleCssSelector(selector: String): (js.UndefOr[String], js.Dictionary[js.Array[String]]) = {
    val errorMessage = s"Unexpected syntax '$selector'"
    val tagRe = new js.RegExp("""^\s*([a-z-]*)(.*)$""", "i")
    val tagMatch = tagRe.exec(selector)
    if (tagMatch == null) throw js.JavaScriptException(new js.Error(errorMessage))
    val rawTag = tagMatch(1).getOrElse("")
    val tag: js.UndefOr[String] = if (rawTag.isEmpty) js.undefined else rawTag
    val attributes = js.Dictionary.empty[js.Array[String]]

    def addAttribute(name: String, value: String): Unit = {
      if (!attributes.contains(name)) attributes(name) = js.Array[String]()
      attributes(name).push(value)
    }

    val partsRegex = new js.RegExp("""([.:#][\w-]+|\[.+?\])""", "gi")
    val rest = tagMatch(2).getOrElse("")
    var step = partsRegex.exec(rest)
    while (step != null) {
      val part = step(0).getOrElse("")
      if (part.nonEmpty) {
        part.charAt(0) match {
          case '.' => addAttribute("class", part.substring(1))
          case '#' => addAttribute("id", part.substring(1))
          case '[' =>
            val attributeMatch = new js.RegExp("""^\[([\w-]+)([~|^$*]?=("(.*?)"|([\w-]+)))?(\s+[is])?\]$""").exec(part)
            if (attributeMatch != null) {
              val name = attributeMatch(1).getOrElse("")
              val quoted = attributeMatch(4)
              val unquoted = attributeMatch(5)
              val value =
                if (quoted.isDefined) quoted.get
                else if (unquoted.isDefined) unquoted.get
                else ""
              addAttribute(name, value)
            } else throw js.JavaScriptException(new js.Error(errorMessage))
          case _ => throw js.JavaScriptException(new js.Error(errorMessage))
        }
      }
      step = partsRegex.exec(rest)
    }
    (tag, attributes)
  }

  def getUTF8Bytes(input: String) = Hashing.getUTF8Bytes(input)
}
