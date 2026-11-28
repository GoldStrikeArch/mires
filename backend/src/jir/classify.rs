//! Classification of rustc standard library operations and type metadata directly into JIR-S
//! schema enums (`StandardOperation` and `TypeMeta`).

use rustc_hir::def_id::DefId;
use rustc_hir::Mutability;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{sym, Symbol};

use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jir::types::{
    AllocatorIdentity, IteratorIdentity, NominalAbi, PinnedBoundary, RepresentationCandidate,
    TypeMeta,
};

use crate::value::typing_env;

/// Classifies a standard library function or method call/body into its JIR-S [`StandardOperation`].
pub(crate) fn standard_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    let diag = tcx.get_diagnostic_name(definition);
    if let Some(op) = diag.and_then(|name| diagnostic_item_op(name.as_str())) {
        return Some(op);
    }
    if let Some(op) = free_fn_op(tcx, definition, arguments, diag) {
        return Some(op);
    }
    if let Some(op) = associated_item_op(tcx, definition, arguments, diag) {
        return Some(op);
    }
    diag.and_then(|name| jir_diagnostic_item_op(name.as_str()))
}

/// Core diagnostic-item operations whose identity is settled unconditionally by their diagnostic
/// name.
fn diagnostic_item_op(name: &str) -> Option<StandardOperation> {
    Some(match name {
        "vec_new" => StandardOperation::VecNew,
        "vec_with_capacity" => StandardOperation::VecWithCapacity,
        "vec_reserve" => StandardOperation::VecReserve,
        "vec_as_slice" => StandardOperation::VecAsSlice,
        "vec_as_mut_slice" => StandardOperation::VecAsMutSlice,
        "vec_pop" => StandardOperation::VecPop,
        "vec_is_empty" => StandardOperation::VecIsEmpty,
        "box_assume_init_into_vec_unsafe" => StandardOperation::VecFromBoxArray,
        "string_new" => StandardOperation::StringNew,
        "string_as_str" => StandardOperation::StringAsStr,
        "string_push_str" => StandardOperation::StringPushStr,
        _ => return None,
    })
}

