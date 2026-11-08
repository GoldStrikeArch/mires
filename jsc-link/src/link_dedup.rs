//! Link-time duplicate item merge: one emitted body per distinct piece of JavaScript.
//!
//! Rust monomorphizes; JavaScript is type erased. Two instantiations of one generic function that
//! differ only in a type parameter therefore print the *same* body under two names, and a whole
//! program is full of them: `Filter::count` per predicate type, a closure shim per closure type, a
//! searcher per haystack type. Nothing earlier in the pipeline can see it, because the two bodies
//! are codegenned from two different `Instance`s, possibly in two different crates.
//!
//! The link step can. It holds every item's printed text, so "these two print the same code" is a
//! string comparison, and it holds every item's references, so redirecting one name to another is
//! a rewrite of the items that name it.
//!
//! # What "the same code" means
//!
//! Two bodies are the same when they differ **only** in the item's own name. That name appears in
//! the declaration (`function NAME(..)`) and in any self-recursive call, and both spell the *same*
//! thing in both items, so replacing it by a hole leaves texts that are equal exactly when the
//! bodies mean the same thing:
//!
//! * a self-recursive body keeps its self-reference, which after the merge is the surviving item;
//! * a body that calls a *different* item keeps that item's name in the text, so a call to the twin
//!   and a call to oneself never compare equal. That is what stops a body which calls its twin from
//!   being merged onto a body which calls itself;
//! * two mutually recursive pairs (`a -> b`, `a' -> b'`) keep the callee names, which differ, so
//!   they simply do not merge. The opportunity is given up; nothing unsound is committed.
//!
//! The hole is not a sentinel string spliced into the text, which some verbatim `js!{}` block could
//! in principle spell. It is a split: the body is cut at every occurrence of its own name and the
//! remaining pieces are compared as a list. Two lists are equal only when the pieces and the number
//! of cuts both agree.
//!
//! One round of merging can expose the next: two bodies that differed only in which twin they
//! called become identical once that twin is merged. So the pass repeats until a round merges
//! nothing, which terminates because every round removes at least one item.
//!
//! # What is never merged away
//!
//! The surviving item of a group is the lexicographically smallest name in it, which makes the
//! choice a function of the program alone. Every other member is merged away unless something
//! outside the item graph names it: a fixed name (`#[no_mangle]`, `#[export_name]`), an export, a
//! dead-code-elimination root, a `view!` cloner whose declaration order is observable, or an item
//! whose native representation late lowering already committed to.
//!
//! Source maps are the one mode that declines outright: an item's map entries are byte offsets into
//! its own text, and redirecting a reference rewrites that text.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{ItemKind, JsName, LinkItem, OptimizationMiss};
use crate::jir::conflict::{self, ConflictRecord};
use crate::jir::summary::{merge_aliases, MergeDecision, Side};
use crate::opts::OptimizerPass;

/// How many merge rounds are run before the pass stops looking for more.
///
/// Every round removes at least one item, so the fixed point is reached with or without this; the
/// bound is what keeps a pathological call graph from spending the whole link step here.
const MAX_ROUNDS: usize = 8;

/// One surviving item and the duplicates it absorbed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DedupRecord {
    pub(crate) winner: JsName,
    pub(crate) merged: BTreeSet<JsName>,
    /// What the merged bodies contributed before the link-time item rename, which is the same
    /// measure the report's `dropped_pre_rename_bytes` uses.
    pub(crate) bytes_saved: usize,
}

/// What the merge did: the report's evidence, and the linker's signal to walk the graph again.
#[derive(Clone, Debug, Default)]
pub(crate) struct DedupSummary {
    records: BTreeMap<JsName, DedupRecord>,
}

impl DedupSummary {
    /// Whether some item's references were rewritten, which is what makes the linker re-reach.
    pub(crate) fn references_changed(&self) -> bool {
        !self.records.is_empty()
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &DedupRecord> + '_ {
        self.records.values()
    }

    /// Drops records whose surviving item the final reachability pass removed.
    pub(crate) fn retain_reachable(&mut self, reachable: &[JsName]) {
        let reachable: BTreeSet<&JsName> = reachable.iter().collect();
        self.records.retain(|winner, _| reachable.contains(winner));
    }
}

/// One group of items that print the same body, and which of them survives.
struct Group {
    winner: JsName,
    victims: Vec<JsName>,
}

