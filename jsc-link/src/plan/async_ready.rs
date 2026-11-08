//! An opt-in Promise driver for generated coroutines with no suspension or observable cleanup.
//! The concrete poll/drop callbacks remain authoritative. Only unreachable scheduler work is cut.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::repr::{Func, OpKind};
use crate::jir::semantic::HostPayload;
use crate::jir::types::{TypeId, TypeKind, TypeTable};
use crate::opts::{OptimizerMode, OptimizerPass, Opts};

use super::async_graph::SourceCaptures;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReadyPlan {
    pub selected: BTreeSet<TypeId>,
    pub rows: Vec<serde_json::Value>,
}

impl ReadyPlan {
    pub fn disable(&mut self) {
        self.selected.clear();
        for row in &mut self.rows {
            row["selected"] = false.into();
            row["would_select"] = false.into();
            if row["eligible"] == true {
                row["reason"] = "proof.program-retry".into();
            }
        }
    }
    pub fn of(
        table: &BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        sources: &SourceCaptures,
        opts: &Opts,
    ) -> Self {
        if !opts.ready_futures || !opts.analyzes(OptimizerPass::FuturePoller) {
            return Self::default();
        }
        let mut candidates = Vec::new();
        for (name, func) in funcs {
            let (Ok(func), Some(interns)) =
                (func, table.get(name).and_then(|item| item.jir.interns()))
            else {
                continue;
            };
            for op in func.blocks.iter().flat_map(|block| &block.ops) {
                let OpKind::Call {
                    host: Some(payload @ HostPayload::Marker { section, .. }),
                    args,
                    ..
                } = &op.kind
                else {
                    continue;
                };
                if interns.str(*section) != Some("rcgjs.host.futurepromise") {
                    continue;
                }
                let Some(value) = args
                    .first()
                    .and_then(|value| func.values.get(value.0 as usize))
                else {
                    continue;
                };
                let result = if args.len() != 1 || payload.helper(interns, "poll_result").is_none()
                {
                    Err("driver.poll-contract-unavailable")
                } else {
                    prove(value.ty, &interns.types, table, funcs, sources, opts)
                };
                candidates.push((name.clone(), value.ty, result));
            }
        }
        // Do not add another runtime facility while an ineligible boundary still needs the
        // generic driver. Complete-package profitability remains an opt-in admission gate.
        let all_eligible = candidates.iter().all(|(_, _, result)| result.is_ok());
        let mut plan = Self::default();
        for (name, future, proof) in candidates {
            let eligible = proof.is_ok();
            let would_select = eligible && all_eligible;
            let selected = would_select && opts.applies(OptimizerPass::FuturePoller);
            if selected {
                plan.selected.insert(future);
            }
            plan.rows.push(serde_json::json!({
                "item": name.as_str(), "future_type": format!("{:032x}", future.0),
                "eligible": eligible, "would_select": would_select, "selected": selected,
                "reason": proof.err().unwrap_or(if all_eligible { "proven.ready-coroutine" } else { "economics.generic-driver-retained" }),
                "poll_callback_retained": true, "drop_callback_retained": true,
                "package_profitability_proven": false,
            }));
        }
        plan
    }
}

