//! Report-only eligibility for iterator/bounds and formatting specialization.
//!
//! These analyses deliberately do not participate in the executable pass mask. They describe
//! work that a future late-IR rewrite could perform, using MIR calls and rustc semantic identities
//! rather than workload names or JavaScript spellings. Keeping them report-only makes
//! `js-opt=report` byte-identical to `off` and avoids claiming a pass before there is a measured,
//! exact lowering for it.

use std::collections::HashMap;

use rustc_abi::Size;
use rustc_hir::LangItem;
use rustc_middle::mir::interpret::{GlobalAlloc, Scalar};
use rustc_middle::mir::{
    self, BasicBlock, Body, ConstValue, Local, Operand, Place, Rvalue, StatementKind,
    TerminatorKind,
};
use rustc_middle::ty::{self, Instance, Ty};
use rustc_span::Symbol;

use crate::base::FnCx;
use crate::opts::{self, OptimizerMode};
use crate::value::typing_env;

const ITERATOR_ANALYSIS: &str = "iterator-bounds-analysis";
const FORMATTING_ANALYSIS: &str = "formatting-analysis";

/// Records deterministic, report-only optimization opportunities for one monomorphized body.
pub(crate) fn analyze<'tcx>(fx: &FnCx<'_, 'tcx>) {
    if opts::get().optimizer == OptimizerMode::Off {
        return;
    }

    analyze_iterators(fx);
    analyze_formatting(fx);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IteratorKind {
    Range,
    Slice,
    Array,
    EnumerateRange,
    EnumerateSlice,
    EnumerateArray,
}

impl IteratorKind {
    fn selection(self, block: BasicBlock) -> String {
        let kind = match self {
            Self::Range => "range",
            Self::Slice => "slice",
            Self::Array => "array",
            Self::EnumerateRange => "enumerate-range",
            Self::EnumerateSlice => "enumerate-slice",
            Self::EnumerateArray => "enumerate-array",
        };
        format!("scalarize-{kind}-loop-bb{}", block.as_usize())
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Range => "a concrete integer Range uses the canonical Iterator::next loop",
            Self::Slice => "a concrete slice iterator uses the canonical Iterator::next loop",
            Self::Array => "a concrete array iterator uses the canonical Iterator::next loop",
            Self::EnumerateRange | Self::EnumerateSlice | Self::EnumerateArray => {
                "one concrete non-escaping iterator is wrapped by one canonical Enumerate"
            }
        }
    }
}

fn analyze_iterators<'tcx>(fx: &FnCx<'_, 'tcx>) {
    for (block, data) in fx.mir.basic_blocks.iter_enumerated() {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            target,
            ..
        } = &data.terminator().kind
        else {
            continue;
        };
        let Some(instance) = resolve_call(fx, func) else {
            continue;
        };
        if !implements_lang_item(fx, instance, LangItem::IteratorNext) {
            continue;
        }

        let span = data.terminator().source_info.span;
        let Some(receiver) = args.first() else {
            fx.record_optimization_miss_at(
                ITERATOR_ANALYSIS,
                "Iterator::next has no concrete receiver",
                span,
            );
            continue;
        };
        let receiver_ty = fx.monomorphize(receiver.node.ty(&fx.mir.local_decls, fx.tcx));
        let kind = match iterator_kind(fx, receiver_ty, false) {
            Ok(kind) => kind,
            Err(reason) => {
                fx.record_optimization_miss_at(ITERATOR_ANALYSIS, reason, span);
                continue;
            }
        };
        if !target.is_some_and(|target| canonical_option_dispatch(fx.mir, *destination, target)) {
            fx.record_optimization_miss_at(
                ITERATOR_ANALYSIS,
                "Iterator::next result does not feed the canonical Option loop dispatch",
                span,
            );
            continue;
        }
        if !target.is_some_and(|target| has_back_edge(fx.mir, target, block)) {
            fx.record_optimization_miss_at(
                ITERATOR_ANALYSIS,
                "Iterator::next dispatch is not part of a canonical loop",
                span,
            );
            continue;
        }

        fx.record_optimization_selection(ITERATOR_ANALYSIS, kind.selection(block), kind.reason());
    }

    // A check is only advertised as removable when a separate, dominating length proof exists.
    // The first implementation intentionally recognizes no implicit relationship between a Range
    // and a slice: proving that relationship requires the shared value-flow solver. Recording the
    // first fallback now keeps the report honest and gives the future rewrite a stable target.
    for data in fx.mir.basic_blocks.iter() {
        let TerminatorKind::Assert { msg, .. } = &data.terminator().kind else {
            continue;
        };
        if !matches!(msg.as_ref(), mir::AssertKind::BoundsCheck { .. }) {
            continue;
        }
        fx.record_optimization_miss_at(
            ITERATOR_ANALYSIS,
            "bounds check lacks a dominating same-length iterator proof",
            data.terminator().source_info.span,
        );
    }
}

