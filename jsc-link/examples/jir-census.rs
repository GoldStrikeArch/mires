//! Emits typed per-object census records without loading rustc.

use std::collections::BTreeSet;
use std::error::Error;

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::semantic::{Callee, ItemId, StandardOperation, Terminator, TypeId};
use rcgjs_object::object::{self as wire, Object};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn Error>> {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: jir-census <file.rlib|object>...".into());
    }
    // StandardOperation is an append-only enum of unit variants. Decode its actual registry,
    // rather than conflating the number of registered variants with instantiated operations.
    let registered: Vec<_> = (0..128u8)
        .map_while(|tag| postcard::from_bytes::<StandardOperation>(&[tag]).ok())
        .map(|operation| format!("{operation:?}"))
        .collect();
    if registered.is_empty() {
        return Err("empty standard-operation registry".into());
    }
    println!(
        "{}",
        json!({"record": "registry", "registeredOperations": registered,
        "schema": rcgjs_object::jir::JIR_SCHEMA, "objectFormat": wire::FORMAT_VERSION})
    );
    for path in paths {
        let bytes = std::fs::read(&path)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let mut count = 0;
        if bytes.starts_with(b"!<arch>\n") {
            let archive = object::read::archive::ArchiveFile::parse(&*bytes)?;
            for member in archive.members() {
                let member = member?;
                let data = member.data(&*bytes)?;
                if let Some(object) = wire::read(data)? {
                    emit(
                        &path,
                        &String::from_utf8_lossy(member.name()),
                        &digest,
                        &object,
                    )?;
                    count += 1;
                }
            }
        } else if let Some(object) = wire::read(&bytes)? {
            emit(&path, "", &digest, &object)?;
            count += 1;
        }
        if count == 0 {
            return Err(format!("{path}: no JIR object members").into());
        }
    }
    Ok(())
}

fn identity(item: ItemId) -> String {
    format!(
        "{:016x}:{:016x}:{:032x}",
        item.definition.crate_id, item.definition.local_hash, item.instantiation
    )
}

fn location(interns: &Interns, source: Option<SourceFactId>) -> Value {
    source.and_then(|id| interns.source_facts.get(id.0 as usize)).map_or(Value::Null, |source| {
        json!({"file": interns.str(source.file), "line": source.line, "column": source.col})
    })
}

fn type_info(interns: &Interns, id: TypeId) -> Result<Value, Box<dyn Error>> {
    let descriptor = interns
        .types
        .get(id)
        .ok_or("unresolved canonical type identity")?;
    Ok(json!({
        "id": format!("{:032x}", id.0),
        "name": descriptor.name.and_then(|name| interns.str(name)),
        "kind": format!("{:?}", descriptor.kind),
        "layout": format!("{:?}", descriptor.layout),
        "observations": format!("{:?}", descriptor.meta),
        "jsFacts": format!("{:?}", descriptor.js),
        "encodedDescriptorSha256": format!("{:x}", Sha256::digest(postcard::to_allocvec(descriptor)?)),
    }))
}

fn emit(
    path: &str,
    member: &str,
    archive_hash: &str,
    object: &Object,
) -> Result<(), Box<dyn Error>> {
    let interns = &object.interns;
    let mut rows = Vec::new();
    for item in &object.summaries {
        let body = object.body(item)?;
        let origins = object.origins(item)?;
        let mut types = BTreeSet::new();
        let mut calls = Vec::new();
        let signature = if let Some(body) = &body {
            types.extend(body.locals.iter().map(|local| local.ty));
            types.extend(body.signature.arguments.iter().copied());
            types.insert(body.signature.result);
            for (index, block) in body.blocks.iter().enumerate() {
                if let Terminator::Call(call) = &block.terminator {
                    types.extend(call.arguments.iter().map(|argument| argument.ty));
                    types.insert(call.destination.ty);
                    let callee = match &call.callee {
                        Callee::Direct { item, link_name } => {
                            json!({"kind": "direct", "item": identity(*item), "symbol": interns.str(*link_name)})
                        }
                        other => json!({"kind": "other", "descriptor": format!("{other:?}")}),
                    };
                    let source = origins
                        .as_ref()
                        .and_then(|table| {
                            table
                                .terminators
                                .get(index)
                                .and_then(|id| table.origin(*id))
                        })
                        .and_then(|origin| origin.source);
                    calls.push(json!({"block": block.id.0, "callee": callee,
                        "semantic": format!("{:?}", call.semantic), "hostPayload": call.host.as_ref().map(|host| format!("{host:?}")),
                        "arguments": call.arguments.iter().map(|argument| format!("{:032x}", argument.ty.0)).collect::<Vec<_>>(),
                        "result": format!("{:032x}", call.destination.ty.0), "location": location(interns, source),
                        "unwind": format!("{:?}", call.unwind)}));
                }
            }
            json!({"arguments": body.signature.arguments.iter().map(|id| format!("{:032x}", id.0)).collect::<Vec<_>>(),
                "result": format!("{:032x}", body.signature.result.0), "abi": format!("{:?}", body.signature.abi)})
        } else {
            Value::Null
        };
        rows.push(json!({
            "symbol": item.key.link_name, "item": item.key.item.map(identity),
            "semanticHash": item.semantic_hash.map(|hash| format!("{hash:?}")),
            "availability": format!("{:?}", item.availability), "class": item.class.names(),
            "kind": format!("{:?}", item.kind), "linkage": format!("{:?}", item.linkage),
            "bodySemantic": body.as_ref().map(|body| format!("{:?}", body.semantic)),
            "signature": signature, "typeIds": types.into_iter().map(|id| format!("{:032x}", id.0)).collect::<Vec<_>>(),
            "dependencySymbols": item.deps, "calls": calls, "baselineBytes": item.estimated_bytes,
            "itemPath": origins.as_ref().and_then(|table| interns.str(table.item_path)),
            "location": location(interns, origins.as_ref().and_then(|table| table.root)),
            "selectedCarrier": Value::Null,
        }));
    }
    println!(
        "{}",
        json!({"record": "object", "path": path, "member": member, "archiveSha256": archive_hash,
        "crate": object.header.crate_name, "emitArgs": object.header.emit_args, "header": format!("{:?}", object.header),
        "scope": "Object inventory before final linking. Reachability and selected carriers require the matching linker report; null is not a baseline selection.", "items": rows,
        "types": interns.types.descriptors().iter().map(|descriptor| type_info(interns, descriptor.id)).collect::<Result<Vec<_>, _>>()?})
    );
    Ok(())
}
