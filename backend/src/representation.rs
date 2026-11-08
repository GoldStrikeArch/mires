//! Conservative local heap-representation analysis.
//!
//! A representation may only change when every use of its allocation site is understood. This
//! first slice proves that narrow fact for local structs and tuples, and reports why `Box`, `Vec`
//! and `String` stay on their existing representation. It deliberately performs no rewrite yet:
//! changing only one side of a move or call would require a representation conversion, which is
//! outside the optimizer contract.

use rustc_middle::mir::visit::{MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{
    Body, Local, Location, Place, PlaceElem, Rvalue, StatementKind, RETURN_PLACE,
};
use rustc_middle::ty::{self, Ty};
use rustc_span::sym;

use crate::base::FnCx;
use crate::opts::{self, OptimizerMode};
use crate::value::{self, typing_env};

const PASS: &str = "heap-representation";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Struct,
    Tuple,
    Box,
    Vec,
    String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Facts {
    assignments: u32,
    aggregate_initializers: u32,
    field_reads: u32,
    field_writes: u32,
    bare_reads: u32,
    borrowed: bool,
    raw_address: bool,
    call_boundary: bool,
    drop_observed: bool,
    unsupported_projection: bool,
}

impl Facts {
    fn first_rejection(self, kind: Kind, needs_drop: bool) -> Option<&'static str> {
        if self.raw_address {
            return Some("raw pointer or byte provenance can observe the allocation");
        }
        match kind {
            Kind::Box => {
                return Some(
                    "Box elision needs cross-crate allocator, unsizing, leak, identity, and drop summaries",
                );
            }
            Kind::Vec => {
                return Some(
                    "native Vec lowering needs cross-crate length, capacity, allocator, and raw-parts summaries",
                );
            }
            Kind::String => {
                return Some(
                    "native String lowering needs cross-crate capacity and exact UTF-8 byte-operation summaries",
                );
            }
            Kind::Struct | Kind::Tuple => {}
        }
        if self.borrowed {
            return Some("a reference can observe the allocation identity or alias its mutation");
        }
        if self.call_boundary {
            return Some("the value crosses a call or yield boundary without a typed summary");
        }
        if self.bare_reads > 0 {
            return Some(
                "a whole-value move, copy, capture, argument, or return needs the baseline representation",
            );
        }
        if self.unsupported_projection {
            return Some("a non-field projection can observe aggregate layout or identity");
        }
        if self.drop_observed || needs_drop {
            return Some("drop behavior is observable");
        }
        if self.assignments != 1 || self.aggregate_initializers != 1 {
            return Some("the allocation site is not one local aggregate initialization");
        }
        if self.field_reads == 0 && self.field_writes == 0 {
            return Some("the allocation has no field operation to specialize");
        }
        None
    }
}

struct Collector {
    facts: Vec<Facts>,
}

impl Collector {
    fn classify(&mut self, place: &Place<'_>, context: PlaceContext) {
        let Some(facts) = self.facts.get_mut(place.local.as_usize()) else {
            return;
        };
        let projection = place.projection;
        let bare = projection.is_empty();
        let field_projection = matches!(projection.first(), Some(PlaceElem::Field(..)));

        if !bare && !field_projection {
            facts.unsupported_projection = true;
        }

        match context {
            PlaceContext::NonUse(_) => {}
            PlaceContext::NonMutatingUse(context) => match context {
                NonMutatingUseContext::SharedBorrow | NonMutatingUseContext::FakeBorrow => {
                    facts.borrowed = true;
                }
                NonMutatingUseContext::RawBorrow => {
                    facts.raw_address = true;
                }
                NonMutatingUseContext::Copy
                | NonMutatingUseContext::Move
                | NonMutatingUseContext::Inspect => {
                    if bare {
                        facts.bare_reads += 1;
                    } else if field_projection {
                        facts.field_reads += 1;
                    }
                }
                NonMutatingUseContext::PlaceMention | NonMutatingUseContext::Projection => {}
            },
            PlaceContext::MutatingUse(context) => match context {
                MutatingUseContext::Store => {
                    if bare {
                        facts.assignments += 1;
                    } else if field_projection {
                        facts.field_writes += 1;
                    }
                }
                MutatingUseContext::Call
                | MutatingUseContext::Yield
                | MutatingUseContext::AsmOutput => {
                    if bare {
                        facts.assignments += 1;
                    }
                    facts.call_boundary = true;
                }
                MutatingUseContext::Borrow => {
                    facts.borrowed = true;
                }
                MutatingUseContext::RawBorrow => {
                    facts.raw_address = true;
                }
                MutatingUseContext::Drop => {
                    facts.drop_observed = true;
                }
                MutatingUseContext::SetDiscriminant | MutatingUseContext::Retag => {
                    facts.unsupported_projection = true;
                }
                MutatingUseContext::Projection => {}
            },
        }
    }
}

