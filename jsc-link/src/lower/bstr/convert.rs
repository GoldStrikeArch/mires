//! Host-boundary converter synthesis for `js-str=bytes`.
//!
//! Walks reachable JIR types to classify how values containing `str` cross between Rust byte
//! strings and host UTF-16 strings, hashes each converter shape to a deduplicated `$bstr$` name,
//! and synthesizes the corresponding [`LinkItem`] declarations.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::types::{
    EnumRepr, FieldDescriptor, FieldKey, NominalForm, PointerForm, RepresentationCandidate,
    TypeDescriptor, TypeId, TypeKind, TypeTable, VariantDescriptor,
};
use crate::jsast::{self, Expr, Stmt};

use super::super::js::value;
use super::{host_in, host_out, is_str_pointer};

const MAX_DEPTH: usize = 64;

/// Which way a value crosses the host boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Crossing {
    /// Rust text reaching the host: every `str` in the value is decoded.
    Out,
    /// Host text becoming Rust's: every `str` in the value is encoded.
    In,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Mode {
    Val,
    Ref,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ConvTarget {
    Type(TypeId, Mode),
    RawVecInner(TypeId, TypeId),
    RawVecPtr(TypeId),
    FatTail(TypeId),
    FatTailHeader(TypeId),
    ArrayBuf(TypeId),
}

#[derive(Clone, Copy)]
struct PeelInfo {
    cur: TypeId,
    allow_undef: bool,
    via_safe_ptr: bool,
    raw_vec_elem: Option<TypeId>,
}

enum LeafKind {
    Str,
    MaybeUninitStr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConvError {
    NoLayout,
    UnionStr,
    UnnamedField,
    TransparentFieldNoKey,
    UnsizedTailOnNonStruct,
    StructNotSingleVariant,
    UnsizedTailPointeeNoLayout,
    FatTailWithoutUnsizedTail,
    UnsizedTailNoLayout,
    UnsupportedUnsizedTail,
    InvalidRawVecLayout,
    RawPointerStr,
    SlicePointerNoPointeeLayout,
    SlicePointerToNonSlice,
    UnconvertiblePointerForm,
    EnumNotTaggedObject,
    CoroutineStr,
    UnconvertibleShape,
    MaxDepthExceeded,
    UnreachableThinIdentity,
    UnreachableFatTailIdentity,
    NativeVecNoElem,
    NativeVecNoBufConv,
}

impl fmt::Display for ConvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLayout => f.write_str("a type with no described layout"),
            Self::UnionStr => f.write_str("a `str` inside a union"),
            Self::UnnamedField => f.write_str("a `str` in an unnamed field"),
            Self::TransparentFieldNoKey => {
                f.write_str("a `str` in a transparent field with no key")
            }
            Self::UnsizedTailOnNonStruct => f.write_str("an unsized tail on a non-struct"),
            Self::StructNotSingleVariant => f.write_str("a struct with no single variant"),
            Self::UnsizedTailPointeeNoLayout => {
                f.write_str("an unsized tail pointee with no layout")
            }
            Self::FatTailWithoutUnsizedTail => {
                f.write_str("a fat-tail pointer without an unsized tail")
            }
            Self::UnsizedTailNoLayout => f.write_str("an unsized tail with no layout"),
            Self::UnsupportedUnsizedTail => {
                f.write_str("a `str` inside an unsupported unsized struct tail")
            }
            Self::InvalidRawVecLayout => f.write_str("invalid RawVec layout"),
            Self::RawPointerStr => f.write_str("a `str` behind a raw pointer"),
            Self::SlicePointerNoPointeeLayout => {
                f.write_str("a slice pointer with no pointee layout")
            }
            Self::SlicePointerToNonSlice => f.write_str("a slice pointer to a non-slice"),
            Self::UnconvertiblePointerForm => {
                f.write_str("a `str` behind a pointer of no convertible form")
            }
            Self::EnumNotTaggedObject => {
                f.write_str("a `str` in an enum whose value is not a tagged object")
            }
            Self::CoroutineStr => f.write_str("a `str` inside a coroutine"),
            Self::UnconvertibleShape => {
                f.write_str("a `str` inside a value of no convertible shape")
            }
            Self::MaxDepthExceeded => {
                write!(f, "a `str` nested more than {MAX_DEPTH} levels deep")
            }
            Self::UnreachableThinIdentity => f.write_str("unreachable thin identity"),
            Self::UnreachableFatTailIdentity => f.write_str("unreachable fat-tail identity"),
            Self::NativeVecNoElem => f.write_str("a native Vec without element type"),
            Self::NativeVecNoBufConv => f.write_str("a native Vec buffer with no conversion"),
        }
    }
}

enum PointerShape {
    Thin(TypeId),
    Slot(TypeId),
    FatSlice { pointee: TypeId, elem: TypeId },
    FatTail(TypeId),
    Noop,
}

impl PointerShape {
    fn pointee(&self) -> Option<TypeId> {
        match *self {
            Self::Thin(p)
            | Self::Slot(p)
            | Self::FatSlice { pointee: p, .. }
            | Self::FatTail(p) => Some(p),
            Self::Noop => None,
        }
    }
}

enum EnumShape<'a> {
    Fieldless,
    Tagged(&'a [VariantDescriptor]),
}

enum TypeShape<'a> {
    RawVec {
        inner_ty: TypeId,
        elem_ty: TypeId,
        fields: &'a [FieldDescriptor],
    },
    Pointer(PointerShape),
    Tuple {
        is_closure: bool,
        fields: Vec<FieldDescriptor>,
    },
    Array {
        element: TypeId,
    },
    Struct {
        cur: TypeId,
        fields: &'a [FieldDescriptor],
    },
    Enum(EnumShape<'a>),
}

struct ClassifiedType<'a> {
    info: PeelInfo,
    shape: TypeShape<'a>,
}

struct Hasher(u64);

impl Hasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, b: u8) {
        self.0 ^= u64::from(b);
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn u32(&mut self, n: u32) {
        for b in n.to_le_bytes() {
            self.byte(b);
        }
    }

    fn bool(&mut self, v: bool) {
        self.byte(u8::from(v));
    }

    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        for &b in s.as_bytes() {
            self.byte(b);
        }
    }
}

