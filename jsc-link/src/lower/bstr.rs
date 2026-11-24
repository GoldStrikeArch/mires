//! Byte strings: the `js-str=bytes` representation of a Rust `str` (experimental).
//!
//! Under `-Cllvm-args=js-str=bytes` a Rust `str` is a JavaScript string whose code unit `i` is
//! byte `i` of its UTF-8 text, so every unit is below 256 and `length` is the Rust length (the
//! way js_of_ocaml holds an OCaml string). The default, `utf16`, is CONTRACT.md's hybrid, and
//! nothing in this module runs under it: every entry point here is reached only behind
//! [`enabled`], which is what keeps the default output byte-identical.
//!
//! The lowering is written once, against the hybrid's runtime helpers (`__rt.str_bytes`,
//! `__rt.bytes_str`, `__rt.str_len`, the native `String` helpers), and so are all the peepholes
//! and purity tables that recognize them by name. A byte string differs from a UTF-16 string in
//! three places only, and this module owns all three:
//!
//! * **what each helper means** — [`rewrite`] swaps every hybrid helper the installed
//!   declaration calls for its byte-string twin in `runtime/shim.js` (a copy of code units where
//!   the hybrid encodes or decodes), turns `__rt.str_len(s)` into `s.length` and drops
//!   `__rt.native_str_wf`, which is the identity on a byte string;
//! * **what a literal is** — [`byte_string`] is the text of a `&'static str` constant as the
//!   JavaScript string of its bytes, and the printer writes its non-ASCII units as `\xNN`;
//! * **where Rust text meets the host** — `__rt.bstr_in` (host text becoming a Rust `str`: UTF-8
//!   encode, lone surrogate to U+FFFD) and `__rt.bstr_out` (the way back: decode), wrapped by
//!   [`host_in`] and [`host_out`] around each crossing the lowering knows by type.
//!
//! [`leftover_helper`] is the backstop: the link step refuses a program in which an item still
//! calls a hybrid helper after the rewrite (a verbatim `js!` fragment, say), rather than mixing
//! the two representations.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::types::{
    EnumRepr, FieldDescriptor, FieldKey, NominalForm, PointerForm, RepresentationCandidate,
    TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, ArrowBody, Expr, Stmt};
use crate::opts::StrRepr;

use super::js::value;

/// Whether this link step holds a `str` as a byte string.
pub fn enabled() -> bool {
    crate::opts::get().str_repr == StrRepr::Bytes
}

/// The byte string of `text`: one `char` per UTF-8 byte, each below U+0100.
pub fn byte_string(text: &str) -> String {
    text.bytes().map(char::from).collect()
}

/// The Rust length of a `str` literal the lowering printed as `literal`.
pub fn literal_len(literal: &str) -> usize {
    if enabled() {
        literal.chars().count()
    } else {
        literal.len()
    }
}

/// A Rust `str` literal: the text itself, or its byte string.
pub fn literal(text: &str) -> Expr {
    if enabled() {
        jsast::string(byte_string(text))
    } else {
        jsast::string(text)
    }
}

/// The Rust text of one `char` code point: its UTF-16 string, or its UTF-8 byte string.
pub fn char_text(point: Expr) -> Expr {
    if enabled() {
        jsast::rt_call("bstr_char", vec![point])
    } else {
        jsast::method_call(jsast::id("String"), "fromCodePoint", vec![point])
    }
}

/// Whether `ty` is a shared `&str` reference: a value the run time holds as the JavaScript string
/// itself. Mutable `&mut str` and raw `str` pointers (`*const str`, `*mut str`, `NonNull<str>`,
/// `Unique<str>`, `Box<str>`) are live `{ buf, off, len }` UTF-8 byte slices
/// (`PointerForm::FatSlice`), identical in `utf16` and `bytes` modes.
pub fn is_str_pointer(table: &TypeTable, ty: TypeId) -> bool {
    value::is_const_str_ptr(table, ty)
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

/// Which way a value crosses the host boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Crossing {
    /// Rust text reaching the host: every `str` in the value is decoded.
    Out,
    /// Host text becoming Rust's: every `str` in the value is encoded.
    In,
}

