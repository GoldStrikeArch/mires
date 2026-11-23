//! Captured semantic operations, kept separate from the emitted dependency graph.

use std::collections::BTreeSet;

use crate::item::LinkJir;
use crate::jir::interns::Interns;
use crate::jir::origin::SourceFactId;
use crate::jir::semantic::{Callee, ItemId, Signature, StandardOperation, Terminator};
use crate::jir::types::TypeId;
use crate::lower::semantic_ops::{self, LowerCx};

use super::{json_string, write_strings};

pub(super) fn write_registry(out: &mut String) {
    out.push('[');
    // The `str` call-site identities exist only for `js-str=bytes`; every other mode forgets
    // them at decode and does not list them.
    for (index, operation) in (0..128u8)
        .map_while(|tag| postcard::from_bytes::<StandardOperation>(&[tag]).ok())
        .filter(|operation| !operation.is_str() || crate::lower::bstr::enabled())
        .enumerate()
    {
        if index != 0 {
            out.push(',');
        }
        out.push_str("{\"operation\":");
        json_string(out, &format!("{operation:?}"));
        out.push_str(&format!(
            " ,\"native_lowering\":{}}}",
            semantic_ops::registry(operation).is_some()
        ));
    }
    out.push(']');
}

fn identity(item: ItemId) -> String {
    format!(
        "{:016x}:{:016x}:{:032x}",
        item.definition.crate_id, item.definition.local_hash, item.instantiation
    )
}

fn type_id(ty: TypeId) -> String {
    format!("{:032x}", ty.0)
}

fn location(out: &mut String, interns: &Interns, source: Option<SourceFactId>) {
    let Some(source) = source.and_then(|id| interns.source_facts.get(id.0 as usize)) else {
        out.push_str("null");
        return;
    };
    out.push_str("{\"file\":");
    match interns.str(source.file) {
        Some(file) => json_string(out, file),
        None => out.push_str("null"),
    }
    out.push_str(&format!(
        ",\"line\":{},\"column\":{}}}",
        source.line, source.col
    ));
}

fn signature(out: &mut String, sig: &Signature) {
    out.push_str("{\"arguments\":");
    write_strings(
        out,
        &sig.arguments
            .iter()
            .copied()
            .map(type_id)
            .collect::<Vec<_>>(),
    );
    out.push_str(",\"result\":");
    json_string(out, &type_id(sig.result));
    out.push_str(",\"abi\":");
    json_string(out, &format!("{:?}", sig.abi));
    out.push('}');
}

