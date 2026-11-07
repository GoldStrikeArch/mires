//! Reading a `static` back into a Rust value the backend can inspect.
//!
//! [`crate::constant`] decodes a const evaluated allocation into JavaScript, which is what a
//! `static` the program keeps needs. This module decodes the same bytes into a [`ConstVal`] the
//! *backend* reads, which is what a `static` the backend consumes at codegen time needs: the
//! template payload `view-abi` hands over, and nothing else so far.
//!
//! Both go through [`crate::constant::raw`], so there is one decoder for scalars, for tags and for
//! the provenance behind a `&str` or a `&[T]`. A second, independent decoder is the failure mode
//! this module is arranged to avoid: it would agree with the first on every case anyone tested and
//! disagree on the one nobody did.
//!
//! # Errors
//!
//! Every function returns `Result<_, String>` and the message names the thing that was being read.
//! A payload the backend cannot decode is a diagnostic at the call site that passed it, not a
//! panic in the compiler, so nothing here unwraps.

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::interpret::{Allocation, GlobalAlloc};
use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::constant::raw;
use crate::naming::Namer;
use crate::value::{self, typing_env};

/// A constant decoded into the shapes the backend's own readers need.
///
/// Deliberately not every shape Rust has. What is missing is what no backend consumer has asked
/// for: a reference to another `static`, a function pointer, a raw pointer. Reading one is an
/// error naming the type rather than a value that quietly stands for the wrong thing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConstVal {
    Bool(bool),
    /// A signed integer, or a `char` as its code point.
    Int(i128),
    Uint(u128),
    Float(f64),
    Str(String),
    /// An array, a slice or a tuple: elements in order, with no field names.
    Seq(Vec<ConstVal>),
    /// A struct: its fields in declaration order, under the names the Rust source gives them.
    Struct(Vec<(String, ConstVal)>),
    /// One variant of an enum, by name, with its fields.
    Enum { variant: String, fields: Vec<(String, ConstVal)> },
    /// The unit value, and the value of any other zero sized type.
    Unit,
}

impl ConstVal {
    /// The name of the shape this value has, for an error that has to say what it found.
    fn kind(&self) -> &'static str {
        match self {
            ConstVal::Bool(_) => "a `bool`",
            ConstVal::Int(_) => "a signed integer",
            ConstVal::Uint(_) => "an unsigned integer",
            ConstVal::Float(_) => "a float",
            ConstVal::Str(_) => "a string",
            ConstVal::Seq(_) => "a sequence",
            ConstVal::Struct(_) => "a struct",
            ConstVal::Enum { .. } => "an enum",
            ConstVal::Unit => "the unit value",
        }
    }

    /// This value as a `bool`.
    pub(crate) fn as_bool(&self, what: &str) -> Result<bool, String> {
        match self {
            ConstVal::Bool(value) => Ok(*value),
            other => Err(format!("`{what}` is {}, not a `bool`", other.kind())),
        }
    }

    /// This value as a `u32`, rejecting one that does not fit.
    pub(crate) fn as_u32(&self, what: &str) -> Result<u32, String> {
        let value = match self {
            ConstVal::Uint(value) => *value,
            ConstVal::Int(value) if *value >= 0 => *value as u128,
            other => return Err(format!("`{what}` is {}, not an unsigned integer", other.kind())),
        };
        u32::try_from(value).map_err(|_| format!("`{what}` is `{value}`, which does not fit a `u32`"))
    }

    /// This value as a string.
    pub(crate) fn as_str(&self, what: &str) -> Result<&str, String> {
        match self {
            ConstVal::Str(text) => Ok(text),
            other => Err(format!("`{what}` is {}, not a string", other.kind())),
        }
    }

    /// This value as a sequence of elements.
    pub(crate) fn as_seq(&self, what: &str) -> Result<&[ConstVal], String> {
        match self {
            ConstVal::Seq(elements) => Ok(elements),
            other => Err(format!("`{what}` is {}, not a sequence", other.kind())),
        }
    }

    /// The field named `name` of a struct or of an enum variant.
    pub(crate) fn field(&self, name: &str) -> Result<&ConstVal, String> {
        let fields = match self {
            ConstVal::Struct(fields) => fields,
            ConstVal::Enum { fields, .. } => fields,
            other => {
                return Err(format!("`{name}` was looked up in {}, which has no fields", other.kind()));
            }
        };
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
            .ok_or_else(|| format!("there is no field `{name}`"))
    }

    /// The name of the variant of an enum value.
    pub(crate) fn variant(&self, what: &str) -> Result<&str, String> {
        match self {
            ConstVal::Enum { variant, .. } => Ok(variant),
            other => Err(format!("`{what}` is {}, not an enum", other.kind())),
        }
    }
}

