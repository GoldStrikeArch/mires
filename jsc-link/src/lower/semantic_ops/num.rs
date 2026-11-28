//! Host-delegated numeric parsing body replacements (`<f64 as FromStr>::from_str` and
//! `<f32 as FromStr>::from_str`).
//!
//! Recognizing `<f64 as core::str::FromStr>::from_str` and `<f32 as core::str::FromStr>::from_str`
//! by exact rustc identity (`StandardOperation::F64FromStr` / `StandardOperation::F32FromStr`)
//! lets `jsc-link` replace their `#[inline(never)]` bodies with compact calls to
//! `__rt.parse_f64` / `__rt.parse_f32`, constructing the exact `Result<T, ParseFloatError>`
//! enum representation dynamically from the [`TypeTable`] so all of `core::num::imp::dec2flt`
//! becomes unreachable and is dropped by reachability.

use std::collections::BTreeSet;

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jir::types::{FieldKey, NominalForm, TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast::{self, BinOp, Expr};

use crate::lower::js::value;
use crate::lower::{LowerInput, LoweredItem};

/// Whether `operation` is a host-delegated float `FromStr` body replacement.
pub(crate) fn is_float_from_str(operation: StandardOperation) -> bool {
    matches!(
        operation,
        StandardOperation::F64FromStr | StandardOperation::F32FromStr
    )
}

/// Replaces `<f64 as FromStr>::from_str` or `<f32 as FromStr>::from_str` with a compact host
/// parser call returning the exact `Result<f64/f32, ParseFloatError>` layout described by
/// `input.table`. Returns `None` (fail closed to baseline lowering) if any ABI or type shape
/// differs from the canonical `core` signature.
pub(crate) fn lower_float_from_str(
    input: &LowerInput<'_>,
    is_f32: bool,
) -> Option<Result<LoweredItem, Decline>> {
    let sig = &input.body.signature;
    if sig.abi.spread.is_some()
        || sig.abi.caller_location
        || sig.abi.parameters != 1
        || sig.arguments.len() != 1
    {
        return None;
    }
    let table = input.table;
    let interns = input.interns;
    if !super::is_shared_str(table, sig.arguments[0]) {
        return None;
    }

    let result_ty = sig.result;
    let result_desc = table.get(table.peeled(result_ty))?;
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &result_desc.kind
    else {
        return None;
    };
    if variants.len() != 2 {
        return None;
    }

    let mut ok_variant = None;
    let mut err_variant = None;
    for (idx, variant) in variants.iter().enumerate() {
        if variant.result_ok {
            ok_variant = Some((idx as u32, variant));
        } else {
            err_variant = Some((idx as u32, variant));
        }
    }
    let (ok_idx, ok_var) = ok_variant?;
    let (err_idx, err_var) = err_variant?;
    if ok_var.fields.len() != 1 || err_var.fields.len() != 1 {
        return None;
    }

    let expected_bits = if is_f32 { 32 } else { 64 };
    if !matches!(
        table.get(table.peeled(ok_var.fields[0].ty)).map(|d| &d.kind),
        Some(TypeKind::Float { bits }) if *bits == expected_bits
    ) {
        return None;
    }

    let ok_key = entry_key(interns, ok_var.fields[0].key)?;
    let err_key = entry_key(interns, err_var.fields[0].key)?;

    let param_name = "$s";
    let parsed_name = "$r";
    let param_expr = jsast::id(param_name);
    let parsed_expr = jsast::id(parsed_name);

    let err_payload = build_parse_float_error(table, interns, err_var.fields[0].ty, &param_expr)?;

    let ok_val = value::enum_value(
        table,
        interns,
        result_ty,
        ok_idx,
        vec![(ok_key, parsed_expr.clone())],
    )?;
    let err_val = value::enum_value(
        table,
        interns,
        result_ty,
        err_idx,
        vec![(err_key, err_payload)],
    )?;

    let helper = if is_f32 { "parse_f32" } else { "parse_f64" };
    let stmts = vec![
        jsast::const_(parsed_name, jsast::rt_call(helper, vec![param_expr])),
        jsast::ret(jsast::cond(
            jsast::binary(BinOp::StrictEq, parsed_expr, jsast::undefined()),
            err_val,
            ok_val,
        )),
    ];

    let name = input.name.as_str();
    let decl = jsast::function(name, vec![param_name.to_owned()], stmts);
    let mut refs = BTreeSet::new();
    jsast::visit_free_idents(&decl, &mut |ident| {
        if ident != name {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });

    Some(Ok(LoweredItem {
        decl,
        refs,
        names: Vec::new(),
        loc: None,
        dts: None,
        zombies: Vec::new(),
        js_alloc_origins: Vec::new(),
        bigints: BTreeSet::new(),
        hoisted_consts: std::collections::BTreeMap::new(),
    }))
}

fn entry_key(interns: &Interns, key: FieldKey) -> Option<String> {
    match key {
        FieldKey::Name(id) => Some(interns.str(id)?.to_owned()),
        FieldKey::Index(index) => Some(format!("_{index}")),
        FieldKey::Transparent => None,
    }
}

/// Builds `ParseFloatError { kind: $s.length === 0 ? Empty : Invalid }` directly from the
/// `TypeTable` descriptors of `ParseFloatError` and `FloatErrorKind`.
fn build_parse_float_error(
    table: &TypeTable,
    interns: &Interns,
    err_ty: TypeId,
    src: &Expr,
) -> Option<Expr> {
    let err_desc = table.get(err_ty)?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &err_desc.kind
    else {
        return None;
    };
    let [struct_var] = variants.as_slice() else {
        return None;
    };
    let [kind_field] = struct_var.fields.as_slice() else {
        return None;
    };

    let kind_ty = kind_field.ty;
    let kind_desc = table.get(table.peeled(kind_ty))?;
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants: kind_variants,
        ..
    } = &kind_desc.kind
    else {
        return None;
    };
    let mut empty_idx = None;
    let mut invalid_idx = None;
    for (idx, variant) in kind_variants.iter().enumerate() {
        if !variant.fields.is_empty() {
            return None;
        }
        match interns.str(variant.name)? {
            "Empty" => empty_idx = Some(idx as u32),
            "Invalid" => invalid_idx = Some(idx as u32),
            _ => {}
        }
    }
    let empty_val = value::enum_value(table, interns, kind_ty, empty_idx?, Vec::new())?;
    let invalid_val = value::enum_value(table, interns, kind_ty, invalid_idx?, Vec::new())?;
    let kind_expr = jsast::cond(
        jsast::binary(
            BinOp::StrictEq,
            jsast::member(src.clone(), "length"),
            jsast::num(0),
        ),
        empty_val,
        invalid_val,
    );

    match kind_field.key {
        FieldKey::Transparent => Some(kind_expr),
        FieldKey::Name(id) => Some(jsast::object(vec![(
            interns.str(id)?.to_owned(),
            kind_expr,
        )])),
        FieldKey::Index(index) => Some(jsast::object(vec![(format!("_{index}"), kind_expr)])),
    }
}
