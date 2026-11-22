//! The byte level primitives every constant reader in the backend shares.
//!
//! Two readers decode the same const evaluated allocations for different consumers: the one in
//! [`crate::constant`] builds JavaScript expressions, and the one in [`crate::constread`] builds
//! Rust values the backend itself inspects. They must agree byte for byte, so the parts that read
//! bytes live here and neither reader has a decoder of its own.
//!
//! Nothing here reports anything. A function returns what it found or an error describing what it
//! could not read, and the caller turns that into whichever kind of diagnostic it deals in.

use rustc_abi::{Align, Size, TagEncoding, VariantIdx, Variants};
use rustc_middle::mir::interpret::{
    alloc_range, read_target_uint, write_target_uint, AllocId, Allocation, CtfeProvenance,
    GlobalAlloc, Scalar,
};
use rustc_middle::mir::Mutability;
use rustc_middle::ty::layout::TyAndLayout;
use rustc_middle::ty::{self, ScalarInt, Ty, TyCtxt};

use crate::value::typing_env;

/// Reads `size` bytes at `offset` as an integer of that width.
pub(crate) fn read_scalar(
    tcx: TyCtxt<'_>,
    alloc: &Allocation,
    offset: Size,
    size: Size,
) -> Option<ScalarInt> {
    let start = offset.bytes_usize();
    let end = start.checked_add(size.bytes_usize())?;
    if end > alloc.len() {
        return None;
    }
    let bytes = alloc.inspect_with_uninit_and_ptr_outside_interpreter(start..end);
    let value = read_target_uint(tcx.data_layout.endian, bytes).ok()?;
    ScalarInt::try_from_uint(value, size)
}

/// An allocation holding the bytes of one scalar, laid out for `ty`.
///
/// The scalar is written at offset zero and the rest is left zero: a value whose layout is a
/// single scalar occupies its low bytes, and the padding a larger layout adds is never read back,
/// because a reader only visits the fields the type declares.
pub(crate) fn scratch_alloc<'tcx>(
    tcx: TyCtxt<'tcx>,
    int: ScalarInt,
    ty: Ty<'tcx>,
) -> Option<Allocation> {
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    let size = layout.size.max(int.size());
    let mut bytes = vec![0u8; size.bytes_usize()];
    let width = int.size().bytes_usize();
    write_target_uint(
        tcx.data_layout.endian,
        &mut bytes[..width],
        int.to_uint(int.size()),
    )
    .ok()?;
    Some(Allocation::from_bytes_byte_aligned_immutable(bytes, ()))
}

/// Whether a value of `ty` is a primitive scalar (`bool`, `char`, integer, or float).
pub(crate) fn is_primitive(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_)
    )
}

/// Whether a value of `ty` is a pointer the readers decode by its provenance, rather than by the
/// fields of a type wrapped around one.
pub(crate) fn is_pointer(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), ty::Ref(..) | ty::RawPtr(..) | ty::FnPtr(..))
}

/// An allocation holding one pointer scalar, provenance included, laid out for `ty`.
///
/// The counterpart of [`scratch_alloc`] for a scalar constant whose bits are a pointer and whose
/// type is not one: `Option<&T>`, `Option<NonNull<T>>`, `Option<fn()>`, a newtype around a `&T`.
/// Such a value is a scalar only because its layout says so, and it decodes like the same value
/// stored in memory: the pointer goes back at offset zero, where a scalar layout keeps its one
/// scalar, together with the provenance that makes it a pointer. Without the provenance the bytes
/// would be the offset into the pointed-at allocation and nothing else, which for `Some(&b'.')` is
/// the number zero, `None`'s niche.
pub(crate) fn scratch_pointer_alloc<'tcx>(
    tcx: TyCtxt<'tcx>,
    pointer: Scalar,
    ty: Ty<'tcx>,
) -> Option<Allocation> {
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    let pointer_size = tcx.data_layout.pointer_size();
    let size = layout.size.max(pointer_size);
    let mut alloc = Allocation::from_bytes(
        vec![0u8; size.bytes_usize()],
        Align::ONE,
        Mutability::Mut,
        (),
    );
    alloc
        .write_scalar(&tcx, alloc_range(Size::ZERO, pointer_size), pointer)
        .ok()?;
    alloc.mutability = Mutability::Not;
    Some(alloc)
}

