//! Layout-driven POD codec for generic `transmute`, `raw_eq`, and equal-size POD pointer views.
//!
//! Operates purely on [`TypeTable`] layout facts (byte size, field byte offsets, scalar kinds,
//! arrays, tuples, structs, fieldless enums, transparent/pattern wrappers, `MaybeUninit`,
//! `NonZero`, and niche enums). Emits zero code when unused, uses ES2018-safe `DataView`
//! little-endian byte packing/unpacking (32-bit word pairs for 64/128-bit `BigInt`s), and
//! automatically skips padding bytes in both `pod_transmute` and `pod_raw_eq`.

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::types::{
    EnumRepr, FieldDescriptor, FieldKey, IntRepr, Niche, NominalForm, RepresentationCandidate,
    TypeId, TypeKind, TypeTable, VariantDescriptor,
};
use rcgjs_object::jsast::{self, BinOp, CompoundOp, Expr, Stmt};

use super::unsize::POINTER_BITS;
use super::{value, JsCx};

/// Maximum recursion depth for POD layout classification and emission.
const MAX_POD_DEPTH: usize = 24;

/// Parameter bound to the incoming source value in `pod_transmute`.
const POD_IN: &str = "$p";
/// Local holding the `DataView` over the scratch `ArrayBuffer`.
const POD_DV: &str = "$dv";

/// Leaf scalar representation within a POD layout.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PodScalar {
    Int {
        signed: bool,
        bits: u32,
    },
    Float {
        bits: u32,
    },
    Bool,
    Char,
    /// Fieldless enum whose JS value is its integer discriminant (`EnumRepr::Number`).
    NumberEnum {
        signed: bool,
        bits: u32,
    },
    /// Thin pointer (32-bit address on wasm32). Unpacking into raw/fn pointers produces a numeric
    /// address; unpacking into references is forbidden.
    ThinPtr {
        ty: TypeId,
        pointee: Option<TypeId>,
        is_ref: bool,
    },
}

/// How a field of a composite POD type is accessed on a JS value.
#[derive(Clone, Debug)]
pub(crate) enum PodFieldAccess {
    Name(String),
    Index(u32),
}

/// One non-ZST field inside a POD struct or tuple, with its exact Rust byte offset.
#[derive(Clone, Debug)]
pub(crate) struct PodField {
    pub access: PodFieldAccess,
    pub offset: u64,
    pub node: PodNode,
}

/// Recursive byte-layout description of a POD (or POD + thin raw-pointer) Rust type.
#[derive(Clone, Debug)]
pub(crate) enum PodNode {
    Zst,
    Scalar(PodScalar),
    Array {
        elem: Box<PodNode>,
        elem_size: u64,
        length: u64,
    },
    Tuple {
        arity: usize,
        fields: Vec<PodField>,
    },
    Struct {
        zst_keys: Vec<String>,
        fields: Vec<PodField>,
    },
    /// Niche-encoded enum whose inhabited niche variants are ZST and whose untagged variant holds
    /// a single non-ZST POD payload field.
    NicheEnum {
        ty: TypeId,
        repr: EnumRepr,
        niche_bits: u32,
        niche_offset: u64,
        niche_variants: Vec<(u32, u128)>,
        untagged_variant: u32,
        payload_key: FieldKey,
        payload: Box<PodNode>,
    },
    /// Fieldless enum with `physical_tag` recorded on every variant (`Tag` / `IntTag`).
    FieldlessEnum {
        ty: TypeId,
        tag_bytes: u8,
        variants: Vec<(u32, u128)>,
    },
}

/// Controls which leaf kinds are accepted by [`classify_pod`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PodPolicy {
    /// Allow thin pointers (`*const T`, `*mut T`, `fn(...)`, `&T` when packing) as 32-bit words.
    pub allow_thin_ptr: bool,
    /// Require that every thin pointer can be unpacked from raw bytes (forbids references).
    pub for_unpack: bool,
}

impl PodNode {
    /// Whether this node (and all its leaves) can be compared with JS `===` for bit-exact `raw_eq`.
    fn compares_strict_eq(&self) -> bool {
        match self {
            PodNode::Zst => true,
            PodNode::Scalar(
                PodScalar::Int { .. }
                | PodScalar::Bool
                | PodScalar::Char
                | PodScalar::NumberEnum { .. },
            ) => true,
            PodNode::FieldlessEnum { .. } => true,
            PodNode::NicheEnum { repr, payload, .. } => {
                repr.is_null_or_sentinel_niche() && payload.compares_strict_eq()
            }
            PodNode::Scalar(PodScalar::Float { .. } | PodScalar::ThinPtr { .. })
            | PodNode::Array { .. }
            | PodNode::Tuple { .. }
            | PodNode::Struct { .. } => false,
        }
    }
}

/// Byte offset and bit width of the scalar holding an enum's niche tag, walking `niche.niche_field`.
pub(crate) fn niche_field_offset_and_width(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: &Niche,
) -> Option<(u64, u32)> {
    enum StepFields<'a> {
        Fields(&'a [FieldDescriptor]),
        Single(TypeId),
        Array(TypeId, u64),
    }

    let mut step_fields =
        StepFields::Fields(&variants.get(niche.untagged_variant as usize)?.fields);
    let mut offset = 0u64;
    let mut leaf_ty = None;
    for &step in &niche.niche_field {
        let next_ty = match step_fields {
            StepFields::Single(base) => {
                if step != 0 {
                    return None;
                }
                base
            }
            StepFields::Array(element, length) => {
                if u64::from(step) >= length {
                    return None;
                }
                let stride = value::size_of(table, element)?;
                offset = offset.checked_add(u64::from(step).checked_mul(stride)?)?;
                element
            }
            StepFields::Fields(fields) => {
                let field = fields.get(step as usize)?;
                offset = offset.checked_add(field.offset)?;
                field.ty
            }
        };
        leaf_ty = Some(next_ty);
        let desc = table.get(next_ty)?;
        step_fields = match &desc.kind {
            TypeKind::Pattern(base) => StepFields::Single(*base),
            TypeKind::Array { element, length } => StepFields::Array(*element, *length),
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants: inner_variants,
                ..
            } => {
                let mut inhabited = inner_variants
                    .iter()
                    .filter(|v| !v.fields.iter().any(|f| value::is_uninhabited(table, f.ty)));
                match (inhabited.next(), inhabited.next()) {
                    (Some(sole), None) => StepFields::Fields(sole.fields.as_slice()),
                    _ => StepFields::Fields(&[]),
                }
            }
            TypeKind::Adt {
                variants: inner_variants,
                ..
            } => StepFields::Fields(inner_variants.first().map_or(&[], |v| v.fields.as_slice())),
            _ => StepFields::Fields(&desc.js.fields),
        };
    }
    leaf_ty?;
    let bits = value::niche_width(table, variants, niche)?;
    matches!(bits, 8 | 16 | 32 | 64 | 128).then_some((offset, bits))
}

