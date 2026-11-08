//! Isolated executable cleanup MIR transport in the dependency's own archive.

use super::*;
use crate::source_mir::PortableMir;
use rustc_hir::def_id::{CrateNum, DefId, LOCAL_CRATE};
use rustc_middle::ty::{GenericArgs, Instance};
use rustc_session::config::CrateType;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const MAGIC: &[u8] = b"RCGJS-MIR-3\0";
const MAX_BYTES: usize = 512 * 1024 * 1024;
static OUTPUT: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static INPUT: Mutex<BTreeMap<CrateNum, Option<Arc<CrateMir>>>> = Mutex::new(BTreeMap::new());

#[derive(Serialize, Deserialize)]
struct CrateMir {
    producer: [u8; 32],
    rustc: String,
    emit_args: String,
    target: String,
    data_layout: String,
    stable_crate: u64,
    crate_hash: String,
    definitions: Vec<DefinitionId>,
    mir: PortableMir,
}

pub(super) fn init() {
    *OUTPUT.lock().unwrap() = None;
    INPUT.lock().unwrap().clear();
}

fn enabled() -> bool {
    cleanup_probe()
}

pub(super) fn prepare(tcx: TyCtxt<'_>) {
    if !enabled() {
        return;
    }
    if !tcx.sess.opts.crate_types.contains(&CrateType::Rlib) {
        return;
    }
    let definitions: Vec<_> = CAPTURED
        .lock()
        .unwrap()
        .values()
        .map(|c| c.definition)
        .collect();
    let mut ids = Vec::new();
    let mut bodies = Vec::new();
    for definition in definitions {
        let def = definition.to_def_id();
        let instance = Instance::new_raw(def, GenericArgs::identity_for_item(tcx, def));
        if let Some(body) = cleanup_body(tcx, instance) {
            ids.push(crate::jir::types::definition_id(tcx, def));
            bodies.push(body);
        }
    }
    let count = ids.len();
    let value = CrateMir {
        producer: crate::identity::get(),
        rustc: tcx.sess.cfg_version.to_owned(),
        emit_args: rcgjs_object::item::normalized_emit_args(&tcx.sess.opts.cg.llvm_args),
        target: tcx.sess.opts.target_triple.to_string(),
        data_layout: tcx.sess.target.data_layout.to_string(),
        stable_crate: tcx.stable_crate_id(LOCAL_CRATE).as_u64(),
        crate_hash: format!("{:?}", tcx.crate_hash(LOCAL_CRATE)),
        definitions: ids,
        mir: PortableMir::capture_bodies(tcx, &bodies),
    };
    let payload = postcard::to_stdvec(&value).expect("encode cleanup MIR archive");
    assert!(
        payload.len() <= MAX_BYTES,
        "cleanup MIR archive is too large"
    );
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&Sha256::digest(&payload));
    bytes.extend_from_slice(&payload);
    eprintln!("cleanup-mir-export: {count} bodies, {} bytes", bytes.len());
    *OUTPUT.lock().unwrap() = Some(bytes);
}

pub(crate) fn artifact() -> Option<Vec<u8>> {
    OUTPUT.lock().unwrap().clone()
}

pub(super) fn body<'tcx>(tcx: TyCtxt<'tcx>, def: DefId) -> Option<Body<'tcx>> {
    if !enabled() {
        return None;
    }
    let cached = INPUT.lock().unwrap().get(&def.krate).cloned();
    let input = match cached {
        Some(value) => value,
        None => {
            let value = read(tcx, def.krate).unwrap_or_else(|error| tcx.dcx().fatal(error));
            INPUT.lock().unwrap().insert(def.krate, value.clone());
            value
        }
    }?;
    let identity = crate::jir::types::definition_id(tcx, def);
    let index = input.definitions.iter().position(|id| *id == identity)?;
    let body = input.mir.body_at(tcx, index);
    if tcx.is_coroutine(def) {
        let original = tcx.optimized_mir(def);
        assert_eq!(
            format!(
                "{:?}",
                body.coroutine
                    .as_ref()
                    .and_then(|c| c.coroutine_layout.as_ref())
            ),
            format!(
                "{:?}",
                original
                    .coroutine
                    .as_ref()
                    .and_then(|c| c.coroutine_layout.as_ref())
            ),
            "imported cleanup coroutine layout differs",
        );
    }
    eprintln!(
        "cleanup-mir-import: {def:?}, {} blocks",
        body.basic_blocks.len()
    );
    Some(body)
}

fn read(tcx: TyCtxt<'_>, krate: CrateNum) -> Result<Option<Arc<CrateMir>>, String> {
    let Some(path) = &tcx.used_crate_source(krate).rlib else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read cleanup MIR library: {e}"))?;
    let archive = object::read::archive::ArchiveFile::parse(&*bytes).map_err(|e| e.to_string())?;
    let mut result = None;
    for member in archive.members() {
        let member = member.map_err(|e| e.to_string())?;
        if !member
            .name()
            .split(|byte| *byte == b'.')
            .any(|part| part == b"rcgjs-mir")
        {
            continue;
        }
        if result.is_some() {
            return Err("duplicate cleanup MIR archive member".into());
        }
        let data = member.data(&*bytes).map_err(|e| e.to_string())?;
        let header = MAGIC.len() + 32;
        if data.len() < header || data.len() > MAX_BYTES + header || !data.starts_with(MAGIC) {
            return Err("invalid cleanup MIR archive header or size".into());
        }
        let payload = &data[header..];
        if data[MAGIC.len()..header] != Sha256::digest(payload)[..] {
            return Err("cleanup MIR archive checksum mismatch".into());
        }
        let (value, tail): (CrateMir, _) =
            postcard::take_from_bytes(payload).map_err(|e| e.to_string())?;
        if !tail.is_empty() {
            return Err("trailing cleanup MIR archive bytes".into());
        }
        if value.producer != crate::identity::get() {
            return Err("cleanup MIR archive producer binary identity mismatch".into());
        }
        if value.emit_args != rcgjs_object::item::normalized_emit_args(&tcx.sess.opts.cg.llvm_args)
            || value.rustc != tcx.sess.cfg_version
            || value.target != tcx.sess.opts.target_triple.to_string()
            || value.data_layout != tcx.sess.target.data_layout.as_ref()
            || value.stable_crate != tcx.stable_crate_id(krate).as_u64()
            || value.crate_hash != format!("{:?}", tcx.crate_hash(krate))
        {
            return Err("cleanup MIR archive compiler, target or crate identity mismatch".into());
        }
        let unique: std::collections::BTreeSet<_> = value.definitions.iter().collect();
        if value.definitions.len() != value.mir.body_count()
            || unique.len() != value.definitions.len()
            || value
                .definitions
                .iter()
                .any(|id| id.crate_id != value.stable_crate)
        {
            return Err("invalid cleanup MIR definition table".into());
        }
        result = Some(Arc::new(value));
    }
    Ok(result)
}