/// Merges every reachable function that prints another one's body, and redirects the references.
///
/// `pinned` names items that must survive under their own name whatever their text says. Late
/// lowering's committed native items are the case: the artifact's representation identity is
/// written out of exactly those names.
pub(crate) fn merge_duplicates(
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    pinned: &BTreeSet<JsName>,
    conflicts: &mut Vec<ConflictRecord>,
) -> DedupSummary {
    let opts = crate::opts::get();
    let mut summary = DedupSummary::default();
    if !opts.analyzes(OptimizerPass::LinkDedup) {
        return summary;
    }

    let mut live: BTreeSet<JsName> = reachable.iter().cloned().collect();
    let mut groups = duplicate_groups(table, &live, pinned);
    if groups.is_empty() {
        return summary;
    }

    for group in &groups {
        if let Some(item) = table.get_mut(&group.winner) {
            item.optimization.select(
                "link-dedup",
                "merged-duplicate-bodies",
                format!(
                    "{} further reachable items print this body under another name",
                    group.victims.len()
                ),
            );
        }
    }

    // An item's source map entries are offsets into its own text, and a redirected reference
    // rewrites that text. Record the candidate, then decline before anything is committed.
    if opts.source_map {
        for group in &groups {
            if let Some(item) = table.get_mut(&group.winner) {
                item.optimization.misses.push(OptimizationMiss::new(
                    "link-dedup",
                    "duplicate item merge does not yet rebase source-map entries",
                ));
            }
        }
        return summary;
    }
    if !opts.applies(OptimizerPass::LinkDedup) {
        return summary;
    }

    for _ in 0..MAX_ROUNDS {
        commit(table, &groups, &mut live, &mut summary, conflicts);
        groups = duplicate_groups(table, &live, pinned);
        if groups.is_empty() {
            break;
        }
    }
    summary
}

/// Runs the emitted-body merge on a measurement table, independent of optimizer report mode.
///
/// D9 measures the program that optimizer mode `on` would emit even while `js-opt=report` is
/// producing baseline bytes. Source maps still disable the merge because that is an emitted-code
/// constraint, not an optimizer-mode choice. Returns whether references changed.
pub(crate) fn merge_duplicates_for_measurement(
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    source_map: bool,
) -> bool {
    if source_map {
        return false;
    }
    let pinned = BTreeSet::new();
    let mut live: BTreeSet<JsName> = reachable.iter().cloned().collect();
    let mut groups = duplicate_groups(table, &live, &pinned);
    if groups.is_empty() {
        return false;
    }
    let mut summary = DedupSummary::default();
    let mut conflicts = Vec::new();
    for _ in 0..MAX_ROUNDS {
        commit(table, &groups, &mut live, &mut summary, &mut conflicts);
        groups = duplicate_groups(table, &live, &pinned);
        if groups.is_empty() {
            break;
        }
    }
    summary.references_changed()
}

/// Folds each group's duplicates into its survivor and points every remaining item at it.
fn commit(
    table: &mut BTreeMap<JsName, LinkItem>,
    groups: &[Group],
    live: &mut BTreeSet<JsName>,
    summary: &mut DedupSummary,
    conflicts: &mut Vec<ConflictRecord>,
) {
    let mut survivor: BTreeMap<JsName, JsName> = BTreeMap::new();
    let mut redirect: BTreeMap<String, String> = BTreeMap::new();
    for group in groups {
        for victim in &group.victims {
            redirect.insert(victim.as_str().to_owned(), group.winner.as_str().to_owned());
            survivor.insert(victim.clone(), group.winner.clone());
        }
    }

    for group in groups {
        let mut bytes_saved = 0;
        for victim in &group.victims {
            let Some(duplicate) = table.get(victim).cloned() else {
                continue;
            };
            bytes_saved += duplicate.js.len();
            let winner = table
                .get_mut(&group.winner)
                .expect("a merge group's surviving item is in the table");
            // These two are **not** one item: their type arguments differ, so their JIR bodies
            // differ for good reason, and the proof that merging them is sound is the printed text
            // this pass has already compared. All that is left is to choose one body for the
            // survivor. v1 instead ran the same conservative merge a same-name duplicate gets and
            // declined the winner's late-IR availability outright, which is where every one of the
            // 74 FingerprintJS `DUPLICATE_CONFLICT`s came from (`JIR.md`).
            choose_alias_body(winner, &duplicate, conflicts);
            // The analyses that produced their metadata ran in two compilation contexts and may
            // have found different facts; the union is what is kept.
            winner.merge_duplicate_metadata(&duplicate);
            absorb_zombies(winner, &duplicate);
        }
        let winner = table
            .get_mut(&group.winner)
            .expect("a merge group's surviving item is in the table");
        winner.optimization.apply("link-dedup");

        let record = summary
            .records
            .entry(group.winner.clone())
            .or_insert_with(|| DedupRecord {
                winner: group.winner.clone(),
                merged: BTreeSet::new(),
                bytes_saved: 0,
            });
        record.merged.extend(group.victims.iter().cloned());
        record.bytes_saved += bytes_saved;
    }

    // Every remaining item that named a duplicate now names the item that survived for it. The
    // rewrite is the item rename's scanner, which leaves strings, comments and property names
    // alone; `refs` is corrected beside the text, because the reachability walk reads it and not
    // the JavaScript.
    let survivors: Vec<JsName> = live
        .iter()
        .filter(|name| !survivor.contains_key(*name))
        .cloned()
        .collect();
    for name in survivors {
        let Some(item) = table.get_mut(&name) else {
            continue;
        };
        if !item
            .refs
            .iter()
            .any(|referenced| survivor.contains_key(referenced))
        {
            continue;
        }
        item.js = crate::minify::apply_renames(&item.js, &redirect);
        item.refs = std::mem::take(&mut item.refs)
            .into_iter()
            .map(|referenced| match survivor.get(&referenced) {
                Some(winner) => winner.clone(),
                None => referenced,
            })
            .collect();
    }

    for victim in survivor.keys() {
        live.remove(victim);
    }
}