/// Classifies `ty` into a recursive [`PodNode`] layout tree, or `None` if `ty` has an
/// unrepresentable / non-POD shape (unsized, fat pointer, non-transparent union, multi-payload
/// enum, native `Vec`/`String`/`BTreeMap` carrier, or missing field offsets).
pub(crate) fn classify_pod(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    policy: PodPolicy,
) -> Option<PodNode> {
    classify_pod_at(table, interns, ty, policy, 0)
}

fn classify_pod_at(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    policy: PodPolicy,
    depth: usize,
) -> Option<PodNode> {
    if depth >= MAX_POD_DEPTH {
        return None;
    }
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if !desc.layout.sized {
        return None;
    }
    if desc.layout.zst {
        return Some(PodNode::Zst);
    }
    let total_size = desc.layout.size?;

    // Never reinterpret a type whose JS shape may be replaced by a native host/typed carrier.
    if desc.meta.candidate != RepresentationCandidate::None {
        return None;
    }

    if let Some((signed, bits)) = table.int_info(peeled) {
        return matches!(bits, 8 | 16 | 32 | 64 | 128)
            .then_some(PodNode::Scalar(PodScalar::Int { signed, bits }));
    }

    match &desc.kind {
        TypeKind::Bool => Some(PodNode::Scalar(PodScalar::Bool)),
        TypeKind::Char => Some(PodNode::Scalar(PodScalar::Char)),
        TypeKind::Float { bits } if matches!(*bits, 32 | 64) => {
            Some(PodNode::Scalar(PodScalar::Float { bits: *bits }))
        }
        TypeKind::FnPtr { .. } if policy.allow_thin_ptr && total_size == 4 => {
            Some(PodNode::Scalar(PodScalar::ThinPtr {
                ty: peeled,
                pointee: None,
                is_ref: false,
            }))
        }
        TypeKind::RawPointer { pointee, .. }
            if policy.allow_thin_ptr
                && total_size == 4
                && table.get(*pointee).is_some_and(|d| d.layout.sized) =>
        {
            Some(PodNode::Scalar(PodScalar::ThinPtr {
                ty: peeled,
                pointee: Some(*pointee),
                is_ref: false,
            }))
        }
        TypeKind::Reference { pointee, .. }
            if policy.allow_thin_ptr
                && !policy.for_unpack
                && total_size == 4
                && table.get(*pointee).is_some_and(|d| d.layout.sized) =>
        {
            Some(PodNode::Scalar(PodScalar::ThinPtr {
                ty: peeled,
                pointee: Some(*pointee),
                is_ref: true,
            }))
        }
        TypeKind::Array { element, length } => {
            let elem_size = value::size_of(table, *element)?;
            if elem_size.checked_mul(*length) != Some(total_size) {
                return None;
            }
            let elem = classify_pod_at(table, interns, *element, policy, depth + 1)?;
            Some(PodNode::Array {
                elem: Box::new(elem),
                elem_size,
                length: *length,
            })
        }
        TypeKind::Tuple(elems) => {
            if elems.is_empty() {
                return Some(PodNode::Zst);
            }
            if desc.js.fields.len() != elems.len() {
                return None;
            }
            let mut fields = Vec::with_capacity(elems.len());
            for (i, fdesc) in desc.js.fields.iter().enumerate() {
                if table.is_zst(fdesc.ty) {
                    continue;
                }
                let fsize = value::size_of(table, fdesc.ty)?;
                if fdesc.offset.checked_add(fsize)? > total_size {
                    return None;
                }
                let idx = match fdesc.key {
                    FieldKey::Index(idx) => idx,
                    _ => i as u32,
                };
                let node = classify_pod_at(table, interns, fdesc.ty, policy, depth + 1)?;
                fields.push(PodField {
                    access: PodFieldAccess::Index(idx),
                    offset: fdesc.offset,
                    node,
                });
            }
            Some(PodNode::Tuple {
                arity: elems.len(),
                fields,
            })
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let [variant] = variants.as_slice() else {
                return None;
            };
            let mut fields = Vec::with_capacity(variant.fields.len());
            let mut zst_keys = Vec::new();
            for fdesc in &variant.fields {
                let key = value::key_name(interns, fdesc.key)?;
                if table.is_zst(fdesc.ty) {
                    zst_keys.push(key);
                    continue;
                }
                let fsize = value::size_of(table, fdesc.ty)?;
                if fdesc.offset.checked_add(fsize)? > total_size {
                    return None;
                }
                let node = classify_pod_at(table, interns, fdesc.ty, policy, depth + 1)?;
                fields.push(PodField {
                    access: PodFieldAccess::Name(key),
                    offset: fdesc.offset,
                    node,
                });
            }
            Some(PodNode::Struct { zst_keys, fields })
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            niche,
            ..
        } => {
            if let Some(EnumRepr::Number { bits, signed }) = desc.js.enum_repr {
                if matches!(bits, 8 | 16 | 32 | 64 | 128) && u64::from(bits / 8) == total_size {
                    return Some(PodNode::Scalar(PodScalar::NumberEnum { signed, bits }));
                }
            }
            // Fieldless enum with physical direct tags on every variant.
            if let Some(EnumRepr::Tag { fieldless: true } | EnumRepr::IntTag { fieldless: true }) =
                desc.js.enum_repr
            {
                if !variants.is_empty() && variants.iter().all(|v| v.physical_tag.is_some()) {
                    let tag_bytes = variants[0].physical_tag?.bytes;
                    if matches!(tag_bytes, 1 | 2 | 4)
                        && u64::from(tag_bytes) == total_size
                        && variants
                            .iter()
                            .all(|v| v.physical_tag.is_some_and(|t| t.bytes == tag_bytes))
                    {
                        let entries = variants
                            .iter()
                            .enumerate()
                            .map(|(i, v)| (i as u32, v.physical_tag.unwrap().bits))
                            .collect();
                        return Some(PodNode::FieldlessEnum {
                            ty: peeled,
                            tag_bytes,
                            variants: entries,
                        });
                    }
                }
            }
            // Niche-encoded enum with ZST niche variants and a single non-ZST POD payload field.
            let niche = niche.as_ref()?;
            let repr = desc.js.enum_repr?;
            let (niche_offset, niche_bits) = niche_field_offset_and_width(table, variants, niche)?;
            if niche_offset.checked_add(u64::from(niche_bits / 8))? > total_size {
                return None;
            }
            let tags = value::niche_variant_tags(table, variants, niche, niche_bits)?;
            for &(vi, _) in &tags {
                let empty = variants.get(vi as usize)?;
                if empty.fields.iter().any(|f| !table.is_zst(f.ty)) {
                    return None;
                }
                value::variant_tag(table, interns, peeled, vi)?;
                value::enum_value(table, interns, peeled, vi, Vec::new())?;
            }
            let untagged = variants.get(niche.untagged_variant as usize)?;
            let mut live = untagged.fields.iter().filter(|f| !table.is_zst(f.ty));
            let payload_field = live.next()?;
            if live.next().is_some() || payload_field.offset != 0 {
                return None;
            }
            let payload_size = value::size_of(table, payload_field.ty)?;
            if payload_size != total_size {
                return None;
            }
            let payload = classify_pod_at(table, interns, payload_field.ty, policy, depth + 1)?;
            Some(PodNode::NicheEnum {
                ty: peeled,
                repr,
                niche_bits,
                niche_offset,
                niche_variants: tags,
                untagged_variant: niche.untagged_variant,
                payload_key: payload_field.key,
                payload: Box::new(payload),
            })
        }
        _ => None,
    }
}