impl<'tcx> Visitor<'tcx> for Collector {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        self.classify(place, context);
        self.super_place(place, context, location);
    }

    // `visit_place` sees the complete projection. Counting the local again after `super_place`
    // would turn every field access into a whole-value use.
    fn visit_local(&mut self, _local: Local, _context: PlaceContext, _location: Location) {}
}

/// Records representation choices and conservative fallbacks for one monomorphized body.
pub(crate) fn analyze<'tcx>(fx: &FnCx<'_, 'tcx>) {
    let mode = opts::get().optimizer;
    if mode == OptimizerMode::Off {
        return;
    }

    let kinds: Vec<Option<Kind>> = fx
        .mir
        .local_decls
        .iter()
        .map(|decl| kind_of(fx, fx.monomorphize(decl.ty)))
        .collect();
    let mut collector = Collector {
        facts: vec![Facts::default(); kinds.len()],
    };
    collector.visit_body(fx.mir);
    mark_aggregate_initializers(fx.mir, &mut collector.facts);

    for local in fx.mir.local_decls.indices() {
        if local == RETURN_PLACE || local.as_usize() <= fx.mir.arg_count {
            continue;
        }
        let Some(kind) = kinds[local.as_usize()] else {
            continue;
        };
        let ty = fx.monomorphize(fx.mir.local_decls[local].ty);
        let facts = collector.facts[local.as_usize()];
        if facts.assignments == 0 {
            continue;
        }

        if matches!(kind, Kind::Struct) && value::transparent_field_indexed(fx.tcx, ty).is_some() {
            fx.record_optimization_selection(
                PASS,
                format!("pinned-transparent-local-{}", local.as_usize()),
                "the type already uses its ABI-defined transparent representation",
            );
            continue;
        }

        let needs_drop = ty.needs_drop(fx.tcx, typing_env());
        match facts.first_rejection(kind, needs_drop) {
            Some(reason) => fx.record_optimization_miss_at(
                PASS,
                reason,
                fx.mir.local_decls[local].source_info.span,
            ),
            None if mode == OptimizerMode::Report => fx.record_optimization_selection(
                PASS,
                format!("scalar-fields-local-{}", local.as_usize()),
                "a non-escaping local aggregate is used only through its fields",
            ),
            None => fx.record_optimization_miss_at(
                PASS,
                "scalar replacement lowering waits for representation-neutral late IR",
                fx.mir.local_decls[local].source_info.span,
            ),
        }
    }
}

fn mark_aggregate_initializers(mir: &Body<'_>, facts: &mut [Facts]) {
    for block in mir.basic_blocks.iter() {
        for statement in &block.statements {
            let StatementKind::Assign(assignment) = &statement.kind else {
                continue;
            };
            let Some(local) = assignment.0.as_local() else {
                continue;
            };
            if matches!(&assignment.1, Rvalue::Aggregate(..)) {
                if let Some(facts) = facts.get_mut(local.as_usize()) {
                    facts.aggregate_initializers += 1;
                }
            }
        }
    }
}

fn kind_of<'tcx>(fx: &FnCx<'_, 'tcx>, ty: Ty<'tcx>) -> Option<Kind> {
    match ty.kind() {
        ty::Tuple(fields) if !fields.is_empty() => Some(Kind::Tuple),
        ty::Adt(def, _) if def.is_box() => Some(Kind::Box),
        ty::Adt(def, _) if fx.tcx.lang_items().string() == Some(def.did()) => Some(Kind::String),
        ty::Adt(def, _) if fx.tcx.is_diagnostic_item(sym::Vec, def.did()) => Some(Kind::Vec),
        ty::Adt(def, _) if def.is_struct() && !value::is_zst(fx.tcx, ty) => Some(Kind::Struct),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_fields() -> Facts {
        Facts {
            assignments: 1,
            aggregate_initializers: 1,
            field_reads: 2,
            ..Facts::default()
        }
    }

    #[test]
    fn local_field_only_struct_is_a_scalar_replacement_candidate() {
        assert_eq!(local_fields().first_rejection(Kind::Struct, false), None);
    }

    #[test]
    fn aliases_raw_pointers_and_drop_each_force_the_baseline() {
        let mut facts = local_fields();
        facts.borrowed = true;
        assert!(facts
            .first_rejection(Kind::Struct, false)
            .unwrap()
            .contains("reference"));

        let mut facts = local_fields();
        facts.raw_address = true;
        assert!(facts
            .first_rejection(Kind::Struct, false)
            .unwrap()
            .contains("raw pointer"));

        assert!(local_fields()
            .first_rejection(Kind::Struct, true)
            .unwrap()
            .contains("drop"));
    }

    #[test]
    fn vec_and_string_wait_for_capacity_and_byte_semantics() {
        assert!(local_fields()
            .first_rejection(Kind::Vec, false)
            .unwrap()
            .contains("capacity"));
        assert!(local_fields()
            .first_rejection(Kind::String, false)
            .unwrap()
            .contains("UTF-8"));
    }
}