/// Whether a value of type `ty` reaches a `str` anywhere the host can see it: itself, a field,
/// an element, a variant's payload, or the pointee of any pointer it holds.
pub fn reaches_str(table: &TypeTable, ty: TypeId) -> bool {
    fn walk(table: &TypeTable, ty: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
        let ty = table.peeled(ty);
        if !seen.insert(ty) {
            return false;
        }
        let Some(descriptor) = table.get(ty) else {
            return false;
        };
        match &descriptor.kind {
            TypeKind::Str => true,
            TypeKind::Reference { pointee, mutable } => {
                if *mutable && value::is_str(table, *pointee) {
                    false
                } else {
                    walk(table, *pointee, seen)
                }
            }
            TypeKind::RawPointer { pointee, .. } => {
                if value::is_str(table, *pointee) {
                    false
                } else {
                    walk(table, *pointee, seen)
                }
            }
            TypeKind::Tuple(elements) => elements.iter().any(|e| walk(table, *e, seen)),
            TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                walk(table, *element, seen)
            }
            TypeKind::Adt { args, .. } => {
                args.iter().any(|a| walk(table, *a, seen))
                    || table
                        .variants(ty)
                        .iter()
                        .any(|variant| variant.fields.iter().any(|f| walk(table, f.ty, seen)))
            }
            TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
                upvars.iter().any(|u| walk(table, *u, seen))
            }
            TypeKind::Coroutine {
                upvars,
                saved_locals,
                ..
            } => upvars
                .iter()
                .copied()
                .chain(saved_locals.iter().map(|(_, ty)| *ty))
                .any(|u| walk(table, u, seen)),
            _ => false,
        }
    }
    walk(table, ty, &mut BTreeSet::new())
}

fn is_raw_vec(
    table: &TypeTable,
    interns: &Interns,
    desc: &TypeDescriptor,
) -> Option<(TypeId, TypeId)> {
    let TypeKind::Adt {
        form: NominalForm::Struct,
        args,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    let elem_ty = *args.first()?;
    let [variant] = variants.as_slice() else {
        return None;
    };
    if interns.str(variant.name) != Some("RawVec") {
        return None;
    }
    let inner_ty = variant
        .fields
        .iter()
        .find(|f| value::key_name(interns, f.key).as_deref() == Some("inner"))?
        .ty;
    let inner_desc = table.get(table.peeled(inner_ty))?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants: inner_variants,
        ..
    } = &inner_desc.kind
    else {
        return None;
    };
    let [inner_var] = inner_variants.as_slice() else {
        return None;
    };
    let has_ptr = inner_var
        .fields
        .iter()
        .any(|f| value::key_name(interns, f.key).as_deref() == Some("ptr"));
    let has_cap = inner_var
        .fields
        .iter()
        .any(|f| value::key_name(interns, f.key).as_deref() == Some("cap"));
    (has_ptr && has_cap).then_some((inner_ty, elem_ty))
}

fn unwrap_layers(table: &TypeTable, interns: &Interns, ty: TypeId) -> Result<PeelInfo, ConvError> {
    let mut cur = ty;
    let mut allow_undef = false;
    let mut via_safe_ptr = false;
    for _ in 0..16 {
        let Some(desc) = table.get(cur) else {
            return Err(ConvError::NoLayout);
        };
        if is_raw_vec(table, interns, desc).is_some() {
            let TypeKind::Adt { args, .. } = &desc.kind else {
                unreachable!()
            };
            return Ok(PeelInfo {
                cur,
                allow_undef,
                via_safe_ptr,
                raw_vec_elem: args.first().copied(),
            });
        }
        if desc.meta.candidate == RepresentationCandidate::Box {
            via_safe_ptr = true;
        }
        if let TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } = &desc.kind
        {
            if let [variant] = variants.as_slice() {
                if matches!(interns.str(variant.name), Some("NonNull" | "Unique")) {
                    via_safe_ptr = true;
                }
            }
        }
        if let TypeKind::Adt {
            form: NominalForm::Union,
            ..
        } = &desc.kind
        {
            if desc.js.transparent.is_none() {
                return Err(ConvError::UnionStr);
            }
            allow_undef = true;
        }
        if let TypeKind::Pattern(base) = desc.kind {
            cur = base;
            continue;
        }
        if let Some(next) = desc.js.transparent {
            cur = next.ty;
        } else {
            return Ok(PeelInfo {
                cur,
                allow_undef,
                via_safe_ptr,
                raw_vec_elem: None,
            });
        }
    }
    let cur = table.peeled(ty);
    Ok(PeelInfo {
        cur,
        allow_undef,
        via_safe_ptr,
        raw_vec_elem: None,
    })
}

fn leaf_kind(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<LeafKind> {
    if !is_str_pointer(table, ty) {
        return None;
    }
    let info = unwrap_layers(table, interns, ty).ok();
    let allow_undef = info.is_some_and(|i| i.allow_undef) || table.crosses_transparent_union(ty);
    Some(if allow_undef {
        LeafKind::MaybeUninitStr
    } else {
        LeafKind::Str
    })
}

fn vec_elem_ty(table: &TypeTable, ty: TypeId) -> Option<TypeId> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    match &desc.kind {
        TypeKind::Slice(elem) => Some(*elem),
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            match &table.get(table.peeled(*pointee))?.kind {
                TypeKind::Slice(elem) => Some(*elem),
                _ => None,
            }
        }
        TypeKind::Adt { args, .. } if desc.meta.candidate == RepresentationCandidate::Vec => {
            args.first().copied()
        }
        _ => None,
    }
}

fn field_prop_name(interns: &Interns, key: FieldKey) -> Result<String, ConvError> {
    match key {
        FieldKey::Name(_) => value::key_name(interns, key).ok_or(ConvError::UnnamedField),
        FieldKey::Index(i) => Ok(i.to_string()),
        FieldKey::Transparent => Err(ConvError::TransparentFieldNoKey),
    }
}

fn single_struct_variant(table: &TypeTable, ty: TypeId) -> Result<&VariantDescriptor, ConvError> {
    let desc = table.get(ty).ok_or(ConvError::NoLayout)?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return Err(ConvError::StructNotSingleVariant);
    };
    let [variant] = variants.as_slice() else {
        return Err(ConvError::StructNotSingleVariant);
    };
    Ok(variant)
}

fn tuple_or_closure_fields(desc: &TypeDescriptor, elems: &[TypeId]) -> Vec<FieldDescriptor> {
    if desc.js.fields.len() == elems.len() {
        desc.js.fields.clone()
    } else {
        elems
            .iter()
            .enumerate()
            .map(|(i, ty)| FieldDescriptor {
                key: FieldKey::Index(i as u32),
                ty: *ty,
                offset: 0,
            })
            .collect()
    }
}

fn header_convertible_fields<'a>(
    table: &TypeTable,
    desc: &'a TypeDescriptor,
    tail_field_idx: u32,
) -> Result<Vec<(usize, &'a FieldDescriptor)>, ConvError> {
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return Err(ConvError::UnsizedTailOnNonStruct);
    };
    let [variant] = variants.as_slice() else {
        return Err(ConvError::StructNotSingleVariant);
    };
    Ok(variant
        .fields
        .iter()
        .enumerate()
        .filter(|(idx, f)| {
            *idx != tail_field_idx as usize && !table.is_zst(f.ty) && reaches_str(table, f.ty)
        })
        .collect())
}

fn fat_tail_header_fields(
    table: &TypeTable,
    peeled_pt: TypeId,
) -> Result<Vec<(usize, &FieldDescriptor)>, ConvError> {
    let pt_desc = table
        .get(peeled_pt)
        .ok_or(ConvError::UnsizedTailPointeeNoLayout)?;
    let tail_idx = pt_desc
        .js
        .unsized_tail
        .ok_or(ConvError::FatTailWithoutUnsizedTail)?
        .field;
    header_convertible_fields(table, pt_desc, tail_idx)
}