/// Byte offset expression inside a `DataView`: either a compile-time constant or `dyn_expr + add`.
#[derive(Clone)]
enum OffExpr {
    Const(u64),
    Dyn(Expr, u64),
}

impl OffExpr {
    fn add_const(&self, delta: u64) -> Self {
        if delta == 0 {
            return self.clone();
        }
        match self {
            OffExpr::Const(c) => OffExpr::Const(c + delta),
            OffExpr::Dyn(e, c) => OffExpr::Dyn(e.clone(), c + delta),
        }
    }

    fn add_scaled(&self, idx: Expr, stride: u64) -> Self {
        let term = if stride == 1 {
            idx
        } else {
            jsast::binary(BinOp::Mul, idx, jsast::num(stride as f64))
        };
        match self {
            OffExpr::Const(c) => OffExpr::Dyn(term, *c),
            OffExpr::Dyn(e, c) => OffExpr::Dyn(jsast::binary(BinOp::Add, e.clone(), term), *c),
        }
    }

    fn to_expr(&self) -> Expr {
        match self {
            OffExpr::Const(c) => jsast::num(*c as f64),
            OffExpr::Dyn(e, 0) => e.clone(),
            OffExpr::Dyn(e, c) => jsast::binary(BinOp::Add, jsast::num(*c as f64), e.clone()),
        }
    }
}

/// Fresh local variable counter for one emitted `pod_transmute` IIFE body.
struct CodecGen<'a, 'b> {
    cx: &'a JsCx<'b>,
    next_id: u32,
}

impl<'a, 'b> CodecGen<'a, 'b> {
    fn new(cx: &'a JsCx<'b>) -> Self {
        Self { cx, next_id: 0 }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("${prefix}{n}")
    }

    fn bind_if_needed(&mut self, prefix: &str, expr: Expr, stmts: &mut Vec<Stmt>) -> Expr {
        if matches!(expr, Expr::Ident(_)) {
            expr
        } else {
            let name = self.fresh(prefix);
            stmts.push(jsast::const_(name.clone(), expr));
            jsast::id(name)
        }
    }