/// Reads the initializer of a `static` back into the value it encodes.
///
/// # Errors
///
/// Returns a message naming what could not be read: an initializer that does not const evaluate,
/// or a value whose type this reader does not decode.
pub(crate) fn read_static<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> Result<ConstVal, String> {
    let ty = tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    let alloc = tcx
        .eval_static_initializer(def_id)
        .map_err(|_| "its initializer does not const evaluate".to_string())?;
    read_alloc(tcx, alloc.inner(), Size::ZERO, ty)
}

/// Rebuilds the value of type `ty` stored at `offset` in a constant allocation.
fn read_alloc<'tcx>(
    tcx: TyCtxt<'tcx>,
    alloc: &Allocation,
    offset: Size,
    ty: Ty<'tcx>,
) -> Result<ConstVal, String> {
    let layout = tcx
        .layout_of(typing_env().as_query_input(ty))
        .map_err(|_| format!("there is no layout for `{ty}`"))?;

    // A transparent wrapper *is* its one non-1-ZST field, so its bytes are that field's bytes and
    // its value is that field's value. First, because it also covers unions.
    if let Some((index, field_ty)) = value::transparent_field_indexed(tcx, ty) {
        let at = offset + layout.fields.offset(index.as_usize());
        return read_alloc(tcx, alloc, at, field_ty);
    }

    let scalar = |size: Size| {
        raw::read_scalar(tcx, alloc, offset, size)
            .ok_or_else(|| format!("a `{ty}` could not be read out of the allocation"))
    };

    match ty.kind() {
        ty::Bool => Ok(ConstVal::Bool(scalar(layout.size)?.to_uint(layout.size) != 0)),
        ty::Char => Ok(ConstVal::Uint(scalar(layout.size)?.to_uint(layout.size))),
        ty::Uint(_) => Ok(ConstVal::Uint(scalar(layout.size)?.to_uint(layout.size))),
        ty::Int(_) => Ok(ConstVal::Int(scalar(layout.size)?.to_int(layout.size))),
        ty::Float(ty::FloatTy::F32) => {
            Ok(ConstVal::Float(f32::from_bits(scalar(layout.size)?.to_uint(layout.size) as u32).into()))
        }
        ty::Float(ty::FloatTy::F64) => {
            Ok(ConstVal::Float(f64::from_bits(scalar(layout.size)?.to_uint(layout.size) as u64)))
        }
        // A pattern only restricts which values inhabit the type; the bytes are the base type's.
        ty::Pat(base, _) => read_alloc(tcx, alloc, offset, *base),
        ty::Adt(def, args) if def.is_enum() => {
            let index = raw::variant_index(tcx, alloc, offset, layout, *def).map_err(|err| {
                match err {
                    raw::TagError::Uninhabited => format!("`{ty}` is uninhabited"),
                    raw::TagError::Unreadable => {
                        format!("the discriminant of `{ty}` could not be read")
                    }
                    raw::TagError::NoSuchVariant(bits) => {
                        format!("`{ty}` has the discriminant `{bits}`, which names no variant")
                    }
                }
            })?;
            let variant = value::variant_def(*def, Some(index));
            let variant_layout = layout.for_variant(
                &rustc_middle::ty::layout::LayoutCx::new(tcx, typing_env()),
                index,
            );
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field_index, field_def) in variant.fields.iter_enumerated() {
                let field_ty = tcx.normalize_erasing_regions(typing_env(), field_def.ty(tcx, args));
                let at = offset + variant_layout.fields.offset(field_index.as_usize());
                fields.push((
                    Namer::field_name(variant, field_index),
                    read_alloc(tcx, alloc, at, field_ty)?,
                ));
            }
            Ok(ConstVal::Enum { variant: variant.name.to_string(), fields })
        }
        ty::Adt(def, args) if def.is_struct() => {
            let variant = def.non_enum_variant();
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field_index, field_def) in variant.fields.iter_enumerated() {
                let field_ty = tcx.normalize_erasing_regions(typing_env(), field_def.ty(tcx, args));
                let at = offset + layout.fields.offset(field_index.as_usize());
                fields.push((
                    Namer::field_name(variant, field_index),
                    read_alloc(tcx, alloc, at, field_ty)?,
                ));
            }
            Ok(ConstVal::Struct(fields))
        }
        ty::Tuple(element_tys) if element_tys.is_empty() => Ok(ConstVal::Unit),
        ty::Tuple(element_tys) => {
            let mut elements = Vec::with_capacity(element_tys.len());
            for (index, element_ty) in element_tys.iter().enumerate() {
                let at = offset + layout.fields.offset(index);
                elements.push(read_alloc(tcx, alloc, at, element_ty)?);
            }
            Ok(ConstVal::Seq(elements))
        }
        ty::Array(element_ty, count) => {
            let count = count.try_to_target_usize(tcx).unwrap_or_default() as usize;
            let mut elements = Vec::with_capacity(count);
            for index in 0..count {
                let at = offset + layout.fields.offset(index);
                elements.push(read_alloc(tcx, alloc, at, *element_ty)?);
            }
            Ok(ConstVal::Seq(elements))
        }
        ty::Ref(..) => read_reference(tcx, alloc, offset, ty),
        _ => Err(format!("a value of type `{ty}` cannot be read out of a constant")),
    }
}