fn iterator_kind<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    mut ty: Ty<'tcx>,
    inside_enumerate: bool,
) -> Result<IteratorKind, &'static str> {
    while let ty::Ref(_, pointee, _) = *ty.kind() {
        ty = pointee;
    }
    let ty::Adt(definition, arguments) = *ty.kind() else {
        return Err(
            "iterator type is not a concrete Range, slice, array, or simple Enumerate graph",
        );
    };

    if fx
        .tcx
        .lang_items()
        .range_struct()
        .is_some_and(|range| range == definition.did())
    {
        let Some(element) = arguments.types().next() else {
            return Err("Range has no concrete element type");
        };
        if !matches!(element.kind(), ty::Int(_) | ty::Uint(_)) {
            return Err("range element is not a fixed-width Rust integer");
        }
        return Ok(if inside_enumerate {
            IteratorKind::EnumerateRange
        } else {
            IteratorKind::Range
        });
    }

    if diagnostic_adt(fx, definition.did(), "SliceIter") {
        return Ok(if inside_enumerate {
            IteratorKind::EnumerateSlice
        } else {
            IteratorKind::Slice
        });
    }
    if diagnostic_adt(fx, definition.did(), "ArrayIntoIter") {
        let Some(element) = arguments.types().next() else {
            return Err("array iterator has no concrete element type");
        };
        if element.needs_drop(fx.tcx, typing_env()) {
            return Err("array iterator element move or drop behavior is observable");
        }
        return Ok(if inside_enumerate {
            IteratorKind::EnumerateArray
        } else {
            IteratorKind::Array
        });
    }
    if diagnostic_adt(fx, definition.did(), "Enumerate") {
        if inside_enumerate {
            return Err("nested or dynamic iterator adaptors are not eligible for scalarization");
        }
        let Some(inner) = arguments.types().next() else {
            return Err("Enumerate has no concrete child iterator type");
        };
        return iterator_kind(fx, inner, true);
    }

    Err("iterator type is not a concrete Range, slice, array, or simple Enumerate graph")
}

fn canonical_option_dispatch(mir: &Body<'_>, destination: Place<'_>, target: BasicBlock) -> bool {
    let Some(destination) = destination.as_local() else {
        return false;
    };
    let data = &mir.basic_blocks[target];
    let discriminant = data.statements.iter().find_map(|statement| {
        let StatementKind::Assign(assignment) = &statement.kind else {
            return None;
        };
        let local = assignment.0.as_local()?;
        match &assignment.1 {
            Rvalue::Discriminant(place) if place.local == destination => Some(local),
            _ => None,
        }
    });
    let Some(discriminant) = discriminant else {
        return false;
    };
    let TerminatorKind::SwitchInt { discr, targets } = &data.terminator().kind else {
        return false;
    };
    operand_local(discr) == Some(discriminant)
        && targets.iter().any(|(value, _)| value == 0)
        && targets.iter().any(|(value, _)| value == 1)
}

fn has_back_edge(mir: &Body<'_>, start: BasicBlock, goal: BasicBlock) -> bool {
    let mut pending = vec![start];
    let mut seen = vec![false; mir.basic_blocks.len()];
    while let Some(block) = pending.pop() {
        if block == goal {
            return true;
        }
        if std::mem::replace(&mut seen[block.as_usize()], true) {
            continue;
        }
        pending.extend(mir.basic_blocks[block].terminator().successors());
    }
    false
}

