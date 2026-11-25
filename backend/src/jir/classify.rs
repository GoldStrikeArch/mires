//! Classification of rustc standard library operations and type metadata directly into JIR-S
//! schema enums (`StandardOperation` and `TypeMeta`).

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{sym, Symbol};

use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jir::types::{
    AllocatorIdentity, IteratorIdentity, NominalAbi, PinnedBoundary, RepresentationCandidate,
    TypeMeta,
};

use crate::value::typing_env;

pub(crate) fn standard_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    [
        ("vec_new", StandardOperation::VecNew),
        ("vec_with_capacity", StandardOperation::VecWithCapacity),
        ("vec_reserve", StandardOperation::VecReserve),
        ("vec_as_slice", StandardOperation::VecAsSlice),
        ("vec_as_mut_slice", StandardOperation::VecAsMutSlice),
        ("vec_pop", StandardOperation::VecPop),
        ("vec_is_empty", StandardOperation::VecIsEmpty),
        (
            "box_assume_init_into_vec_unsafe",
            StandardOperation::VecFromBoxArray,
        ),
        ("string_new", StandardOperation::StringNew),
        ("string_as_str", StandardOperation::StringAsStr),
        ("string_push_str", StandardOperation::StringPushStr),
    ]
    .into_iter()
    .find_map(|(name, operation)| {
        (tcx.get_diagnostic_item(Symbol::intern(name)) == Some(definition)).then_some(operation)
    })
    .or_else(|| format_to_string_operation(tcx, definition, arguments))
    .or_else(|| format_args_item_operation(tcx, definition, arguments))
    .or_else(|| {
        if tcx.get_diagnostic_item(Symbol::intern("vec_from_elem")) == Some(definition) {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            if vec_elem_ty(tcx, sig.output()).is_some_and(is_trivial_clone_primitive_ty) {
                return Some(StandardOperation::VecFromElem);
            }
            return None;
        }
        if tcx.get_diagnostic_item(Symbol::intern("mem_replace")) == Some(definition) {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            if sig.inputs().len() == 2
                && matches!(sig.inputs()[0].kind(), ty::Ref(_, p, rustc_hir::Mutability::Mut) if standard_owner(tcx, *p) == Some(RepresentationCandidate::Vec))
                && standard_owner(tcx, sig.inputs()[1]) == Some(RepresentationCandidate::Vec)
                && standard_owner(tcx, sig.output()) == Some(RepresentationCandidate::Vec)
            {
                return Some(StandardOperation::VecReplace);
            }
        }
        if tcx.get_diagnostic_item(Symbol::intern("mem_swap")) == Some(definition) {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            if sig.inputs().len() == 2
                && matches!(sig.inputs()[0].kind(), ty::Ref(_, p, rustc_hir::Mutability::Mut) if standard_owner(tcx, *p) == Some(RepresentationCandidate::Vec))
                && matches!(sig.inputs()[1].kind(), ty::Ref(_, p, rustc_hir::Mutability::Mut) if standard_owner(tcx, *p) == Some(RepresentationCandidate::Vec))
            {
                return Some(StandardOperation::VecMemSwap);
            }
        }
        None
    })
    .or_else(|| {
        let associated = tcx.opt_associated_item(definition)?;
        let signature = tcx
            .fn_sig(definition)
            .instantiate(tcx, arguments)
            .skip_binder();
        let receiver = signature.inputs().first().map(|ty| ty.peel_refs());
        let result = signature.output().peel_refs();
        let symbol = associated.name();
        let name = symbol.as_str();
        let trait_id = associated
            .trait_item_def_id()
            .map(|trait_item| tcx.parent(trait_item));
        let exact_extend = associated.trait_item_def_id().is_some_and(|trait_item| {
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_extend"), trait_item)
        });
        let exact_fmt_write_fmt =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_fmt_write_fmt"), definition);
        let exact_fmt_spec_write_fmt =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_fmt_spec_write_fmt"), definition);
        let exact_spec_to_string =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_spec_to_string"), definition);
        let exact_to_string = associated.trait_item_def_id().is_some_and(|trait_item| {
            tcx.is_diagnostic_item(Symbol::intern("to_string_method"), trait_item)
        });
        let fmt_write = trait_id
            .is_some_and(|trait_id| tcx.is_diagnostic_item(Symbol::intern("FmtWrite"), trait_id));
        let exact_from_str = trait_id.is_some_and(|trait_id| {
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_from_str"), trait_id)
        }) && name == "from_str";
        if exact_from_str
            && matches!(associated.container, ty::AssocContainer::TraitImpl(_))
            && matches!(
                signature.inputs(),
                [input] if matches!(
                    input.kind(),
                    ty::Ref(_, pointee, rustc_hir::Mutability::Not) if pointee.is_str()
                )
            )
        {
            let self_ty = tcx.normalize_erasing_regions(
                typing_env(),
                tcx.type_of(tcx.parent(definition))
                    .instantiate(tcx, arguments),
            );
            match self_ty.kind() {
                ty::Float(ty::FloatTy::F64) => return Some(StandardOperation::F64FromStr),
                ty::Float(ty::FloatTy::F32) => return Some(StandardOperation::F32FromStr),
                _ => {}
            }
        }
        let inherent = matches!(associated.container, ty::AssocContainer::InherentImpl);
        // An unrelated impl can accept a standard value or return String. Only the
        // owning standard impl gives these method names their semantic identity.
        let owner = inherent.then(|| {
            tcx.normalize_erasing_regions(
                typing_env(),
                tcx.type_of(tcx.parent(definition))
                    .instantiate(tcx, arguments),
            )
        });
        let standard_impl = owner.and_then(|owner| standard_owner(tcx, owner));
        let global_string_result = tcx.lang_items().string().is_some_and(|string| {
            matches!(signature.output().kind(), ty::Adt(definition, _) if definition.did() == string)
        });
        let exact_f64_spec_to_string = f64_spec_to_string_operation(
            exact_spec_to_string,
            signature.inputs().len() == 1
                && signature.inputs().first().is_some_and(|input| {
                matches!(
                    input.kind(),
                    ty::Ref(_, pointee, rustc_hir::Mutability::Not)
                        if matches!(pointee.kind(), ty::Float(ty::FloatTy::F64))
                )
            }),
            global_string_result,
        );
        if exact_f64_spec_to_string.is_some() {
            return exact_f64_spec_to_string;
        }
        let exact_primitive_to_string = primitive_to_string_operation(
            exact_to_string,
            shared_primitive_display(signature.inputs()),
            global_string_result,
        );
        if exact_primitive_to_string.is_some() {
            return exact_primitive_to_string;
        }
        if owner.is_some_and(|owner| matches!(owner.kind(), ty::Slice(_)))
            && receiver.is_some_and(|receiver| matches!(receiver.kind(), ty::Slice(_)))
        {
            if name == "swap" {
                return Some(StandardOperation::SliceSwap);
            }
            if name == "to_vec"
                && signature.inputs().len() == 1
                && vec_elem_ty(tcx, result).is_some_and(is_trivial_clone_primitive_ty)
            {
                return Some(StandardOperation::SliceToVec);
            }
        }
        let receiver = receiver.and_then(|ty| standard_owner(tcx, ty));
        if let Some(operation) =
            fmt_adapter_operation(receiver, exact_fmt_write_fmt, exact_fmt_spec_write_fmt)
        {
            return Some(operation);
        }
        if let Some(operation) = fmt_write_operation(receiver, fmt_write, name) {
            return Some(operation);
        }
        let lang = tcx.lang_items();
        let is_usize_range_index = |index: Ty<'tcx>| {
            let ty::Adt(range, range_args) = index.kind() else {
                return false;
            };
            let usize_or_none = range_args
                .types()
                .all(|ty| matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)));
            let shapes = [
                lang.range_struct(),
                lang.range_from_struct(),
                lang.range_to_struct(),
                lang.range_full_struct(),
            ];
            usize_or_none && shapes.contains(&Some(range.did()))
        };
        let is_clone_trait = trait_id
            .is_some_and(|id| tcx.is_diagnostic_item(Symbol::intern("Clone"), id));
        let is_default_trait = trait_id
            .is_some_and(|id| tcx.is_diagnostic_item(Symbol::intern("Default"), id));
        let is_into_iter_trait = trait_id
            .is_some_and(|id| tcx.is_diagnostic_item(Symbol::intern("IntoIterator"), id));
        match receiver {
            Some(RepresentationCandidate::Vec)
                if standard_impl == Some(RepresentationCandidate::Vec) => match name {
                "len" => Some(StandardOperation::VecLen),
                "capacity" => Some(StandardOperation::VecCapacity),
                "push" => Some(StandardOperation::VecPush),
                "push_mut" => Some(StandardOperation::VecPushMut),
                "insert" => Some(StandardOperation::VecInsert),
                "insert_mut" => Some(StandardOperation::VecInsertMut),
                "clear" => Some(StandardOperation::VecClear),
                "reserve_exact" => Some(StandardOperation::VecReserveExact),
                "shrink_to_fit" => Some(StandardOperation::VecShrinkToFit),
                "truncate" => Some(StandardOperation::VecTruncate),
                "resize" => owner
                    .and_then(|ty| vec_elem_ty(tcx, ty))
                    .is_some_and(is_trivial_clone_primitive_ty)
                    .then_some(StandardOperation::VecResize),
                "remove" => Some(StandardOperation::VecRemove),
                "swap_remove" => Some(StandardOperation::VecSwapRemove),
                "extend_from_slice" => owner
                    .and_then(|ty| vec_elem_ty(tcx, ty))
                    .is_some_and(is_trivial_clone_primitive_ty)
                    .then_some(StandardOperation::VecExtendFromSlice),
                "swap" => Some(StandardOperation::VecSwap),
                _ => None,
            },
            Some(RepresentationCandidate::Vec)
                if is_into_iter_trait
                    && name == "into_iter"
                    && signature.inputs().len() == 1 =>
            {
                match signature.inputs()[0].kind() {
                    ty::Ref(_, _, rustc_hir::Mutability::Not) => Some(StandardOperation::VecIter),
                    ty::Ref(_, _, rustc_hir::Mutability::Mut) => {
                        Some(StandardOperation::VecIterMut)
                    }
                    _ => None,
                }
            }
            Some(RepresentationCandidate::Vec)
                if lang.index_trait() == trait_id && name == "index" =>
            {
                match signature.inputs().get(1) {
                    Some(ty) if is_usize_range_index(*ty) => Some(StandardOperation::VecRangeIndex),
                    Some(ty) if matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)) => {
                        Some(StandardOperation::VecIndex)
                    }
                    _ => None,
                }
            }
            Some(RepresentationCandidate::Vec)
                if lang.index_mut_trait() == trait_id && name == "index_mut" =>
            {
                match signature.inputs().get(1) {
                    Some(ty) if is_usize_range_index(*ty) => {
                        Some(StandardOperation::VecRangeIndexMut)
                    }
                    Some(ty) if matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)) => {
                        Some(StandardOperation::VecIndexMut)
                    }
                    _ => None,
                }
            }
            Some(RepresentationCandidate::Vec)
                if is_clone_trait
                    && name == "clone"
                    && signature.inputs().len() == 1
                    && vec_elem_ty(tcx, result).is_some_and(is_trivial_clone_primitive_ty) =>
            {
                Some(StandardOperation::VecClone)
            }
            Some(RepresentationCandidate::Vec)
                if lang.deref_trait() == trait_id && name == "deref" =>
            {
                Some(StandardOperation::VecAsSlice)
            }
            Some(RepresentationCandidate::Vec)
                if lang.deref_mut_trait() == trait_id && name == "deref_mut" =>
            {
                Some(StandardOperation::VecAsMutSlice)
            }
            Some(RepresentationCandidate::Vec)
                if lang.drop_trait() == trait_id && name == "drop" =>
            {
                Some(StandardOperation::VecDrop)
            }
            Some(RepresentationCandidate::Vec)
                if exact_extend
                    && signature
                        .inputs()
                        .get(1)
                        .is_some_and(|arg| standard_owner(tcx, *arg) == Some(RepresentationCandidate::Vec)) =>
            {
                Some(StandardOperation::VecExtend)
            }
            Some(RepresentationCandidate::String)
                if standard_impl == Some(RepresentationCandidate::String) => match name {
                "len" => Some(StandardOperation::StringLen),
                "capacity" => Some(StandardOperation::StringCapacity),
                "push" => Some(StandardOperation::StringPush),
                "push_str" => Some(StandardOperation::StringPushStr),
                "clear" => Some(StandardOperation::StringClear),
                "as_str" => Some(StandardOperation::StringAsStr),
                _ => None,
            },
            Some(RepresentationCandidate::String)
                if lang.deref_trait() == trait_id && name == "deref" =>
            {
                Some(StandardOperation::StringAsStr)
            }
            Some(RepresentationCandidate::String)
                if lang.drop_trait() == trait_id && name == "drop" =>
            {
                Some(StandardOperation::StringDrop)
            }
            Some(RepresentationCandidate::BTreeMap)
                if standard_impl == Some(RepresentationCandidate::BTreeMap) =>
            {
                let key_ty = owner.and_then(|ty| btree_map_key_ty(tcx, ty));
                if !key_ty.is_some_and(|k| is_native_map_key_ty(tcx, k)) {
                    None
                } else {
                    let key_ty = key_ty.unwrap();
                    let valid_lookup = || {
                        signature.inputs().len() == 2
                            && is_native_map_lookup_key_ty(tcx, key_ty, signature.inputs()[1])
                    };
                    match name {
                        "len" if signature.inputs().len() == 1 => {
                            Some(StandardOperation::BTreeMapLen)
                        }
                        "is_empty" if signature.inputs().len() == 1 => {
                            Some(StandardOperation::BTreeMapIsEmpty)
                        }
                        "clear" if signature.inputs().len() == 1 => {
                            Some(StandardOperation::BTreeMapClear)
                        }
                        "contains_key" if valid_lookup() => {
                            Some(StandardOperation::BTreeMapContainsKey)
                        }
                        "get" if valid_lookup() => Some(StandardOperation::BTreeMapGet),
                        "get_mut" if valid_lookup() => Some(StandardOperation::BTreeMapGetMut),
                        "insert" if signature.inputs().len() == 3 => {
                            Some(StandardOperation::BTreeMapInsert)
                        }
                        "remove" if valid_lookup() => Some(StandardOperation::BTreeMapRemove),
                        _ => None,
                    }
                }
            }
            Some(RepresentationCandidate::BTreeMap)
                if lang.drop_trait() == trait_id && name == "drop" =>
            {
                signature
                    .inputs()
                    .first()
                    .map(|ty| ty.peel_refs())
                    .and_then(|ty| btree_map_key_ty(tcx, ty))
                    .is_some_and(|k| is_native_map_key_ty(tcx, k))
                    .then_some(StandardOperation::BTreeMapDrop)
            }
            _ => match standard_owner(tcx, result) {
                Some(RepresentationCandidate::String)
                    if standard_impl == Some(RepresentationCandidate::String)
                        && name == "with_capacity" => {
                    Some(StandardOperation::StringWithCapacity)
                }
                Some(RepresentationCandidate::Vec)
                    if is_default_trait
                        && name == "default"
                        && signature.inputs().is_empty() =>
                {
                    Some(StandardOperation::VecNew)
                }
                Some(RepresentationCandidate::BTreeMap)
                    if signature.inputs().is_empty()
                        && ((standard_impl == Some(RepresentationCandidate::BTreeMap)
                            && name == "new")
                            || (is_default_trait && name == "default"))
                        && btree_map_key_ty(tcx, result)
                            .is_some_and(|k| is_native_map_key_ty(tcx, k)) =>
                {
                    Some(StandardOperation::BTreeMapNew)
                }
                _ => None,
            },
        }
    })
}