/// Why the variant of an enum could not be worked out from its bytes.
pub(crate) enum TagError {
    /// The type has no variants at all, so no value of it exists.
    Uninhabited,
    /// The tag's bytes could not be read, or they hold a pointer that names no variant.
    Unreadable,
    /// The tag was read, and names no variant.
    NoSuchVariant(u128),
}

/// Which variant of `def` the bytes at `offset` hold.
///
/// The bytes hold a *tag*, and which variant that tag names depends on how the layout chose to
/// encode it. A direct tag is the discriminant itself; a niche tag is an otherwise-invalid value
/// of some field, which is how `Option<&T>` fits in one pointer.
///
/// A tag can also be a pointer: the niche of `Option<&T>` is the reference itself. Its bytes are
/// then only the offset into the allocation it points at, so they are never read as a number —
/// `Some(&b'.')` points at offset zero, and zero is `None`'s niche. See [`pointer_tag_variant`].
pub(crate) fn variant_index<'tcx>(
    tcx: TyCtxt<'tcx>,
    alloc: &Allocation,
    offset: Size,
    layout: TyAndLayout<'tcx>,
    def: ty::AdtDef<'tcx>,
) -> Result<VariantIdx, TagError> {
    match &layout.variants {
        Variants::Empty => Err(TagError::Uninhabited),
        Variants::Single { index } => Ok(*index),
        Variants::Multiple {
            tag,
            tag_encoding,
            tag_field,
            ..
        } => {
            let tag_size = tag.size(&tcx);
            let tag_offset = offset + layout.fields.offset(tag_field.as_usize());
            if !alloc
                .provenance()
                .range_empty(alloc_range(tag_offset, tag_size), &tcx)
            {
                return pointer_tag_variant(tcx, alloc, tag_offset, tag_size, tag_encoding);
            }
            let bits = read_scalar(tcx, alloc, tag_offset, tag_size)
                .map(|int| int.to_uint(tag_size))
                .ok_or(TagError::Unreadable)?;
            match tag_encoding {
                // The tag *is* the discriminant, truncated to the tag's width.
                TagEncoding::Direct => def
                    .discriminants(tcx)
                    .find(|(_, discr)| tag_size.truncate(discr.val) == bits)
                    .map(|(index, _)| index)
                    .ok_or(TagError::NoSuchVariant(bits)),
                // `i = tag.wrapping_sub(niche_start) + niche_variants.start`, and anything landing
                // outside `niche_variants` was the untagged variant. The arithmetic is done at the
                // tag's width, which is what `wrapping` means here.
                TagEncoding::Niche {
                    untagged_variant,
                    niche_variants,
                    niche_start,
                } => {
                    let relative = tag_size.truncate(bits.wrapping_sub(*niche_start));
                    let count =
                        niche_variants.last.index() as u128 - niche_variants.start.index() as u128;
                    Ok(if relative <= count {
                        VariantIdx::from_usize(niche_variants.start.index() + relative as usize)
                    } else {
                        *untagged_variant
                    })
                }
            }
        }
    }
}

/// Which variant a tag whose bytes carry provenance names.
///
/// A pointer has no address at compile time, so it is compared with the niche the way rustc's own
/// interpreter compares it (`InterpCx::read_discriminant`): it names the untagged variant when the
/// niche is the single value zero and the pointer cannot be null, which is every `Option<&T>`,
/// `Option<NonNull<T>>` and `Option<fn()>`. Anything else — a direct tag, a niche with more than
/// one value, a pointer that might be null, bytes holding only part of a pointer — is a tag that
/// interpreter refuses to read as well, and is refused here.
fn pointer_tag_variant(
    tcx: TyCtxt<'_>,
    alloc: &Allocation,
    tag_offset: Size,
    tag_size: Size,
    tag_encoding: &TagEncoding<VariantIdx>,
) -> Result<VariantIdx, TagError> {
    let TagEncoding::Niche {
        untagged_variant,
        niche_variants,
        niche_start,
    } = tag_encoding
    else {
        return Err(TagError::Unreadable);
    };
    if *niche_start != 0
        || niche_variants.start != niche_variants.last
        || tag_size != tcx.data_layout.pointer_size()
    {
        return Err(TagError::Unreadable);
    }
    let pointer = read_pointer(tcx, alloc, tag_offset, false);
    match pointer.alloc_id() {
        Some(alloc_id) if !may_be_null(tcx, alloc_id, pointer.offset) => Ok(*untagged_variant),
        _ => Err(TagError::Unreadable),
    }
}