fn analyze_formatting<'tcx>(fx: &FnCx<'_, 'tcx>) {
    let definitions = local_definitions(fx.mir);
    let constructors = format_constructors(fx);
    for (block, data) in fx.mir.basic_blocks.iter_enumerated() {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            ..
        } = &data.terminator().kind
        else {
            continue;
        };
        let Some(instance) = resolve_call(fx, func) else {
            continue;
        };
        if !is_format_arguments_new(fx, instance) {
            continue;
        }
        let span = data.terminator().source_info.span;
        let Some(template) = args
            .first()
            .and_then(|argument| constant_bytes(fx, &argument.node, &definitions))
        else {
            fx.record_optimization_miss_at(
                FORMATTING_ANALYSIS,
                "format template is not a compiler-generated constant byte sequence",
                span,
            );
            continue;
        };
        let placeholders = match crate::late_ir::decode_default_format_template(&template) {
            Ok(pieces) => pieces.len().saturating_sub(1),
            Err(reason) => {
                fx.record_optimization_miss_at(FORMATTING_ANALYSIS, reason, span);
                continue;
            }
        };
        let Some(argument_locals) = args
            .get(1)
            .and_then(|argument| aggregate_elements(&argument.node, &definitions))
        else {
            fx.record_optimization_miss_at(
                FORMATTING_ANALYSIS,
                "format arguments do not come from one compiler-generated aggregate",
                span,
            );
            continue;
        };
        if placeholders != argument_locals.len() {
            fx.record_optimization_miss_at(
                FORMATTING_ANALYSIS,
                "format template and argument aggregate have different arities",
                span,
            );
            continue;
        }

        let first_fallback = argument_locals.iter().find_map(|local| {
            constructors
                .get(local)
                .copied()
                .unwrap_or(FormatArgument::Other)
                .fallback()
        });
        if let Some(reason) = first_fallback {
            fx.record_optimization_miss_at(FORMATTING_ANALYSIS, reason, span);
            continue;
        }

        let consumer = destination
            .as_local()
            .map_or(FormatConsumer::Other, |local| {
                format_consumer(fx, local, &definitions)
            });
        fx.record_optimization_selection(
            FORMATTING_ANALYSIS,
            format!(
                "native-default-display-builder-{}-bb{}",
                consumer.as_str(),
                block.as_usize()
            ),
            "literal pieces and exact default Display arguments use supported semantic types",
        );
        if let Some(reason) = consumer.fallback() {
            fx.record_optimization_miss_at(FORMATTING_ANALYSIS, reason, span);
        }
    }
}

/// What takes the built `Arguments` value, which decides what a lowering would have to produce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormatConsumer {
    /// The exact `fn(Arguments) -> String` that owns the formatted result.
    Format,
    /// One of the exact compiler-marked `Write::write_fmt` adapters.
    WriteFmt,
    Other,
}

impl FormatConsumer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::WriteFmt => "write-fmt",
            Self::Other => "other",
        }
    }

    /// Why a recognized site is still not lowerable today.
    ///
    /// Format and write_fmt sinks now have a compiler-owned native String constructor, so they
    /// no longer record a construction blocker. Unrecognized consumers still stay on baseline.
    fn fallback(self) -> Option<&'static str> {
        match self {
            Self::Format | Self::WriteFmt => None,
            Self::Other => Some("format arguments are consumed by an unrecognized formatter sink"),
        }
    }
}

