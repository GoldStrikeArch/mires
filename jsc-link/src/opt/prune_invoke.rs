//! Whole-program sound pruning of provably non-throwing `RTerm::Invoke` terminators.
//!
//! Computes a monotone fixed point over the JIR call graph tracking which functions can transitively
//! reach a catchable JavaScript/host throw: a throwing `HostOperation` (`may_throw()`), a host /
//! view-ABI / raw-JS call, an unresolved or foreign (`extern "C-unwind"`) import, an explicit
//! `RTerm::Throw`, or an indirect/vtable/drop call not proven non-throwing. An `Invoke` whose
//! protected operation cannot throw a catchable exception is lowered to `RTerm::Goto` to its normal
//! edge, and unreachable unwind/cleanup blocks are swept by `dce::run_with`.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::refs::{self, RefScan};
use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{Func, Op, OpKind, RTerm};
use crate::jir::semantic::{
    BodyKind, CallSemantic, Callee, DropGlue, HostPayload, ItemId, VtableEntry,
};
use crate::jir::types::{TypeId, TypeKind, TypeTable};
use crate::late_ir::AllocatorProvider;

fn is_rust_alloc_shim_symbol(sym: &str) -> bool {
    sym.contains("___rust_alloc")
        || sym.contains("___rust_dealloc")
        || sym.contains("___rust_realloc")
        || sym.contains("___rust_alloc_zeroed")
        || sym.contains("___rust_alloc_error_handler")
        || sym.contains("___rust_no_alloc_shim_is_unstable")
}

/// Known non-host-throwing runtime helpers and abort/runtime symbols.
///
/// Any panic/abort helper here throws an `AbortError`, which every `TryCatch` handler
/// immediately rethrows (`if (__rt.is_abort($caught)) throw $caught;`) without running `Drop`s.
fn is_non_host_runtime_symbol(sym: &str) -> bool {
    if sym.ends_with("rust_begin_unwind") {
        return true;
    }
    matches!(
        sym,
        "alloc"
            | "dealloc"
            | "realloc"
            | "alloc_zeroed"
            | "alloc_error"
            | "panic"
            | "abort"
            | "abort_error"
            | "js_abort"
            | "unreachable"
            | "rust_begin_unwind"
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "bcmp"
            | "strlen"
            | "copy"
            | "copy_nonoverlapping"
            | "write_bytes"
            | "overwrite"
            | "_put"
            | "box"
            | "unscale"
            | "scale"
            | "retype"
            | "retype_rc"
            | "offset"
            | "offset_from"
            | "ptr_eq"
            | "slice_ptr_eq"
            | "fat_ptr_eq"
            | "ptr_cmp"
            | "addr"
            | "slice_from_parts"
            | "chunk_slice"
            | "thin"
            | "tail_raw"
            | "tail_ref"
            | "read_array"
            | "write_array"
            | "byte_slice_ref"
            | "str_bytes"
            | "str_bytes_mut"
            | "bytes_str"
            | "str_len"
            | "f2i"
            | "fmod"
            | "fmodf"
            | "clone"
            | "u8array_from_bytes"
            | "native_str_wf"
            | "native_str_utf16_index"
            | "native_str_byte_index"
            | "native_str_is_char_boundary"
            | "native_f64_to_string"
            | "native_str_push"
            | "native_str_pop"
            | "native_str_truncate"
            | "_sb_check"
            | "native_vec_new"
            | "typed_vec_push"
            | "native_vec_set_capacity"
            | "native_vec_set_len"
            | "convert_host_sb"
            | "convert_str_block"
            | "convert_sb_block"
            | "convert_nv_block"
            | "convert_block_nv"
            | "convert_sv_nv"
            | "ptr_is_null"
    )
}

