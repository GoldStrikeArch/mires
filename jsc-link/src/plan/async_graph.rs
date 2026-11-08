//! Async facts from source captures and the decoded post-transform type table.
//!
//! The table proves coroutine states and saved-local types, but it does not prove canonical await
//! relationships or wake sources. Suspension graphs remain report-only; the separate ready plan
//! proves a bounded nonsuspending driver selection against monomorphic code.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::item::{JsName, LinkItem};
use crate::jir::coroutine::{CoroutineSource, SourceEdgeKind, SourceTerminator};
use crate::jir::decline::Decline;
use crate::jir::repr::Func;
use crate::jir::types::{TypeId, TypeKind, TypeTable};
use crate::jir::{DefinitionId, Interns};
use crate::lane_report::push_string;

/// The versioned async report and optional ready driver plan for one lane run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AsyncPlan {
    graphs: Vec<AsyncGraph>,
    pub(crate) ready: super::async_ready::ReadyPlan,
}

/// The state facts visible for one coroutine type used by one item.
#[derive(Clone, Debug, Eq, PartialEq)]
struct AsyncGraph {
    item: JsName,
    coroutine: TypeId,
    state_count: usize,
    estimated_suspension_state_count: usize,
    saved_locals: usize,
    reference_saved_locals: usize,
    raw_pointer_saved_locals: usize,
    cleanup_saved_locals: usize,
    source: SourceCapture,
    runtime_frame_matches_type: bool,
    reason: &'static str,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum SourceCapture {
    #[default]
    Missing,
    Conflicting,
    Captured(SourceReport),
}

impl SourceCapture {
    fn json(&self, out: &mut String) {
        let Self::Captured(source) = self else {
            out.push_str(match self {
                Self::Missing => "{\"status\":\"unavailable\"}",
                _ => "{\"status\":\"conflicting\"}",
            });
            return;
        };
        let _ = write!(out, "{{\"status\":\"captured\",\"producer\":\"rustc-mir-promoted-before-runtime-lowering\",\"block_count\":{},\"local_count\":{},\"yield_count\":{},\"yields\":[",
            source.facts.block_count, source.facts.local_count, source.facts.yields.len());
        for (index, (point, location)) in source
            .facts
            .yields
            .iter()
            .zip(&source.locations)
            .enumerate()
        {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"block\":{},\"resume\":{},\"drop\":",
                point.block, point.resume
            );
            if let Some(drop) = point.drop {
                let _ = write!(out, "{drop}");
            } else {
                out.push_str("null");
            }
            out.push_str(",\"resume_local\":");
            if let Some(local) = point.resume_local {
                let _ = write!(out, "{local}");
            } else {
                out.push_str("null");
            }
            out.push_str(",\"liveness\":");
            out.push_str(
                &serde_json::to_string(&point.liveness).expect("source liveness serializes"),
            );
            out.push_str(",\"source\":");
            if let Some((file, line, col)) = location {
                out.push_str("{\"file\":");
                push_string(out, file);
                let _ = write!(out, ",\"line\":{line},\"column\":{col}}}");
            } else {
                out.push_str("null");
            }
            out.push('}');
        }
        out.push_str("],\"runtime_frame\":");
        out.push_str(
            &serde_json::to_string(&source.facts.runtime_frame).expect("runtime frame serializes"),
        );
        out.push_str(",\"initialization\":");
        out.push_str(
            &serde_json::to_string(&source.facts.initialization)
                .expect("source initialization serializes"),
        );
        out.push_str(",\"blocks\":");
        out.push_str(&serde_json::to_string(&source.facts.blocks).expect("source CFG serializes"));
        out.push_str(",\"segments\":");
        out.push_str(
            &serde_json::to_string(&segments(&source.facts)).expect("source segments serialize"),
        );
        out.push('}');
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceReport {
    /// Source ids are removed here because they are local to an object's intern table.
    facts: CoroutineSource,
    locations: Vec<Option<(String, u32, u32)>>,
}

impl SourceReport {
    fn of(source: &CoroutineSource, interns: &Interns) -> Self {
        let mut facts = source.clone();
        let locations = facts
            .yields
            .iter_mut()
            .map(|point| {
                let fact = interns.source_facts.get(point.source.take()?.0 as usize)?;
                Some((interns.str(fact.file)?.to_owned(), fact.line, fact.col))
            })
            .collect();
        Self { facts, locations }
    }
}

/// Normal-flow reachability from entry and each resume, stopping at the next yield.
/// Segments can overlap and contain cycles. Imaginary borrow-check edges are excluded; unwind
/// and cancellation edges remain in the source graph but do not establish normal progress.
fn segments(source: &CoroutineSource) -> Vec<serde_json::Value> {
    std::iter::once(0)
        .chain(source.yields.iter().map(|point| point.resume))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|entry| {
            let mut blocks = BTreeSet::new();
            let mut pending = vec![entry];
            let mut yields = BTreeSet::new();
            let mut terminals = BTreeSet::new();
            let mut unwind = BTreeSet::new();
            while let Some(index) = pending.pop() {
                if !blocks.insert(index) {
                    continue;
                }
                let block = &source.blocks[index as usize];
                if block.kind == SourceTerminator::Yield {
                    yields.insert(index);
                    continue;
                }
                if !block
                    .successors
                    .iter()
                    .any(|edge| edge.kind == SourceEdgeKind::Normal)
                {
                    terminals.insert(index);
                }
                for edge in &block.successors {
                    match edge.kind {
                        SourceEdgeKind::Normal => pending.push(edge.target),
                        SourceEdgeKind::Unwind => {
                            unwind.insert((index, edge.target));
                        }
                        SourceEdgeKind::Resume
                        | SourceEdgeKind::Cancel
                        | SourceEdgeKind::Imaginary => {}
                    }
                }
            }
            serde_json::json!({
                "entry": entry, "blocks": blocks, "yield_blocks": yields,
                "terminal_blocks": terminals, "unwind_edges": unwind,
            })
        })
        .collect()
}