/// Which call takes the `Arguments` value built into `arguments`.
///
/// A value that reaches its consumer through one borrow or move is followed; anything longer is
/// reported as unrecognized rather than guessed at.
fn format_consumer<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    arguments: Local,
    definitions: &[Option<&Rvalue<'tcx>>],
) -> FormatConsumer {
    let mut carriers = vec![arguments];
    for (index, definition) in definitions.iter().enumerate() {
        if definition.is_some_and(|rvalue| referenced_local(rvalue) == Some(arguments)) {
            carriers.push(Local::from_usize(index));
        }
    }

    for data in fx.mir.basic_blocks.iter() {
        let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
            continue;
        };
        let takes_arguments = args.iter().any(|argument| {
            operand_local(&argument.node).is_some_and(|local| carriers.contains(&local))
        });
        if !takes_arguments {
            continue;
        }
        let Some(instance) = resolve_call(fx, func) else {
            return FormatConsumer::Other;
        };
        if is_format_to_string(fx, instance) {
            return FormatConsumer::Format;
        }
        if is_write_fmt_adapter(fx, instance) {
            return FormatConsumer::WriteFmt;
        }
        return FormatConsumer::Other;
    }
    FormatConsumer::Other
}

/// Whether `instance` is the exact `fn(Arguments) -> String` that owns a formatted result.
fn is_format_to_string<'tcx>(fx: &FnCx<'_, 'tcx>, instance: Instance<'tcx>) -> bool {
    let instance_ty = instance.ty(fx.tcx, typing_env());
    if !matches!(instance_ty.kind(), ty::FnDef(..) | ty::FnPtr(..)) {
        return false;
    }
    let signature = fx
        .tcx
        .normalize_erasing_late_bound_regions(typing_env(), instance_ty.fn_sig(fx.tcx));
    let ty::Adt(result, _) = *signature.output().kind() else {
        return false;
    };
    if !fx
        .tcx
        .lang_items()
        .string()
        .is_some_and(|string| string == result.did())
    {
        return false;
    }
    matches!(
        signature.inputs(),
        [input] if matches!(
            input.kind(),
            ty::Adt(definition, _) if fx.tcx.is_lang_item(definition.did(), LangItem::FormatArguments)
        )
    )
}

/// Whether `instance` is one of the exact compiler-marked `Write::write_fmt` adapters.
fn is_write_fmt_adapter<'tcx>(fx: &FnCx<'_, 'tcx>, instance: Instance<'tcx>) -> bool {
    let write_fmt = Symbol::intern("rcgjs_fmt_write_fmt");
    let definition = instance.def_id();
    fx.tcx.is_diagnostic_item(write_fmt, definition)
        || fx
            .tcx
            .is_diagnostic_item(Symbol::intern("rcgjs_fmt_spec_write_fmt"), definition)
        || fx
            .tcx
            .trait_item_of(definition)
            .is_some_and(|trait_item| fx.tcx.is_diagnostic_item(write_fmt, trait_item))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormatArgument {
    Supported,
    Float,
    UnsupportedDisplay,
    Other,
}

impl FormatArgument {
    fn fallback(self) -> Option<&'static str> {
        match self {
            Self::Supported => None,
            Self::Float => Some("default Display for floats remains on Rust formatting"),
            Self::UnsupportedDisplay => Some(
                "default Display implementation is not one of str, String, char, bool, or integers",
            ),
            Self::Other => Some("format argument uses Debug or another non-Display formatter"),
        }
    }
}

fn format_constructors<'tcx>(fx: &FnCx<'_, 'tcx>) -> HashMap<Local, FormatArgument> {
    let mut constructors = HashMap::new();
    for data in fx.mir.basic_blocks.iter() {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            ..
        } = &data.terminator().kind
        else {
            continue;
        };
        let Some(destination) = destination.as_local() else {
            continue;
        };
        let Some(instance) = resolve_call(fx, func) else {
            continue;
        };
        if !returns_lang_adt(fx, instance, LangItem::FormatArgument) {
            continue;
        }
        let name = fx.tcx.item_name(instance.def_id());
        let argument = if name == Symbol::intern("new_display") {
            let value_ty = args
                .first()
                .map(|argument| fx.monomorphize(argument.node.ty(&fx.mir.local_decls, fx.tcx)))
                .map(peel_references);
            match value_ty.map(|ty| ty.kind()) {
                Some(ty::Str | ty::Char | ty::Bool | ty::Int(_) | ty::Uint(_)) => {
                    FormatArgument::Supported
                }
                Some(ty::Float(ty::FloatTy::F64)) => FormatArgument::Supported,
                Some(ty::Float(_)) => FormatArgument::Float,
                Some(ty::Adt(definition, _))
                    if fx
                        .tcx
                        .lang_items()
                        .string()
                        .is_some_and(|string| string == definition.did()) =>
                {
                    FormatArgument::Supported
                }
                Some(_) | None => FormatArgument::UnsupportedDisplay,
            }
        } else {
            FormatArgument::Other
        };
        constructors.insert(destination, argument);
    }
    constructors
}