fn prove(
    future: TypeId,
    types: &TypeTable,
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    sources: &SourceCaptures,
    opts: &Opts,
) -> Result<(), &'static str> {
    let Some(TypeKind::Coroutine {
        def,
        upvars,
        states,
        saved_locals,
        ..
    }) = types.get(future).map(|ty| &ty.kind)
    else {
        return Err("legality.custom-poller");
    };
    let source = sources.source(*def).ok_or("graph.source-unavailable")?;
    if !source.yields.is_empty() || states.len() != 3 || !saved_locals.is_empty() {
        return Err("graph.suspending-coroutine");
    }
    if source.blocks.iter().any(|block| {
        block.kind == crate::jir::coroutine::SourceTerminator::Drop && block.drop_local != Some(1)
    }) {
        return Err("legality.source-local-cleanup");
    }
    if upvars
        .iter()
        .any(|ty| !plain_capture(types, *ty, &mut BTreeSet::new()))
    {
        return Err("legality.retained-state");
    }
    let mut matching = funcs.iter().filter_map(|(name, func)| {
        let func = func.as_ref().ok()?;
        if func.item.definition != *def || func.sig.arguments.len() != 2 {
            return None;
        }
        let interns = table.get(name)?.jir.interns()?;
        let first = interns.types.peeled(func.sig.arguments[0]);
        match &interns.types.get(first)?.kind {
            TypeKind::Reference { pointee, .. } if *pointee == future => Some((func, interns)),
            _ => None,
        }
    });
    let (func, interns) = matching.next().ok_or("driver.generated-poll-unavailable")?;
    if matching.next().is_some() {
        return Err("driver.generated-poll-ambiguous");
    }
    let mut func = func.clone();
    let mut proof_opts = opts.clone();
    proof_opts.optimizer = OptimizerMode::On;
    crate::opt::optimize(
        &mut func,
        &interns.types,
        interns,
        &proof_opts,
        &mut Default::default(),
    )
    .map_err(|_| "proof.poll-lowering")?;
    let context = *func.params.get(1).ok_or("driver.context-unavailable")?;
    if func.blocks.iter().any(|block| {
        block.term.uses().contains(&context)
            || block.ops.iter().any(|op| op.kind.uses().contains(&context))
    }) {
        return Err("legality.context-observed");
    }
    // Ordinary callees could hide source-local cleanup, including after rustc MIR inlining.
    // This first slice admits only typed extern calls in the coroutine's source body as well.
    let item = table
        .values()
        .find(|item| {
            item.jir
                .body
                .as_ref()
                .is_some_and(|body| body.item == func.item)
        })
        .and_then(|item| item.jir.body.as_ref())
        .ok_or("proof.poll-body-unavailable")?;
    let externs: BTreeSet<_> = item
        .blocks
        .iter()
        .filter_map(|block| {
            let crate::jir::semantic::Terminator::Call(call) = &block.terminator else {
                return None;
            };
            let crate::jir::semantic::Callee::Direct { item, .. } = &call.callee else {
                return None;
            };
            matches!(call.host, Some(HostPayload::Extern(_))).then_some(item.definition)
        })
        .collect();
    if source.blocks.iter().any(|block| {
        matches!(
            block.kind,
            crate::jir::coroutine::SourceTerminator::Call
                | crate::jir::coroutine::SourceTerminator::TailCall
        ) && block.callee.is_none_or(|callee| !externs.contains(&callee))
    }) {
        return Err("legality.source-callee-cleanup");
    }
    if item.blocks.iter().any(|block| matches!(&block.terminator,
        crate::jir::semantic::Terminator::Call(call) if !matches!(call.host, Some(HostPayload::Extern(_))))) {
        return Err("legality.poll-callee-cleanup");
    }
    if func.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            &op.kind,
            OpKind::Intrinsic { .. }
                | OpKind::RawView { .. }
                | OpKind::Drop { .. }
                | OpKind::Zombie(_)
                | OpKind::Host { .. }
                | OpKind::Call { host: None, .. }
                | OpKind::Call {
                    host: Some(
                        HostPayload::Block(_)
                            | HostPayload::Marker { .. }
                            | HostPayload::ViewAbi { .. }
                            | HostPayload::Map { .. }
                            | HostPayload::KeyedList(_)
                    ),
                    ..
                }
        )
    }) {
        return Err("legality.poll-effects");
    }
    Ok(())
}

fn plain_capture(types: &TypeTable, ty: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
    if !seen.insert(ty) {
        return true;
    }
    let Some(descriptor) = types.get(ty) else {
        return false;
    };
    if descriptor.layout.needs_drop {
        return false;
    }
    match &descriptor.kind {
        TypeKind::Bool
        | TypeKind::Char
        | TypeKind::Integer { .. }
        | TypeKind::Float { .. }
        | TypeKind::Opaque => true,
        TypeKind::Tuple(fields) => fields.iter().all(|ty| plain_capture(types, *ty, seen)),
        TypeKind::Array { element, .. } | TypeKind::Pattern(element) => {
            plain_capture(types, *element, seen)
        }
        TypeKind::Adt { variants, .. } => variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .all(|field| plain_capture(types, field.ty, seen)),
        _ => false,
    }
}