    /// Emits `DataView` writes packing `val` (of layout `node`) at byte offset `off`.
    fn emit_pack(&mut self, stmts: &mut Vec<Stmt>, off: &OffExpr, node: &PodNode, val: Expr) {
        let dv = jsast::id(POD_DV);
        match node {
            PodNode::Zst => {}
            PodNode::Scalar(scalar) => match *scalar {
                PodScalar::Int { signed, bits, .. }
                | PodScalar::NumberEnum { signed, bits, .. } => {
                    self.emit_pack_int(stmts, off, signed, bits, val);
                }
                PodScalar::Float { bits } => {
                    let method = if bits == 32 {
                        "setFloat32"
                    } else {
                        "setFloat64"
                    };
                    stmts.push(jsast::expr_stmt(jsast::method_call(
                        dv,
                        method,
                        vec![off.to_expr(), val, Expr::Bool(true)],
                    )));
                }
                PodScalar::Bool => {
                    let byte = jsast::cond(val, jsast::num(1), jsast::num(0));
                    stmts.push(jsast::expr_stmt(jsast::method_call(
                        dv,
                        "setUint8",
                        vec![off.to_expr(), byte],
                    )));
                }
                PodScalar::Char => {
                    stmts.push(jsast::expr_stmt(jsast::method_call(
                        dv,
                        "setUint32",
                        vec![off.to_expr(), val, Expr::Bool(true)],
                    )));
                }
                PodScalar::ThinPtr {
                    pointee, is_ref, ..
                } => {
                    let addr = self.cx.pod_thin_ptr_addr(pointee, is_ref, val);
                    stmts.push(jsast::expr_stmt(jsast::method_call(
                        dv,
                        "setUint32",
                        vec![off.to_expr(), addr, Expr::Bool(true)],
                    )));
                }
            },
            PodNode::Array {
                elem,
                elem_size,
                length,
            } => {
                if *length == 0 {
                    return;
                }
                let arr = self.bind_if_needed("pa", val, stmts);
                let idx_name = self.fresh("i");
                let idx = jsast::id(idx_name.clone());
                stmts.push(jsast::let_(idx_name, jsast::num(0)));
                let mut body = Vec::new();
                let elem_off = off.add_scaled(idx.clone(), *elem_size);
                self.emit_pack(&mut body, &elem_off, elem, jsast::index(arr, idx.clone()));
                body.push(jsast::expr_stmt(jsast::compound_assign(
                    CompoundOp::Add,
                    idx.clone(),
                    jsast::num(1),
                )));
                stmts.push(jsast::while_(
                    jsast::binary(BinOp::Lt, idx, jsast::num(*length as f64)),
                    body,
                ));
            }
            PodNode::Tuple { fields, .. } | PodNode::Struct { fields, .. } => {
                if fields.is_empty() {
                    return;
                }
                let base = if fields.len() == 1 {
                    val
                } else {
                    self.bind_if_needed("ps", val, stmts)
                };
                for field in fields {
                    let read = match &field.access {
                        PodFieldAccess::Name(name) => jsast::member(base.clone(), name.clone()),
                        PodFieldAccess::Index(idx) => {
                            jsast::index(base.clone(), jsast::num(f64::from(*idx)))
                        }
                    };
                    self.emit_pack(stmts, &off.add_const(field.offset), &field.node, read);
                }
            }
            PodNode::FieldlessEnum {
                ty,
                tag_bytes,
                variants,
            } => {
                let bound = self.bind_if_needed("pe", val, stmts);
                let Some((_, last_bits)) = variants.last() else {
                    return;
                };
                let mut expr = jsast::num(*last_bits as f64);
                for &(vi, raw_bits) in variants[..variants.len() - 1].iter().rev() {
                    if let Some(tag_lit) =
                        value::variant_tag(self.cx.table, self.cx.interns, *ty, vi)
                    {
                        expr = jsast::cond(
                            jsast::binary(BinOp::StrictEq, bound.clone(), tag_lit),
                            jsast::num(raw_bits as f64),
                            expr,
                        );
                    }
                }
                self.emit_pack_int(stmts, off, false, u32::from(*tag_bytes) * 8, expr);
            }
            PodNode::NicheEnum {
                ty,
                repr,
                niche_bits,
                niche_offset,
                niche_variants,
                payload_key,
                payload,
                ..
            } => {
                let bound = self.bind_if_needed("pn", val, stmts);
                let payload_in = if repr.is_null_or_sentinel_niche() {
                    bound.clone()
                } else {
                    value::field_expr(self.cx.interns, bound.clone(), *payload_key)
                };
                let mut else_stmts = Vec::new();
                self.emit_pack(&mut else_stmts, off, payload, payload_in);
                let mut cur = else_stmts;
                for &(vi, raw_bits) in niche_variants.iter().rev() {
                    let Some(tag_lit) = value::variant_tag(self.cx.table, self.cx.interns, *ty, vi)
                    else {
                        continue;
                    };
                    let mut then_stmts = Vec::new();
                    let raw_expr = if *niche_bits > 32 {
                        jsast::bigint(raw_bits)
                    } else {
                        jsast::num(raw_bits as f64)
                    };
                    self.emit_pack_int(
                        &mut then_stmts,
                        &off.add_const(*niche_offset),
                        false,
                        *niche_bits,
                        raw_expr,
                    );
                    let test = jsast::binary(
                        BinOp::StrictEq,
                        value::tag_expr(*repr, bound.clone()),
                        tag_lit,
                    );
                    cur = vec![jsast::if_else(test, then_stmts, cur)];
                }
                stmts.extend(cur);
            }
        }
    }

    fn emit_pack_int(
        &mut self,
        stmts: &mut Vec<Stmt>,
        off: &OffExpr,
        signed: bool,
        bits: u32,
        val: Expr,
    ) {
        let dv = jsast::id(POD_DV);
        match bits {
            8 => {
                let m = if signed { "setInt8" } else { "setUint8" };
                stmts.push(jsast::expr_stmt(jsast::method_call(
                    dv,
                    m,
                    vec![off.to_expr(), val],
                )));
            }
            16 => {
                let m = if signed { "setInt16" } else { "setUint16" };
                stmts.push(jsast::expr_stmt(jsast::method_call(
                    dv,
                    m,
                    vec![off.to_expr(), val, Expr::Bool(true)],
                )));
            }
            32 => {
                let m = if signed { "setInt32" } else { "setUint32" };
                stmts.push(jsast::expr_stmt(jsast::method_call(
                    dv,
                    m,
                    vec![off.to_expr(), val, Expr::Bool(true)],
                )));
            }
            64 | 128 => {
                let bound = self.bind_if_needed("pb", val, stmts);
                let words = bits / 32;
                for w in 0..words {
                    let shifted = if w == 0 {
                        bound.clone()
                    } else {
                        jsast::binary(BinOp::Shr, bound.clone(), jsast::bigint(w * 32))
                    };
                    let u32_word = jsast::to_number(jsast::bigint_mask(false, 32, shifted));
                    stmts.push(jsast::expr_stmt(jsast::method_call(
                        dv.clone(),
                        "setUint32",
                        vec![
                            off.add_const(u64::from(w) * 4).to_expr(),
                            u32_word,
                            Expr::Bool(true),
                        ],
                    )));
                }
            }
            _ => unreachable!("validated integer bit width"),
        }
    }

