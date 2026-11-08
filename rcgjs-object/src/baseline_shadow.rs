//! Non-emitting checks for the executable late-IR migration.
//!
//! This module deliberately does not claim that late IR can reproduce the baseline JavaScript.
//! There is no late-IR JavaScript lowerer yet. It checks the two prerequisites that can be proved
//! today: an isolated replay of the current MIR lowerer is identical, and the owned late IR makes
//! an exact serialization round trip.

use serde::{Deserialize, Serialize};

use crate::item::JsItem;
use crate::late_ir::LateIrAnalysisSummary;

pub const BASELINE_SHADOW_VERSION: u16 = 1;

const DUPLICATE_CONFLICT: &str =
    "duplicate definitions carried conflicting baseline shadow results";

/// The result of checking one late-IR body beside its printed JavaScript fallback.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BaselineShadowSummary {
    version: u16,
    baseline_replay: Check,
    ir_roundtrip: Check,
    baseline_fingerprint: Option<u64>,
    ir_fingerprint: Option<u64>,
}

impl BaselineShadowSummary {
    /// Checks one available late-IR body against an isolated replay of current MIR codegen.
    pub fn check(baseline: &JsItem, replay: &JsItem, late_ir: &LateIrAnalysisSummary) -> Self {
        if late_ir.body().is_none() {
            return Self {
                version: BASELINE_SHADOW_VERSION,
                baseline_replay: Check::NotRun,
                ir_roundtrip: Check::NotRun,
                baseline_fingerprint: None,
                ir_fingerprint: None,
            };
        }

        let (baseline_js, _) = baseline.to_js_mapped(false);
        let baseline_fingerprint = fingerprint(baseline_js.as_bytes());
        let baseline_replay = match compare_items(baseline, replay) {
            Ok(()) => Check::Identical,
            Err(differences) => Check::Different(differences.join(", ")),
        };

        let body_bytes = postcard::to_allocvec(
            late_ir
                .body()
                .expect("the unavailable case returned before serialization"),
        )
        .map_err(|error| error.to_string());
        let (ir_roundtrip, ir_fingerprint) = match (body_bytes, late_ir.to_bytes()) {
            (Ok(body), Ok(summary)) => {
                let check = match LateIrAnalysisSummary::from_bytes(&summary) {
                    Ok(decoded) if decoded == *late_ir => Check::Identical,
                    Ok(_) => Check::Different("decoded body differs".to_owned()),
                    Err(error) => Check::Different(format!("decode failed: {error}")),
                };
                (check, Some(fingerprint(&body)))
            }
            (Err(error), _) | (_, Err(error)) => {
                (Check::Different(format!("encode failed: {error}")), None)
            }
        };

        Self {
            version: BASELINE_SHADOW_VERSION,
            baseline_replay,
            ir_roundtrip,
            baseline_fingerprint: Some(baseline_fingerprint),
            ir_fingerprint,
        }
    }

    /// Marks a synthesized item or another item that has no replayable late-IR body.
    pub fn unavailable() -> Self {
        Self {
            version: BASELINE_SHADOW_VERSION,
            baseline_replay: Check::NotRun,
            ir_roundtrip: Check::NotRun,
            baseline_fingerprint: None,
            ir_fingerprint: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != BASELINE_SHADOW_VERSION {
            return Err(format!(
                "baseline shadow summary has format {}, but this backend reads format {}",
                self.version, BASELINE_SHADOW_VERSION
            ));
        }
        if matches!(self.baseline_replay, Check::Identical) && self.baseline_fingerprint.is_none() {
            return Err("identical baseline replay has no fingerprint".to_owned());
        }
        if matches!(self.ir_roundtrip, Check::Identical) && self.ir_fingerprint.is_none() {
            return Err("identical late-IR round trip has no fingerprint".to_owned());
        }
        Ok(())
    }

    pub fn status(&self) -> &'static str {
        match (&self.baseline_replay, &self.ir_roundtrip) {
            (Check::Disabled, Check::Disabled) => "disabled",
            (Check::NotRun, Check::NotRun) => "unavailable",
            (Check::Identical, Check::Identical) => "identity-gated",
            _ => "rejected",
        }
    }