fn fat_tail_parts(
    table: &TypeTable,
    pointee: TypeId,
) -> Result<(TypeId, Option<TypeId>, Option<TypeId>), ConvError> {
    let peeled_pt = table.peeled(pointee);
    let pt_desc = table
        .get(peeled_pt)
        .ok_or(ConvError::UnsizedTailPointeeNoLayout)?;
    let tail_field = pt_desc
        .js
        .unsized_tail
        .ok_or(ConvError::FatTailWithoutUnsizedTail)?;
    let header_needed = !header_convertible_fields(table, pt_desc, tail_field.field)?.is_empty();
    let header = header_needed.then_some(peeled_pt);
    let tail_peeled = table.peeled(tail_field.ty);
    let tail_kind = &table
        .get(tail_peeled)
        .ok_or(ConvError::UnsizedTailNoLayout)?
        .kind;
    let slice_elem = match tail_kind {
        TypeKind::Str => None,
        TypeKind::Slice(elem) => {
            if reaches_str(table, *elem) && !table.is_zst(*elem) {
                Some(*elem)
            } else {
                None
            }
        }
        _ => return Err(ConvError::UnsupportedUnsizedTail),
    };
    Ok((peeled_pt, header, slice_elem))
}

fn classify_pointer(
    table: &TypeTable,
    info: PeelInfo,
    desc: &TypeDescriptor,
    pointee: TypeId,
) -> Result<PointerShape, ConvError> {
    if matches!(desc.kind, TypeKind::RawPointer { .. }) && !info.via_safe_ptr {
        return Err(ConvError::RawPointerStr);
    }
    let pointee_noop = table.is_zst(pointee) || !reaches_str(table, pointee);
    match table.pointer_form(info.cur) {
        Some(PointerForm::Thin) => Ok(PointerShape::Thin(pointee)),
        Some(PointerForm::Slot) => Ok(PointerShape::Slot(pointee)),
        Some(PointerForm::FatSlice) => {
            if pointee_noop {
                let elem = table.get(table.peeled(pointee)).and_then(|d| match d.kind {
                    TypeKind::Slice(e) => Some(e),
                    _ => None,
                });
                return match elem {
                    Some(elem) => Ok(PointerShape::FatSlice { pointee, elem }),
                    None => Ok(PointerShape::Noop),
                };
            }
            let TypeKind::Slice(elem) = table
                .get(table.peeled(pointee))
                .map(|d| &d.kind)
                .ok_or(ConvError::SlicePointerNoPointeeLayout)?
            else {
                return Err(ConvError::SlicePointerToNonSlice);
            };
            Ok(PointerShape::FatSlice {
                pointee,
                elem: *elem,
            })
        }
        Some(PointerForm::FatTail) => Ok(PointerShape::FatTail(pointee)),
        _ if pointee_noop => Ok(PointerShape::Noop),
        _ => Err(ConvError::UnconvertiblePointerForm),
    }
}

fn classify_type<'a>(
    table: &'a TypeTable,
    interns: &Interns,
    ty: TypeId,
) -> Result<ClassifiedType<'a>, ConvError> {
    let info = unwrap_layers(table, interns, ty)?;
    let desc = table.get(info.cur).ok_or(ConvError::NoLayout)?;
    if let Some(elem_ty) = info.raw_vec_elem {
        let (inner_ty, _) =
            is_raw_vec(table, interns, desc).ok_or(ConvError::InvalidRawVecLayout)?;
        let TypeKind::Adt { variants, .. } = &desc.kind else {
            unreachable!()
        };
        return Ok(ClassifiedType {
            info,
            shape: TypeShape::RawVec {
                inner_ty,
                elem_ty,
                fields: &variants[0].fields,
            },
        });
    }
    let shape = match &desc.kind {
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            TypeShape::Pointer(classify_pointer(table, info, desc, *pointee)?)
        }
        TypeKind::Tuple(elems) => TypeShape::Tuple {
            is_closure: false,
            fields: tuple_or_closure_fields(desc, elems),
        },
        TypeKind::Closure { upvars, .. } => TypeShape::Tuple {
            is_closure: true,
            fields: tuple_or_closure_fields(desc, upvars),
        },
        TypeKind::Array { element, .. } => TypeShape::Array { element: *element },
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let [variant] = variants.as_slice() else {
                return Err(ConvError::StructNotSingleVariant);
            };
            TypeShape::Struct {
                cur: info.cur,
                fields: &variant.fields,
            }
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } => match table.enum_repr(info.cur) {
            Some(EnumRepr::Number { .. })
            | Some(EnumRepr::Tag { fieldless: true })
            | Some(EnumRepr::IntTag { fieldless: true }) => TypeShape::Enum(EnumShape::Fieldless),
            Some(EnumRepr::Tag { fieldless: false })
            | Some(EnumRepr::IntTag { fieldless: false }) => {
                TypeShape::Enum(EnumShape::Tagged(variants))
            }
            _ => return Err(ConvError::EnumNotTaggedObject),
        },
        TypeKind::Adt {
            form: NominalForm::Union,
            ..
        } => return Err(ConvError::UnionStr),
        TypeKind::Coroutine { .. } | TypeKind::CoroutineClosure { .. } => {
            return Err(ConvError::CoroutineStr);
        }
        _ => return Err(ConvError::UnconvertibleShape),
    };
    Ok(ClassifiedType { info, shape })
}