    /// Emits statements + result expression unpacking a value of layout `node` from byte offset `off`.
    fn emit_unpack(&mut self, stmts: &mut Vec<Stmt>, off: &OffExpr, node: &PodNode) -> Expr {
        let dv = jsast::id(POD_DV);
        match node {
            PodNode::Zst => jsast::undefined(),
            PodNode::Scalar(scalar) => match *scalar {
                PodScalar::Int { signed, bits, .. }
                | PodScalar::NumberEnum { signed, bits, .. } => {
                    self.emit_unpack_int(off, signed, bits)
                }
                PodScalar::Float { bits } => {
                    let method = if bits == 32 {
                        "getFloat32"
                    } else {
                        "getFloat64"
                    };
                    jsast::method_call(dv, method, vec![off.to_expr(), Expr::Bool(true)])
                }
                PodScalar::Bool => jsast::binary(
                    BinOp::StrictNe,
                    jsast::method_call(dv, "getUint8", vec![off.to_expr()]),
                    jsast::num(0),
                ),
                PodScalar::Char | PodScalar::ThinPtr { .. } => {
                    jsast::method_call(dv, "getUint32", vec![off.to_expr(), Expr::Bool(true)])
                }
            },
            PodNode::Array {
                elem,
                elem_size,
                length,
            } => {
                if *length == 0 {
                    return jsast::array(Vec::new());
                }
                let arr_name = self.fresh("a");
                let idx_name = self.fresh("j");
                let arr = jsast::id(arr_name.clone());
                let idx = jsast::id(idx_name.clone());
                stmts.push(jsast::const_(
                    arr_name,
                    jsast::new(jsast::id("Array"), vec![jsast::num(*length as f64)]),
                ));
                stmts.push(jsast::let_(idx_name, jsast::num(0)));
                let mut body = Vec::new();
                let elem_off = off.add_scaled(idx.clone(), *elem_size);
                let val = self.emit_unpack(&mut body, &elem_off, elem);
                body.push(jsast::assign_stmt(
                    jsast::index(arr.clone(), idx.clone()),
                    val,
                ));
                body.push(jsast::expr_stmt(jsast::compound_assign(
                    CompoundOp::Add,
                    idx.clone(),
                    jsast::num(1),
                )));
                stmts.push(jsast::while_(
                    jsast::binary(BinOp::Lt, idx, jsast::num(*length as f64)),
                    body,
                ));
                arr
            }
            PodNode::Tuple { arity, fields } => {
                let mut elems = vec![jsast::undefined(); *arity];
                for field in fields {
                    let PodFieldAccess::Index(idx) = field.access else {
                        continue;
                    };
                    let val = self.emit_unpack(stmts, &off.add_const(field.offset), &field.node);
                    if (idx as usize) < elems.len() {
                        elems[idx as usize] = val;
                    }
                }
                jsast::array(elems)
            }
            PodNode::Struct { zst_keys, fields } => {
                let mut entries = Vec::with_capacity(fields.len() + zst_keys.len());
                let mut proto_val = None;
                for field in fields {
                    let PodFieldAccess::Name(ref name) = field.access else {
                        continue;
                    };
                    let val = self.emit_unpack(stmts, &off.add_const(field.offset), &field.node);
                    if name == "__proto__" {
                        proto_val = Some(val);
                    } else {
                        entries.push((name.clone(), val));
                    }
                }
                for key in zst_keys {
                    if key == "__proto__" {
                        proto_val = Some(jsast::undefined());
                    } else {
                        entries.push((key.clone(), jsast::undefined()));
                    }
                }
                let obj = jsast::object(entries);
                if let Some(pv) = proto_val {
                    let obj_name = self.fresh("o");
                    stmts.push(jsast::const_(obj_name.clone(), obj));
                    stmts.push(jsast::expr_stmt(value::define_proto_prop(
                        jsast::id(obj_name.clone()),
                        pv,
                    )));
                    jsast::id(obj_name)
                } else {
                    obj
                }
            }
            PodNode::FieldlessEnum {
                ty,
                tag_bytes,
                variants,
            } => {
                let raw = self.emit_unpack_int(off, false, u32::from(*tag_bytes) * 8);
                let bound = self.bind_if_needed("te", raw, stmts);
                let last_vi = variants.last().map_or(0, |&(vi, _)| vi);
                let mut expr = value::variant_tag(self.cx.table, self.cx.interns, *ty, last_vi)
                    .unwrap_or_else(jsast::undefined);
                for &(vi, raw_bits) in variants[..variants.len() - 1].iter().rev() {
                    if let Some(tag_lit) =
                        value::variant_tag(self.cx.table, self.cx.interns, *ty, vi)
                    {
                        expr = jsast::cond(
                            jsast::binary(
                                BinOp::StrictEq,
                                bound.clone(),
                                jsast::num(raw_bits as f64),
                            ),
                            tag_lit,
                            expr,
                        );
                    }
                }
                expr
            }
            PodNode::NicheEnum {
                ty,
                niche_bits,
                niche_offset,
                niche_variants,
                untagged_variant,
                payload_key,
                payload,
                ..
            } => {
                let tag_raw =
                    self.emit_unpack_int(&off.add_const(*niche_offset), false, *niche_bits);
                let tag_var = self.bind_if_needed("nt", tag_raw, stmts);
                let res_name = self.fresh("nr");
                stmts.push(jsast::let_uninit(res_name.clone()));

                let mut else_stmts = Vec::new();
                let payload_val = self.emit_unpack(&mut else_stmts, off, payload);
                let key = value::key_name(self.cx.interns, *payload_key).unwrap_or_default();
                let filled = value::enum_value(
                    self.cx.table,
                    self.cx.interns,
                    *ty,
                    *untagged_variant,
                    vec![(key, payload_val)],
                )
                .unwrap_or_else(jsast::undefined);
                else_stmts.push(jsast::assign_stmt(jsast::id(res_name.clone()), filled));

                let mut cur = else_stmts;
                for &(vi, raw_bits) in niche_variants.iter().rev() {
                    let Some(empty) =
                        value::enum_value(self.cx.table, self.cx.interns, *ty, vi, Vec::new())
                    else {
                        continue;
                    };
                    let raw_lit = if *niche_bits > 32 {
                        jsast::bigint(raw_bits)
                    } else {
                        jsast::num(raw_bits as f64)
                    };
                    let test = jsast::binary(BinOp::StrictEq, tag_var.clone(), raw_lit);
                    cur = vec![jsast::if_else(
                        test,
                        vec![jsast::assign_stmt(jsast::id(res_name.clone()), empty)],
                        cur,
                    )];
                }
                stmts.extend(cur);
                jsast::id(res_name)
            }
        }
    }