const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Mode {
    Val,
    Ref,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ConvTarget {
    Type(TypeId, Mode),
    RawVecInner(TypeId, TypeId),
    RawVecPtr(TypeId),
    FatTail(TypeId),
    FatTailHeader(TypeId),
    ArrayBuf(TypeId),
    NativeVecCarrier(TypeId),
    SliceViewCarrier(TypeId),
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

fn unwrap_layers(table: &TypeTable, interns: &Interns, ty: TypeId) -> Result<PeelInfo, String> {
    let mut cur = ty;
    let mut allow_undef = false;
    let mut via_safe_ptr = false;
    for _ in 0..16 {
        let Some(desc) = table.get(cur) else {
            return Err("a type with no described layout".to_owned());
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
                return Err("a `str` inside a union".to_owned());
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

fn field_prop_name(interns: &Interns, key: FieldKey) -> Result<String, String> {
    match key {
        FieldKey::Name(_) => {
            value::key_name(interns, key).ok_or_else(|| "a `str` in an unnamed field".to_owned())
        }
        FieldKey::Index(i) => Ok(i.to_string()),
        FieldKey::Transparent => Err("a `str` in a transparent field with no key".to_owned()),
    }
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
) -> Result<Vec<(usize, &'a FieldDescriptor)>, String> {
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return Err("an unsized tail on a non-struct".to_owned());
    };
    let [variant] = variants.as_slice() else {
        return Err("a struct with no single variant".to_owned());
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

fn fat_tail_parts(
    table: &TypeTable,
    pointee: TypeId,
) -> Result<(TypeId, Option<TypeId>, Option<TypeId>), String> {
    let peeled_pt = table.peeled(pointee);
    let pt_desc = table
        .get(peeled_pt)
        .ok_or_else(|| "an unsized tail pointee with no layout".to_owned())?;
    let tail_field = pt_desc
        .js
        .unsized_tail
        .ok_or_else(|| "a fat-tail pointer without an unsized tail".to_owned())?;
    let header_needed = !header_convertible_fields(table, pt_desc, tail_field.field)?.is_empty();
    let header = header_needed.then_some(peeled_pt);
    let tail_peeled = table.peeled(tail_field.ty);
    let tail_kind = &table
        .get(tail_peeled)
        .ok_or_else(|| "an unsized tail with no layout".to_owned())?
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
        _ => return Err("a `str` inside an unsupported unsized struct tail".to_owned()),
    };
    Ok((peeled_pt, header, slice_elem))
}

fn target_is_identity(
    table: &TypeTable,
    interns: &Interns,
    target: ConvTarget,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, String> {
    if !seen.insert(target) {
        return Ok(false);
    }
    match target {
        ConvTarget::NativeVecCarrier(ty) | ConvTarget::SliceViewCarrier(ty) => {
            Ok(vec_elem_ty(table, ty).is_none_or(|e| !reaches_str(table, e) || table.is_zst(e)))
        }
        ConvTarget::ArrayBuf(elem) | ConvTarget::RawVecPtr(elem) => {
            Ok(!reaches_str(table, elem) || table.is_zst(elem) || {
                if leaf_kind(table, interns, elem).is_some() {
                    false
                } else {
                    target_is_identity(table, interns, ConvTarget::Type(elem, Mode::Ref), seen)?
                }
            })
        }
        ConvTarget::RawVecInner(inner_ty, elem_ty) => {
            let elem_noop = !reaches_str(table, elem_ty)
                || table.is_zst(elem_ty)
                || target_is_identity(table, interns, ConvTarget::RawVecPtr(elem_ty), seen)?;
            if !elem_noop {
                return Ok(false);
            }
            let peeled = table.peeled(inner_ty);
            let Some(desc) = table.get(peeled) else {
                return Err("a type with no described layout".to_owned());
            };
            let TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } = &desc.kind
            else {
                return Err("a struct with no single variant".to_owned());
            };
            let [variant] = variants.as_slice() else {
                return Err("a struct with no single variant".to_owned());
            };
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
            let Some(pt_desc) = table.get(peeled_pt) else {
                return Err("an unsized tail pointee with no layout".to_owned());
            };
            let tail_idx = pt_desc
                .js
                .unsized_tail
                .ok_or_else(|| "a fat-tail pointer without an unsized tail".to_owned())?
                .field;
            Ok(header_convertible_fields(table, pt_desc, tail_idx)?.is_empty())
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
        ConvTarget::Type(ty, mode) => {
            if !reaches_str(table, ty) || table.is_zst(ty) {
                return Ok(true);
            }
            if leaf_kind(table, interns, ty).is_some() {
                return Ok(false);
            }
            let info = unwrap_layers(table, interns, ty)?;
            if let Some(elem_ty) = info.raw_vec_elem {
                let inner_ty = is_raw_vec(
                    table,
                    interns,
                    table
                        .get(info.cur)
                        .ok_or_else(|| "a type with no described layout".to_owned())?,
                )
                .map(|(i, _)| i)
                .ok_or_else(|| "invalid RawVec layout".to_owned())?;
                return target_is_identity(
                    table,
                    interns,
                    ConvTarget::RawVecInner(inner_ty, elem_ty),
                    seen,
                );
            }
            let Some(desc) = table.get(info.cur) else {
                return Err("a type with no described layout".to_owned());
            };
            match &desc.kind {
                TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                    if matches!(desc.kind, TypeKind::RawPointer { .. }) && !info.via_safe_ptr {
                        return Err("a `str` behind a raw pointer".to_owned());
                    }
                    if table.is_zst(*pointee) || !reaches_str(table, *pointee) {
                        return Ok(true);
                    }
                    match table.pointer_form(info.cur) {
                        Some(PointerForm::Thin) => target_is_identity(
                            table,
                            interns,
                            ConvTarget::Type(*pointee, Mode::Ref),
                            seen,
                        ),
                        Some(PointerForm::Slot) => {
                            field_is_identity(table, interns, *pointee, Mode::Ref, seen)
                        }
                        Some(PointerForm::FatSlice) => {
                            let TypeKind::Slice(elem) = table
                                .get(table.peeled(*pointee))
                                .map(|d| &d.kind)
                                .ok_or_else(|| {
                                    "a slice pointer with no pointee layout".to_owned()
                                })?
                            else {
                                return Err("a slice pointer to a non-slice".to_owned());
                            };
                            field_is_identity(table, interns, *elem, Mode::Ref, seen)
                        }
                        Some(PointerForm::FatTail) => {
                            target_is_identity(table, interns, ConvTarget::FatTail(*pointee), seen)
                        }
                        _ => Err("a `str` behind a pointer of no convertible form".to_owned()),
                    }
                }
                TypeKind::Tuple(elems) => {
                    let fields = tuple_or_closure_fields(desc, elems);
                    for f in fields {
                        if !field_is_identity(table, interns, f.ty, mode, seen)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                TypeKind::Closure { upvars, .. } => {
                    let fields = tuple_or_closure_fields(desc, upvars);
                    for f in fields {
                        if !field_is_identity(table, interns, f.ty, mode, seen)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                TypeKind::Array { element, .. } => {
                    field_is_identity(table, interns, *element, mode, seen)
                }
                TypeKind::Adt {
                    form: NominalForm::Struct,
                    variants,
                    ..
                } => {
                    let [variant] = variants.as_slice() else {
                        return Err("a struct with no single variant".to_owned());
                    };
                    for f in &variant.fields {
                        if !field_is_identity(table, interns, f.ty, mode, seen)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                TypeKind::Adt {
                    form: NominalForm::Enum,
                    variants,
                    ..
                } => match table.enum_repr(info.cur) {
                    Some(EnumRepr::Number { .. }) | Some(EnumRepr::Tag { fieldless: true }) => {
                        Ok(true)
                    }
                    Some(EnumRepr::Tag { fieldless: false }) => {
                        for variant in variants {
                            for f in &variant.fields {
                                if !field_is_identity(table, interns, f.ty, mode, seen)? {
                                    return Ok(false);
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Err("a `str` in an enum whose value is not a tagged object".to_owned()),
                },
                TypeKind::Adt {
                    form: NominalForm::Union,
                    ..
                } => Err("a `str` inside a union".to_owned()),
                TypeKind::Coroutine { .. } | TypeKind::CoroutineClosure { .. } => {
                    Err("a `str` inside a coroutine".to_owned())
                }
                _ => Err("a `str` inside a value of no convertible shape".to_owned()),
            }
        }
    }
}

fn field_is_identity(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    mode: Mode,
    seen: &mut BTreeSet<ConvTarget>,
) -> Result<bool, String> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(true);
    }
    if leaf_kind(table, interns, ty).is_some() {
        return Ok(false);
    }
    target_is_identity(table, interns, ConvTarget::Type(ty, mode), seen)
}

fn hash_target(
    table: &TypeTable,
    interns: &Interns,
    target: ConvTarget,
    dir: Crossing,
    depth: usize,
    stack: &mut Vec<ConvTarget>,
    h: &mut Hasher,
) -> Result<(), String> {
    if let Some(pos) = stack.iter().rposition(|t| *t == target) {
        h.byte(b'C');
        h.u32((stack.len() - 1 - pos) as u32);
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(format!("a `str` nested more than {MAX_DEPTH} levels deep"));
    }
    h.byte(match dir {
        Crossing::Out => b'O',
        Crossing::In => b'I',
    });
    stack.push(target);
    let res = (|| -> Result<(), String> {
        match target {
            ConvTarget::NativeVecCarrier(ty) => {
                h.str("NativeVec");
                let elem = vec_elem_ty(table, ty)
                    .ok_or_else(|| "a native Vec without element type".to_owned())?;
                hash_target(
                    table,
                    interns,
                    ConvTarget::ArrayBuf(elem),
                    dir,
                    depth + 1,
                    stack,
                    h,
                )
            }
            ConvTarget::SliceViewCarrier(ty) => {
                h.str("SliceView");
                let elem = vec_elem_ty(table, ty)
                    .ok_or_else(|| "a slice view without element type".to_owned())?;
                hash_field_child(table, interns, elem, Mode::Ref, dir, depth + 1, stack, h)
            }
            ConvTarget::ArrayBuf(elem) => {
                h.str("ArrayBuf");
                hash_field_child(table, interns, elem, Mode::Ref, dir, depth + 1, stack, h)
            }
            ConvTarget::RawVecPtr(elem) => {
                h.str("RawVecPtr");
                hash_field_child(table, interns, elem, Mode::Ref, dir, depth + 1, stack, h)
            }
            ConvTarget::RawVecInner(inner_ty, elem_ty) => {
                h.str("RawVecInner");
                let peeled = table.peeled(inner_ty);
                let desc = table
                    .get(peeled)
                    .ok_or_else(|| "a type with no described layout".to_owned())?;
                let TypeKind::Adt {
                    form: NominalForm::Struct,
                    variants,
                    ..
                } = &desc.kind
                else {
                    return Err("a struct with no single variant".to_owned());
                };
                let [variant] = variants.as_slice() else {
                    return Err("a struct with no single variant".to_owned());
                };
                for field in &variant.fields {
                    let prop = field_prop_name(interns, field.key)?;
                    if prop == "ptr" {
                        h.str("ptr");
                        hash_target(
                            table,
                            interns,
                            ConvTarget::RawVecPtr(elem_ty),
                            dir,
                            depth + 1,
                            stack,
                            h,
                        )?;
                    } else if !table.is_zst(field.ty) && reaches_str(table, field.ty) {
                        h.str(&prop);
                        hash_field_child(
                            table,
                            interns,
                            field.ty,
                            Mode::Ref,
                            dir,
                            depth + 1,
                            stack,
                            h,
                        )?;
                    }
                }
                Ok(())
            }
            ConvTarget::FatTailHeader(peeled_pt) => {
                h.str("FatTailHeader");
                let pt_desc = table
                    .get(peeled_pt)
                    .ok_or_else(|| "an unsized tail pointee with no layout".to_owned())?;
                let tail_idx = pt_desc
                    .js
                    .unsized_tail
                    .ok_or_else(|| "a fat-tail pointer without an unsized tail".to_owned())?
                    .field;
                for (_, f) in header_convertible_fields(table, pt_desc, tail_idx)? {
                    let prop = field_prop_name(interns, f.key)?;
                    h.str(&prop);
                    hash_field_child(table, interns, f.ty, Mode::Ref, dir, depth + 1, stack, h)?;
                }
                Ok(())
            }
            ConvTarget::FatTail(pointee) => {
                h.str("FatTail");
                let (_, header, slice_elem) = fat_tail_parts(table, pointee)?;
                h.bool(header.is_some());
                if let Some(hdr) = header {
                    hash_target(
                        table,
                        interns,
                        ConvTarget::FatTailHeader(hdr),
                        dir,
                        depth + 1,
                        stack,
                        h,
                    )?;
                }
                h.bool(slice_elem.is_some());
                if let Some(elem) = slice_elem {
                    hash_field_child(table, interns, elem, Mode::Ref, dir, depth + 1, stack, h)?;
                }
                Ok(())
            }
            ConvTarget::Type(ty, mode) => {
                h.byte(match mode {
                    Mode::Val => b'V',
                    Mode::Ref => b'R',
                });
                let info = unwrap_layers(table, interns, ty)?;
                h.bool(info.allow_undef);
                if let Some(elem_ty) = info.raw_vec_elem {
                    h.str("RawVec");
                    let desc = table
                        .get(info.cur)
                        .ok_or_else(|| "a type with no described layout".to_owned())?;
                    let (inner_ty, _) = is_raw_vec(table, interns, desc)
                        .ok_or_else(|| "invalid RawVec layout".to_owned())?;
                    let TypeKind::Adt { variants, .. } = &desc.kind else {
                        unreachable!()
                    };
                    for f in &variants[0].fields {
                        let prop = field_prop_name(interns, f.key)?;
                        if prop == "inner" {
                            h.str("inner");
                            hash_target(
                                table,
                                interns,
                                ConvTarget::RawVecInner(inner_ty, elem_ty),
                                dir,
                                depth + 1,
                                stack,
                                h,
                            )?;
                        } else if !table.is_zst(f.ty) && reaches_str(table, f.ty) {
                            h.str(&prop);
                            hash_field_child(
                                table,
                                interns,
                                f.ty,
                                Mode::Ref,
                                dir,
                                depth + 1,
                                stack,
                                h,
                            )?;
                        }
                    }
                    return Ok(());
                }
                let desc = table
                    .get(info.cur)
                    .ok_or_else(|| "a type with no described layout".to_owned())?;
                match &desc.kind {
                    TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                        if matches!(desc.kind, TypeKind::RawPointer { .. }) && !info.via_safe_ptr {
                            return Err("a `str` behind a raw pointer".to_owned());
                        }
                        match table.pointer_form(info.cur) {
                            Some(PointerForm::Thin) => {
                                h.str("Thin");
                                hash_target(
                                    table,
                                    interns,
                                    ConvTarget::Type(*pointee, Mode::Ref),
                                    dir,
                                    depth + 1,
                                    stack,
                                    h,
                                )
                            }
                            Some(PointerForm::Slot) => {
                                h.str("Slot");
                                hash_field_child(
                                    table,
                                    interns,
                                    *pointee,
                                    Mode::Ref,
                                    dir,
                                    depth + 1,
                                    stack,
                                    h,
                                )
                            }
                            Some(PointerForm::FatSlice) => {
                                h.str("FatSlice");
                                let TypeKind::Slice(elem) = table
                                    .get(table.peeled(*pointee))
                                    .map(|d| &d.kind)
                                    .ok_or_else(|| {
                                        "a slice pointer with no pointee layout".to_owned()
                                    })?
                                else {
                                    return Err("a slice pointer to a non-slice".to_owned());
                                };
                                hash_field_child(
                                    table,
                                    interns,
                                    *elem,
                                    Mode::Ref,
                                    dir,
                                    depth + 1,
                                    stack,
                                    h,
                                )
                            }
                            Some(PointerForm::FatTail) => {
                                h.str("FatTailPtr");
                                hash_target(
                                    table,
                                    interns,
                                    ConvTarget::FatTail(*pointee),
                                    dir,
                                    depth + 1,
                                    stack,
                                    h,
                                )
                            }
                            _ => Err("a `str` behind a pointer of no convertible form".to_owned()),
                        }
                    }
                    TypeKind::Tuple(elems) | TypeKind::Closure { upvars: elems, .. } => {
                        h.str(if matches!(desc.kind, TypeKind::Closure { .. }) {
                            "Closure"
                        } else {
                            "Tuple"
                        });
                        let fields = tuple_or_closure_fields(desc, elems);
                        h.u32(fields.len() as u32);
                        for f in &fields {
                            let prop = field_prop_name(interns, f.key)?;
                            h.str(&prop);
                            hash_field_child(table, interns, f.ty, mode, dir, depth + 1, stack, h)?;
                        }
                        Ok(())
                    }
                    TypeKind::Array { element, .. } => {
                        h.str("Array");
                        hash_field_child(table, interns, *element, mode, dir, depth + 1, stack, h)
                    }
                    TypeKind::Adt {
                        form: NominalForm::Struct,
                        variants,
                        ..
                    } => {
                        h.str("Struct");
                        let [variant] = variants.as_slice() else {
                            return Err("a struct with no single variant".to_owned());
                        };
                        h.u32(variant.fields.len() as u32);
                        for f in &variant.fields {
                            let prop = field_prop_name(interns, f.key)?;
                            h.str(&prop);
                            hash_field_child(table, interns, f.ty, mode, dir, depth + 1, stack, h)?;
                        }
                        Ok(())
                    }
                    TypeKind::Adt {
                        form: NominalForm::Enum,
                        variants,
                        ..
                    } => {
                        if !matches!(
                            table.enum_repr(info.cur),
                            Some(EnumRepr::Tag { fieldless: false })
                        ) {
                            return Err(
                                "a `str` in an enum whose value is not a tagged object".to_owned()
                            );
                        }
                        h.str("Enum");
                        h.u32(variants.len() as u32);
                        for variant in variants {
                            h.str(value::variant_name(interns, variant));
                            h.u32(variant.fields.len() as u32);
                            for f in &variant.fields {
                                let prop = field_prop_name(interns, f.key)?;
                                h.str(&prop);
                                hash_field_child(
                                    table,
                                    interns,
                                    f.ty,
                                    mode,
                                    dir,
                                    depth + 1,
                                    stack,
                                    h,
                                )?;
                            }
                        }
                        Ok(())
                    }
                    TypeKind::Adt {
                        form: NominalForm::Union,
                        ..
                    } => Err("a `str` inside a union".to_owned()),
                    TypeKind::Coroutine { .. } | TypeKind::CoroutineClosure { .. } => {
                        Err("a `str` inside a coroutine".to_owned())
                    }
                    _ => Err("a `str` inside a value of no convertible shape".to_owned()),
                }
            }
        }
    })();
    stack.pop();
    res
}

#[allow(clippy::too_many_arguments)] // Recursive helper mirroring `hash_target` with `(ty, mode)` unpacked.
fn hash_field_child(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    mode: Mode,
    dir: Crossing,
    depth: usize,
    stack: &mut Vec<ConvTarget>,
    h: &mut Hasher,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("a `str` nested more than {MAX_DEPTH} levels deep"));
    }
    if !reaches_str(table, ty) || table.is_zst(ty) {
        h.byte(b'-');
        return Ok(());
    }
    match leaf_kind(table, interns, ty) {
        Some(LeafKind::Str) => {
            h.byte(b's');
            Ok(())
        }
        Some(LeafKind::MaybeUninitStr) => {
            h.byte(b'u');
            Ok(())
        }
        None => hash_target(
            table,
            interns,
            ConvTarget::Type(ty, mode),
            dir,
            depth,
            stack,
            h,
        ),
    }
}

fn target_name(
    table: &TypeTable,
    interns: &Interns,
    target: ConvTarget,
    dir: Crossing,
    cache: &mut BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, String>>,
) -> Result<Option<JsName>, String> {
    if let Some(cached) = cache.get(&(target, dir)) {
        return cached.clone();
    }
    let res = (|| -> Result<Option<JsName>, String> {
        if target_is_identity(table, interns, target, &mut BTreeSet::new())? {
            return Ok(None);
        }
        let mut h = Hasher::new();
        let mut stack = Vec::new();
        hash_target(table, interns, target, dir, 0, &mut stack, &mut h)?;
        let prefix = match (dir, is_ref_target(table, interns, target)) {
            (Crossing::Out, false) => "$bstr$out$",
            (Crossing::In, false) => "$bstr$in$",
            (Crossing::Out, true) => "$bstr$out$r$",
            (Crossing::In, true) => "$bstr$in$r$",
        };
        Ok(Some(JsName::new(format!("{prefix}{:016x}", h.0))))
    })();
    cache.insert((target, dir), res.clone());
    res
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
    let target = match carrier {
        Carrier::NativeVec => ConvTarget::NativeVecCarrier(ty),
        Carrier::SliceView => ConvTarget::SliceViewCarrier(ty),
        _ => ConvTarget::Type(ty, Mode::Val),
    };
    let mut cache = BTreeMap::new();
    match target_name(table, interns, target, direction, &mut cache)? {
        Some(name) => Ok(jsast::call(
            jsast::id(name.as_str().to_owned()),
            vec![value],
        )),
        None => Ok(value),
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

fn child_spec_expr(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    cache: &mut BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, String>>,
    pending: &mut Vec<(ConvTarget, Crossing)>,
) -> Result<Option<Expr>, String> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(None);
    }
    if let Some(leaf) = leaf_kind(table, interns, ty) {
        return Ok(Some(jsast::string(match leaf {
            LeafKind::Str => "s",
            LeafKind::MaybeUninitStr => "u",
        })));
    }
    target_pair_expr(
        table,
        interns,
        ConvTarget::Type(ty, Mode::Ref),
        cache,
        pending,
    )
}

fn target_pair_expr(
    table: &TypeTable,
    interns: &Interns,
    target: ConvTarget,
    cache: &mut BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, String>>,
    pending: &mut Vec<(ConvTarget, Crossing)>,
) -> Result<Option<Expr>, String> {
    let Some(out_name) = target_name(table, interns, target, Crossing::Out, cache)? else {
        return Ok(None);
    };
    let Some(in_name) = target_name(table, interns, target, Crossing::In, cache)? else {
        return Ok(None);
    };
    pending.push((target, Crossing::Out));
    pending.push((target, Crossing::In));
    Ok(Some(jsast::array(vec![
        jsast::id(out_name.as_str().to_owned()),
        jsast::id(in_name.as_str().to_owned()),
    ])))
}

fn child_val_expr(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    read: Expr,
    dir: Crossing,
    cache: &mut BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, String>>,
    pending: &mut Vec<(ConvTarget, Crossing)>,
) -> Result<Expr, String> {
    if !reaches_str(table, ty) || table.is_zst(ty) {
        return Ok(read);
    }
    if let Some(leaf) = leaf_kind(table, interns, ty) {
        return Ok(match leaf {
            LeafKind::Str => match dir {
                Crossing::Out => host_out(read),
                Crossing::In => host_in(read),
            },
            LeafKind::MaybeUninitStr => wrap_leaf(leaf, read, dir),
        });
    }
    let target = ConvTarget::Type(ty, Mode::Val);
    match target_name(table, interns, target, dir, cache)? {
        Some(name) => {
            pending.push((target, dir));
            Ok(jsast::call(jsast::id(name.as_str().to_owned()), vec![read]))
        }
        None => Ok(read),
    }
}

fn build_decl(
    table: &TypeTable,
    interns: &Interns,
    name: &JsName,
    target: ConvTarget,
    dir: Crossing,
    cache: &mut BTreeMap<(ConvTarget, Crossing), Result<Option<JsName>, String>>,
    pending: &mut Vec<(ConvTarget, Crossing)>,
) -> Result<Stmt, String> {
    let param = "v".to_owned();
    let p = || jsast::id(param.clone());
    let ref_call = |spec: Expr| jsast::rt_call(spec_ref_fn(dir), vec![p(), spec]);
    let mut body: Vec<Stmt> = Vec::new();
    match target {
        ConvTarget::NativeVecCarrier(ty) => {
            let elem = vec_elem_ty(table, ty)
                .ok_or_else(|| "a native Vec without element type".to_owned())?;
            let buf_pair =
                target_pair_expr(table, interns, ConvTarget::ArrayBuf(elem), cache, pending)?
                    .ok_or_else(|| "a native Vec buffer with no conversion".to_owned())?;
            let spec = jsast::object_of(vec![
                ("k", jsast::string("o")),
                ("f", jsast::object(vec![("buf".to_owned(), buf_pair)])),
            ]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::SliceViewCarrier(ty) => {
            let elem = vec_elem_ty(table, ty)
                .ok_or_else(|| "a slice view without element type".to_owned())?;
            let elem_spec = child_spec_expr(table, interns, elem, cache, pending)?
                .unwrap_or_else(|| jsast::string("s"));
            let spec = jsast::object_of(vec![("k", jsast::string("slice")), ("c", elem_spec)]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::ArrayBuf(elem) => {
            let elem_spec = child_spec_expr(table, interns, elem, cache, pending)?
                .unwrap_or_else(|| jsast::string("s"));
            let spec = jsast::object_of(vec![("k", jsast::string("a")), ("c", elem_spec)]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::RawVecPtr(elem) => {
            let elem_spec = child_spec_expr(table, interns, elem, cache, pending)?
                .unwrap_or_else(|| jsast::string("s"));
            let spec = jsast::object_of(vec![("k", jsast::string("slice")), ("c", elem_spec)]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::RawVecInner(inner_ty, elem_ty) => {
            let peeled = table.peeled(inner_ty);
            let desc = table
                .get(peeled)
                .ok_or_else(|| "a type with no described layout".to_owned())?;
            let TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } = &desc.kind
            else {
                return Err("a struct with no single variant".to_owned());
            };
            let mut entries = Vec::new();
            for f in &variants[0].fields {
                let prop = field_prop_name(interns, f.key)?;
                if prop == "ptr" {
                    if let Some(pair) = target_pair_expr(
                        table,
                        interns,
                        ConvTarget::RawVecPtr(elem_ty),
                        cache,
                        pending,
                    )? {
                        entries.push((prop, pair));
                    }
                } else if let Some(spec) = child_spec_expr(table, interns, f.ty, cache, pending)? {
                    entries.push((prop, spec));
                }
            }
            let spec = jsast::object_of(vec![
                ("k", jsast::string("o")),
                ("f", jsast::object(entries)),
            ]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::FatTailHeader(peeled_pt) => {
            let pt_desc = table
                .get(peeled_pt)
                .ok_or_else(|| "an unsized tail pointee with no layout".to_owned())?;
            let tail_idx = pt_desc
                .js
                .unsized_tail
                .ok_or_else(|| "a fat-tail pointer without an unsized tail".to_owned())?
                .field;
            let mut entries = Vec::new();
            for (_, f) in header_convertible_fields(table, pt_desc, tail_idx)? {
                let prop = field_prop_name(interns, f.key)?;
                if let Some(spec) = child_spec_expr(table, interns, f.ty, cache, pending)? {
                    entries.push((prop, spec));
                }
            }
            let spec = jsast::object_of(vec![
                ("k", jsast::string("o")),
                ("f", jsast::object(entries)),
            ]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::FatTail(pointee) => {
            let (_, header, slice_elem) = fat_tail_parts(table, pointee)?;
            let hdr_expr = match header {
                Some(h) => {
                    target_pair_expr(table, interns, ConvTarget::FatTailHeader(h), cache, pending)?
                        .unwrap_or_else(jsast::null)
                }
                None => jsast::null(),
            };
            let elem_expr = match slice_elem {
                Some(e) => {
                    child_spec_expr(table, interns, e, cache, pending)?.unwrap_or_else(jsast::null)
                }
                None => jsast::null(),
            };
            let spec = jsast::object_of(vec![
                ("k", jsast::string("tail")),
                ("h", hdr_expr),
                ("c", elem_expr),
            ]);
            body.push(jsast::ret(ref_call(spec)));
        }
        ConvTarget::Type(ty, mode) => {
            let info = unwrap_layers(table, interns, ty)?;
            if info.allow_undef && mode == Mode::Val && info.raw_vec_elem.is_none() {
                body.push(jsast::if_else(
                    jsast::binary(jsast::BinOp::StrictEq, p(), jsast::undefined()),
                    vec![jsast::ret(jsast::undefined())],
                    Vec::new(),
                ));
            }
            if let Some(elem_ty) = info.raw_vec_elem {
                let desc = table
                    .get(info.cur)
                    .ok_or_else(|| "a type with no described layout".to_owned())?;
                let (inner_ty, _) = is_raw_vec(table, interns, desc)
                    .ok_or_else(|| "invalid RawVec layout".to_owned())?;
                let TypeKind::Adt { variants, .. } = &desc.kind else {
                    unreachable!()
                };
                let mut entries = Vec::new();
                for f in &variants[0].fields {
                    let prop = field_prop_name(interns, f.key)?;
                    if prop == "inner" {
                        if let Some(pair) = target_pair_expr(
                            table,
                            interns,
                            ConvTarget::RawVecInner(inner_ty, elem_ty),
                            cache,
                            pending,
                        )? {
                            entries.push((prop, pair));
                        }
                    } else if let Some(spec) =
                        child_spec_expr(table, interns, f.ty, cache, pending)?
                    {
                        entries.push((prop, spec));
                    }
                }
                let spec = jsast::object_of(vec![
                    ("k", jsast::string("o")),
                    ("f", jsast::object(entries)),
                ]);
                body.push(jsast::ret(ref_call(spec)));
            } else {
                let desc = table
                    .get(info.cur)
                    .ok_or_else(|| "a type with no described layout".to_owned())?;
                match &desc.kind {
                    TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                        match table.pointer_form(info.cur) {
                            Some(PointerForm::Thin) => {
                                let inner_target = ConvTarget::Type(*pointee, Mode::Ref);
                                let Some(inner_name) =
                                    target_name(table, interns, inner_target, dir, cache)?
                                else {
                                    return Err("unreachable thin identity".to_owned());
                                };
                                pending.push((inner_target, dir));
                                body.push(jsast::ret(jsast::call(
                                    jsast::id(inner_name.as_str().to_owned()),
                                    vec![p()],
                                )));
                            }
                            Some(PointerForm::Slot) => {
                                let c = child_spec_expr(table, interns, *pointee, cache, pending)?
                                    .unwrap_or_else(jsast::null);
                                let spec =
                                    jsast::object_of(vec![("k", jsast::string("slot")), ("c", c)]);
                                body.push(jsast::ret(ref_call(spec)));
                            }
                            Some(PointerForm::FatSlice) => {
                                let TypeKind::Slice(elem) = table
                                    .get(table.peeled(*pointee))
                                    .map(|d| &d.kind)
                                    .ok_or_else(|| {
                                        "a slice pointer with no pointee layout".to_owned()
                                    })?
                                else {
                                    return Err("a slice pointer to a non-slice".to_owned());
                                };
                                let c = child_spec_expr(table, interns, *elem, cache, pending)?
                                    .unwrap_or_else(jsast::null);
                                let spec =
                                    jsast::object_of(vec![("k", jsast::string("slice")), ("c", c)]);
                                body.push(jsast::ret(ref_call(spec)));
                            }
                            Some(PointerForm::FatTail) => {
                                let inner_target = ConvTarget::FatTail(*pointee);
                                let Some(inner_name) =
                                    target_name(table, interns, inner_target, dir, cache)?
                                else {
                                    return Err("unreachable fat-tail identity".to_owned());
                                };
                                pending.push((inner_target, dir));
                                body.push(jsast::ret(jsast::call(
                                    jsast::id(inner_name.as_str().to_owned()),
                                    vec![p()],
                                )));
                            }
                            _ => {
                                return Err(
                                    "a `str` behind a pointer of no convertible form".to_owned()
                                )
                            }
                        }
                    }
                    TypeKind::Tuple(elems) | TypeKind::Closure { upvars: elems, .. } => {
                        let fields = tuple_or_closure_fields(desc, elems);
                        if mode == Mode::Ref {
                            let mut entries = Vec::new();
                            for (i, f) in fields.iter().enumerate() {
                                if let Some(spec) =
                                    child_spec_expr(table, interns, f.ty, cache, pending)?
                                {
                                    entries.push((i.to_string(), spec));
                                }
                            }
                            let spec = jsast::object_of(vec![
                                ("k", jsast::string("o")),
                                ("f", jsast::object(entries)),
                            ]);
                            body.push(jsast::ret(ref_call(spec)));
                        } else {
                            let mut items = Vec::with_capacity(fields.len());
                            for (i, f) in fields.iter().enumerate() {
                                let read = jsast::index(p(), jsast::num(i as f64));
                                items.push(child_val_expr(
                                    table, interns, f.ty, read, dir, cache, pending,
                                )?);
                            }
                            body.push(jsast::ret(jsast::array(items)));
                        }
                    }
                    TypeKind::Array { element, .. } => {
                        if mode == Mode::Ref {
                            let c = child_spec_expr(table, interns, *element, cache, pending)?
                                .unwrap_or_else(jsast::null);
                            let spec = jsast::object_of(vec![("k", jsast::string("a")), ("c", c)]);
                            body.push(jsast::ret(ref_call(spec)));
                        } else {
                            let elem_param = "$e".to_owned();
                            let elem_id = jsast::id(elem_param.clone());
                            let converted = child_val_expr(
                                table,
                                interns,
                                *element,
                                elem_id.clone(),
                                dir,
                                cache,
                                pending,
                            )?;
                            let guarded = jsast::cond(
                                jsast::binary(jsast::BinOp::StrictEq, elem_id, jsast::undefined()),
                                jsast::undefined(),
                                converted,
                            );
                            body.push(jsast::ret(jsast::method_call(
                                p(),
                                "map",
                                vec![jsast::arrow(vec![elem_param], guarded)],
                            )));
                        }
                    }
                    TypeKind::Adt {
                        form: NominalForm::Struct,
                        variants,
                        ..
                    } => {
                        let [variant] = variants.as_slice() else {
                            return Err("a struct with no single variant".to_owned());
                        };
                        if mode == Mode::Ref {
                            let mut entries = Vec::new();
                            for f in &variant.fields {
                                if let Some(spec) =
                                    child_spec_expr(table, interns, f.ty, cache, pending)?
                                {
                                    entries.push((field_prop_name(interns, f.key)?, spec));
                                }
                            }
                            let spec = jsast::object_of(vec![
                                ("k", jsast::string("o")),
                                ("f", jsast::object(entries)),
                            ]);
                            body.push(jsast::ret(ref_call(spec)));
                        } else {
                            let mut entries = Vec::with_capacity(variant.fields.len());
                            for f in &variant.fields {
                                let key = field_prop_name(interns, f.key)?;
                                let read = jsast::member(p(), key.clone());
                                let expr = child_val_expr(
                                    table, interns, f.ty, read, dir, cache, pending,
                                )?;
                                entries.push((key, expr));
                            }
                            body.push(jsast::ret(jsast::object(entries)));
                        }
                    }
                    TypeKind::Adt {
                        form: NominalForm::Enum,
                        variants,
                        ..
                    } => {
                        if mode == Mode::Ref {
                            let mut v_entries = Vec::new();
                            for variant in variants {
                                let mut f_entries = Vec::new();
                                for f in &variant.fields {
                                    if let Some(spec) =
                                        child_spec_expr(table, interns, f.ty, cache, pending)?
                                    {
                                        f_entries.push((field_prop_name(interns, f.key)?, spec));
                                    }
                                }
                                if !f_entries.is_empty() {
                                    v_entries.push((
                                        value::variant_name(interns, variant).to_owned(),
                                        jsast::object(f_entries),
                                    ));
                                }
                            }
                            let spec = jsast::object_of(vec![
                                ("k", jsast::string("e")),
                                ("v", jsast::object(v_entries)),
                            ]);
                            body.push(jsast::ret(ref_call(spec)));
                        } else {
                            for variant in variants {
                                let has_conv = variant
                                    .fields
                                    .iter()
                                    .any(|f| !table.is_zst(f.ty) && reaches_str(table, f.ty));
                                if !has_conv {
                                    continue;
                                }
                                let mut entries = Vec::with_capacity(variant.fields.len() + 1);
                                entries.push((
                                    value::TAG.to_owned(),
                                    value::tag_literal(interns, variant),
                                ));
                                let mut any_changed = false;
                                for f in &variant.fields {
                                    let key = field_prop_name(interns, f.key)?;
                                    let read = jsast::member(p(), key.clone());
                                    let expr = child_val_expr(
                                        table,
                                        interns,
                                        f.ty,
                                        read.clone(),
                                        dir,
                                        cache,
                                        pending,
                                    )?;
                                    if expr != read {
                                        any_changed = true;
                                    }
                                    entries.push((key, expr));
                                }
                                if any_changed {
                                    body.push(jsast::if_else(
                                        jsast::binary(
                                            jsast::BinOp::StrictEq,
                                            jsast::member(p(), value::TAG),
                                            value::tag_literal(interns, variant),
                                        ),
                                        vec![jsast::ret(jsast::object(entries))],
                                        Vec::new(),
                                    ));
                                }
                            }
                            body.push(jsast::ret(p()));
                        }
                    }
                    _ => return Err("unsupported type in converter synthesis".to_owned()),
                }
            }
        }
    }
    Ok(jsast::function(name.as_str().to_owned(), vec![param], body))
}

fn make_link_item(name: JsName, decl: Stmt) -> LinkItem {
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
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: None,
        jir_decline: None,
        jir_item: None,
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
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: Default::default(),
    }
}

/// Synthesizes deduplicated `$bstr$out$...` and `$bstr$in$...` converter [`LinkItem`]s into
/// `table` (and appends them to `reachable`) for every `$bstr$` helper called by reachable items.
pub fn synthesize_converters(table: &mut BTreeMap<JsName, LinkItem>, reachable: &mut Vec<JsName>) {
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
        let mut cache = BTreeMap::new();
        let mut pending: Vec<(ConvTarget, Crossing)> = Vec::new();
        let mut expanded: BTreeSet<(ConvTarget, Crossing)> = BTreeSet::new();

        for desc in types.descriptors() {
            if needed.is_empty() {
                break;
            }
            if desc.layout.zst || !reaches_str(types, desc.id) {
                continue;
            }
            let mut candidates = [
                (ConvTarget::Type(desc.id, Mode::Val), Crossing::Out),
                (ConvTarget::Type(desc.id, Mode::Val), Crossing::In),
                (ConvTarget::Type(desc.id, Mode::Ref), Crossing::Out),
                (ConvTarget::Type(desc.id, Mode::Ref), Crossing::In),
            ]
            .to_vec();
            if vec_elem_ty(types, desc.id).is_some() {
                candidates.push((ConvTarget::NativeVecCarrier(desc.id), Crossing::Out));
                candidates.push((ConvTarget::NativeVecCarrier(desc.id), Crossing::In));
                candidates.push((ConvTarget::SliceViewCarrier(desc.id), Crossing::Out));
                candidates.push((ConvTarget::SliceViewCarrier(desc.id), Crossing::In));
            }
            for (target, dir) in candidates {
                if needed.is_empty() {
                    break;
                }
                let Ok(Some(cname)) = target_name(types, &interns, target, dir, &mut cache) else {
                    continue;
                };
                if !needed.contains(&cname) {
                    continue;
                }
                pending.push((target, dir));
                while let Some((cur_target, cur_dir)) = pending.pop() {
                    if !expanded.insert((cur_target, cur_dir)) {
                        continue;
                    }
                    let Ok(Some(cur_name)) =
                        target_name(types, &interns, cur_target, cur_dir, &mut cache)
                    else {
                        continue;
                    };
                    needed.remove(&cur_name);
                    if table.contains_key(&cur_name) {
                        continue;
                    }
                    if let Ok(decl) = build_decl(
                        types,
                        &interns,
                        &cur_name,
                        cur_target,
                        cur_dir,
                        &mut cache,
                        &mut pending,
                    ) {
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

/// `__rt.bstr_in(value)`: host text as the Rust `str` it becomes.
pub fn host_in(value: Expr) -> Expr {
    match value {
        // A literal the lowering spelled itself is already a byte string.
        Expr::Str(_) => value,
        value => jsast::rt_call("bstr_in", vec![value]),
    }
}

/// `__rt.bstr_out(value)`: a Rust `str` as the host text it encodes.
pub fn host_out(value: Expr) -> Expr {
    match value {
        Expr::Str(text) if text.is_ascii() => Expr::Str(text),
        value => jsast::rt_call("bstr_out", vec![value]),
    }
}

/// The byte-string twin of a hybrid helper, or `None` when `member` is not one.
fn twin(member: &str) -> Option<&'static str> {
    Some(match member {
        "str_bytes" => "bstr_bytes",
        "str_bytes_mut" => "bstr_bytes_mut",
        "bytes_str" => "bstr_of",
        "native_str_push" => "bstr_push",
        "native_str_pop" => "bstr_pop",
        "native_str_truncate" => "bstr_truncate",
        "native_str_is_char_boundary" => "bstr_boundary",
        "native_str_utf16_index" | "native_str_byte_index" => "bstr_index",
        "convert_host_sb" => "bstr_host_sb",
        "convert_str_block" => "bstr_block",
        "convert_sb_block" => "bstr_sb_block",
        "_sb_check" => "bstr_sb_check",
        "str_to_lowercase" => "bstr_to_lowercase",
        "str_to_uppercase" => "bstr_to_uppercase",
        _ => return None,
    })
}

/// The hybrid helpers no byte-string program may call once [`rewrite`] has run.
const HYBRID_HELPERS: &[&str] = &[
    "str_bytes",
    "str_bytes_mut",
    "bytes_str",
    "str_len",
    "native_str_wf",
    "native_str_push",
    "native_str_pop",
    "native_str_truncate",
    "native_str_is_char_boundary",
    "native_str_utf16_index",
    "native_str_byte_index",
    "convert_host_sb",
    "convert_str_block",
    "convert_sb_block",
    "_sb_check",
    "str_to_lowercase",
    "str_to_uppercase",
];

/// The first hybrid helper `js` still names, if any.
pub fn leftover_helper(js: &str) -> Option<&'static str> {
    HYBRID_HELPERS.iter().copied().find(|helper| {
        let needle = format!("__rt.{helper}");
        js.match_indices(&needle).any(|(at, _)| {
            let next = js[at + needle.len()..].chars().next();
            !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        })
    })
}

/// The runtime member a call's callee names: `__rt.<member>`.
fn rt_member(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Member(object, member) if matches!(&**object, Expr::Ident(rt) if rt == "__rt") => {
            Some(member)
        }
        _ => None,
    }
}

/// Rewrites one installed declaration from the hybrid helpers to the byte-string ones.
pub fn rewrite(stmt: &mut Stmt) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => rewrite_expr(e),
        Stmt::Let(_, init) | Stmt::Return(init) => {
            if let Some(e) = init {
                rewrite_expr(e);
            }
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    rewrite_expr(e);
                }
            }
        }
        Stmt::If(test, then, otherwise) => {
            rewrite_expr(test);
            rewrite_all(then);
            if let Some(otherwise) = otherwise {
                rewrite_all(otherwise);
            }
        }
        Stmt::Switch(disc, cases) => {
            rewrite_expr(disc);
            for case in cases {
                for test in &mut case.tests {
                    rewrite_expr(test);
                }
                rewrite_all(&mut case.body);
            }
        }
        Stmt::Labeled(_, inner) => rewrite(inner),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => rewrite_all(body),
        Stmt::FunctionDecl { body, .. } => {
            rewrite_all(body);
            if crate::opts::get().profile == Some(crate::opts::Profile::ReleaseSpeed) {
                elide_readonly_bstr_bytes(body);
                elide_ascii_only_bstr_in(body);
            }
        }
        Stmt::While(test, body) => {
            rewrite_expr(test);
            rewrite_all(body);
        }
        Stmt::TryCatch { body, handler, .. } => {
            rewrite_all(body);
            rewrite_all(handler);
        }
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => {}
    }
}

fn rewrite_all(stmts: &mut [Stmt]) {
    for stmt in stmts {
        rewrite(stmt);
    }
}

fn rewrite_expr(expr: &mut Expr) {
    match expr {
        Expr::Member(object, _) | Expr::Unary(_, object) => rewrite_expr(object),
        Expr::Index(object, index) => {
            rewrite_expr(object);
            rewrite_expr(index);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            rewrite_expr(callee);
            for arg in args.iter_mut() {
                rewrite_expr(arg);
            }
        }
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            rewrite_expr(l);
            rewrite_expr(r);
        }
        Expr::Cond(t, c, a) => {
            rewrite_expr(t);
            rewrite_expr(c);
            rewrite_expr(a);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                rewrite_expr(value);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                rewrite_expr(item);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(body)) => rewrite_expr(body),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => rewrite_all(stmts),
        _ => {}
    }
    // Children first, so a helper's argument is already a byte-string expression.
    if let Expr::Call(callee, args) = expr {
        if let Some(member) = rt_member(callee) {
            match (member, args.len()) {
                ("str_len", 1) => {
                    let string = args.pop().expect("one argument");
                    *expr = match string {
                        Expr::Str(text) => jsast::num(text.chars().count() as f64),
                        string => jsast::member(string, "length"),
                    };
                    return;
                }
                ("native_str_wf", 1) => {
                    *expr = args.pop().expect("one argument");
                    return;
                }
                (member, _) => {
                    if let Some(twin) = twin(member) {
                        **callee = jsast::member(jsast::id("__rt"), twin);
                    }
                }
            }
        }
    }
    if let Some(folded) = try_fold_bstr_slice_roundtrip(expr) {
        *expr = folded;
        return;
    }
    if let Expr::Binary(op @ (jsast::BinOp::StrictEq | jsast::BinOp::StrictNe), l, r) = expr {
        if is_ascii_str_lit(r) {
            if let Some(arg) = unwrap_bstr_in_call(l) {
                **l = arg;
                return;
            }
        }
        if is_ascii_str_lit(l) {
            if let Some(arg) = unwrap_bstr_in_call(r) {
                **r = arg;
                return;
            }
        }
        let _ = op;
    }
}

fn is_ascii_str_lit(expr: &Expr) -> bool {
    matches!(expr, Expr::Str(s) if s.bytes().all(|b| b < 128))
}

fn unwrap_bstr_in_call(expr: &Expr) -> Option<Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() == 1 && rt_member(callee) == Some("bstr_in") {
        Some(args[0].clone())
    } else {
        None
    }
}

fn unwrap_bstr_bytes_call(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() == 1 && rt_member(callee) == Some("bstr_bytes") {
        Some(&args[0])
    } else {
        None
    }
}

fn collect_fn_locals(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(n, _) | Stmt::Const(n, _) => {
                out.insert(n.clone());
            }
            Stmt::Lets(decls) => {
                for (n, _) in decls {
                    out.insert(n.clone());
                }
            }
            Stmt::If(_, a, b) => {
                collect_fn_locals(a, out);
                if let Some(b) = b {
                    collect_fn_locals(b, out);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    collect_fn_locals(&c.body, out);
                }
            }
            Stmt::Labeled(_, inner) => collect_fn_locals(std::slice::from_ref(inner.as_ref()), out),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                collect_fn_locals(b, out);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_fn_locals(body, out);
                collect_fn_locals(handler, out);
            }
            _ => {}
        }
    }
}

/// Elides `__rt.bstr_bytes(S)` for local variables whose every assignment is `__rt.bstr_bytes(...)`
/// and whose every read is strictly `.len`, `.buf[v.off]`, or `.buf[v.off + idx]`: rewrites the
/// assignment to `v = S` and reads to `v.length` / `v.charCodeAt(0)` / `v.charCodeAt(idx)`.
fn elide_readonly_bstr_bytes(body: &mut [Stmt]) {
    #[derive(Default)]
    struct Usage {
        bstr_writes: usize,
        safe_reads: usize,
        bad: bool,
    }

    fn is_var_member<'a>(expr: &'a Expr, prop: &str) -> Option<&'a str> {
        match expr {
            Expr::Member(obj, p) if p == prop => match obj.as_ref() {
                Expr::Ident(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn match_var_index(expr: &Expr) -> Option<(&str, Option<&Expr>)> {
        let Expr::Index(obj, idx) = expr else {
            return None;
        };
        let name = is_var_member(obj, "buf")?;
        if is_var_member(idx, "off") == Some(name) {
            return Some((name, None));
        }
        if let Expr::Binary(jsast::BinOp::Add, l, r) = idx.as_ref() {
            if is_var_member(l, "off") == Some(name) {
                return Some((name, Some(r.as_ref())));
            }
            if is_var_member(r, "off") == Some(name) {
                return Some((name, Some(l.as_ref())));
            }
        }
        None
    }

    fn mark_all_bad(expr: &Expr, locals: &BTreeSet<String>, usage: &mut BTreeMap<String, Usage>) {
        match expr {
            Expr::Ident(n) => {
                if locals.contains(n) {
                    usage.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => mark_all_bad(o, locals, usage),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                mark_all_bad(o, locals, usage);
                mark_all_bad(i, locals, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                mark_all_bad(c, locals, usage);
                for a in args {
                    mark_all_bad(a, locals, usage);
                }
            }
            Expr::Cond(c, t, e) => {
                mark_all_bad(c, locals, usage);
                mark_all_bad(t, locals, usage);
                mark_all_bad(e, locals, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    mark_all_bad(v, locals, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    mark_all_bad(x, locals, usage);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => mark_all_bad(b, locals, usage),
            Expr::Arrow(_, ArrowBody::Block(s)) => scan_stmts(s, locals, true, usage),
            _ => {}
        }
    }

    fn scan_expr(
        expr: &Expr,
        locals: &BTreeSet<String>,
        in_closure: bool,
        usage: &mut BTreeMap<String, Usage>,
    ) {
        if in_closure {
            mark_all_bad(expr, locals, usage);
            return;
        }
        if let Some((name, idx_opt)) = match_var_index(expr) {
            if locals.contains(name) {
                usage.entry(name.to_owned()).or_default().safe_reads += 1;
                if let Some(idx) = idx_opt {
                    scan_expr(idx, locals, false, usage);
                }
                return;
            }
        }
        if let Some(name) = is_var_member(expr, "len") {
            if locals.contains(name) {
                usage.entry(name.to_owned()).or_default().safe_reads += 1;
                return;
            }
        }
        match expr {
            Expr::Ident(n) => {
                if locals.contains(n) {
                    usage.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if locals.contains(n) {
                        if let Some(arg) = unwrap_bstr_bytes_call(rhs) {
                            usage.entry(n.clone()).or_default().bstr_writes += 1;
                            scan_expr(arg, locals, false, usage);
                            return;
                        }
                        usage.entry(n.clone()).or_default().bad = true;
                        scan_expr(rhs, locals, false, usage);
                        return;
                    }
                }
                // Any write through `b.buf[...]` disqualifies `b`, while scanning indices in `lhs`
                // for other read-only variables (`other.buf[other.off + i]` could only be on RHS).
                mark_all_bad(lhs, locals, usage);
                scan_expr(rhs, locals, false, usage);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                mark_all_bad(lhs, locals, usage);
                scan_expr(rhs, locals, false, usage);
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => scan_expr(o, locals, false, usage),
            Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                scan_expr(o, locals, false, usage);
                scan_expr(i, locals, false, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, false, usage);
                for a in args {
                    scan_expr(a, locals, false, usage);
                }
            }
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, false, usage);
                scan_expr(t, locals, false, usage);
                scan_expr(e, locals, false, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, false, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    scan_expr(x, locals, false, usage);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => mark_all_bad(b, locals, usage),
            Expr::Arrow(_, ArrowBody::Block(s)) => scan_stmts(s, locals, true, usage),
            _ => {}
        }
    }

    fn scan_stmts(
        stmts: &[Stmt],
        locals: &BTreeSet<String>,
        in_closure: bool,
        usage: &mut BTreeMap<String, Usage>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init)
                    if !in_closure && locals.contains(n) =>
                {
                    if let Some(arg) = unwrap_bstr_bytes_call(init) {
                        usage.entry(n.clone()).or_default().bstr_writes += 1;
                        scan_expr(arg, locals, false, usage);
                    } else {
                        usage.entry(n.clone()).or_default().bad = true;
                        scan_expr(init, locals, false, usage);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if !in_closure && locals.contains(n) {
                                if let Some(arg) = unwrap_bstr_bytes_call(init) {
                                    usage.entry(n.clone()).or_default().bstr_writes += 1;
                                    scan_expr(arg, locals, false, usage);
                                    continue;
                                }
                                usage.entry(n.clone()).or_default().bad = true;
                            }
                            scan_expr(init, locals, in_closure, usage);
                        }
                    }
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => scan_expr(e, locals, in_closure, usage),
                Stmt::If(t, a, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(a, locals, in_closure, usage);
                    if let Some(b) = b {
                        scan_stmts(b, locals, in_closure, usage);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_expr(d, locals, in_closure, usage);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, in_closure, usage);
                        }
                        scan_stmts(&c.body, locals, in_closure, usage);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        locals,
                        in_closure,
                        usage,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::While(t, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        usage.entry(binding.clone()).or_default().bad = true;
                    }
                    scan_stmts(body, locals, in_closure, usage);
                    scan_stmts(handler, locals, in_closure, usage);
                }
                Stmt::FunctionDecl { body, .. } => scan_stmts(body, locals, true, usage),
                _ => {}
            }
        }
    }

    fn rewrite_expr_elided(expr: &mut Expr, elided: &BTreeSet<String>) {
        // Rewrite `v.buf[v.off]` -> `v.charCodeAt(0)` and `v.buf[v.off + idx]` -> `v.charCodeAt(idx)`.
        if let Some((name, idx_opt)) = match_var_index(expr) {
            if elided.contains(name) {
                let name = name.to_owned();
                let mut idx_expr = idx_opt.cloned().unwrap_or_else(|| jsast::num(0));
                rewrite_expr_elided(&mut idx_expr, elided);
                *expr = jsast::method_call(jsast::id(name), "charCodeAt", vec![idx_expr]);
                return;
            }
        }
        if let Some(name) = is_var_member(expr, "len") {
            if elided.contains(name) {
                *expr = jsast::member(jsast::id(name.to_owned()), "length");
                return;
            }
        }
        match expr {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if elided.contains(n) {
                        if let Some(arg) = unwrap_bstr_bytes_call(rhs).cloned() {
                            let mut arg = arg;
                            rewrite_expr_elided(&mut arg, elided);
                            **rhs = arg;
                            return;
                        }
                    }
                }
                rewrite_expr_elided(lhs, elided);
                rewrite_expr_elided(rhs, elided);
            }
            Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                rewrite_expr_elided(l, elided);
                rewrite_expr_elided(r, elided);
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => rewrite_expr_elided(o, elided),
            Expr::Call(c, args) | Expr::New(c, args) => {
                rewrite_expr_elided(c, elided);
                for a in args {
                    rewrite_expr_elided(a, elided);
                }
            }
            Expr::Cond(c, t, e) => {
                rewrite_expr_elided(c, elided);
                rewrite_expr_elided(t, elided);
                rewrite_expr_elided(e, elided);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    rewrite_expr_elided(v, elided);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    rewrite_expr_elided(x, elided);
                }
            }
            _ => {}
        }
    }

    fn rewrite_stmts_elided(stmts: &mut [Stmt], elided: &BTreeSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) if elided.contains(n) => {
                    if let Some(arg) = unwrap_bstr_bytes_call(init).cloned() {
                        let mut arg = arg;
                        rewrite_expr_elided(&mut arg, elided);
                        *init = arg;
                        continue;
                    }
                    rewrite_expr_elided(init, elided);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if elided.contains(n) {
                                if let Some(arg) = unwrap_bstr_bytes_call(init).cloned() {
                                    let mut arg = arg;
                                    rewrite_expr_elided(&mut arg, elided);
                                    *init = arg;
                                    continue;
                                }
                            }
                            rewrite_expr_elided(init, elided);
                        }
                    }
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => rewrite_expr_elided(e, elided),
                Stmt::If(t, a, b) => {
                    rewrite_expr_elided(t, elided);
                    rewrite_stmts_elided(a, elided);
                    if let Some(b) = b {
                        rewrite_stmts_elided(b, elided);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rewrite_expr_elided(d, elided);
                    for c in cases {
                        for t in &mut c.tests {
                            rewrite_expr_elided(t, elided);
                        }
                        rewrite_stmts_elided(&mut c.body, elided);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts_elided(std::slice::from_mut(inner.as_mut()), elided);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => rewrite_stmts_elided(b, elided),
                Stmt::While(t, b) => {
                    rewrite_expr_elided(t, elided);
                    rewrite_stmts_elided(b, elided);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts_elided(body, elided);
                    rewrite_stmts_elided(handler, elided);
                }
                _ => {}
            }
        }
    }

    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    if locals.is_empty() {
        return;
    }
    let mut usage = BTreeMap::new();
    scan_stmts(body, &locals, false, &mut usage);
    let elided: BTreeSet<String> = usage
        .into_iter()
        .filter(|(_, u)| !u.bad && u.bstr_writes >= 1 && u.safe_reads >= 1)
        .map(|(n, _)| n)
        .collect();
    if !elided.is_empty() {
        rewrite_stmts_elided(body, &elided);
    }
}

/// Elides `__rt.bstr_in(X)` when the bound local (and any pure copy aliases of it) is ONLY
/// compared via `===`/`!==`/`switch` against all-ASCII string literals (`c < 128`) or
/// single-assignment const ASCII string locals (`F = "ASCII"` / `F = ["ASCII"][0]`).
fn elide_ascii_only_bstr_in(body: &mut [Stmt]) {
    fn is_const_ascii_init(expr: &Expr) -> bool {
        if is_ascii_str_lit(expr) {
            return true;
        }
        // Matches `["ASCII"][0]` (unboxed single-element `&str` slot constant).
        if let Expr::Index(arr, idx) = expr {
            if matches!(idx.as_ref(), Expr::Num(n) if *n == 0.0) {
                if let Expr::Array(items) = arr.as_ref() {
                    return items.len() == 1 && is_ascii_str_lit(&items[0]);
                }
            }
        }
        false
    }

    fn collect_const_ascii_locals(
        stmts: &[Stmt],
        locals: &BTreeSet<String>,
        writes: &mut BTreeMap<String, (usize, bool)>,
    ) {
        fn scan_e(
            e: &Expr,
            locals: &BTreeSet<String>,
            writes: &mut BTreeMap<String, (usize, bool)>,
        ) {
            match e {
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if locals.contains(n) {
                            let entry = writes.entry(n.clone()).or_insert((0, true));
                            entry.0 += 1;
                            if !is_const_ascii_init(rhs) {
                                entry.1 = false;
                            }
                            scan_e(rhs, locals, writes);
                            return;
                        }
                    }
                    scan_e(lhs, locals, writes);
                    scan_e(rhs, locals, writes);
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if locals.contains(n) {
                            let entry = writes.entry(n.clone()).or_insert((0, true));
                            entry.0 += 1;
                            entry.1 = false;
                        }
                    }
                    scan_e(lhs, locals, writes);
                    scan_e(rhs, locals, writes);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => scan_e(o, locals, writes),
                Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                    scan_e(o, locals, writes);
                    scan_e(i, locals, writes);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    scan_e(c, locals, writes);
                    for a in args {
                        scan_e(a, locals, writes);
                    }
                }
                Expr::Cond(c, t, f) => {
                    scan_e(c, locals, writes);
                    scan_e(t, locals, writes);
                    scan_e(f, locals, writes);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        scan_e(v, locals, writes);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        scan_e(x, locals, writes);
                    }
                }
                Expr::Arrow(_, ArrowBody::Expr(b)) => scan_e(b, locals, writes),
                Expr::Arrow(_, ArrowBody::Block(s)) => {
                    collect_const_ascii_locals(s, locals, writes)
                }
                _ => {}
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                    if locals.contains(n) {
                        let entry = writes.entry(n.clone()).or_insert((0, true));
                        entry.0 += 1;
                        if !is_const_ascii_init(init) {
                            entry.1 = false;
                        }
                    }
                    scan_e(init, locals, writes);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if locals.contains(n) {
                                let entry = writes.entry(n.clone()).or_insert((0, true));
                                entry.0 += 1;
                                if !is_const_ascii_init(init) {
                                    entry.1 = false;
                                }
                            }
                            scan_e(init, locals, writes);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    scan_e(e, locals, writes);
                }
                Stmt::If(t, a, b) => {
                    scan_e(t, locals, writes);
                    collect_const_ascii_locals(a, locals, writes);
                    if let Some(b) = b {
                        collect_const_ascii_locals(b, locals, writes);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_e(d, locals, writes);
                    for c in cases {
                        for t in &c.tests {
                            scan_e(t, locals, writes);
                        }
                        collect_const_ascii_locals(&c.body, locals, writes);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    collect_const_ascii_locals(
                        std::slice::from_ref(inner.as_ref()),
                        locals,
                        writes,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                    collect_const_ascii_locals(b, locals, writes);
                }
                Stmt::While(t, b) => {
                    scan_e(t, locals, writes);
                    collect_const_ascii_locals(b, locals, writes);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        writes.insert(binding.clone(), (2, false));
                    }
                    collect_const_ascii_locals(body, locals, writes);
                    collect_const_ascii_locals(handler, locals, writes);
                }
                _ => {}
            }
        }
    }

    fn is_ascii_expr(expr: &Expr, const_ascii: &BTreeSet<String>) -> bool {
        is_ascii_str_lit(expr) || matches!(expr, Expr::Ident(n) if const_ascii.contains(n))
    }

    #[derive(Default)]
    struct Info {
        bstr_in_writes: usize,
        copy_from: Vec<String>,
        bad: bool,
    }

    fn mark_all_bad(expr: &Expr, locals: &BTreeSet<String>, info: &mut BTreeMap<String, Info>) {
        match expr {
            Expr::Ident(n) => {
                if locals.contains(n) {
                    info.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => mark_all_bad(o, locals, info),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                mark_all_bad(o, locals, info);
                mark_all_bad(i, locals, info);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                mark_all_bad(c, locals, info);
                for a in args {
                    mark_all_bad(a, locals, info);
                }
            }
            Expr::Cond(c, t, e) => {
                mark_all_bad(c, locals, info);
                mark_all_bad(t, locals, info);
                mark_all_bad(e, locals, info);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    mark_all_bad(v, locals, info);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    mark_all_bad(x, locals, info);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => mark_all_bad(b, locals, info),
            Expr::Arrow(_, ArrowBody::Block(s)) => {
                scan_stmts(s, locals, &BTreeSet::new(), true, info)
            }
            _ => {}
        }
    }

    fn scan_expr(
        expr: &Expr,
        locals: &BTreeSet<String>,
        const_ascii: &BTreeSet<String>,
        in_closure: bool,
        info: &mut BTreeMap<String, Info>,
    ) {
        if in_closure {
            mark_all_bad(expr, locals, info);
            return;
        }
        if let Expr::Binary(jsast::BinOp::StrictEq | jsast::BinOp::StrictNe, l, r) = expr {
            if let Expr::Ident(n) = l.as_ref() {
                if locals.contains(n) && is_ascii_expr(r, const_ascii) {
                    return;
                }
            }
            if let Expr::Ident(n) = r.as_ref() {
                if locals.contains(n) && is_ascii_expr(l, const_ascii) {
                    return;
                }
            }
        }
        match expr {
            Expr::Ident(n) => {
                if locals.contains(n) {
                    info.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if locals.contains(n) {
                        if let Some(arg) = unwrap_bstr_in_call(rhs) {
                            info.entry(n.clone()).or_default().bstr_in_writes += 1;
                            scan_expr(&arg, locals, const_ascii, false, info);
                            return;
                        }
                        if let Expr::Ident(src) = rhs.as_ref() {
                            if locals.contains(src) {
                                info.entry(n.clone())
                                    .or_default()
                                    .copy_from
                                    .push(src.clone());
                                return;
                            }
                        }
                        info.entry(n.clone()).or_default().bad = true;
                        scan_expr(rhs, locals, const_ascii, false, info);
                        return;
                    }
                }
                mark_all_bad(lhs, locals, info);
                scan_expr(rhs, locals, const_ascii, false, info);
            }
            Expr::CompoundAssign(_, l, r) => {
                mark_all_bad(l, locals, info);
                scan_expr(r, locals, const_ascii, false, info);
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => {
                scan_expr(o, locals, const_ascii, false, info)
            }
            Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                scan_expr(o, locals, const_ascii, false, info);
                scan_expr(i, locals, const_ascii, false, info);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, const_ascii, false, info);
                for a in args {
                    scan_expr(a, locals, const_ascii, false, info);
                }
            }
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, const_ascii, false, info);
                scan_expr(t, locals, const_ascii, false, info);
                scan_expr(e, locals, const_ascii, false, info);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, const_ascii, false, info);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    scan_expr(x, locals, const_ascii, false, info);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => mark_all_bad(b, locals, info),
            Expr::Arrow(_, ArrowBody::Block(s)) => scan_stmts(s, locals, const_ascii, true, info),
            _ => {}
        }
    }

    fn scan_stmts(
        stmts: &[Stmt],
        locals: &BTreeSet<String>,
        const_ascii: &BTreeSet<String>,
        in_closure: bool,
        info: &mut BTreeMap<String, Info>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init)
                    if !in_closure && locals.contains(n) =>
                {
                    if let Some(arg) = unwrap_bstr_in_call(init) {
                        info.entry(n.clone()).or_default().bstr_in_writes += 1;
                        scan_expr(&arg, locals, const_ascii, false, info);
                    } else if let Expr::Ident(src) = init {
                        if locals.contains(src) {
                            info.entry(n.clone())
                                .or_default()
                                .copy_from
                                .push(src.clone());
                        } else {
                            info.entry(n.clone()).or_default().bad = true;
                        }
                    } else {
                        info.entry(n.clone()).or_default().bad = true;
                        scan_expr(init, locals, const_ascii, false, info);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if !in_closure && locals.contains(n) {
                                if let Some(arg) = unwrap_bstr_in_call(init) {
                                    info.entry(n.clone()).or_default().bstr_in_writes += 1;
                                    scan_expr(&arg, locals, const_ascii, false, info);
                                    continue;
                                }
                                if let Expr::Ident(src) = init {
                                    if locals.contains(src) {
                                        info.entry(n.clone())
                                            .or_default()
                                            .copy_from
                                            .push(src.clone());
                                        continue;
                                    }
                                }
                                info.entry(n.clone()).or_default().bad = true;
                            }
                            scan_expr(init, locals, const_ascii, in_closure, info);
                        }
                    }
                }
                Stmt::Switch(d, cases) => {
                    let all_ascii_cases = cases
                        .iter()
                        .all(|c| c.tests.iter().all(|t| is_ascii_expr(t, const_ascii)));
                    if !in_closure && all_ascii_cases {
                        if let Expr::Ident(n) = d {
                            if locals.contains(n) {
                                for c in cases {
                                    scan_stmts(&c.body, locals, const_ascii, false, info);
                                }
                                continue;
                            }
                        }
                    }
                    scan_expr(d, locals, const_ascii, in_closure, info);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, const_ascii, in_closure, info);
                        }
                        scan_stmts(&c.body, locals, const_ascii, in_closure, info);
                    }
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => scan_expr(e, locals, const_ascii, in_closure, info),
                Stmt::If(t, a, b) => {
                    scan_expr(t, locals, const_ascii, in_closure, info);
                    scan_stmts(a, locals, const_ascii, in_closure, info);
                    if let Some(b) = b {
                        scan_stmts(b, locals, const_ascii, in_closure, info);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        locals,
                        const_ascii,
                        in_closure,
                        info,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    scan_stmts(b, locals, const_ascii, in_closure, info);
                }
                Stmt::While(t, b) => {
                    scan_expr(t, locals, const_ascii, in_closure, info);
                    scan_stmts(b, locals, const_ascii, in_closure, info);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        info.entry(binding.clone()).or_default().bad = true;
                    }
                    scan_stmts(body, locals, const_ascii, in_closure, info);
                    scan_stmts(handler, locals, const_ascii, in_closure, info);
                }
                Stmt::FunctionDecl { body, .. } => {
                    scan_stmts(body, locals, const_ascii, true, info)
                }
                _ => {}
            }
        }
    }

    fn rewrite_bstr_in_stmts(stmts: &mut [Stmt], elided: &BTreeSet<String>) {
        fn rw_expr(e: &mut Expr, elided: &BTreeSet<String>) {
            match e {
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if elided.contains(n) {
                            if let Some(arg) = unwrap_bstr_in_call(rhs) {
                                **rhs = arg;
                                rw_expr(rhs, elided);
                                return;
                            }
                        }
                    }
                    rw_expr(lhs, elided);
                    rw_expr(rhs, elided);
                }
                Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                    rw_expr(l, elided);
                    rw_expr(r, elided);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => rw_expr(o, elided),
                Expr::Call(c, args) | Expr::New(c, args) => {
                    rw_expr(c, elided);
                    for a in args {
                        rw_expr(a, elided);
                    }
                }
                Expr::Cond(c, t, e) => {
                    rw_expr(c, elided);
                    rw_expr(t, elided);
                    rw_expr(e, elided);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        rw_expr(v, elided);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        rw_expr(x, elided);
                    }
                }
                _ => {}
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) if elided.contains(n) => {
                    if let Some(arg) = unwrap_bstr_in_call(init) {
                        *init = arg;
                    }
                    rw_expr(init, elided);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if elided.contains(n) {
                                if let Some(arg) = unwrap_bstr_in_call(init) {
                                    *init = arg;
                                }
                            }
                            rw_expr(init, elided);
                        }
                    }
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => rw_expr(e, elided),
                Stmt::If(t, a, b) => {
                    rw_expr(t, elided);
                    rewrite_bstr_in_stmts(a, elided);
                    if let Some(b) = b {
                        rewrite_bstr_in_stmts(b, elided);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rw_expr(d, elided);
                    for c in cases {
                        for t in &mut c.tests {
                            rw_expr(t, elided);
                        }
                        rewrite_bstr_in_stmts(&mut c.body, elided);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_bstr_in_stmts(std::slice::from_mut(inner.as_mut()), elided);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => rewrite_bstr_in_stmts(b, elided),
                Stmt::While(t, b) => {
                    rw_expr(t, elided);
                    rewrite_bstr_in_stmts(b, elided);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_bstr_in_stmts(body, elided);
                    rewrite_bstr_in_stmts(handler, elided);
                }
                _ => {}
            }
        }
    }

    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    if locals.is_empty() {
        return;
    }
    let mut ascii_writes = BTreeMap::new();
    collect_const_ascii_locals(body, &locals, &mut ascii_writes);
    let const_ascii: BTreeSet<String> = ascii_writes
        .into_iter()
        .filter(|(_, (count, ok))| *count == 1 && *ok)
        .map(|(n, _)| n)
        .collect();
    let mut info = BTreeMap::new();
    scan_stmts(body, &locals, &const_ascii, false, &mut info);
    // Propagate `bad` backwards through `dst = src` copies.
    loop {
        let mut changed = false;
        let bad_set: BTreeSet<String> = info
            .iter()
            .filter(|(_, v)| v.bad)
            .map(|(k, _)| k.clone())
            .collect();
        let mut to_mark = Vec::new();
        for (dst, v) in &info {
            if bad_set.contains(dst) {
                for src in &v.copy_from {
                    if !bad_set.contains(src) {
                        to_mark.push(src.clone());
                    }
                }
            }
        }
        for src in to_mark {
            let entry = info.entry(src).or_default();
            if !entry.bad {
                entry.bad = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let elided: BTreeSet<String> = info
        .into_iter()
        .filter(|(_, v)| !v.bad && v.bstr_in_writes >= 1)
        .map(|(k, _)| k)
        .collect();
    if !elided.is_empty() {
        rewrite_bstr_in_stmts(body, &elided);
    }
}

/// Folds `(h = __rt.{sfp,slice_from_parts}(offset(bstr_bytes(S), D), L), __rt.bstr_of(h.buf, h.off, h.len))`
/// (produced by `core::str::traits::SliceIndex::get_unchecked`, `str::split`, `str::lines`, etc.)
/// directly into `S.slice(D, E)` / `S.slice(D, D + L)` with zero byte-array allocation.
fn try_fold_bstr_slice_roundtrip(expr: &Expr) -> Option<Expr> {
    let Expr::Seq(items) = expr else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let Expr::Assign(h_lhs, sfp_rhs) = &items[0] else {
        return None;
    };
    let Expr::Ident(h_name) = h_lhs.as_ref() else {
        return None;
    };
    if !h_name.starts_with("$t") {
        return None;
    }
    // Final expression: `__rt.bstr_of(h.buf, h.off, h.len)`
    let Expr::Call(bstr_of_callee, bstr_of_args) = &items[1] else {
        return None;
    };
    if rt_member(bstr_of_callee) != Some("bstr_of") || bstr_of_args.len() != 3 {
        return None;
    }
    let is_h_member = |e: &Expr, prop: &str| matches!(e, Expr::Member(o, p) if p == prop && matches!(o.as_ref(), Expr::Ident(n) if n == h_name));
    if !is_h_member(&bstr_of_args[0], "buf")
        || !is_h_member(&bstr_of_args[1], "off")
        || !is_h_member(&bstr_of_args[2], "len")
    {
        return None;
    }
    // `sfp_rhs`: `__rt.sfp(ptr_expr, len_expr)` or `__rt.slice_from_parts(ptr_expr, len_expr)`
    let Expr::Call(sfp_callee, sfp_args) = sfp_rhs.as_ref() else {
        return None;
    };
    if !matches!(rt_member(sfp_callee), Some("sfp" | "slice_from_parts")) || sfp_args.len() != 2 {
        return None;
    }
    let (s_expr, j_temp, start_expr) = match_bstr_bytes_ptr(&sfp_args[0])?;
    let len_expr = &sfp_args[1];

    let is_simple_val = |e: &Expr| {
        matches!(
            e,
            Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Str(_)
        )
    };
    let mut seq = Vec::new();
    let recv = if is_simple_val(s_expr) {
        s_expr.clone()
    } else {
        let j = j_temp?;
        seq.push(jsast::assign(jsast::id(j), s_expr.clone()));
        jsast::id(j)
    };

    let call = match start_expr {
        None => jsast::method_call(recv, "slice", vec![jsast::num(0.0), len_expr.clone()]),
        Some(start) => {
            if let Some(end) = match_end_minus_start(len_expr, start) {
                if is_simple_val(start) {
                    jsast::method_call(recv, "slice", vec![start.clone(), end.clone()])
                } else {
                    seq.push(jsast::assign(jsast::id(h_name), start.clone()));
                    jsast::method_call(recv, "slice", vec![jsast::id(h_name), end.clone()])
                }
            } else if is_simple_val(start) {
                jsast::method_call(
                    recv,
                    "slice",
                    vec![
                        start.clone(),
                        jsast::binary(jsast::BinOp::Add, start.clone(), len_expr.clone()),
                    ],
                )
            } else {
                seq.push(jsast::assign(jsast::id(h_name), start.clone()));
                jsast::method_call(
                    recv,
                    "slice",
                    vec![
                        jsast::id(h_name),
                        jsast::binary(jsast::BinOp::Add, jsast::id(h_name), len_expr.clone()),
                    ],
                )
            }
        }
    };
    if seq.is_empty() {
        Some(call)
    } else {
        seq.push(call);
        Some(Expr::Seq(seq))
    }
}

/// Extracts `(s_expr, optional_temp_name, optional_offset)` from a pointer built over
/// `__rt.bstr_bytes(s_expr)` (with or without a `+ start` offset).
fn match_bstr_bytes_ptr(expr: &Expr) -> Option<(&Expr, Option<&str>, Option<&Expr>)> {
    // Pattern A: `(g = base, D === 0 ? g : { buf: g.buf, off: g.off + D, ... })`
    // or `(g = base, { buf: g.buf, off: g.off + D, ... })`
    if let Expr::Seq(items) = expr {
        if items.len() == 2 {
            if let Expr::Assign(g_lhs, base_rhs) = &items[0] {
                if let Expr::Ident(g_name) = g_lhs.as_ref() {
                    if g_name.starts_with("$t") {
                        // Direct `{ buf: j.buf, off: j.off }` over `j = __rt.bstr_bytes(S)`
                        if let Some(s) = match_direct_bstr_bytes(base_rhs) {
                            if is_buf_off_copy(&items[1], g_name) {
                                return Some((s, Some(g_name), None));
                            }
                        }
                        // Offset over `g = base`
                        if let Some((s, j_temp)) = match_base_bstr_bytes(base_rhs) {
                            if let Some(start) = match_offset_of(&items[1], g_name) {
                                return Some((s, j_temp.or(Some(g_name)), Some(start)));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some((s, j_temp)) = match_base_bstr_bytes(expr) {
        return Some((s, j_temp, None));
    }
    None
}

fn match_direct_bstr_bytes(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if rt_member(callee) == Some("bstr_bytes") && args.len() == 1 {
        Some(&args[0])
    } else {
        None
    }
}

fn is_buf_off_copy(expr: &Expr, var: &str) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    entries.len() == 2
        && entries[0].0 == "buf"
        && matches!(&entries[0].1, Expr::Member(o, p) if p == "buf" && matches!(o.as_ref(), Expr::Ident(n) if n == var))
        && entries[1].0 == "off"
        && matches!(&entries[1].1, Expr::Member(o, p) if p == "off" && matches!(o.as_ref(), Expr::Ident(n) if n == var))
}

fn match_base_bstr_bytes(expr: &Expr) -> Option<(&Expr, Option<&str>)> {
    if let Some(s) = match_direct_bstr_bytes(expr) {
        return Some((s, None));
    }
    let Expr::Seq(items) = expr else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let Expr::Assign(j_lhs, rhs) = &items[0] else {
        return None;
    };
    let Expr::Ident(j_name) = j_lhs.as_ref() else {
        return None;
    };
    if !j_name.starts_with("$t") {
        return None;
    }
    let s = match_direct_bstr_bytes(rhs)?;
    if is_buf_off_copy(&items[1], j_name) {
        Some((s, Some(j_name)))
    } else {
        None
    }
}

fn match_offset_of<'a>(expr: &'a Expr, base: &str) -> Option<&'a Expr> {
    // Either `D === 0 ? base : { buf: base.buf, off: base.off + D, ... }`
    // or `{ buf: base.buf, off: base.off + D, ... }`.
    let obj = match expr {
        Expr::Cond(cond, then_e, else_e) => {
            if !matches!(then_e.as_ref(), Expr::Ident(n) if n == base) {
                return None;
            }
            let Expr::Binary(jsast::BinOp::StrictEq, l, r) = cond.as_ref() else {
                return None;
            };
            if !matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) {
                return None;
            }
            let obj_start = match_offset_obj(else_e, base)?;
            return (l.as_ref() == obj_start).then_some(obj_start);
        }
        other => other,
    };
    match_offset_obj(obj, base)
}

fn match_offset_obj<'a>(expr: &'a Expr, base: &str) -> Option<&'a Expr> {
    let Expr::Object(entries) = expr else {
        return None;
    };
    if entries.len() < 2 || entries.len() > 3 {
        return None;
    }
    if entries[0].0 != "buf"
        || !matches!(&entries[0].1, Expr::Member(o, p) if p == "buf" && matches!(o.as_ref(), Expr::Ident(n) if n == base))
    {
        return None;
    }
    if entries[1].0 != "off" {
        return None;
    }
    let Expr::Binary(jsast::BinOp::Add, l, r) = &entries[1].1 else {
        return None;
    };
    if !matches!(l.as_ref(), Expr::Member(o, p) if p == "off" && matches!(o.as_ref(), Expr::Ident(n) if n == base))
    {
        return None;
    }
    if entries.len() == 3
        && (entries[2].0 != "sc"
            || !matches!(&entries[2].1, Expr::Member(o, p) if p == "sc" && matches!(o.as_ref(), Expr::Ident(n) if n == base)))
    {
        return None;
    }
    Some(r.as_ref())
}

fn match_end_minus_start<'a>(len_expr: &'a Expr, start: &Expr) -> Option<&'a Expr> {
    let inner = match len_expr {
        Expr::Binary(jsast::BinOp::UShr, l, r) if matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) => {
            l.as_ref()
        }
        other => other,
    };
    let Expr::Binary(jsast::BinOp::Sub, end, sub_rhs) = inner else {
        return None;
    };
    (sub_rhs.as_ref() == start).then_some(end.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::types::VariantDescriptor;

    fn rt(member: &str, args: Vec<Expr>) -> Expr {
        jsast::rt_call(member, args)
    }

    #[test]
    fn a_literal_is_the_string_of_its_bytes() {
        assert_eq!(byte_string("aé😀"), "a\u{c3}\u{a9}\u{f0}\u{9f}\u{98}\u{80}");
    }

    #[test]
    fn the_rewrite_swaps_helpers_and_folds_lengths() {
        let mut stmt = jsast::ret(jsast::binary(
            jsast::BinOp::Add,
            rt(
                "str_len",
                vec![rt(
                    "bytes_str",
                    vec![jsast::id("b"), jsast::num(0), jsast::num(2)],
                )],
            ),
            rt("str_len", vec![rt("native_str_wf", vec![jsast::id("s")])]),
        ));
        rewrite(&mut stmt);
        let printed = jsast::program_to_string(std::slice::from_ref(&stmt));
        assert!(
            printed.contains("__rt.bstr_of(b, 0, 2).length"),
            "{printed}"
        );
        assert!(printed.contains("s.length"), "{printed}");
        assert_eq!(leftover_helper(&printed), None, "{printed}");
    }

    #[test]
    fn leftovers_are_found_by_whole_name() {
        assert_eq!(leftover_helper("__rt.str_bytes(s)"), Some("str_bytes"));
        assert_eq!(
            leftover_helper("__rt.str_bytes_mut(s)"),
            Some("str_bytes_mut")
        );
        assert_eq!(leftover_helper("__rt.bstr_bytes(s); __rt.str_lenx"), None);
    }

    fn make_desc(id: u128, kind: TypeKind, zst: bool) -> TypeDescriptor {
        use crate::jir::types::{JsFacts, LayoutFacts, TypeMeta};
        let mut d = TypeDescriptor {
            id: TypeId(id),
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(if zst { 0 } else { 8 }),
                unsized_prefix: None,
                align: Some(1),
                sized: true,
                zst,
                one_zst: zst,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: TypeId(id),
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        if zst {
            d.js.is_indirect = true;
        }
        d
    }

    #[test]
    fn reaches_str_walks_adt_args_closures_and_coroutines_while_excluding_string() {
        use crate::jir::semantic::DefinitionId;
        let mut interns = Interns::new();
        let def = DefinitionId {
            crate_id: 1,
            local_hash: 1,
        };
        let str_id = TypeId(1);
        let ref_str_id = TypeId(2);
        let u8_id = TypeId(3);
        let phantom_str_id = TypeId(4);
        let string_raw_id = TypeId(5);
        let closure_id = TypeId(6);
        let coro_id = TypeId(7);
        let coro_closure_id = TypeId(8);
        let zst_arr_id = TypeId(9);

        let mut ref_str = make_desc(
            ref_str_id.0,
            TypeKind::Reference {
                pointee: str_id,
                mutable: false,
            },
            false,
        );
        ref_str.js.pointer_form = Some(PointerForm::Str);

        let mut phantom_str = make_desc(
            phantom_str_id.0,
            TypeKind::Adt {
                def,
                args: vec![ref_str_id],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: interns.intern_str("PhantomData"),
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            true,
        );
        phantom_str.js.fields = Vec::new();

        let string_adt = make_desc(
            string_raw_id.0,
            TypeKind::Adt {
                def,
                args: vec![u8_id],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: interns.intern_str("String"),
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(interns.intern_str("buf")),
                        ty: u8_id,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            false,
        );

        let closure_d = make_desc(
            closure_id.0,
            TypeKind::Closure {
                def,
                upvars: vec![ref_str_id],
                fn_pointer_shim: None,
            },
            false,
        );

        let coro_d = make_desc(
            coro_id.0,
            TypeKind::Coroutine {
                def,
                upvars: Vec::new(),
                saved_locals: vec![(0, ref_str_id)],
                states: Vec::new(),
                variants: Vec::new(),
            },
            false,
        );

        let coro_closure_d = make_desc(
            coro_closure_id.0,
            TypeKind::CoroutineClosure {
                def,
                upvars: vec![ref_str_id],
            },
            false,
        );

        let zst_arr_d = make_desc(
            zst_arr_id.0,
            TypeKind::Array {
                element: ref_str_id,
                length: 0,
            },
            true,
        );

        let table = TypeTable::from(vec![
            make_desc(str_id.0, TypeKind::Str, false),
            ref_str,
            make_desc(
                u8_id.0,
                TypeKind::Integer {
                    signed: false,
                    bits: 8,
                },
                false,
            ),
            phantom_str,
            string_adt,
            closure_d,
            coro_d,
            coro_closure_d,
            zst_arr_d,
        ]);

        assert!(reaches_str(&table, phantom_str_id));
        assert!(!reaches_str(&table, string_raw_id));
        assert!(reaches_str(&table, closure_id));
        assert!(reaches_str(&table, coro_id));
        assert!(reaches_str(&table, coro_closure_id));
        assert!(reaches_str(&table, zst_arr_id));

        // ZST array `[&str; 0]` short-circuits without `.map()`.
        let passed = host_convert(&table, &interns, zst_arr_id, jsast::id("z"), Crossing::Out)
            .expect("zst array must pass");
        assert_eq!(passed, jsast::id("z"));

        // Coroutine value in `js!` is refused.
        assert!(host_convert(&table, &interns, coro_id, jsast::id("g"), Crossing::Out).is_err());

        // Closure value emits a deduplicated helper function call (not inline Object.assign).
        let clo_call = host_convert(&table, &interns, closure_id, jsast::id("c"), Crossing::Out)
            .expect("closure converts");
        assert!(
            matches!(clo_call, Expr::Call(ref f, _) if matches!(&**f, Expr::Ident(n) if n.starts_with("$bstr$out$"))),
            "{clo_call:?}"
        );
    }

    #[test]
    fn depth_cap_allows_64_and_refuses_65_and_refuses_raw_and_union() {
        use crate::jir::semantic::DefinitionId;
        let mut interns = Interns::new();
        let def = DefinitionId {
            crate_id: 2,
            local_hash: 1,
        };
        let str_id = TypeId(1);
        let ref_str_id = TypeId(2);
        let mut ref_str = make_desc(
            ref_str_id.0,
            TypeKind::Reference {
                pointee: str_id,
                mutable: false,
            },
            false,
        );
        ref_str.js.pointer_form = Some(PointerForm::Str);

        let mut descs = vec![make_desc(str_id.0, TypeKind::Str, false), ref_str];
        let mut cur = ref_str_id;
        let f_name = interns.intern_str("_0");
        let s_name = interns.intern_str("Wrap");
        for lvl in 1..=66u128 {
            let id = TypeId(100 + lvl);
            let d = make_desc(
                id.0,
                TypeKind::Adt {
                    def,
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f_name),
                            ty: cur,
                            offset: 0,
                        }],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                false,
            );
            descs.push(d);
            cur = id;
        }
        let d63 = TypeId(163);
        let d64 = TypeId(164);
        let d65 = TypeId(165);
        let raw_pair = TypeId(200);
        let tup = TypeId(201);
        let uni = TypeId(202);
        let u32_id = TypeId(203);

        descs.push(make_desc(
            u32_id.0,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            false,
        ));
        let mut tup_d = make_desc(tup.0, TypeKind::Tuple(vec![ref_str_id, u32_id]), false);
        tup_d.js.fields = vec![
            FieldDescriptor {
                key: FieldKey::Index(0),
                ty: ref_str_id,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Index(1),
                ty: u32_id,
                offset: 4,
            },
        ];
        descs.push(tup_d);
        descs.push(make_desc(
            raw_pair.0,
            TypeKind::RawPointer {
                pointee: tup,
                mutable: false,
            },
            false,
        ));
        descs.push(make_desc(
            uni.0,
            TypeKind::Adt {
                def,
                args: Vec::new(),
                form: NominalForm::Union,
                variants: vec![VariantDescriptor {
                    name: interns.intern_str("U"),
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Name(interns.intern_str("a")),
                            ty: ref_str_id,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Name(interns.intern_str("n")),
                            ty: u32_id,
                            offset: 0,
                        },
                    ],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            false,
        ));

        let table = TypeTable::from(descs);
        assert!(host_convert(&table, &interns, d63, jsast::id("v"), Crossing::Out).is_ok());
        assert!(host_convert(&table, &interns, d64, jsast::id("v"), Crossing::Out).is_ok());
        assert!(host_convert(&table, &interns, d65, jsast::id("v"), Crossing::Out).is_err());
        assert!(host_convert(&table, &interns, raw_pair, jsast::id("p"), Crossing::Out).is_err());
        assert!(host_convert(&table, &interns, uni, jsast::id("u"), Crossing::Out).is_err());
    }

    #[test]
    fn slot_pointer_and_slice_pointer_emit_distinct_descriptors() {
        let interns = Interns::default();
        let str_id = TypeId(1);
        let ref_str_id = TypeId(2);
        let slot_ref_id = TypeId(3);
        let slice_id = TypeId(4);
        let ref_slice_id = TypeId(5);

        let mut ref_str = make_desc(
            ref_str_id.0,
            TypeKind::Reference {
                pointee: str_id,
                mutable: false,
            },
            false,
        );
        ref_str.js.pointer_form = Some(PointerForm::Str);

        let mut slot_ref = make_desc(
            slot_ref_id.0,
            TypeKind::Reference {
                pointee: ref_str_id,
                mutable: false,
            },
            false,
        );
        slot_ref.js.pointer_form = Some(PointerForm::Slot);

        let slice_desc = make_desc(slice_id.0, TypeKind::Slice(ref_str_id), false);

        let mut ref_slice = make_desc(
            ref_slice_id.0,
            TypeKind::Reference {
                pointee: slice_id,
                mutable: false,
            },
            false,
        );
        ref_slice.js.pointer_form = Some(PointerForm::FatSlice);

        let table = TypeTable::from(vec![
            make_desc(str_id.0, TypeKind::Str, false),
            ref_str,
            slot_ref,
            slice_desc,
            ref_slice,
        ]);

        let mut cache = BTreeMap::new();
        let mut pending = Vec::new();
        let slot_stmt = build_decl(
            &table,
            &interns,
            &JsName::new("slot_fn".to_owned()),
            ConvTarget::Type(slot_ref_id, Mode::Val),
            Crossing::Out,
            &mut cache,
            &mut pending,
        )
        .expect("slot decl");
        let slot_js = make_link_item(JsName::new("slot_fn".to_owned()), slot_stmt).js;
        assert!(
            slot_js.contains("k: \"slot\"") || slot_js.contains("k:\"slot\""),
            "slot JS: {slot_js}"
        );
        assert!(!slot_js.contains("k: \"slice\"") && !slot_js.contains("k:\"slice\""));

        let slice_stmt = build_decl(
            &table,
            &interns,
            &JsName::new("slice_fn".to_owned()),
            ConvTarget::Type(ref_slice_id, Mode::Val),
            Crossing::Out,
            &mut cache,
            &mut pending,
        )
        .expect("slice decl");
        let slice_js = make_link_item(JsName::new("slice_fn".to_owned()), slice_stmt).js;
        assert!(
            slice_js.contains("k: \"slice\"") || slice_js.contains("k:\"slice\""),
            "slice JS: {slice_js}"
        );
        assert!(!slice_js.contains("k: \"slot\"") && !slice_js.contains("k:\"slot\""));
    }

    #[test]
    fn mut_str_pointers_are_live_byte_slices_not_bstr() {
        let str_id = TypeId(1);
        let ref_str_id = TypeId(2);
        let mut_str_id = TypeId(3);
        let const_ptr_id = TypeId(4);
        let mut_ptr_id = TypeId(5);
        let tuple_mut_id = TypeId(6);

        let mut ref_str = make_desc(
            ref_str_id.0,
            TypeKind::Reference {
                pointee: str_id,
                mutable: false,
            },
            false,
        );
        ref_str.js.pointer_form = Some(PointerForm::Str);

        let mut mut_str = make_desc(
            mut_str_id.0,
            TypeKind::Reference {
                pointee: str_id,
                mutable: true,
            },
            false,
        );
        mut_str.js.pointer_form = Some(PointerForm::FatSlice);

        let mut const_ptr = make_desc(
            const_ptr_id.0,
            TypeKind::RawPointer {
                pointee: str_id,
                mutable: false,
            },
            false,
        );
        const_ptr.js.pointer_form = Some(PointerForm::FatSlice);

        let mut mut_ptr = make_desc(
            mut_ptr_id.0,
            TypeKind::RawPointer {
                pointee: str_id,
                mutable: true,
            },
            false,
        );
        mut_ptr.js.pointer_form = Some(PointerForm::FatSlice);

        let tuple_mut = make_desc(tuple_mut_id.0, TypeKind::Tuple(vec![mut_str_id]), false);
        let box_str_id = TypeId(7);
        let mut box_str = make_desc(
            box_str_id.0,
            TypeKind::Adt {
                def: crate::jir::DefinitionId {
                    crate_id: 1,
                    local_hash: 7,
                },
                args: vec![str_id],
                form: NominalForm::Struct,
                variants: Vec::new(),
                niche: None,
            },
            false,
        );
        box_str.meta.candidate = RepresentationCandidate::Box;
        box_str.js.peeled = const_ptr_id;
        let tuple_box_id = TypeId(8);
        let tuple_box = make_desc(tuple_box_id.0, TypeKind::Tuple(vec![box_str_id]), false);

        let table = TypeTable::from(vec![
            make_desc(str_id.0, TypeKind::Str, false),
            ref_str,
            mut_str,
            const_ptr,
            mut_ptr,
            tuple_mut,
            box_str,
            tuple_box,
        ]);

        assert!(is_str_pointer(&table, ref_str_id));
        assert!(!is_str_pointer(&table, const_ptr_id));
        assert!(!is_str_pointer(&table, mut_str_id));
        assert!(!is_str_pointer(&table, mut_ptr_id));
        assert!(!is_str_pointer(&table, box_str_id));

        assert!(reaches_str(&table, ref_str_id));
        assert!(!reaches_str(&table, const_ptr_id));
        assert!(!reaches_str(&table, mut_str_id));
        assert!(!reaches_str(&table, mut_ptr_id));
        assert!(!reaches_str(&table, tuple_mut_id));
        assert!(!reaches_str(&table, box_str_id));
        assert!(!reaches_str(&table, tuple_box_id));
    }

    #[test]
    fn rewrites_str_case_helpers_and_catches_leftovers() {
        let mut stmt = jsast::ret(jsast::binary(
            jsast::BinOp::Add,
            rt("str_to_lowercase", vec![jsast::id("a")]),
            rt("str_to_uppercase", vec![jsast::id("b")]),
        ));
        rewrite(&mut stmt);
        let printed = jsast::program_to_string(std::slice::from_ref(&stmt));
        assert!(printed.contains("__rt.bstr_to_lowercase(a)"), "{printed}");
        assert!(printed.contains("__rt.bstr_to_uppercase(b)"), "{printed}");
        assert_eq!(leftover_helper(&printed), None, "{printed}");
        assert_eq!(
            leftover_helper("__rt.str_to_lowercase(s)"),
            Some("str_to_lowercase")
        );
        assert_eq!(
            leftover_helper("__rt.str_to_uppercase(s)"),
            Some("str_to_uppercase")
        );
    }

    #[test]
    fn folds_bstr_slice_roundtrip_and_elides_readonly_bstr_bytes() {
        // 1. `($t0 = __rt.sfp(__rt.bstr_bytes(s), b), __rt.bstr_of($t0.buf, $t0.off, $t0.len))` -> `s.slice(0, b)`.
        let mut roundtrip = jsast::ret(Expr::Seq(vec![
            Expr::Assign(
                Box::new(jsast::id("$t0")),
                Box::new(rt(
                    "sfp",
                    vec![rt("str_bytes", vec![jsast::id("s")]), jsast::id("b")],
                )),
            ),
            rt(
                "bytes_str",
                vec![
                    Expr::Member(Box::new(jsast::id("$t0")), "buf".to_owned()),
                    Expr::Member(Box::new(jsast::id("$t0")), "off".to_owned()),
                    Expr::Member(Box::new(jsast::id("$t0")), "len".to_owned()),
                ],
            ),
        ]));
        rewrite(&mut roundtrip);
        let printed = jsast::program_to_string(std::slice::from_ref(&roundtrip));
        assert!(printed.contains("s.slice(0, b)"), "{printed}");
        assert!(!printed.contains("bstr_bytes"), "{printed}");

        // 2. Read-only `let bv; bv = __rt.bstr_bytes(s); return bv.buf[bv.off + i];` -> `bv = s; return bv.charCodeAt(i);`.
        let mut body = vec![
            Stmt::Let("bv".to_owned(), None),
            jsast::assign_stmt(jsast::id("bv"), rt("bstr_bytes", vec![jsast::id("s")])),
            Stmt::Return(Some(Expr::Index(
                Box::new(Expr::Member(Box::new(jsast::id("bv")), "buf".to_owned())),
                Box::new(Expr::Binary(
                    jsast::BinOp::Add,
                    Box::new(Expr::Member(Box::new(jsast::id("bv")), "off".to_owned())),
                    Box::new(jsast::id("i")),
                )),
            ))),
        ];
        elide_readonly_bstr_bytes(&mut body);
        let printed_ro = jsast::program_to_string(&body);
        assert!(printed_ro.contains("bv = s;"), "{printed_ro}");
        assert!(printed_ro.contains("bv.charCodeAt(i)"), "{printed_ro}");
        assert!(!printed_ro.contains("bstr_bytes"), "{printed_ro}");
    }
}
