package fp

import scala.scalajs.js

object Confidence {
  val commentTemplate =
    "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

  def getConfidence(components: js.Dynamic): js.Dynamic = {
    val openConfidenceScore = getOpenConfidenceScore(components)
    val proConfidenceScore = deriveProConfidenceScore(openConfidenceScore)
    js.Dynamic.literal(
      score = openConfidenceScore,
      comment = commentTemplate.replace("$", s"$proConfidenceScore"),
    )
  }

  private def getOpenConfidenceScore(components: js.Dynamic): Double = {
    if (Browser.isAndroid()) return 0.4
    if (Browser.isWebKit())
      return if (Browser.isDesktopWebKit() && !(Browser.isWebKit616OrNewer() && Browser.isSafariWebKit())) 0.5 else 0.3
    val platform =
      if (Host.has(components.platform.asInstanceOf[js.Any], "value"))
        Host.toJsString(components.platform.value.asInstanceOf[js.Any])
      else ""
    if (new js.RegExp("^Win").test(platform)) 0.6
    else if (new js.RegExp("^Mac").test(platform)) 0.5
    else 0.7
  }

  private def deriveProConfidenceScore(openConfidenceScore: Double): Double =
    Data.round(0.99 + 0.01 * openConfidenceScore, 0.0001)
}
