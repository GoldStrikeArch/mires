open Bindings
open Data
open Browser

let commentTemplate = "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

let getOpenConfidenceScore = (components: any) =>
  if isAndroid() {
    0.4
  } else if isWebKit() {
    if isDesktopWebKit() && !(isWebKit616OrNewer() && isSafariWebKit()) {
      0.5
    } else {
      0.3
    }
  } else {
    let platformComponent = get(components, "platform")
    let platform = if hasKey(platformComponent, "value") {
      asString(get(platformComponent, "value"))
    } else {
      ""
    }
    if testRe(newRegExp("^Win", ""), platform) {
      0.6
    } else if testRe(newRegExp("^Mac", ""), platform) {
      0.5
    } else {
      0.7
    }
  }

let deriveProConfidenceScore = (openConfidenceScore: float) =>
  round(0.99 +. 0.01 *. openConfidenceScore, ~base=0.0001)

let getConfidence = (components: any) => {
  let openConfidenceScore = getOpenConfidenceScore(components)
  let proConfidenceScore = deriveProConfidenceScore(openConfidenceScore)
  let comment = stringReplace(
    commentTemplate,
    newRegExp("\\$", "g"),
    jsString(proConfidenceScore),
  )
  let result = emptyObject()
  set(result, "score", openConfidenceScore)
  set(result, "comment", comment)
  result
}