/// Applies [`merge_aliases`] to the winner and one of the duplicates it absorbed.
///
/// Never a decline and never an error: the two items were proved interchangeable by their text.
/// When the bodies differ the smaller normalized hash wins, so the survivor's body does not depend
/// on the order the groups were built in, and the choice is recorded for the autopsy.
fn choose_alias_body(
    winner: &mut LinkItem,
    duplicate: &LinkItem,
    conflicts: &mut Vec<ConflictRecord>,
) {
    let (Some(left), Some(right)) = (winner.jir.summary.as_ref(), duplicate.jir.summary.as_ref())
    else {
        // A format 19 object on one side. Keep whichever body exists; there is nothing to compare.
        if !winner.jir.has_body() && duplicate.jir.has_body() {
            winner.jir = duplicate.jir.clone();
        }
        return;
    };
    match merge_aliases(left, right) {
        MergeDecision::Merge | MergeDecision::Available(Side::Left) => {}
        MergeDecision::Available(Side::Right) => winner.jir = duplicate.jir.clone(),
        MergeDecision::Choose { keep, .. } => {
            conflicts.push(alias_conflict(winner, duplicate));
            if keep == Side::Right {
                winner.jir = duplicate.jir.clone();
            }
        }
        // `merge_aliases` has no error case: two items with one text are never a name collision.
        MergeDecision::Error(_) => {}
    }
}

/// The autopsy record for two aliases whose bodies differ. Their bodies are already decoded — this
/// pass runs after the reachable ones were — so nothing is inflated here.
fn alias_conflict(winner: &LinkItem, duplicate: &LinkItem) -> ConflictRecord {
    let entry = |item: &LinkItem| conflict::ConflictEntry {
        origin: item.crate_name.clone(),
        path: item.debug_path.clone(),
        blob: item
            .jir
            .summary
            .as_ref()
            .and_then(|summary| summary.body_hash),
        normalized: item
            .jir
            .summary
            .as_ref()
            .and_then(|summary| summary.semantic_hash),
    };
    let entries = vec![entry(winner), entry(duplicate)];
    let name = winner.name.as_str().to_owned();
    let normalized = |item: &LinkItem| {
        let body = item.jir.body.as_ref()?;
        let interns = item.jir.interns()?;
        Some(crate::jir::summary::NormalizedBody::of(body, interns))
    };
    match (normalized(winner), normalized(duplicate)) {
        (Some(left), Some(right)) => {
            let origins = winner
                .jir
                .origins
                .as_ref()
                .zip(duplicate.jir.origins.as_ref());
            ConflictRecord::of(&name, "alias", &left, &right, origins).with_entries(entries)
        }
        _ => ConflictRecord {
            name,
            relation: "alias",
            verdict: conflict::Verdict::Different,
            cause: conflict::Cause::Undecodable,
            difference: Some("a body was not decoded".to_owned()),
            entries,
        },
    }
}

