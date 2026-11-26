//! Shared collision-free synthetic name pool and static item builder for whole-program plans.
//!
//! Whole-program plans (`abi_flat`, `bstr_abi`, `enum_repr`, `soa_vec`, `versions`) all need to
//! synthesize top-level JavaScript identifiers that never shadow anything the program defines or
//! references, and several emit synthetic module-scope `ItemKind::Static` declarations (`let $r1;`,
//! `const $se = {};`, `const $sn = {};`). Centralizing both here guarantees uniform `$1..$63`
//! collision avoidance and consistent `LinkItem` metadata across every plan.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{ItemKind, ItemMap, JsName, LinkItem, Linkage};
use crate::jir::decline::{Decline, VersionDecline};

/// Exclusive upper bound on numeric `$N` collision suffixes (`$1..$63`).
const MAX_SUFFIX: u32 = 64;

/// Finds the first spelling among `base`, `base$1`, ..., `base$63` absent from `taken`.
pub(crate) fn find_suffixed(base: &str, taken: &BTreeSet<String>) -> Option<JsName> {
    if !taken.contains(base) {
        return Some(JsName::new(base.to_owned()));
    }
    (1..MAX_SUFFIX)
        .map(|suffix| format!("{base}${suffix}"))
        .find(|cand| !taken.contains(cand))
        .map(JsName::new)
}

/// Deterministic, collision-free pool of synthetic top-level JavaScript identifiers.
#[derive(Clone, Debug, Default)]
pub(crate) struct NamePool {
    taken: BTreeSet<String>,
}

impl NamePool {
    /// Seeds the pool with every name defined in `table` and every identifier referenced by any
    /// item's `refs`.
    pub(crate) fn from_table(table: &BTreeMap<JsName, LinkItem>) -> Self {
        let mut taken = BTreeSet::new();
        for (name, item) in table {
            taken.insert(name.as_str().to_owned());
            for referenced in &item.refs {
                taken.insert(referenced.as_str().to_owned());
            }
        }
        Self { taken }
    }

    /// Allocates `base` (or `{base}$1..{base}$63`) not yet in the pool and reserves it.
    pub(crate) fn alloc_suffixed(&mut self, base: &str) -> Option<JsName> {
        let chosen = find_suffixed(base, &self.taken)?;
        self.taken.insert(chosen.as_str().to_owned());
        Some(chosen)
    }

    /// Allocates a collision-free `$n{MAX_NATIVE_VERSIONS}` native version name for `base` and
    /// reserves it in the pool.
    pub(crate) fn alloc_native_version(&mut self, base: &str) -> Result<JsName, Decline> {
        let preferred = format!("{base}$n{}", super::versions::MAX_NATIVE_VERSIONS);
        self.alloc_suffixed(&preferred)
            .ok_or(Decline::Version(VersionDecline::NativeCloneCollision))
    }

    /// Finds `base` or `{base}{suffix}_` (`suffix >= 1`) such that no taken name starts with the
    /// returned prefix.
    pub(crate) fn alloc_unused_prefix(&self, base: &str) -> String {
        if !self.has_prefix(base) {
            return base.to_owned();
        }
        let mut suffix = 1u32;
        loop {
            let cand = format!("{base}{suffix}_");
            if !self.has_prefix(&cand) {
                return cand;
            }
            suffix += 1;
        }
    }

    fn has_prefix(&self, prefix: &str) -> bool {
        use std::ops::Bound;
        self.taken
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .next()
            .is_some_and(|next| next.starts_with(prefix))
    }
}

/// Builds a synthetic internal `ItemKind::Static` item (e.g. a spill register or sentinel token).
pub(crate) fn synthesized_static_item(name: JsName, js: String, debug_path: &str) -> LinkItem {
    LinkItem {
        name,
        kind: ItemKind::Static,
        js,
        refs: Default::default(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.to_owned(),
        crate_name: "generated".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_item(name: &str, refs: &[&str]) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: refs.iter().map(|r| JsName::new((*r).to_owned())).collect(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: String::new(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        }
    }

    #[test]
    fn name_pool_avoids_keys_refs_and_prior_allocations() {
        let mut table = BTreeMap::new();
        table.insert(JsName::new("$r1".to_owned()), empty_item("$r1", &[]));
        table.insert(
            JsName::new("user".to_owned()),
            empty_item("user", &["$r1$1", "$__mires_soa_e0"]),
        );
        let mut pool = NamePool::from_table(&table);
        assert_eq!(pool.alloc_suffixed("$r1").unwrap().as_str(), "$r1$2");
        assert_eq!(pool.alloc_suffixed("$r1").unwrap().as_str(), "$r1$3");
        assert_eq!(pool.alloc_unused_prefix("$__mires_soa_"), "$__mires_soa_1_");
    }
}