fn fmt_write_operation(
    receiver: Option<RepresentationCandidate>,
    exact_trait: bool,
    name: &str,
) -> Option<StandardOperation> {
    if receiver != Some(RepresentationCandidate::String) || !exact_trait {
        return None;
    }
    match name {
        "write_str" => Some(StandardOperation::StringFmtWriteStr),
        "write_char" => Some(StandardOperation::StringFmtWriteChar),
        _ => None,
    }
}

fn fmt_adapter_operation(
    receiver: Option<RepresentationCandidate>,
    exact_write_fmt: bool,
    exact_spec_write_fmt: bool,
) -> Option<StandardOperation> {
    if receiver != Some(RepresentationCandidate::String) {
        return None;
    }
    match (exact_write_fmt, exact_spec_write_fmt) {
        (true, false) => Some(StandardOperation::StringFmtWriteFmt),
        (false, true) => Some(StandardOperation::StringFmtSpecWriteFmt),
        _ => None,
    }
}

fn f64_spec_to_string_operation(
    exact_marker: bool,
    shared_f64_input: bool,
    global_string_result: bool,
) -> Option<StandardOperation> {
    (exact_marker && shared_f64_input && global_string_result)
        .then_some(StandardOperation::F64SpecToString)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimitiveDisplay {
    Integer,
    Bool,
    Char,
}

fn shared_primitive_display<'tcx>(inputs: &[Ty<'tcx>]) -> Option<PrimitiveDisplay> {
    let [input] = inputs else {
        return None;
    };
    let ty::Ref(_, pointee, rustc_hir::Mutability::Not) = input.kind() else {
        return None;
    };
    match pointee.kind() {
        ty::Int(_) | ty::Uint(_) => Some(PrimitiveDisplay::Integer),
        ty::Bool => Some(PrimitiveDisplay::Bool),
        ty::Char => Some(PrimitiveDisplay::Char),
        _ => None,
    }
}