fn is_abort_only_stdlib_item(item: &LinkItem) -> bool {
    let path = item.debug_path.as_str();
    let crate_name = item.crate_name.as_str();
    if !matches!(crate_name, "core" | "alloc" | "std")
        && !item.name.as_str().contains("rust_begin_unwind")
    {
        return false;
    }
    item.name.as_str().contains("rust_begin_unwind")
        || path.starts_with("core::panicking::")
        || path.starts_with("core::slice::index::slice_")
        || path.starts_with("alloc::raw_vec::handle_error")
        || path.starts_with("alloc::alloc::handle_alloc_error")
}

fn type_contains_fn_ptr(ty: TypeId, types: &TypeTable, seen: &mut BTreeSet<TypeId>) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    let Some(desc) = types.get(ty) else {
        return true;
    };
    match &desc.kind {
        TypeKind::FnPtr { .. } => true,
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            type_contains_fn_ptr(*pointee, types, seen)
        }
        TypeKind::Slice(elem) | TypeKind::Array { element: elem, .. } => {
            type_contains_fn_ptr(*elem, types, seen)
        }
        TypeKind::Tuple(fields) => fields.iter().any(|f| type_contains_fn_ptr(*f, types, seen)),
        TypeKind::Adt { variants, .. } => variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| type_contains_fn_ptr(f.ty, types, seen))
        }),
        _ => false,
    }
}

fn host_payload_cannot_throw(payload: &HostPayload, interns: &Interns) -> bool {
    let HostPayload::Marker {
        section,
        template: None,
        helpers,
        ..
    } = payload
    else {
        return false;
    };
    if !helpers.is_empty() {
        return false;
    }
    interns
        .str(*section)
        .and_then(|s| s.strip_prefix("rcgjs.host."))
        .and_then(rcgjs_object::host::HostOperation::from_marker)
        .is_some_and(|op| !op.may_throw())
}

