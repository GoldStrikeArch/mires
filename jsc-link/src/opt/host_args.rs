//! Removes private fixed argument slices at dynamic host calls.
//!
//! The array, borrow, and unsizing cast must form a single-use chain in the call's block.
//! Reusing that array preserves its element evaluation and host identity. Dynamic slices,
//! shared arrays, storage observations, and other coercions keep the ordinary call.
//! A promoted empty array is replaced with a fresh empty argument array at each call.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

use crate::jir::interns::Interns;
use crate::jir::repr::{Carrier, Func, OpKind, Root, ValueId};
use crate::jir::semantic::{Callee, CastKind, ConstTree, HostPayload};
use crate::jir::types::{PinnedBoundary, TypeKind};
use crate::jir::TypeTable;
use rcgjs_object::host::HostOperation;

use super::cfg::DefUse;

/// Why a dynamic host call site kept its ordinary argument slice.
///
/// Variants are ordered alphabetically by [`Self::as_code`] so `BTreeMap` iteration
/// matches `str` ordering and [`Borrow<str>`] satisfies its `Ord` contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HostArgsDecline {
    Arity,
    PackBorrow,
    PackCarrier,
    PackCoercion,
    PackDynamic,
    PackElement,
    PackEscapes,
    PackNotFresh,
    PackNotLocal,
}

impl HostArgsDecline {
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::Arity => "host.argument-arity",
            Self::PackBorrow => "host.argument-pack-borrow",
            Self::PackCarrier => "host.argument-pack-carrier",
            Self::PackCoercion => "host.argument-pack-coercion",
            Self::PackDynamic => "host.argument-pack-dynamic",
            Self::PackElement => "host.argument-pack-element",
            Self::PackEscapes => "host.argument-pack-escapes",
            Self::PackNotFresh => "host.argument-pack-not-fresh",
            Self::PackNotLocal => "host.argument-pack-not-local",
        }
    }
}

impl Borrow<str> for HostArgsDecline {
    fn borrow(&self) -> &str {
        self.as_code()
    }
}

impl Hash for HostArgsDecline {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_code().hash(state);
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostArgsStats {
    pub would_lower: u32,
    pub final_emitted: u32,
    pub declines: BTreeMap<HostArgsDecline, u32>,
}

impl HostArgsStats {
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"declines\":{");
        for (index, (reason, count)) in self.declines.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            crate::lane_report::push_string(&mut out, reason.as_code());
            let _ = write!(out, ":{count}");
        }
        let _ = write!(
            out,
            "}},\"final_emitted\":{},\"would_lower\":{}}}",
            self.final_emitted, self.would_lower
        );
        out
    }
}

pub fn run(func: &mut Func, table: &TypeTable, interns: &Interns) -> HostArgsStats {
    let mut stats = HostArgsStats::default();
    let def_use = DefUse::of(func);
    for block in &mut func.blocks {
        let defs: BTreeMap<_, _> = block
            .ops
            .iter()
            .enumerate()
            .filter_map(|(index, op)| op.result.map(|value| (value, index)))
            .collect();
        for index in 0..block.ops.len() {
            let OpKind::Call {
                callee: Callee::Direct { .. },
                function: None,
                track_caller: None,
                host: Some(payload @ HostPayload::Marker { section, .. }),
                args,
                ..
            } = &block.ops[index].kind
            else {
                continue;
            };
            let Some(operation @ (HostOperation::Invoke | HostOperation::Construct)) = interns
                .str(*section)
                .and_then(|section| section.strip_prefix("rcgjs.host."))
                .and_then(HostOperation::from_marker)
            else {
                continue;
            };
            let candidate = (|| {
                if args.len() != operation.arity() {
                    return Err(HostArgsDecline::Arity);
                }
                let definition = |value: ValueId| {
                    if def_use.use_count(value) != 1 {
                        return Err(HostArgsDecline::PackEscapes);
                    }
                    if func.values[value.0 as usize].carrier != Carrier::Baseline {
                        return Err(HostArgsDecline::PackCarrier);
                    }
                    defs.get(&value)
                        .copied()
                        .filter(|position| *position < index)
                        .ok_or(HostArgsDecline::PackNotLocal)
                };
                let cast = definition(*args.last().unwrap())?;
                let OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v,
                    from,
                    to,
                } = block.ops[cast].kind
                else {
                    return Err(HostArgsDecline::PackDynamic);
                };
                let Some(TypeKind::Reference {
                    pointee: array_ty,
                    mutable: false,
                }) = table.get(from).map(|ty| &ty.kind)
                else {
                    return Err(HostArgsDecline::PackCoercion);
                };
                let Some(TypeKind::Array { element, length }) =
                    table.get(*array_ty).map(|ty| &ty.kind)
                else {
                    return Err(HostArgsDecline::PackCoercion);
                };
                let Some(TypeKind::Reference {
                    pointee: slice_ty,
                    mutable: false,
                }) = table.get(to).map(|ty| &ty.kind)
                else {
                    return Err(HostArgsDecline::PackCoercion);
                };
                if !matches!(table.get(*slice_ty).map(|ty| &ty.kind), Some(TypeKind::Slice(ty)) if ty == element)
                    || !table.get(*element).is_some_and(|ty| {
                        ty.meta.boundary == PinnedBoundary::Host
                            && !ty.js.needs_clone
                            && !ty.layout.needs_drop
                    })
                {
                    return Err(HostArgsDecline::PackElement);
                }
                let borrow = definition(v)?;
                if *length == 0 && borrow < cast {
                    if let OpKind::Const(ConstTree::Reference(value)) = &block.ops[borrow].kind {
                        if matches!(value.as_ref(), ConstTree::Aggregate(fields) if fields.is_empty())
                        {
                            return Ok((*args.last().unwrap(), Some((cast, *array_ty))));
                        }
                    }
                }
                let OpKind::BorrowView {
                    root: Root::Value(array),
                    path,
                    mutable: false,
                } = &block.ops[borrow].kind
                else {
                    return Err(HostArgsDecline::PackBorrow);
                };
                if !path.is_empty()
                    || func.values[array.0 as usize].ty != *array_ty
                    || borrow >= cast
                {
                    return Err(HostArgsDecline::PackBorrow);
                }
                let fresh = definition(*array)?;
                let OpKind::Aggregate { ty, fields, .. } = &block.ops[fresh].kind else {
                    return Err(HostArgsDecline::PackNotFresh);
                };
                if ty != array_ty || fields.len() as u64 != *length || fresh >= borrow {
                    return Err(HostArgsDecline::PackNotFresh);
                }
                Ok((*array, None))
            })();
            match candidate {
                Ok((array, empty)) => {
                    let mut args = args.clone();
                    let payload = payload.clone();
                    *args.last_mut().unwrap() = array;
                    if let Some((cast, ty)) = empty {
                        // The promoted Rust allocation is never exposed to Reflect, whose
                        // replacement may retain or mutate the argument array.
                        block.ops[cast].kind = OpKind::Aggregate {
                            ty,
                            variant: 0,
                            active_field: None,
                            fields: Vec::new(),
                        };
                        func.values[array.0 as usize].ty = ty;
                    }
                    block.ops[index].kind = OpKind::Host { payload, args };
                    stats.would_lower += 1;
                }
                Err(reason) => *stats.declines.entry(reason).or_default() += 1,
            }
        }
    }
    stats
}