/// JIR-only standard-library operations identified strictly by rustc diagnostic items
/// (`patches/0016-host-sort-and-case-mapping.patch` and native `String`/`Vec` sysroot hooks).
fn jir_diagnostic_item_op(name: &str) -> Option<StandardOperation> {
    Some(match name {
        "rcgjs_slice_stable_sort" | "rcgjs_slice_unstable_sort" => StandardOperation::SliceSort,
        "rcgjs_slice_sort_cmp" => StandardOperation::SliceSortCmp,
        "rcgjs_str_to_lowercase" => StandardOperation::StrToLowercase,
        "rcgjs_str_to_uppercase" => StandardOperation::StrToUppercase,
        "rcgjs_typed_alloc" => StandardOperation::TypedAlloc,
        "rcgjs_typed_realloc" => StandardOperation::TypedRealloc,
        "rcgjs_typed_dealloc" => StandardOperation::TypedDealloc,
        "rcgjs_box_new" => StandardOperation::BoxNew,
        "rcgjs_typed_dealloc_raw" => StandardOperation::TypedDeallocRaw,
        "rcgjs_rc_alloc_slice" => StandardOperation::RcAllocSlice,
        "rcgjs_arc_alloc_slice" => StandardOperation::ArcAllocSlice,
        "rcgjs_str_new" => StandardOperation::NativeStrNew,
        "rcgjs_str_with_capacity" => StandardOperation::NativeStrWithCapacity,
        "rcgjs_str_from_str" => StandardOperation::NativeStrFromStr,
        "rcgjs_str_as_str" => StandardOperation::NativeStrAsStr,
        "rcgjs_str_len" => StandardOperation::NativeStrLen,
        "rcgjs_str_capacity" => StandardOperation::NativeStrCapacity,
        "rcgjs_str_push_str" => StandardOperation::NativeStrPushStr,
        "rcgjs_str_push" => StandardOperation::NativeStrPush,
        "rcgjs_str_clear" => StandardOperation::NativeStrClear,
        "rcgjs_str_truncate_unchecked" => StandardOperation::NativeStrTruncate,
        "rcgjs_str_reserve" => StandardOperation::NativeStrReserve,
        "rcgjs_str_shrink_to" => StandardOperation::NativeStrShrinkTo,
        "rcgjs_str_from_vec_unchecked" => StandardOperation::NativeStrFromVec,
        "rcgjs_str_into_bytes" => StandardOperation::NativeStrIntoBytes,
        "rcgjs_str_replace_slice" => StandardOperation::NativeStrReplaceSlice,
        "rcgjs_str_extend_within" => StandardOperation::NativeStrExtendWithin,
        "rcgjs_str_clone_from" => StandardOperation::NativeStrCloneFrom,
        "rcgjs_str_to_ascii_lowercase" => StandardOperation::NativeStrToAsciiLowercase,
        "rcgjs_str_to_ascii_uppercase" => StandardOperation::NativeStrToAsciiUppercase,
        "rcgjs_str_repeat" => StandardOperation::NativeStrRepeat,
        "rcgjs_str_eq" => StandardOperation::BstrEq,
        "rcgjs_str_cmp" => StandardOperation::BstrCmp,
        "rcgjs_str_starts_with" => StandardOperation::BstrStartsWith,
        "rcgjs_str_ends_with" => StandardOperation::BstrEndsWith,
        "rcgjs_str_contains" => StandardOperation::BstrContains,
        "rcgjs_str_find_char" => StandardOperation::BstrFindChar,
        "rcgjs_str_rfind_char" => StandardOperation::BstrRfindChar,
        "rcgjs_str_find_str" => StandardOperation::BstrFindStr,
        "rcgjs_str_rfind_str" => StandardOperation::BstrRfindStr,
        "rcgjs_vec_new" => StandardOperation::NativeVecNew,
        "rcgjs_vec_with_capacity" => StandardOperation::NativeVecWithCapacity,
        "rcgjs_vec_from_elem_zeroed" => StandardOperation::NativeVecFromElemZeroed,
        "rcgjs_vec_from_array" => StandardOperation::NativeVecFromArray,
        "rcgjs_vec_from_raw_parts" => StandardOperation::NativeVecFromRawParts,
        "rcgjs_vec_capacity" => StandardOperation::NativeVecCapacity,
        "rcgjs_vec_as_ptr" => StandardOperation::NativeVecAsPtr,
        "rcgjs_vec_as_mut_ptr" => StandardOperation::NativeVecAsMutPtr,
        "rcgjs_vec_as_slice" => StandardOperation::NativeVecAsSlice,
        "rcgjs_vec_as_mut_slice" => StandardOperation::NativeVecAsMutSlice,
        "rcgjs_vec_reserve" => StandardOperation::NativeVecReserve,
        "rcgjs_vec_reserve_exact" => StandardOperation::NativeVecReserveExact,
        "rcgjs_vec_try_reserve" => StandardOperation::NativeVecTryReserve,
        "rcgjs_vec_try_reserve_exact" => StandardOperation::NativeVecTryReserveExact,
        "rcgjs_vec_shrink_to" => StandardOperation::NativeVecShrinkTo,
        "rcgjs_vec_try_shrink_to" => StandardOperation::NativeVecTryShrinkTo,
        "rcgjs_vec_grow_one" => StandardOperation::NativeVecGrowOne,
        "rcgjs_vec_push" => StandardOperation::NativeVecPush,
        "rcgjs_vec_push_mut" => StandardOperation::NativeVecPushMut,
        "rcgjs_vec_pop" => StandardOperation::NativeVecPop,
        "rcgjs_vec_into_boxed_slice" => StandardOperation::NativeVecIntoBoxedSlice,
        _ => return None,
    })
}

