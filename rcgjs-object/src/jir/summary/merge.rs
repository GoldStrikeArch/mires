//! The link-time item identity rule: duplicate, alias, and object header merging (W4).

use super::{ItemClass, ItemSummary};
use crate::jir::decline::{Decline, MergeDecline};
use crate::jir::hash::Hash128;
use crate::jir::semantic::ItemId;

/// Which of the two summaries a decision keeps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Left,
    Right,
}

/// What the linker does with two summaries that claim one [`super::SemanticKey`].
///
/// The rule, in full:
///
/// | situation | decision |
/// | --- | --- |
/// | headers disagree ([`merge_headers`], checked once per object pair) | [`MergeDecision::Error`] |
/// | different [`super::SemanticKey`], same `link_name` | [`MergeDecision::Error`] |
/// | same key, equal normalized hash | [`MergeDecision::Merge`] — keep either body, union the origins |
/// | same key, one body only | [`MergeDecision::Available`] — keep the side that has one |
/// | same key, different normalized hashes, cleanup edges on one side only | [`MergeDecision::Choose`] — keep the side with cleanup edges |
/// | same key, different normalized hashes, otherwise | [`MergeDecision::Choose`] — keep the smaller hash |
///
/// A body difference is never unavailability. v1's `DUPLICATE_CONFLICT` degrade
/// ([`MergeDecline::Conflict`]) has no case left here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeDecision {
    /// The two summaries agree: keep either body and union the diagnostics.
    Merge,
    /// Only one side carries a body: keep that side's. Not a decline — an item encoded in one
    /// codegen unit and declined in another is ordinary.
    Available(Side),
    /// Both sides carry a body and the normal forms differ. Keep `keep`'s body, whose hash is
    /// `chosen`, and drop the other, whose hash is `rejected`; record `decline` against the item.
    /// See `choose_body`: the choice depends only on the two summaries, so the outcome does not
    /// depend on the order objects were read in.
    Choose {
        keep: Side,
        chosen: Hash128,
        rejected: Hash128,
        decline: Decline,
    },
    /// The two summaries cannot be one item. A hard link error, as in v1.
    Error(MergeError),
}

/// Why two summaries cannot be merged at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeError {
    /// Two different Rust items were emitted under one JavaScript name: v1's "two different
    /// definitions of `name` reached the same program". The likeliest cause is objects compiled
    /// with different emit-affecting options, which [`merge_headers`] catches first.
    NameCollision {
        link_name: String,
        left: Option<ItemId>,
        right: Option<ItemId>,
    },
    /// The caller grouped two summaries that do not even share a name.
    DifferentNames { left: String, right: String },
    /// The objects were not written by the same compiler, target, or options.
    Header(Decline),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameCollision { link_name, .. } => write!(
                f,
                "two different definitions of `{link_name}` reached the same program"
            ),
            Self::DifferentNames { left, right } => write!(
                f,
                "`{left}` and `{right}` are not the same item and cannot be merged"
            ),
            Self::Header(decline) => decline.fmt(f),
        }
    }
}

impl std::error::Error for MergeError {}

/// What the linker does with two summaries for one JavaScript name.
///
/// The headers are the caller's business: check them once per pair of objects with
/// [`merge_headers`], not once per item.
pub fn merge_duplicates(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    if a.key.link_name != b.key.link_name {
        return MergeDecision::Error(MergeError::DifferentNames {
            left: a.key.link_name.clone(),
            right: b.key.link_name.clone(),
        });
    }
    // Same JavaScript name, different Rust items: v1's real conflict, and the only one left.
    if a.key.item != b.key.item {
        return MergeDecision::Error(MergeError::NameCollision {
            link_name: a.key.link_name.clone(),
            left: a.key.item,
            right: b.key.item,
        });
    }
    choose_body(a, b)
}