/// Whether the pointer `offset` bytes into `alloc_id` may be null (`InterpCx::scalar_may_be_null`).
///
/// It cannot be when it is in bounds of its allocation, one past the end included, nor when the
/// offset is not a multiple of the allocation's alignment. A `TypeId`'s pointer stands for bits
/// that are deliberately not revealed, so it is never known not to be null.
fn may_be_null(tcx: TyCtxt<'_>, alloc_id: AllocId, offset: Size) -> bool {
    let global = tcx.global_alloc(alloc_id);
    if matches!(global, GlobalAlloc::TypeId { .. }) {
        return true;
    }
    let (size, align) = global.size_and_align(tcx, typing_env());
    offset > size && offset.bytes().is_multiple_of(align.bytes())
}

/// Whether a pointer to this type carries metadata, and so occupies two pointer sized slots.
pub(crate) fn is_wide_pointee<'tcx>(tcx: TyCtxt<'tcx>, pointee: Ty<'tcx>) -> bool {
    matches!(
        crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee)).kind(),
        ty::Str | ty::Slice(_) | ty::Dynamic(..)
    ) || crate::value::unsized_tail(tcx, pointee).is_some()
}

/// A pointer read out of an allocation: which allocation it names, where into it, and how long the
/// pointee is when the pointer is wide.
///
/// The bytes at the pointer's own offset are only half of it: they hold the offset *within* the
/// allocation being pointed at, and the allocation itself comes from the provenance recorded beside
/// them. No provenance means the bytes are an address and nothing else, which is the address form
/// of the pointer model: `ptr::null()`, an integer cast to a pointer.
pub(crate) struct Pointer {
    pub(crate) provenance: Option<CtfeProvenance>,
    /// The offset into the pointed-at allocation, or the bare address when there is no provenance.
    pub(crate) offset: Size,
    /// The length of the pointee (or offset into a vtable), for a wide pointer.
    pub(crate) meta: Option<u64>,
    /// The provenance of the metadata slot, for a wide pointer whose metadata is a vtable pointer.
    pub(crate) meta_provenance: Option<CtfeProvenance>,
    /// Whether the pointer's own bytes could be read at all.
    pub(crate) readable: bool,
}

impl Pointer {
    /// The allocation this pointer names, if it names one.
    pub(crate) fn alloc_id(&self) -> Option<AllocId> {
        self.provenance.map(|provenance| provenance.alloc_id())
    }

    /// The allocation the metadata slot names, if it names one (a vtable for `dyn Trait`).
    pub(crate) fn meta_alloc_id(&self) -> Option<AllocId> {
        self.meta_provenance.map(|provenance| provenance.alloc_id())
    }
}

/// Reads the pointer stored at `offset`, taking its metadata from the next slot when `wide`.
pub(crate) fn read_pointer(
    tcx: TyCtxt<'_>,
    alloc: &Allocation,
    offset: Size,
    wide: bool,
) -> Pointer {
    let pointer_size = tcx.data_layout.pointer_size();
    let bits = read_scalar(tcx, alloc, offset, pointer_size);
    let (meta, meta_provenance) = match wide {
        true => (
            read_scalar(tcx, alloc, offset + pointer_size, pointer_size)
                .map(|int| int.to_uint(pointer_size) as u64),
            alloc
                .provenance()
                .ptrs()
                .get(&(offset + pointer_size))
                .copied(),
        ),
        false => (None, None),
    };
    Pointer {
        provenance: alloc.provenance().ptrs().get(&offset).copied(),
        offset: bits.map_or(Size::ZERO, |int| {
            Size::from_bytes(int.to_uint(pointer_size) as u64)
        }),
        meta,
        meta_provenance,
        readable: bits.is_some(),
    }
}