    fn emit_unpack_int(&self, off: &OffExpr, signed: bool, bits: u32) -> Expr {
        let dv = jsast::id(POD_DV);
        match bits {
            8 => jsast::method_call(
                dv,
                if signed { "getInt8" } else { "getUint8" },
                vec![off.to_expr()],
            ),
            16 => jsast::method_call(
                dv,
                if signed { "getInt16" } else { "getUint16" },
                vec![off.to_expr(), Expr::Bool(true)],
            ),
            32 => jsast::method_call(
                dv,
                if signed { "getInt32" } else { "getUint32" },
                vec![off.to_expr(), Expr::Bool(true)],
            ),
            64 | 128 => {
                let words = bits / 32;
                let mut combined = jsast::to_bigint(jsast::method_call(
                    dv.clone(),
                    "getUint32",
                    vec![off.to_expr(), Expr::Bool(true)],
                ));
                for w in 1..words {
                    let method = if signed && w + 1 == words {
                        "getInt32"
                    } else {
                        "getUint32"
                    };
                    let word = jsast::to_bigint(jsast::method_call(
                        dv.clone(),
                        method,
                        vec![off.add_const(u64::from(w) * 4).to_expr(), Expr::Bool(true)],
                    ));
                    let shifted = jsast::binary(BinOp::Shl, word, jsast::bigint(w * 32));
                    combined = jsast::binary(BinOp::BitOr, combined, shifted);
                }
                combined
            }
            _ => unreachable!("validated integer bit width"),
        }
    }
}

/// Builds a standalone arrow function `($p) => { ... }` that converts a value of `from_ty` to
/// `to_ty` through a little-endian `DataView`, returning `None` if either layout is non-POD or
/// their byte sizes differ.
pub(crate) fn pod_transmute_arrow(
    cx: &JsCx<'_>,
    from_ty: TypeId,
    to_ty: TypeId,
    allow_thin_ptr: bool,
) -> Option<Expr> {
    let table = cx.table;
    let size = value::size_of(table, from_ty)?;
    if size == 0 || value::size_of(table, to_ty) != Some(size) {
        return None;
    }
    let from_node = classify_pod(
        table,
        cx.interns,
        from_ty,
        PodPolicy {
            allow_thin_ptr,
            for_unpack: false,
        },
    )?;
    let to_node = classify_pod(
        table,
        cx.interns,
        to_ty,
        PodPolicy {
            allow_thin_ptr,
            for_unpack: true,
        },
    )?;
    let mut gen = CodecGen::new(cx);
    let mut body = Vec::new();
    body.push(jsast::const_(
        POD_DV,
        jsast::new(
            jsast::id("DataView"),
            vec![jsast::new(
                jsast::id("ArrayBuffer"),
                vec![jsast::num(size as f64)],
            )],
        ),
    ));
    gen.emit_pack(&mut body, &OffExpr::Const(0), &from_node, jsast::id(POD_IN));
    let out = gen.emit_unpack(&mut body, &OffExpr::Const(0), &to_node);
    body.push(jsast::ret(out));
    Some(jsast::arrow_block(vec![POD_IN.to_owned()], body))
}

/// Emits an immediately invoked little-endian `DataView` pack/unpack expression transmuting
/// `value` from `from_ty` to `to_ty`, or `None` when not POD-compatible.
pub(crate) fn pod_transmute(
    cx: &JsCx<'_>,
    from_ty: TypeId,
    to_ty: TypeId,
    value: Expr,
) -> Option<Expr> {
    let arrow = pod_transmute_arrow(cx, from_ty, to_ty, true)?;
    Some(jsast::call(arrow, vec![value]))
}

/// Lowers `raw_eq(lhs_val, rhs_val)` recursively for an already-classified [`PodNode`].
///
/// `lhs_val` and `rhs_val` are already the JS **values** (unwrapped from `{ $b, $o }` when
/// `pointee` is direct, or the direct JS array/object when `pointee` is indirect).
pub(crate) fn emit_pod_raw_eq(cx: &JsCx<'_>, node: &PodNode, lhs_val: Expr, rhs_val: Expr) -> Expr {
    emit_raw_eq_node(cx, node, lhs_val, rhs_val, 0)
}