/// Free functions and format-args constructors requiring signature inspection.
fn free_fn_op<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    diag: Option<Symbol>,
) -> Option<StandardOperation> {
    if let Some(op) = format_to_string_operation(tcx, definition, arguments)
        .or_else(|| format_args_item_operation(tcx, definition, arguments))
    {
        return Some(op);
    }
    let diag = diag?;
    match diag.as_str() {
        "vec_from_elem" => {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            vec_elem_ty(tcx, sig.output())
                .is_some_and(is_trivial_clone_primitive_ty)
                .then_some(StandardOperation::VecFromElem)
        }
        "mem_replace" => {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            let [dest, src] = sig.inputs() else {
                return None;
            };
            let is_vec_mut_ref = matches!(
                dest.kind(),
                ty::Ref(_, p, Mutability::Mut)
                    if standard_owner(tcx, *p) == Some(RepresentationCandidate::Vec)
            );
            (is_vec_mut_ref
                && standard_owner(tcx, *src) == Some(RepresentationCandidate::Vec)
                && standard_owner(tcx, sig.output()) == Some(RepresentationCandidate::Vec))
            .then_some(StandardOperation::VecReplace)
        }
        "mem_swap" => {
            let sig = tcx
                .fn_sig(definition)
                .instantiate(tcx, arguments)
                .skip_binder();
            let [left, right] = sig.inputs() else {
                return None;
            };
            let is_vec_mut_ref = |ty: Ty<'tcx>| {
                matches!(
                    ty.kind(),
                    ty::Ref(_, p, Mutability::Mut)
                        if standard_owner(tcx, *p) == Some(RepresentationCandidate::Vec)
                )
            };
            (is_vec_mut_ref(*left) && is_vec_mut_ref(*right))
                .then_some(StandardOperation::VecMemSwap)
        }
        _ => None,
    }
}

/// Shared context precomputed once for an associated method/item candidate.
struct AssocCall<'tcx> {
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    diag: Option<Symbol>,
    associated: ty::AssocItem,
    signature: ty::FnSig<'tcx>,
    trait_id: Option<DefId>,
    trait_diag: Option<Symbol>,
    trait_item_diag: Option<Symbol>,
    owner: Option<Ty<'tcx>>,
    standard_impl: Option<RepresentationCandidate>,
    receiver_ty: Option<Ty<'tcx>>,
    receiver: Option<RepresentationCandidate>,
    result_ty: Ty<'tcx>,
    result_owner: Option<RepresentationCandidate>,
}

impl<'tcx> AssocCall<'tcx> {
    fn has_diag(&self, expected: &str) -> bool {
        self.diag.is_some_and(|sym| sym.as_str() == expected)
    }

    fn has_trait_diag(&self, expected: &str) -> bool {
        self.trait_diag.is_some_and(|sym| sym.as_str() == expected)
    }

    fn has_trait_item_diag(&self, expected: &str) -> bool {
        self.trait_item_diag
            .is_some_and(|sym| sym.as_str() == expected)
    }
}

fn associated_item_op<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    diag: Option<Symbol>,
) -> Option<StandardOperation> {
    let associated = tcx.opt_associated_item(definition)?;
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let trait_item = associated.trait_item_def_id();
    let trait_id = trait_item.map(|item| tcx.parent(item));
    let trait_item_diag = trait_item.and_then(|item| tcx.get_diagnostic_name(item));
    let trait_diag = trait_id.and_then(|id| tcx.get_diagnostic_name(id));
    let inherent = matches!(associated.container, ty::AssocContainer::InherentImpl);
    // An unrelated impl can accept a standard value or return String. Only the owning standard
    // impl gives these method names their semantic identity.
    let owner = inherent.then(|| {
        tcx.normalize_erasing_regions(
            typing_env(),
            tcx.type_of(tcx.parent(definition))
                .instantiate(tcx, arguments),
        )
    });
    let standard_impl = owner.and_then(|owner| standard_owner(tcx, owner));
    let receiver_ty = signature.inputs().first().map(|ty| ty.peel_refs());
    let receiver = receiver_ty.and_then(|ty| standard_owner(tcx, ty));
    let result_ty = signature.output().peel_refs();
    let result_owner = standard_owner(tcx, result_ty);
    let name_sym = associated.name();
    let name = name_sym.as_str();
    let call = AssocCall {
        tcx,
        definition,
        arguments,
        diag,
        associated,
        signature,
        trait_id,
        trait_diag,
        trait_item_diag,
        owner,
        standard_impl,
        receiver_ty,
        receiver,
        result_ty,
        result_owner,
    };

    fmt_op(&call, name)
        .or_else(|| slice_op(&call, name))
        .or_else(|| vec_op(&call, name))
        .or_else(|| string_op(&call, name))
        .or_else(|| btree_map_op(&call, name))
}