/// Adds the duplicate's unlowered constructs to the item that now stands for it.
///
/// A zombie is only an error when the program can reach it, and both items were reachable a moment
/// ago, so the union is what keeps the same set reported.
fn absorb_zombies(winner: &mut LinkItem, duplicate: &LinkItem) {
    for zombie in &duplicate.zombies {
        let known = winner
            .zombies
            .iter()
            .any(|known| known.loc == zombie.loc && known.message == zombie.message);
        if !known {
            winner.zombies.push(zombie.clone());
        }
    }
}

/// Groups the live functions by their body and returns the groups with something to merge.
fn duplicate_groups(
    table: &BTreeMap<JsName, LinkItem>,
    live: &BTreeSet<JsName>,
    pinned: &BTreeSet<JsName>,
) -> Vec<Group> {
    let mut buckets: BTreeMap<Vec<&str>, Vec<&JsName>> = BTreeMap::new();
    for name in live {
        let Some((name, item)) = table.get_key_value(name) else {
            continue;
        };
        // Only a function has a body that says everything about it. A module level binding is
        // ordered against its users, an import declares a name, and neither is worth the care.
        if item.kind != ItemKind::Fn {
            continue;
        }
        buckets.entry(body(item)).or_default().push(name);
    }

    let mut groups = Vec::new();
    for members in buckets.into_values() {
        if members.len() < 2 {
            continue;
        }
        // `live` is ordered, so the first member is the lexicographically smallest name in the
        // group and the choice depends on nothing but the program.
        let winner = members[0].clone();
        let victims: Vec<JsName> = members[1..]
            .iter()
            .filter(|name| {
                table
                    .get(**name)
                    .is_some_and(|item| is_mergeable(item, name, pinned))
            })
            .map(|name| (*name).clone())
            .collect();
        if victims.is_empty() {
            continue;
        }
        groups.push(Group { winner, victims });
    }
    groups
}

/// The item's code cut at every occurrence of its own name.
///
/// Joining the pieces with the name reconstructs the code, so two items whose pieces are equal
/// print the same body under two names and nothing else.
fn body(item: &LinkItem) -> Vec<&str> {
    let code = item.code();
    let name = item.name.as_str();
    let mut pieces = Vec::new();
    let mut cursor = 0;
    crate::minify::for_each_ident(code, |at, ident| {
        if ident == name {
            pieces.push(&code[cursor..at]);
            cursor = at + ident.len();
        }
    });
    pieces.push(&code[cursor..]);
    pieces
}