/// Prunes `RTerm::Invoke` terminators whose protected operation cannot throw a catchable host
/// exception, converting them to `RTerm::Goto` and sweeping unreachable cleanup blocks with DCE.
///
/// Returns the number of functions modified.
pub(crate) fn run(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> usize {
    let interns_map: BTreeMap<JsName, &Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();

    let mut by_item: BTreeMap<ItemId, BTreeSet<JsName>> = BTreeMap::new();
    let mut scan = RefScan::default();
    let mut external_fn_ptr = false;
    let extra_roots = &crate::opts::get().roots;

    for (name, res) in funcs.iter() {
        let Ok(func) = res else { continue };
        by_item.entry(func.item).or_default().insert(name.clone());
        let types = interns_map.get(name).map(|i| &i.types);
        if let (Some(item), Some(types)) = (table.get(name), types) {
            let is_root = item.linkage.root || extra_roots.iter().any(|r| r == name.as_str());
            if is_root
                && func
                    .sig
                    .arguments
                    .iter()
                    .any(|&arg_ty| type_contains_fn_ptr(arg_ty, types, &mut BTreeSet::new()))
            {
                external_fn_ptr = true;
            }
        }
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, types, &mut scan);
                if let (Some(types), OpKind::Call { host: Some(_), .. } | OpKind::Host { .. }) =
                    (types, &op.kind)
                {
                    if let Some(res_val) = op.result {
                        if let Some(vdef) = func.values.get(res_val.0 as usize) {
                            if type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new()) {
                                external_fn_ptr = true;
                            }
                        }
                    }
                }
            }
        }
    }

    let has_custom_allocator = table
        .values()
        .any(|item| item.late_ir.artifact_allocator_provider() == AllocatorProvider::Custom);
    let non_throwing_external = |sym: &str| -> bool {
        is_non_host_runtime_symbol(sym) || (!has_custom_allocator && is_rust_alloc_shim_symbol(sym))
    };

    let mut has_opaque_fn = false;
    let mut vtable_methods: BTreeMap<u32, BTreeSet<JsName>> = BTreeMap::new();
    let mut vtable_drops: BTreeSet<JsName> = BTreeSet::new();
    let mut vtable_unknown = false;
    let mut static_refs: BTreeSet<JsName> = BTreeSet::new();

    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        match item.kind {
            ItemKind::Fn => {
                if !funcs.get(name).is_some_and(Result::is_ok)
                    && !non_throwing_external(name.as_str())
                    && !is_abort_only_stdlib_item(item)
                {
                    has_opaque_fn = true;
                }
            }
            ItemKind::Vtable => {
                let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns())
                else {
                    vtable_unknown = true;
                    continue;
                };
                let BodyKind::Vtable { entries, .. } = &body.kind else {
                    vtable_unknown = true;
                    continue;
                };
                for (slot, entry) in entries.iter().enumerate() {
                    match entry {
                        VtableEntry::Drop(Some(d)) => {
                            if let Some(s) = interns.str(d.link_name) {
                                vtable_drops.insert(JsName::new(s.to_owned()));
                            } else {
                                vtable_unknown = true;
                            }
                        }
                        VtableEntry::Method(m) => {
                            if let Some(s) = interns.str(m.link_name) {
                                vtable_methods
                                    .entry(slot as u32)
                                    .or_default()
                                    .insert(JsName::new(s.to_owned()));
                            } else {
                                vtable_unknown = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            ItemKind::Alloc | ItemKind::Import => {}
            _ => {
                static_refs.extend(item.refs.iter().cloned());
            }
        }
    }

    let ok_funcs: BTreeSet<JsName> = funcs
        .iter()
        .filter(|(_, res)| res.is_ok())
        .map(|(name, _)| name.clone())
        .collect();

    let abort_only_names: BTreeSet<JsName> = table
        .iter()
        .filter(|(_, item)| is_abort_only_stdlib_item(item))
        .map(|(name, _)| name.clone())
        .collect();

    let target_name_throws = |target: &JsName, may_throw: &BTreeSet<JsName>| -> bool {
        if abort_only_names.contains(target) {
            return false;
        }
        if ok_funcs.contains(target) {
            return may_throw.contains(target);
        }
        if let Some(item) = table.get(target) {
            return match item.kind {
                ItemKind::Alloc => false,
                ItemKind::Import => true,
                ItemKind::Fn => !non_throwing_external(target.as_str()),
                _ => true,
            };
        }
        !non_throwing_external(target.as_str())
    };

    let op_throws = |op: &Op, interns: &Interns, may_throw: &BTreeSet<JsName>| -> bool {
        let resolve =
            |id: StrId| -> Option<JsName> { interns.str(id).map(|s| JsName::new(s.to_owned())) };
        match &op.kind {
            OpKind::Host { payload, .. } => !host_payload_cannot_throw(payload, interns),
            OpKind::Intrinsic {
                fallback: Some(d), ..
            } => resolve(d.link_name).is_none_or(|n| target_name_throws(&n, may_throw)),
            OpKind::Intrinsic { fallback: None, .. } => false,
            OpKind::Drop {
                glue: DropGlue::None,
                ..
            } => false,
            OpKind::Drop {
                glue: DropGlue::Direct { link_name, .. },
                ..
            } => resolve(*link_name).is_none_or(|n| target_name_throws(&n, may_throw)),
            OpKind::Drop {
                glue: DropGlue::Dynamic,
                ..
            } => {
                vtable_unknown
                    || vtable_drops
                        .iter()
                        .any(|n| target_name_throws(n, may_throw))
            }
            OpKind::Call {
                callee,
                semantic,
                host,
                ..
            } => {
                if let Some(hp) = host {
                    return !host_payload_cannot_throw(hp, interns);
                }
                if matches!(
                    semantic,
                    CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
                ) {
                    return true;
                }
                if matches!(semantic, CallSemantic::RawParts) {
                    return false;
                }
                match callee {
                    Callee::Nop => false,
                    Callee::Direct { link_name, .. } => {
                        resolve(*link_name).is_none_or(|n| target_name_throws(&n, may_throw))
                    }
                    Callee::Intrinsic {
                        fallback: Some(d), ..
                    } => resolve(d.link_name).is_none_or(|n| target_name_throws(&n, may_throw)),
                    Callee::Intrinsic { fallback: None, .. } => false,
                    Callee::Runtime { member } => interns
                        .str(*member)
                        .is_none_or(|s| !is_non_host_runtime_symbol(s)),
                    Callee::Foreign { symbol, .. } => match interns.str(*symbol) {
                        None => true,
                        Some(sym) => {
                            let js_name = JsName::new(sym.to_owned());
                            if table.contains_key(&js_name) {
                                target_name_throws(&js_name, may_throw)
                            } else {
                                !non_throwing_external(sym)
                            }
                        }
                    },
                    Callee::Virtual { slot, .. } => {
                        vtable_unknown
                            || vtable_methods.get(slot).is_none_or(|ms| {
                                ms.iter().any(|m| target_name_throws(m, may_throw))
                            })
                    }
                    Callee::Indirect { signature } => {
                        external_fn_ptr
                            || has_opaque_fn
                            || static_refs
                                .iter()
                                .any(|n| ok_funcs.contains(n) && target_name_throws(n, may_throw))
                            || scan.fn_pointer_targets.get(signature).is_none_or(|items| {
                                items.iter().any(|it| {
                                    by_item.get(it).is_none_or(|names| {
                                        names.iter().any(|n| target_name_throws(n, may_throw))
                                    })
                                })
                            })
                    }
                }
            }
            _ => false,
        }
    };

    let mut may_throw_host: BTreeSet<JsName> = BTreeSet::new();
    loop {
        let mut changed = false;
        for (name, res) in funcs.iter() {
            if abort_only_names.contains(name) || may_throw_host.contains(name) {
                continue;
            }
            let Ok(func) = res else { continue };
            let Some(interns) = interns_map.get(name).copied() else {
                may_throw_host.insert(name.clone());
                changed = true;
                continue;
            };
            // Traverse only reachable blocks from entry, following an `Invoke` unwind edge only
            // when the protected op can actually throw; otherwise dead `RTerm::Throw(None)`
            // cleanup pads would falsely taint non-throwing callers and block transitive pruning.
            let mut throws = false;
            if !func.blocks.is_empty() {
                let mut visited = vec![false; func.blocks.len()];
                let mut work = vec![0usize];
                while let Some(bi) = work.pop() {
                    if visited.get(bi).copied().unwrap_or(true) {
                        continue;
                    }
                    visited[bi] = true;
                    let block = &func.blocks[bi];
                    if matches!(block.term, RTerm::Throw(_)) {
                        throws = true;
                        break;
                    }
                    match &block.term {
                        RTerm::Invoke { normal, unwind } => {
                            let protected_idx = block.ops.len().saturating_sub(1);
                            if block.ops[..protected_idx]
                                .iter()
                                .any(|op| op_throws(op, interns, &may_throw_host))
                            {
                                throws = true;
                                break;
                            }
                            work.push(normal.0 .0 as usize);
                            let last_throws = block
                                .ops
                                .last()
                                .is_some_and(|op| op_throws(op, interns, &may_throw_host));
                            if last_throws {
                                work.push(unwind.0 .0 as usize);
                            }
                        }
                        _ => {
                            if block
                                .ops
                                .iter()
                                .any(|op| op_throws(op, interns, &may_throw_host))
                            {
                                throws = true;
                                break;
                            }
                            for succ in block.term.successors() {
                                work.push(succ.0 as usize);
                            }
                        }
                    }
                }
            }
            if throws {
                may_throw_host.insert(name.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let func_names: Vec<JsName> = funcs.keys().cloned().collect();
    let mut modified = 0usize;
    for name in func_names {
        let Some(interns) = interns_map.get(&name).copied() else {
            continue;
        };
        let Some(Ok(func)) = funcs.get(&name) else {
            continue;
        };
        if !func.has_invoke() {
            continue;
        }
        let mut rewritten = func.clone();
        let mut pruned_any = false;
        let block_count = rewritten.blocks.len();
        for bi in 0..block_count {
            let (normal_target, normal_args, unwind_target) = match &rewritten.blocks[bi].term {
                RTerm::Invoke { normal, unwind } => (normal.0, normal.1.clone(), unwind.0),
                _ => continue,
            };
            let last_throws = rewritten.blocks[bi]
                .ops
                .last()
                .is_some_and(|op| op_throws(op, interns, &may_throw_host));
            let passthrough_rethrow = rewritten
                .blocks
                .get(unwind_target.0 as usize)
                .is_some_and(|u| u.ops.is_empty() && matches!(u.term, RTerm::Throw(None)));
            if !last_throws || passthrough_rethrow {
                rewritten.blocks[bi].term = RTerm::Goto(normal_target, normal_args);
                pruned_any = true;
            }
        }
        if !pruned_any {
            continue;
        }
        let dce_opts = crate::opt::dce::Options {
            no_constant_params: false,
            no_deletions: false,
            pure_call: None,
        };
        let _ =
            crate::opt::dce::run_with(&mut rewritten, &interns.types, &dce_opts, &mut Vec::new());
        if rewritten
            .verify_carriers(&interns.types, crate::opt::observed_carriers(&rewritten))
            .is_ok()
        {
            funcs.insert(name, Ok(rewritten));
            modified += 1;
        }
    }
    modified
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{item_map, LinkJir, Linkage};
    use crate::jir::interns::Interns;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{AbiSignature, DefinitionId, Signature, TypeId};
    use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeMeta};
    use crate::jir::OriginId;

    const O: OriginId = OriginId(0);
    const UNIT: TypeId = TypeId(0);

    fn item_id(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    fn test_item(name: &str, interns: Interns) -> LinkItem {
        let mut jir = LinkJir::default();
        jir.use_interns(interns);
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: item_map(Vec::new()),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir,
        }
    }

    fn unit_types(interns: &mut Interns) -> TypeId {
        interns.intern_type(TypeDescriptor {
            id: UNIT,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                sized: true,
                zst: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: UNIT,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        UNIT
    }

    #[test]
    fn prunes_non_throwing_direct_invoke_and_sweeps_cleanup() {
        let mut callee_interns = Interns::new();
        let unit = unit_types(&mut callee_interns);
        let callee_sig = Signature {
            arguments: Vec::new(),
            result: unit,
            abi: AbiSignature::default(),
        };
        let mut cb = FuncBuilder::new(item_id(1), callee_sig);
        cb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let callee_func = cb.finish();

        let mut caller_interns = Interns::new();
        let unit_caller = unit_types(&mut caller_interns);
        let target_str = caller_interns.intern_str("non_throwing_callee");
        let caller_sig = Signature {
            arguments: Vec::new(),
            result: unit_caller,
            abi: AbiSignature::default(),
        };
        let mut b = FuncBuilder::new(item_id(2), caller_sig);
        let b0 = FuncBuilder::ENTRY;
        let b_norm = b.block();
        let b_unw = b.block();
        b.effect(
            b0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(1),
                    link_name: target_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        b.terminate(
            b0,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        b.terminate(b_norm, RTerm::Return(None));
        b.terminate(b_unw, RTerm::Throw(None));
        let caller_func = b.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("non_throwing_callee".to_owned()),
            test_item("non_throwing_callee", callee_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );

        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("non_throwing_callee".to_owned()),
            Ok(callee_func),
        );
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller_func));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(modified, 1);

        let updated = funcs
            .get(&JsName::new("caller".to_owned()))
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(!updated.has_invoke(), "non-throwing invoke must be pruned");
        assert!(
            matches!(updated.blocks[0].term, RTerm::Goto(_, _)),
            "pruned invoke becomes Goto"
        );
    }

    #[test]
    fn preserves_invoke_reaching_throwing_host_op_and_foreign_import() {
        let mut host_callee_interns = Interns::new();
        let unit_h = unit_types(&mut host_callee_interns);
        let section_id = host_callee_interns.intern_str("rcgjs.host.invoke");
        let host_sig = Signature {
            arguments: Vec::new(),
            result: unit_h,
            abi: AbiSignature::default(),
        };
        let mut hb = FuncBuilder::new(item_id(10), host_sig);
        hb.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: section_id,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 10,
                    },
                    template: None,
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        hb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let host_callee = hb.finish();

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let host_target = caller_interns.intern_str("throwing_host_callee");
        let foreign_sym = caller_interns.intern_str("external_c_unwind_fn");
        let caller_sig = Signature {
            arguments: Vec::new(),
            result: unit_c,
            abi: AbiSignature::default(),
        };
        let mut cb = FuncBuilder::new(item_id(11), caller_sig);
        let c0 = FuncBuilder::ENTRY;
        let c_mid = cb.block();
        let c_norm = cb.block();
        let c_unw = cb.block();
        cb.effect(
            c0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(10),
                    link_name: host_target,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            c0,
            RTerm::Invoke {
                normal: (c_mid, Vec::new()),
                unwind: (c_unw, Vec::new()),
            },
        );
        cb.effect(
            c_mid,
            OpKind::Call {
                callee: Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 12,
                    },
                    symbol: foreign_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            c_mid,
            RTerm::Invoke {
                normal: (c_norm, Vec::new()),
                unwind: (c_unw, Vec::new()),
            },
        );
        cb.terminate(c_norm, RTerm::Return(None));
        cb.effect(
            c_unw,
            OpKind::Call {
                callee: Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 13,
                    },
                    symbol: foreign_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(c_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("throwing_host_callee".to_owned()),
            test_item("throwing_host_callee", host_callee_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("throwing_host_callee".to_owned()),
            Ok(host_callee),
        );
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(modified, 0);
        let updated = funcs
            .get(&JsName::new("caller".to_owned()))
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(matches!(updated.blocks[0].term, RTerm::Invoke { .. }));
        assert!(matches!(updated.blocks[1].term, RTerm::Invoke { .. }));
    }

    #[test]
    fn prunes_two_hop_invoke_chain_with_throw_none_cleanup_pads() {
        let mut leaf_interns = Interns::new();
        let unit_leaf = unit_types(&mut leaf_interns);
        let mut lb = FuncBuilder::new(
            item_id(10),
            Signature {
                arguments: Vec::new(),
                result: unit_leaf,
                abi: AbiSignature::default(),
            },
        );
        lb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let leaf = lb.finish();

        let mut inner_interns = Interns::new();
        let unit_inner = unit_types(&mut inner_interns);
        let leaf_sym = inner_interns.intern_str("leaf");
        let mut ib = FuncBuilder::new(
            item_id(11),
            Signature {
                arguments: Vec::new(),
                result: unit_inner,
                abi: AbiSignature::default(),
            },
        );
        let i0 = FuncBuilder::ENTRY;
        let i_norm = ib.block();
        let i_unw = ib.block();
        ib.effect(
            i0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(10),
                    link_name: leaf_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        ib.terminate(
            i0,
            RTerm::Invoke {
                normal: (i_norm, Vec::new()),
                unwind: (i_unw, Vec::new()),
            },
        );
        ib.terminate(i_norm, RTerm::Return(None));
        ib.terminate(i_unw, RTerm::Throw(None));
        let inner = ib.finish();

        let mut outer_interns = Interns::new();
        let unit_outer = unit_types(&mut outer_interns);
        let inner_sym = outer_interns.intern_str("inner");
        let mut ob = FuncBuilder::new(
            item_id(12),
            Signature {
                arguments: Vec::new(),
                result: unit_outer,
                abi: AbiSignature::default(),
            },
        );
        let o0 = FuncBuilder::ENTRY;
        let o_norm = ob.block();
        let o_unw = ob.block();
        ob.effect(
            o0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(11),
                    link_name: inner_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        ob.terminate(
            o0,
            RTerm::Invoke {
                normal: (o_norm, Vec::new()),
                unwind: (o_unw, Vec::new()),
            },
        );
        ob.terminate(o_norm, RTerm::Return(None));
        ob.terminate(o_unw, RTerm::Throw(None));
        let outer = ob.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("leaf".to_owned()),
            test_item("leaf", leaf_interns),
        );
        table.insert(
            JsName::new("inner".to_owned()),
            test_item("inner", inner_interns),
        );
        table.insert(
            JsName::new("outer".to_owned()),
            test_item("outer", outer_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("leaf".to_owned()), Ok(leaf));
        funcs.insert(JsName::new("inner".to_owned()), Ok(inner));
        funcs.insert(JsName::new("outer".to_owned()), Ok(outer));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(modified, 2, "both inner and outer Invokes must be pruned");
        assert!(!funcs[&JsName::new("inner".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
        assert!(!funcs[&JsName::new("outer".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn preserves_standard_op_invoke_when_direct_callee_throws() {
        use crate::jir::semantic::StandardOperation;

        let mut fmt_interns = Interns::new();
        let unit_fmt = unit_types(&mut fmt_interns);
        let section_id = fmt_interns.intern_str("rcgjs.host.invoke");
        let mut fb = FuncBuilder::new(
            item_id(20),
            Signature {
                arguments: Vec::new(),
                result: unit_fmt,
                abi: AbiSignature::default(),
            },
        );
        fb.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: section_id,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 20,
                    },
                    template: None,
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        fb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let throwing_fmt = fb.finish();

        let mut caller_interns = Interns::new();
        let unit_caller = unit_types(&mut caller_interns);
        let fmt_sym = caller_interns.intern_str("fmt_to_string_throwing");
        let cleanup_sym = caller_interns.intern_str("cleanup_pad");
        let mut cb = FuncBuilder::new(
            item_id(21),
            Signature {
                arguments: Vec::new(),
                result: unit_caller,
                abi: AbiSignature::default(),
            },
        );
        let b0 = FuncBuilder::ENTRY;
        let b_norm = cb.block();
        let b_unw = cb.block();
        cb.effect(
            b0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(20),
                    link_name: fmt_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::FormatArgsToString),
                host: None,
            },
            O,
        );
        cb.terminate(
            b0,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.terminate(b_norm, RTerm::Return(None));
        cb.effect(
            b_unw,
            OpKind::Call {
                callee: Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 22,
                    },
                    symbol: cleanup_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(b_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("fmt_to_string_throwing".to_owned()),
            test_item("fmt_to_string_throwing", fmt_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("fmt_to_string_throwing".to_owned()),
            Ok(throwing_fmt),
        );
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(
            modified, 0,
            "Standard op with throwing Callee::Direct body must not be pruned"
        );
        assert!(funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn prunes_v0_mangled_alloc_shim_and_passthrough_rethrow_invoke() {
        let mut leaf_interns = Interns::new();
        let unit_leaf = unit_types(&mut leaf_interns);
        let rust_alloc_sym = leaf_interns.intern_str("_RNvCsxyz_7___rustc12___rust_alloc");
        let rt_sym = leaf_interns.intern_str("native_str_wf");
        let mut lb = FuncBuilder::new(
            item_id(30),
            Signature {
                arguments: Vec::new(),
                result: unit_leaf,
                abi: AbiSignature::default(),
            },
        );
        lb.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 31,
                    },
                    symbol: rust_alloc_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        lb.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Runtime { member: rt_sym },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        lb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let non_throwing_alloc_fn = lb.finish();

        // Throwing callee that guarantees an unwind edge cannot be dropped by calleepurity alone,
        // yet a caller Invoke targeting a pure pass-through `Throw(None)` unwind block IS pruned.
        let mut throw_interns = Interns::new();
        let unit_throw = unit_types(&mut throw_interns);
        let section_id = throw_interns.intern_str("rcgjs.host.invoke");
        let mut tb = FuncBuilder::new(
            item_id(32),
            Signature {
                arguments: Vec::new(),
                result: unit_throw,
                abi: AbiSignature::default(),
            },
        );
        tb.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: section_id,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 32,
                    },
                    template: None,
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        tb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let throwing_callee = tb.finish();

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let alloc_fn_sym = caller_interns.intern_str("non_throwing_alloc_fn");
        let throw_fn_sym = caller_interns.intern_str("throwing_callee");
        let mut cb = FuncBuilder::new(
            item_id(33),
            Signature {
                arguments: Vec::new(),
                result: unit_c,
                abi: AbiSignature::default(),
            },
        );
        let b0 = FuncBuilder::ENTRY;
        let b_mid = cb.block();
        let b_norm = cb.block();
        let b_unw = cb.block();
        cb.effect(
            b0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(30),
                    link_name: alloc_fn_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            b0,
            RTerm::Invoke {
                normal: (b_mid, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.effect(
            b_mid,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(32),
                    link_name: throw_fn_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            b_mid,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.terminate(b_norm, RTerm::Return(None));
        cb.terminate(b_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("non_throwing_alloc_fn".to_owned()),
            test_item("non_throwing_alloc_fn", leaf_interns),
        );
        table.insert(
            JsName::new("throwing_callee".to_owned()),
            test_item("throwing_callee", throw_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("non_throwing_alloc_fn".to_owned()),
            Ok(non_throwing_alloc_fn),
        );
        funcs.insert(
            JsName::new("throwing_callee".to_owned()),
            Ok(throwing_callee),
        );
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(modified, 1);
        assert!(
            !funcs[&JsName::new("caller".to_owned())]
                .as_ref()
                .unwrap()
                .has_invoke(),
            "both non-throwing alloc shim invoke and pass-through rethrow invoke must prune"
        );
    }

    #[test]
    fn preserves_invoke_to_throwing_custom_global_allocator_and_rust_prefixed_import() {
        let mut alloc_interns = Interns::new();
        let unit_alloc = unit_types(&mut alloc_interns);
        let section_id = alloc_interns.intern_str("rcgjs.host.invoke");
        let mut ab = FuncBuilder::new(
            item_id(40),
            Signature {
                arguments: Vec::new(),
                result: unit_alloc,
                abi: AbiSignature::default(),
            },
        );
        ab.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: section_id,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 40,
                    },
                    template: None,
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        ab.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let throwing_alloc = ab.finish();

        let alloc_name = "_RNvCsabc_7___rustc12___rust_alloc";
        let mut caller_interns = Interns::new();
        let unit_caller = unit_types(&mut caller_interns);
        let alloc_sym = caller_interns.intern_str(alloc_name);
        let import_sym = caller_interns.intern_str("___rust_foo_import");
        let mut cb = FuncBuilder::new(
            item_id(41),
            Signature {
                arguments: Vec::new(),
                result: unit_caller,
                abi: AbiSignature::default(),
            },
        );
        let b0 = FuncBuilder::ENTRY;
        let b_mid = cb.block();
        let b_norm = cb.block();
        let b_unw = cb.block();
        cb.effect(
            b0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(40),
                    link_name: alloc_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            b0,
            RTerm::Invoke {
                normal: (b_mid, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.effect(
            b_mid,
            OpKind::Call {
                callee: Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 42,
                    },
                    symbol: import_sym,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(
            b_mid,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.terminate(b_norm, RTerm::Return(None));
        cb.effect(
            b_unw,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: caller_interns.intern_str("rcgjs.host.isnull"),
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 43,
                    },
                    template: None,
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        cb.terminate(b_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut alloc_item = test_item(alloc_name, alloc_interns);
        alloc_item.late_ir.allocator_provider = AllocatorProvider::Custom;
        let mut import_item = test_item("___rust_foo_import", Interns::new());
        import_item.kind = ItemKind::Import;

        let mut table = BTreeMap::new();
        table.insert(JsName::new(alloc_name.to_owned()), alloc_item);
        table.insert(JsName::new("___rust_foo_import".to_owned()), import_item);
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new(alloc_name.to_owned()), Ok(throwing_alloc));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(
            modified, 0,
            "throwing custom global allocator and ___rust_* import must not be pruned"
        );
        let updated = funcs[&JsName::new("caller".to_owned())].as_ref().unwrap();
        assert!(matches!(updated.blocks[0].term, RTerm::Invoke { .. }));
        assert!(matches!(updated.blocks[1].term, RTerm::Invoke { .. }));
    }
}