/// Collected from all objects before item deduplication or reachability can discard them.
#[derive(Default)]
pub(crate) struct SourceCaptures(BTreeMap<DefinitionId, SourceCapture>);

impl SourceCaptures {
    pub(crate) fn source(&self, def: DefinitionId) -> Option<&CoroutineSource> {
        match self.0.get(&def)? {
            SourceCapture::Captured(source) => Some(&source.facts),
            _ => None,
        }
    }
    pub(crate) fn extend(&mut self, interns: &Interns) {
        for (&definition, source) in &interns.coroutine_sources {
            let candidate = SourceCapture::Captured(SourceReport::of(source, interns));
            let current = self
                .0
                .entry(definition)
                .or_insert_with(|| candidate.clone());
            if *current != candidate {
                *current = SourceCapture::Conflicting;
            }
        }
    }
}

impl AsyncPlan {
    /// Derives only facts available in decoded JIR and records why they cannot authorize lowering.
    pub(crate) fn of(
        table: &BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        sources: &SourceCaptures,
        opts: &crate::opts::Opts,
    ) -> Self {
        let mut graphs: BTreeMap<TypeId, ((u8, u8), AsyncGraph)> = BTreeMap::new();
        for (item, func) in funcs {
            let Ok(func) = func else {
                continue;
            };
            let Some(types) = type_table(table, item) else {
                continue;
            };
            let roots = func
                .sig
                .arguments
                .iter()
                .copied()
                .chain(std::iter::once(func.sig.result))
                .chain(func.values.iter().map(|value| value.ty))
                .chain(func.cells.iter().map(|cell| cell.ty));
            for coroutine in coroutine_types(types, roots) {
                let Some(TypeKind::Coroutine {
                    def,
                    saved_locals,
                    states,
                    ..
                }) = types.get(coroutine).map(|descriptor| &descriptor.kind)
                else {
                    continue;
                };
                let reference_saved_locals = saved_locals
                    .iter()
                    .filter(|(_, ty)| contains_kind(types, *ty, is_reference))
                    .count();
                let raw_pointer_saved_locals = saved_locals
                    .iter()
                    .filter(|(_, ty)| contains_kind(types, *ty, is_raw_pointer))
                    .count();
                let cleanup_saved_locals = saved_locals
                    .iter()
                    .filter(|(_, ty)| {
                        types
                            .get(*ty)
                            .is_some_and(|descriptor| descriptor.layout.needs_drop)
                    })
                    .count();
                let reason = if raw_pointer_saved_locals > 0 {
                    "legality.raw-pointer-across-suspension"
                } else if reference_saved_locals > 0 {
                    "legality.borrow-across-suspension"
                } else if cleanup_saved_locals > 0 {
                    "legality.cleanup-state-unavailable"
                } else {
                    "graph.await-relationship-unavailable"
                };
                let candidate = AsyncGraph {
                    item: item.clone(),
                    coroutine,
                    state_count: states.len(),
                    estimated_suspension_state_count: states.len().saturating_sub(3),
                    saved_locals: saved_locals.len(),
                    reference_saved_locals,
                    raw_pointer_saved_locals,
                    cleanup_saved_locals,
                    runtime_frame_matches_type: sources
                        .source(*def)
                        .and_then(|source| source.runtime_frame.as_ref())
                        .is_some_and(|frame| {
                            frame.saved_local_sources.len() == saved_locals.len()
                                && &frame.states == states
                        }),
                    source: sources.0.get(def).cloned().unwrap_or_default(),
                    reason,
                };
                let rank = (owner_rank(func, coroutine), library_rank(item));
                match graphs.entry(coroutine) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert((rank, candidate));
                    }
                    std::collections::btree_map::Entry::Occupied(mut slot)
                        if (rank, item.as_str()) < (slot.get().0, slot.get().1.item.as_str()) =>
                    {
                        slot.insert((rank, candidate));
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {}
                }
            }
        }
        let mut graphs: Vec<AsyncGraph> = graphs.into_values().map(|(_, graph)| graph).collect();
        graphs.sort_by(|left, right| {
            (left.item.as_str(), left.coroutine).cmp(&(right.item.as_str(), right.coroutine))
        });
        Self {
            graphs,
            ready: super::async_ready::ReadyPlan::of(table, funcs, sources, opts),
        }
    }

    /// The schema-8 `async` block in deterministic item and type order.
    pub fn json(&self) -> String {
        let mut out = format!(
            "{{\"schema\":8,\"mode\":\"{}\",\"ready_boundaries\":{},\"graphs\":[",
            if self.ready.selected.is_empty() {
                "report-only"
            } else {
                "ready-boundaries"
            },
            serde_json::to_string(&self.ready.rows).expect("ready boundaries serialize")
        );
        for (index, graph) in self.graphs.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, graph.item.as_str());
            out.push_str(",\"coroutine_type\":");
            push_string(&mut out, &format!("{:032x}", graph.coroutine.0));
            let _ = write!(
                out,
                ",\"state_count\":{},\"estimated_suspension_state_count\":{},\
                 \"suspension_state_count_derivation\":\"state-count-minus-three\",\
                 \"saved_locals\":{},\"reference_saved_locals\":{},\
                 \"raw_pointer_saved_locals\":{},\"cleanup_saved_locals\":{}",
                graph.state_count,
                graph.estimated_suspension_state_count,
                graph.saved_locals,
                graph.reference_saved_locals,
                graph.raw_pointer_saved_locals,
                graph.cleanup_saved_locals,
            );
            out.push_str(",\"await_relationships_proven\":false,\"wake_sources_proven\":false");
            let _ = write!(
                out,
                ",\"runtime_frame_matches_type\":{}",
                graph.runtime_frame_matches_type
            );
            out.push_str(",\"source_suspensions\":");
            graph.source.json(&mut out);
            out.push_str(",\"selected\":false,\"reason\":");
            push_string(&mut out, graph.reason);
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