/// Formatting, `Display`/`ToString`, and float `FromStr` operations.
fn fmt_op<'tcx>(call: &AssocCall<'tcx>, name: &str) -> Option<StandardOperation> {
    let tcx = call.tcx;
    if call.has_trait_diag("rcgjs_from_str")
        && name == "from_str"
        && matches!(call.associated.container, ty::AssocContainer::TraitImpl(_))
        && matches!(call.signature.inputs(), [input] if is_shared_str_ref(*input))
    {
        let self_ty = tcx.normalize_erasing_regions(
            typing_env(),
            tcx.type_of(tcx.parent(call.definition))
                .instantiate(tcx, call.arguments),
        );
        match self_ty.kind() {
            ty::Float(ty::FloatTy::F64) => return Some(StandardOperation::F64FromStr),
            ty::Float(ty::FloatTy::F32) => return Some(StandardOperation::F32FromStr),
            _ => {}
        }
    }

    let global_string_result = tcx.lang_items().string().is_some_and(
        |string| matches!(call.signature.output().kind(), ty::Adt(def, _) if def.did() == string),
    );
    if global_string_result {
        if call.has_diag("rcgjs_spec_to_string")
            && matches!(
                call.signature.inputs(),
                [input]
                    if matches!(
                        input.kind(),
                        ty::Ref(_, pointee, Mutability::Not)
                            if matches!(pointee.kind(), ty::Float(ty::FloatTy::F64))
                    )
            )
        {
            return Some(StandardOperation::F64SpecToString);
        }
        if call.has_trait_item_diag("to_string_method") {
            if let Some(display) = shared_primitive_display(call.signature.inputs()) {
                return Some(match display {
                    PrimitiveDisplay::Integer => StandardOperation::IntToString,
                    PrimitiveDisplay::Bool => StandardOperation::BoolToString,
                    PrimitiveDisplay::Char => StandardOperation::CharToString,
                });
            }
        }
    }

    if call.receiver == Some(RepresentationCandidate::String) {
        if call.has_diag("rcgjs_fmt_write_fmt") {
            return Some(StandardOperation::StringFmtWriteFmt);
        }
        if call.has_diag("rcgjs_fmt_spec_write_fmt") {
            return Some(StandardOperation::StringFmtSpecWriteFmt);
        }
        if call.has_trait_diag("FmtWrite") {
            return match name {
                "write_str" => Some(StandardOperation::StringFmtWriteStr),
                "write_char" => Some(StandardOperation::StringFmtWriteChar),
                _ => None,
            };
        }
    }

    None
}

/// Inherent `[T]` slice operations.
fn slice_op<'tcx>(call: &AssocCall<'tcx>, name: &str) -> Option<StandardOperation> {
    if !call
        .owner
        .is_some_and(|owner| matches!(owner.kind(), ty::Slice(_)))
        || !call
            .receiver_ty
            .is_some_and(|receiver| matches!(receiver.kind(), ty::Slice(_)))
    {
        return None;
    }
    match name {
        "swap" => Some(StandardOperation::SliceSwap),
        "to_vec"
            if call.signature.inputs().len() == 1
                && vec_elem_ty(call.tcx, call.result_ty)
                    .is_some_and(is_trivial_clone_primitive_ty) =>
        {
            Some(StandardOperation::SliceToVec)
        }
        _ => None,
    }
}