fn primitive_to_string_operation(
    exact_marker: bool,
    receiver: Option<PrimitiveDisplay>,
    global_string_result: bool,
) -> Option<StandardOperation> {
    if !exact_marker || !global_string_result {
        return None;
    }
    Some(match receiver? {
        PrimitiveDisplay::Integer => StandardOperation::IntToString,
        PrimitiveDisplay::Bool => StandardOperation::BoolToString,
        PrimitiveDisplay::Char => StandardOperation::CharToString,
    })
}

fn format_to_string_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    if tcx.opt_item_name(definition) != Some(Symbol::intern("format"))
        || tcx.crate_name(definition.krate).as_str() != "alloc"
    {
        return None;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let [input] = signature.inputs() else {
        return None;
    };
    let ty::Adt(result, _) = *signature.output().kind() else {
        return None;
    };
    let ty::Adt(arguments_ty, _) = *input.kind() else {
        return None;
    };
    (tcx.lang_items().string() == Some(result.did())
        && tcx.is_lang_item(arguments_ty.did(), rustc_hir::LangItem::FormatArguments))
    .then_some(StandardOperation::FormatArgsToString)
}

fn format_args_item_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    format_args_associated_operation(
        tcx,
        definition,
        arguments,
        tcx.opt_item_name(definition)?.as_str(),
    )
}

