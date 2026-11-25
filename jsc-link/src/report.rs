//! Deterministic optimizer diagnostics and size reporting.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use crate::diag::Sink;

use crate::item::{ItemKind, JsName, LinkItem, OptimizationMiss, OptimizationSelection};
use crate::lane_report::push_string as json_string;
use crate::opts::OptimizerMode;

mod operations;

/// Warns once for every reachable rejected workspace candidate.
pub(crate) fn warn_misses(dcx: &Sink, table: &BTreeMap<JsName, LinkItem>, reachable: &[JsName]) {
    let opts = crate::opts::get();
    if opts.optimizer == OptimizerMode::Off || !opts.optimization_warnings {
        return;
    }

    let mut misses: BTreeMap<&str, BTreeSet<(&str, &str)>> = BTreeMap::new();
    for name in reachable {
        let Some(item) = table.get(name) else {
            continue;
        };
        for miss in &item.optimization.misses {
            let (Some(source), Some(location)) = (miss.source.as_deref(), miss.location.as_deref())
            else {
                continue;
            };
            if !crate::opts::is_workspace_source(source) {
                continue;
            }
            misses
                .entry(location)
                .or_default()
                .insert((miss.pass.as_str(), miss.reason.as_str()));
        }
    }

    for (location, reasons) in misses {
        let reasons = reasons
            .into_iter()
            .map(|(pass, reason)| format!("`{pass}`: {reason}"))
            .collect::<Vec<_>>()
            .join("; ");
        dcx.warn(format!(
            "{location}: rustc_codegen_js optimizations were not applied: {reasons}"
        ));
    }
}

