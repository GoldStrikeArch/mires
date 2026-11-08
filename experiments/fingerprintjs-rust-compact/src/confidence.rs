use alloc::string::{String, ToString};

use crate::round;

pub const CONFIDENCE_COMMENT_TEMPLATE: &str = "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score";

/// Confidence returned with a visitor result.
#[derive(Clone, Debug, PartialEq)]
pub struct Confidence {
    pub score: f64,
    pub comment: Option<String>,
}

/// Browser facts used by the v5.2.0 confidence formula.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfidenceSignals {
    pub android: bool,
    pub webkit: bool,
    pub desktop_webkit: bool,
    pub webkit_616_or_newer: bool,
    pub safari_webkit: bool,
}

/// Computes the v5.2.0 open-source confidence and Pro comparison comment.
pub fn get_confidence(platform: Option<&str>, signals: ConfidenceSignals) -> Confidence {
    let score = open_confidence_score(platform, signals);
    let pro_score = derive_pro_confidence_score(score).to_string();
    Confidence {
        score,
        comment: Some(CONFIDENCE_COMMENT_TEMPLATE.replace('$', &pro_score)),
    }
}

fn open_confidence_score(platform: Option<&str>, signals: ConfidenceSignals) -> f64 {
    if signals.android {
        0.4
    } else if signals.webkit {
        if signals.desktop_webkit && !(signals.webkit_616_or_newer && signals.safari_webkit) {
            0.5
        } else {
            0.3
        }
    } else if platform.unwrap_or_default().starts_with("Win") {
        0.6
    } else if platform.unwrap_or_default().starts_with("Mac") {
        0.5
    } else {
        0.7
    }
}

fn derive_pro_confidence_score(open_score: f64) -> f64 {
    round(0.99 + 0.01 * open_score, 0.0001)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_upstream_platform_scores() {
        for (platform, signals, score, pro_score) in [
            (
                Some("Linux armv8l"),
                ConfidenceSignals {
                    android: true,
                    ..ConfidenceSignals::default()
                },
                0.4,
                "0.994",
            ),
            (
                Some("MacIntel"),
                ConfidenceSignals {
                    webkit: true,
                    desktop_webkit: true,
                    webkit_616_or_newer: false,
                    safari_webkit: true,
                    android: false,
                },
                0.5,
                "0.995",
            ),
            (
                Some("MacIntel"),
                ConfidenceSignals {
                    webkit: true,
                    desktop_webkit: true,
                    webkit_616_or_newer: true,
                    safari_webkit: true,
                    android: false,
                },
                0.3,
                "0.993",
            ),
            (Some("Win32"), ConfidenceSignals::default(), 0.6, "0.996"),
            (
                Some("Linux x86_64"),
                ConfidenceSignals::default(),
                0.7,
                "0.997",
            ),
        ] {
            let confidence = get_confidence(platform, signals);
            assert_eq!(confidence.score, score);
            assert!(confidence.comment.unwrap().starts_with(pro_score));
        }
    }
}