fn format_args_associated_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    name: &str,
) -> Option<StandardOperation> {
    if name == "new"
        && tcx
            .get_diagnostic_item(Symbol::intern("FmtArgumentsNew"))
            .is_some_and(|item| tcx.parent(definition) == item)
    {
        return Some(StandardOperation::FormatArgsNew);
    }
    if name != "new_display" && name != "new_lower_hex" && name != "new_upper_hex" {
        return None;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let is_format_argument = matches!(
        signature.output().kind(),
        ty::Adt(result, _)
            if tcx.is_lang_item(result.did(), rustc_hir::LangItem::FormatArgument)
    );
    if !is_format_argument {
        return None;
    }
    let [input] = signature.inputs() else {
        return None;
    };
    match name {
        "new_display" => {
            proven_display_ty(tcx, *input).then_some(StandardOperation::FormatArgumentNewDisplay)
        }
        "new_lower_hex" => {
            proven_int_ty(*input).then_some(StandardOperation::FormatArgumentNewLowerHex)
        }
        "new_upper_hex" => {
            proven_int_ty(*input).then_some(StandardOperation::FormatArgumentNewUpperHex)
        }
        _ => None,
    }
}

fn proven_display_ty<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> bool {
    while let ty::Ref(_, pointee, _) = *ty.kind() {
        ty = pointee;
    }
    match ty.kind() {
        ty::Str | ty::Char | ty::Bool | ty::Int(_) | ty::Uint(_) => true,
        ty::Float(ty::FloatTy::F32 | ty::FloatTy::F64) => true,
        ty::Adt(definition, _) => tcx.lang_items().string() == Some(definition.did()),
        _ => false,
    }
}

fn proven_int_ty<'tcx>(mut ty: Ty<'tcx>) -> bool {
    while let ty::Ref(_, pointee, _) = *ty.kind() {
        ty = pointee;
    }
    matches!(ty.kind(), ty::Int(_) | ty::Uint(_))
}

