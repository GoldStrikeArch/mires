package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const commentTemplate = "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

// getConfidence tells how much the agent is sure about the visitor identifier.
func getConfidence(components *js.Object) *js.Object {
	openConfidenceScore := getOpenConfidenceScore(components)
	proConfidenceScore := deriveProConfidenceScore(openConfidenceScore)
	return object(
		"score", openConfidenceScore,
		"comment", strings.ReplaceAll(commentTemplate, "$", jsString(proConfidenceScore)),
	)
}

// getOpenConfidenceScore is an approximate assessment: the true probability depends on the
// number of website visitors, which the agent doesn't know.
func getOpenConfidenceScore(components *js.Object) float64 {
	if isAndroid() {
		return 0.4
	}

	// Safari (mobile and desktop)
	if isWebKit() {
		if isDesktopWebKit() && !(isWebKit616OrNewer() && isSafariWebKit()) {
			return 0.5
		}
		return 0.3
	}

	platformComponent := components.Get("platform")
	var platform *js.Object = toJS("")
	if has(platformComponent, "value") {
		platform = platformComponent.Get("value")
	}

	switch {
	case startsWith(platform, "Win"):
		// Windows has a higher variety of devices than macOS; Chrome provides more entropy
		// than Firefox and the two are assigned the same score.
		return 0.6
	case startsWith(platform, "Mac"):
		// Chrome, Safari and Firefox provide decreasing entropy and are decreasingly popular.
		return 0.5
	default:
		// Another platform, e.g. a desktop Linux. It's rare, so it should be pretty unique.
		return 0.7
	}
}

func deriveProConfidenceScore(openConfidenceScore float64) float64 {
	return round(0.99+0.01*openConfidenceScore, 0.0001)
}