fn owner_rank(func: &Func, coroutine: TypeId) -> u8 {
    if func.sig.result == coroutine {
        0
    } else if func.values.iter().any(|value| value.ty == coroutine)
        || func.cells.iter().any(|cell| cell.ty == coroutine)
    {
        1
    } else if func.sig.arguments.contains(&coroutine) {
        2
    } else {
        3
    }
}

fn library_rank(item: &JsName) -> u8 {
    u8::from(item.as_str().starts_with("core$") || item.as_str().starts_with("alloc$"))
}

fn type_table<'a>(table: &'a BTreeMap<JsName, LinkItem>, name: &JsName) -> Option<&'a TypeTable> {
    table
        .get(name)
        .and_then(|item| item.jir.interns())
        .map(|interns| &interns.types)
}

fn coroutine_types(types: &TypeTable, roots: impl IntoIterator<Item = TypeId>) -> BTreeSet<TypeId> {
    let mut found = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut pending: Vec<TypeId> = roots.into_iter().collect();
    while let Some(ty) = pending.pop() {
        if !seen.insert(ty) {
            continue;
        }
        let Some(descriptor) = types.get(ty) else {
            continue;
        };
        if matches!(descriptor.kind, TypeKind::Coroutine { .. }) {
            found.insert(ty);
        }
        pending.extend(child_types(&descriptor.kind));
    }
    found
}

