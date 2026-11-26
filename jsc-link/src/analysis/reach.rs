//! Shared whole-program reachability BFS utilities and host-delegation predicates.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::jir::semantic::BodySemantic;
use rcgjs_object::jsast;

use crate::item::{JsName, LinkItem};
use crate::plan::native_types::NativeBody;

/// Whether `item` is replaced whole during lowering by a pure host-delegated body with no
/// outgoing Rust calls or byte views (`lower/semantic_ops/num.rs`), and optionally (when
/// `include_typed_heap` is `true`) a Phase 3 typed-heap `Global` allocator hook whose fallback
/// body only performs `Global` allocate/grow/shrink/deallocate without byte dereferences or
/// escaping byte views.
pub(crate) fn is_host_delegated_body(item: &LinkItem, include_typed_heap: bool) -> bool {
    matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op))
            if crate::lower::semantic_ops::num::is_float_from_str(op)
                || (include_typed_heap && op.is_typed_heap())
    )
}

/// Extracts every identifier referenced by a [`NativeBody`]'s JavaScript statements, deduplicated
/// in deterministic order.
pub(crate) fn native_body_idents(body: &NativeBody) -> BTreeSet<String> {
    let mut idents = BTreeSet::new();
    for stmt in &body.statements {
        jsast::visit_idents(stmt, &mut |id| {
            idents.insert(id.to_owned());
        });
    }
    idents
}

/// Collects the seed root names from `table` matching `item.linkage.root`, optionally
/// `item.linkage.exported`, or explicit `extra_roots`.
pub(crate) fn root_seeds(
    table: &BTreeMap<JsName, LinkItem>,
    extra_roots: &[String],
    include_exported: bool,
) -> Vec<JsName> {
    table
        .iter()
        .filter_map(|(name, item)| {
            let is_seed = item.linkage.root
                || (include_exported && item.linkage.exported)
                || extra_roots.iter().any(|r| r == name.as_str());
            is_seed.then(|| name.clone())
        })
        .collect()
}

/// Policy driving [`walk_reachable`] over a graph of `K` nodes.
pub(crate) trait ReachPolicy<K> {
    /// Expands `node`, passing each outgoing neighbor to `enqueue`.
    fn expand(&mut self, node: &K, enqueue: &mut dyn FnMut(K));
}

/// Runs a deterministic breadth-first search starting from `seeds` according to `policy`,
/// returning the set of all visited nodes.
pub(crate) fn walk_reachable<K, P>(
    seeds: impl IntoIterator<Item = K>,
    policy: &mut P,
) -> BTreeSet<K>
where
    K: Ord + Clone,
    P: ReachPolicy<K>,
{
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        if visited.insert(seed.clone()) {
            queue.push_back(seed);
        }
    }
    while let Some(node) = queue.pop_front() {
        let mut enqueue = |next: K| {
            if visited.insert(next.clone()) {
                queue.push_back(next);
            }
        };
        policy.expand(&node, &mut enqueue);
    }
    visited
}