fn emit_raw_eq_node(cx: &JsCx<'_>, node: &PodNode, lhs: Expr, rhs: Expr, depth: usize) -> Expr {
    match node {
        PodNode::Zst => jsast::boolean(true),
        PodNode::Scalar(
            PodScalar::Int { .. }
            | PodScalar::Bool
            | PodScalar::Char
            | PodScalar::NumberEnum { .. },
        )
        | PodNode::FieldlessEnum { .. } => jsast::binary(BinOp::StrictEq, lhs, rhs),
        PodNode::Scalar(PodScalar::Float { bits }) => {
            if *bits == 32 {
                let to_bits = |v: Expr| {
                    let src = jsast::method_call(jsast::id("Float32Array"), "of", vec![v]);
                    let view =
                        jsast::new(jsast::id("Uint32Array"), vec![jsast::member(src, "buffer")]);
                    jsast::index(view, jsast::num(0))
                };
                jsast::binary(BinOp::StrictEq, to_bits(lhs), to_bits(rhs))
            } else {
                // Compare both 32-bit halves via ES2018-safe Uint32Array (no BigUint64Array needed).
                let l = format!("$fl{depth}");
                let r = format!("$fr{depth}");
                let mk_u32 = |v: Expr| {
                    jsast::new(
                        jsast::id("Uint32Array"),
                        vec![jsast::member(
                            jsast::method_call(jsast::id("Float64Array"), "of", vec![v]),
                            "buffer",
                        )],
                    )
                };
                let eq = jsast::binary(
                    BinOp::And,
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::index(jsast::id(l.clone()), jsast::num(0)),
                        jsast::index(jsast::id(r.clone()), jsast::num(0)),
                    ),
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::index(jsast::id(l.clone()), jsast::num(1)),
                        jsast::index(jsast::id(r.clone()), jsast::num(1)),
                    ),
                );
                jsast::call(jsast::arrow(vec![l, r], eq), vec![mk_u32(lhs), mk_u32(rhs)])
            }
        }
        PodNode::Scalar(PodScalar::ThinPtr { ty, .. }) => cx.ptr_eq(*ty, lhs, rhs),
        PodNode::Array { elem, length, .. } => {
            if *length == 0 {
                return jsast::boolean(true);
            }
            let e_param = if depth == 0 {
                "$e".to_owned()
            } else {
                format!("$e{depth}")
            };
            let i_param = if depth == 0 {
                "$i".to_owned()
            } else {
                format!("$i{depth}")
            };
            let make_every = |lhs_arr: Expr, rhs_arr: Expr| {
                let elem_eq = emit_raw_eq_node(
                    cx,
                    elem,
                    jsast::id(e_param.clone()),
                    jsast::index(rhs_arr, jsast::id(i_param.clone())),
                    depth + 1,
                );
                jsast::method_call(
                    lhs_arr,
                    "every",
                    vec![jsast::arrow(vec![e_param, i_param], elem_eq)],
                )
            };
            if is_cheap_ref(&rhs) {
                make_every(lhs, rhs)
            } else {
                let r_name = format!("$r{depth}");
                jsast::call(
                    jsast::arrow(vec![r_name.clone()], make_every(lhs, jsast::id(r_name))),
                    vec![rhs],
                )
            }
        }
        PodNode::Tuple { fields, .. } | PodNode::Struct { fields, .. } => {
            if fields.is_empty() {
                return jsast::boolean(true);
            }
            let build = |l_expr: Expr, r_expr: Expr| {
                let mut iter = fields.iter().map(|field| {
                    let (lf, rf) = match &field.access {
                        PodFieldAccess::Name(name) => (
                            jsast::member(l_expr.clone(), name.clone()),
                            jsast::member(r_expr.clone(), name.clone()),
                        ),
                        PodFieldAccess::Index(idx) => (
                            jsast::index(l_expr.clone(), jsast::num(f64::from(*idx))),
                            jsast::index(r_expr.clone(), jsast::num(f64::from(*idx))),
                        ),
                    };
                    emit_raw_eq_node(cx, &field.node, lf, rf, depth + 1)
                });
                let first = iter.next().expect("non-empty fields");
                iter.fold(first, |acc, next| jsast::binary(BinOp::And, acc, next))
            };
            if fields.len() == 1 || (is_cheap_ref(&lhs) && is_cheap_ref(&rhs)) {
                build(lhs, rhs)
            } else {
                let l_name = format!("$l{depth}");
                let r_name = format!("$r{depth}");
                jsast::call(
                    jsast::arrow(
                        vec![l_name.clone(), r_name.clone()],
                        build(jsast::id(l_name), jsast::id(r_name)),
                    ),
                    vec![lhs, rhs],
                )
            }
        }
        PodNode::NicheEnum {
            ty,
            repr,
            niche_variants,
            payload_key,
            payload,
            ..
        } => {
            if repr.is_null_or_sentinel_niche() && payload.compares_strict_eq() {
                return jsast::binary(BinOp::StrictEq, lhs, rhs);
            }
            let l_name = format!("$nl{depth}");
            let r_name = format!("$nr{depth}");
            let l_id = jsast::id(l_name.clone());
            let r_id = jsast::id(r_name.clone());
            let body = if repr.is_null_or_sentinel_niche() {
                let none_lit = value::niche_none_expr(cx.table, *repr).unwrap_or_else(jsast::null);
                let l_none = jsast::binary(BinOp::StrictEq, l_id.clone(), none_lit.clone());
                let r_none = jsast::binary(BinOp::StrictEq, r_id.clone(), none_lit);
                let payload_eq = emit_raw_eq_node(cx, payload, l_id, r_id, depth + 1);
                jsast::cond(
                    l_none,
                    r_none.clone(),
                    jsast::binary(BinOp::And, jsast::not(r_none), payload_eq),
                )
            } else {
                let l_tag = value::tag_expr(*repr, l_id.clone());
                let r_tag = value::tag_expr(*repr, r_id.clone());
                let tags_eq = jsast::binary(BinOp::StrictEq, l_tag.clone(), r_tag);
                let mut is_niche: Option<Expr> = None;
                for &(vi, _) in niche_variants {
                    if let Some(tag_lit) = value::variant_tag(cx.table, cx.interns, *ty, vi) {
                        let cmp = jsast::binary(BinOp::StrictEq, l_tag.clone(), tag_lit);
                        is_niche = Some(match is_niche {
                            Some(prev) => jsast::binary(BinOp::Or, prev, cmp),
                            None => cmp,
                        });
                    }
                }
                let l_pay = value::field_expr(cx.interns, l_id, *payload_key);
                let r_pay = value::field_expr(cx.interns, r_id, *payload_key);
                let pay_eq = emit_raw_eq_node(cx, payload, l_pay, r_pay, depth + 1);
                let rest = match is_niche {
                    Some(niche_test) => jsast::binary(BinOp::Or, niche_test, pay_eq),
                    None => pay_eq,
                };
                jsast::binary(BinOp::And, tags_eq, rest)
            };
            jsast::call(jsast::arrow(vec![l_name, r_name], body), vec![lhs, rhs])
        }
    }
}

/// Whether `expr` is cheap and side-effect-free to reference multiple times inside a predicate.
fn is_cheap_ref(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) => true,
        Expr::Member(obj, _) => is_cheap_ref(obj),
        Expr::Index(obj, idx) => {
            is_cheap_ref(obj)
                && matches!(idx.as_ref(), Expr::Ident(_) | Expr::Num(_) | Expr::Str(_))
        }
        _ => false,
    }
}

/// Returns `true` if `ty` is a valid scalar type for `niche_from_scalar` / `niche_to_scalar`,
/// returning `(bits, literal_fn_result)` for a given raw bit pattern.
pub(crate) fn niche_scalar_bits(table: &TypeTable, ty: TypeId) -> Option<u32> {
    let peeled = table.peeled(ty);
    if let Some((_, bits)) = table.int_info(peeled) {
        return Some(bits);
    }
    if let Some(EnumRepr::Number { bits, .. }) = table.enum_repr(peeled) {
        return Some(bits);
    }
    match value::kind(table, peeled)? {
        TypeKind::Char => Some(POINTER_BITS),
        TypeKind::Bool => Some(8),
        _ => None,
    }
}