fn contains_kind(types: &TypeTable, root: TypeId, predicate: fn(&TypeKind) -> bool) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(ty) = pending.pop() {
        if !seen.insert(ty) {
            continue;
        }
        let Some(descriptor) = types.get(ty) else {
            continue;
        };
        if predicate(&descriptor.kind) {
            return true;
        }
        pending.extend(child_types(&descriptor.kind));
    }
    false
}

fn is_reference(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::Reference { .. })
}

fn is_raw_pointer(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::RawPointer { .. })
}

fn child_types(kind: &TypeKind) -> Vec<TypeId> {
    match kind {
        TypeKind::Tuple(fields) => fields.clone(),
        TypeKind::Array { element, .. } | TypeKind::Slice(element) | TypeKind::Pattern(element) => {
            vec![*element]
        }
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            vec![*pointee]
        }
        TypeKind::Adt { args, variants, .. } => args
            .iter()
            .copied()
            .chain(
                variants
                    .iter()
                    .flat_map(|variant| variant.fields.iter().map(|field| field.ty)),
            )
            .collect(),
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
            upvars.clone()
        }
        TypeKind::Coroutine {
            upvars,
            saved_locals,
            ..
        } => upvars
            .iter()
            .copied()
            .chain(saved_locals.iter().map(|(_, ty)| *ty))
            .collect(),
        TypeKind::Bool
        | TypeKind::Char
        | TypeKind::Integer { .. }
        | TypeKind::Float { .. }
        | TypeKind::Str
        | TypeKind::Never
        | TypeKind::Opaque
        | TypeKind::FnDef { .. }
        | TypeKind::FnPtr { .. }
        | TypeKind::Dynamic { .. }
        | TypeKind::Foreign(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::coroutine::{
        SourceBlock, SourceEdge, SourceEdgeKind, SourceTerminator, SourceYield,
    };
    use crate::jir::SourceFact;

    #[test]
    fn source_segments_stop_at_yields_and_exclude_nonexecuting_edges() {
        use SourceEdgeKind::{Cancel, Imaginary, Normal, Resume, Unwind};
        use SourceTerminator::{Call, CoroutineDrop, Drop, FalseEdge, Return, Switch, Yield};
        let block = |kind, edges: &[(SourceEdgeKind, u32)]| SourceBlock {
            drop_local: None,
            callee: None,
            kind,
            successors: edges
                .iter()
                .map(|&(kind, target)| SourceEdge { kind, target })
                .collect(),
        };
        let point = |block, resume| SourceYield {
            liveness: None,
            block,
            resume,
            drop: Some(6),
            resume_local: None,
            source: None,
        };
        let source = CoroutineSource {
            initialization: None,
            runtime_frame: None,
            block_count: 9,
            local_count: 1,
            blocks: vec![
                block(Switch, &[(Normal, 1), (Normal, 2)]),
                block(FalseEdge, &[(Normal, 3), (Imaginary, 6)]),
                block(Call, &[(Normal, 3), (Unwind, 6)]),
                block(Yield, &[(Resume, 4), (Cancel, 6)]),
                block(Switch, &[(Normal, 0), (Normal, 5)]),
                block(Drop, &[(Normal, 8), (Cancel, 6)]),
                block(CoroutineDrop, &[]),
                block(Yield, &[(Resume, 8), (Cancel, 6)]),
                block(Return, &[]),
            ],
            yields: vec![point(3, 4), point(7, 8)],
        };
        assert!(source.valid(&Interns::new()));
        assert_eq!(
            segments(&source),
            vec![
                serde_json::json!({"entry":0,"blocks":[0,1,2,3],"yield_blocks":[3],"terminal_blocks":[],"unwind_edges":[[2,6]]}),
                serde_json::json!({"entry":4,"blocks":[0,1,2,3,4,5,8],"yield_blocks":[3],"terminal_blocks":[8],"unwind_edges":[[2,6]]}),
                serde_json::json!({"entry":8,"blocks":[8],"yield_blocks":[],"terminal_blocks":[8],"unwind_edges":[]}),
            ]
        );
    }

    #[test]
    fn merges_source_facts_by_definition_and_resolved_origin() {
        let definition = DefinitionId {
            crate_id: 1,
            local_hash: 2,
        };
        let source = CoroutineSource {
            initialization: None,
            runtime_frame: None,
            block_count: 3,
            local_count: 2,
            blocks: vec![
                SourceBlock {
                    drop_local: None,
                    callee: None,
                    kind: SourceTerminator::Yield,
                    successors: vec![
                        SourceEdge {
                            kind: SourceEdgeKind::Resume,
                            target: 1,
                        },
                        SourceEdge {
                            kind: SourceEdgeKind::Cancel,
                            target: 2,
                        },
                    ],
                },
                SourceBlock {
                    drop_local: None,
                    callee: None,
                    kind: SourceTerminator::Return,
                    successors: vec![],
                },
                SourceBlock {
                    drop_local: None,
                    callee: None,
                    kind: SourceTerminator::CoroutineDrop,
                    successors: vec![],
                },
            ],
            yields: vec![SourceYield {
                liveness: None,
                block: 0,
                resume: 1,
                drop: Some(2),
                resume_local: None,
                source: None,
            }],
        };
        let make = |padding: bool| {
            let mut interns = Interns::new();
            if padding {
                let file = interns.intern_str("unrelated.rs");
                interns.intern_source_fact(SourceFact {
                    file,
                    line: 1,
                    col: 0,
                });
            }
            let file = interns.intern_str("future.rs");
            let position = interns.intern_source_fact(SourceFact {
                file,
                line: 2,
                col: 8,
            });
            let mut source = source.clone();
            source.yields[0].source = Some(position);
            interns.coroutine_sources.insert(definition, source);
            interns
        };
        let first = make(false);
        let mut second = make(true);
        let mut captures = SourceCaptures::default();
        captures.extend(&first);
        captures.extend(&second);
        assert!(matches!(
            captures.0[&definition],
            SourceCapture::Captured(_)
        ));
        let mut json = String::new();
        captures.0[&definition].json(&mut json);
        let report: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(report["yields"][0]["source"]["file"], "future.rs");
        assert!(report["yields"][0]["resume_local"].is_null());
        second
            .coroutine_sources
            .get_mut(&definition)
            .unwrap()
            .yields[0]
            .resume = 2;
        captures.extend(&second);
        captures.extend(&first);
        assert_eq!(captures.0[&definition], SourceCapture::Conflicting);
        json.clear();
        captures.0[&definition].json(&mut json);
        assert_eq!(json, "{\"status\":\"conflicting\"}");
        json.clear();
        SourceCapture::default().json(&mut json);
        assert_eq!(json, "{\"status\":\"unavailable\"}");
    }
}