/// Follows a `&str` or `&[T]` stored at `offset` into the allocation it points at.
///
/// Only the two wide shapes and a reference to an aggregate are followed. A reference to another
/// `static` is not: the backend consumers of this reader want the value, and a `static`'s value is
/// a JavaScript binding rather than bytes.
fn read_reference<'tcx>(
    tcx: TyCtxt<'tcx>,
    alloc: &Allocation,
    offset: Size,
    ty: Ty<'tcx>,
) -> Result<ConstVal, String> {
    let pointee = ty
        .builtin_deref(true)
        .ok_or_else(|| format!("`{ty}` is a reference with no pointee"))?;
    let pointer = raw::read_pointer(tcx, alloc, offset, raw::is_wide_pointee(pointee));
    let alloc_id = pointer
        .alloc_id()
        .ok_or_else(|| format!("a `{ty}` has no provenance, so it points at nothing"))?;
    let GlobalAlloc::Memory(target) = tcx.global_alloc(alloc_id) else {
        return Err(format!("a `{ty}` does not point at a constant allocation"));
    };
    let target = target.inner();

    match pointee.kind() {
        ty::Str => {
            let length = pointer
                .meta
                .ok_or_else(|| format!("a `{ty}` carries no length"))?;
            raw::read_str(target, pointer.offset, length)
                .map(|text| ConstVal::Str(text.to_owned()))
                .map_err(|err| match err {
                    raw::StrError::PastEnd => {
                        format!("a `{ty}` runs past the end of its allocation")
                    }
                    raw::StrError::NotUtf8 => format!("a `{ty}` is not valid UTF-8"),
                })
        }
        ty::Slice(element_ty) => {
            let length = pointer
                .meta
                .ok_or_else(|| format!("a `{ty}` carries no length"))?;
            let length = usize::try_from(length)
                .map_err(|_| format!("a `{ty}` claims a length that does not fit in memory"))?;
            let element = tcx
                .layout_of(typing_env().as_query_input(*element_ty))
                .map_err(|_| format!("there is no layout for `{element_ty}`"))?;
            let stride = element.size.bytes();
            if !raw::slice_fits(target, pointer.offset, stride, length) {
                return Err(format!("a `{ty}` runs past the end of its allocation"));
            }
            let mut elements = Vec::with_capacity(length);
            for index in 0..length {
                let at = pointer.offset + Size::from_bytes(stride * index as u64);
                elements.push(read_alloc(tcx, target, at, *element_ty)?);
            }
            Ok(ConstVal::Seq(elements))
        }
        _ => read_alloc(tcx, target, pointer.offset, pointee),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value shaped like one `view-abi` hole, for the accessors to be exercised over.
    fn hole() -> ConstVal {
        ConstVal::Struct(vec![
            ("path".to_owned(), ConstVal::Str("cn".to_owned())),
            (
                "kind".to_owned(),
                ConstVal::Enum { variant: "Attribute".to_owned(), fields: Vec::new() },
            ),
            ("name".to_owned(), ConstVal::Str("href".to_owned())),
            ("reactive".to_owned(), ConstVal::Bool(true)),
            ("effect_group".to_owned(), ConstVal::Uint(3)),
        ])
    }

    #[test]
    fn accessors_read_the_shapes_they_name() {
        let hole = hole();
        assert_eq!(hole.field("path").unwrap().as_str("path").unwrap(), "cn");
        assert_eq!(hole.field("name").unwrap().as_str("name").unwrap(), "href");
        assert!(hole.field("reactive").unwrap().as_bool("reactive").unwrap());
        assert_eq!(hole.field("effect_group").unwrap().as_u32("effect_group").unwrap(), 3);
        assert_eq!(hole.field("kind").unwrap().variant("kind").unwrap(), "Attribute");
    }

    #[test]
    fn an_accessor_naming_the_wrong_shape_says_what_it_found() {
        let hole = hole();
        assert_eq!(
            hole.field("path").unwrap().as_u32("path").unwrap_err(),
            "`path` is a string, not an unsigned integer"
        );
        assert_eq!(
            hole.field("reactive").unwrap().as_str("reactive").unwrap_err(),
            "`reactive` is a `bool`, not a string"
        );
        assert_eq!(hole.field("missing").unwrap_err(), "there is no field `missing`");
        assert_eq!(
            ConstVal::Unit.field("path").unwrap_err(),
            "`path` was looked up in the unit value, which has no fields"
        );
    }

    #[test]
    fn a_u32_accessor_rejects_a_value_that_does_not_fit() {
        let big = ConstVal::Uint(u128::from(u32::MAX) + 1);
        assert_eq!(
            big.as_u32("effect_group").unwrap_err(),
            "`effect_group` is `4294967296`, which does not fit a `u32`"
        );
        // `u32::MAX` itself is `NO_EFFECT_GROUP`, and must read back rather than being rejected.
        assert_eq!(ConstVal::Uint(u128::from(u32::MAX)).as_u32("g").unwrap(), u32::MAX);
        // A signed literal that happens to be non-negative is still a `u32`; a negative one is not.
        assert_eq!(ConstVal::Int(7).as_u32("g").unwrap(), 7);
        assert!(ConstVal::Int(-1).as_u32("g").is_err());
    }

    #[test]
    fn a_sequence_reads_back_element_by_element() {
        let events = ConstVal::Seq(vec![
            ConstVal::Str("click".to_owned()),
            ConstVal::Str("input".to_owned()),
        ]);
        let elements = events.as_seq("events").unwrap();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].as_str("events[0]").unwrap(), "click");
        assert_eq!(elements[1].as_str("events[1]").unwrap(), "input");
        assert_eq!(
            ConstVal::Unit.as_seq("events").unwrap_err(),
            "`events` is the unit value, not a sequence"
        );
    }
}
