//! Bounded ABI versions for the representation plan (Phase 4, D4).
//!
//! An item whose committed slots take a native carrier is rewritten in place when every reachable
//! reference agrees. If a baseline reference must coexist with prepared native callers, D4 keeps
//! the baseline body and emits a private `$n`-suffixed native clone, capped at
//! [`MAX_NATIVE_VERSIONS`] per item. Its name follows the same collision-safe discipline as the
//! spill registers in `plan/abi_flat.rs`; an existing program name is never shadowed.
//!
//! A pin on an item's public boundary keeps the original baseline body. Prepared JIR callers may
//! instead target one private clone; [`budget`] enforces the cap and [`allocate_version_name`]
//! owns its collision-proof name.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::decline::{Decline, VersionDecline};

/// The native version cap per item, beside the always-retained baseline version (D3). With the
/// cap at 1 the plan's branch-and-bound for more versions is never needed and is not built.
pub const MAX_NATIVE_VERSIONS: u32 = 1;

/// Enforces [`MAX_NATIVE_VERSIONS`]: `requested` native versions of one item beyond the baseline.
pub fn budget(requested: u32) -> Result<(), Decline> {
    if requested > MAX_NATIVE_VERSIONS {
        return Err(Decline::Version(VersionDecline::AbiVersions {
            versions: MAX_NATIVE_VERSIONS,
        }));
    }
    Ok(())
}

/// A name for the `$n1` native version of `base` that nothing in the program already uses.
///
/// `taken` is every name the program spells — the item table's keys plus every identifier any
/// item's text mentions, exactly the set the spill allocator checks. The preferred spelling is
/// stepped past with `$1`, `$2`, and later numeric suffixes; running out is a
/// [`VersionDecline::NativeCloneCollision`], which declines the clone rather than shadowing a
/// name (the `$rN` lesson).
///
/// Owned-set form used while allocating several clones and adding each selected name to `taken`.
pub fn allocate_version_name_owned(
    base: &str,
    taken: &BTreeSet<String>,
) -> Result<JsName, Decline> {
    let preferred = format!("{base}$n{MAX_NATIVE_VERSIONS}");
    std::iter::once(preferred.clone())
        .chain((1..64).map(|suffix| format!("{preferred}${suffix}")))
        .find(|name| !taken.contains(name))
        .map(JsName::new)
        .ok_or(Decline::Version(VersionDecline::NativeCloneCollision))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_admits_one_native_version_and_declines_more() {
        assert!(budget(0).is_ok());
        assert!(budget(1).is_ok());
        let declined = budget(2).unwrap_err();
        assert_eq!(declined.code(), "version.abi-versions");
    }

    #[test]
    fn a_version_name_steps_past_everything_the_program_spells() {
        let taken: BTreeSet<String> = ["push$h1", "push$h1$n1", "push$h1$n1$1"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let name = allocate_version_name_owned("push$h1", &taken).unwrap();
        assert_eq!(name.as_str(), "push$h1$n1$2");
        // And is collision-proof against the namer's `$h` stubs by construction.
        assert!(!name.as_str().contains("$h2"));
    }

    /// The `$rN` lesson as a fixture: a program that binds every candidate spelling declines the
    /// clone with the exact code, never shadows.
    #[test]
    fn an_exhausted_allocator_is_a_native_clone_collision() {
        let names: Vec<String> = std::iter::once("f$n1".to_owned())
            .chain((1..64).map(|suffix| format!("f$n1${suffix}")))
            .collect();
        let taken: BTreeSet<String> = names.into_iter().collect();
        let declined = allocate_version_name_owned("f", &taken).unwrap_err();
        assert_eq!(declined.code(), "version.native-clone-collision");
    }
}