fn target_is_identity(
    table: &TypeTable,
    interns: &Interns,
    target: ConvTarget,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, ConvError> {
    if !seen.insert(target) {
        return Ok(false);
    }
    match target {
        ConvTarget::ArrayBuf(elem) | ConvTarget::RawVecPtr(elem) => {
            field_is_identity(table, interns, elem, Mode::Ref, seen)
        }
        ConvTarget::RawVecInner(inner_ty, elem_ty) => {
            let elem_noop = !reaches_str(table, elem_ty)
                || table.is_zst(elem_ty)
                || target_is_identity(table, interns, ConvTarget::RawVecPtr(elem_ty), seen)?;
            if !elem_noop {
                return Ok(false);
            }
            let variant = single_struct_variant(table, table.peeled(inner_ty))?;
            for field in &variant.fields {
                if value::key_name(interns, field.key).as_deref() == Some("ptr")
                    || table.is_zst(field.ty)
                    || !reaches_str(table, field.ty)
                {
                    continue;
                }
                if !field_is_identity(table, interns, field.ty, Mode::Ref, seen)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ConvTarget::FatTailHeader(peeled_pt) => {
            Ok(fat_tail_header_fields(table, peeled_pt)?.is_empty())
        }
        ConvTarget::FatTail(pointee) => {
            let (_, header, slice_elem) = fat_tail_parts(table, pointee)?;
            let header_noop = match header {
                Some(h) => target_is_identity(table, interns, ConvTarget::FatTailHeader(h), seen)?,
                None => true,
            };
            let elem_noop = match slice_elem {
                Some(e) => field_is_identity(table, interns, e, Mode::Ref, seen)?,
                None => true,
            };
            Ok(header_noop && elem_noop)
        }
        ConvTarget::Type(ty, mode) => type_is_identity(table, interns, ty, mode, seen),
    }
}

fn type_is_identity(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    mode: Mode,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, ConvError> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(true);
    }
    if leaf_kind(table, interns, ty).is_some() {
        return Ok(false);
    }
    let classified = classify_type(table, interns, ty)?;
    match classified.shape {
        TypeShape::RawVec {
            inner_ty, elem_ty, ..
        } => target_is_identity(
            table,
            interns,
            ConvTarget::RawVecInner(inner_ty, elem_ty),
            seen,
        ),
        TypeShape::Pointer(ptr) => {
            let Some(pointee) = ptr.pointee() else {
                return Ok(true);
            };
            if table.is_zst(pointee) || !reaches_str(table, pointee) {
                return Ok(true);
            }
            match ptr {
                PointerShape::Thin(p) => {
                    target_is_identity(table, interns, ConvTarget::Type(p, Mode::Ref), seen)
                }
                PointerShape::Slot(p) => field_is_identity(table, interns, p, Mode::Ref, seen),
                PointerShape::FatSlice { elem, .. } => {
                    field_is_identity(table, interns, elem, Mode::Ref, seen)
                }
                PointerShape::FatTail(p) => {
                    target_is_identity(table, interns, ConvTarget::FatTail(p), seen)
                }
                PointerShape::Noop => Ok(true),
            }
        }
        TypeShape::Tuple { fields, .. } => fields_are_identity(table, interns, &fields, mode, seen),
        TypeShape::Array { element } => field_is_identity(table, interns, element, mode, seen),
        TypeShape::Struct { fields, .. } => fields_are_identity(table, interns, fields, mode, seen),
        TypeShape::Enum(EnumShape::Fieldless) => Ok(true),
        TypeShape::Enum(EnumShape::Tagged(variants)) => {
            for variant in variants {
                if !fields_are_identity(table, interns, &variant.fields, mode, seen)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

fn fields_are_identity(
    table: &TypeTable,
    interns: &Interns,
    fields: &[FieldDescriptor],
    mode: Mode,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, ConvError> {
    for f in fields {
        if !field_is_identity(table, interns, f.ty, mode, seen)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn field_is_identity(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    mode: Mode,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, ConvError> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(true);
    }
    if leaf_kind(table, interns, ty).is_some() {
        return Ok(false);
    }
    target_is_identity(table, interns, ConvTarget::Type(ty, mode), seen)
}

struct TargetHasher<'a> {
    table: &'a TypeTable,
    interns: &'a Interns,
    dir: Crossing,
    stack: Vec<ConvTarget>,
    hasher: Hasher,
}

impl<'a> TargetHasher<'a> {
    fn new(table: &'a TypeTable, interns: &'a Interns, dir: Crossing) -> Self {
        Self {
            table,
            interns,
            dir,
            stack: Vec::new(),
            hasher: Hasher::new(),
        }
    }

    fn hash_target(&mut self, target: ConvTarget, depth: usize) -> Result<(), ConvError> {
        if let Some(pos) = self.stack.iter().rposition(|t| *t == target) {
            self.hasher.byte(b'C');
            self.hasher.u32((self.stack.len() - 1 - pos) as u32);
            return Ok(());
        }
        if depth > MAX_DEPTH {
            return Err(ConvError::MaxDepthExceeded);
        }
        self.hasher.byte(match self.dir {
            Crossing::Out => b'O',
            Crossing::In => b'I',
        });
        self.stack.push(target);
        let res = match target {
            ConvTarget::ArrayBuf(elem) => {
                self.hasher.str("ArrayBuf");
                self.hash_field_child(elem, Mode::Ref, depth + 1)
            }
            ConvTarget::RawVecPtr(elem) => {
                self.hasher.str("RawVecPtr");
                self.hash_field_child(elem, Mode::Ref, depth + 1)
            }
            ConvTarget::RawVecInner(inner_ty, elem_ty) => {
                self.hash_raw_vec_inner(inner_ty, elem_ty, depth)
            }
            ConvTarget::FatTailHeader(peeled_pt) => self.hash_fat_tail_header(peeled_pt, depth),
            ConvTarget::FatTail(pointee) => self.hash_fat_tail(pointee, depth),
            ConvTarget::Type(ty, mode) => self.hash_type(ty, mode, depth),
        };
        self.stack.pop();
        res
    }

    fn hash_raw_vec_inner(
        &mut self,
        inner_ty: TypeId,
        elem_ty: TypeId,
        depth: usize,
    ) -> Result<(), ConvError> {
        self.hasher.str("RawVecInner");
        let variant = single_struct_variant(self.table, self.table.peeled(inner_ty))?;
        for field in &variant.fields {
            let prop = field_prop_name(self.interns, field.key)?;
            if prop == "ptr" {
                self.hasher.str("ptr");
                self.hash_target(ConvTarget::RawVecPtr(elem_ty), depth + 1)?;
            } else if !self.table.is_zst(field.ty) && reaches_str(self.table, field.ty) {
                self.hasher.str(&prop);
                self.hash_field_child(field.ty, Mode::Ref, depth + 1)?;
            }
        }
        Ok(())
    }

    fn hash_fat_tail_header(&mut self, peeled_pt: TypeId, depth: usize) -> Result<(), ConvError> {
        self.hasher.str("FatTailHeader");
        for (_, f) in fat_tail_header_fields(self.table, peeled_pt)? {
            let prop = field_prop_name(self.interns, f.key)?;
            self.hasher.str(&prop);
            self.hash_field_child(f.ty, Mode::Ref, depth + 1)?;
        }
        Ok(())
    }

    fn hash_fat_tail(&mut self, pointee: TypeId, depth: usize) -> Result<(), ConvError> {
        self.hasher.str("FatTail");
        let (_, header, slice_elem) = fat_tail_parts(self.table, pointee)?;
        self.hasher.bool(header.is_some());
        if let Some(hdr) = header {
            self.hash_target(ConvTarget::FatTailHeader(hdr), depth + 1)?;
        }
        self.hasher.bool(slice_elem.is_some());
        if let Some(elem) = slice_elem {
            self.hash_field_child(elem, Mode::Ref, depth + 1)?;
        }
        Ok(())
    }

    fn hash_type(&mut self, ty: TypeId, mode: Mode, depth: usize) -> Result<(), ConvError> {
        self.hasher.byte(match mode {
            Mode::Val => b'V',
            Mode::Ref => b'R',
        });
        let classified = classify_type(self.table, self.interns, ty)?;
        self.hasher.bool(classified.info.allow_undef);
        match classified.shape {
            TypeShape::RawVec {
                inner_ty,
                elem_ty,
                fields,
            } => {
                self.hasher.str("RawVec");
                for f in fields {
                    let prop = field_prop_name(self.interns, f.key)?;
                    if prop == "inner" {
                        self.hasher.str("inner");
                        self.hash_target(ConvTarget::RawVecInner(inner_ty, elem_ty), depth + 1)?;
                    } else if !self.table.is_zst(f.ty) && reaches_str(self.table, f.ty) {
                        self.hasher.str(&prop);
                        self.hash_field_child(f.ty, Mode::Ref, depth + 1)?;
                    }
                }
                Ok(())
            }
            TypeShape::Pointer(ptr) => self.hash_pointer(ptr, depth),
            TypeShape::Tuple { is_closure, fields } => {
                self.hasher
                    .str(if is_closure { "Closure" } else { "Tuple" });
                self.hash_fields(&fields, mode, depth)
            }
            TypeShape::Array { element } => {
                self.hasher.str("Array");
                self.hash_field_child(element, mode, depth + 1)
            }
            TypeShape::Struct { fields, .. } => {
                self.hasher.str("Struct");
                self.hash_fields(fields, mode, depth)
            }
            TypeShape::Enum(EnumShape::Fieldless) => Err(ConvError::EnumNotTaggedObject),
            TypeShape::Enum(EnumShape::Tagged(variants)) => {
                self.hasher.str("Enum");
                self.hasher.u32(variants.len() as u32);
                for variant in variants {
                    self.hasher.str(value::variant_name(self.interns, variant));
                    self.hash_fields(&variant.fields, mode, depth)?;
                }
                Ok(())
            }
        }
    }

    fn hash_pointer(&mut self, ptr: PointerShape, depth: usize) -> Result<(), ConvError> {
        match ptr {
            PointerShape::Thin(pointee) => {
                self.hasher.str("Thin");
                self.hash_target(ConvTarget::Type(pointee, Mode::Ref), depth + 1)
            }
            PointerShape::Slot(pointee) => {
                self.hasher.str("Slot");
                self.hash_field_child(pointee, Mode::Ref, depth + 1)
            }
            PointerShape::FatSlice { elem, .. } => {
                self.hasher.str("FatSlice");
                self.hash_field_child(elem, Mode::Ref, depth + 1)
            }
            PointerShape::FatTail(pointee) => {
                self.hasher.str("FatTailPtr");
                self.hash_target(ConvTarget::FatTail(pointee), depth + 1)
            }
            PointerShape::Noop => Err(ConvError::UnconvertiblePointerForm),
        }
    }

    fn hash_fields(
        &mut self,
        fields: &[FieldDescriptor],
        mode: Mode,
        depth: usize,
    ) -> Result<(), ConvError> {
        self.hasher.u32(fields.len() as u32);
        for f in fields {
            let prop = field_prop_name(self.interns, f.key)?;
            self.hasher.str(&prop);
            self.hash_field_child(f.ty, mode, depth + 1)?;
        }
        Ok(())
    }

    fn hash_field_child(&mut self, ty: TypeId, mode: Mode, depth: usize) -> Result<(), ConvError> {
        if depth > MAX_DEPTH {
            return Err(ConvError::MaxDepthExceeded);
        }
        if !reaches_str(self.table, ty) || self.table.is_zst(ty) {
            self.hasher.byte(b'-');
            return Ok(());
        }
        match leaf_kind(self.table, self.interns, ty) {
            Some(LeafKind::Str) => {
                self.hasher.byte(b's');
                Ok(())
            }
            Some(LeafKind::MaybeUninitStr) => {
                self.hasher.byte(b'u');
                Ok(())
            }
            None => self.hash_target(ConvTarget::Type(ty, mode), depth),
        }
    }
}

fn is_ref_target(table: &TypeTable, interns: &Interns, target: ConvTarget) -> bool {
    match target {
        ConvTarget::Type(_, Mode::Ref) => true,
        ConvTarget::Type(ty, Mode::Val) => {
            let Ok(info) = unwrap_layers(table, interns, ty) else {
                return false;
            };
            info.raw_vec_elem.is_some()
                || table.get(info.cur).is_some_and(|d| {
                    matches!(
                        d.kind,
                        TypeKind::Reference { .. } | TypeKind::RawPointer { .. }
                    )
                })
        }
        _ => true,
    }
}

fn wrap_leaf(leaf: LeafKind, value: Expr, dir: Crossing) -> Expr {
    let conv = match dir {
        Crossing::Out => host_out(value.clone()),
        Crossing::In => host_in(value.clone()),
    };
    match leaf {
        LeafKind::Str => conv,
        LeafKind::MaybeUninitStr => {
            let param = "$u0".to_owned();
            let id = jsast::id(param.clone());
            let body = match dir {
                Crossing::Out => host_out(id.clone()),
                Crossing::In => host_in(id.clone()),
            };
            jsast::call(
                jsast::arrow(
                    vec![param],
                    jsast::cond(
                        jsast::binary(jsast::BinOp::StrictEq, id, jsast::undefined()),
                        jsast::undefined(),
                        body,
                    ),
                ),
                vec![value],
            )
        }
    }
}

fn spec_ref_fn(dir: Crossing) -> &'static str {
    match dir {
        Crossing::Out => "bstr_ref",
        Crossing::In => "bstr_ref_in",
    }
}

fn obj_spec(entries: Vec<(String, Expr)>) -> Expr {
    jsast::object_of(vec![
        ("k", jsast::string("o")),
        ("f", jsast::object(entries)),
    ])
}

fn ref_ret(dir: Crossing, spec: Expr) -> Stmt {
    jsast::ret(jsast::rt_call(spec_ref_fn(dir), vec![jsast::id("v"), spec]))
}

pub(super) struct ConvCx<'a> {
    table: &'a TypeTable,
    interns: &'a Interns,
    native_vec: bool,
    cache: BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, ConvError>>,
    pending: Vec<(ConvTarget, Crossing)>,
}

impl<'a> ConvCx<'a> {
    pub(super) fn new(table: &'a TypeTable, interns: &'a Interns, native_vec: bool) -> Self {
        Self {
            table,
            interns,
            native_vec,
            cache: BTreeMap::new(),
            pending: Vec::new(),
        }
    }

    fn push_pending(&mut self, target: ConvTarget, dir: Crossing) {
        self.pending.push((target, dir));
    }

    fn pop_pending(&mut self) -> Option<(ConvTarget, Crossing)> {
        self.pending.pop()
    }

    fn target_name(
        &mut self,
        target: ConvTarget,
        dir: Crossing,
    ) -> Result<Option<JsName>, ConvError> {
        if let Some(cached) = self.cache.get(&(target, dir)) {
            return cached.clone();
        }
        let res = (|| -> Result<Option<JsName>, ConvError> {
            if target_is_identity(self.table, self.interns, target, &mut BTreeSet::new())? {
                return Ok(None);
            }
            let mut hasher = TargetHasher::new(self.table, self.interns, dir);
            hasher.hash_target(target, 0)?;
            let prefix = match (dir, is_ref_target(self.table, self.interns, target)) {
                (Crossing::Out, false) => "$bstr$out$",
                (Crossing::In, false) => "$bstr$in$",
                (Crossing::Out, true) => "$bstr$out$r$",
                (Crossing::In, true) => "$bstr$in$r$",
            };
            Ok(Some(JsName::new(format!(
                "{prefix}{:016x}",
                hasher.hasher.0
            ))))
        })();
        self.cache.insert((target, dir), res.clone());
        res
    }

    fn child_spec_expr(&mut self, ty: TypeId) -> Result<Option<Expr>, ConvError> {
        if !reaches_str(self.table, ty) || self.table.is_zst(ty) {
            return Ok(None);
        }
        if let Some(leaf) = leaf_kind(self.table, self.interns, ty) {
            return Ok(Some(jsast::string(match leaf {
                LeafKind::Str => "s",
                LeafKind::MaybeUninitStr => "u",
            })));
        }
        self.target_pair_expr(ConvTarget::Type(ty, Mode::Ref))
    }

    fn target_pair_expr(&mut self, target: ConvTarget) -> Result<Option<Expr>, ConvError> {
        let Some(out_name) = self.target_name(target, Crossing::Out)? else {
            return Ok(None);
        };
        let Some(in_name) = self.target_name(target, Crossing::In)? else {
            return Ok(None);
        };
        self.pending.push((target, Crossing::Out));
        self.pending.push((target, Crossing::In));
        Ok(Some(jsast::array(vec![
            jsast::id(out_name.as_str().to_owned()),
            jsast::id(in_name.as_str().to_owned()),
        ])))
    }

    fn child_val_expr(&mut self, ty: TypeId, read: Expr, dir: Crossing) -> Result<Expr, ConvError> {
        if !reaches_str(self.table, ty) || self.table.is_zst(ty) {
            return Ok(read);
        }
        if let Some(leaf) = leaf_kind(self.table, self.interns, ty) {
            return Ok(match leaf {
                LeafKind::Str => match dir {
                    Crossing::Out => host_out(read),
                    Crossing::In => host_in(read),
                },
                LeafKind::MaybeUninitStr => wrap_leaf(leaf, read, dir),
            });
        }
        let target = ConvTarget::Type(ty, Mode::Val);
        match self.target_name(target, dir)? {
            Some(name) => {
                self.pending.push((target, dir));
                Ok(jsast::call(jsast::id(name.as_str().to_owned()), vec![read]))
            }
            None => Ok(read),
        }
    }

    pub(super) fn build_decl(
        &mut self,
        name: &JsName,
        target: ConvTarget,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        let mut body: Vec<Stmt> = Vec::new();
        match target {
            ConvTarget::ArrayBuf(elem) => {
                let elem_spec = self
                    .child_spec_expr(elem)?
                    .unwrap_or_else(|| jsast::string("s"));
                let spec = jsast::object_of(vec![("k", jsast::string("a")), ("c", elem_spec)]);
                body.push(ref_ret(dir, spec));
            }
            ConvTarget::RawVecPtr(elem) => {
                let elem_spec = self
                    .child_spec_expr(elem)?
                    .unwrap_or_else(|| jsast::string("s"));
                let spec = jsast::object_of(vec![("k", jsast::string("slice")), ("c", elem_spec)]);
                body.push(ref_ret(dir, spec));
            }
            ConvTarget::RawVecInner(inner_ty, elem_ty) => {
                body.push(self.build_raw_vec_inner(inner_ty, elem_ty, dir)?);
            }
            ConvTarget::FatTailHeader(peeled_pt) => {
                body.push(self.build_fat_tail_header(peeled_pt, dir)?);
            }
            ConvTarget::FatTail(pointee) => {
                body.push(self.build_fat_tail(pointee, dir)?);
            }
            ConvTarget::Type(ty, mode) => {
                self.build_type_body(ty, mode, dir, &mut body)?;
            }
        }
        Ok(jsast::function(
            name.as_str().to_owned(),
            vec!["v".to_owned()],
            body,
        ))
    }

    fn build_raw_vec_inner(
        &mut self,
        inner_ty: TypeId,
        elem_ty: TypeId,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        let variant = single_struct_variant(self.table, self.table.peeled(inner_ty))?;
        let mut entries = Vec::new();
        for f in &variant.fields {
            let prop = field_prop_name(self.interns, f.key)?;
            if prop == "ptr" {
                if let Some(pair) = self.target_pair_expr(ConvTarget::RawVecPtr(elem_ty))? {
                    entries.push((prop, pair));
                }
            } else if let Some(spec) = self.child_spec_expr(f.ty)? {
                entries.push((prop, spec));
            }
        }
        Ok(ref_ret(dir, obj_spec(entries)))
    }

    fn build_fat_tail_header(
        &mut self,
        peeled_pt: TypeId,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        let mut entries = Vec::new();
        for (_, f) in fat_tail_header_fields(self.table, peeled_pt)? {
            let prop = field_prop_name(self.interns, f.key)?;
            if let Some(spec) = self.child_spec_expr(f.ty)? {
                entries.push((prop, spec));
            }
        }
        Ok(ref_ret(dir, obj_spec(entries)))
    }

    fn build_fat_tail(&mut self, pointee: TypeId, dir: Crossing) -> Result<Stmt, ConvError> {
        let (_, header, slice_elem) = fat_tail_parts(self.table, pointee)?;
        let hdr_expr = match header {
            Some(h) => self
                .target_pair_expr(ConvTarget::FatTailHeader(h))?
                .unwrap_or_else(jsast::null),
            None => jsast::null(),
        };
        let elem_expr = match slice_elem {
            Some(e) => self.child_spec_expr(e)?.unwrap_or_else(jsast::null),
            None => jsast::null(),
        };
        let spec = jsast::object_of(vec![
            ("k", jsast::string("tail")),
            ("h", hdr_expr),
            ("c", elem_expr),
        ]);
        Ok(ref_ret(dir, spec))
    }

    fn build_type_body(
        &mut self,
        ty: TypeId,
        mode: Mode,
        dir: Crossing,
        body: &mut Vec<Stmt>,
    ) -> Result<(), ConvError> {
        let classified = classify_type(self.table, self.interns, ty)?;
        if classified.info.allow_undef
            && mode == Mode::Val
            && !matches!(classified.shape, TypeShape::RawVec { .. })
        {
            body.push(jsast::if_else(
                jsast::binary(jsast::BinOp::StrictEq, jsast::id("v"), jsast::undefined()),
                vec![jsast::ret(jsast::undefined())],
                Vec::new(),
            ));
        }
        match classified.shape {
            TypeShape::RawVec {
                inner_ty,
                elem_ty,
                fields,
            } => body.push(self.build_raw_vec(inner_ty, elem_ty, fields, dir)?),
            TypeShape::Pointer(ptr) => body.push(self.build_pointer(ptr, dir)?),
            TypeShape::Tuple { fields, .. } => body.push(self.build_tuple(&fields, mode, dir)?),
            TypeShape::Array { element } => body.push(self.build_array(element, mode, dir)?),
            TypeShape::Struct { cur, fields } => {
                if self.native_vec && crate::analysis::heap::is_global_vec_ty(self.table, cur) {
                    body.push(self.build_native_vec(cur, mode, dir)?);
                } else {
                    body.push(self.build_struct(fields, mode, dir)?);
                }
            }
            TypeShape::Enum(EnumShape::Fieldless) => return Err(ConvError::EnumNotTaggedObject),
            TypeShape::Enum(EnumShape::Tagged(variants)) => {
                self.build_enum(variants, mode, dir, body)?;
            }
        }
        Ok(())
    }

    fn build_raw_vec(
        &mut self,
        inner_ty: TypeId,
        elem_ty: TypeId,
        fields: &[FieldDescriptor],
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        let mut entries = Vec::new();
        for f in fields {
            let prop = field_prop_name(self.interns, f.key)?;
            if prop == "inner" {
                if let Some(pair) =
                    self.target_pair_expr(ConvTarget::RawVecInner(inner_ty, elem_ty))?
                {
                    entries.push((prop, pair));
                }
            } else if let Some(spec) = self.child_spec_expr(f.ty)? {
                entries.push((prop, spec));
            }
        }
        Ok(ref_ret(dir, obj_spec(entries)))
    }

    fn build_pointer(&mut self, ptr: PointerShape, dir: Crossing) -> Result<Stmt, ConvError> {
        match ptr {
            PointerShape::Thin(pointee) => {
                let inner_target = ConvTarget::Type(pointee, Mode::Ref);
                let inner_name = self
                    .target_name(inner_target, dir)?
                    .ok_or(ConvError::UnreachableThinIdentity)?;
                self.pending.push((inner_target, dir));
                Ok(jsast::ret(jsast::call(
                    jsast::id(inner_name.as_str().to_owned()),
                    vec![jsast::id("v")],
                )))
            }
            PointerShape::Slot(pointee) => {
                let c = self.child_spec_expr(pointee)?.unwrap_or_else(jsast::null);
                let spec = jsast::object_of(vec![("k", jsast::string("slot")), ("c", c)]);
                Ok(ref_ret(dir, spec))
            }
            PointerShape::FatSlice { elem, .. } => {
                let c = self.child_spec_expr(elem)?.unwrap_or_else(jsast::null);
                let spec = jsast::object_of(vec![("k", jsast::string("slice")), ("c", c)]);
                Ok(ref_ret(dir, spec))
            }
            PointerShape::FatTail(pointee) => {
                let inner_target = ConvTarget::FatTail(pointee);
                let inner_name = self
                    .target_name(inner_target, dir)?
                    .ok_or(ConvError::UnreachableFatTailIdentity)?;
                self.pending.push((inner_target, dir));
                Ok(jsast::ret(jsast::call(
                    jsast::id(inner_name.as_str().to_owned()),
                    vec![jsast::id("v")],
                )))
            }
            PointerShape::Noop => Err(ConvError::UnconvertiblePointerForm),
        }
    }

    fn build_tuple(
        &mut self,
        fields: &[FieldDescriptor],
        mode: Mode,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        if mode == Mode::Ref {
            let mut entries = Vec::new();
            for (i, f) in fields.iter().enumerate() {
                if let Some(spec) = self.child_spec_expr(f.ty)? {
                    entries.push((i.to_string(), spec));
                }
            }
            Ok(ref_ret(dir, obj_spec(entries)))
        } else {
            let mut items = Vec::with_capacity(fields.len());
            for (i, f) in fields.iter().enumerate() {
                let read = jsast::index(jsast::id("v"), jsast::num(i as f64));
                items.push(self.child_val_expr(f.ty, read, dir)?);
            }
            Ok(jsast::ret(jsast::array(items)))
        }
    }

    fn build_array(
        &mut self,
        element: TypeId,
        mode: Mode,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        if mode == Mode::Ref {
            let c = self.child_spec_expr(element)?.unwrap_or_else(jsast::null);
            let spec = jsast::object_of(vec![("k", jsast::string("a")), ("c", c)]);
            Ok(ref_ret(dir, spec))
        } else {
            let elem_param = "$e".to_owned();
            let elem_id = jsast::id(elem_param.clone());
            let converted = self.child_val_expr(element, elem_id.clone(), dir)?;
            let guarded = jsast::cond(
                jsast::binary(jsast::BinOp::StrictEq, elem_id, jsast::undefined()),
                jsast::undefined(),
                converted,
            );
            Ok(jsast::ret(jsast::method_call(
                jsast::id("v"),
                "map",
                vec![jsast::arrow(vec![elem_param], guarded)],
            )))
        }
    }

    fn build_native_vec(
        &mut self,
        cur: TypeId,
        mode: Mode,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        let elem = vec_elem_ty(self.table, cur).ok_or(ConvError::NativeVecNoElem)?;
        if mode == Mode::Ref {
            let buf_pair = self
                .target_pair_expr(ConvTarget::ArrayBuf(elem))?
                .ok_or(ConvError::NativeVecNoBufConv)?;
            Ok(ref_ret(dir, obj_spec(vec![("buf".to_owned(), buf_pair)])))
        } else {
            let buf_conv = self
                .target_name(ConvTarget::ArrayBuf(elem), dir)?
                .map(|cname| {
                    self.pending.push((ConvTarget::ArrayBuf(elem), dir));
                    jsast::call(
                        jsast::id(cname.as_str().to_owned()),
                        vec![jsast::member(jsast::id("v"), "buf")],
                    )
                })
                .unwrap_or_else(|| jsast::member(jsast::id("v"), "buf"));
            let entries = vec![
                ("buf".to_owned(), buf_conv),
                ("len".to_owned(), jsast::member(jsast::id("v"), "len")),
            ];
            Ok(jsast::ret(jsast::object(entries)))
        }
    }

    fn build_struct(
        &mut self,
        fields: &[FieldDescriptor],
        mode: Mode,
        dir: Crossing,
    ) -> Result<Stmt, ConvError> {
        if mode == Mode::Ref {
            let mut entries = Vec::new();
            for f in fields {
                if let Some(spec) = self.child_spec_expr(f.ty)? {
                    entries.push((field_prop_name(self.interns, f.key)?, spec));
                }
            }
            Ok(ref_ret(dir, obj_spec(entries)))
        } else {
            let mut entries = Vec::with_capacity(fields.len());
            for f in fields {
                let key = field_prop_name(self.interns, f.key)?;
                let read = jsast::member(jsast::id("v"), key.clone());
                let expr = self.child_val_expr(f.ty, read, dir)?;
                entries.push((key, expr));
            }
            Ok(jsast::ret(jsast::object(entries)))
        }
    }

    fn build_enum(
        &mut self,
        variants: &[VariantDescriptor],
        mode: Mode,
        dir: Crossing,
        body: &mut Vec<Stmt>,
    ) -> Result<(), ConvError> {
        if mode == Mode::Ref {
            let mut v_entries = Vec::new();
            for variant in variants {
                let mut f_entries = Vec::new();
                for f in &variant.fields {
                    if let Some(spec) = self.child_spec_expr(f.ty)? {
                        f_entries.push((field_prop_name(self.interns, f.key)?, spec));
                    }
                }
                if !f_entries.is_empty() {
                    v_entries.push((
                        value::variant_name(self.interns, variant).to_owned(),
                        jsast::object(f_entries),
                    ));
                }
            }
            let spec = jsast::object_of(vec![
                ("k", jsast::string("e")),
                ("v", jsast::object(v_entries)),
            ]);
            body.push(ref_ret(dir, spec));
        } else {
            for variant in variants {
                let has_conv = variant
                    .fields
                    .iter()
                    .any(|f| !self.table.is_zst(f.ty) && reaches_str(self.table, f.ty));
                if !has_conv {
                    continue;
                }
                let mut entries = Vec::with_capacity(variant.fields.len() + 1);
                entries.push((
                    value::TAG.to_owned(),
                    value::tag_literal(self.interns, variant),
                ));
                let mut any_changed = false;
                for f in &variant.fields {
                    let key = field_prop_name(self.interns, f.key)?;
                    let read = jsast::member(jsast::id("v"), key.clone());
                    let expr = self.child_val_expr(f.ty, read.clone(), dir)?;
                    if expr != read {
                        any_changed = true;
                    }
                    entries.push((key, expr));
                }
                if any_changed {
                    body.push(jsast::if_else(
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::member(jsast::id("v"), value::TAG),
                            value::tag_literal(self.interns, variant),
                        ),
                        vec![jsast::ret(jsast::object(entries))],
                        Vec::new(),
                    ));
                }
            }
            body.push(jsast::ret(jsast::id("v")));
        }
        Ok(())
    }
}

/// `value`, a JavaScript value of Rust type `ty`, as it must cross the host boundary in
/// `direction`: every `str` it reaches converted, everything else exactly as it was, so the host
/// sees what it sees under `js-str=utf16` and Rust holds what it would have held.
pub fn host_convert(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    value: Expr,
    direction: Crossing,
) -> Result<Expr, String> {
    host_convert_carrier(table, interns, ty, Carrier::Baseline, value, direction)
}

/// Like [`host_convert`], accounting for an explicit non-baseline value [`Carrier`] at the host
/// boundary (`Carrier::NativeVec` / `Carrier::SliceView`).
pub fn host_convert_carrier(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    carrier: Carrier,
    value: Expr,
    direction: Crossing,
) -> Result<Expr, String> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(value);
    }
    if let Some(leaf) = leaf_kind(table, interns, ty) {
        return Ok(wrap_leaf(leaf, value, direction));
    }
    let _ = carrier;
    let target = ConvTarget::Type(ty, Mode::Val);
    let mut cx = ConvCx::new(table, interns, false);
    match cx
        .target_name(target, direction)
        .map_err(|e| e.to_string())?
    {
        Some(name) => Ok(jsast::call(
            jsast::id(name.as_str().to_owned()),
            vec![value],
        )),
        None => Ok(value),
    }
}

pub(super) fn make_link_item(name: JsName, decl: Stmt) -> LinkItem {
    let mut refs = BTreeSet::new();
    jsast::visit_free_idents(&decl, &mut |ident| {
        if ident != name.as_str() {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });
    let mut js_item = JsItem {
        name: name.clone(),
        kind: ItemKind::Fn,
        decl,
        refs: refs.clone(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: name.as_str().to_owned(),
        order: None,
        loc: None,
        names: Vec::new(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    };
    crate::minify::shorten_locals(std::slice::from_mut(&mut js_item));
    let (js, mappings) = js_item.to_js_mapped(crate::opts::get().comments);
    LinkItem {
        name,
        kind: ItemKind::Fn,
        js,
        refs,
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: js_item.debug_path,
        crate_name: "generated".to_owned(),
        order: None,
        map: item_map(mappings),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

/// Synthesizes deduplicated `$bstr$out$...` and `$bstr$in$...` converter [`LinkItem`]s into
/// `table` (and appends them to `reachable`) for every `$bstr$` helper called by reachable items.
pub fn synthesize_converters(
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &mut Vec<JsName>,
    native_vec: bool,
) {
    let mut needed: BTreeSet<JsName> = BTreeSet::new();
    for name in reachable.iter() {
        let Some(item) = table.get(name) else {
            continue;
        };
        for r in &item.refs {
            if r.as_str().starts_with("$bstr$") && !table.contains_key(r) {
                needed.insert(r.clone());
            }
        }
    }
    if needed.is_empty() {
        return;
    }

    let source_names: Vec<JsName> = reachable
        .iter()
        .filter(|name| table.get(*name).is_some_and(|i| i.jir.interns().is_some()))
        .cloned()
        .collect();

    let mut synthesized: Vec<(JsName, LinkItem)> = Vec::new();
    for src_name in &source_names {
        if needed.is_empty() {
            break;
        }
        let Some(interns_ref) = table.get(src_name).and_then(|i| i.jir.interns()) else {
            continue;
        };
        let interns: Interns = interns_ref.clone();
        let types = &interns.types;
        let mut cx = ConvCx::new(types, &interns, native_vec);
        let mut expanded: BTreeSet<(ConvTarget, Crossing)> = BTreeSet::new();

        for desc in types.descriptors() {
            if needed.is_empty() {
                break;
            }
            if desc.layout.zst || !reaches_str(types, desc.id) {
                continue;
            }
            let candidates = [
                (ConvTarget::Type(desc.id, Mode::Val), Crossing::Out),
                (ConvTarget::Type(desc.id, Mode::Val), Crossing::In),
                (ConvTarget::Type(desc.id, Mode::Ref), Crossing::Out),
                (ConvTarget::Type(desc.id, Mode::Ref), Crossing::In),
            ];
            for (target, dir) in candidates {
                if needed.is_empty() {
                    break;
                }
                let Ok(Some(cname)) = cx.target_name(target, dir) else {
                    continue;
                };
                if !needed.contains(&cname) {
                    continue;
                }
                cx.push_pending(target, dir);
                while let Some((cur_target, cur_dir)) = cx.pop_pending() {
                    if !expanded.insert((cur_target, cur_dir)) {
                        continue;
                    }
                    let Ok(Some(cur_name)) = cx.target_name(cur_target, cur_dir) else {
                        continue;
                    };
                    needed.remove(&cur_name);
                    if table.contains_key(&cur_name) {
                        continue;
                    }
                    if let Ok(decl) = cx.build_decl(&cur_name, cur_target, cur_dir) {
                        let item = make_link_item(cur_name.clone(), decl);
                        synthesized.push((cur_name, item));
                    }
                }
            }
        }
    }

    for (name, item) in synthesized {
        if !table.contains_key(&name) {
            table.insert(name.clone(), item);
            reachable.push(name);
        }
    }
}