/// The concrete type and principal trait named by a wide trait-object pointer's vtable metadata.
///
/// `None` unless the metadata slot points at offset zero of a `GlobalAlloc::VTable`.
pub(crate) fn read_vtable<'tcx>(
    tcx: TyCtxt<'tcx>,
    pointer: &Pointer,
) -> Option<(Ty<'tcx>, Option<ty::ExistentialTraitRef<'tcx>>)> {
    if pointer.meta != Some(0) {
        return None;
    }
    let vtable_id = pointer.meta_alloc_id()?;
    let GlobalAlloc::VTable(concrete_ty, dyn_existential) = tcx.global_alloc(vtable_id) else {
        return None;
    };
    let principal = dyn_existential
        .principal()
        .map(|principal| tcx.instantiate_bound_regions_with_erased(principal));
    Some((concrete_ty, principal))
}

/// Maximum nesting depth when resolving a constant pointer into fields/elements of a `static`.
const MAX_STATIC_PATH_DEPTH: u32 = 32;

/// One JavaScript projection step from a `static` binding down to a nested field or element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StaticPathStep {
    /// A struct field (named or tuple-struct), already escaped via [`crate::naming::Namer::field_name`].
    Field(String),
    /// A tuple or array element at numeric index `index`.
    Index(u32),
}

/// Peels patterns and ONLY non-union transparent wrappers (`#[repr(transparent)]` structs,
/// `UnsafeCell`, `Cell`, `ManuallyDrop`, `NonNull`, `Box`, packed single-field newtypes), stopping
/// at any `union` (`MaybeUninit<T>`, user 1-live-field unions) so a static pointer never silently
/// crosses a union boundary.
fn peel_non_union_transparent<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> Ty<'tcx> {
    ty = crate::value::peel_pattern(ty);
    for _ in 0..16 {
        if matches!(ty.kind(), ty::Adt(def, _) if def.is_union()) {
            break;
        }
        let Some((_, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) else {
            break;
        };
        ty = crate::value::peel_pattern(field_ty);
    }
    ty
}

/// Whether `cur_ty` and `pointee` have the same JS value shape and boxing (`peel_transparent` /
/// `peel_pattern` / `is_indirect`) without crossing any union (`MaybeUninit` / `union`) boundary.
fn static_types_match<'tcx>(tcx: TyCtxt<'tcx>, cur_ty: Ty<'tcx>, pointee: Ty<'tcx>) -> bool {
    let cur_peeled = tcx.erase_and_anonymize_regions(peel_non_union_transparent(tcx, cur_ty));
    let pointee_peeled = tcx.erase_and_anonymize_regions(peel_non_union_transparent(tcx, pointee));
    cur_peeled == pointee_peeled
        && crate::value::is_indirect(tcx, cur_ty) == crate::value::is_indirect(tcx, pointee)
}

/// Resolves a constant pointer at byte `offset` into `static` `def_id` with target type `pointee`
/// to a sequence of JavaScript projection steps from the static root binding.
///
/// - `Some(vec![])` means the pointer names the whole `static` binding (offset 0 with matching
///   JS shape and boxing), keeping `ConstTree::Global` unchanged.
/// - `Some(steps)` (non-empty) names a concrete sub-field/element reachable through structs,
///   tuples, arrays, and non-union transparent wrappers.
/// - `None` fails closed for unions, enums, unsized pointees (slices/`str`/unsized tails),
///   type-punned pointers, ambiguous ZST fields, or out-of-bounds/misaligned offsets.
pub(crate) fn resolve_static_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: rustc_hir::def_id::DefId,
    offset: Size,
    pointee: Ty<'tcx>,
) -> Option<Vec<StaticPathStep>> {
    let static_ty =
        tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    if !static_ty.is_sized(tcx, typing_env()) || !pointee.is_sized(tcx, typing_env()) {
        return None;
    }
    let pointee_layout = tcx.layout_of(typing_env().as_query_input(pointee)).ok()?;
    resolve_static_path_inner(tcx, static_ty, offset, pointee, pointee_layout.size, 0)
}

fn checked_sub_size(lhs: Size, rhs: Size) -> Option<Size> {
    lhs.bytes().checked_sub(rhs.bytes()).map(Size::from_bytes)
}