fn is_trivial_clone_primitive_ty(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_)
    )
}

fn vec_elem_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(definition, args) = ty.kind() else {
        return None;
    };
    if !tcx.is_diagnostic_item(sym::Vec, definition.did()) {
        return None;
    }
    args.types().next()
}

fn btree_map_key_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(definition, args) = ty.kind() else {
        return None;
    };
    if !tcx.is_diagnostic_item(Symbol::intern("BTreeMap"), definition.did()) {
        return None;
    }
    args.types().next()
}

fn is_shared_str_ref(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        ty::Ref(_, pointee, rustc_hir::Mutability::Not) if pointee.is_str()
    )
}

fn is_native_map_key_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    match ty.kind() {
        ty::Bool | ty::Char => true,
        ty::Int(int) => matches!(
            int,
            ty::IntTy::I8 | ty::IntTy::I16 | ty::IntTy::I32 | ty::IntTy::Isize | ty::IntTy::I64
        ),
        ty::Uint(uint) => matches!(
            uint,
            ty::UintTy::U8
                | ty::UintTy::U16
                | ty::UintTy::U32
                | ty::UintTy::Usize
                | ty::UintTy::U64
        ),
        ty::Ref(_, pointee, rustc_hir::Mutability::Not) if pointee.is_str() => true,
        ty::Adt(definition, _) => tcx.lang_items().string() == Some(definition.did()),
        _ => false,
    }
}