/// `Vec<T>` inherent methods, trait impls, and `Default::default`.
fn vec_op<'tcx>(call: &AssocCall<'tcx>, name: &str) -> Option<StandardOperation> {
    let tcx = call.tcx;
    let lang = tcx.lang_items();
    if call.receiver == Some(RepresentationCandidate::Vec) {
        if call.standard_impl == Some(RepresentationCandidate::Vec) {
            return match name {
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
                "resize" => call
                    .owner
                    .and_then(|ty| vec_elem_ty(tcx, ty))
                    .is_some_and(is_trivial_clone_primitive_ty)
                    .then_some(StandardOperation::VecResize),
                "remove" => Some(StandardOperation::VecRemove),
                "swap_remove" => Some(StandardOperation::VecSwapRemove),
                "extend_from_slice" => call
                    .owner
                    .and_then(|ty| vec_elem_ty(tcx, ty))
                    .is_some_and(is_trivial_clone_primitive_ty)
                    .then_some(StandardOperation::VecExtendFromSlice),
                "swap" => Some(StandardOperation::VecSwap),
                _ => None,
            };
        }
        if call.has_trait_diag("IntoIterator")
            && name == "into_iter"
            && call.signature.inputs().len() == 1
        {
            return match call.signature.inputs()[0].kind() {
                ty::Ref(_, _, Mutability::Not) => Some(StandardOperation::VecIter),
                ty::Ref(_, _, Mutability::Mut) => Some(StandardOperation::VecIterMut),
                _ => None,
            };
        }
        if lang.index_trait() == call.trait_id && name == "index" {
            return match call.signature.inputs().get(1) {
                Some(&ty) if is_usize_range_index(tcx, ty) => {
                    Some(StandardOperation::VecRangeIndex)
                }
                Some(&ty) if matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)) => {
                    Some(StandardOperation::VecIndex)
                }
                _ => None,
            };
        }
        if lang.index_mut_trait() == call.trait_id && name == "index_mut" {
            return match call.signature.inputs().get(1) {
                Some(&ty) if is_usize_range_index(tcx, ty) => {
                    Some(StandardOperation::VecRangeIndexMut)
                }
                Some(&ty) if matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)) => {
                    Some(StandardOperation::VecIndexMut)
                }
                _ => None,
            };
        }
        if call.has_trait_diag("Clone")
            && name == "clone"
            && call.signature.inputs().len() == 1
            && vec_elem_ty(tcx, call.result_ty).is_some_and(is_trivial_clone_primitive_ty)
        {
            return Some(StandardOperation::VecClone);
        }
        if lang.deref_trait() == call.trait_id && name == "deref" {
            return Some(StandardOperation::VecAsSlice);
        }
        if lang.deref_mut_trait() == call.trait_id && name == "deref_mut" {
            return Some(StandardOperation::VecAsMutSlice);
        }
        if lang.drop_trait() == call.trait_id && name == "drop" {
            return Some(StandardOperation::VecDrop);
        }
        if call.has_trait_item_diag("rcgjs_extend")
            && call
                .signature
                .inputs()
                .get(1)
                .is_some_and(|arg| standard_owner(tcx, *arg) == Some(RepresentationCandidate::Vec))
        {
            return Some(StandardOperation::VecExtend);
        }
        return None;
    }

    (call.result_owner == Some(RepresentationCandidate::Vec)
        && call.has_trait_diag("Default")
        && name == "default"
        && call.signature.inputs().is_empty())
    .then_some(StandardOperation::VecNew)
}

/// `String` inherent methods, trait impls, and `String::with_capacity`.
fn string_op<'tcx>(call: &AssocCall<'tcx>, name: &str) -> Option<StandardOperation> {
    let lang = call.tcx.lang_items();
    if call.receiver == Some(RepresentationCandidate::String) {
        if call.standard_impl == Some(RepresentationCandidate::String) {
            return match name {
                "len" => Some(StandardOperation::StringLen),
                "capacity" => Some(StandardOperation::StringCapacity),
                "push" => Some(StandardOperation::StringPush),
                "push_str" => Some(StandardOperation::StringPushStr),
                "clear" => Some(StandardOperation::StringClear),
                "as_str" => Some(StandardOperation::StringAsStr),
                _ => None,
            };
        }
        if lang.deref_trait() == call.trait_id && name == "deref" {
            return Some(StandardOperation::StringAsStr);
        }
        if lang.drop_trait() == call.trait_id && name == "drop" {
            return Some(StandardOperation::StringDrop);
        }
        return None;
    }

    (call.result_owner == Some(RepresentationCandidate::String)
        && call.standard_impl == Some(RepresentationCandidate::String)
        && name == "with_capacity")
        .then_some(StandardOperation::StringWithCapacity)
}