/// Whether an item may lose its name to an identical one.
fn is_mergeable(item: &LinkItem, name: &JsName, pinned: &BTreeSet<JsName>) -> bool {
    item.linkage.fixed_name.is_none()
        && !item.linkage.root
        && !item.linkage.exported
        // A declaration whose position is part of what it means cannot be moved onto another one.
        && item.order.is_none()
        && !pinned.contains(name)
        && !crate::opts::get()
            .roots
            .iter()
            .any(|root| root == name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemMap, Linkage};

    fn item(name: &str, js: &str) -> LinkItem {
        let mut refs = BTreeSet::new();
        crate::minify::for_each_ident(js, |_, ident| {
            if ident != name && ident.contains("$h") {
                refs.insert(JsName::new(ident.to_owned()));
            }
        });
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: js.to_owned(),
            refs,
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        }
    }

    fn table(items: Vec<LinkItem>) -> BTreeMap<JsName, LinkItem> {
        items
            .into_iter()
            .map(|item| (item.name.clone(), item))
            .collect()
    }

    fn live(table: &BTreeMap<JsName, LinkItem>) -> BTreeSet<JsName> {
        table.keys().cloned().collect()
    }

    fn merged(groups: &[Group]) -> Vec<(String, Vec<String>)> {
        groups
            .iter()
            .map(|group| {
                (
                    group.winner.as_str().to_owned(),
                    group
                        .victims
                        .iter()
                        .map(|victim| victim.as_str().to_owned())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_body_is_cut_at_its_own_name_and_nowhere_else() {
        let self_call = item(
            "count$ha",
            "function count$ha(n) { return n ? count$ha(n - 1) : other$hz(n); }\n",
        );
        assert_eq!(
            body(&self_call),
            [
                "function ",
                "(n) { return n ? ",
                "(n - 1) : other$hz(n); }\n"
            ]
        );

        // A comment naming the item is stripped with the header, and a string spelling it is text.
        let commented = item(
            "count$ha",
            "\n// crate::count\nfunction count$ha() { return \"count$ha\"; }\n",
        );
        assert_eq!(
            body(&commented),
            ["function ", "() { return \"count$ha\"; }\n"]
        );
    }

    #[test]
    fn identical_bodies_merge_into_the_smallest_name() {
        let duplicate = "function z_dup$h2(x) { return x + 1; }\n";
        let mut table = table(vec![
            item("z_dup$h2", duplicate),
            item("a_dup$h1", "function a_dup$h1(x) { return x + 1; }\n"),
            item("m_other$h3", "function m_other$h3(x) { return x + 2; }\n"),
            item(
                "caller$h0",
                "function caller$h0(x) { return z_dup$h2(x) + m_other$h3(x); }\n",
            ),
        ]);
        let live = live(&table);
        let groups = duplicate_groups(&table, &live, &BTreeSet::new());
        assert_eq!(
            merged(&groups),
            [("a_dup$h1".to_owned(), vec!["z_dup$h2".to_owned()])]
        );

        let mut live = live;
        let mut summary = DedupSummary::default();
        commit(
            &mut table,
            &groups,
            &mut live,
            &mut summary,
            &mut Vec::new(),
        );

        let caller = &table[&JsName::new("caller$h0".to_owned())];
        assert!(
            caller.js.contains("return a_dup$h1(x) + m_other$h3(x)"),
            "{}",
            caller.js
        );
        assert_eq!(
            caller.refs,
            BTreeSet::from([
                JsName::new("a_dup$h1".to_owned()),
                JsName::new("m_other$h3".to_owned()),
            ])
        );
        assert!(!live.contains(&JsName::new("z_dup$h2".to_owned())));
        assert!(table[&JsName::new("a_dup$h1".to_owned())]
            .optimization
            .applied
            .contains("link-dedup"));
        let records: Vec<_> = summary.records().collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].winner.as_str(), "a_dup$h1");
        assert_eq!(
            records[0].merged,
            BTreeSet::from([JsName::new("z_dup$h2".to_owned())])
        );
        assert_eq!(records[0].bytes_saved, duplicate.len());
    }

    #[test]
    fn a_self_recursive_duplicate_merges_and_keeps_calling_itself() {
        let mut table = table(vec![
            item(
                "down_b$h2",
                "function down_b$h2(n) { return n ? down_b$h2(n - 1) : 0; }\n",
            ),
            item(
                "down_a$h1",
                "function down_a$h1(n) { return n ? down_a$h1(n - 1) : 0; }\n",
            ),
        ]);
        let mut live = live(&table);
        let groups = duplicate_groups(&table, &live, &BTreeSet::new());
        assert_eq!(
            merged(&groups),
            [("down_a$h1".to_owned(), vec!["down_b$h2".to_owned()])]
        );
        commit(
            &mut table,
            &groups,
            &mut live,
            &mut DedupSummary::default(),
            &mut Vec::new(),
        );
        assert_eq!(
            table[&JsName::new("down_a$h1".to_owned())].js,
            "function down_a$h1(n) { return n ? down_a$h1(n - 1) : 0; }\n"
        );
    }

    /// A body that calls itself and a body that calls the twin are not the same code: merging them
    /// would turn a terminating call into an unbounded recursion.
    #[test]
    fn a_call_to_a_twin_never_merges_with_a_call_to_oneself() {
        let table = table(vec![
            item(
                "leaf$h1",
                "function leaf$h1(n) { return n ? leaf$h1(n - 1) : 0; }\n",
            ),
            item(
                "hop$h2",
                "function hop$h2(n) { return n ? leaf$h1(n - 1) : 0; }\n",
            ),
        ]);
        assert!(duplicate_groups(&table, &live(&table), &BTreeSet::new()).is_empty());
    }

    /// Mutual recursion keeps the callee's name in the text, so two such pairs never compare equal.
    #[test]
    fn mutually_recursive_pairs_are_left_alone() {
        let table = table(vec![
            item(
                "even_a$h1",
                "function even_a$h1(n) { return odd_a$h2(n); }\n",
            ),
            item(
                "odd_a$h2",
                "function odd_a$h2(n) { return even_a$h1(n); }\n",
            ),
            item(
                "even_b$h3",
                "function even_b$h3(n) { return odd_b$h4(n); }\n",
            ),
            item(
                "odd_b$h4",
                "function odd_b$h4(n) { return even_b$h3(n); }\n",
            ),
        ]);
        assert!(duplicate_groups(&table, &live(&table), &BTreeSet::new()).is_empty());
    }

    /// Merging a callee makes two callers that differed only in which twin they named identical,
    /// which is the whole reason the pass runs to a fixed point.
    #[test]
    fn a_later_round_merges_what_the_first_one_exposed() {
        let mut table = table(vec![
            item("leaf_a$h1", "function leaf_a$h1(x) { return x + 1; }\n"),
            item("leaf_b$h2", "function leaf_b$h2(x) { return x + 1; }\n"),
            item(
                "use_a$h3",
                "function use_a$h3(x) { return leaf_a$h1(x) * 2; }\n",
            ),
            item(
                "use_b$h4",
                "function use_b$h4(x) { return leaf_b$h2(x) * 2; }\n",
            ),
            item(
                "root$h0",
                "function root$h0(x) { return use_a$h3(x) + use_b$h4(x); }\n",
            ),
        ]);
        let mut live = live(&table);
        let mut summary = DedupSummary::default();
        let mut groups = duplicate_groups(&table, &live, &BTreeSet::new());
        let mut rounds = 0;
        while !groups.is_empty() && rounds < MAX_ROUNDS {
            commit(
                &mut table,
                &groups,
                &mut live,
                &mut summary,
                &mut Vec::new(),
            );
            groups = duplicate_groups(&table, &live, &BTreeSet::new());
            rounds += 1;
        }

        assert_eq!(rounds, 2);
        let winners: Vec<&str> = summary
            .records()
            .map(|record| record.winner.as_str())
            .collect();
        assert_eq!(winners, ["leaf_a$h1", "use_a$h3"]);
        assert!(
            table[&JsName::new("root$h0".to_owned())]
                .js
                .contains("return use_a$h3(x) + use_a$h3(x)"),
            "{}",
            table[&JsName::new("root$h0".to_owned())].js
        );
    }

    #[test]
    fn a_named_item_survives_and_absorbs_the_others() {
        let mut exported = item("dup_z$h2", "function dup_z$h2(x) { return x; }\n");
        exported.linkage = Linkage::exported("dup_z$h2".to_owned());
        let mut rooted = item("dup_y$h3", "function dup_y$h3(x) { return x; }\n");
        rooted.linkage = Linkage::rooted("dup_y$h3".to_owned());
        let table = table(vec![
            exported,
            rooted,
            item("dup_a$h1", "function dup_a$h1(x) { return x; }\n"),
        ]);

        // The smallest name wins, and neither named item may be merged away.
        let groups = duplicate_groups(&table, &live(&table), &BTreeSet::new());
        assert!(groups.is_empty());

        // With a mergeable member the group forms, and the named ones still stay.
        let mut with_plain = table;
        let plain = item("dup_b$h4", "function dup_b$h4(x) { return x; }\n");
        with_plain.insert(plain.name.clone(), plain);
        let groups = duplicate_groups(&with_plain, &live(&with_plain), &BTreeSet::new());
        assert_eq!(
            merged(&groups),
            [("dup_a$h1".to_owned(), vec!["dup_b$h4".to_owned()])]
        );
    }

    #[test]
    fn a_pinned_item_keeps_its_own_name() {
        let table = table(vec![
            item("dup_a$h1", "function dup_a$h1(x) { return x; }\n"),
            item("dup_b$h2", "function dup_b$h2(x) { return x; }\n"),
        ]);
        let pinned = BTreeSet::from([JsName::new("dup_b$h2".to_owned())]);
        assert!(duplicate_groups(&table, &live(&table), &pinned).is_empty());
    }

    #[test]
    fn a_string_that_spells_an_item_is_never_redirected() {
        let mut table = table(vec![
            item("dup_a$h1", "function dup_a$h1(x) { return x; }\n"),
            item("dup_b$h2", "function dup_b$h2(x) { return x; }\n"),
            item(
                "caller$h0",
                "function caller$h0(x) { return dup_b$h2(x) + \"dup_b$h2\".length + x.dup_b$h2; }\n",
            ),
        ]);
        let mut live = live(&table);
        let groups = duplicate_groups(&table, &live, &BTreeSet::new());
        commit(
            &mut table,
            &groups,
            &mut live,
            &mut DedupSummary::default(),
            &mut Vec::new(),
        );

        assert_eq!(
            table[&JsName::new("caller$h0".to_owned())].js,
            "function caller$h0(x) { return dup_a$h1(x) + \"dup_b$h2\".length + x.dup_b$h2; }\n"
        );
    }
}
