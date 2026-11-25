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
                    Some(EnumRepr::Number { .. })
                    | Some(EnumRepr::Tag { fieldless: true })
                    | Some(EnumRepr::IntTag { fieldless: true }) => Ok(true),
                    Some(EnumRepr::Tag { fieldless: false })
                    | Some(EnumRepr::IntTag { fieldless: false }) => {
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
                            Some(
                                EnumRepr::Tag { fieldless: false }
                                    | EnumRepr::IntTag { fieldless: false }
                            )
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

#[allow(clippy::too_many_arguments)]
fn build_decl(
    table: &TypeTable,
    interns: &Interns,
    name: &JsName,
    target: ConvTarget,
    dir: Crossing,
    native_vec: bool,
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
                    } if native_vec && crate::analysis::heap::is_global_vec_ty(table, info.cur) => {
                        let elem = vec_elem_ty(table, info.cur)
                            .ok_or_else(|| "a native Vec without element type".to_owned())?;
                        if mode == Mode::Ref {
                            let buf_pair = target_pair_expr(
                                table,
                                interns,
                                ConvTarget::ArrayBuf(elem),
                                cache,
                                pending,
                            )?
                            .ok_or_else(|| "a native Vec buffer with no conversion".to_owned())?;
                            let spec = jsast::object_of(vec![
                                ("k", jsast::string("o")),
                                ("f", jsast::object(vec![("buf".to_owned(), buf_pair)])),
                            ]);
                            body.push(jsast::ret(ref_call(spec)));
                        } else {
                            let buf_conv = target_name(
                                table,
                                interns,
                                ConvTarget::ArrayBuf(elem),
                                dir,
                                cache,
                            )?
                            .map(|cname| {
                                pending.push((ConvTarget::ArrayBuf(elem), dir));
                                jsast::call(
                                    jsast::id(cname.as_str().to_owned()),
                                    vec![jsast::member(p(), "buf")],
                                )
                            })
                            .unwrap_or_else(|| jsast::member(p(), "buf"));
                            let entries = vec![
                                ("buf".to_owned(), buf_conv),
                                ("len".to_owned(), jsast::member(p(), "len")),
                            ];
                            body.push(jsast::ret(jsast::object(entries)));
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
                        native_vec,
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
                let known_ascii = elide_known_ascii_bstr_out(body);
                elide_proven_bstr_sub(body, &known_ascii);
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
    // Inline ASCII-invariant method calls on `__rt.bstr_in(x)`:
    // `__rt.bstr_in(x).{startsWith,endsWith,includes}("ASCII")` -> `x.{...}("ASCII")`.
    if let Expr::Call(callee, args) = expr {
        if args.len() == 1 && is_ascii_str_lit(&args[0]) {
            if let Expr::Member(obj, method) = callee.as_mut() {
                if matches!(method.as_str(), "startsWith" | "endsWith" | "includes") {
                    if let Some(arg) = unwrap_bstr_in_call(obj) {
                        **obj = arg;
                        return;
                    }
                }
            }
        }
    }
    // Inline emptiness check: `__rt.bstr_in(x).length === 0` / `!== 0` / `> 0`.
    if let Expr::Binary(op, l, r) = expr {
        let zero_rhs = matches!(r.as_ref(), Expr::Num(n) if *n == 0.0);
        let zero_lhs = matches!(l.as_ref(), Expr::Num(n) if *n == 0.0);
        if zero_rhs
            && matches!(
                op,
                jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Gt
            )
        {
            if let Expr::Member(obj, prop) = l.as_mut() {
                if prop == "length" {
                    if let Some(arg) = unwrap_bstr_in_call(obj) {
                        **obj = arg;
                    }
                }
            }
        } else if zero_lhs
            && matches!(
                op,
                jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Lt
            )
        {
            if let Expr::Member(obj, prop) = r.as_mut() {
                if prop == "length" {
                    if let Some(arg) = unwrap_bstr_in_call(obj) {
                        **obj = arg;
                    }
                }
            }
        }
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

    fn is_ascii_charcode0_cmp<'a>(
        op: jsast::BinOp,
        l: &'a Expr,
        r: &'a Expr,
        locals: &BTreeSet<String>,
    ) -> Option<&'a str> {
        let match_cc0 = |e: &'a Expr| -> Option<&'a str> {
            let Expr::Call(callee, args) = e else {
                return None;
            };
            if args.len() != 1 || !matches!(&args[0], Expr::Num(n) if *n == 0.0) {
                return None;
            }
            let Expr::Member(obj, method) = callee.as_ref() else {
                return None;
            };
            if method != "charCodeAt" {
                return None;
            }
            match obj.as_ref() {
                Expr::Ident(n) if locals.contains(n) => Some(n.as_str()),
                _ => None,
            }
        };
        let check_num = |op: jsast::BinOp, k: f64| -> bool {
            if !k.is_finite() || k.fract() != 0.0 || k < 0.0 {
                return false;
            }
            match op {
                jsast::BinOp::StrictEq
                | jsast::BinOp::StrictNe
                | jsast::BinOp::Le
                | jsast::BinOp::Gt => k <= 127.0,
                jsast::BinOp::Lt | jsast::BinOp::Ge => k <= 128.0,
                _ => false,
            }
        };
        if let (Some(n), Expr::Num(k)) = (match_cc0(l), r) {
            if check_num(op, *k) {
                return Some(n);
            }
        }
        if let (Expr::Num(k), Some(n)) = (l, match_cc0(r)) {
            let flipped = match op {
                jsast::BinOp::StrictEq => jsast::BinOp::StrictEq,
                jsast::BinOp::StrictNe => jsast::BinOp::StrictNe,
                jsast::BinOp::Lt => jsast::BinOp::Gt,
                jsast::BinOp::Le => jsast::BinOp::Ge,
                jsast::BinOp::Gt => jsast::BinOp::Lt,
                jsast::BinOp::Ge => jsast::BinOp::Le,
                _ => return None,
            };
            if check_num(flipped, *k) {
                return Some(n);
            }
        }
        None
    }

    fn is_emptiness_cmp<'a>(
        op: jsast::BinOp,
        l: &'a Expr,
        r: &'a Expr,
        locals: &BTreeSet<String>,
    ) -> Option<&'a str> {
        let match_len = |e: &'a Expr| -> Option<&'a str> {
            let Expr::Member(obj, prop) = e else {
                return None;
            };
            if prop != "length" {
                return None;
            }
            match obj.as_ref() {
                Expr::Ident(n) if locals.contains(n) => Some(n.as_str()),
                _ => None,
            }
        };
        if let (Some(n), Expr::Num(k)) = (match_len(l), r) {
            if *k == 0.0
                && matches!(
                    op,
                    jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Gt
                )
            {
                return Some(n);
            }
        }
        if let (Expr::Num(k), Some(n)) = (l, match_len(r)) {
            if *k == 0.0
                && matches!(
                    op,
                    jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Lt
                )
            {
                return Some(n);
            }
        }
        None
    }

    fn scan_write_rhs(
        lhs: &str,
        rhs: &Expr,
        locals: &BTreeSet<String>,
        const_ascii: &BTreeSet<String>,
        info: &mut BTreeMap<String, Info>,
    ) -> bool {
        if let Some(arg) = unwrap_bstr_in_call(rhs) {
            info.entry(lhs.to_owned()).or_default().bstr_in_writes += 1;
            scan_expr(&arg, locals, const_ascii, false, info);
            return true;
        }
        if is_ascii_expr(rhs, const_ascii) {
            return true;
        }
        if let Expr::Ident(src) = rhs {
            if locals.contains(src) {
                info.entry(lhs.to_owned())
                    .or_default()
                    .copy_from
                    .push(src.clone());
                return true;
            }
        }
        if let Expr::Cond(cond, t, f) = rhs {
            let mut ok = true;
            let check_arm = |arm: &Expr| {
                unwrap_bstr_in_call(arm).is_some()
                    || is_ascii_expr(arm, const_ascii)
                    || matches!(arm, Expr::Ident(s) if locals.contains(s))
            };
            ok &= check_arm(t) && check_arm(f);
            if ok {
                scan_expr(cond, locals, const_ascii, false, info);
                for arm in [t.as_ref(), f.as_ref()] {
                    if let Some(arg) = unwrap_bstr_in_call(arm) {
                        info.entry(lhs.to_owned()).or_default().bstr_in_writes += 1;
                        scan_expr(&arg, locals, const_ascii, false, info);
                    } else if let Expr::Ident(src) = arm {
                        if locals.contains(src) && !const_ascii.contains(src) {
                            info.entry(lhs.to_owned())
                                .or_default()
                                .copy_from
                                .push(src.clone());
                        }
                    }
                }
                return true;
            }
        }
        false
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
        if let Expr::Binary(op, l, r) = expr {
            if matches!(op, jsast::BinOp::StrictEq | jsast::BinOp::StrictNe) {
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
            if is_ascii_charcode0_cmp(*op, l, r, locals).is_some()
                || is_emptiness_cmp(*op, l, r, locals).is_some()
            {
                return;
            }
        }
        if let Expr::Call(callee, args) = expr {
            if args.len() == 1 && is_ascii_expr(&args[0], const_ascii) {
                if let Expr::Member(obj, method) = callee.as_ref() {
                    if matches!(method.as_str(), "startsWith" | "endsWith" | "includes") {
                        if let Expr::Ident(n) = obj.as_ref() {
                            if locals.contains(n) {
                                return;
                            }
                        }
                    }
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
                        if scan_write_rhs(n, rhs, locals, const_ascii, info) {
                            return;
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
                    if !scan_write_rhs(n, init, locals, const_ascii, info) {
                        info.entry(n.clone()).or_default().bad = true;
                        scan_expr(init, locals, const_ascii, false, info);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if !in_closure && locals.contains(n) {
                                if scan_write_rhs(n, init, locals, const_ascii, info) {
                                    continue;
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

    fn unwrap_bstr_in_rhs(rhs: &mut Expr) {
        if let Some(arg) = unwrap_bstr_in_call(rhs) {
            *rhs = arg;
            return;
        }
        if let Expr::Cond(_, t, f) = rhs {
            if let Some(arg) = unwrap_bstr_in_call(t) {
                **t = arg;
            }
            if let Some(arg) = unwrap_bstr_in_call(f) {
                **f = arg;
            }
        }
    }

    fn rewrite_bstr_in_stmts(stmts: &mut [Stmt], elided: &BTreeSet<String>) {
        fn rw_expr(e: &mut Expr, elided: &BTreeSet<String>) {
            match e {
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if elided.contains(n) {
                            unwrap_bstr_in_rhs(rhs);
                            rw_expr(rhs, elided);
                            return;
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
                    unwrap_bstr_in_rhs(init);
                    rw_expr(init, elided);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if elided.contains(n) {
                                unwrap_bstr_in_rhs(init);
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

/// Collects local variables referenced inside nested closures (`Arrow` / inner `FunctionDecl`).
fn collect_closure_captured(stmts: &[Stmt], locals: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    fn walk_expr(e: &Expr, locals: &BTreeSet<String>, in_clo: bool, out: &mut BTreeSet<String>) {
        match e {
            Expr::Ident(n) if in_clo && locals.contains(n) => {
                out.insert(n.clone());
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => walk_expr(o, locals, in_clo, out),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                walk_expr(o, locals, in_clo, out);
                walk_expr(i, locals, in_clo, out);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                walk_expr(c, locals, in_clo, out);
                for a in args {
                    walk_expr(a, locals, in_clo, out);
                }
            }
            Expr::Cond(c, t, f) => {
                walk_expr(c, locals, in_clo, out);
                walk_expr(t, locals, in_clo, out);
                walk_expr(f, locals, in_clo, out);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    walk_expr(v, locals, in_clo, out);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    walk_expr(x, locals, in_clo, out);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => walk_expr(b, locals, true, out),
            Expr::Arrow(_, ArrowBody::Block(s)) => walk_stmts(s, locals, true, out),
            _ => {}
        }
    }
    fn walk_stmts(
        stmts: &[Stmt],
        locals: &BTreeSet<String>,
        in_clo: bool,
        out: &mut BTreeSet<String>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => walk_expr(e, locals, in_clo, out),
                Stmt::Lets(decls) => {
                    for (_, init) in decls {
                        if let Some(e) = init {
                            walk_expr(e, locals, in_clo, out);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    walk_expr(t, locals, in_clo, out);
                    walk_stmts(a, locals, in_clo, out);
                    if let Some(b) = b {
                        walk_stmts(b, locals, in_clo, out);
                    }
                }
                Stmt::Switch(d, cases) => {
                    walk_expr(d, locals, in_clo, out);
                    for c in cases {
                        for t in &c.tests {
                            walk_expr(t, locals, in_clo, out);
                        }
                        walk_stmts(&c.body, locals, in_clo, out);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    walk_stmts(std::slice::from_ref(inner.as_ref()), locals, in_clo, out);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => walk_stmts(b, locals, in_clo, out),
                Stmt::While(t, b) => {
                    walk_expr(t, locals, in_clo, out);
                    walk_stmts(b, locals, in_clo, out);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    walk_stmts(body, locals, in_clo, out);
                    walk_stmts(handler, locals, in_clo, out);
                }
                Stmt::FunctionDecl { body, .. } => walk_stmts(body, locals, true, out),
                _ => {}
            }
        }
    }
    walk_stmts(stmts, locals, false, out);
}

/// Intraprocedural JS-AST `KnownAscii` pass over a function body: proves local scalar strings and
/// local native `{ s, cap }` `String` objects contain only ASCII bytes (`0..128`), and elides
/// `__rt.bstr_out(e)` / `__rt.bstr_in(e)` / `__rt._bstr_is_ascii(e)` / `__rt.bstr_to_{lower,upper}case(e)`.
///
/// Returns the set of scalar local variable names proven `KnownAscii` for downstream passes.
fn elide_known_ascii_bstr_out(body: &mut [Stmt]) -> BTreeSet<String> {
    fn int_range(e: &Expr, depth: u8) -> Option<(i64, i64)> {
        if depth > 6 {
            return None;
        }
        match e {
            Expr::Num(n)
                if n.is_finite()
                    && n.fract() == 0.0
                    && *n >= i64::MIN as f64
                    && *n <= i64::MAX as f64 =>
            {
                let v = *n as i64;
                Some((v, v))
            }
            Expr::Binary(jsast::BinOp::BitAnd, l, r) => {
                let lr = int_range(l, depth + 1);
                let rr = int_range(r, depth + 1);
                match (lr, rr) {
                    (Some((a, b)), Some((c, d)))
                        if a >= 0 && c >= 0 && b <= 0x7fff_ffff && d <= 0x7fff_ffff =>
                    {
                        Some((0, b.min(d)))
                    }
                    (_, Some((c, d))) if c >= 0 && d <= 0x7fff_ffff => Some((0, d)),
                    (Some((a, b)), _) if a >= 0 && b <= 0x7fff_ffff => Some((0, b)),
                    _ => None,
                }
            }
            Expr::Binary(jsast::BinOp::UShr, l, r) => {
                let (k, k2) = int_range(r, depth + 1)?;
                if k != k2 || !(0..=31).contains(&k) {
                    return None;
                }
                let (lo, hi) = int_range(l, depth + 1)?;
                if lo >= 0 && hi <= 0xffff_ffff {
                    Some((lo >> k, hi >> k))
                } else {
                    None
                }
            }
            Expr::Binary(jsast::BinOp::Add, l, r) => {
                let (a, b) = int_range(l, depth + 1)?;
                let (c, d) = int_range(r, depth + 1)?;
                Some((a.checked_add(c)?, b.checked_add(d)?))
            }
            Expr::Binary(jsast::BinOp::Rem, l, r) => {
                let (m, m2) = int_range(r, depth + 1)?;
                if m != m2 || m < 1 {
                    return None;
                }
                if int_range(l, depth + 1).is_some_and(|(lo, _)| lo >= 0) {
                    Some((0, m - 1))
                } else {
                    Some((-m, m))
                }
            }
            Expr::Cond(_, t, f) => {
                let (a, b) = int_range(t, depth + 1)?;
                let (c, d) = int_range(f, depth + 1)?;
                Some((a.min(c), b.max(d)))
            }
            _ => None,
        }
    }

    fn is_ascii_code(e: &Expr) -> bool {
        int_range(e, 0).is_some_and(|(lo, hi)| lo >= 0 && hi <= 127)
    }

    fn is_numeric_or_bool(e: &Expr, depth: u8) -> bool {
        if depth > 6 {
            return false;
        }
        match e {
            Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_) | Expr::BigInt(_) => true,
            Expr::Unary(
                jsast::UnOp::Neg | jsast::UnOp::Pos | jsast::UnOp::Not | jsast::UnOp::BitNot,
                _,
            ) => true,
            Expr::Binary(op, l, r) => match op {
                jsast::BinOp::Add => {
                    is_numeric_or_bool(l, depth + 1) && is_numeric_or_bool(r, depth + 1)
                }
                jsast::BinOp::And | jsast::BinOp::Or => {
                    is_numeric_or_bool(l, depth + 1) && is_numeric_or_bool(r, depth + 1)
                }
                _ => true,
            },
            Expr::Call(callee, _) => match callee.as_ref() {
                Expr::Ident(n) => n == "Number",
                Expr::Member(obj, _) => matches!(obj.as_ref(), Expr::Ident(n) if n == "Math"),
                _ => false,
            },
            Expr::Cond(_, t, f) => {
                is_numeric_or_bool(t, depth + 1) && is_numeric_or_bool(f, depth + 1)
            }
            _ => false,
        }
    }

    fn is_known_ascii_expr(
        e: &Expr,
        scalars: &BTreeSet<String>,
        objs: &BTreeSet<String>,
        depth: u8,
    ) -> bool {
        if depth > 10 {
            return false;
        }
        match e {
            Expr::Str(s) => s.bytes().all(|b| b < 128),
            Expr::Ident(n) => scalars.contains(n),
            Expr::Member(obj, prop) if prop == "s" => {
                matches!(obj.as_ref(), Expr::Ident(n) if objs.contains(n))
            }
            Expr::Member(obj, prop) if prop == "text" => {
                if let Expr::Call(callee, _) = obj.as_ref() {
                    matches!(
                        rt_member(callee),
                        Some("native_f64_to_string" | "native_f32_to_string")
                    )
                } else {
                    false
                }
            }
            Expr::Binary(jsast::BinOp::Add, l, r) => {
                let la = is_known_ascii_expr(l, scalars, objs, depth + 1);
                let ra = is_known_ascii_expr(r, scalars, objs, depth + 1);
                (la && (ra || is_numeric_or_bool(r, 0))) || (ra && is_numeric_or_bool(l, 0))
            }
            Expr::Cond(_, t, f) => {
                is_known_ascii_expr(t, scalars, objs, depth + 1)
                    && is_known_ascii_expr(f, scalars, objs, depth + 1)
            }
            Expr::Seq(items) => items
                .last()
                .is_some_and(|last| is_known_ascii_expr(last, scalars, objs, depth + 1)),
            Expr::Call(callee, args) => {
                if let Some(member) = rt_member(callee) {
                    return match member {
                        "fmt_int_dec" | "fmt_int_hex" | "fmt_float_fixed" => true,
                        "bstr_out" | "bstr_in" | "bstr_sub" | "bstr_sub_from" | "bstr_sub_to"
                        | "bstr_trim" | "bstr_trim_start" | "bstr_trim_end"
                        | "bstr_to_lowercase" | "bstr_to_uppercase" => args
                            .first()
                            .is_some_and(|a| is_known_ascii_expr(a, scalars, objs, depth + 1)),
                        _ => false,
                    };
                }
                match callee.as_ref() {
                    Expr::Ident(n) if n == "String" && args.len() == 1 => {
                        is_numeric_or_bool(&args[0], 0)
                            || is_known_ascii_expr(&args[0], scalars, objs, depth + 1)
                    }
                    Expr::Member(obj, method) => match (obj.as_ref(), method.as_str()) {
                        (Expr::Ident(n), "fromCharCode") if n == "String" => {
                            args.iter().all(is_ascii_code)
                        }
                        (recv, "toString")
                            if args.is_empty()
                                || (args.len() == 1 && is_numeric_or_bool(&args[0], 0)) =>
                        {
                            is_numeric_or_bool(recv, 0)
                                || is_known_ascii_expr(recv, scalars, objs, depth + 1)
                        }
                        (
                            recv,
                            "slice" | "substring" | "repeat" | "toLowerCase" | "toUpperCase"
                            | "trim" | "trimStart" | "trimEnd",
                        ) => is_known_ascii_expr(recv, scalars, objs, depth + 1),
                        _ => false,
                    },
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn match_nstr_obj_init(e: &Expr) -> Option<&Expr> {
        let Expr::Object(entries) = e else {
            return None;
        };
        if entries.len() != 2 {
            return None;
        }
        let mut s_val = None;
        let mut has_cap = false;
        for (k, v) in entries {
            if k == "s" {
                s_val = Some(v);
            } else if k == "cap" {
                has_cap = true;
            }
        }
        has_cap.then_some(s_val?)
    }

    #[derive(Default)]
    struct ScalarInfo {
        /// Plain assignments `x = rhs` (can ground `x`).
        plain_writes: Vec<Expr>,
        /// Compound `x += rhs` writes (cannot ground `x` alone).
        append_writes: Vec<Expr>,
        bad: bool,
    }

    #[derive(Default)]
    struct ObjInfo {
        /// Initializer `s` expressions from `o = { s: init, cap: ... }` or `o.s = rhs`.
        plain_writes: Vec<Expr>,
        /// Compound `o.s += rhs`.
        append_writes: Vec<Expr>,
        bad: bool,
    }

    fn mark_obj_bad_in_expr(
        e: &Expr,
        locals: &BTreeSet<String>,
        objs: &mut BTreeMap<String, ObjInfo>,
    ) {
        match e {
            Expr::Ident(n) if locals.contains(n) => {
                objs.entry(n.clone()).or_default().bad = true;
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => mark_obj_bad_in_expr(o, locals, objs),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                mark_obj_bad_in_expr(o, locals, objs);
                mark_obj_bad_in_expr(i, locals, objs);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                mark_obj_bad_in_expr(c, locals, objs);
                for a in args {
                    mark_obj_bad_in_expr(a, locals, objs);
                }
            }
            Expr::Cond(c, t, f) => {
                mark_obj_bad_in_expr(c, locals, objs);
                mark_obj_bad_in_expr(t, locals, objs);
                mark_obj_bad_in_expr(f, locals, objs);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    mark_obj_bad_in_expr(v, locals, objs);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    mark_obj_bad_in_expr(x, locals, objs);
                }
            }
            _ => {}
        }
    }

    fn collect_expr(
        e: &Expr,
        locals: &BTreeSet<String>,
        scalars: &mut BTreeMap<String, ScalarInfo>,
        objs: &mut BTreeMap<String, ObjInfo>,
    ) {
        match e {
            Expr::Ident(n) if locals.contains(n) => {
                // Standalone read of `o` (not `o.s` / `o.cap` / `nstr_reserve(o, ...)`) escapes `o`.
                objs.entry(n.clone()).or_default().bad = true;
            }
            Expr::Member(obj, prop) => {
                if let Expr::Ident(n) = obj.as_ref() {
                    if locals.contains(n) && (prop == "s" || prop == "cap") {
                        return;
                    }
                }
                collect_expr(obj, locals, scalars, objs);
            }
            Expr::Assign(lhs, rhs) => match lhs.as_ref() {
                Expr::Ident(n) if locals.contains(n) => {
                    scalars
                        .entry(n.clone())
                        .or_default()
                        .plain_writes
                        .push((**rhs).clone());
                    if let Some(s_init) = match_nstr_obj_init(rhs) {
                        objs.entry(n.clone())
                            .or_default()
                            .plain_writes
                            .push(s_init.clone());
                    } else {
                        objs.entry(n.clone()).or_default().bad = true;
                    }
                    collect_expr(rhs, locals, scalars, objs);
                }
                Expr::Member(obj, prop) => {
                    if let Expr::Ident(n) = obj.as_ref() {
                        if locals.contains(n) {
                            if prop == "s" {
                                objs.entry(n.clone())
                                    .or_default()
                                    .plain_writes
                                    .push((**rhs).clone());
                                collect_expr(rhs, locals, scalars, objs);
                                return;
                            }
                            if prop == "cap" {
                                collect_expr(rhs, locals, scalars, objs);
                                return;
                            }
                        }
                    }
                    collect_expr(lhs, locals, scalars, objs);
                    collect_expr(rhs, locals, scalars, objs);
                }
                _ => {
                    collect_expr(lhs, locals, scalars, objs);
                    collect_expr(rhs, locals, scalars, objs);
                }
            },
            Expr::CompoundAssign(op, lhs, rhs) => match lhs.as_ref() {
                Expr::Ident(n) if locals.contains(n) => {
                    objs.entry(n.clone()).or_default().bad = true;
                    if *op == jsast::BinOp::Add {
                        scalars
                            .entry(n.clone())
                            .or_default()
                            .append_writes
                            .push((**rhs).clone());
                    } else {
                        scalars.entry(n.clone()).or_default().bad = true;
                    }
                    collect_expr(rhs, locals, scalars, objs);
                }
                Expr::Member(obj, prop) => {
                    if let Expr::Ident(n) = obj.as_ref() {
                        if locals.contains(n) && prop == "s" {
                            if *op == jsast::BinOp::Add {
                                objs.entry(n.clone())
                                    .or_default()
                                    .append_writes
                                    .push((**rhs).clone());
                            } else {
                                objs.entry(n.clone()).or_default().bad = true;
                            }
                            collect_expr(rhs, locals, scalars, objs);
                            return;
                        }
                        if locals.contains(n) && prop == "cap" {
                            collect_expr(rhs, locals, scalars, objs);
                            return;
                        }
                    }
                    collect_expr(lhs, locals, scalars, objs);
                    collect_expr(rhs, locals, scalars, objs);
                }
                _ => {
                    collect_expr(lhs, locals, scalars, objs);
                    collect_expr(rhs, locals, scalars, objs);
                }
            },
            Expr::Call(callee, args) => {
                if rt_member(callee) == Some("nstr_reserve") && args.len() == 2 {
                    if let Expr::Ident(n) = &args[0] {
                        if locals.contains(n) {
                            collect_expr(&args[1], locals, scalars, objs);
                            return;
                        }
                    }
                }
                collect_expr(callee, locals, scalars, objs);
                for a in args {
                    collect_expr(a, locals, scalars, objs);
                }
            }
            Expr::New(callee, args) => {
                collect_expr(callee, locals, scalars, objs);
                for a in args {
                    collect_expr(a, locals, scalars, objs);
                }
            }
            Expr::Unary(_, o) => collect_expr(o, locals, scalars, objs),
            Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                collect_expr(o, locals, scalars, objs);
                collect_expr(i, locals, scalars, objs);
            }
            Expr::Cond(c, t, f) => {
                collect_expr(c, locals, scalars, objs);
                collect_expr(t, locals, scalars, objs);
                collect_expr(f, locals, scalars, objs);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    collect_expr(v, locals, scalars, objs);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    collect_expr(x, locals, scalars, objs);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => mark_obj_bad_in_expr(b, locals, objs),
            Expr::Arrow(_, ArrowBody::Block(_)) => {}
            _ => {}
        }
    }

    fn collect_stmts(
        stmts: &[Stmt],
        locals: &BTreeSet<String>,
        scalars: &mut BTreeMap<String, ScalarInfo>,
        objs: &mut BTreeMap<String, ObjInfo>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                    if locals.contains(n) {
                        scalars
                            .entry(n.clone())
                            .or_default()
                            .plain_writes
                            .push(init.clone());
                        if let Some(s_init) = match_nstr_obj_init(init) {
                            objs.entry(n.clone())
                                .or_default()
                                .plain_writes
                                .push(s_init.clone());
                        } else {
                            objs.entry(n.clone()).or_default().bad = true;
                        }
                    }
                    collect_expr(init, locals, scalars, objs);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            if locals.contains(n) {
                                scalars
                                    .entry(n.clone())
                                    .or_default()
                                    .plain_writes
                                    .push(init.clone());
                                if let Some(s_init) = match_nstr_obj_init(init) {
                                    objs.entry(n.clone())
                                        .or_default()
                                        .plain_writes
                                        .push(s_init.clone());
                                } else {
                                    objs.entry(n.clone()).or_default().bad = true;
                                }
                            }
                            collect_expr(init, locals, scalars, objs);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    collect_expr(e, locals, scalars, objs);
                }
                Stmt::If(t, a, b) => {
                    collect_expr(t, locals, scalars, objs);
                    collect_stmts(a, locals, scalars, objs);
                    if let Some(b) = b {
                        collect_stmts(b, locals, scalars, objs);
                    }
                }
                Stmt::Switch(d, cases) => {
                    collect_expr(d, locals, scalars, objs);
                    for c in cases {
                        for t in &c.tests {
                            collect_expr(t, locals, scalars, objs);
                        }
                        collect_stmts(&c.body, locals, scalars, objs);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    collect_stmts(std::slice::from_ref(inner.as_ref()), locals, scalars, objs);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => collect_stmts(b, locals, scalars, objs),
                Stmt::While(t, b) => {
                    collect_expr(t, locals, scalars, objs);
                    collect_stmts(b, locals, scalars, objs);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        scalars.entry(binding.clone()).or_default().bad = true;
                        objs.entry(binding.clone()).or_default().bad = true;
                    }
                    collect_stmts(body, locals, scalars, objs);
                    collect_stmts(handler, locals, scalars, objs);
                }
                _ => {}
            }
        }
    }

    fn rewrite_expr_ascii(
        expr: &mut Expr,
        known_scalars: &BTreeSet<String>,
        known_objs: &BTreeSet<String>,
    ) {
        match expr {
            Expr::Member(o, _) | Expr::Unary(_, o) => {
                rewrite_expr_ascii(o, known_scalars, known_objs);
            }
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                rewrite_expr_ascii(o, known_scalars, known_objs);
                rewrite_expr_ascii(i, known_scalars, known_objs);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                rewrite_expr_ascii(c, known_scalars, known_objs);
                for a in args {
                    rewrite_expr_ascii(a, known_scalars, known_objs);
                }
            }
            Expr::Cond(c, t, f) => {
                rewrite_expr_ascii(c, known_scalars, known_objs);
                rewrite_expr_ascii(t, known_scalars, known_objs);
                rewrite_expr_ascii(f, known_scalars, known_objs);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    rewrite_expr_ascii(v, known_scalars, known_objs);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    rewrite_expr_ascii(x, known_scalars, known_objs);
                }
            }
            _ => {}
        }
        if let Expr::Call(callee, args) = expr {
            if let Some(member) = rt_member(callee) {
                match (member, args.len()) {
                    ("bstr_out" | "bstr_in", 1)
                        if is_known_ascii_expr(&args[0], known_scalars, known_objs, 0) =>
                    {
                        *expr = args.pop().expect("one argument");
                    }
                    ("_bstr_is_ascii", 1)
                        if is_known_ascii_expr(&args[0], known_scalars, known_objs, 0) =>
                    {
                        let arg = args.pop().expect("one argument");
                        *expr = if jsast::is_droppable(&arg) {
                            Expr::Bool(true)
                        } else {
                            Expr::Seq(vec![arg, Expr::Bool(true)])
                        };
                    }
                    ("_bstr_is_ascii_range", 3)
                        if is_known_ascii_expr(&args[0], known_scalars, known_objs, 0) =>
                    {
                        let mut seq = Vec::new();
                        for a in args.drain(..) {
                            if !jsast::is_droppable(&a) {
                                seq.push(a);
                            }
                        }
                        *expr = if seq.is_empty() {
                            Expr::Bool(true)
                        } else {
                            seq.push(Expr::Bool(true));
                            Expr::Seq(seq)
                        };
                    }
                    ("bstr_to_lowercase", 1)
                        if is_known_ascii_expr(&args[0], known_scalars, known_objs, 0) =>
                    {
                        let arg = args.pop().expect("one argument");
                        *expr = jsast::method_call(arg, "toLowerCase", Vec::new());
                    }
                    ("bstr_to_uppercase", 1)
                        if is_known_ascii_expr(&args[0], known_scalars, known_objs, 0) =>
                    {
                        let arg = args.pop().expect("one argument");
                        *expr = jsast::method_call(arg, "toUpperCase", Vec::new());
                    }
                    _ => {}
                }
            }
        }
    }

    fn rewrite_stmts_ascii(
        stmts: &mut [Stmt],
        known_scalars: &BTreeSet<String>,
        known_objs: &BTreeSet<String>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => rewrite_expr_ascii(e, known_scalars, known_objs),
                Stmt::Lets(decls) => {
                    for (_, init) in decls {
                        if let Some(e) = init {
                            rewrite_expr_ascii(e, known_scalars, known_objs);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    rewrite_expr_ascii(t, known_scalars, known_objs);
                    rewrite_stmts_ascii(a, known_scalars, known_objs);
                    if let Some(b) = b {
                        rewrite_stmts_ascii(b, known_scalars, known_objs);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rewrite_expr_ascii(d, known_scalars, known_objs);
                    for c in cases {
                        for t in &mut c.tests {
                            rewrite_expr_ascii(t, known_scalars, known_objs);
                        }
                        rewrite_stmts_ascii(&mut c.body, known_scalars, known_objs);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts_ascii(
                        std::slice::from_mut(inner.as_mut()),
                        known_scalars,
                        known_objs,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    rewrite_stmts_ascii(b, known_scalars, known_objs);
                }
                Stmt::While(t, b) => {
                    rewrite_expr_ascii(t, known_scalars, known_objs);
                    rewrite_stmts_ascii(b, known_scalars, known_objs);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts_ascii(body, known_scalars, known_objs);
                    rewrite_stmts_ascii(handler, known_scalars, known_objs);
                }
                _ => {}
            }
        }
    }

    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    let mut captured = BTreeSet::new();
    collect_closure_captured(body, &locals, &mut captured);

    let mut scalars: BTreeMap<String, ScalarInfo> = BTreeMap::new();
    let mut objs: BTreeMap<String, ObjInfo> = BTreeMap::new();
    collect_stmts(body, &locals, &mut scalars, &mut objs);
    for c in &captured {
        scalars.entry(c.clone()).or_default().bad = true;
        objs.entry(c.clone()).or_default().bad = true;
    }

    // Phase A: Greatest fixpoint — shrink optimistic sets until every write to every retained local
    // is proven pure ASCII.
    let mut known_scalars: BTreeSet<String> = scalars
        .iter()
        .filter(|(_, info)| !info.bad && !info.plain_writes.is_empty())
        .map(|(n, _)| n.clone())
        .collect();
    let mut known_objs: BTreeSet<String> = objs
        .iter()
        .filter(|(_, info)| !info.bad && !info.plain_writes.is_empty())
        .map(|(n, _)| n.clone())
        .collect();

    loop {
        let mut changed = false;
        let bad_scalars: Vec<String> = known_scalars
            .iter()
            .filter(|n| {
                let info = &scalars[*n];
                !info
                    .plain_writes
                    .iter()
                    .all(|e| is_known_ascii_expr(e, &known_scalars, &known_objs, 0))
                    || !info.append_writes.iter().all(|e| {
                        is_known_ascii_expr(e, &known_scalars, &known_objs, 0)
                            || is_numeric_or_bool(e, 0)
                    })
            })
            .cloned()
            .collect();
        for n in bad_scalars {
            known_scalars.remove(&n);
            changed = true;
        }
        let bad_objs: Vec<String> = known_objs
            .iter()
            .filter(|n| {
                let info = &objs[*n];
                !info
                    .plain_writes
                    .iter()
                    .all(|e| is_known_ascii_expr(e, &known_scalars, &known_objs, 0))
                    || !info.append_writes.iter().all(|e| {
                        is_known_ascii_expr(e, &known_scalars, &known_objs, 0)
                            || is_numeric_or_bool(e, 0)
                    })
            })
            .cloned()
            .collect();
        for n in bad_objs {
            known_objs.remove(&n);
            changed = true;
        }
        if !changed {
            break;
        }
    }

    // Phase B: Least fixpoint for groundedness from non-cyclic constructive ASCII roots so
    // ungrounded self-cycles (`x = y; y = x;`) cannot claim KnownAscii.
    let mut grounded_scalars: BTreeSet<String> = BTreeSet::new();
    let mut grounded_objs: BTreeSet<String> = BTreeSet::new();
    loop {
        let mut changed = false;
        for n in &known_scalars {
            if grounded_scalars.contains(n) {
                continue;
            }
            let info = &scalars[n];
            if info
                .plain_writes
                .iter()
                .any(|e| is_known_ascii_expr(e, &grounded_scalars, &grounded_objs, 0))
            {
                grounded_scalars.insert(n.clone());
                changed = true;
            }
        }
        for n in &known_objs {
            if grounded_objs.contains(n) {
                continue;
            }
            let info = &objs[n];
            if info
                .plain_writes
                .iter()
                .any(|e| is_known_ascii_expr(e, &grounded_scalars, &grounded_objs, 0))
            {
                grounded_objs.insert(n.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    rewrite_stmts_ascii(body, &grounded_scalars, &grounded_objs);
    grounded_scalars
}

/// Rewrites `__rt.bstr_sub` / `__rt.bstr_sub_from` / `__rt.bstr_sub_to` calls to direct
/// `s.slice(...)` (or `s` / `""`) whenever BOTH bounds (`0 <= start <= end <= s.length`) AND
/// UTF-8 char boundaries at `start`/`end` are statically proven from dominating guards,
/// `KnownAscii` provenance, or loop-invariant boundary induction.
fn elide_proven_bstr_sub(body: &mut [Stmt], known_ascii: &BTreeSet<String>) {
    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum IndexAtom {
        Const(u32),
        Var(String),
        VarPlus(String, u32),
        VarMinus(String, u32),
        StrLen(String),
    }

    impl IndexAtom {
        fn mentions(&self, var: &str) -> bool {
            match self {
                IndexAtom::Const(_) => false,
                IndexAtom::Var(v)
                | IndexAtom::VarPlus(v, _)
                | IndexAtom::VarMinus(v, _)
                | IndexAtom::StrLen(v) => v == var,
            }
        }

        fn plus_one(&self) -> Option<IndexAtom> {
            match self {
                IndexAtom::Const(k) => k.checked_add(1).map(IndexAtom::Const),
                IndexAtom::Var(v) => Some(IndexAtom::VarPlus(v.clone(), 1)),
                IndexAtom::VarPlus(v, k) => {
                    k.checked_add(1).map(|n| IndexAtom::VarPlus(v.clone(), n))
                }
                IndexAtom::VarMinus(v, 1) => Some(IndexAtom::Var(v.clone())),
                IndexAtom::VarMinus(v, k) if *k > 1 => Some(IndexAtom::VarMinus(v.clone(), k - 1)),
                _ => None,
            }
        }
    }

    #[derive(Clone, Default)]
    struct SubEnv {
        str_alias: BTreeMap<String, String>,
        len_alias: BTreeMap<String, String>,
        range_fields: BTreeMap<(String, &'static str), IndexAtom>,
        idx_alias: BTreeMap<String, IndexAtom>,
        valid_boundary: BTreeSet<(String, IndexAtom)>,
        lt_len: BTreeSet<(String, IndexAtom)>,
        le_len: BTreeSet<(String, IndexAtom)>,
        le_pairs: BTreeSet<(IndexAtom, IndexAtom)>,
        lt_pairs: BTreeSet<(IndexAtom, IndexAtom)>,
        gt_zero: BTreeSet<String>,
        ge_zero: BTreeSet<String>,
    }

    impl SubEnv {
        fn resolve_str_ident(&self, name: &str) -> String {
            let mut cur = name;
            for _ in 0..8 {
                if let Some(next) = self.str_alias.get(cur) {
                    if next == cur {
                        break;
                    }
                    cur = next.as_str();
                } else {
                    break;
                }
            }
            cur.to_owned()
        }

        fn resolve_str(&self, expr: &Expr) -> Option<String> {
            match expr {
                Expr::Ident(n) => Some(self.resolve_str_ident(n)),
                _ => None,
            }
        }

        fn kill_var(&mut self, var: &str) {
            self.str_alias.retain(|k, v| k != var && v != var);
            self.len_alias.retain(|k, v| k != var && v != var);
            self.range_fields
                .retain(|(obj, _), atom| obj != var && !atom.mentions(var));
            self.idx_alias
                .retain(|k, atom| k != var && !atom.mentions(var));
            self.valid_boundary
                .retain(|(s, atom)| s != var && !atom.mentions(var));
            self.lt_len
                .retain(|(s, atom)| s != var && !atom.mentions(var));
            self.le_len
                .retain(|(s, atom)| s != var && !atom.mentions(var));
            self.le_pairs
                .retain(|(a, b)| !a.mentions(var) && !b.mentions(var));
            self.lt_pairs
                .retain(|(a, b)| !a.mentions(var) && !b.mentions(var));
            self.gt_zero.remove(var);
            self.ge_zero.remove(var);
        }

        fn intersect(a: &SubEnv, b: &SubEnv) -> SubEnv {
            let mut out = SubEnv::default();
            for (k, v) in &a.str_alias {
                if b.str_alias.get(k) == Some(v) {
                    out.str_alias.insert(k.clone(), v.clone());
                }
            }
            for (k, v) in &a.len_alias {
                if b.len_alias.get(k) == Some(v) {
                    out.len_alias.insert(k.clone(), v.clone());
                }
            }
            for (k, v) in &a.range_fields {
                if b.range_fields.get(k) == Some(v) {
                    out.range_fields.insert(k.clone(), v.clone());
                }
            }
            for (k, v) in &a.idx_alias {
                if b.idx_alias.get(k) == Some(v) {
                    out.idx_alias.insert(k.clone(), v.clone());
                }
            }
            for x in &a.valid_boundary {
                if b.valid_boundary.contains(x) {
                    out.valid_boundary.insert(x.clone());
                }
            }
            for x in &a.lt_len {
                if b.lt_len.contains(x) {
                    out.lt_len.insert(x.clone());
                }
            }
            for x in &a.le_len {
                if b.le_len.contains(x) {
                    out.le_len.insert(x.clone());
                }
            }
            for x in &a.le_pairs {
                if b.le_pairs.contains(x) {
                    out.le_pairs.insert(x.clone());
                }
            }
            for x in &a.lt_pairs {
                if b.lt_pairs.contains(x) {
                    out.lt_pairs.insert(x.clone());
                }
            }
            for x in &a.gt_zero {
                if b.gt_zero.contains(x) {
                    out.gt_zero.insert(x.clone());
                }
            }
            for x in &a.ge_zero {
                if b.ge_zero.contains(x) {
                    out.ge_zero.insert(x.clone());
                }
            }
            out
        }

        fn is_lt_len_var(&self, root_s: &str, var: &str) -> bool {
            let v = IndexAtom::Var(var.to_owned());
            if self.lt_len.contains(&(root_s.to_owned(), v.clone())) {
                return true;
            }
            if let Some(orig) = self.idx_alias.get(var) {
                return self.lt_len.contains(&(root_s.to_owned(), orig.clone()));
            }
            false
        }

        fn resolve_u32_atom(
            &self,
            expr: &Expr,
            root_s_hint: Option<&str>,
            u32_locals: &BTreeSet<String>,
        ) -> Option<IndexAtom> {
            match expr {
                Expr::Num(n)
                    if n.is_finite()
                        && n.fract() == 0.0
                        && *n >= 0.0
                        && *n <= f64::from(u32::MAX) =>
                {
                    Some(IndexAtom::Const(*n as u32))
                }
                Expr::Ident(n) => {
                    if let Some(s) = self.len_alias.get(n) {
                        return Some(IndexAtom::StrLen(s.clone()));
                    }
                    if u32_locals.contains(n) || self.ge_zero.contains(n) {
                        return Some(IndexAtom::Var(n.clone()));
                    }
                    None
                }
                Expr::Member(obj, prop) => {
                    if prop == "length" {
                        if let Some(s) = self.resolve_str(obj) {
                            return Some(IndexAtom::StrLen(s));
                        }
                    }
                    if prop == "start" || prop == "end" {
                        if let Expr::Ident(obj_n) = obj.as_ref() {
                            let key = if prop == "start" { "start" } else { "end" };
                            if let Some(atom) = self.range_fields.get(&(obj_n.clone(), key)) {
                                return Some(atom.clone());
                            }
                        }
                    }
                    None
                }
                Expr::Binary(jsast::BinOp::UShr, l, r) if matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) => {
                    self.resolve_u32_atom(l, root_s_hint, u32_locals)
                }
                Expr::Binary(jsast::BinOp::Add, l, r) => {
                    let (v, k) = match (l.as_ref(), r.as_ref()) {
                        (Expr::Ident(v), Expr::Num(k)) | (Expr::Num(k), Expr::Ident(v))
                            if k.is_finite()
                                && k.fract() == 0.0
                                && *k >= 1.0
                                && *k <= 65536.0
                                && u32_locals.contains(v) =>
                        {
                            (v, *k as u32)
                        }
                        _ => return None,
                    };
                    // `v + k` cannot wrap `u32` when `v < s.length` (string length < 2^30).
                    let no_wrap = match root_s_hint {
                        Some(s) => self.is_lt_len_var(s, v),
                        None => self
                            .lt_len
                            .iter()
                            .any(|(_, a)| matches!(a, IndexAtom::Var(x) if x == v)),
                    };
                    no_wrap.then(|| IndexAtom::VarPlus(v.clone(), k))
                }
                Expr::Binary(jsast::BinOp::Sub, l, r) => {
                    if let (Expr::Ident(v), Expr::Num(k)) = (l.as_ref(), r.as_ref()) {
                        if *k == 1.0 && u32_locals.contains(v) && self.gt_zero.contains(v) {
                            return Some(IndexAtom::VarMinus(v.clone(), 1));
                        }
                    }
                    None
                }
                _ => None,
            }
        }

        fn is_valid_boundary_atom(
            &self,
            known_ascii: &BTreeSet<String>,
            root_s: &str,
            atom: &IndexAtom,
        ) -> bool {
            match atom {
                IndexAtom::Const(0) => true,
                IndexAtom::StrLen(s) if s == root_s => true,
                _ => {
                    let key = (root_s.to_owned(), atom.clone());
                    if self.valid_boundary.contains(&key) {
                        return true;
                    }
                    if known_ascii.contains(root_s)
                        && (self.le_len.contains(&key) || self.lt_len.contains(&key))
                    {
                        return true;
                    }
                    if let IndexAtom::Var(v) = atom {
                        if let Some(orig) = self.idx_alias.get(v) {
                            if orig != atom {
                                return self.is_valid_boundary_atom(known_ascii, root_s, orig);
                            }
                        }
                    }
                    false
                }
            }
        }

        fn is_le_atoms(&self, root_s: &str, a: &IndexAtom, b: &IndexAtom) -> bool {
            if a == b {
                return true;
            }
            if matches!(a, IndexAtom::Const(0)) {
                return true;
            }
            if matches!(b, IndexAtom::StrLen(s) if s == root_s) {
                return true;
            }
            if let (IndexAtom::Const(x), IndexAtom::Const(y)) = (a, b) {
                return x <= y;
            }
            if self.le_pairs.contains(&(a.clone(), b.clone()))
                || self.lt_pairs.contains(&(a.clone(), b.clone()))
            {
                return true;
            }
            // Check `Var(v) <= VarPlus(v, k)` and `VarPlus(v, k1) <= VarPlus(v, k2)` when `v < len`.
            match (a, b) {
                (IndexAtom::Var(v1), IndexAtom::VarPlus(v2, _)) if v1 == v2 => {
                    return self.is_lt_len_var(root_s, v1);
                }
                (IndexAtom::VarPlus(v1, k1), IndexAtom::VarPlus(v2, k2))
                    if v1 == v2 && k1 <= k2 =>
                {
                    return self.is_lt_len_var(root_s, v1);
                }
                _ => {}
            }
            // Resolve one step through `idx_alias`.
            if let IndexAtom::Var(vb) = b {
                if let Some(orig_b) = self.idx_alias.get(vb) {
                    if orig_b != b && self.is_le_atoms(root_s, a, orig_b) {
                        return true;
                    }
                }
            }
            if let IndexAtom::Var(va) = a {
                if let Some(orig_a) = self.idx_alias.get(va) {
                    if orig_a != a && self.is_le_atoms(root_s, orig_a, b) {
                        return true;
                    }
                }
            }
            false
        }
    }

    fn is_u32_expr(e: &Expr, u32_locals: &BTreeSet<String>) -> bool {
        match e {
            Expr::Num(n) => {
                n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX)
            }
            Expr::Ident(n) => u32_locals.contains(n),
            Expr::Member(_, prop) => prop == "length",
            Expr::Binary(jsast::BinOp::UShr, _, r) => {
                matches!(r.as_ref(), Expr::Num(n) if *n == 0.0)
            }
            Expr::Cond(_, t, f) => is_u32_expr(t, u32_locals) && is_u32_expr(f, u32_locals),
            _ => false,
        }
    }

    fn collect_u32_locals(
        body: &[Stmt],
        locals: &BTreeSet<String>,
        captured: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut writes: BTreeMap<String, Vec<Expr>> = BTreeMap::new();
        let mut bad: BTreeSet<String> = captured.clone();

        fn scan_e(
            e: &Expr,
            locals: &BTreeSet<String>,
            writes: &mut BTreeMap<String, Vec<Expr>>,
            bad: &mut BTreeSet<String>,
        ) {
            match e {
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if locals.contains(n) {
                            writes.entry(n.clone()).or_default().push((**rhs).clone());
                        }
                    }
                    scan_e(lhs, locals, writes, bad);
                    scan_e(rhs, locals, writes, bad);
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if locals.contains(n) {
                            bad.insert(n.clone());
                        }
                    }
                    scan_e(lhs, locals, writes, bad);
                    scan_e(rhs, locals, writes, bad);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => scan_e(o, locals, writes, bad),
                Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                    scan_e(o, locals, writes, bad);
                    scan_e(i, locals, writes, bad);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    scan_e(c, locals, writes, bad);
                    for a in args {
                        scan_e(a, locals, writes, bad);
                    }
                }
                Expr::Cond(c, t, f) => {
                    scan_e(c, locals, writes, bad);
                    scan_e(t, locals, writes, bad);
                    scan_e(f, locals, writes, bad);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        scan_e(v, locals, writes, bad);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        scan_e(x, locals, writes, bad);
                    }
                }
                _ => {}
            }
        }
        fn scan_s(
            stmts: &[Stmt],
            locals: &BTreeSet<String>,
            writes: &mut BTreeMap<String, Vec<Expr>>,
            bad: &mut BTreeSet<String>,
        ) {
            for stmt in stmts {
                match stmt {
                    Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                        if locals.contains(n) {
                            writes.entry(n.clone()).or_default().push(init.clone());
                        }
                        scan_e(init, locals, writes, bad);
                    }
                    Stmt::Lets(decls) => {
                        for (n, init) in decls {
                            if let Some(init) = init {
                                if locals.contains(n) {
                                    writes.entry(n.clone()).or_default().push(init.clone());
                                }
                                scan_e(init, locals, writes, bad);
                            }
                        }
                    }
                    Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                        scan_e(e, locals, writes, bad);
                    }
                    Stmt::If(t, a, b) => {
                        scan_e(t, locals, writes, bad);
                        scan_s(a, locals, writes, bad);
                        if let Some(b) = b {
                            scan_s(b, locals, writes, bad);
                        }
                    }
                    Stmt::Switch(d, cases) => {
                        scan_e(d, locals, writes, bad);
                        for c in cases {
                            for t in &c.tests {
                                scan_e(t, locals, writes, bad);
                            }
                            scan_s(&c.body, locals, writes, bad);
                        }
                    }
                    Stmt::Labeled(_, inner) => {
                        scan_s(std::slice::from_ref(inner.as_ref()), locals, writes, bad);
                    }
                    Stmt::LoopForever(_, b) | Stmt::Block(b) => scan_s(b, locals, writes, bad),
                    Stmt::While(t, b) => {
                        scan_e(t, locals, writes, bad);
                        scan_s(b, locals, writes, bad);
                    }
                    Stmt::TryCatch {
                        body,
                        binding,
                        handler,
                    } => {
                        bad.insert(binding.clone());
                        scan_s(body, locals, writes, bad);
                        scan_s(handler, locals, writes, bad);
                    }
                    _ => {}
                }
            }
        }
        scan_s(body, locals, &mut writes, &mut bad);
        let mut u32_set: BTreeSet<String> = writes
            .keys()
            .filter(|k| !bad.contains(*k))
            .cloned()
            .collect();
        loop {
            let remove: Vec<String> = u32_set
                .iter()
                .filter(|k| !writes[*k].iter().all(|e| is_u32_expr(e, &u32_set)))
                .cloned()
                .collect();
            if remove.is_empty() {
                break;
            }
            for k in remove {
                u32_set.remove(&k);
            }
        }
        u32_set
    }

    fn collect_assigned_vars_stmts(stmts: &[Stmt], out: &mut BTreeSet<String>) {
        fn scan_e(e: &Expr, out: &mut BTreeSet<String>) {
            match e {
                Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        out.insert(n.clone());
                    } else if let Expr::Member(obj, _) = lhs.as_ref() {
                        if let Expr::Ident(n) = obj.as_ref() {
                            out.insert(n.clone());
                        }
                    }
                    scan_e(lhs, out);
                    scan_e(rhs, out);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => scan_e(o, out),
                Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                    scan_e(o, out);
                    scan_e(i, out);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    scan_e(c, out);
                    for a in args {
                        scan_e(a, out);
                    }
                }
                Expr::Cond(c, t, f) => {
                    scan_e(c, out);
                    scan_e(t, out);
                    scan_e(f, out);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        scan_e(v, out);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        scan_e(x, out);
                    }
                }
                _ => {}
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                    out.insert(n.clone());
                    scan_e(e, out);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(e) = init {
                            out.insert(n.clone());
                            scan_e(e, out);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => scan_e(e, out),
                Stmt::If(t, a, b) => {
                    scan_e(t, out);
                    collect_assigned_vars_stmts(a, out);
                    if let Some(b) = b {
                        collect_assigned_vars_stmts(b, out);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_e(d, out);
                    for c in cases {
                        for t in &c.tests {
                            scan_e(t, out);
                        }
                        collect_assigned_vars_stmts(&c.body, out);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    collect_assigned_vars_stmts(std::slice::from_ref(inner.as_ref()), out);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => collect_assigned_vars_stmts(b, out),
                Stmt::While(t, b) => {
                    scan_e(t, out);
                    collect_assigned_vars_stmts(b, out);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    out.insert(binding.clone());
                    collect_assigned_vars_stmts(body, out);
                    collect_assigned_vars_stmts(handler, out);
                }
                _ => {}
            }
        }
    }

    fn stmts_always_exit(stmts: &[Stmt]) -> bool {
        let Some(last) = stmts.last() else {
            return false;
        };
        match last {
            Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break(_) | Stmt::Continue(_) => true,
            Stmt::Block(b) => stmts_always_exit(b),
            Stmt::If(_, then_b, Some(else_b)) => {
                stmts_always_exit(then_b) && stmts_always_exit(else_b)
            }
            _ => false,
        }
    }

    fn contains_break_to(stmt: &Stmt, label: &str) -> bool {
        match stmt {
            Stmt::Break(Some(l)) => l == label,
            Stmt::If(_, a, b) => {
                a.iter().any(|s| contains_break_to(s, label))
                    || b.as_ref()
                        .is_some_and(|b| b.iter().any(|s| contains_break_to(s, label)))
            }
            Stmt::Switch(_, cases) => cases
                .iter()
                .any(|c| c.body.iter().any(|s| contains_break_to(s, label))),
            Stmt::Labeled(_, inner) => contains_break_to(inner, label),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                b.iter().any(|s| contains_break_to(s, label))
            }
            Stmt::TryCatch { body, handler, .. } => {
                body.iter().any(|s| contains_break_to(s, label))
                    || handler.iter().any(|s| contains_break_to(s, label))
            }
            _ => false,
        }
    }

    fn record_ascii_char_at(env: &mut SubEnv, root_s: String, idx_atom: IndexAtom) {
        env.valid_boundary
            .insert((root_s.clone(), idx_atom.clone()));
        env.lt_len.insert((root_s.clone(), idx_atom.clone()));
        env.le_len.insert((root_s.clone(), idx_atom.clone()));
        if let Some(next) = idx_atom.plus_one() {
            env.valid_boundary.insert((root_s.clone(), next.clone()));
            env.le_len.insert((root_s.clone(), next.clone()));
            env.lt_pairs.insert((idx_atom.clone(), next.clone()));
            env.le_pairs.insert((idx_atom.clone(), next.clone()));
            if let IndexAtom::VarPlus(ref v, _) = idx_atom {
                let v_atom = IndexAtom::Var(v.clone());
                env.lt_pairs.insert((v_atom.clone(), next.clone()));
                env.le_pairs.insert((v_atom, next));
            }
        }
        if let IndexAtom::Var(ref v) = idx_atom {
            if let Some(orig) = env.idx_alias.get(v).cloned() {
                env.valid_boundary.insert((root_s.clone(), orig.clone()));
                env.lt_len.insert((root_s.clone(), orig.clone()));
                env.le_len.insert((root_s, orig));
            }
        }
    }

    fn apply_guard(env: &mut SubEnv, cond: &Expr, positive: bool, u32_locals: &BTreeSet<String>) {
        match cond {
            Expr::Unary(jsast::UnOp::Not, inner) => {
                apply_guard(env, inner, !positive, u32_locals);
            }
            Expr::Binary(jsast::BinOp::And, l, r) if positive => {
                apply_guard(env, l, true, u32_locals);
                apply_guard(env, r, true, u32_locals);
            }
            Expr::Binary(jsast::BinOp::Or, l, r) if !positive => {
                apply_guard(env, l, false, u32_locals);
                apply_guard(env, r, false, u32_locals);
            }
            Expr::Binary(op, l, r) => {
                let eff_op = if positive {
                    *op
                } else {
                    match op {
                        jsast::BinOp::Lt => jsast::BinOp::Ge,
                        jsast::BinOp::Le => jsast::BinOp::Gt,
                        jsast::BinOp::Gt => jsast::BinOp::Le,
                        jsast::BinOp::Ge => jsast::BinOp::Lt,
                        jsast::BinOp::StrictEq => jsast::BinOp::StrictNe,
                        jsast::BinOp::StrictNe => jsast::BinOp::StrictEq,
                        _ => return,
                    }
                };

                // 1. Check `s.charCodeAt(idx) === K` (K < 128) or `< K` (K <= 128) or `<= K` (K <= 127).
                fn match_cc<'a>(env: &SubEnv, e: &'a Expr) -> Option<(String, &'a Expr)> {
                    let Expr::Call(callee, args) = e else {
                        return None;
                    };
                    if args.len() != 1 {
                        return None;
                    }
                    let Expr::Member(obj, method) = callee.as_ref() else {
                        return None;
                    };
                    if method != "charCodeAt" {
                        return None;
                    }
                    let root_s = env.resolve_str(obj)?;
                    Some((root_s, &args[0]))
                }
                let is_ascii_guard_num = |op: jsast::BinOp, k: f64| -> bool {
                    if !k.is_finite() || k.fract() != 0.0 || k < 0.0 {
                        return false;
                    }
                    match op {
                        jsast::BinOp::StrictEq | jsast::BinOp::Le => k <= 127.0,
                        jsast::BinOp::Lt => (1.0..=128.0).contains(&k),
                        _ => false,
                    }
                };
                if let (Some((root_s, idx_e)), Expr::Num(k)) = (match_cc(env, l), r.as_ref()) {
                    if is_ascii_guard_num(eff_op, *k) {
                        if let Some(atom) = env.resolve_u32_atom(idx_e, Some(&root_s), u32_locals) {
                            record_ascii_char_at(env, root_s, atom);
                        }
                    }
                    return;
                }
                if let (Expr::Num(k), Some((root_s, idx_e))) = (l.as_ref(), match_cc(env, r)) {
                    let flipped = match eff_op {
                        jsast::BinOp::StrictEq => jsast::BinOp::StrictEq,
                        jsast::BinOp::Gt => jsast::BinOp::Lt,
                        jsast::BinOp::Ge => jsast::BinOp::Le,
                        _ => return,
                    };
                    if is_ascii_guard_num(flipped, *k) {
                        if let Some(atom) = env.resolve_u32_atom(idx_e, Some(&root_s), u32_locals) {
                            record_ascii_char_at(env, root_s, atom);
                        }
                    }
                    return;
                }

                // 2. Ordering / nonzero guards between `u32` atoms (seed non-negative guards for
                // parameter identifiers like `i >= 0` or `i > 0`).
                match (eff_op, l.as_ref(), r.as_ref()) {
                    (jsast::BinOp::Ge, Expr::Ident(v), Expr::Num(k))
                    | (jsast::BinOp::Le, Expr::Num(k), Expr::Ident(v))
                        if *k >= 0.0 =>
                    {
                        env.ge_zero.insert(v.clone());
                        if *k > 0.0 {
                            env.gt_zero.insert(v.clone());
                        }
                    }
                    (jsast::BinOp::Gt, Expr::Ident(v), Expr::Num(k))
                    | (jsast::BinOp::Lt, Expr::Num(k), Expr::Ident(v))
                        if *k >= 0.0 =>
                    {
                        env.ge_zero.insert(v.clone());
                        env.gt_zero.insert(v.clone());
                    }
                    _ => {}
                }
                let a = env.resolve_u32_atom(l, None, u32_locals);
                let b = env.resolve_u32_atom(r, None, u32_locals);
                if let (Some(a), Some(b)) = (a, b) {
                    match eff_op {
                        jsast::BinOp::Lt => {
                            env.lt_pairs.insert((a.clone(), b.clone()));
                            env.le_pairs.insert((a.clone(), b.clone()));
                            if let IndexAtom::StrLen(s) = &b {
                                env.lt_len.insert((s.clone(), a.clone()));
                                env.le_len.insert((s.clone(), a.clone()));
                            }
                            if a == IndexAtom::Const(0) {
                                if let IndexAtom::Var(v) = b {
                                    env.gt_zero.insert(v);
                                }
                            }
                        }
                        jsast::BinOp::Le => {
                            env.le_pairs.insert((a.clone(), b.clone()));
                            if let IndexAtom::StrLen(s) = &b {
                                env.le_len.insert((s.clone(), a));
                            }
                        }
                        jsast::BinOp::Gt => {
                            env.lt_pairs.insert((b.clone(), a.clone()));
                            env.le_pairs.insert((b.clone(), a.clone()));
                            if let IndexAtom::StrLen(s) = &a {
                                env.lt_len.insert((s.clone(), b.clone()));
                                env.le_len.insert((s.clone(), b.clone()));
                            }
                            if b == IndexAtom::Const(0) {
                                if let IndexAtom::Var(v) = a {
                                    env.gt_zero.insert(v);
                                }
                            }
                        }
                        jsast::BinOp::Ge => {
                            env.le_pairs.insert((b.clone(), a.clone()));
                            if let IndexAtom::StrLen(s) = &a {
                                env.le_len.insert((s.clone(), b));
                            }
                        }
                        jsast::BinOp::StrictNe => {
                            if b == IndexAtom::Const(0) {
                                if let IndexAtom::Var(v) = a {
                                    env.gt_zero.insert(v);
                                }
                            } else if a == IndexAtom::Const(0) {
                                if let IndexAtom::Var(v) = b {
                                    env.gt_zero.insert(v);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn update_env_for_assign(
        env: &mut SubEnv,
        lhs: &str,
        rhs: &Expr,
        u32_locals: &BTreeSet<String>,
        known_ascii: &BTreeSet<String>,
        inv_check: &mut Option<&mut BTreeSet<(String, String)>>,
    ) {
        // Compute pre-kill facts about `rhs`.
        let rhs_str_alias = match rhs {
            Expr::Ident(s) if s != lhs => Some(env.resolve_str_ident(s)),
            _ => None,
        };
        let rhs_len_of = match rhs {
            Expr::Member(obj, prop) if prop == "length" => env.resolve_str(obj),
            _ => None,
        };
        let rhs_range_fields: Vec<(&'static str, IndexAtom)> = match rhs {
            Expr::Object(entries) => entries
                .iter()
                .filter_map(|(k, v)| {
                    let key = if k == "start" {
                        "start"
                    } else if k == "end" {
                        "end"
                    } else {
                        return None;
                    };
                    let atom = env.resolve_u32_atom(v, None, u32_locals)?;
                    (!atom.mentions(lhs)).then_some((key, atom))
                })
                .collect(),
            _ => Vec::new(),
        };
        let rhs_atom = env.resolve_u32_atom(rhs, None, u32_locals);

        // If verifying loop-invariant boundary candidates, check whether `rhs` preserves `(s, lhs)`.
        if let Some(inv) = inv_check.as_deref_mut() {
            let to_drop: Vec<(String, String)> = inv
                .iter()
                .filter(|(s, v)| {
                    v == lhs
                        && !rhs_atom
                            .as_ref()
                            .is_some_and(|a| env.is_valid_boundary_atom(known_ascii, s, a))
                })
                .cloned()
                .collect();
            for pair in to_drop {
                inv.remove(&pair);
            }
        }

        // Snapshot boundary/order facts carried by `rhs_atom` before killing `lhs`.
        let mut carried_valid: Vec<String> = Vec::new();
        let mut carried_lt_len: Vec<String> = Vec::new();
        let mut carried_le_len: Vec<String> = Vec::new();
        let mut carried_le_from: Vec<IndexAtom> = Vec::new();
        let mut carried_lt_from: Vec<IndexAtom> = Vec::new();
        let mut carried_le_to: Vec<IndexAtom> = Vec::new();
        let mut carried_lt_to: Vec<IndexAtom> = Vec::new();

        if let Some(ref atom) = rhs_atom {
            for (s, a) in &env.valid_boundary {
                if a == atom && s != lhs {
                    carried_valid.push(s.clone());
                }
            }
            if let IndexAtom::Var(v) = atom {
                if let Some(orig) = env.idx_alias.get(v) {
                    for (s, a) in &env.valid_boundary {
                        if a == orig && s != lhs {
                            carried_valid.push(s.clone());
                        }
                    }
                }
            }
            for (s, a) in &env.lt_len {
                if a == atom && s != lhs {
                    carried_lt_len.push(s.clone());
                }
            }
            for (s, a) in &env.le_len {
                if a == atom && s != lhs {
                    carried_le_len.push(s.clone());
                }
            }
            for (a, b) in &env.le_pairs {
                if b == atom && !a.mentions(lhs) {
                    carried_le_from.push(a.clone());
                }
                if a == atom && !b.mentions(lhs) {
                    carried_le_to.push(b.clone());
                }
            }
            for (a, b) in &env.lt_pairs {
                if b == atom && !a.mentions(lhs) {
                    carried_lt_from.push(a.clone());
                }
                if a == atom && !b.mentions(lhs) {
                    carried_lt_to.push(b.clone());
                }
            }
        }

        // Kill all old facts involving `lhs`.
        env.kill_var(lhs);

        let lhs_atom = IndexAtom::Var(lhs.to_owned());
        if let Some(root_s) = rhs_str_alias {
            if root_s != lhs {
                env.str_alias.insert(lhs.to_owned(), root_s);
            }
        }
        if let Some(root_s) = rhs_len_of {
            if root_s != lhs {
                env.len_alias.insert(lhs.to_owned(), root_s.clone());
                env.valid_boundary
                    .insert((root_s.clone(), lhs_atom.clone()));
                env.le_len.insert((root_s, lhs_atom.clone()));
            }
        }
        for (field, atom) in rhs_range_fields {
            env.range_fields.insert((lhs.to_owned(), field), atom);
        }
        if let Some(atom) = rhs_atom {
            if !atom.mentions(lhs) {
                env.idx_alias.insert(lhs.to_owned(), atom.clone());
            }
            if let IndexAtom::StrLen(s) = &atom {
                if s != lhs {
                    env.valid_boundary.insert((s.clone(), lhs_atom.clone()));
                    env.le_len.insert((s.clone(), lhs_atom.clone()));
                }
            }
        }
        for s in carried_valid {
            env.valid_boundary.insert((s.clone(), lhs_atom.clone()));
            env.le_len.insert((s, lhs_atom.clone()));
        }
        for s in carried_lt_len {
            env.lt_len.insert((s.clone(), lhs_atom.clone()));
            env.le_len.insert((s, lhs_atom.clone()));
        }
        for s in carried_le_len {
            env.le_len.insert((s, lhs_atom.clone()));
        }
        for a in carried_le_from {
            env.le_pairs.insert((a, lhs_atom.clone()));
        }
        for a in carried_lt_from {
            env.lt_pairs.insert((a.clone(), lhs_atom.clone()));
            env.le_pairs.insert((a, lhs_atom.clone()));
        }
        for b in carried_le_to {
            env.le_pairs.insert((lhs_atom.clone(), b));
        }
        for b in carried_lt_to {
            env.lt_pairs.insert((lhs_atom.clone(), b.clone()));
            env.le_pairs.insert((lhs_atom.clone(), b));
        }
    }

    fn try_rewrite_bstr_sub_call(
        expr: &mut Expr,
        env: &SubEnv,
        u32_locals: &BTreeSet<String>,
        known_ascii: &BTreeSet<String>,
    ) {
        let Expr::Call(callee, args) = expr else {
            return;
        };
        let Some(member) = rt_member(callee) else {
            return;
        };
        match (member, args.len()) {
            ("bstr_sub", 4 | 5) => {
                if !jsast::is_droppable(&args[3])
                    || args.get(4).is_some_and(|l| !jsast::is_droppable(l))
                {
                    return;
                }
                let Some(root_s) = env.resolve_str(&args[0]) else {
                    return;
                };
                let Some(a) = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals) else {
                    return;
                };
                let Some(b) = env.resolve_u32_atom(&args[2], Some(&root_s), u32_locals) else {
                    return;
                };
                if a == IndexAtom::Const(0)
                    && b == IndexAtom::Const(0)
                    && jsast::is_droppable(&args[0])
                    && jsast::is_droppable(&args[1])
                    && jsast::is_droppable(&args[2])
                {
                    *expr = jsast::string("");
                    return;
                }
                if a == IndexAtom::Const(0)
                    && b == IndexAtom::StrLen(root_s.clone())
                    && jsast::is_droppable(&args[1])
                    && jsast::is_droppable(&args[2])
                {
                    *expr = args[0].clone();
                    return;
                }
                if env.is_valid_boundary_atom(known_ascii, &root_s, &a)
                    && env.is_valid_boundary_atom(known_ascii, &root_s, &b)
                    && env.is_le_atoms(&root_s, &a, &b)
                {
                    *expr = jsast::method_call(
                        args[0].clone(),
                        "slice",
                        vec![args[1].clone(), args[2].clone()],
                    );
                }
            }
            ("bstr_sub_from", 3 | 4) => {
                if !jsast::is_droppable(&args[2])
                    || args.get(3).is_some_and(|l| !jsast::is_droppable(l))
                {
                    return;
                }
                let Some(root_s) = env.resolve_str(&args[0]) else {
                    return;
                };
                let Some(a) = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals) else {
                    return;
                };
                if a == IndexAtom::Const(0) && jsast::is_droppable(&args[1]) {
                    *expr = args[0].clone();
                    return;
                }
                if env.is_valid_boundary_atom(known_ascii, &root_s, &a) {
                    *expr = jsast::method_call(args[0].clone(), "slice", vec![args[1].clone()]);
                }
            }
            ("bstr_sub_to", 3 | 4) => {
                if !jsast::is_droppable(&args[2])
                    || args.get(3).is_some_and(|l| !jsast::is_droppable(l))
                {
                    return;
                }
                let Some(root_s) = env.resolve_str(&args[0]) else {
                    return;
                };
                let Some(b) = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals) else {
                    return;
                };
                if b == IndexAtom::Const(0)
                    && jsast::is_droppable(&args[0])
                    && jsast::is_droppable(&args[1])
                {
                    *expr = jsast::string("");
                    return;
                }
                if b == IndexAtom::StrLen(root_s.clone()) && jsast::is_droppable(&args[1]) {
                    *expr = args[0].clone();
                    return;
                }
                if env.is_valid_boundary_atom(known_ascii, &root_s, &b) {
                    *expr = jsast::method_call(
                        args[0].clone(),
                        "slice",
                        vec![jsast::num(0), args[1].clone()],
                    );
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_expr(
        expr: &mut Expr,
        env: &mut SubEnv,
        u32_locals: &BTreeSet<String>,
        known_ascii: &BTreeSet<String>,
        rewrite: bool,
        inv_check: &mut Option<&mut BTreeSet<(String, String)>>,
    ) {
        match expr {
            Expr::Assign(lhs, rhs) => {
                walk_expr(rhs, env, u32_locals, known_ascii, rewrite, inv_check);
                if let Expr::Ident(n) = lhs.as_ref() {
                    update_env_for_assign(env, n, rhs, u32_locals, known_ascii, inv_check);
                } else {
                    walk_expr(lhs, env, u32_locals, known_ascii, rewrite, inv_check);
                    if let Expr::Member(obj, _) = lhs.as_ref() {
                        if let Expr::Ident(n) = obj.as_ref() {
                            env.range_fields.retain(|(o, _), _| o != n);
                        }
                    }
                }
                return;
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                walk_expr(rhs, env, u32_locals, known_ascii, rewrite, inv_check);
                walk_expr(lhs, env, u32_locals, known_ascii, rewrite, inv_check);
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let Some(inv) = inv_check.as_deref_mut() {
                        inv.retain(|(_, v)| v != n);
                    }
                    env.kill_var(n);
                }
                return;
            }
            Expr::Binary(jsast::BinOp::And, l, r) => {
                walk_expr(l, env, u32_locals, known_ascii, rewrite, inv_check);
                let mut r_env = env.clone();
                apply_guard(&mut r_env, l, true, u32_locals);
                walk_expr(r, &mut r_env, u32_locals, known_ascii, rewrite, inv_check);
                return;
            }
            Expr::Cond(c, t, f) => {
                walk_expr(c, env, u32_locals, known_ascii, rewrite, inv_check);
                let mut t_env = env.clone();
                apply_guard(&mut t_env, c, true, u32_locals);
                walk_expr(t, &mut t_env, u32_locals, known_ascii, rewrite, inv_check);
                let mut f_env = env.clone();
                apply_guard(&mut f_env, c, false, u32_locals);
                walk_expr(f, &mut f_env, u32_locals, known_ascii, rewrite, inv_check);
                *env = SubEnv::intersect(&t_env, &f_env);
                return;
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => {
                walk_expr(o, env, u32_locals, known_ascii, rewrite, inv_check);
            }
            Expr::Index(o, i) | Expr::Binary(_, o, i) => {
                walk_expr(o, env, u32_locals, known_ascii, rewrite, inv_check);
                walk_expr(i, env, u32_locals, known_ascii, rewrite, inv_check);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                walk_expr(c, env, u32_locals, known_ascii, rewrite, inv_check);
                for a in args {
                    walk_expr(a, env, u32_locals, known_ascii, rewrite, inv_check);
                }
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    walk_expr(v, env, u32_locals, known_ascii, rewrite, inv_check);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    walk_expr(x, env, u32_locals, known_ascii, rewrite, inv_check);
                }
            }
            _ => {}
        }
        if rewrite {
            try_rewrite_bstr_sub_call(expr, env, u32_locals, known_ascii);
        }
    }

    fn collect_candidate_strs_expr(e: &Expr, env: &SubEnv, out: &mut BTreeSet<String>) {
        match e {
            Expr::Member(obj, prop) if prop == "length" => {
                if let Some(root_s) = env.resolve_str(obj) {
                    out.insert(root_s);
                }
                collect_candidate_strs_expr(obj, env, out);
            }
            Expr::Call(callee, args) => {
                if let Expr::Member(obj, method) = callee.as_ref() {
                    if method == "charCodeAt" {
                        if let Some(root_s) = env.resolve_str(obj) {
                            out.insert(root_s);
                        }
                    }
                }
                if matches!(
                    rt_member(callee),
                    Some("bstr_sub" | "bstr_sub_from" | "bstr_sub_to")
                ) {
                    if let Some(first) = args.first() {
                        if let Some(root_s) = env.resolve_str(first) {
                            out.insert(root_s);
                        }
                    }
                }
                collect_candidate_strs_expr(callee, env, out);
                for a in args {
                    collect_candidate_strs_expr(a, env, out);
                }
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => collect_candidate_strs_expr(o, env, out),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                collect_candidate_strs_expr(o, env, out);
                collect_candidate_strs_expr(i, env, out);
            }
            Expr::New(c, args) => {
                collect_candidate_strs_expr(c, env, out);
                for a in args {
                    collect_candidate_strs_expr(a, env, out);
                }
            }
            Expr::Cond(c, t, f) => {
                collect_candidate_strs_expr(c, env, out);
                collect_candidate_strs_expr(t, env, out);
                collect_candidate_strs_expr(f, env, out);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    collect_candidate_strs_expr(v, env, out);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    collect_candidate_strs_expr(x, env, out);
                }
            }
            _ => {}
        }
    }

    fn collect_candidate_strs_stmts(stmts: &[Stmt], env: &SubEnv, out: &mut BTreeSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(_, Some(e))
                | Stmt::Const(_, e)
                | Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Return(Some(e)) => collect_candidate_strs_expr(e, env, out),
                Stmt::Lets(decls) => {
                    for (_, init) in decls {
                        if let Some(e) = init {
                            collect_candidate_strs_expr(e, env, out);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    collect_candidate_strs_expr(t, env, out);
                    collect_candidate_strs_stmts(a, env, out);
                    if let Some(b) = b {
                        collect_candidate_strs_stmts(b, env, out);
                    }
                }
                Stmt::Switch(d, cases) => {
                    collect_candidate_strs_expr(d, env, out);
                    for c in cases {
                        for t in &c.tests {
                            collect_candidate_strs_expr(t, env, out);
                        }
                        collect_candidate_strs_stmts(&c.body, env, out);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    collect_candidate_strs_stmts(std::slice::from_ref(inner.as_ref()), env, out);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    collect_candidate_strs_stmts(b, env, out)
                }
                Stmt::While(t, b) => {
                    collect_candidate_strs_expr(t, env, out);
                    collect_candidate_strs_stmts(b, env, out);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    collect_candidate_strs_stmts(body, env, out);
                    collect_candidate_strs_stmts(handler, env, out);
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_stmts(
        stmts: &mut [Stmt],
        env: &mut SubEnv,
        u32_locals: &BTreeSet<String>,
        known_ascii: &BTreeSet<String>,
        captured: &BTreeSet<String>,
        active_inv: &BTreeSet<(String, String)>,
        rewrite: bool,
        inv_check: &mut Option<&mut BTreeSet<(String, String)>>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                    walk_expr(init, env, u32_locals, known_ascii, rewrite, inv_check);
                    update_env_for_assign(env, n, init, u32_locals, known_ascii, inv_check);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            walk_expr(init, env, u32_locals, known_ascii, rewrite, inv_check);
                            update_env_for_assign(env, n, init, u32_locals, known_ascii, inv_check);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    walk_expr(e, env, u32_locals, known_ascii, rewrite, inv_check);
                }
                Stmt::If(test, then_b, else_b) => {
                    walk_expr(test, env, u32_locals, known_ascii, rewrite, inv_check);
                    let mut then_env = env.clone();
                    apply_guard(&mut then_env, test, true, u32_locals);
                    walk_stmts(
                        then_b,
                        &mut then_env,
                        u32_locals,
                        known_ascii,
                        captured,
                        active_inv,
                        rewrite,
                        inv_check,
                    );
                    let mut else_env = env.clone();
                    apply_guard(&mut else_env, test, false, u32_locals);
                    if let Some(else_b) = else_b {
                        walk_stmts(
                            else_b,
                            &mut else_env,
                            u32_locals,
                            known_ascii,
                            captured,
                            active_inv,
                            rewrite,
                            inv_check,
                        );
                    }
                    let then_exits = stmts_always_exit(then_b);
                    let else_exits = else_b.as_ref().is_some_and(|b| stmts_always_exit(b));
                    if then_exits && !else_exits {
                        *env = else_env;
                    } else if else_exits && !then_exits {
                        *env = then_env;
                    } else {
                        *env = SubEnv::intersect(&then_env, &else_env);
                    }
                }
                Stmt::Switch(disc, cases) => {
                    walk_expr(disc, env, u32_locals, known_ascii, rewrite, inv_check);
                    let mut modified = BTreeSet::new();
                    for c in cases.iter() {
                        collect_assigned_vars_stmts(&c.body, &mut modified);
                    }
                    for c in cases {
                        for t in &mut c.tests {
                            walk_expr(t, env, u32_locals, known_ascii, rewrite, inv_check);
                        }
                        let mut case_env = env.clone();
                        walk_stmts(
                            &mut c.body,
                            &mut case_env,
                            u32_locals,
                            known_ascii,
                            captured,
                            active_inv,
                            rewrite,
                            inv_check,
                        );
                    }
                    for v in &modified {
                        env.kill_var(v);
                    }
                    for (s, v) in active_inv {
                        env.valid_boundary
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                        env.le_len.insert((s.clone(), IndexAtom::Var(v.clone())));
                    }
                }
                Stmt::Block(b) => {
                    walk_stmts(
                        b,
                        env,
                        u32_locals,
                        known_ascii,
                        captured,
                        active_inv,
                        rewrite,
                        inv_check,
                    );
                }
                Stmt::Labeled(label, inner) => {
                    let is_loop = matches!(inner.as_ref(), Stmt::LoopForever(..) | Stmt::While(..));
                    if is_loop {
                        walk_stmts(
                            std::slice::from_mut(inner.as_mut()),
                            env,
                            u32_locals,
                            known_ascii,
                            captured,
                            active_inv,
                            rewrite,
                            inv_check,
                        );
                    } else {
                        let pre = env.clone();
                        let has_break = contains_break_to(inner, label);
                        let mut modified = BTreeSet::new();
                        if has_break {
                            collect_assigned_vars_stmts(
                                std::slice::from_ref(inner.as_ref()),
                                &mut modified,
                            );
                        }
                        walk_stmts(
                            std::slice::from_mut(inner.as_mut()),
                            env,
                            u32_locals,
                            known_ascii,
                            captured,
                            active_inv,
                            rewrite,
                            inv_check,
                        );
                        if has_break {
                            let mut safe_pre = pre;
                            for v in &modified {
                                safe_pre.kill_var(v);
                            }
                            for (s, v) in active_inv {
                                safe_pre
                                    .valid_boundary
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                                safe_pre
                                    .le_len
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                            }
                            *env = SubEnv::intersect(env, &safe_pre);
                        }
                    }
                }
                Stmt::LoopForever(_, loop_body) => {
                    let mut modified = BTreeSet::new();
                    collect_assigned_vars_stmts(loop_body, &mut modified);
                    let mut inv: BTreeSet<(String, String)> = BTreeSet::new();
                    for (s, atom) in &env.valid_boundary {
                        if let IndexAtom::Var(v) = atom {
                            if !modified.contains(s)
                                && !captured.contains(s)
                                && !captured.contains(v)
                                && modified.contains(v)
                            {
                                inv.insert((s.clone(), v.clone()));
                            }
                        }
                    }
                    // Also seed `v` whose alias is `Const(0)` for any unmodified known string in env.
                    let mut candidate_strs: BTreeSet<String> = known_ascii.clone();
                    for s in env.len_alias.values() {
                        candidate_strs.insert(s.clone());
                    }
                    for s in env.str_alias.values() {
                        candidate_strs.insert(s.clone());
                    }
                    for (s, _) in &env.valid_boundary {
                        candidate_strs.insert(s.clone());
                    }
                    collect_candidate_strs_stmts(loop_body, env, &mut candidate_strs);
                    for (v, atom) in &env.idx_alias {
                        if *atom == IndexAtom::Const(0)
                            && modified.contains(v)
                            && !captured.contains(v)
                        {
                            for s in &candidate_strs {
                                if !modified.contains(s) && !captured.contains(s) {
                                    inv.insert((s.clone(), v.clone()));
                                }
                            }
                        }
                    }
                    // Fixpoint verify loop-invariant boundary pairs.
                    for _ in 0..4 {
                        let prev_len = inv.len();
                        let mut check_env = env.clone();
                        for v in &modified {
                            check_env.kill_var(v);
                        }
                        let mut merged_inv = active_inv.clone();
                        for (s, v) in &inv {
                            if !modified.contains(s) {
                                merged_inv.insert((s.clone(), v.clone()));
                                check_env
                                    .valid_boundary
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                                check_env
                                    .le_len
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                            }
                        }
                        let mut inv_ref = Some(&mut inv);
                        walk_stmts(
                            loop_body,
                            &mut check_env,
                            u32_locals,
                            known_ascii,
                            captured,
                            &merged_inv,
                            false,
                            &mut inv_ref,
                        );
                        if inv.len() == prev_len {
                            break;
                        }
                    }
                    // Propagate outer `inv_check` if we are nested inside another loop verification.
                    if let Some(outer) = inv_check.as_deref_mut() {
                        outer.retain(|(s, v)| {
                            !modified.contains(v) || inv.contains(&(s.clone(), v.clone()))
                        });
                    }
                    let mut loop_env = env.clone();
                    for v in &modified {
                        loop_env.kill_var(v);
                    }
                    let mut merged_inv = active_inv.clone();
                    for (s, v) in &inv {
                        merged_inv.insert((s.clone(), v.clone()));
                        loop_env
                            .valid_boundary
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                        loop_env
                            .le_len
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                    }
                    let mut body_env = loop_env.clone();
                    walk_stmts(
                        loop_body,
                        &mut body_env,
                        u32_locals,
                        known_ascii,
                        captured,
                        &merged_inv,
                        rewrite,
                        inv_check,
                    );
                    *env = loop_env;
                }
                Stmt::While(test, loop_body) => {
                    let mut modified = BTreeSet::new();
                    collect_assigned_vars_stmts(loop_body, &mut modified);
                    let mut inv: BTreeSet<(String, String)> = BTreeSet::new();
                    for (s, atom) in &env.valid_boundary {
                        if let IndexAtom::Var(v) = atom {
                            if !modified.contains(s)
                                && !captured.contains(s)
                                && !captured.contains(v)
                                && modified.contains(v)
                            {
                                inv.insert((s.clone(), v.clone()));
                            }
                        }
                    }
                    let mut candidate_strs: BTreeSet<String> = known_ascii.clone();
                    for s in env.len_alias.values() {
                        candidate_strs.insert(s.clone());
                    }
                    for s in env.str_alias.values() {
                        candidate_strs.insert(s.clone());
                    }
                    for (s, _) in &env.valid_boundary {
                        candidate_strs.insert(s.clone());
                    }
                    collect_candidate_strs_stmts(loop_body, env, &mut candidate_strs);
                    for (v, atom) in &env.idx_alias {
                        if *atom == IndexAtom::Const(0)
                            && modified.contains(v)
                            && !captured.contains(v)
                        {
                            for s in &candidate_strs {
                                if !modified.contains(s) && !captured.contains(s) {
                                    inv.insert((s.clone(), v.clone()));
                                }
                            }
                        }
                    }
                    for _ in 0..4 {
                        let prev_len = inv.len();
                        let mut check_env = env.clone();
                        for v in &modified {
                            check_env.kill_var(v);
                        }
                        let mut merged_inv = active_inv.clone();
                        for (s, v) in &inv {
                            if !modified.contains(s) {
                                merged_inv.insert((s.clone(), v.clone()));
                                check_env
                                    .valid_boundary
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                                check_env
                                    .le_len
                                    .insert((s.clone(), IndexAtom::Var(v.clone())));
                            }
                        }
                        apply_guard(&mut check_env, test, true, u32_locals);
                        let mut inv_ref = Some(&mut inv);
                        walk_stmts(
                            loop_body,
                            &mut check_env,
                            u32_locals,
                            known_ascii,
                            captured,
                            &merged_inv,
                            false,
                            &mut inv_ref,
                        );
                        if inv.len() == prev_len {
                            break;
                        }
                    }
                    if let Some(outer) = inv_check.as_deref_mut() {
                        outer.retain(|(s, v)| {
                            !modified.contains(v) || inv.contains(&(s.clone(), v.clone()))
                        });
                    }
                    let mut loop_env = env.clone();
                    for v in &modified {
                        loop_env.kill_var(v);
                    }
                    let mut merged_inv = active_inv.clone();
                    for (s, v) in &inv {
                        merged_inv.insert((s.clone(), v.clone()));
                        loop_env
                            .valid_boundary
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                        loop_env
                            .le_len
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                    }
                    walk_expr(
                        test,
                        &mut loop_env,
                        u32_locals,
                        known_ascii,
                        rewrite,
                        inv_check,
                    );
                    let mut body_env = loop_env.clone();
                    apply_guard(&mut body_env, test, true, u32_locals);
                    walk_stmts(
                        loop_body,
                        &mut body_env,
                        u32_locals,
                        known_ascii,
                        captured,
                        &merged_inv,
                        rewrite,
                        inv_check,
                    );
                    *env = loop_env;
                }
                Stmt::TryCatch {
                    body: try_body,
                    binding,
                    handler,
                } => {
                    let mut try_env = env.clone();
                    let mut modified = BTreeSet::new();
                    collect_assigned_vars_stmts(try_body, &mut modified);
                    modified.insert(binding.clone());
                    walk_stmts(
                        try_body,
                        &mut try_env,
                        u32_locals,
                        known_ascii,
                        captured,
                        active_inv,
                        rewrite,
                        inv_check,
                    );
                    let mut handler_env = env.clone();
                    for v in &modified {
                        handler_env.kill_var(v);
                    }
                    for (s, v) in active_inv {
                        handler_env
                            .valid_boundary
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                        handler_env
                            .le_len
                            .insert((s.clone(), IndexAtom::Var(v.clone())));
                    }
                    walk_stmts(
                        handler,
                        &mut handler_env,
                        u32_locals,
                        known_ascii,
                        captured,
                        active_inv,
                        rewrite,
                        inv_check,
                    );
                    if stmts_always_exit(handler) {
                        *env = try_env;
                    } else {
                        *env = SubEnv::intersect(&try_env, &handler_env);
                    }
                }
                _ => {}
            }
        }
    }

    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    let mut captured = BTreeSet::new();
    collect_closure_captured(body, &locals, &mut captured);
    let u32_locals = collect_u32_locals(body, &locals, &captured);
    let mut env = SubEnv::default();
    let empty_inv = BTreeSet::new();
    let mut no_check: Option<&mut BTreeSet<(String, String)>> = None;
    walk_stmts(
        body,
        &mut env,
        &u32_locals,
        known_ascii,
        &captured,
        &empty_inv,
        true,
        &mut no_check,
    );
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
            false,
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
            false,
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

    fn run_speed_passes(stmt: &mut Stmt) {
        rewrite(stmt);
        if let Stmt::FunctionDecl { body, .. } = stmt {
            elide_readonly_bstr_bytes(body);
            elide_ascii_only_bstr_in(body);
            let known_ascii = elide_known_ascii_bstr_out(body);
            elide_proven_bstr_sub(body, &known_ascii);
        }
    }

    #[test]
    fn widened_elide_ascii_only_bstr_in_handles_methods_charcodeat_and_cond() {
        let with_speed = run_speed_passes;

        // Positive 1: ASCII startsWith / endsWith / includes / charCodeAt(0) / length === 0.
        let mut f_ok = Stmt::FunctionDecl {
            name: "f_ok".to_owned(),
            params: vec!["s".to_owned()],
            body: vec![
                Stmt::Let("a".to_owned(), Some(rt("bstr_in", vec![jsast::id("s")]))),
                jsast::ret(jsast::binary(
                    jsast::BinOp::Or,
                    jsast::binary(
                        jsast::BinOp::Or,
                        jsast::method_call(jsast::id("a"), "startsWith", vec![jsast::string("+")]),
                        jsast::method_call(jsast::id("a"), "endsWith", vec![jsast::string("-")]),
                    ),
                    jsast::binary(
                        jsast::BinOp::Or,
                        jsast::method_call(jsast::id("a"), "includes", vec![jsast::string("0")]),
                        jsast::binary(
                            jsast::BinOp::Or,
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::method_call(
                                    jsast::id("a"),
                                    "charCodeAt",
                                    vec![jsast::num(0)],
                                ),
                                jsast::num(45),
                            ),
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::member(jsast::id("a"), "length"),
                                jsast::num(0),
                            ),
                        ),
                    ),
                )),
            ],
        };
        with_speed(&mut f_ok);
        let p_ok = jsast::stmt_to_string(&f_ok);
        assert!(!p_ok.contains("bstr_in"), "{p_ok}");

        // Positive 2: Cond arm with bstr_in + ASCII fallback literal, and inline bstr_in unwrap.
        let mut f_cond = Stmt::FunctionDecl {
            name: "f_cond".to_owned(),
            params: vec!["s".to_owned(), "flag".to_owned()],
            body: vec![
                Stmt::Let(
                    "a".to_owned(),
                    Some(jsast::cond(
                        jsast::id("flag"),
                        rt("bstr_in", vec![jsast::id("s")]),
                        jsast::string("0"),
                    )),
                ),
                jsast::ret(jsast::binary(
                    jsast::BinOp::Or,
                    jsast::binary(jsast::BinOp::StrictEq, jsast::id("a"), jsast::string("1")),
                    jsast::binary(
                        jsast::BinOp::Or,
                        jsast::method_call(
                            rt("bstr_in", vec![jsast::id("s")]),
                            "startsWith",
                            vec![jsast::string("abc")],
                        ),
                        jsast::binary(
                            jsast::BinOp::StrictNe,
                            jsast::member(rt("bstr_in", vec![jsast::id("s")]), "length"),
                            jsast::num(0),
                        ),
                    ),
                )),
            ],
        };
        with_speed(&mut f_cond);
        let p_cond = jsast::stmt_to_string(&f_cond);
        assert!(!p_cond.contains("bstr_in"), "{p_cond}");

        // Negative cases: length === 1, charCodeAt(1), non-ASCII literal, charCodeAt(0) === 128.
        let negatives = [
            jsast::binary(
                jsast::BinOp::StrictEq,
                jsast::member(jsast::id("a"), "length"),
                jsast::num(1),
            ),
            jsast::binary(
                jsast::BinOp::StrictEq,
                jsast::method_call(jsast::id("a"), "charCodeAt", vec![jsast::num(1)]),
                jsast::num(45),
            ),
            jsast::method_call(
                jsast::id("a"),
                "startsWith",
                vec![jsast::string("\u{00c3}\u{00a9}")],
            ),
            jsast::binary(
                jsast::BinOp::StrictEq,
                jsast::method_call(jsast::id("a"), "charCodeAt", vec![jsast::num(0)]),
                jsast::num(128),
            ),
        ];
        for (idx, ret_expr) in negatives.into_iter().enumerate() {
            let mut f_bad = Stmt::FunctionDecl {
                name: format!("f_bad_{idx}"),
                params: vec!["s".to_owned()],
                body: vec![
                    Stmt::Let("a".to_owned(), Some(rt("bstr_in", vec![jsast::id("s")]))),
                    jsast::ret(ret_expr),
                ],
            };
            with_speed(&mut f_bad);
            let p_bad = jsast::stmt_to_string(&f_bad);
            assert!(
                p_bad.contains("bstr_in"),
                "negative case {idx} must keep bstr_in: {p_bad}"
            );
        }
    }

    #[test]
    fn elide_known_ascii_bstr_out_tracks_scalar_and_native_string_objects() {
        let with_speed = run_speed_passes;

        // Positive 1: scalar string built from literals, bounded fromCharCode, toString, slice.
        let mut f_scalar = Stmt::FunctionDecl {
            name: "fmt_scalar".to_owned(),
            params: vec!["d".to_owned(), "n".to_owned()],
            body: vec![
                Stmt::Let("s".to_owned(), Some(jsast::string(""))),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::id("s"),
                    jsast::string("-"),
                )),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::id("s"),
                    jsast::method_call(
                        jsast::id("String"),
                        "fromCharCode",
                        vec![jsast::binary(
                            jsast::BinOp::UShr,
                            jsast::binary(
                                jsast::BinOp::Add,
                                jsast::num(48),
                                jsast::binary(jsast::BinOp::BitAnd, jsast::id("d"), jsast::num(7)),
                            ),
                            jsast::num(0),
                        )],
                    ),
                )),
                jsast::assign_stmt(
                    jsast::id("s"),
                    jsast::binary(
                        jsast::BinOp::Add,
                        jsast::id("s"),
                        jsast::method_call(
                            jsast::binary(jsast::BinOp::BitOr, jsast::id("n"), jsast::num(0)),
                            "toString",
                            Vec::new(),
                        ),
                    ),
                ),
                Stmt::Let(
                    "t".to_owned(),
                    Some(jsast::method_call(
                        jsast::id("s"),
                        "slice",
                        vec![jsast::num(0), jsast::num(2)],
                    )),
                ),
                jsast::ret(jsast::array(vec![
                    rt("bstr_out", vec![jsast::id("s")]),
                    rt("_bstr_is_ascii", vec![jsast::id("s")]),
                    rt("bstr_to_lowercase", vec![jsast::id("t")]),
                ])),
            ],
        };
        with_speed(&mut f_scalar);
        let p_scalar = jsast::stmt_to_string(&f_scalar);
        assert!(!p_scalar.contains("bstr_out"), "{p_scalar}");
        assert!(!p_scalar.contains("_bstr_is_ascii"), "{p_scalar}");
        assert!(!p_scalar.contains("bstr_to_lowercase"), "{p_scalar}");
        assert!(p_scalar.contains("t.toLowerCase()"), "{p_scalar}");
        assert!(p_scalar.contains("[s, true,"), "{p_scalar}");

        // Positive 2: local native `{ s, cap }` String object mutated in place.
        let mut f_obj = Stmt::FunctionDecl {
            name: "fmt_obj".to_owned(),
            params: vec!["d".to_owned()],
            body: vec![
                Stmt::Let(
                    "o".to_owned(),
                    Some(jsast::object_of(vec![
                        ("s", jsast::string("")),
                        ("cap", jsast::num(0)),
                    ])),
                ),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::member(jsast::id("o"), "s"),
                    jsast::string("0."),
                )),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::member(jsast::id("o"), "s"),
                    jsast::method_call(
                        jsast::id("String"),
                        "fromCharCode",
                        vec![jsast::binary(
                            jsast::BinOp::Add,
                            jsast::num(48),
                            jsast::binary(jsast::BinOp::Rem, jsast::id("d"), jsast::num(10)),
                        )],
                    ),
                )),
                jsast::ret(rt("bstr_out", vec![jsast::member(jsast::id("o"), "s")])),
            ],
        };
        with_speed(&mut f_obj);
        let p_obj = jsast::stmt_to_string(&f_obj);
        assert!(!p_obj.contains("bstr_out"), "{p_obj}");
        assert!(p_obj.contains("return o.s;"), "{p_obj}");

        // Negative 1: non-ASCII concat keeps bstr_out.
        let mut f_non_ascii = Stmt::FunctionDecl {
            name: "non_ascii".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Let("s".to_owned(), Some(jsast::string("a"))),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::id("s"),
                    jsast::string("\u{00c3}\u{00a9}"),
                )),
                jsast::ret(rt("bstr_out", vec![jsast::id("s")])),
            ],
        };
        with_speed(&mut f_non_ascii);
        assert!(
            jsast::stmt_to_string(&f_non_ascii).contains("bstr_out"),
            "{f_non_ascii}"
        );

        // Negative 2: unbounded String.fromCharCode(x) keeps bstr_out.
        let mut f_dyn = Stmt::FunctionDecl {
            name: "dyn_cc".to_owned(),
            params: vec!["x".to_owned()],
            body: vec![
                Stmt::Let(
                    "s".to_owned(),
                    Some(jsast::method_call(
                        jsast::id("String"),
                        "fromCharCode",
                        vec![jsast::id("x")],
                    )),
                ),
                jsast::ret(rt("bstr_out", vec![jsast::id("s")])),
            ],
        };
        with_speed(&mut f_dyn);
        assert!(
            jsast::stmt_to_string(&f_dyn).contains("bstr_out"),
            "{f_dyn}"
        );

        // Negative 3: escaped native `{ s, cap }` object keeps bstr_out.
        let mut f_esc = Stmt::FunctionDecl {
            name: "esc_obj".to_owned(),
            params: vec!["ext".to_owned()],
            body: vec![
                Stmt::Let(
                    "o".to_owned(),
                    Some(jsast::object_of(vec![
                        ("s", jsast::string("abc")),
                        ("cap", jsast::num(3)),
                    ])),
                ),
                jsast::expr_stmt(jsast::call(jsast::id("ext"), vec![jsast::id("o")])),
                jsast::ret(rt("bstr_out", vec![jsast::member(jsast::id("o"), "s")])),
            ],
        };
        with_speed(&mut f_esc);
        assert!(
            jsast::stmt_to_string(&f_esc).contains("bstr_out"),
            "{f_esc}"
        );

        // Negative 4: ungrounded self-cycle keeps bstr_out.
        let mut f_cyc = Stmt::FunctionDecl {
            name: "cyc".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Let("a".to_owned(), None),
                Stmt::Let("b".to_owned(), None),
                jsast::assign_stmt(jsast::id("a"), jsast::id("b")),
                jsast::assign_stmt(jsast::id("b"), jsast::id("a")),
                jsast::ret(rt("bstr_out", vec![jsast::id("a")])),
            ],
        };
        with_speed(&mut f_cyc);
        assert!(
            jsast::stmt_to_string(&f_cyc).contains("bstr_out"),
            "{f_cyc}"
        );
    }

    #[test]
    fn elide_proven_bstr_sub_folds_guarded_and_loop_invariant_slices_soundly() {
        let with_speed = run_speed_passes;

        // Positive 1: dominating `s.charCodeAt(0) === 35` proves `bstr_sub_from(s, 1)` -> `s.slice(1)`.
        let mut f_prefix = Stmt::FunctionDecl {
            name: "strip_hash".to_owned(),
            params: vec!["s".to_owned()],
            body: vec![
                jsast::if_(
                    jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::num(0)]),
                        jsast::num(35),
                    ),
                    vec![jsast::ret(rt(
                        "bstr_sub_from",
                        vec![
                            jsast::id("s"),
                            jsast::num(1),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ))],
                ),
                jsast::ret(jsast::id("s")),
            ],
        };
        with_speed(&mut f_prefix);
        let p_prefix = jsast::stmt_to_string(&f_prefix);
        assert!(p_prefix.contains("return s.slice(1);"), "{p_prefix}");
        assert!(!p_prefix.contains("bstr_sub"), "{p_prefix}");

        // Positive 2: `marked` `expand_tabs` loop-invariant `d` + `{ start, end }` range object.
        let mut f_tabs = Stmt::FunctionDecl {
            name: "expand_tabs".to_owned(),
            params: vec!["a".to_owned()],
            body: vec![
                Stmt::Let("d".to_owned(), Some(jsast::num(0))),
                Stmt::Let("c".to_owned(), Some(jsast::num(0))),
                Stmt::Let("out".to_owned(), Some(jsast::string(""))),
                Stmt::LoopForever(
                    None,
                    vec![jsast::if_else(
                        jsast::binary(
                            jsast::BinOp::Lt,
                            jsast::id("c"),
                            jsast::member(jsast::id("a"), "length"),
                        ),
                        vec![
                            jsast::if_(
                                jsast::binary(
                                    jsast::BinOp::StrictEq,
                                    jsast::method_call(
                                        jsast::id("a"),
                                        "charCodeAt",
                                        vec![jsast::id("c")],
                                    ),
                                    jsast::num(9),
                                ),
                                vec![
                                    jsast::if_(
                                        jsast::binary(
                                            jsast::BinOp::Gt,
                                            jsast::id("c"),
                                            jsast::id("d"),
                                        ),
                                        vec![
                                            Stmt::Let(
                                                "w".to_owned(),
                                                Some(jsast::object_of(vec![
                                                    ("start", jsast::id("d")),
                                                    ("end", jsast::id("c")),
                                                ])),
                                            ),
                                            jsast::expr_stmt(jsast::compound_assign(
                                                jsast::BinOp::Add,
                                                jsast::id("out"),
                                                rt(
                                                    "bstr_sub",
                                                    vec![
                                                        jsast::id("a"),
                                                        jsast::member(jsast::id("w"), "start"),
                                                        jsast::member(jsast::id("w"), "end"),
                                                        jsast::id("fail"),
                                                        jsast::id("loc"),
                                                    ],
                                                ),
                                            )),
                                        ],
                                    ),
                                    jsast::assign_stmt(
                                        jsast::id("d"),
                                        jsast::binary(
                                            jsast::BinOp::UShr,
                                            jsast::binary(
                                                jsast::BinOp::Add,
                                                jsast::id("c"),
                                                jsast::num(1),
                                            ),
                                            jsast::num(0),
                                        ),
                                    ),
                                ],
                            ),
                            jsast::assign_stmt(
                                jsast::id("c"),
                                jsast::binary(
                                    jsast::BinOp::UShr,
                                    jsast::binary(jsast::BinOp::Add, jsast::id("c"), jsast::num(1)),
                                    jsast::num(0),
                                ),
                            ),
                        ],
                        vec![Stmt::Break(None)],
                    )],
                ),
                jsast::ret(rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("a"),
                        jsast::id("d"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                )),
            ],
        };
        with_speed(&mut f_tabs);
        let p_tabs = jsast::stmt_to_string(&f_tabs);
        assert!(!p_tabs.contains("bstr_sub"), "{p_tabs}");
        assert!(p_tabs.contains("a.slice(w.start, w.end)"), "{p_tabs}");
        assert!(p_tabs.contains("a.slice(d)"), "{p_tabs}");

        // Positive 3: `rtrim_byte` pattern (`c = a.length`, `c > 0 && a.charCodeAt((c - 1) >>> 0) === 32`).
        let mut f_rtrim = Stmt::FunctionDecl {
            name: "rtrim_byte".to_owned(),
            params: vec!["a".to_owned()],
            body: vec![
                Stmt::Let(
                    "c".to_owned(),
                    Some(jsast::member(jsast::id("a"), "length")),
                ),
                Stmt::LoopForever(
                    None,
                    vec![jsast::if_else(
                        jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::Gt, jsast::id("c"), jsast::num(0)),
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::method_call(
                                    jsast::id("a"),
                                    "charCodeAt",
                                    vec![jsast::binary(
                                        jsast::BinOp::UShr,
                                        jsast::binary(
                                            jsast::BinOp::Sub,
                                            jsast::id("c"),
                                            jsast::num(1),
                                        ),
                                        jsast::num(0),
                                    )],
                                ),
                                jsast::num(32),
                            ),
                        ),
                        vec![jsast::assign_stmt(
                            jsast::id("c"),
                            jsast::binary(
                                jsast::BinOp::UShr,
                                jsast::binary(jsast::BinOp::Sub, jsast::id("c"), jsast::num(1)),
                                jsast::num(0),
                            ),
                        )],
                        vec![Stmt::Break(None)],
                    )],
                ),
                jsast::ret(rt(
                    "bstr_sub_to",
                    vec![
                        jsast::id("a"),
                        jsast::id("c"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                )),
            ],
        };
        with_speed(&mut f_rtrim);
        let p_rtrim = jsast::stmt_to_string(&f_rtrim);
        assert!(!p_rtrim.contains("bstr_sub"), "{p_rtrim}");
        assert!(p_rtrim.contains("a.slice(0, c)"), "{p_rtrim}");

        // Positive 4: trivial `0`/`s.length` and KnownAscii + bounds guard.
        let mut f_triv = Stmt::FunctionDecl {
            name: "triv".to_owned(),
            params: vec!["x".to_owned(), "i".to_owned()],
            body: vec![
                Stmt::Let("s".to_owned(), Some(jsast::string("hello"))),
                jsast::if_(
                    jsast::binary(
                        jsast::BinOp::And,
                        jsast::binary(jsast::BinOp::Ge, jsast::id("i"), jsast::num(0)),
                        jsast::binary(
                            jsast::BinOp::Le,
                            jsast::id("i"),
                            jsast::member(jsast::id("s"), "length"),
                        ),
                    ),
                    vec![jsast::ret(rt(
                        "bstr_sub_from",
                        vec![
                            jsast::id("s"),
                            jsast::id("i"),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ))],
                ),
                jsast::ret(jsast::array(vec![
                    rt(
                        "bstr_sub_from",
                        vec![
                            jsast::id("x"),
                            jsast::num(0),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ),
                    rt(
                        "bstr_sub",
                        vec![
                            jsast::id("x"),
                            jsast::num(0),
                            jsast::member(jsast::id("x"), "length"),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ),
                    rt(
                        "bstr_sub_to",
                        vec![
                            jsast::id("x"),
                            jsast::num(0),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ),
                ])),
            ],
        };
        with_speed(&mut f_triv);
        let p_triv = jsast::stmt_to_string(&f_triv);
        assert!(!p_triv.contains("bstr_sub"), "{p_triv}");
        assert!(p_triv.contains("return s.slice(i);"), "{p_triv}");
        assert!(p_triv.contains("[x, x, \"\"]"), "{p_triv}");

        // Negative 1: non-ASCII byte check (128) must NOT fold `bstr_sub_from(s, 1)`.
        let mut f_bad_byte = Stmt::FunctionDecl {
            name: "bad_byte".to_owned(),
            params: vec!["s".to_owned()],
            body: vec![
                jsast::if_(
                    jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::num(0)]),
                        jsast::num(128),
                    ),
                    vec![jsast::ret(rt(
                        "bstr_sub_from",
                        vec![
                            jsast::id("s"),
                            jsast::num(1),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ))],
                ),
                jsast::ret(jsast::id("s")),
            ],
        };
        with_speed(&mut f_bad_byte);
        assert!(
            jsast::stmt_to_string(&f_bad_byte).contains("bstr_sub_from"),
            "{f_bad_byte}"
        );

        // Negative 2: KnownAscii without proven `a <= b <= s.length` must NOT fold `bstr_sub`.
        let mut f_bad_bounds = Stmt::FunctionDecl {
            name: "bad_bounds".to_owned(),
            params: vec!["a".to_owned(), "b".to_owned()],
            body: vec![
                Stmt::Let("s".to_owned(), Some(jsast::string("hello"))),
                jsast::ret(rt(
                    "bstr_sub",
                    vec![
                        jsast::id("s"),
                        jsast::id("a"),
                        jsast::id("b"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                )),
            ],
        };
        with_speed(&mut f_bad_bounds);
        assert!(
            jsast::stmt_to_string(&f_bad_bounds).contains("bstr_sub"),
            "{f_bad_bounds}"
        );

        // Negative 3: u32 wraparound: checking `s.charCodeAt((b + 1) >>> 0) === 93` without
        // proving `b < s.length` must NOT prove `b` is a valid boundary (b could be 0xFFFF_FFFF).
        let mut f_wrap = Stmt::FunctionDecl {
            name: "bad_wrap".to_owned(),
            params: vec!["s".to_owned(), "b".to_owned()],
            body: vec![
                jsast::if_(
                    jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::method_call(
                            jsast::id("s"),
                            "charCodeAt",
                            vec![jsast::binary(
                                jsast::BinOp::UShr,
                                jsast::binary(jsast::BinOp::Add, jsast::id("b"), jsast::num(1)),
                                jsast::num(0),
                            )],
                        ),
                        jsast::num(93),
                    ),
                    vec![jsast::ret(rt(
                        "bstr_sub_from",
                        vec![
                            jsast::id("s"),
                            jsast::id("b"),
                            jsast::id("fail"),
                            jsast::id("loc"),
                        ],
                    ))],
                ),
                jsast::ret(jsast::id("s")),
            ],
        };
        with_speed(&mut f_wrap);
        assert!(
            jsast::stmt_to_string(&f_wrap).contains("bstr_sub_from"),
            "{f_wrap}"
        );
    }
}