fn is_native_map_lookup_key_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    key_ty: Ty<'tcx>,
    arg_ty: Ty<'tcx>,
) -> bool {
    let ty::Ref(_, q_ty, rustc_hir::Mutability::Not) = *arg_ty.kind() else {
        return false;
    };
    if q_ty == key_ty || (is_shared_str_ref(key_ty) && is_shared_str_ref(q_ty)) {
        return true;
    }
    if q_ty.is_str() {
        return is_shared_str_ref(key_ty)
            || matches!(
                key_ty.kind(),
                ty::Adt(def, _) if tcx.lang_items().string() == Some(def.did())
            );
    }
    false
}

fn standard_owner(tcx: TyCtxt<'_>, ty: Ty<'_>) -> Option<RepresentationCandidate> {
    let ty::Adt(definition, _) = ty.kind() else {
        return None;
    };
    if tcx.is_diagnostic_item(sym::Vec, definition.did()) {
        Some(RepresentationCandidate::Vec)
    } else if tcx.lang_items().string() == Some(definition.did()) {
        Some(RepresentationCandidate::String)
    } else if tcx.is_diagnostic_item(Symbol::intern("BTreeMap"), definition.did()) {
        Some(RepresentationCandidate::BTreeMap)
    } else {
        None
    }
}

