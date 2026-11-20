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
pub(crate) fn is_wide_pointee(pointee: Ty<'_>) -> bool {
    matches!(pointee.kind(), ty::Str | ty::Slice(_))
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
    /// The length of the pointee, for a wide pointer.
    pub(crate) meta: Option<u64>,
    /// Whether the pointer's own bytes could be read at all.
    pub(crate) readable: bool,
}

impl Pointer {
    /// The allocation this pointer names, if it names one.
    pub(crate) fn alloc_id(&self) -> Option<AllocId> {
        self.provenance.map(|provenance| provenance.alloc_id())
    }
}

/// Reads the pointer stored at `offset`, taking its length from the next slot when `wide`.
pub(crate) fn read_pointer(
    tcx: TyCtxt<'_>,
    alloc: &Allocation,
    offset: Size,
    wide: bool,
) -> Pointer {
    let pointer_size = tcx.data_layout.pointer_size();
    let bits = read_scalar(tcx, alloc, offset, pointer_size);
    let meta = match wide {
        true => read_scalar(tcx, alloc, offset + pointer_size, pointer_size)
            .map(|int| int.to_uint(pointer_size) as u64),
        false => None,
    };
    Pointer {
        provenance: alloc.provenance().ptrs().get(&offset).copied(),
        offset: bits.map_or(Size::ZERO, |int| {
            Size::from_bytes(int.to_uint(pointer_size) as u64)
        }),
        meta,
        readable: bits.is_some(),
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