/// Writes the requested whole-program report.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write(
    dcx: &Sink,
    path: &Path,
    label: &str,
    program_bytes: usize,
    item_bytes: &BTreeMap<JsName, usize>,
    synthesized_bytes: usize,
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    parents: &HashMap<JsName, JsName>,
    dedup: &crate::link_dedup::DedupSummary,
    lane: Option<&crate::link::lane::LaneRun>,
) {
    let mut names: Vec<&JsName> = reachable.iter().collect();
    names.sort();

    let mut crates: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for name in &names {
        let Some(item) = table.get(*name) else {
            continue;
        };
        let entry = crates.entry(crate_label(item)).or_default();
        entry.0 += 1;
        entry.1 += item_bytes.get(*name).copied().unwrap_or(item.js.len());
    }
    let reachable_set: BTreeSet<&JsName> = names.iter().copied().collect();
    let dropped: Vec<&LinkItem> = table
        .iter()
        .filter(|(name, _)| !reachable_set.contains(name))
        .map(|(_, item)| item)
        .collect();
    let dropped_bytes: usize = dropped.iter().map(|item| item.js.len()).sum();

    let mut out = String::new();
    let opts = crate::opts::get();
    out.push_str("{\n  \"version\": 6,\n  \"optimizer\": ");
    json_string(&mut out, opts.optimizer.as_str());
    out.push_str(",\n  \"javascript_target\": {\"syntax_ceiling\": ");
    json_string(&mut out, opts.ecma_target.as_str());
    out.push_str(", \"syntax_only\": true, \"strict_runtime_compatibility_claimed\": false, ");
    out.push_str("\"raw_javascript_policy\": \"validated-by-pinned-final-artifact-parser\", ");
    out.push_str("\"selected_post_es2018_capabilities\": [");
    if opts.ecma_target == crate::opts::EcmaTarget::Latest {
        json_string(&mut out, "bigint-literal");
        out.push_str(", ");
        json_string(&mut out, "logical-assignment");
    }
    out.push_str("]}");
    out.push_str(",\n  \"passes\": {\"registered\": ");
    write_passes(&mut out, crate::opts::OptimizerPass::ORDERED.into_iter());
    out.push_str(", \"requested\": ");
    write_passes(&mut out, opts.requested_passes());
    out.push_str(", \"analyzed\": ");
    write_passes(&mut out, opts.analyzed_passes());
    out.push_str(", \"emission_enabled\": ");
    write_passes(&mut out, opts.emission_enabled_passes());
    out.push_str(", \"applied\": ");
    write_passes(
        &mut out,
        actually_applied_passes(table, reachable, dedup).into_iter(),
    );
    out.push('}');
    out.push_str(",\n  \"jir_census_schema\": 1,\n  \"jir_census_scope\": ");
    json_string(&mut out, "Captured JIR-S of final reachable items. Captured calls may have been inlined or removed; link_dependencies describe the emitted item graph. Identity loss before JIR capture is not measured. Native signature validation is not a representation commit or a complete ownership proof.");
    out.push_str(",\n  \"standard_operation_registry\": ");
    operations::write_registry(&mut out);
    out.push_str(",\n  \"jir_lane\": ");
    match lane {
        Some(lane) => out.push_str(&lane.render(opts.lane)),
        None => out.push_str("null"),
    }
    out.push_str(",\n  \"unit\": ");
    json_string(&mut out, label);
    let reachable_item_bytes: usize = names.iter().filter_map(|name| item_bytes.get(*name)).sum();
    let emitted_item_bytes: usize = item_bytes.values().sum();
    let link_generated_item_bytes = emitted_item_bytes.saturating_sub(reachable_item_bytes);
    debug_assert_eq!(program_bytes, emitted_item_bytes + synthesized_bytes);
    out.push_str(&format!(
        ",\n  \"program_bytes\": {program_bytes},\n  \"reachable_item_bytes\": \
         {reachable_item_bytes},\n  \"link_generated_item_bytes\": \
         {link_generated_item_bytes},\n  \"synthesized_bytes\": {synthesized_bytes},\n  \
         \"dropped_items\": {},\n  \"dropped_pre_rename_bytes\": {dropped_bytes},\n",
        dropped.len(),
    ));

    out.push_str("  \"dedup\": [");
    write_dedup(&mut out, dedup);
    out.push_str("],\n  \"crates\": [");
    for (index, (name, (items, bytes))) in crates.iter().enumerate() {
        out.push_str(if index == 0 { "\n" } else { ",\n" });
        out.push_str("    {\"name\": ");
        json_string(&mut out, name);
        out.push_str(&format!(", \"items\": {items}, \"bytes\": {bytes}}}"));
    }
    if !crates.is_empty() {
        out.push('\n');
        out.push_str("  ");
    }
    out.push_str("],\n  \"items\": [");

    for (index, name) in names.iter().enumerate() {
        let Some(item) = table.get(*name) else {
            continue;
        };
        out.push_str(if index == 0 { "\n" } else { ",\n" });
        out.push_str("    {\n      \"name\": ");
        json_string(&mut out, name.as_str());
        out.push_str(",\n      \"crate\": ");
        json_string(&mut out, &crate_label(item));
        out.push_str(",\n      \"path\": ");
        json_string(&mut out, &item.debug_path);
        out.push_str(",\n      \"kind\": ");
        json_string(&mut out, kind(item.kind));
        out.push_str(&format!(
            ",\n      \"bytes\": {},\n      \"root\": {},\n      \"root_chain\": ",
            item_bytes.get(*name).copied().unwrap_or(item.js.len()),
            item.linkage.root || opts.roots.iter().any(|root| root == item.name.as_str())
        ));
        write_strings(&mut out, &root_chain(name, parents));
        out.push_str(",\n      \"selections\": ");
        write_selections(&mut out, &item.optimization.selections);
        out.push_str(",\n      \"misses\": ");
        write_misses(&mut out, &item.optimization.misses);
        out.push_str(",\n      \"link_dependencies\": ");
        write_strings(
            &mut out,
            &item
                .refs
                .iter()
                .filter(|name| table.contains_key(*name))
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        );
        out.push_str(",\n      \"jir_census\": ");
        operations::write_item(&mut out, &item.jir);
        out.push_str("\n    }");
    }
    if !names.is_empty() {
        out.push('\n');
        out.push_str("  ");
    }
    out.push_str("]\n}\n");

    if let Err(err) = fs::write(path, out.as_bytes()) {
        dcx.err(format!(
            "error writing optimization report `{}`: {err}",
            path.display()
        ));
    }
}

/// The items that absorbed a duplicate body, each with the names it now stands for.
fn write_dedup(out: &mut String, dedup: &crate::link_dedup::DedupSummary) {
    for (index, record) in dedup.records().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"winner\": ");
        json_string(out, record.winner.as_str());
        out.push_str(", \"merged\": [");
        for (merged_index, merged) in record.merged.iter().enumerate() {
            if merged_index > 0 {
                out.push_str(", ");
            }
            json_string(out, merged.as_str());
        }
        out.push_str(&format!("], \"bytes_saved\": {}}}", record.bytes_saved));
    }
}