pub(super) fn write_item(out: &mut String, jir: &LinkJir) {
    let (Some(body), Some(interns)) = (&jir.body, jir.interns()) else {
        out.push_str("null");
        return;
    };
    out.push_str("{\"item\":");
    json_string(out, &identity(body.item));
    out.push_str(",\"bodySemantic\":");
    json_string(out, &format!("{:?}", body.semantic));
    out.push_str(",\"signature\":");
    signature(out, &body.signature);
    out.push_str(",\"location\":");
    location(
        out,
        interns,
        jir.origins.as_ref().and_then(|origins| origins.root),
    );
    out.push_str(",\"nativeSignature\":");
    let cx = LowerCx {
        semantic: body.semantic,
        sig: &body.signature,
        table: &interns.types,
        interns,
        element_glue: None,
        element_glue_nonthrowing: false,
        panic_impl: None,
    };
    match semantic_ops::validate(&cx) {
        Ok(op) => {
            out.push_str("{\"operation\":");
            json_string(out, &format!("{:?}", op.id));
            out.push_str(",\"decline\":null,\"carriers\":");
            write_strings(
                out,
                &op.carriers
                    .iter()
                    .map(|carrier| format!("{carrier:?}"))
                    .collect::<Vec<_>>(),
            );
            out.push_str(",\"possibleRuntimeHelpers\":");
            write_strings(
                out,
                &op.helpers
                    .iter()
                    .map(|helper| (*helper).to_owned())
                    .collect::<Vec<_>>(),
            );
            out.push_str(",\"requiredGlue\":");
            json_string(out, &format!("{:?}", op.deps));
            out.push_str(&format!(
                ",\"canThrow\":{},\"mutates\":{}}}",
                op.effects.can_throw, op.effects.mutates
            ));
        }
        Err(decline) => {
            out.push_str("{\"decline\":");
            json_string(out, &format!("{decline:?}"));
            out.push_str(",\"reason\":");
            json_string(out, decline.reason());
            out.push('}');
        }
    }
    let mut types: BTreeSet<_> = body.locals.iter().map(|local| local.ty).collect();
    types.extend(body.signature.arguments.iter().copied());
    types.insert(body.signature.result);
    out.push_str(",\"calls\":[");
    let mut first = true;
    for (index, block) in body.blocks.iter().enumerate() {
        let Terminator::Call(call) = &block.terminator else {
            continue;
        };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!("{{\"block\":{},\"callee\":", block.id.0));
        match &call.callee {
            Callee::Direct { item, link_name } => {
                out.push_str("{\"kind\":\"direct\",\"item\":");
                json_string(out, &identity(*item));
                out.push_str(",\"symbol\":");
                match interns.str(*link_name) {
                    Some(name) => json_string(out, name),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
            other => {
                out.push_str("{\"kind\":\"other\",\"descriptor\":");
                json_string(out, &format!("{other:?}"));
                out.push('}');
            }
        }
        out.push_str(",\"semantic\":");
        json_string(out, &format!("{:?}", call.semantic));
        out.push_str(",\"hostPayload\":");
        match &call.host {
            Some(host) => json_string(out, &format!("{host:?}")),
            None => out.push_str("null"),
        }
        out.push_str(",\"arguments\":");
        write_strings(
            out,
            &call
                .arguments
                .iter()
                .map(|arg| type_id(arg.ty))
                .collect::<Vec<_>>(),
        );
        out.push_str(",\"result\":");
        json_string(out, &type_id(call.destination.ty));
        out.push_str(",\"location\":");
        location(
            out,
            interns,
            jir.origins
                .as_ref()
                .and_then(|origins| origins.terminator(index))
                .and_then(|origin| origin.source),
        );
        out.push_str(",\"unwind\":");
        json_string(out, &format!("{:?}", call.unwind));
        out.push('}');
        types.extend(call.arguments.iter().map(|arg| arg.ty));
        types.insert(call.destination.ty);
    }
    out.push_str("],\"types\":[");
    for (index, ty) in types.into_iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        json_string(out, &type_id(ty));
        out.push_str(",\"name\":");
        let descriptor = interns.types.get(ty);
        match descriptor
            .and_then(|ty| ty.name)
            .and_then(|name| interns.str(name))
        {
            Some(name) => json_string(out, name),
            None => out.push_str("null"),
        }
        for (key, value) in [
            ("kind", descriptor.map(|ty| format!("{:?}", ty.kind))),
            ("layout", descriptor.map(|ty| format!("{:?}", ty.layout))),
            (
                "observations",
                descriptor.map(|ty| format!("{:?}", ty.meta)),
            ),
        ] {
            out.push(',');
            json_string(out, key);
            out.push(':');
            match value {
                Some(value) => json_string(out, &value),
                None => out.push_str("null"),
            }
        }
        out.push('}');
    }
    out.push_str("]}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::semantic::{AbiSignature, Body, BodyKind, BodySemantic, DefinitionId};

    #[test]
    fn known_operation_identity_does_not_certify_a_signature_or_native_lowering() {
        let mut registry = String::new();
        write_registry(&mut registry);
        let registry: serde_json::Value = serde_json::from_str(&registry).unwrap();
        let rows = registry.as_array().unwrap();
        assert_eq!(rows.len(), 66);
        assert_eq!(
            rows.iter()
                .filter(|row| row["native_lowering"] == true)
                .count(),
            59
        );
        for (semantic, decline) in [
            (BodySemantic::Ordinary, "NotAStandardOperation"),
            (
                BodySemantic::Standard(StandardOperation::VecSwap),
                "UnsupportedOperation(VecSwap)",
            ),
            (
                BodySemantic::Standard(StandardOperation::StringNew),
                "ParameterCount { expected: 0, actual: 1 }",
            ),
        ] {
            let mut jir = LinkJir::default();
            jir.use_interns(Interns::default());
            jir.body = Some(Body {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 2,
                    },
                    instantiation: 3,
                },
                kind: BodyKind::Function,
                semantic,
                signature: Signature {
                    abi: AbiSignature {
                        parameters: 1,
                        spread: None,
                        caller_location: false,
                        inline_never: false,
                    },
                    arguments: vec![TypeId(1)],
                    result: TypeId(1),
                },
                locals: vec![],
                blocks: vec![],
            });
            let mut out = String::new();
            write_item(&mut out, &jir);
            let report: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(report["nativeSignature"]["decline"], decline);
            assert_eq!(report["signature"]["arguments"][0], type_id(TypeId(1)));
            assert!(report["types"][0]["layout"].is_null());
            assert!(report["location"].is_null());
        }
    }
}
