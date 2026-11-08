private const val COMMENT_TEMPLATE =
    "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

internal fun getConfidence(components: dynamic): dynamic {
    val openConfidenceScore = getOpenConfidenceScore(components)
    val proConfidenceScore = deriveProConfidenceScore(openConfidenceScore)
    val result = obj()
    jsSet(result, "score", openConfidenceScore)
    jsSet(result, "comment", jsReplace(COMMENT_TEMPLATE, "\\$", "g", jsString(proConfidenceScore)))
    return result
}

private fun getOpenConfidenceScore(components: dynamic): Double {
    if (isAndroid()) return 0.4
    if (isWebKit()) {
        return if (isDesktopWebKit() && !(isWebKit616OrNewer() && isSafariWebKit())) 0.5 else 0.3
    }
    val platformComponent = components.platform
    val platform = if (jsHas(platformComponent, "value")) {
        jsString(platformComponent.value)
    } else {
        ""
    }
    return when {
        platform.startsWith("Win") -> 0.6
        platform.startsWith("Mac") -> 0.5
        else -> 0.7
    }
}

private fun deriveProConfidenceScore(openConfidenceScore: Double): Double {
    return round(0.99 + 0.01 * openConfidenceScore, 0.0001)
}