fn is_format_arguments_new<'tcx>(fx: &FnCx<'_, 'tcx>, instance: Instance<'tcx>) -> bool {
    if !returns_lang_adt(fx, instance, LangItem::FormatArguments)
        || fx.tcx.item_name(instance.def_id()) != Symbol::intern("new")
    {
        return false;
    }
    fx.tcx
        .get_diagnostic_item(Symbol::intern("FmtArgumentsNew"))
        .is_some_and(|item| fx.tcx.parent(instance.def_id()) == item)
}

fn returns_lang_adt<'tcx>(fx: &FnCx<'_, 'tcx>, instance: Instance<'tcx>, item: LangItem) -> bool {
    let instance_ty = instance.ty(fx.tcx, typing_env());
    if !matches!(instance_ty.kind(), ty::FnDef(..) | ty::FnPtr(..)) {
        return false;
    }
    let signature = instance_ty.fn_sig(fx.tcx);
    let signature = fx
        .tcx
        .normalize_erasing_late_bound_regions(typing_env(), signature);
    let ty::Adt(definition, _) = *signature.output().kind() else {
        return false;
    };
    fx.tcx.is_lang_item(definition.did(), item)
}

fn local_definitions<'a, 'tcx>(mir: &'a Body<'tcx>) -> Vec<Option<&'a Rvalue<'tcx>>> {
    let mut definitions = vec![None; mir.local_decls.len()];
    let mut multiple = vec![false; mir.local_decls.len()];
    for data in mir.basic_blocks.iter() {
        for statement in &data.statements {
            let StatementKind::Assign(assignment) = &statement.kind else {
                continue;
            };
            let Some(local) = assignment.0.as_local() else {
                continue;
            };
            if definitions[local.as_usize()].is_some() {
                multiple[local.as_usize()] = true;
            } else {
                definitions[local.as_usize()] = Some(&assignment.1);
            }
        }
    }
    for local in mir.local_decls.indices() {
        if multiple[local.as_usize()] {
            definitions[local.as_usize()] = None;
        }
    }
    definitions
}

fn aggregate_elements<'tcx>(
    operand: &Operand<'tcx>,
    definitions: &[Option<&Rvalue<'tcx>>],
) -> Option<Vec<Local>> {
    let local = operand_local(operand)?;
    let local = referenced_local(definitions.get(local.as_usize())?.as_deref()?)?;
    let Rvalue::Aggregate(kind, elements) = definitions.get(local.as_usize())?.as_deref()? else {
        return None;
    };
    if !matches!(kind.as_ref(), mir::AggregateKind::Array(_)) {
        return None;
    }
    elements.iter().map(operand_local).collect()
}

fn referenced_local(rvalue: &Rvalue<'_>) -> Option<Local> {
    match rvalue {
        Rvalue::Ref(_, _, place) | Rvalue::Reborrow(_, _, place) => place.as_local(),
        Rvalue::Use(operand, _) => operand_local(operand),
        _ => None,
    }
}

fn constant_bytes<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    operand: &Operand<'tcx>,
    definitions: &[Option<&Rvalue<'tcx>>],
) -> Option<Vec<u8>> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            let local = place.as_local()?;
            let Rvalue::Use(value, _) = definitions.get(local.as_usize())?.as_deref()? else {
                return None;
            };
            constant_bytes(fx, value, definitions)
        }
        Operand::Constant(constant) => {
            let konst = fx.monomorphize(constant.const_);
            let ty = konst.ty();
            let length = match ty.peel_refs().kind() {
                ty::Array(element, length)
                    if matches!(element.kind(), ty::Uint(ty::UintTy::U8)) =>
                {
                    length.try_to_target_usize(fx.tcx)?
                }
                ty::Slice(element) if matches!(element.kind(), ty::Uint(ty::UintTy::U8)) => 0,
                _ => return None,
            };
            match konst.eval(fx.tcx, typing_env(), constant.span).ok()? {
                ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
                    let (provenance, offset) = pointer.prov_and_relative_offset();
                    allocation_bytes(fx, provenance.alloc_id(), offset, length)
                }
                ConstValue::Slice { alloc_id, meta } => {
                    allocation_bytes(fx, alloc_id, Size::ZERO, meta)
                }
                _ => None,
            }
        }
        Operand::RuntimeChecks(_) => None,
    }
}