fn root_chain(name: &JsName, parents: &HashMap<JsName, JsName>) -> Vec<String> {
    let mut chain = vec![name.as_str().to_owned()];
    let mut current = name;
    while let Some(parent) = parents.get(current) {
        chain.push(parent.as_str().to_owned());
        current = parent;
        if chain.len() == 128 {
            break;
        }
    }
    chain.reverse();
    chain
}

fn crate_label(item: &LinkItem) -> String {
    item.crate_name.clone()
}

fn kind(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Fn => "function",
        ItemKind::Static => "static",
        ItemKind::Vtable => "vtable",
        ItemKind::Alloc => "allocator",
        ItemKind::Const => "constant",
        ItemKind::Import => "import",
    }
}

fn write_selections(out: &mut String, selections: &[OptimizationSelection]) {
    let mut selections: Vec<&OptimizationSelection> = selections.iter().collect();
    selections
        .sort_by(|a, b| (&a.pass, &a.selected, &a.reason).cmp(&(&b.pass, &b.selected, &b.reason)));
    out.push('[');
    for (index, selection) in selections.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"pass\": ");
        json_string(out, &selection.pass);
        out.push_str(", \"selected\": ");
        json_string(out, &selection.selected);
        out.push_str(", \"reason\": ");
        json_string(out, &selection.reason);
        out.push('}');
    }
    out.push(']');
}

fn write_misses(out: &mut String, misses: &[OptimizationMiss]) {
    let mut misses: Vec<&OptimizationMiss> = misses.iter().collect();
    misses.sort_by(|a, b| {
        (&a.pass, &a.source, &a.location, &a.reason).cmp(&(
            &b.pass,
            &b.source,
            &b.location,
            &b.reason,
        ))
    });
    out.push('[');
    for (index, miss) in misses.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"pass\": ");
        json_string(out, &miss.pass);
        out.push_str(", \"reason\": ");
        json_string(out, &miss.reason);
        out.push_str(", \"location\": ");
        match &miss.location {
            Some(location) => json_string(out, location),
            None => out.push_str("null"),
        }
        let workspace = miss
            .source
            .as_deref()
            .is_some_and(crate::opts::is_workspace_source);
        out.push_str(&format!(", \"workspace\": {workspace}}}"));
    }
    out.push(']');
}

fn write_strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        json_string(out, value);
    }
    out.push(']');
}

fn write_passes(out: &mut String, passes: impl Iterator<Item = crate::opts::OptimizerPass>) {
    out.push('[');
    for (index, pass) in passes.enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        json_string(out, pass.as_str());
    }
    out.push(']');
}

/// Passes with evidence that they changed at least one reachable emitted item.
fn actually_applied_passes(
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    dedup: &crate::link_dedup::DedupSummary,
) -> Vec<crate::opts::OptimizerPass> {
    use crate::opts::OptimizerPass;

    let opts = crate::opts::get();
    let applied = |pass: &str| {
        reachable.iter().any(|name| {
            table
                .get(name)
                .is_some_and(|item| item.optimization.applied.contains(pass))
        })
    };

    opts.emission_enabled_passes()
        .filter(|pass| match pass {
            OptimizerPass::Sroa => applied("sroa"),
            OptimizerPass::AstDce => applied("ast-dce"),
            OptimizerPass::FuturePoller => applied("future-poller"),
            // The v1 text lowerer is deleted; the pass name stays registered but never applies.
            OptimizerPass::NativeRepresentations => false,
            OptimizerPass::LinkDedup => applied("link-dedup") && dedup.records().next().is_some(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_escape_every_json_control_character() {
        let mut out = String::new();
        json_string(&mut out, "a\u{b}\u{1f}\n\"\\z");
        assert_eq!(out, "\"a\\u000b\\u001f\\n\\\"\\\\z\"");
    }

    #[test]
    fn root_chains_run_from_the_root_to_the_item() {
        let root = JsName::new("root".to_owned());
        let middle = JsName::new("middle".to_owned());
        let leaf = JsName::new("leaf".to_owned());
        let parents = HashMap::from([(middle.clone(), root), (leaf.clone(), middle)]);

        assert_eq!(root_chain(&leaf, &parents), ["root", "middle", "leaf"]);
    }
}