pub(crate) fn type_meta<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> TypeMeta {
    let (candidate, abi, boundary, allocator) = match ty.kind() {
        ty::Adt(definition, arguments) => {
            let candidate = if definition.is_box() {
                RepresentationCandidate::Box
            } else if tcx.lang_items().string() == Some(definition.did()) {
                RepresentationCandidate::String
            } else if tcx.is_diagnostic_item(sym::Vec, definition.did()) {
                RepresentationCandidate::Vec
            } else if tcx.is_diagnostic_item(Symbol::intern("BTreeMap"), definition.did()) {
                RepresentationCandidate::BTreeMap
            } else {
                RepresentationCandidate::None
            };
            let abi = if definition.repr().transparent() {
                NominalAbi::Transparent
            } else if definition.repr().c() {
                NominalAbi::C
            } else {
                NominalAbi::Rust
            };
            let crate_name = tcx.crate_name(definition.did().krate);
            let boundary = match crate_name.as_str() {
                "mires_js" => PinnedBoundary::Host,
                _ => PinnedBoundary::None,
            };
            let alloc_arg = match candidate {
                RepresentationCandidate::BTreeMap => 2,
                _ => 1,
            };
            let allocator = allocator_identity(
                candidate,
                allocator_matches_global(tcx, arguments, alloc_arg),
            );
            (candidate, abi, boundary, allocator)
        }
        _ => (
            RepresentationCandidate::None,
            NominalAbi::Rust,
            PinnedBoundary::None,
            AllocatorIdentity::NotApplicable,
        ),
    };
    TypeMeta {
        candidate,
        abi,
        boundary,
        allocator,
        iterator: iterator_identity(tcx, ty),
    }
}

fn allocator_matches_global<'tcx>(
    tcx: TyCtxt<'tcx>,
    arguments: ty::GenericArgsRef<'tcx>,
    arg_index: usize,
) -> Option<bool> {
    let allocator = arguments.types().nth(arg_index)?;
    let global = tcx.lang_items().global_alloc_ty()?;
    Some(matches!(
        allocator.kind(),
        ty::Adt(definition, arguments)
            if definition.did() == global && arguments.is_empty()
    ))
}

fn allocator_identity(
    candidate: RepresentationCandidate,
    matches_global: Option<bool>,
) -> AllocatorIdentity {
    match candidate {
        RepresentationCandidate::None => AllocatorIdentity::NotApplicable,
        RepresentationCandidate::String => AllocatorIdentity::Global,
        RepresentationCandidate::Box
        | RepresentationCandidate::Vec
        | RepresentationCandidate::BTreeMap => match matches_global {
            Some(true) => AllocatorIdentity::Global,
            Some(false) => AllocatorIdentity::Custom,
            None => AllocatorIdentity::Unknown,
        },
    }
}

fn iterator_identity(tcx: TyCtxt<'_>, ty: Ty<'_>) -> IteratorIdentity {
    let ty::Adt(definition, _) = ty.kind() else {
        return IteratorIdentity::None;
    };
    if let Some(name) = tcx.get_diagnostic_name(definition.did()) {
        return match name.as_str() {
            "SliceIter" => IteratorIdentity::Slice,
            "Enumerate" => IteratorIdentity::Enumerate,
            "Take" => IteratorIdentity::Take,
            "Skip" => IteratorIdentity::Skip,
            _ => IteratorIdentity::None,
        };
    }
    // `core::slice::IterMut` has no `#[rustc_diagnostic_item]` on this toolchain,
    // so identify it unforgeably as the `IterMut` sibling inside `SliceIter`'s `core` module.
    if let Some(slice_iter) = tcx.get_diagnostic_item(rustc_span::Symbol::intern("SliceIter")) {
        if tcx.parent(definition.did()) == tcx.parent(slice_iter)
            && tcx.item_name(definition.did()).as_str() == "IterMut"
        {
            return IteratorIdentity::SliceMut;
        }
    }
    IteratorIdentity::None
}