/// `BTreeMap<K, V>` inherent methods, `Drop`, and constructors (`new` / `default`).
fn btree_map_op<'tcx>(call: &AssocCall<'tcx>, name: &str) -> Option<StandardOperation> {
    let tcx = call.tcx;
    let lang = tcx.lang_items();
    if call.receiver == Some(RepresentationCandidate::BTreeMap) {
        if call.standard_impl == Some(RepresentationCandidate::BTreeMap) {
            let key_ty = call
                .owner
                .and_then(|ty| btree_map_key_ty(tcx, ty))
                .filter(|&k| is_native_map_key_ty(tcx, k))?;
            let valid_lookup = || {
                call.signature.inputs().len() == 2
                    && is_native_map_lookup_key_ty(tcx, key_ty, call.signature.inputs()[1])
            };
            return match name {
                "len" if call.signature.inputs().len() == 1 => Some(StandardOperation::BTreeMapLen),
                "is_empty" if call.signature.inputs().len() == 1 => {
                    Some(StandardOperation::BTreeMapIsEmpty)
                }
                "clear" if call.signature.inputs().len() == 1 => {
                    Some(StandardOperation::BTreeMapClear)
                }
                "contains_key" if valid_lookup() => Some(StandardOperation::BTreeMapContainsKey),
                "get" if valid_lookup() => Some(StandardOperation::BTreeMapGet),
                "get_mut" if valid_lookup() => Some(StandardOperation::BTreeMapGetMut),
                "insert" if call.signature.inputs().len() == 3 => {
                    Some(StandardOperation::BTreeMapInsert)
                }
                "remove" if valid_lookup() => Some(StandardOperation::BTreeMapRemove),
                _ => None,
            };
        }
        if lang.drop_trait() == call.trait_id && name == "drop" {
            return call
                .receiver_ty
                .and_then(|ty| btree_map_key_ty(tcx, ty))
                .is_some_and(|k| is_native_map_key_ty(tcx, k))
                .then_some(StandardOperation::BTreeMapDrop);
        }
        return None;
    }

    (call.result_owner == Some(RepresentationCandidate::BTreeMap)
        && call.signature.inputs().is_empty()
        && ((call.standard_impl == Some(RepresentationCandidate::BTreeMap) && name == "new")
            || (call.has_trait_diag("Default") && name == "default"))
        && btree_map_key_ty(tcx, call.result_ty).is_some_and(|k| is_native_map_key_ty(tcx, k)))
    .then_some(StandardOperation::BTreeMapNew)
}