/// Formats `raw_bits` as a JS literal for any scalar accepted by [`niche_scalar_bits`].
pub(crate) fn niche_scalar_literal(table: &TypeTable, ty: TypeId, raw_bits: u128) -> Option<Expr> {
    let peeled = table.peeled(ty);
    if let Some(lit) = value::int_literal(table, peeled, raw_bits) {
        return Some(lit);
    }
    if let Some(EnumRepr::Number { bits, signed }) = table.enum_repr(peeled) {
        let repr = if bits > 32 {
            IntRepr::BigInt
        } else {
            IntRepr::Number
        };
        return Some(value::signed_literal(
            repr,
            value::resign(signed, bits, raw_bits),
        ));
    }
    match value::kind(table, peeled)? {
        TypeKind::Char => Some(jsast::num((raw_bits as u32) as f64)),
        TypeKind::Bool => Some(jsast::boolean(raw_bits != 0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::semantic::DefinitionId;
    use rcgjs_object::jir::types::{
        EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, Niche,
        NominalForm, PointerForm, TypeDescriptor, TypeId, TypeKind, TypeMeta, VariantDescriptor,
    };
    use rcgjs_object::jsast::{self, Style};
    use rcgjs_object::opts::EcmaTarget;

    use super::*;

    fn mk_base(id: u128, kind: TypeKind, size: u64) -> TypeDescriptor {
        let tid = TypeId(id);
        TypeDescriptor {
            id: tid,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 8)),
                sized: true,
                zst: size == 0,
                one_zst: size == 0,
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
                peeled: tid,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn mk_int(id: u128, signed: bool, bits: u32) -> TypeDescriptor {
        let mut d = mk_base(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        d.js.int = Some(IntFacts {
            signed,
            bits,
            repr: if bits > 32 {
                IntRepr::BigInt
            } else {
                IntRepr::Number
            },
        });
        d.js.byte_element = bits == 8;
        d
    }

    #[test]
    fn classifies_ipv6_arrays_nested_arrays_structs_and_niche_enums() {
        let mut interns = Interns::new();
        let u8_ty = TypeId(1);
        let u16_ty = TypeId(2);
        let u32_ty = TypeId(3);
        let u64_ty = TypeId(4);
        let arr_u8_16 = TypeId(5);
        let arr_u16_8 = TypeId(6);
        let arr_u32_2 = TypeId(7);
        let arr_nested = TypeId(8);
        let pair_ty = TypeId(9);
        let char_ty = TypeId(10);
        let opt_nz_char = TypeId(11);
        let ptr_u8 = TypeId(12);
        let arr_u8_4 = TypeId(13);
        let ident_ty = TypeId(14);

        interns.intern_type(mk_int(1, false, 8));
        interns.intern_type(mk_int(2, false, 16));
        interns.intern_type(mk_int(3, false, 32));
        interns.intern_type(mk_int(4, false, 64));
        interns.intern_type(mk_base(
            5,
            TypeKind::Array {
                element: u8_ty,
                length: 16,
            },
            16,
        ));
        interns.intern_type(mk_base(
            6,
            TypeKind::Array {
                element: u16_ty,
                length: 8,
            },
            16,
        ));
        interns.intern_type(mk_base(
            7,
            TypeKind::Array {
                element: u32_ty,
                length: 2,
            },
            8,
        ));
        interns.intern_type(mk_base(
            8,
            TypeKind::Array {
                element: arr_u32_2,
                length: 3,
            },
            24,
        ));

        let k_a = interns.intern_str("a");
        let k_b = interns.intern_str("b");
        let s_pair = interns.intern_str("Pair");
        interns.intern_type(mk_base(
            9,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_pair,
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Name(k_a),
                            ty: u16_ty,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Name(k_b),
                            ty: u16_ty,
                            offset: 2,
                        },
                    ],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: vec![],
                }],
                niche: None,
            },
            4,
        ));

        interns.intern_type(mk_base(10, TypeKind::Char, 4));
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let k_0 = interns.intern_str("_0");
        let mut opt = mk_base(
            11,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 2,
                },
                args: vec![char_ty],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_none,
                        fields: vec![],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    },
                    VariantDescriptor {
                        name: s_some,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(k_0),
                            ty: char_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    },
                ],
                niche: Some(Niche {
                    untagged_variant: 1,
                    niche_field: vec![0],
                    niche_start: 0,
                    valid_range: (1, 0x10_ffff),
                }),
            },
            4,
        );
        opt.js.enum_repr = Some(EnumRepr::NullNiche {
            none_variant: 0,
            some_variant: 1,
        });
        interns.intern_type(opt);

        let mut p = mk_base(
            12,
            TypeKind::RawPointer {
                pointee: u8_ty,
                mutable: false,
            },
            4,
        );
        p.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(p);
        interns.intern_type(mk_base(
            13,
            TypeKind::Array {
                element: u8_ty,
                length: 4,
            },
            4,
        ));
        let k_head = interns.intern_str("head");
        let k_tail = interns.intern_str("tail");
        let s_ident = interns.intern_str("Identifier");
        interns.intern_type(mk_base(
            14,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 9,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_ident,
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Name(k_head),
                            ty: ptr_u8,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Name(k_tail),
                            ty: arr_u8_4,
                            offset: 4,
                        },
                    ],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: vec![],
                }],
                niche: None,
            },
            8,
        ));

        let strict = PodPolicy {
            allow_thin_ptr: false,
            for_unpack: true,
        };
        assert!(classify_pod(&interns.types, &interns, arr_u8_16, strict).is_some());
        assert!(classify_pod(&interns.types, &interns, arr_u16_8, strict).is_some());
        assert!(classify_pod(&interns.types, &interns, arr_nested, strict).is_some());
        assert!(classify_pod(&interns.types, &interns, pair_ty, strict).is_some());
        assert!(classify_pod(&interns.types, &interns, opt_nz_char, strict).is_some());
        assert!(classify_pod(&interns.types, &interns, ident_ty, strict).is_none());
        assert!(classify_pod(
            &interns.types,
            &interns,
            ident_ty,
            PodPolicy {
                allow_thin_ptr: true,
                for_unpack: true,
            }
        )
        .is_some());
        assert_eq!(niche_scalar_bits(&interns.types, char_ty), Some(32));
        let lit0 = niche_scalar_literal(&interns.types, char_ty, 0).expect("char 0 literal");
        assert_eq!(
            jsast::expr_to_string_styled_for_target(&lit0, Style::Compact, EcmaTarget::Es2018),
            "0"
        );
        let _ = u64_ty;
    }
}