    pub fn baseline_replay(&self) -> &'static str {
        self.baseline_replay.as_str()
    }

    pub fn ir_roundtrip(&self) -> &'static str {
        self.ir_roundtrip.as_str()
    }

    pub fn difference(&self) -> Option<&str> {
        match (&self.baseline_replay, &self.ir_roundtrip) {
            (Check::Different(reason), _) | (_, Check::Different(reason)) => Some(reason),
            _ => None,
        }
    }

    pub fn baseline_fingerprint(&self) -> Option<String> {
        self.baseline_fingerprint
            .map(|value| format!("fnv1a64:{value:016x}"))
    }

    pub fn ir_fingerprint(&self) -> Option<String> {
        self.ir_fingerprint
            .map(|value| format!("fnv1a64:{value:016x}"))
    }

    /// Conservatively combines checks attached to duplicate JavaScript definitions.
    pub fn merge(&mut self, other: &Self) -> bool {
        if self == other {
            return false;
        }
        if self.status() == "disabled" {
            *self = other.clone();
            return false;
        }
        if other.status() == "disabled" {
            return false;
        }
        self.baseline_replay = Check::Different(DUPLICATE_CONFLICT.to_owned());
        self.ir_roundtrip = Check::Different(DUPLICATE_CONFLICT.to_owned());
        self.baseline_fingerprint = None;
        self.ir_fingerprint = None;
        true
    }

    /// v2's duplicate rule: a difference between two duplicates' replay checks is a choice, not
    /// a degrade. See [`crate::late_ir::LateIrAnalysisSummary::merge_choosing`] for why.
    ///
    /// Returns whether a choice had to be made.
    pub fn merge_choosing(&mut self, other: &Self) -> bool {
        let mine = self.clone();
        let theirs = other.clone();
        if !self.merge(other) {
            return false;
        }
        *self = match (
            postcard::to_allocvec(&mine).ok(),
            postcard::to_allocvec(&theirs).ok(),
        ) {
            (Some(left), Some(right)) if right < left => theirs,
            (Some(_), Some(_)) => mine,
            _ => return true,
        };
        true
    }

    pub fn with_version_for_test(mut self, version: u16) -> Self {
        self.version = version;
        self
    }
}

impl Default for BaselineShadowSummary {
    fn default() -> Self {
        Self {
            version: BASELINE_SHADOW_VERSION,
            baseline_replay: Check::Disabled,
            ir_roundtrip: Check::Disabled,
            baseline_fingerprint: None,
            ir_fingerprint: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Check {
    // Append only: postcard serializes this enum by index.
    Disabled,
    NotRun,
    Identical,
    Different(String),
}

impl Check {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotRun => "not-run",
            Self::Identical => "identical",
            Self::Different(_) => "different",
        }
    }
}

/// Compares every part of an item that current MIR lowering controls before local minification.
fn compare_items(baseline: &JsItem, replay: &JsItem) -> Result<(), Vec<&'static str>> {
    let mut differences = Vec::new();
    if baseline.name != replay.name || baseline.kind != replay.kind {
        differences.push("identity");
    }
    if baseline.to_js_mapped(false) != replay.to_js_mapped(false) {
        differences.push("JavaScript or source mappings");
    }
    if baseline.refs != replay.refs {
        differences.push("references");
    }
    if !same_zombies(baseline, replay) {
        differences.push("diagnostics");
    }
    if baseline.linkage.fixed_name != replay.linkage.fixed_name
        || baseline.linkage.root != replay.linkage.root
        || baseline.linkage.exported != replay.linkage.exported
    {
        differences.push("linkage");
    }
    if baseline.debug_path != replay.debug_path
        || baseline.order != replay.order
        || baseline.loc != replay.loc
        || baseline.names != replay.names
    {
        differences.push("metadata");
    }
    if baseline.optimization != replay.optimization {
        differences.push("optimizer metadata");
    }
    let dts_matches = match (
        postcard::to_allocvec(&baseline.dts),
        postcard::to_allocvec(&replay.dts),
    ) {
        (Ok(baseline), Ok(replay)) => baseline == replay,
        _ => false,
    };
    if !dts_matches {
        differences.push("TypeScript signature");
    }

    match differences.is_empty() {
        true => Ok(()),
        false => Err(differences),
    }
}

fn same_zombies(baseline: &JsItem, replay: &JsItem) -> bool {
    baseline.zombies.len() == replay.zombies.len()
        && baseline.zombies.iter().zip(&replay.zombies).all(|(a, b)| {
            a.loc == b.loc && a.message == b.message && a.kind == b.kind && a.span == b.span
        })
}

/// Stable diagnostic fingerprint. Exact comparisons never rely on this hash.
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemKind, JsItem, JsName, Linkage};
    use crate::jsast;

    fn item(value: f64) -> JsItem {
        JsItem::new(
            JsName::new("f".to_owned()),
            ItemKind::Fn,
            jsast::function("f", Vec::new(), vec![jsast::ret(jsast::num(value))]),
            Vec::new(),
            Linkage::internal(),
            "crate::f".to_owned(),
        )
    }

    #[test]
    fn exact_replay_comparison_detects_changed_javascript() {
        assert_eq!(compare_items(&item(1.0), &item(1.0)), Ok(()));
        assert_eq!(
            compare_items(&item(1.0), &item(2.0)),
            Err(vec!["JavaScript or source mappings"])
        );
    }

    #[test]
    fn fingerprint_is_stable_and_not_used_as_equality() {
        assert_eq!(fingerprint(b"late ir"), 0x0b7cf013a84bd2f4);
    }

    #[test]
    fn duplicate_conflict_is_rejected() {
        let mut left = BaselineShadowSummary::unavailable();
        left.baseline_replay = Check::Identical;
        left.ir_roundtrip = Check::Identical;
        left.baseline_fingerprint = Some(1);
        left.ir_fingerprint = Some(2);
        let mut right = left.clone();
        right.ir_fingerprint = Some(3);

        assert!(left.merge(&right));
        assert_eq!(left.status(), "rejected");
        assert_eq!(left.difference(), Some(DUPLICATE_CONFLICT));
    }
}