/// The `str` call-site identity of a resolved method instance (`StandardOperation::is_str`), for
/// the link step's `js-str=bytes` lowering (`jsc-link/src/lower/bstr.rs`).
///
/// Identity is the rustc definition, never a name alone: an inherent method of `impl str`, or
/// the `Index`/`PartialEq`/`Ord` impl whose self type is `str`, with the exact argument types the
/// lowering spells (a `&str` receiver; a `&str` or `char` pattern; a `usize` range of the four
/// `core::ops` shapes). Anything else, a user method of the same name included, is untagged.
/// One method of `impl [u8]` joins them, `is_ascii` (`&[u8] -> bool`), which `str::is_ascii` is
/// written with: an inherent impl of a primitive type is one only `core` can write.
pub(crate) fn str_op<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    if let Some(diag) = tcx.get_diagnostic_name(definition) {
        match diag.as_str() {
            "rcgjs_str_byte_at" => return Some(StandardOperation::StrByteAt),
            "rcgjs_str_slice_unchecked" => return Some(StandardOperation::StrSliceUnchecked),
            _ => {}
        }
    }
    let associated = tcx.opt_associated_item(definition)?;
    let trait_id = match associated.container {
        ty::AssocContainer::InherentImpl => None,
        ty::AssocContainer::TraitImpl(Ok(trait_item)) => Some(tcx.parent(trait_item)),
        _ => return None,
    };
    let owner = tcx.normalize_erasing_regions(
        typing_env(),
        tcx.type_of(tcx.parent(definition))
            .instantiate(tcx, arguments),
    );
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let symbol = associated.name();
    let name = symbol.as_str();
    let byte_slice = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Slice(element) if matches!(element.kind(), ty::Uint(ty::UintTy::U8)));
    if byte_slice(owner) {
        let byte_slice_ref = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Ref(_, pointee, Mutability::Not) if byte_slice(*pointee));
        let exact = trait_id.is_none()
            && name == "is_ascii"
            && matches!(signature.inputs(), [receiver] if byte_slice_ref(*receiver))
            && signature.output().is_bool();
        return exact.then_some(StandardOperation::SliceIsAscii);
    }
    if !owner.is_str() {
        return None;
    }
    let [receiver, rest @ ..] = signature.inputs() else {
        return None;
    };
    if !is_shared_str_ref(*receiver) {
        return None;
    }
    let lang = tcx.lang_items();
    match (trait_id, name, rest) {
        (None, "len", []) => Some(StandardOperation::StrLen),
        (None, "is_char_boundary", [index])
            if matches!(index.kind(), ty::Uint(ty::UintTy::Usize)) =>
        {
            Some(StandardOperation::StrIsCharBoundary)
        }
        (None, "trim", []) => Some(StandardOperation::StrTrim),
        (None, "trim_start", []) => Some(StandardOperation::StrTrimStart),
        (None, "trim_end", []) => Some(StandardOperation::StrTrimEnd),
        (None, "is_ascii", []) => Some(StandardOperation::StrIsAscii),
        (
            None,
            "starts_with" | "ends_with" | "find" | "rfind" | "contains" | "strip_prefix"
            | "strip_suffix",
            [pattern],
        ) if is_shared_str_ref(*pattern) || matches!(pattern.kind(), ty::Char) => {
            Some(match name {
                "starts_with" => StandardOperation::StrStartsWith,
                "ends_with" => StandardOperation::StrEndsWith,
                "find" => StandardOperation::StrFind,
                "rfind" => StandardOperation::StrRFind,
                "contains" => StandardOperation::StrContains,
                "strip_prefix" => StandardOperation::StrStripPrefix,
                _ => StandardOperation::StrStripSuffix,
            })
        }
        (None, "get", [index]) if is_usize_range_index(tcx, *index) => {
            Some(StandardOperation::StrGet)
        }
        (Some(trait_id), "index", [index])
            if lang.index_trait() == Some(trait_id) && is_usize_range_index(tcx, *index) =>
        {
            Some(StandardOperation::StrIndex)
        }
        (Some(trait_id), "eq", [other])
            if lang.eq_trait() == Some(trait_id) && is_shared_str_ref(*other) =>
        {
            Some(StandardOperation::StrEq)
        }
        (Some(trait_id), "cmp", [other])
            if tcx.get_diagnostic_name(trait_id) == Some(sym::Ord) && is_shared_str_ref(*other) =>
        {
            Some(StandardOperation::StrCmp)
        }
        (Some(trait_id), "partial_cmp", [other])
            if lang.partial_ord_trait() == Some(trait_id) && is_shared_str_ref(*other) =>
        {
            Some(StandardOperation::StrPartialCmp)
        }
        _ => None,
    }
}

/// Whether `index` is one of the four `core::ops` range shapes over `usize` (`Range<usize>`,
/// `RangeFrom<usize>`, `RangeTo<usize>`, or `RangeFull`).
fn is_usize_range_index<'tcx>(tcx: TyCtxt<'tcx>, index: Ty<'tcx>) -> bool {
    let ty::Adt(range, range_args) = index.kind() else {
        return false;
    };
    let usize_or_none = range_args
        .types()
        .all(|ty| matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)));
    let lang = tcx.lang_items();
    let shapes = [
        lang.range_struct(),
        lang.range_from_struct(),
        lang.range_to_struct(),
        lang.range_full_struct(),
    ];
    usize_or_none && shapes.contains(&Some(range.did()))
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
    let ty::Ref(_, pointee, Mutability::Not) = input.kind() else {
        return None;
    };
    match pointee.kind() {
        ty::Int(_) | ty::Uint(_) => Some(PrimitiveDisplay::Integer),
        ty::Bool => Some(PrimitiveDisplay::Bool),
        ty::Char => Some(PrimitiveDisplay::Char),
        _ => None,
    }
}

fn format_to_string_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    if !tcx
        .opt_item_name(definition)
        .is_some_and(|name| name.as_str() == "format")
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
    let name_sym = tcx.opt_item_name(definition)?;
    let name = name_sym.as_str();
    if name == "new"
        && tcx
            .opt_parent(definition)
            .and_then(|parent| tcx.get_diagnostic_name(parent))
            .is_some_and(|diag| diag.as_str() == "FmtArgumentsNew")
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
    if tcx.get_diagnostic_name(definition.did()) != Some(sym::Vec) {
        return None;
    }
    args.types().next()
}