fn resolve_static_path_inner<'tcx>(
    tcx: TyCtxt<'tcx>,
    cur_ty: Ty<'tcx>,
    rem_offset: Size,
    pointee: Ty<'tcx>,
    pointee_size: Size,
    depth: u32,
) -> Option<Vec<StaticPathStep>> {
    if rem_offset == Size::ZERO && static_types_match(tcx, cur_ty, pointee) {
        return Some(Vec::new());
    }
    if depth >= MAX_STATIC_PATH_DEPTH {
        return None;
    }
    // Never descend into a non-indirect value (primitives, fieldless enums, transparent unions, or
    // boxed direct statics) or a zero-sized container (which has no JS object/array storage to
    // index into safely).
    if !crate::value::is_indirect(tcx, cur_ty) {
        return None;
    }
    let cur_layout = tcx.layout_of(typing_env().as_query_input(cur_ty)).ok()?;
    if cur_layout.is_zst() {
        return None;
    }

    // Peel patterns and non-union transparent wrappers (e.g. `UnsafeCell`, `Cell`, `ManuallyDrop`,
    // `#[repr(transparent)]` structs) whose JS representation IS the inner aggregate.
    let mut ty = crate::value::peel_pattern(cur_ty);
    let mut rem = rem_offset;
    for _ in 0..16 {
        if matches!(ty.kind(), ty::Adt(def, _) if def.is_union()) {
            return None;
        }
        let Some((idx, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) else {
            break;
        };
        let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
        rem = checked_sub_size(rem, layout.fields.offset(idx.as_usize()))?;
        ty = crate::value::peel_pattern(field_ty);
    }

    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    if layout.is_zst() {
        return None;
    }

    match ty.kind() {
        ty::Adt(def, args) if def.is_struct() => {
            let variant = def.non_enum_variant();
            let mut matched: Option<Vec<StaticPathStep>> = None;
            for (idx, field) in variant.fields.iter_enumerated() {
                let field_offset = layout.fields.offset(idx.as_usize());
                let Some(sub_offset) = checked_sub_size(rem, field_offset) else {
                    continue;
                };
                let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                let Ok(field_layout) = tcx.layout_of(typing_env().as_query_input(field_ty)) else {
                    continue;
                };
                if sub_offset
                    .bytes()
                    .checked_add(pointee_size.bytes())
                    .is_none_or(|end| end > field_layout.size.bytes())
                {
                    continue;
                }
                if let Some(mut tail) = resolve_static_path_inner(
                    tcx,
                    field_ty,
                    sub_offset,
                    pointee,
                    pointee_size,
                    depth + 1,
                ) {
                    if matched.is_some() {
                        // Ambiguous match (e.g. two ZST fields of the same type at the same offset).
                        return None;
                    }
                    let mut path = Vec::with_capacity(1 + tail.len());
                    path.push(StaticPathStep::Field(crate::naming::Namer::field_name(
                        variant, idx,
                    )));
                    path.append(&mut tail);
                    matched = Some(path);
                }
            }
            matched
        }
        ty::Tuple(elems) => {
            let mut matched: Option<Vec<StaticPathStep>> = None;
            for (idx, elem_ty) in elems.iter().enumerate() {
                let field_offset = layout.fields.offset(idx);
                let Some(sub_offset) = checked_sub_size(rem, field_offset) else {
                    continue;
                };
                let Ok(elem_layout) = tcx.layout_of(typing_env().as_query_input(elem_ty)) else {
                    continue;
                };
                if sub_offset
                    .bytes()
                    .checked_add(pointee_size.bytes())
                    .is_none_or(|end| end > elem_layout.size.bytes())
                {
                    continue;
                }
                if let Some(mut tail) = resolve_static_path_inner(
                    tcx,
                    elem_ty,
                    sub_offset,
                    pointee,
                    pointee_size,
                    depth + 1,
                ) {
                    if matched.is_some() {
                        return None;
                    }
                    let idx_u32 = u32::try_from(idx).ok()?;
                    let mut path = Vec::with_capacity(1 + tail.len());
                    path.push(StaticPathStep::Index(idx_u32));
                    path.append(&mut tail);
                    matched = Some(path);
                }
            }
            matched
        }
        ty::Array(elem, len) => {
            let count = len.try_to_target_usize(tcx)?;
            let elem_ty = *elem;
            let elem_layout = tcx.layout_of(typing_env().as_query_input(elem_ty)).ok()?;
            let stride = elem_layout.size.bytes();
            if stride == 0 {
                return None;
            }
            let idx = rem.bytes() / stride;
            if idx >= count {
                return None;
            }
            let idx_u32 = u32::try_from(idx).ok()?;
            let sub_offset = Size::from_bytes(rem.bytes() % stride);
            if sub_offset
                .bytes()
                .checked_add(pointee_size.bytes())
                .is_none_or(|end| end > stride)
            {
                return None;
            }
            let mut tail = resolve_static_path_inner(
                tcx,
                elem_ty,
                sub_offset,
                pointee,
                pointee_size,
                depth + 1,
            )?;
            let mut path = Vec::with_capacity(1 + tail.len());
            path.push(StaticPathStep::Index(idx_u32));
            path.append(&mut tail);
            Some(path)
        }
        _ => None,
    }
}

/// Resolves a wide slice constant (`&[E]` / `*const [E]`) of `length` elements at byte `offset`
/// into `static` `def_id` to `(path_to_array, start_index)`.
///
/// Matches a `static` array `[E; N]` (or a nested array field/element inside structs/tuples/arrays)
/// whose element type matches `element_ty` and where `start_index + length <= N`. Fails closed
/// (`None`) on ZST elements, type-punned slices, unions, enums, misaligned offsets, or
/// out-of-bounds lengths.
pub(crate) fn resolve_static_slice<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: rustc_hir::def_id::DefId,
    offset: Size,
    element_ty: Ty<'tcx>,
    length: u64,
) -> Option<(Vec<StaticPathStep>, u32)> {
    let static_ty =
        tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    if !static_ty.is_sized(tcx, typing_env()) || !element_ty.is_sized(tcx, typing_env()) {
        return None;
    }
    let elem_layout = tcx
        .layout_of(typing_env().as_query_input(element_ty))
        .ok()?;
    let stride = elem_layout.size.bytes();
    if stride == 0 {
        return None;
    }
    let slice_bytes = length.checked_mul(stride)?;
    resolve_static_slice_inner(
        tcx,
        static_ty,
        offset,
        element_ty,
        stride,
        length,
        slice_bytes,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn resolve_static_slice_inner<'tcx>(
    tcx: TyCtxt<'tcx>,
    cur_ty: Ty<'tcx>,
    rem_offset: Size,
    element_ty: Ty<'tcx>,
    stride: u64,
    length: u64,
    slice_bytes: u64,
    depth: u32,
) -> Option<(Vec<StaticPathStep>, u32)> {
    if depth >= MAX_STATIC_PATH_DEPTH || !crate::value::is_indirect(tcx, cur_ty) {
        return None;
    }
    let cur_layout = tcx.layout_of(typing_env().as_query_input(cur_ty)).ok()?;
    if cur_layout.is_zst() {
        return None;
    }

    let mut ty = crate::value::peel_pattern(cur_ty);
    let mut rem = rem_offset;
    for _ in 0..16 {
        if matches!(ty.kind(), ty::Adt(def, _) if def.is_union()) {
            return None;
        }
        let Some((idx, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) else {
            break;
        };
        let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
        rem = checked_sub_size(rem, layout.fields.offset(idx.as_usize()))?;
        ty = crate::value::peel_pattern(field_ty);
    }

    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    if layout.is_zst() {
        return None;
    }

    match ty.kind() {
        ty::Array(arr_elem, count) => {
            let count = count.try_to_target_usize(tcx)?;
            if static_types_match(tcx, *arr_elem, element_ty) {
                if !rem.bytes().is_multiple_of(stride) {
                    return None;
                }
                let start = rem.bytes() / stride;
                if start.checked_add(length).is_none_or(|end| end > count) {
                    return None;
                }
                let start_u32 = u32::try_from(start).ok()?;
                return Some((Vec::new(), start_u32));
            }
            let outer_layout = tcx.layout_of(typing_env().as_query_input(*arr_elem)).ok()?;
            let outer_stride = outer_layout.size.bytes();
            if outer_stride == 0 {
                return None;
            }
            let idx = rem.bytes() / outer_stride;
            if idx >= count {
                return None;
            }
            let idx_u32 = u32::try_from(idx).ok()?;
            let sub_offset = Size::from_bytes(rem.bytes() % outer_stride);
            if sub_offset
                .bytes()
                .checked_add(slice_bytes)
                .is_none_or(|end| end > outer_stride)
            {
                return None;
            }
            let (mut tail, start) = resolve_static_slice_inner(
                tcx,
                *arr_elem,
                sub_offset,
                element_ty,
                stride,
                length,
                slice_bytes,
                depth + 1,
            )?;
            let mut path = Vec::with_capacity(1 + tail.len());
            path.push(StaticPathStep::Index(idx_u32));
            path.append(&mut tail);
            Some((path, start))
        }
        ty::Adt(def, args) if def.is_struct() => {
            let variant = def.non_enum_variant();
            let mut matched: Option<(Vec<StaticPathStep>, u32)> = None;
            for (idx, field) in variant.fields.iter_enumerated() {
                let field_offset = layout.fields.offset(idx.as_usize());
                let Some(sub_offset) = checked_sub_size(rem, field_offset) else {
                    continue;
                };
                let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                let Ok(field_layout) = tcx.layout_of(typing_env().as_query_input(field_ty)) else {
                    continue;
                };
                if sub_offset
                    .bytes()
                    .checked_add(slice_bytes)
                    .is_none_or(|end| end > field_layout.size.bytes())
                {
                    continue;
                }
                if let Some((mut tail, start)) = resolve_static_slice_inner(
                    tcx,
                    field_ty,
                    sub_offset,
                    element_ty,
                    stride,
                    length,
                    slice_bytes,
                    depth + 1,
                ) {
                    if matched.is_some() {
                        return None;
                    }
                    let mut path = Vec::with_capacity(1 + tail.len());
                    path.push(StaticPathStep::Field(crate::naming::Namer::field_name(
                        variant, idx,
                    )));
                    path.append(&mut tail);
                    matched = Some((path, start));
                }
            }
            matched
        }
        ty::Tuple(elems) => {
            let mut matched: Option<(Vec<StaticPathStep>, u32)> = None;
            for (idx, elem_ty) in elems.iter().enumerate() {
                let field_offset = layout.fields.offset(idx);
                let Some(sub_offset) = checked_sub_size(rem, field_offset) else {
                    continue;
                };
                let Ok(elem_layout) = tcx.layout_of(typing_env().as_query_input(elem_ty)) else {
                    continue;
                };
                if sub_offset
                    .bytes()
                    .checked_add(slice_bytes)
                    .is_none_or(|end| end > elem_layout.size.bytes())
                {
                    continue;
                }
                if let Some((mut tail, start)) = resolve_static_slice_inner(
                    tcx,
                    elem_ty,
                    sub_offset,
                    element_ty,
                    stride,
                    length,
                    slice_bytes,
                    depth + 1,
                ) {
                    if matched.is_some() {
                        return None;
                    }
                    let idx_u32 = u32::try_from(idx).ok()?;
                    let mut path = Vec::with_capacity(1 + tail.len());
                    path.push(StaticPathStep::Index(idx_u32));
                    path.append(&mut tail);
                    matched = Some((path, start));
                }
            }
            matched
        }
        _ => None,
    }
}

/// Why the bytes of a `&str` could not be turned into one.
pub(crate) enum StrError {
    /// The text runs past the end of the allocation holding it.
    PastEnd,
    /// The bytes are not valid UTF-8.
    NotUtf8,
}

/// The `str` whose `length` bytes start at `offset` in `alloc`.
pub(crate) fn read_str(alloc: &Allocation, offset: Size, length: u64) -> Result<&str, StrError> {
    let start = offset.bytes_usize();
    let end = start.saturating_add(usize::try_from(length).unwrap_or(usize::MAX));
    if end > alloc.len() {
        return Err(StrError::PastEnd);
    }
    let bytes = alloc.inspect_with_uninit_and_ptr_outside_interpreter(start..end);
    std::str::from_utf8(bytes).map_err(|_| StrError::NotUtf8)
}

/// Whether a slice of `length` elements of `stride` bytes each fits in `alloc` from `offset`.
///
/// An element of a zero sized type has no bytes to read, so the check is vacuous for one; the
/// slice still has the length it claims, because that is what `len()` answers.
pub(crate) fn slice_fits(alloc: &Allocation, offset: Size, stride: u64, length: usize) -> bool {
    let end = offset
        .bytes()
        .saturating_add(stride.saturating_mul(length as u64));
    end <= alloc.len() as u64
}