fn allocation_bytes(
    fx: &FnCx<'_, '_>,
    alloc_id: rustc_middle::mir::interpret::AllocId,
    offset: Size,
    length: u64,
) -> Option<Vec<u8>> {
    let GlobalAlloc::Memory(allocation) = fx.tcx.global_alloc(alloc_id) else {
        return None;
    };
    let start = offset.bytes_usize();
    let end = start.checked_add(usize::try_from(length).ok()?)?;
    if end > allocation.inner().len() {
        return None;
    }
    Some(
        allocation
            .inner()
            .inspect_with_uninit_and_ptr_outside_interpreter(start..end)
            .to_vec(),
    )
}

#[cfg(test)]
fn default_template_placeholders(bytes: &[u8]) -> Result<usize, &'static str> {
    crate::late_ir::decode_default_format_template(bytes)
        .map(|pieces| pieces.len().saturating_sub(1))
}

fn resolve_call<'tcx>(fx: &FnCx<'_, 'tcx>, operand: &Operand<'tcx>) -> Option<Instance<'tcx>> {
    let ty = fx.monomorphize(operand.ty(&fx.mir.local_decls, fx.tcx));
    let ty::FnDef(def_id, arguments) = *ty.kind() else {
        return None;
    };
    let arguments = arguments.no_bound_vars()?;
    Instance::try_resolve(fx.tcx, typing_env(), def_id, arguments)
        .ok()
        .flatten()
}

fn implements_lang_item<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    instance: Instance<'tcx>,
    item: LangItem,
) -> bool {
    fx.tcx.is_lang_item(instance.def_id(), item)
        || fx
            .tcx
            .trait_item_of(instance.def_id())
            .is_some_and(|trait_item| fx.tcx.is_lang_item(trait_item, item))
}

fn diagnostic_adt(fx: &FnCx<'_, '_>, def_id: rustc_hir::def_id::DefId, name: &str) -> bool {
    fx.tcx
        .get_diagnostic_name(def_id)
        .is_some_and(|symbol| symbol == Symbol::intern(name))
}

fn operand_local(operand: &Operand<'_>) -> Option<Local> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => place.as_local(),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => None,
    }
}

fn peel_references(mut ty: Ty<'_>) -> Ty<'_> {
    while let ty::Ref(_, pointee, _) = *ty.kind() {
        ty = pointee;
    }
    ty
}

#[cfg(test)]
mod tests {
    use super::default_template_placeholders;

    #[test]
    fn accepts_literal_pieces_and_default_placeholders() {
        assert_eq!(
            default_template_placeholders(b"\x05item:\xc0\x01:\xc0\x01:\xc0\x01:\xc0\0"),
            Ok(4)
        );
    }

    #[test]
    fn accepts_long_literal_piece() {
        let mut template = vec![0x80, 0x80, 0x00];
        template.extend(std::iter::repeat_n(b'x', 128));
        template.extend([0xc0, 0]);
        assert_eq!(default_template_placeholders(&template), Ok(1));
    }

    #[test]
    fn rejects_non_default_formatting_before_later_errors() {
        assert_eq!(
            default_template_placeholders(&[0xc1, 0]),
            Err("format template uses flags, width, precision, or explicit argument selection")
        );
    }

    #[test]
    fn rejects_truncated_literal() {
        assert_eq!(
            default_template_placeholders(&[3, b'a', 0]),
            Err("format literal piece runs past the template")
        );
    }
}