fn btree_map_key_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(definition, args) = ty.kind() else {
        return None;
    };
    if !tcx
        .get_diagnostic_name(definition.did())
        .is_some_and(|name| name.as_str() == "BTreeMap")
    {
        return None;
    }
    args.types().next()
}

fn is_shared_str_ref(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        ty::Ref(_, pointee, Mutability::Not) if pointee.is_str()
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
        ty::Ref(_, pointee, Mutability::Not) if pointee.is_str() => true,
        ty::Adt(definition, _) => tcx.lang_items().string() == Some(definition.did()),
        _ => false,
    }
}

fn is_native_map_lookup_key_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    key_ty: Ty<'tcx>,
    arg_ty: Ty<'tcx>,
) -> bool {
    let ty::Ref(_, q_ty, Mutability::Not) = *arg_ty.kind() else {
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
    let did = definition.did();
    if tcx.lang_items().string() == Some(did) {
        return Some(RepresentationCandidate::String);
    }
    match tcx.get_diagnostic_name(did) {
        Some(sym::Vec) => Some(RepresentationCandidate::Vec),
        Some(name) if name.as_str() == "BTreeMap" => Some(RepresentationCandidate::BTreeMap),
        _ => None,
    }
}

pub(crate) fn type_meta<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> TypeMeta {
    let (candidate, abi, boundary, allocator) = match ty.kind() {
        ty::Adt(definition, arguments) => {
            let did = definition.did();
            let candidate = if definition.is_box() {
                RepresentationCandidate::Box
            } else if tcx.lang_items().string() == Some(did) {
                RepresentationCandidate::String
            } else {
                match tcx.get_diagnostic_name(did) {
                    Some(sym::Vec) => RepresentationCandidate::Vec,
                    Some(name) if name.as_str() == "BTreeMap" => RepresentationCandidate::BTreeMap,
                    _ => RepresentationCandidate::None,
                }
            };
            let abi = if definition.repr().transparent() {
                NominalAbi::Transparent
            } else if definition.repr().c() {
                NominalAbi::C
            } else {
                NominalAbi::Rust
            };
            let boundary = if is_host_boundary_ty(tcx, ty) {
                PinnedBoundary::Host
            } else {
                PinnedBoundary::None
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

/// Whether `ty` is a host-boundary type: either any ADT defined in `mires_js` or any
/// `#[repr(transparent)]` struct from any crate whose single non-ZST field recursively resolves
/// (through transparent single-field structs) to a host-boundary type.
fn is_host_boundary_ty<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> bool {
    let mut visited = rustc_data_structures::fx::FxHashSet::default();
    for _ in 0..32 {
        let peeled = crate::value::peel_pattern(ty);
        if !visited.insert(peeled) {
            return false;
        }
        let ty::Adt(definition, arguments) = peeled.kind() else {
            return false;
        };
        if tcx.crate_name(definition.did().krate).as_str() == "mires_js" {
            return true;
        }
        if !definition.is_struct() || !definition.repr().transparent() {
            return false;
        }
        let Some(inner) = single_non_zst_field_ty(tcx, *definition, arguments) else {
            return false;
        };
        ty = inner;
    }
    false
}

/// The sole non-ZST field type of struct `definition`, or `None` if it has zero or multiple
/// non-ZST fields.
fn single_non_zst_field_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: ty::AdtDef<'tcx>,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<Ty<'tcx>> {
    let mut live = definition
        .non_enum_variant()
        .fields
        .iter()
        .filter_map(|field| {
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, arguments));
            (!crate::value::is_zst(tcx, field_ty)).then_some(field_ty)
        });
    let field = live.next()?;
    if live.next().is_some() {
        return None;
    }
    Some(field)
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
    if let Some(slice_iter) = tcx.get_diagnostic_item(Symbol::intern("SliceIter")) {
        if tcx.parent(definition.did()) == tcx.parent(slice_iter)
            && tcx.item_name(definition.did()).as_str() == "IterMut"
        {
            return IteratorIdentity::SliceMut;
        }
    }
    IteratorIdentity::None
}