/// What the linker does with two items the **text** dedup pass proved interchangeable.
///
/// This is the other duplicate relation, and the one the FingerprintJS numbers are actually about.
/// `link_dedup` merges items whose printed JavaScript is byte-identical under *different* names —
/// two monomorphizations of one generic that erase to the same code — and points every reference
/// at one winner. Those items are not the same [`super::SemanticKey`]: their type arguments differ,
/// so their bodies differ, legitimately. The proof that the merge is sound is the printed text,
/// which the caller has already established; all that is left is to choose one body for the
/// surviving item.
///
/// So this is [`merge_duplicates`] without the identity check: never an error, never a loss of
/// availability, and the deterministic `choose_body` pick recorded as
/// [`MergeDecline::BodyChoice`] when the bodies differ. v1 instead declined the winner's late-IR
/// availability outright, which is where all 74 of its `DUPLICATE_CONFLICT`s came from.
pub fn merge_aliases(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    choose_body(a, b)
}

/// Which of two bodies for one item survives: the shared tail of [`merge_duplicates`] and
/// [`merge_aliases`].
///
/// A copy with cleanup edges ([`ItemClass::UNWIND`]) beats a copy without them. Two duplicates
/// were lowered from one Rust body, so the copy without cleanup edges came from MIR that had
/// already lost them (rustc's optimized MIR under `-Cpanic=abort`), and keeping it would silently
/// skip the destructors a host exception owes. (Aliases are already proved interchangeable by
/// their printed text; the same rule only keeps the body that describes more.) Otherwise the
/// smaller normalized hash wins. The decision reads only the two summaries, so it does not depend
/// on the order objects were read in, and folding it over several copies keeps the same one
/// whatever the order.
fn choose_body(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    match (a.semantic_hash, b.semantic_hash) {
        (None, None) => MergeDecision::Merge,
        (Some(_), None) => MergeDecision::Available(Side::Left),
        (None, Some(_)) => MergeDecision::Available(Side::Right),
        (Some(left), Some(right)) if left == right => MergeDecision::Merge,
        (Some(left), Some(right)) => {
            let cleanup = |summary: &ItemSummary| summary.class.contains(ItemClass::UNWIND);
            let keep_left = match (cleanup(a), cleanup(b)) {
                (true, false) => true,
                (false, true) => false,
                _ => left < right,
            };
            let (keep, chosen, rejected) = if keep_left {
                (Side::Left, left, right)
            } else {
                (Side::Right, right, left)
            };
            MergeDecision::Choose {
                keep,
                chosen,
                rejected,
                decline: Decline::Merge(MergeDecline::BodyChoice {
                    chosen: hash_key(chosen),
                    rejected: hash_key(rejected),
                }),
            }
        }
    }
}

/// The hash as the one integer its `Display` spells, for [`MergeDecline::BodyChoice`].
fn hash_key(hash: Hash128) -> u128 {
    (u128::from(hash.hi) << 64) | u128::from(hash.lo)
}

/// Whether two objects may contribute items to one program.
///
/// The JIR schema, the producing rustc, the target data layout and the emit-affecting options all
/// have to agree: type ids, definition ids and printed text are only comparable within one of
/// each. A mismatch is a hard error naming the rebuild that fixes it, never a silent degrade.
pub fn merge_headers(a: &crate::object::Header, b: &crate::object::Header) -> Result<(), Decline> {
    let mismatch = |field: &str, left: String, right: String| {
        Err(Decline::Merge(MergeDecline::Header {
            field: field.to_owned(),
            left,
            right,
        }))
    };
    if a.jir_schema != b.jir_schema {
        return mismatch(
            "jir schema",
            a.jir_schema.to_string(),
            b.jir_schema.to_string(),
        );
    }
    if a.rustc_version_hash != b.rustc_version_hash {
        return mismatch(
            "rustc version",
            format!("{:016x}", a.rustc_version_hash),
            format!("{:016x}", b.rustc_version_hash),
        );
    }
    if a.data_layout != b.data_layout {
        return mismatch("data layout", a.data_layout.clone(), b.data_layout.clone());
    }
    if a.emit_args != b.emit_args {
        return mismatch("emit options", a.emit_args.clone(), b.emit_args.clone());
    }
    Ok(())
}
