//! Whole-program sound pruning of provably non-throwing `RTerm::Invoke` terminators.
//!
//! Computes a monotone fixed point over the JIR call graph tracking which functions can transitively
//! reach a catchable JavaScript/host throw: a throwing `HostOperation` (`may_throw()`), a host /
//! view-ABI / raw-JS call, an unresolved or foreign (`extern "C-unwind"`) import, an explicit
//! `RTerm::Throw`, or an indirect/vtable/drop call not proven non-throwing. An `Invoke` whose
//! protected operation cannot throw a catchable exception is lowered to `RTerm::Goto` to its normal
//! edge, and unreachable unwind/cleanup blocks are swept by `dce::run_with`.

use std::collections::{BTreeMap, BTreeSet};

use super::witness::{self, InvokePruneReason, InvokePrunedSite};
use crate::analysis::refs::{self, RefScan};
use crate::item::AllocatorProvider;
use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{CellId, Func, Op, OpKind, RBlockId, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, BodyKind, BodySemantic, CallSemantic, Callee, CastKind, DropGlue, HostPayload,
    ItemId, UnaryOp, VtableEntry,
};
use crate::jir::types::{TypeId, TypeKind, TypeTable};

/// Statistics and witness outcomes for one [`run_checked`] pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PruneInvokeStats {
    pub modified: usize,
    pub witnesses_recorded: u32,
    pub witnesses_failed: u32,
}

/// Known non-host-throwing sysroot runtime helpers and abort/runtime symbols.
///
/// Any panic/abort helper here throws an `AbortError`, which every `TryCatch` handler
/// immediately rethrows (`if (__rt.is_abort($caught)) throw $caught;`) without running `Drop`s.
/// Consulted ONLY for `Callee::Runtime { sysroot: true, .. }`.
///
/// Deliberately excluded after line-by-line audit of `runtime/shim.js`:
/// - `bstr_utf8_bytes`, `bstr_str_in`: throw bare `TypeError` on non-string values.
/// - `bstr_ref`, `bstr_ref_in`: construct Proxy membranes / call `bstr_str_in` / `Reflect.*`.
/// - `bstr_sub`, `bstr_sub_from`, `bstr_sub_to`: invoke caller-supplied `fail` callback argument.
pub(crate) fn is_non_throwing_sysroot_shim_member(sym: &str) -> bool {
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
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "bcmp"
            | "strlen"
            | "copy"
            | "copy_nonoverlapping"
            | "write_bytes"
            | "compare_bytes"
            | "overwrite"
            | "_put"
            | "box"
            | "prefix"
            | "unproject"
            | "erase"
            | "unref"
            | "unscale"
            | "unscale_scalar"
            | "unscale_uninit"
            | "scale"
            | "scale_scalar"
            | "byte_sign"
            | "read_byte"
            | "write_byte"
            | "read_byte_at"
            | "write_byte_at"
            | "retype"
            | "retype_rc"
            | "offset"
            | "offset_from"
            | "ptr_eq"
            | "slice_ptr_eq"
            | "fat_ptr_eq"
            | "ptr_cmp"
            | "slice_ptr_cmp"
            | "fat_ptr_cmp"
            | "addr"
            | "slice_from_parts"
            | "sfp"
            | "nstr_into_vec"
            | "nstr_ascii_lower"
            | "nstr_ascii_upper"
            | "bstr_cmp"
            | "bstr_find_char"
            | "bstr_rfind_char"
            | "bstr_find_str"
            | "bstr_rfind_str"
            | "chunk_slice"
            | "thin"
            | "tail_raw"
            | "tail_ref"
            | "tail_slice"
            | "tail_slice_raw"
            | "read_array"
            | "array_ref"
            | "write_array"
            | "unwindow"
            | "unwindow_slice"
            | "byte_slice_ref"
            | "str_bytes"
            | "str_bytes_mut"
            | "detach_str_memo"
            | "bytes_str"
            | "str_len"
            | "bstr_in"
            | "bstr_out"
            | "bstr_char"
            | "bstr_bytes"
            | "bstr_bytes_mut"
            | "bstr_of"
            | "bstr_boundary"
            | "bstr_get"
            | "bstr_get_from"
            | "bstr_get_to"
            | "bytes_is_ascii"
            | "bstr_trim"
            | "bstr_trim_start"
            | "bstr_trim_end"
            | "sb_new"
            | "sb_push"
            | "sb_push_char"
            | "sb_take"
            | "map_new"
            | "map_len"
            | "map_clear"
            | "map_has"
            | "map_set"
            | "map_get"
            | "map_del"
            | "map_keys"
            | "map_values"
            | "f2i"
            | "f2i_big"
            | "i2f32_big"
            | "fmod"
            | "fmodf"
            | "clone"
            | "u8array_from_bytes"
            | "native_f64_to_string"
            | "native_f32_to_string"
            | "fmt_float_fixed"
            | "fmt_int_hex"
            | "fmt_int_dec"
            | "rcgjs_fmt_float_parts"
            | "parse_f64"
            | "parse_f32"
            | "native_vec_new"
            | "typed_vec_push"
            | "native_vec_set_capacity"
            | "native_vec_set_len"
            | "ptr_is_null"
            | "host_str_to_lowercase"
            | "host_str_to_uppercase"
            | "_put_plain"
            | "_put_tracked"
            | "t_box_new_tracked"
            | "t_alloc_arr"
            | "t_alloc_z"
            | "t_realloc_ta"
            | "t_realloc_ta_checked"
            | "t_realloc_arr"
            | "t_realloc_arr_checked"
            | "t_dealloc_maybe"
            | "nv_grow_u8"
            | "nv_grow_i8"
            | "nv_grow_u16"
            | "nv_grow_i16"
            | "nv_grow_u32"
            | "nv_grow_i32"
            | "nv_grow_f32"
            | "nv_grow_f64"
            | "nv_with_cap_u8"
            | "nv_with_cap_i8"
            | "nv_with_cap_u16"
            | "nv_with_cap_i16"
            | "nv_with_cap_u32"
            | "nv_with_cap_i32"
            | "nv_with_cap_f32"
            | "nv_with_cap_f64"
            | "nv_zeroed_u8"
            | "nv_zeroed_i8"
            | "nv_zeroed_u16"
            | "nv_zeroed_i16"
            | "nv_zeroed_u32"
            | "nv_zeroed_i32"
            | "nv_zeroed_f32"
            | "nv_zeroed_f64"
            | "nv_reserve_u8"
            | "nv_reserve_i8"
            | "nv_reserve_u16"
            | "nv_reserve_i16"
            | "nv_reserve_u32"
            | "nv_reserve_i32"
            | "nv_reserve_f32"
            | "nv_reserve_f64"
            | "nv_zeroed_arr_p"
            | "nv_zeroed_arr_fn_p"
            | "nv_zeroed_arr_fn_tracked_p"
            | "nv_resize_zero_ta"
    )
}

pub(crate) fn type_contains_fn_ptr(
    ty: TypeId,
    types: &TypeTable,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
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

/// Whether `name` is the crate graph's `#[panic_handler]` (`LangItem::PanicImpl`), established
/// strictly from the rustc semantic tag (`BodySemantic::PanicImpl`), never from symbol names.
///
/// Both the JIR and baseline emitters wrap `PanicImpl` bodies in `try { ... } catch ($e) { throw
/// __rt.abort_error($e); }` (unless already an unconditional literal abort), so calling one can
/// only diverge or raise a registered abort — never a catchable host exception.
pub(crate) fn is_panic_impl_item(
    table: &BTreeMap<JsName, LinkItem>,
    name: &(impl AsRef<str> + ?Sized),
) -> bool {
    table
        .get(name.as_ref())
        .filter(|item| item.kind == ItemKind::Fn)
        .and_then(|item| item.jir.body.as_ref())
        .is_some_and(|body| body.semantic == BodySemantic::PanicImpl)
}

pub(crate) fn host_payload_cannot_throw(payload: &HostPayload, interns: &Interns) -> bool {
    match payload {
        HostPayload::Map { marker } => interns.str(*marker).is_some_and(|m| {
            matches!(
                m,
                "new"
                    | "len"
                    | "clear"
                    | "has"
                    | "set"
                    | "get"
                    | "getmut"
                    | "try_get"
                    | "try_getmut"
                    | "del"
                    | "keys"
                    | "values"
            )
        }),
        HostPayload::Marker {
            section, helpers, ..
        } => {
            if !helpers.is_empty() {
                return false;
            }
            interns
                .str(*section)
                .and_then(|s| s.strip_prefix("rcgjs.host."))
                .and_then(rcgjs_object::host::HostOperation::from_marker)
                .is_some_and(|op| !op.may_throw())
        }
        _ => false,
    }
}

/// Prunes `RTerm::Invoke` terminators whose protected operation cannot throw a catchable host
/// exception, converting them to `RTerm::Goto` and sweeping unreachable cleanup blocks with DCE.
///
/// Returns the number of functions modified.
#[cfg(test)]
pub(crate) fn run(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> usize {
    run_checked(table, funcs, live).modified
}

/// Whole-program scope and monotone throw-reachability solver for `prune_invoke`.
struct ThrowAnalysis<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    interns_map: BTreeMap<JsName, &'a Interns>,
    ok_funcs: BTreeSet<JsName>,
    by_item: BTreeMap<ItemId, BTreeSet<JsName>>,
    scan: RefScan,
    external_fn_ptr: bool,
    has_custom_allocator: bool,
    has_opaque_fn: bool,
    vtable_methods: BTreeMap<u32, BTreeSet<JsName>>,
    vtable_drops: BTreeSet<JsName>,
    vtable_unknown: bool,
    static_refs: BTreeSet<JsName>,
}

impl<'a> ThrowAnalysis<'a> {
    fn collect_program_scope(
        table: &'a BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        live: &BTreeSet<JsName>,
    ) -> Self {
        let interns_map: BTreeMap<JsName, &'a Interns> = table
            .iter()
            .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
            .collect();

        let ok_funcs: BTreeSet<JsName> = funcs
            .iter()
            .filter(|(_, res)| res.is_ok())
            .map(|(name, _)| name.clone())
            .collect();

        let mut by_item: BTreeMap<ItemId, BTreeSet<JsName>> = BTreeMap::new();
        let mut scan = RefScan::default();
        let mut external_fn_ptr = false;
        let extra_roots = &crate::opts::get().roots;

        for (name, res) in funcs.iter() {
            let Ok(func) = res else { continue };
            by_item.entry(func.item).or_default().insert(name.clone());
            let interns = interns_map.get(name).copied();
            let types = interns.map(|i| &i.types);
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
                    let fn_ptr_boundary = match &op.kind {
                        OpKind::Host { args, .. }
                        | OpKind::Call {
                            host: Some(_),
                            args,
                            ..
                        } => Some(args.as_slice()),
                        OpKind::Call {
                            callee: Callee::Runtime { sysroot: false, .. },
                            args,
                            ..
                        } => Some(args.as_slice()),
                        OpKind::Call {
                            callee: Callee::Foreign { symbol, .. },
                            args,
                            ..
                        } => {
                            let known_ok = interns
                                .and_then(|i| i.str(*symbol))
                                .is_some_and(|s| ok_funcs.contains(s));
                            (!known_ok).then_some(args.as_slice())
                        }
                        _ => None,
                    };
                    if let Some(args) = fn_ptr_boundary {
                        let Some(types) = types else {
                            external_fn_ptr = true;
                            continue;
                        };
                        if let Some(res_val) = op.result {
                            if let Some(vdef) = func.values.get(res_val.0 as usize) {
                                if type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new()) {
                                    external_fn_ptr = true;
                                }
                            }
                        }
                        if args.iter().any(|arg| {
                            func.values.get(arg.0 as usize).is_some_and(|vdef| {
                                type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new())
                            })
                        }) {
                            external_fn_ptr = true;
                        }
                    }
                }
            }
        }

        let has_custom_allocator = table.values().any(|item| {
            matches!(
                item.allocator_provider,
                AllocatorProvider::Custom | AllocatorProvider::Conflict
            )
        });

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
                    if !funcs.get(name).is_some_and(Result::is_ok) {
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

        Self {
            table,
            interns_map,
            ok_funcs,
            by_item,
            scan,
            external_fn_ptr,
            has_custom_allocator,
            has_opaque_fn,
            vtable_methods,
            vtable_drops,
            vtable_unknown,
            static_refs,
        }
    }

    fn target_name_throws(&self, target: &str, may_throw: &BTreeSet<JsName>) -> bool {
        if is_panic_impl_item(self.table, target) {
            return false;
        }
        if self.ok_funcs.contains(target) {
            return may_throw.contains(target);
        }
        if let Some(item) = self.table.get(target) {
            return match item.kind {
                ItemKind::Alloc => self.has_custom_allocator,
                _ => true,
            };
        }
        true
    }

    fn intrinsic_without_fallback_throws(
        &self,
        name: StrId,
        args: &[ValueId],
        func: &Func,
        interns: &Interns,
        may_throw: &BTreeSet<JsName>,
    ) -> bool {
        let Some(sym) = interns.str(name) else {
            return true;
        };
        match sym {
            "catch_unwind" => true,
            "const_eval_select" => {
                let Some(runtime_arg) = args.get(2) else {
                    return true;
                };
                let Some(vdef) = func.values.get(runtime_arg.0 as usize) else {
                    return true;
                };
                let peeled = interns.types.peeled(vdef.ty);
                let Some(desc) = interns.types.get(peeled) else {
                    return true;
                };
                let TypeKind::FnDef {
                    instance: Some(instance),
                    ..
                } = &desc.kind
                else {
                    return true;
                };
                let Some(target) = interns.str(instance.link_name) else {
                    return true;
                };
                crate::lower::js::host::is_marker_name(target)
                    || self.target_name_throws(target, may_throw)
            }
            _ => false,
        }
    }

    fn op_throws(
        &self,
        func: &Func,
        op: &Op,
        interns: &Interns,
        may_throw: &BTreeSet<JsName>,
    ) -> bool {
        match &op.kind {
            OpKind::Host { payload, .. } => !host_payload_cannot_throw(payload, interns),
            OpKind::Intrinsic {
                fallback: Some(d), ..
            } => interns
                .str(d.link_name)
                .is_none_or(|n| self.target_name_throws(n, may_throw)),
            OpKind::Intrinsic {
                name,
                fallback: None,
                args,
                ..
            } => self.intrinsic_without_fallback_throws(*name, args, func, interns, may_throw),
            OpKind::Drop {
                glue: DropGlue::None,
                ..
            } => false,
            OpKind::Drop {
                glue: DropGlue::Direct { link_name, .. },
                ..
            } => interns
                .str(*link_name)
                .is_none_or(|n| self.target_name_throws(n, may_throw)),
            OpKind::Drop {
                glue: DropGlue::Dynamic,
                ..
            } => {
                self.vtable_unknown
                    || self
                        .vtable_drops
                        .iter()
                        .any(|n| self.target_name_throws(n.as_str(), may_throw))
            }
            OpKind::Call {
                callee,
                args,
                semantic,
                host,
                ..
            } => {
                if let Some(hp) = host {
                    return !host_payload_cannot_throw(hp, interns);
                }
                if matches!(semantic, CallSemantic::Host | CallSemantic::RawJavaScript) {
                    return true;
                }
                match callee {
                    Callee::Nop => false,
                    Callee::Direct { link_name, .. } => interns.str(*link_name).is_none_or(|s| {
                        crate::lower::js::host::is_marker_name(s)
                            || self.target_name_throws(s, may_throw)
                    }),
                    Callee::Intrinsic {
                        fallback: Some(d), ..
                    } => interns
                        .str(d.link_name)
                        .is_none_or(|n| self.target_name_throws(n, may_throw)),
                    Callee::Intrinsic {
                        name,
                        fallback: None,
                        ..
                    } => self
                        .intrinsic_without_fallback_throws(*name, args, func, interns, may_throw),
                    Callee::Runtime { member, sysroot } => {
                        !*sysroot
                            || interns
                                .str(*member)
                                .is_none_or(|s| !is_non_throwing_sysroot_shim_member(s))
                    }
                    Callee::Foreign { symbol, .. } => match interns.str(*symbol) {
                        Some(sym) if self.table.contains_key(sym) => {
                            self.target_name_throws(sym, may_throw)
                        }
                        _ => true,
                    },
                    Callee::Virtual { slot, .. } => {
                        self.vtable_unknown
                            || self.vtable_methods.get(slot).is_none_or(|ms| {
                                ms.iter()
                                    .any(|m| self.target_name_throws(m.as_str(), may_throw))
                            })
                    }
                    Callee::Indirect { signature } => {
                        self.external_fn_ptr
                            || self.has_opaque_fn
                            || self.static_refs.iter().any(|n| {
                                (self.ok_funcs.contains(n)
                                    || self.table.get(n).is_some_and(|it| {
                                        matches!(it.kind, ItemKind::Fn | ItemKind::Import)
                                    }))
                                    && self.target_name_throws(n.as_str(), may_throw)
                            })
                            || self.scan.named.iter().any(|it| {
                                self.by_item.get(it).is_none_or(|names| {
                                    names
                                        .iter()
                                        .any(|n| self.target_name_throws(n.as_str(), may_throw))
                                })
                            })
                            || self
                                .scan
                                .fn_pointer_targets
                                .get(signature)
                                .is_none_or(|items| {
                                    items.iter().any(|it| {
                                        self.by_item.get(it).is_none_or(|names| {
                                            names.iter().any(|n| {
                                                self.target_name_throws(n.as_str(), may_throw)
                                            })
                                        })
                                    })
                                })
                    }
                }
            }
            _ => false,
        }
    }

    fn solve_may_throw(&self, funcs: &BTreeMap<JsName, Result<Func, Decline>>) -> BTreeSet<JsName> {
        let mut may_throw_host: BTreeSet<JsName> = BTreeSet::new();
        loop {
            let mut changed = false;
            for (name, res) in funcs.iter() {
                if may_throw_host.contains(name) || is_panic_impl_item(self.table, name) {
                    continue;
                }
                let Ok(func) = res else { continue };
                let Some(interns) = self.interns_map.get(name).copied() else {
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
                                let protected_idx = block.protected_op_index().unwrap_or(0);
                                if block.ops[..protected_idx]
                                    .iter()
                                    .any(|op| self.op_throws(func, op, interns, &may_throw_host))
                                {
                                    throws = true;
                                    break;
                                }
                                work.push(normal.0 .0 as usize);
                                let last_throws = block.protected_op().is_some_and(|op| {
                                    self.op_throws(func, op, interns, &may_throw_host)
                                });
                                if last_throws {
                                    work.push(unwind.0 .0 as usize);
                                }
                            }
                            _ => {
                                if block
                                    .ops
                                    .iter()
                                    .any(|op| self.op_throws(func, op, interns, &may_throw_host))
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
        may_throw_host
    }

    fn rewrite_prunable_invokes(
        &self,
        pre_func: &Func,
        rewritten: &mut Func,
        interns: &Interns,
        may_throw_host: &BTreeSet<JsName>,
        noop_funcs: &BTreeSet<JsName>,
    ) -> Option<(Vec<InvokePrunedSite>, Vec<witness::Witness>)> {
        let mut pruned_sites = Vec::new();
        let block_count = rewritten.blocks.len();
        for bi in 0..block_count {
            let (normal_target, normal_args, unwind_target) = match &rewritten.blocks[bi].term {
                RTerm::Invoke { normal, unwind } => (normal.0, normal.1.clone(), unwind.0),
                _ => continue,
            };
            let block = &rewritten.blocks[bi];
            let protected_idx = block.protected_op_index().unwrap_or(0);
            let prefix_throws = block.ops[..protected_idx]
                .iter()
                .any(|op| self.op_throws(rewritten, op, interns, may_throw_host));
            let last_throws = block
                .protected_op()
                .is_some_and(|op| self.op_throws(rewritten, op, interns, may_throw_host));
            if !prefix_throws && !last_throws {
                rewritten.blocks[bi].term = RTerm::Goto(normal_target, normal_args);
                pruned_sites.push(InvokePrunedSite {
                    block: RBlockId(bi as u32),
                    reason: InvokePruneReason::ProtectedOpCannotThrow,
                });
            } else if is_noop_unwind_chain(pre_func, interns, unwind_target, noop_funcs) {
                rewritten.blocks[bi].term = RTerm::Goto(normal_target, normal_args);
                pruned_sites.push(InvokePrunedSite {
                    block: RBlockId(bi as u32),
                    reason: InvokePruneReason::NoopUnwindChain,
                });
            }
        }
        if pruned_sites.is_empty() {
            return None;
        }
        let dce_opts = crate::opt::dce::Options {
            no_constant_params: false,
            no_deletions: false,
            pure_call: None,
        };
        let mut dce_witnesses = Vec::new();
        let _ = crate::opt::dce::run_with(rewritten, &interns.types, &dce_opts, &mut dce_witnesses);
        Some((pruned_sites, dce_witnesses))
    }

    fn commit_verified_prunes(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        live: &BTreeSet<JsName>,
        may_throw_host: &BTreeSet<JsName>,
    ) -> PruneInvokeStats {
        let noop_funcs: BTreeSet<JsName> = funcs
            .iter()
            .filter_map(|(name, res)| {
                let f = res.as_ref().ok()?;
                (f.blocks.len() == 1
                    && f.blocks[0].ops.is_empty()
                    && matches!(f.blocks[0].term, RTerm::Return(None)))
                .then(|| name.clone())
            })
            .collect();

        let func_names: Vec<JsName> = funcs.keys().cloned().collect();
        let mut stats = PruneInvokeStats::default();
        let mut approved = Vec::new();
        for name in func_names {
            let Some(interns) = self.interns_map.get(&name).copied() else {
                continue;
            };
            let Some(Ok(pre_func)) = funcs.get(&name) else {
                continue;
            };
            if !pre_func.has_invoke() {
                continue;
            }
            match super::tx::try_rewrite_func(
                pre_func,
                &interns.types,
                |rewritten| {
                    self.rewrite_prunable_invokes(
                        pre_func,
                        rewritten,
                        interns,
                        may_throw_host,
                        &noop_funcs,
                    )
                },
                |rewritten, (pruned_sites, dce_witnesses)| {
                    witness::check_all(dce_witnesses, rewritten, &interns.types).is_ok()
                        && witness::check_invoke_pruning(
                            pre_func,
                            interns,
                            pruned_sites,
                            self.table,
                            funcs,
                            live,
                        )
                        .is_ok()
                },
            ) {
                super::tx::TxOutcome::Unchanged => {}
                super::tx::TxOutcome::RolledBack => {
                    stats.witnesses_failed += 1;
                }
                super::tx::TxOutcome::Committed {
                    func: rewritten, ..
                } => {
                    stats.witnesses_recorded += 1;
                    approved.push((name, rewritten));
                }
            }
        }

        for (name, rewritten) in approved {
            funcs.insert(name, Ok(rewritten));
            stats.modified += 1;
        }
        stats
    }
}

/// Prunes `RTerm::Invoke` terminators with independent witness checking and per-function rollback.
pub(crate) fn run_checked(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> PruneInvokeStats {
    let analysis = ThrowAnalysis::collect_program_scope(table, funcs, live);
    let may_throw = analysis.solve_may_throw(funcs);
    analysis.commit_verified_prunes(funcs, live, &may_throw)
}

const MAX_NOOP_UNWIND_BLOCKS: usize = 64;

fn collect_escaped_cells(func: &Func) -> BTreeSet<CellId> {
    let mut escaped = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(cell),
                    ..
                }
                | OpKind::Drop {
                    root: Root::Cell(cell),
                    glue: DropGlue::Direct { .. } | DropGlue::Dynamic,
                    ..
                } => {
                    escaped.insert(*cell);
                }
                _ => {}
            }
        }
    }
    escaped
}

fn is_bool_or_int(ty: TypeId, types: &TypeTable) -> bool {
    matches!(
        types.get(types.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Bool | TypeKind::Integer { .. })
    )
}

fn is_noop_unwind_chain(
    func: &Func,
    interns: &Interns,
    start: RBlockId,
    noop_funcs: &BTreeSet<JsName>,
) -> bool {
    check_noop_unwind_chain(func, interns, start, |link_name| {
        let name = interns
            .str(link_name)
            .ok_or_else(|| format!("unresolvable str#{}", link_name.0))?;
        if noop_funcs.contains(name) {
            Ok(())
        } else {
            Err(format!("unwind callee `{name}` is not an empty return"))
        }
    })
    .is_ok()
}

pub(crate) fn check_noop_unwind_chain<F>(
    func: &Func,
    interns: &Interns,
    start: RBlockId,
    mut check_empty_callee: F,
) -> Result<(), String>
where
    F: FnMut(StrId) -> Result<(), String>,
{
    let is_used = |v: ValueId| -> bool {
        func.blocks.iter().any(|b| {
            let mut found = false;
            for op in &b.ops {
                op.kind.for_each_use(|u| {
                    if u == v {
                        found = true;
                    }
                });
                if found {
                    return true;
                }
            }
            b.term.for_each_use(|u| {
                if u == v {
                    found = true;
                }
            });
            found
        })
    };
    let mut escaped_cells: Option<BTreeSet<CellId>> = None;

    // 3-color iterative DFS over the cleanup subgraph starting at `start`:
    // absent = unvisited, 1 = visiting (on active path), 2 = finished (acyclic, all paths
    // terminate in `RTerm::Throw(None)`). Any back-edge (color == 1) rejects cycles.
    let mut color: BTreeMap<RBlockId, u8> = BTreeMap::new();
    let mut stack: Vec<(RBlockId, bool)> = vec![(start, false)];
    while let Some((cur, exiting)) = stack.pop() {
        if exiting {
            color.insert(cur, 2);
            continue;
        }
        match color.get(&cur).copied() {
            Some(2) => continue,
            Some(1) => return Err(format!("noop unwind DAG cycles at bb{}", cur.0)),
            _ => {}
        }
        if color.len() >= MAX_NOOP_UNWIND_BLOCKS {
            return Err("noop unwind DAG exceeds 64-block bound".to_owned());
        }
        let block = func
            .blocks
            .get(cur.0 as usize)
            .ok_or_else(|| format!("missing unwind block bb{}", cur.0))?;
        for op in &block.ops {
            match &op.kind {
                OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Const(_)
                | OpKind::Field { .. }
                | OpKind::CellLoad(_)
                | OpKind::BorrowView { .. }
                | OpKind::RawView { .. }
                | OpKind::Discriminant(_) => {}
                OpKind::CellStore { cell, .. } => {
                    let escaped = escaped_cells.get_or_insert_with(|| collect_escaped_cells(func));
                    if escaped.contains(cell) {
                        return Err(format!(
                            "cell store to escaped cell c{} in unwind block bb{}",
                            cell.0, cur.0
                        ));
                    }
                }
                OpKind::Unary {
                    op: UnaryOp::Not | UnaryOp::Neg,
                    ty,
                    ..
                } => {
                    if !is_bool_or_int(*ty, &interns.types) {
                        return Err(format!(
                            "unary op on non-bool/int type t{} in unwind block bb{}",
                            ty.0, cur.0
                        ));
                    }
                }
                OpKind::Binary {
                    op:
                        BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor,
                    ty,
                    ..
                } => {
                    if !is_bool_or_int(*ty, &interns.types) {
                        return Err(format!(
                            "binary op on non-bool/int type t{} in unwind block bb{}",
                            ty.0, cur.0
                        ));
                    }
                }
                OpKind::Cast {
                    kind: CastKind::IntToInt | CastKind::Subtype,
                    ..
                } => {}
                OpKind::Drop { glue, .. } => match glue {
                    DropGlue::None => {}
                    DropGlue::Direct { link_name, .. } => check_empty_callee(*link_name)?,
                    DropGlue::Dynamic => {
                        return Err(format!("dynamic drop in unwind block bb{}", cur.0));
                    }
                },
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    semantic: CallSemantic::Ordinary,
                    host: None,
                    function: None,
                    ..
                } => {
                    if let Some(res) = op.result {
                        if is_used(res) {
                            return Err(format!("unwind call result v{} is used", res.0));
                        }
                    }
                    check_empty_callee(*link_name)?;
                }
                OpKind::Call {
                    callee: Callee::Nop,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                    function: None,
                    ..
                } => {
                    if let Some(res) = op.result {
                        if is_used(res) {
                            return Err(format!("unwind nop call result v{} is used", res.0));
                        }
                    }
                }
                other => {
                    return Err(format!("non-noop op {other:?} in unwind block bb{}", cur.0));
                }
            }
        }
        color.insert(cur, 1);
        stack.push((cur, true));
        match &block.term {
            RTerm::Throw(None) => {}
            RTerm::Goto(next, _) => {
                if color.get(next).copied() == Some(1) {
                    return Err(format!("noop unwind DAG cycles at bb{}", next.0));
                }
                if !color.contains_key(next) {
                    stack.push((*next, false));
                }
            }
            RTerm::Switch { cases, default, .. } => {
                if color.get(&default.0).copied() == Some(1) {
                    return Err(format!("noop unwind DAG cycles at bb{}", default.0 .0));
                }
                if !color.contains_key(&default.0) {
                    stack.push((default.0, false));
                }
                for &(_, target, _) in cases {
                    if color.get(&target).copied() == Some(1) {
                        return Err(format!("noop unwind DAG cycles at bb{}", target.0));
                    }
                    if !color.contains_key(&target) {
                        stack.push((target, false));
                    }
                }
            }
            other => {
                return Err(format!(
                    "unwind block bb{} ends in non-noop terminator {other:?}",
                    cur.0
                ));
            }
        }
    }
    if color.is_empty() {
        Err("noop unwind DAG is empty".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-block JIR fixtures inline"
)]
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
            allocator_provider: Default::default(),
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
        let rt_sym = leaf_interns.intern_str("str_len");
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
                callee: Callee::Runtime {
                    member: rt_sym,
                    sysroot: true,
                },
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

        let mut alloc_shim_item = test_item("_RNvCsxyz_7___rustc12___rust_alloc", Interns::new());
        alloc_shim_item.kind = ItemKind::Alloc;
        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("_RNvCsxyz_7___rustc12___rust_alloc".to_owned()),
            alloc_shim_item,
        );
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
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        cb.terminate(b_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut alloc_item = test_item(alloc_name, alloc_interns);
        alloc_item.allocator_provider = AllocatorProvider::Custom;
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

    #[test]
    fn prunes_invoke_with_noop_unwind_chain_and_preserves_exceeding_hop_limit() {
        use crate::jir::repr::Root;
        use crate::jir::semantic::ConstTree;

        let sig = |result| Signature {
            arguments: Vec::new(),
            result,
            abi: AbiSignature::default(),
        };
        let mut noop_b = FuncBuilder::new(item_id(10), sig(UNIT));
        noop_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let noop_fn = noop_b.finish();

        let mut non_noop_interns = Interns::new();
        let unit_non_noop = unit_types(&mut non_noop_interns);
        let host_sec = non_noop_interns.intern_str("rcgjs.host.isnull");
        let mut non_noop_b = FuncBuilder::new(item_id(11), sig(unit_non_noop));
        non_noop_b.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section: host_sec,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 11,
                    },
                    helpers: Vec::new(),
                },
                args: Vec::new(),
            },
            O,
        );
        non_noop_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let non_noop_fn = non_noop_b.finish();

        // Caller has three throwing Invokes:
        // 1) unwinds through a 4-block chain (Copy/Move + DropGlue::None + Direct noop drop +
        //    Direct noop call -> Throw(None)) => PRUNED;
        // 2) unwinds through a 65-block Goto chain (> 64-block DAG cap) => PRESERVED;
        // 3) unwinds through a block calling `non_noop_fn` => PRESERVED.
        let mut interns = Interns::new();
        interns.intern_type(TypeDescriptor {
            id: UNIT,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                size: Some(0),
                zst: true,
                sized: true,
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
        let noop_str = interns.intern_str("noop_drop");
        let non_noop_str = interns.intern_str("side_effect");
        let throwing_host = HostPayload::Marker {
            section: interns.intern_str("rcgjs.host.invoke"),
            def: DefinitionId {
                crate_id: 0,
                local_hash: 50,
            },
            helpers: Vec::new(),
        };

        let mut cb = FuncBuilder::new(item_id(1), sig(UNIT));
        let unit_val = cb.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), UNIT, O);
        let b1 = cb.block();
        let b2 = cb.block();
        let b_done = cb.block();

        // Invoke #1 (entry): unwinds via 4 noop blocks -> Throw(None) => pruned.
        let u1_a = cb.block();
        let u1_b = cb.block();
        let u1_c = cb.block();
        let u1_d = cb.block();
        cb.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: throwing_host.clone(),
                args: Vec::new(),
            },
            O,
        );
        cb.terminate(
            FuncBuilder::ENTRY,
            RTerm::Invoke {
                normal: (b1, Vec::new()),
                unwind: (u1_a, Vec::new()),
            },
        );
        let copied = cb.op(u1_a, OpKind::Copy(unit_val), UNIT, O);
        let moved = cb.op(u1_a, OpKind::Move(copied), UNIT, O);
        cb.effect(
            u1_a,
            OpKind::Drop {
                root: Root::Value(moved),
                path: Vec::new(),
                glue: DropGlue::None,
            },
            O,
        );
        cb.terminate(u1_a, RTerm::Goto(u1_b, Vec::new()));
        cb.effect(
            u1_b,
            OpKind::Drop {
                root: Root::Value(unit_val),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item_id(10),
                    link_name: noop_str,
                },
            },
            O,
        );
        cb.terminate(u1_b, RTerm::Goto(u1_c, Vec::new()));
        let _unused_res = cb.op(
            u1_c,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(10),
                    link_name: noop_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            UNIT,
            O,
        );
        cb.effect(
            u1_c,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(10),
                    link_name: noop_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(u1_c, RTerm::Goto(u1_d, Vec::new()));
        cb.terminate(u1_d, RTerm::Throw(None));

        // Invoke #2 (b1): unwinds via 65 blocks (> 64-block cap) -> Throw(None) => preserved.
        let mut long_chain = Vec::with_capacity(65);
        for _ in 0..65 {
            long_chain.push(cb.block());
        }
        cb.effect(
            b1,
            OpKind::Host {
                payload: throwing_host.clone(),
                args: Vec::new(),
            },
            O,
        );
        cb.terminate(
            b1,
            RTerm::Invoke {
                normal: (b2, Vec::new()),
                unwind: (long_chain[0], Vec::new()),
            },
        );
        for w in long_chain.windows(2) {
            cb.terminate(w[0], RTerm::Goto(w[1], Vec::new()));
        }
        cb.terminate(long_chain[64], RTerm::Throw(None));

        // Invoke #3 (b2): unwinds via a block calling `non_noop_fn` => preserved.
        let u3 = cb.block();
        cb.effect(
            b2,
            OpKind::Host {
                payload: throwing_host,
                args: Vec::new(),
            },
            O,
        );
        cb.terminate(
            b2,
            RTerm::Invoke {
                normal: (b_done, Vec::new()),
                unwind: (u3, Vec::new()),
            },
        );
        cb.effect(
            u3,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(11),
                    link_name: non_noop_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(u3, RTerm::Throw(None));
        cb.terminate(b_done, RTerm::Return(None));

        let caller = cb.finish();
        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("noop_drop".to_owned()),
            test_item("noop_drop", Interns::new()),
        );
        table.insert(
            JsName::new("side_effect".to_owned()),
            test_item("side_effect", non_noop_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("noop_drop".to_owned()), Ok(noop_fn));
        funcs.insert(JsName::new("side_effect".to_owned()), Ok(non_noop_fn));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let modified = run(&table, &mut funcs, &live);
        assert_eq!(modified, 1);
        let updated = funcs[&JsName::new("caller".to_owned())].as_ref().unwrap();
        assert!(matches!(updated.blocks[0].term, RTerm::Goto(..)));
        assert!(matches!(updated.blocks[1].term, RTerm::Invoke { .. }));
        assert!(matches!(updated.blocks[2].term, RTerm::Invoke { .. }));
    }

    fn build_caller_with_cleanup_drop(
        caller_interns: &mut Interns,
        unit: TypeId,
        callee: Callee,
        drop_name: &str,
    ) -> Func {
        use crate::jir::repr::Root;
        use crate::jir::semantic::ConstTree;

        let drop_str = caller_interns.intern_str(drop_name);
        let mut cb = FuncBuilder::new(
            item_id(101),
            Signature {
                arguments: Vec::new(),
                result: unit,
                abi: AbiSignature::default(),
            },
        );
        let unit_val = cb.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), unit, O);
        let b0 = FuncBuilder::ENTRY;
        let b_norm = cb.block();
        let b_unw = cb.block();
        cb.effect(
            b0,
            OpKind::Call {
                callee,
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
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        cb.terminate(b_norm, RTerm::Return(None));
        cb.effect(
            b_unw,
            OpKind::Drop {
                root: Root::Value(unit_val),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item_id(102),
                    link_name: drop_str,
                },
            },
            O,
        );
        cb.terminate(b_unw, RTerm::Throw(None));
        cb.finish()
    }

    fn build_non_empty_drop(drop_interns: &mut Interns, unit: TypeId) -> Func {
        let host_throw = drop_interns.intern_str("host_note");
        let mut db = FuncBuilder::new(
            item_id(102),
            Signature {
                arguments: Vec::new(),
                result: unit,
                abi: AbiSignature::default(),
            },
        );
        db.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: host_throw,
                    sysroot: false,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        db.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        db.finish()
    }

    #[test]
    fn preserves_invoke_calling_user_rust_begin_unwind_and_drop() {
        let mut unwind_interns = Interns::new();
        let unit_u = unit_types(&mut unwind_interns);
        let host_throw = unwind_interns.intern_str("host_throw");
        let mut ub = FuncBuilder::new(
            item_id(100),
            Signature {
                arguments: Vec::new(),
                result: unit_u,
                abi: AbiSignature::default(),
            },
        );
        ub.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: host_throw,
                    sysroot: false,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        ub.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let user_unwind = ub.finish();

        let mut drop_interns = Interns::new();
        let unit_d = unit_types(&mut drop_interns);
        let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let unwind_str = caller_interns.intern_str("user_rust_begin_unwind");
        let caller = build_caller_with_cleanup_drop(
            &mut caller_interns,
            unit_c,
            Callee::Direct {
                item: item_id(100),
                link_name: unwind_str,
            },
            "real_drop",
        );

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("user_rust_begin_unwind".to_owned()),
            test_item("user_rust_begin_unwind", unwind_interns),
        );
        table.insert(
            JsName::new("real_drop".to_owned()),
            test_item("real_drop", drop_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("user_rust_begin_unwind".to_owned()),
            Ok(user_unwind),
        );
        funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 0);
        assert!(funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn preserves_non_sysroot_runtime_imports_and_unresolved_foreign() {
        for sym in ["clone", "copy", "alloc", "custom_rust_begin_unwind"] {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let sym_id = caller_interns.intern_str(sym);
            let caller = build_caller_with_cleanup_drop(
                &mut caller_interns,
                unit_c,
                Callee::Runtime {
                    member: sym_id,
                    sysroot: false,
                },
                "real_drop",
            );

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            table.insert(
                JsName::new("caller".to_owned()),
                test_item("caller", caller_interns),
            );
            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(
                stats.modified, 0,
                "non-sysroot Callee::Runtime `{sym}` must not be pruned"
            );
            assert!(funcs[&JsName::new("caller".to_owned())]
                .as_ref()
                .unwrap()
                .has_invoke());
        }

        // Unresolved Callee::Foreign not in `table` must also not be pruned even if named `clone`.
        let mut drop_interns = Interns::new();
        let unit_d = unit_types(&mut drop_interns);
        let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let foreign_sym = caller_interns.intern_str("clone");
        let caller = build_caller_with_cleanup_drop(
            &mut caller_interns,
            unit_c,
            Callee::Foreign {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 109,
                },
                symbol: foreign_sym,
            },
            "real_drop",
        );

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("real_drop".to_owned()),
            test_item("real_drop", drop_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 0);
        assert!(funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn preserves_spoofed_core_crate_fn_calling_throwing_host() {
        let mut spoof_interns = Interns::new();
        let unit_s = unit_types(&mut spoof_interns);
        let host_throw = spoof_interns.intern_str("host_throw");
        let mut sb = FuncBuilder::new(
            item_id(110),
            Signature {
                arguments: Vec::new(),
                result: unit_s,
                abi: AbiSignature::default(),
            },
        );
        sb.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: host_throw,
                    sysroot: false,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        sb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let spoof_fn = sb.finish();

        let mut spoof_item = test_item("core::panicking::panic_spoof", spoof_interns);
        spoof_item.crate_name = "core".to_owned();
        spoof_item.debug_path = "core::panicking::panic".to_owned();

        let mut drop_interns = Interns::new();
        let unit_d = unit_types(&mut drop_interns);
        let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let spoof_str = caller_interns.intern_str("core::panicking::panic_spoof");
        let caller = build_caller_with_cleanup_drop(
            &mut caller_interns,
            unit_c,
            Callee::Direct {
                item: item_id(110),
                link_name: spoof_str,
            },
            "real_drop",
        );

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("core::panicking::panic_spoof".to_owned()),
            spoof_item,
        );
        table.insert(
            JsName::new("real_drop".to_owned()),
            test_item("real_drop", drop_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("core::panicking::panic_spoof".to_owned()),
            Ok(spoof_fn),
        );
        funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 0);
        assert!(funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn negative_witness_rejects_false_claims_and_fault_injection_rolls_back() {
        // 1) False ProtectedOpCannotThrow: callee calls `user_rust_begin_unwind` (throwing).
        {
            let mut unwind_interns = Interns::new();
            let unit_u = unit_types(&mut unwind_interns);
            let host_throw = unwind_interns.intern_str("host_throw");
            let mut ub = FuncBuilder::new(
                item_id(120),
                Signature {
                    arguments: Vec::new(),
                    result: unit_u,
                    abi: AbiSignature::default(),
                },
            );
            ub.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: host_throw,
                        sysroot: false,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                O,
            );
            ub.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let user_unwind = ub.finish();

            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let unwind_str = caller_interns.intern_str("user_rust_begin_unwind");
            let caller = build_caller_with_cleanup_drop(
                &mut caller_interns,
                unit_c,
                Callee::Direct {
                    item: item_id(120),
                    link_name: unwind_str,
                },
                "real_drop",
            );

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("user_rust_begin_unwind".to_owned()),
                test_item("user_rust_begin_unwind", unwind_interns),
            );
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(
                JsName::new("user_rust_begin_unwind".to_owned()),
                Ok(user_unwind),
            );
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let false_protected = [InvokePrunedSite {
                block: FuncBuilder::ENTRY,
                reason: InvokePruneReason::ProtectedOpCannotThrow,
            }];
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &false_protected,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "false ProtectedOpCannotThrow for user_rust_begin_unwind must fail"
            );

            let false_noop = [InvokePrunedSite {
                block: FuncBuilder::ENTRY,
                reason: InvokePruneReason::NoopUnwindChain,
            }];
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &false_noop,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "false NoopUnwindChain with real DropGlue::Direct must fail"
            );
        }

        // 2) False ProtectedOpCannotThrow: direct non-sysroot `Callee::Runtime { member: "clone", sysroot: false }`.
        {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let clone_str = caller_interns.intern_str("clone");
            let caller = build_caller_with_cleanup_drop(
                &mut caller_interns,
                unit_c,
                Callee::Runtime {
                    member: clone_str,
                    sysroot: false,
                },
                "real_drop",
            );

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let false_protected = [InvokePrunedSite {
                block: FuncBuilder::ENTRY,
                reason: InvokePruneReason::ProtectedOpCannotThrow,
            }];
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &false_protected,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "false ProtectedOpCannotThrow for non-sysroot clone must fail"
            );
        }

        // 3) `RCGJS_WITNESS_FAULT=prune-invoke` rolls back all Invokes and increments `witnesses_failed`.
        {
            let mut callee_interns = Interns::new();
            let unit_callee = unit_types(&mut callee_interns);
            let mut cb = FuncBuilder::new(
                item_id(130),
                Signature {
                    arguments: Vec::new(),
                    result: unit_callee,
                    abi: AbiSignature::default(),
                },
            );
            cb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let callee_func = cb.finish();

            let mut caller_interns = Interns::new();
            let unit_caller = unit_types(&mut caller_interns);
            let target_str = caller_interns.intern_str("non_throwing_callee");
            let mut b = FuncBuilder::new(
                item_id(131),
                Signature {
                    arguments: Vec::new(),
                    result: unit_caller,
                    abi: AbiSignature::default(),
                },
            );
            let b0 = FuncBuilder::ENTRY;
            let b_norm = b.block();
            let b_unw = b.block();
            b.effect(
                b0,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item_id(130),
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

            let _fault = witness::ScopedWitnessFault::new("prune-invoke");
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(stats.modified, 0, "fault injection must roll back");
            assert!(stats.witnesses_failed > 0);
            assert_eq!(stats.witnesses_recorded, 0);
            assert!(
                funcs[&JsName::new("caller".to_owned())]
                    .as_ref()
                    .unwrap()
                    .has_invoke(),
                "rolled-back caller must preserve its RTerm::Invoke"
            );
        }
    }

    #[test]
    fn preserves_raw_parts_semantic_when_callee_throws_and_witness_rejects() {
        use crate::jir::repr::Root;
        use crate::jir::semantic::ConstTree;

        let mut drop_interns = Interns::new();
        let unit_d = unit_types(&mut drop_interns);
        let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let copy_sym = caller_interns.intern_str("copy");
        let drop_str = caller_interns.intern_str("real_drop");
        let mut cb = FuncBuilder::new(
            item_id(140),
            Signature {
                arguments: Vec::new(),
                result: unit_c,
                abi: AbiSignature::default(),
            },
        );
        let unit_val = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Unit),
            unit_c,
            O,
        );
        let b0 = FuncBuilder::ENTRY;
        let b_norm = cb.block();
        let b_unw = cb.block();
        cb.effect(
            b0,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: copy_sym,
                    sysroot: false,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::RawParts,
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
            OpKind::Drop {
                root: Root::Value(unit_val),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item_id(102),
                    link_name: drop_str,
                },
            },
            O,
        );
        cb.terminate(b_unw, RTerm::Throw(None));
        let caller = cb.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("real_drop".to_owned()),
            test_item("real_drop", drop_interns),
        );
        let caller_item = test_item("caller", caller_interns);
        let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
        table.insert(JsName::new("caller".to_owned()), caller_item);

        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 0, "RawParts must not bypass callee check");
        assert!(funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
        assert!(
            witness::check_invoke_pruning(
                &caller,
                &caller_interns_ref,
                &[InvokePrunedSite {
                    block: FuncBuilder::ENTRY,
                    reason: InvokePruneReason::ProtectedOpCannotThrow,
                }],
                &table,
                &funcs,
                &live,
            )
            .is_err(),
            "witness must reject RawParts with non-sysroot callee"
        );
    }

    #[test]
    fn preserves_indirect_call_with_const_global_fn_or_external_fn_ptr_and_witness_rejects() {
        use crate::jir::interns::SigId;
        use crate::jir::semantic::{CastKind, ConstTree, DirectItemRef, GlobalReference};

        fn intern_fn_types(interns: &mut Interns) -> (TypeId, TypeId, TypeId, TypeId) {
            let unit = unit_types(interns);
            let fn_ptr = TypeId(1);
            let pure_fndef = TypeId(2);
            let throw_fndef = TypeId(3);
            let pure_str = interns.intern_str("pure_fn");
            let throw_str = interns.intern_str("throw_fn");
            let mut intern_kind = |id: TypeId, kind: TypeKind| {
                interns.intern_type(TypeDescriptor {
                    id,
                    name: None,
                    kind,
                    layout: LayoutFacts {
                        sized: true,
                        ..LayoutFacts::default()
                    },
                    meta: TypeMeta::default(),
                    js: JsFacts {
                        int: None,
                        is_indirect: false,
                        needs_clone: false,
                        crosses_transparent_union: false,
                        transparent: None,
                        peeled: id,
                        unsized_tail: None,
                        enum_repr: None,
                        is_primitive_enum: false,
                        pointer_form: None,
                        byte_element: false,
                        fields: Vec::new(),
                    },
                });
            };
            intern_kind(fn_ptr, TypeKind::FnPtr { sig: SigId(0) });
            intern_kind(
                pure_fndef,
                TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 150,
                    },
                    sig: SigId(0),
                    instance: Some(DirectItemRef {
                        item: item_id(150),
                        link_name: pure_str,
                    }),
                },
            );
            intern_kind(
                throw_fndef,
                TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 151,
                    },
                    sig: SigId(0),
                    instance: Some(DirectItemRef {
                        item: item_id(151),
                        link_name: throw_str,
                    }),
                },
            );
            (unit, fn_ptr, pure_fndef, throw_fndef)
        }

        // Case A: `ConstTree::Global::Function` (`scan.named`) points to a throwing fn while
        // `fn_pointer_targets[fn_ptr]` only sees `pure_fn`.
        {
            let mut pure_interns = Interns::new();
            let (unit_p, _, _, _) = intern_fn_types(&mut pure_interns);
            let mut pb = FuncBuilder::new(
                item_id(150),
                Signature {
                    arguments: Vec::new(),
                    result: unit_p,
                    abi: AbiSignature::default(),
                },
            );
            pb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let pure_fn = pb.finish();

            let mut throw_interns = Interns::new();
            let (unit_t, _, _, _) = intern_fn_types(&mut throw_interns);
            let host_throw = throw_interns.intern_str("host_throw");
            let mut tb = FuncBuilder::new(
                item_id(151),
                Signature {
                    arguments: Vec::new(),
                    result: unit_t,
                    abi: AbiSignature::default(),
                },
            );
            tb.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: host_throw,
                        sysroot: false,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                O,
            );
            tb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let throw_fn = tb.finish();

            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let (unit_c, fn_ptr, pure_fndef, _throw_fndef) = intern_fn_types(&mut caller_interns);
            let drop_str = caller_interns.intern_str("real_drop");
            let throw_link = caller_interns.intern_str("throw_fn");
            let mut cb = FuncBuilder::new(
                item_id(152),
                Signature {
                    arguments: Vec::new(),
                    result: unit_c,
                    abi: AbiSignature::default(),
                },
            );
            let b0 = FuncBuilder::ENTRY;
            let b_norm = cb.block();
            let b_unw = cb.block();
            let pure_val = cb.op(b0, OpKind::Const(ConstTree::Unit), pure_fndef, O);
            let _reified = cb.op(
                b0,
                OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v: pure_val,
                    from: pure_fndef,
                    to: fn_ptr,
                },
                fn_ptr,
                O,
            );
            let const_throw_ptr = cb.op(
                b0,
                OpKind::Const(ConstTree::Global {
                    target: GlobalReference::Function {
                        item: item_id(151),
                        link_name: throw_link,
                    },
                    offset: 0,
                }),
                fn_ptr,
                O,
            );
            cb.effect(
                b0,
                OpKind::Call {
                    callee: Callee::Indirect { signature: fn_ptr },
                    function: Some(const_throw_ptr),
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
                    normal: (b_norm, Vec::new()),
                    unwind: (b_unw, Vec::new()),
                },
            );
            cb.terminate(b_norm, RTerm::Return(None));
            cb.effect(
                b_unw,
                OpKind::Drop {
                    root: crate::jir::repr::Root::Value(pure_val),
                    path: Vec::new(),
                    glue: DropGlue::Direct {
                        item: item_id(102),
                        link_name: drop_str,
                    },
                },
                O,
            );
            cb.terminate(b_unw, RTerm::Throw(None));
            let caller = cb.finish();

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("pure_fn".to_owned()),
                test_item("pure_fn", pure_interns),
            );
            table.insert(
                JsName::new("throw_fn".to_owned()),
                test_item("throw_fn", throw_interns),
            );
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("pure_fn".to_owned()), Ok(pure_fn));
            funcs.insert(JsName::new("throw_fn".to_owned()), Ok(throw_fn));
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(
                stats.modified, 0,
                "ConstTree::Global::Function must block indirect prune"
            );
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &[InvokePrunedSite {
                        block: FuncBuilder::ENTRY,
                        reason: InvokePruneReason::ProtectedOpCannotThrow,
                    }],
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "witness must reject indirect prune when scan.named throws"
            );
        }

        // Case B: Non-sysroot `Callee::Runtime { sysroot: false }` returns a `FnPtr`.
        {
            let mut pure_interns = Interns::new();
            let (unit_p, _, _, _) = intern_fn_types(&mut pure_interns);
            let mut pb = FuncBuilder::new(
                item_id(150),
                Signature {
                    arguments: Vec::new(),
                    result: unit_p,
                    abi: AbiSignature::default(),
                },
            );
            pb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let pure_fn = pb.finish();

            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let (unit_c, fn_ptr, pure_fndef, _) = intern_fn_types(&mut caller_interns);
            let drop_str = caller_interns.intern_str("real_drop");
            let get_cb = caller_interns.intern_str("get_host_cb");
            let mut cb = FuncBuilder::new(
                item_id(153),
                Signature {
                    arguments: Vec::new(),
                    result: unit_c,
                    abi: AbiSignature::default(),
                },
            );
            let b0 = FuncBuilder::ENTRY;
            let b1 = cb.block();
            let b_norm = cb.block();
            let b_unw = cb.block();
            let pure_val = cb.op(b0, OpKind::Const(ConstTree::Unit), pure_fndef, O);
            let _reified = cb.op(
                b0,
                OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v: pure_val,
                    from: pure_fndef,
                    to: fn_ptr,
                },
                fn_ptr,
                O,
            );
            let ext_ptr = cb.op(
                b0,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: get_cb,
                        sysroot: false,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                fn_ptr,
                O,
            );
            cb.terminate(b0, RTerm::Goto(b1, Vec::new()));
            cb.effect(
                b1,
                OpKind::Call {
                    callee: Callee::Indirect { signature: fn_ptr },
                    function: Some(ext_ptr),
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                O,
            );
            cb.terminate(
                b1,
                RTerm::Invoke {
                    normal: (b_norm, Vec::new()),
                    unwind: (b_unw, Vec::new()),
                },
            );
            cb.terminate(b_norm, RTerm::Return(None));
            cb.effect(
                b_unw,
                OpKind::Drop {
                    root: crate::jir::repr::Root::Value(pure_val),
                    path: Vec::new(),
                    glue: DropGlue::Direct {
                        item: item_id(102),
                        link_name: drop_str,
                    },
                },
                O,
            );
            cb.terminate(b_unw, RTerm::Throw(None));
            let caller = cb.finish();

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("pure_fn".to_owned()),
                test_item("pure_fn", pure_interns),
            );
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("pure_fn".to_owned()), Ok(pure_fn));
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(
                stats.modified, 0,
                "external fn ptr from non-sysroot runtime must block indirect prune"
            );
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &[InvokePrunedSite {
                        block: b1,
                        reason: InvokePruneReason::ProtectedOpCannotThrow,
                    }],
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "witness must reject indirect prune when external runtime fn returns fn ptr"
            );
        }
    }

    #[test]
    fn preserves_throwing_intrinsics_marker_direct_and_allows_swallowing_callee() {
        use crate::jir::semantic::ConstTree;

        // 1) `catch_unwind` (fallback: None), `const_eval_select` (throwing runtime arm), and
        //    marker-named `Callee::Direct` (host: None) are NOT pruned and rejected by witness.
        {
            let mut throw_interns = Interns::new();
            let unit_t = unit_types(&mut throw_interns);
            let host_throw = throw_interns.intern_str("host_throw");
            let mut tb = FuncBuilder::new(
                item_id(160),
                Signature {
                    arguments: Vec::new(),
                    result: unit_t,
                    abi: AbiSignature::default(),
                },
            );
            tb.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: host_throw,
                        sysroot: false,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                O,
            );
            tb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            let throw_fn = tb.finish();

            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let throw_fndef = TypeId(1);
            let throw_str = caller_interns.intern_str("throw_fn");
            caller_interns.intern_type(TypeDescriptor {
                id: throw_fndef,
                name: None,
                kind: TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 160,
                    },
                    sig: crate::jir::interns::SigId(0),
                    instance: Some(crate::jir::semantic::DirectItemRef {
                        item: item_id(160),
                        link_name: throw_str,
                    }),
                },
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
                    peeled: throw_fndef,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            let catch_name = caller_interns.intern_str("catch_unwind");
            let ces_name = caller_interns.intern_str("const_eval_select");
            let marker_name = caller_interns.intern_str("rcgjs.host.invoke");
            let drop_str = caller_interns.intern_str("real_drop");

            let mut cb = FuncBuilder::new(
                item_id(161),
                Signature {
                    arguments: Vec::new(),
                    result: unit_c,
                    abi: AbiSignature::default(),
                },
            );
            let b0 = FuncBuilder::ENTRY;
            let b1 = cb.block();
            let b2 = cb.block();
            let b_done = cb.block();
            let b_unw = cb.block();
            let unit_val = cb.op(b0, OpKind::Const(ConstTree::Unit), unit_c, O);
            let fn_val = cb.op(b0, OpKind::Const(ConstTree::Unit), throw_fndef, O);
            cb.effect(
                b0,
                OpKind::Intrinsic {
                    name: catch_name,
                    types: Vec::new(),
                    fallback: None,
                    args: vec![unit_val, unit_val],
                    track_caller: None,
                },
                O,
            );
            cb.terminate(
                b0,
                RTerm::Invoke {
                    normal: (b1, Vec::new()),
                    unwind: (b_unw, Vec::new()),
                },
            );
            cb.effect(
                b1,
                OpKind::Intrinsic {
                    name: ces_name,
                    types: Vec::new(),
                    fallback: None,
                    args: vec![unit_val, fn_val, fn_val],
                    track_caller: None,
                },
                O,
            );
            cb.terminate(
                b1,
                RTerm::Invoke {
                    normal: (b2, Vec::new()),
                    unwind: (b_unw, Vec::new()),
                },
            );
            cb.effect(
                b2,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item_id(162),
                        link_name: marker_name,
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
                b2,
                RTerm::Invoke {
                    normal: (b_done, Vec::new()),
                    unwind: (b_unw, Vec::new()),
                },
            );
            cb.terminate(b_done, RTerm::Return(None));
            cb.effect(
                b_unw,
                OpKind::Drop {
                    root: crate::jir::repr::Root::Value(unit_val),
                    path: Vec::new(),
                    glue: DropGlue::Direct {
                        item: item_id(102),
                        link_name: drop_str,
                    },
                },
                O,
            );
            cb.terminate(b_unw, RTerm::Throw(None));
            let caller = cb.finish();

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("throw_fn".to_owned()),
                test_item("throw_fn", throw_interns),
            );
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("throw_fn".to_owned()), Ok(throw_fn));
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(stats.modified, 0);
            for bb in [b0, b1, b2] {
                assert!(
                    witness::check_invoke_pruning(
                        &caller,
                        &caller_interns_ref,
                        &[InvokePrunedSite {
                            block: bb,
                            reason: InvokePruneReason::ProtectedOpCannotThrow,
                        }],
                        &table,
                        &funcs,
                        &live,
                    )
                    .is_err(),
                    "witness must reject throwing intrinsic/marker at bb{}",
                    bb.0
                );
            }
        }

        // 2) A callee that catches and swallows a throwing `Host` op on its own `Invoke` unwind
        //    edge (returning `Return(None)` without `Throw`) MUST pass `witness::check_invoke_pruning`
        //    with `witnesses_recorded == 1, witnesses_failed == 0`.
        {
            let mut sw_interns = Interns::new();
            let unit_sw = unit_types(&mut sw_interns);
            let sec_id = sw_interns.intern_str("rcgjs.host.invoke");
            let mut sb = FuncBuilder::new(
                item_id(170),
                Signature {
                    arguments: Vec::new(),
                    result: unit_sw,
                    abi: AbiSignature::default(),
                },
            );
            let s0 = FuncBuilder::ENTRY;
            let s_norm = sb.block();
            let s_unw = sb.block();
            sb.effect(
                s0,
                OpKind::Host {
                    payload: HostPayload::Marker {
                        section: sec_id,
                        def: DefinitionId {
                            crate_id: 0,
                            local_hash: 170,
                        },
                        helpers: Vec::new(),
                    },
                    args: Vec::new(),
                },
                O,
            );
            sb.terminate(
                s0,
                RTerm::Invoke {
                    normal: (s_norm, Vec::new()),
                    unwind: (s_unw, Vec::new()),
                },
            );
            sb.terminate(s_norm, RTerm::Return(None));
            sb.terminate(s_unw, RTerm::Return(None));
            let swallower = sb.finish();

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let sw_str = caller_interns.intern_str("swallower");
            let mut cb = FuncBuilder::new(
                item_id(171),
                Signature {
                    arguments: Vec::new(),
                    result: unit_c,
                    abi: AbiSignature::default(),
                },
            );
            let c0 = FuncBuilder::ENTRY;
            let c_norm = cb.block();
            let c_unw = cb.block();
            cb.effect(
                c0,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item_id(170),
                        link_name: sw_str,
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
                    normal: (c_norm, Vec::new()),
                    unwind: (c_unw, Vec::new()),
                },
            );
            cb.terminate(c_norm, RTerm::Return(None));
            cb.terminate(c_unw, RTerm::Throw(None));
            let caller = cb.finish();

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("swallower".to_owned()),
                test_item("swallower", sw_interns),
            );
            table.insert(
                JsName::new("caller".to_owned()),
                test_item("caller", caller_interns),
            );
            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("swallower".to_owned()), Ok(swallower));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(stats.modified, 1);
            assert_eq!(stats.witnesses_recorded, 1);
            assert_eq!(stats.witnesses_failed, 0);
            assert!(!funcs[&JsName::new("caller".to_owned())]
                .as_ref()
                .unwrap()
                .has_invoke());
        }
    }

    #[test]
    fn alloc_shim_invoke_with_non_noop_drop_prunes_default_and_preserves_custom_and_conflict() {
        let rust_alloc_name = "_RNvCsxyz_7___rustc12___rust_alloc";
        for direct_to_alloc in [false, true] {
            for provider in [
                AllocatorProvider::Default,
                AllocatorProvider::Custom,
                AllocatorProvider::Conflict,
            ] {
                let mut drop_interns = Interns::new();
                let unit_d = unit_types(&mut drop_interns);
                let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

                let mut leaf_interns = Interns::new();
                let unit_leaf = unit_types(&mut leaf_interns);
                let rust_alloc_leaf_sym = leaf_interns.intern_str(rust_alloc_name);
                let mut lb = FuncBuilder::new(
                    item_id(180),
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
                                local_hash: 181,
                            },
                            symbol: rust_alloc_leaf_sym,
                        },
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

                let mut caller_interns = Interns::new();
                let unit_c = unit_types(&mut caller_interns);
                let callee = if direct_to_alloc {
                    let alloc_sym = caller_interns.intern_str(rust_alloc_name);
                    Callee::Foreign {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 181,
                        },
                        symbol: alloc_sym,
                    }
                } else {
                    let wrapper_sym = caller_interns.intern_str("non_throwing_alloc_fn");
                    Callee::Direct {
                        item: item_id(180),
                        link_name: wrapper_sym,
                    }
                };
                let caller = build_caller_with_cleanup_drop(
                    &mut caller_interns,
                    unit_c,
                    callee,
                    "real_drop",
                );

                let mut alloc_shim_item = test_item(rust_alloc_name, Interns::new());
                alloc_shim_item.kind = ItemKind::Alloc;
                alloc_shim_item.allocator_provider = provider;

                let mut table = BTreeMap::new();
                table.insert(JsName::new(rust_alloc_name.to_owned()), alloc_shim_item);
                table.insert(
                    JsName::new("real_drop".to_owned()),
                    test_item("real_drop", drop_interns),
                );
                if !direct_to_alloc {
                    table.insert(
                        JsName::new("non_throwing_alloc_fn".to_owned()),
                        test_item("non_throwing_alloc_fn", leaf_interns),
                    );
                }
                let caller_item = test_item("caller", caller_interns);
                let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
                table.insert(JsName::new("caller".to_owned()), caller_item);

                let mut funcs = BTreeMap::new();
                funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
                if !direct_to_alloc {
                    funcs.insert(
                        JsName::new("non_throwing_alloc_fn".to_owned()),
                        Ok(non_throwing_alloc_fn),
                    );
                }
                funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

                let live: BTreeSet<JsName> = table.keys().cloned().collect();
                let stats = run_checked(&table, &mut funcs, &live);
                let updated = funcs[&JsName::new("caller".to_owned())].as_ref().unwrap();
                if provider == AllocatorProvider::Default {
                    assert_eq!(
                        stats.modified, 1,
                        "default allocator alloc invoke must prune (direct={direct_to_alloc})"
                    );
                    assert_eq!(stats.witnesses_recorded, 1);
                    assert_eq!(stats.witnesses_failed, 0);
                    assert!(
                        !updated.has_invoke(),
                        "default allocator caller must have zero remaining Invoke (direct={direct_to_alloc})"
                    );
                } else {
                    assert_eq!(
                        stats.modified, 0,
                        "custom/conflict allocator alloc invoke must NOT prune ({provider:?}, direct={direct_to_alloc})"
                    );
                    assert!(updated.has_invoke());
                    let forged = [InvokePrunedSite {
                        block: FuncBuilder::ENTRY,
                        reason: InvokePruneReason::ProtectedOpCannotThrow,
                    }];
                    assert!(
                        witness::check_invoke_pruning(
                            &caller,
                            &caller_interns_ref,
                            &forged,
                            &table,
                            &funcs,
                            &live,
                        )
                        .is_err(),
                        "forged ProtectedOpCannotThrow with {provider:?} must fail witness check"
                    );
                }
            }
        }
    }

    fn semantic_body(item: ItemId, unit: TypeId, semantic: BodySemantic) -> crate::jir::Body {
        crate::jir::Body {
            item,
            kind: BodyKind::Function,
            semantic,
            signature: Signature {
                arguments: Vec::new(),
                result: unit,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        }
    }

    #[test]
    fn prunes_invoke_calling_semantic_panic_impl_even_when_it_throws_host_value() {
        // Real `#[panic_handler]` tagged with `BodySemantic::PanicImpl` calling a host throw:
        // emission wraps it in `try { ... } catch ($e) { throw __rt.abort_error($e); }`, so both
        // `prune_invoke` and the independent witness treat callers as non-host-throwing.
        let mut panic_interns = Interns::new();
        let unit_p = unit_types(&mut panic_interns);
        let host_throw = panic_interns.intern_str("rcgjs.host.throw");
        let mut pb = FuncBuilder::new(
            item_id(200),
            Signature {
                arguments: Vec::new(),
                result: unit_p,
                abi: AbiSignature::default(),
            },
        );
        pb.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(201),
                    link_name: host_throw,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Host,
                host: Some(HostPayload::Marker {
                    section: host_throw,
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 201,
                    },
                    helpers: Vec::new(),
                }),
            },
            O,
        );
        pb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let panic_fn = pb.finish();

        let mut drop_interns = Interns::new();
        let unit_d = unit_types(&mut drop_interns);
        let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

        let mut caller_interns = Interns::new();
        let unit_c = unit_types(&mut caller_interns);
        let panic_sym = caller_interns.intern_str("custom_panic_handler_symbol");
        let caller = build_caller_with_cleanup_drop(
            &mut caller_interns,
            unit_c,
            Callee::Foreign {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 200,
                },
                symbol: panic_sym,
            },
            "real_drop",
        );

        let mut panic_item = test_item("custom_panic_handler_symbol", panic_interns);
        panic_item.jir.body = Some(semantic_body(item_id(200), unit_p, BodySemantic::PanicImpl));

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("custom_panic_handler_symbol".to_owned()),
            panic_item,
        );
        table.insert(
            JsName::new("real_drop".to_owned()),
            test_item("real_drop", drop_interns),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", caller_interns),
        );

        let mut funcs = BTreeMap::new();
        funcs.insert(
            JsName::new("custom_panic_handler_symbol".to_owned()),
            Ok(panic_fn),
        );
        funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 1);
        assert_eq!(stats.witnesses_recorded, 1);
        assert_eq!(stats.witnesses_failed, 0);
        assert!(!funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn prunes_map_markers_and_abort_host_op_and_rejects_unknown_map_marker() {
        for (marker, should_prune) in [
            ("new", true),
            ("len", true),
            ("clear", true),
            ("has", true),
            ("set", true),
            ("get", true),
            ("getmut", true),
            ("del", true),
            ("keys", true),
            ("evil_unknown", false),
        ] {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let marker_str = caller_interns.intern_str(marker);
            let drop_str = caller_interns.intern_str("real_drop");
            let mut cb = FuncBuilder::new(
                item_id(210),
                Signature {
                    arguments: Vec::new(),
                    result: unit_c,
                    abi: AbiSignature::default(),
                },
            );
            let entry = FuncBuilder::ENTRY;
            let normal = cb.block();
            let unwind = cb.block();
            cb.effect(
                entry,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item_id(211),
                        link_name: marker_str,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Host,
                    host: Some(HostPayload::Map { marker: marker_str }),
                },
                O,
            );
            cb.terminate(
                entry,
                RTerm::Invoke {
                    normal: (normal, Vec::new()),
                    unwind: (unwind, Vec::new()),
                },
            );
            cb.terminate(normal, RTerm::Return(None));
            let val = cb.op(
                unwind,
                OpKind::Const(crate::jir::semantic::ConstTree::Aggregate(Vec::new())),
                unit_c,
                O,
            );
            cb.effect(
                unwind,
                OpKind::Drop {
                    root: crate::jir::repr::Root::Value(val),
                    path: Vec::new(),
                    glue: DropGlue::Direct {
                        item: item_id(212),
                        link_name: drop_str,
                    },
                },
                O,
            );
            cb.terminate(unwind, RTerm::Throw(None));
            let caller = cb.finish();

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            let updated = funcs[&JsName::new("caller".to_owned())].as_ref().unwrap();
            if should_prune {
                assert_eq!(stats.modified, 1, "map marker `{marker}` must prune");
                assert_eq!(stats.witnesses_failed, 0);
                assert!(!updated.has_invoke());
            } else {
                assert_eq!(
                    stats.modified, 0,
                    "unknown map marker `{marker}` must NOT prune"
                );
                assert!(updated.has_invoke());
                let forged = [InvokePrunedSite {
                    block: FuncBuilder::ENTRY,
                    reason: InvokePruneReason::ProtectedOpCannotThrow,
                }];
                assert!(witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &forged,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err());
            }
        }
    }

    #[test]
    fn audited_sysroot_bstr_and_ptr_helpers_prune_while_throwing_bstr_helpers_stay_preserved() {
        let safe_helpers = [
            "bstr_bytes",
            "bstr_of",
            "bstr_in",
            "bstr_out",
            "bstr_boundary",
            "bstr_get",
            "bstr_get_from",
            "bstr_get_to",
            "bytes_is_ascii",
            "bstr_trim",
            "bstr_trim_start",
            "bstr_trim_end",
            "unscale_scalar",
            "unscale_uninit",
            "compare_bytes",
            "tail_slice",
            "unwindow",
            "unwindow_slice",
            "array_ref",
            "map_new",
            "map_has",
            "map_get",
            "map_set",
            "map_del",
            "map_len",
            "map_clear",
            "map_keys",
            "map_values",
        ];
        let unsafe_helpers = [
            "bstr_utf8_bytes",
            "bstr_str_in",
            "bstr_ref",
            "bstr_ref_in",
            "bstr_sub",
            "bstr_sub_from",
            "bstr_sub_to",
            "u8array_length",
            "u8array_into_bytes",
        ];
        for (sym, should_prune) in safe_helpers
            .iter()
            .map(|&s| (s, true))
            .chain(unsafe_helpers.iter().map(|&s| (s, false)))
        {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let sym_id = caller_interns.intern_str(sym);
            let caller = build_caller_with_cleanup_drop(
                &mut caller_interns,
                unit_c,
                Callee::Runtime {
                    member: sym_id,
                    sysroot: true,
                },
                "real_drop",
            );

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", caller_interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            let updated = funcs[&JsName::new("caller".to_owned())].as_ref().unwrap();
            if should_prune {
                assert_eq!(
                    stats.modified, 1,
                    "audited sysroot helper `{sym}` must prune"
                );
                assert_eq!(stats.witnesses_failed, 0);
                assert!(!updated.has_invoke());
            } else {
                assert_eq!(
                    stats.modified, 0,
                    "throwing/callback sysroot helper `{sym}` must NOT prune"
                );
                assert!(updated.has_invoke());
                let forged = [InvokePrunedSite {
                    block: FuncBuilder::ENTRY,
                    reason: InvokePruneReason::ProtectedOpCannotThrow,
                }];
                assert!(witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &forged,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err());
            }
        }
    }

    #[test]
    fn prunes_branching_cleanup_dag_with_switch_cell_ops_and_diamond_merge() {
        use crate::jir::semantic::ConstTree;
        use crate::jir::types::{IntFacts, IntRepr};

        let sig = |result| Signature {
            arguments: Vec::new(),
            result,
            abi: AbiSignature::default(),
        };
        let mut noop_b = FuncBuilder::new(item_id(20), sig(UNIT));
        noop_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let noop_fn = noop_b.finish();

        let mut interns = Interns::new();
        let unit_ty = unit_types(&mut interns);
        let bool_ty = TypeId(1);
        let u8_ty = TypeId(2);
        let u32_ty = TypeId(3);
        let ref_unit_ty = TypeId(4);
        interns.intern_type(TypeDescriptor {
            id: bool_ty,
            name: None,
            kind: TypeKind::Bool,
            layout: LayoutFacts {
                size: Some(1),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: bool_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        interns.intern_type(TypeDescriptor {
            id: u8_ty,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            layout: LayoutFacts {
                size: Some(1),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 8,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: u8_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        interns.intern_type(TypeDescriptor {
            id: u32_ty,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: u32_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        interns.intern_type(TypeDescriptor {
            id: ref_unit_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: unit_ty,
                mutable: false,
            },
            layout: LayoutFacts {
                size: Some(4),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: ref_unit_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let noop_str = interns.intern_str("noop_drop");
        let throwing_host = HostPayload::Marker {
            section: interns.intern_str("rcgjs.host.invoke"),
            def: DefinitionId {
                crate_id: 0,
                local_hash: 51,
            },
            helpers: Vec::new(),
        };

        let mut cb = FuncBuilder::new(item_id(21), sig(unit_ty));
        let flag_cell = cb.cell(bool_ty, false, O);
        let unit_val = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Unit),
            unit_ty,
            O,
        );
        let init_flag = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            bool_ty,
            O,
        );
        cb.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(flag_cell), O);
        cb.effect(
            FuncBuilder::ENTRY,
            OpKind::CellStore {
                cell: flag_cell,
                v: init_flag,
            },
            O,
        );
        cb.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: throwing_host,
                args: Vec::new(),
            },
            O,
        );
        let b_done = cb.block();
        let u_entry = cb.block();
        let u_left = cb.block();
        let u_right = cb.block();
        let u_merge = cb.block();
        cb.terminate(
            FuncBuilder::ENTRY,
            RTerm::Invoke {
                normal: (b_done, Vec::new()),
                unwind: (u_entry, Vec::new()),
            },
        );
        cb.terminate(b_done, RTerm::Return(None));

        // Cleanup DAG entry: reads & clears unescaped drop-flag cell, runs pure flag ops,
        // BorrowView/Field/Discriminant, and switches to two arms that merge before Throw(None).
        let loaded = cb.op(u_entry, OpKind::CellLoad(flag_cell), bool_ty, O);
        let zero_bool = cb.op(
            u_entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
            bool_ty,
            O,
        );
        cb.effect(
            u_entry,
            OpKind::CellStore {
                cell: flag_cell,
                v: zero_bool,
            },
            O,
        );
        let inverted = cb.op(
            u_entry,
            OpKind::Unary {
                op: UnaryOp::Not,
                v: loaded,
                ty: bool_ty,
            },
            bool_ty,
            O,
        );
        let widened = cb.op(
            u_entry,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: inverted,
                from: bool_ty,
                to: u8_ty,
            },
            u8_ty,
            O,
        );
        let mask = cb.op(
            u_entry,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            u8_ty,
            O,
        );
        let masked = cb.op(
            u_entry,
            OpKind::Binary {
                op: BinaryOp::BitAnd,
                l: widened,
                r: mask,
                ty: u8_ty,
            },
            u8_ty,
            O,
        );
        let _view = cb.op(
            u_entry,
            OpKind::BorrowView {
                root: Root::Value(unit_val),
                path: Vec::new(),
                mutable: false,
            },
            ref_unit_ty,
            O,
        );
        let _field = cb.op(
            u_entry,
            OpKind::Field {
                base: unit_val,
                variant: None,
                field: 0,
            },
            unit_ty,
            O,
        );
        let _discr = cb.op(u_entry, OpKind::Discriminant(unit_val), u32_ty, O);
        cb.terminate(
            u_entry,
            RTerm::Switch {
                scrut: masked,
                scrut_ty: u8_ty,
                cases: vec![(1, u_left, Vec::new())],
                default: (u_right, Vec::new()),
            },
        );

        // Left arm: DropGlue::None + Callee::Nop -> Goto(u_merge).
        cb.effect(
            u_left,
            OpKind::Drop {
                root: Root::Value(unit_val),
                path: Vec::new(),
                glue: DropGlue::None,
            },
            O,
        );
        cb.effect(
            u_left,
            OpKind::Call {
                callee: Callee::Nop,
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        cb.terminate(u_left, RTerm::Goto(u_merge, Vec::new()));

        // Right arm: direct noop drop -> Goto(u_merge).
        cb.effect(
            u_right,
            OpKind::Drop {
                root: Root::Value(unit_val),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item_id(20),
                    link_name: noop_str,
                },
            },
            O,
        );
        cb.terminate(u_right, RTerm::Goto(u_merge, Vec::new()));

        // Diamond merge block -> Throw(None).
        cb.terminate(u_merge, RTerm::Throw(None));
        let caller = cb.finish();

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("noop_drop".to_owned()),
            test_item("noop_drop", Interns::new()),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            test_item("caller", interns),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("noop_drop".to_owned()), Ok(noop_fn));
        funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        let stats = run_checked(&table, &mut funcs, &live);
        assert_eq!(stats.modified, 1, "branching pure cleanup DAG must prune");
        assert_eq!(stats.witnesses_failed, 0, "witness must accept pruned DAG");
        assert!(!funcs[&JsName::new("caller".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());
    }

    #[test]
    fn preserves_cleanup_dag_with_cycle_escaped_cell_store_non_noop_drop_return_or_div() {
        use crate::jir::semantic::ConstTree;
        use crate::jir::types::{IntFacts, IntRepr};

        #[derive(Clone, Copy, Debug)]
        enum BadCase {
            Cycle,
            EscapedCellStore,
            NonNoopDropInArm,
            ReturnInArm,
            DivOp,
            ThrowValue,
        }

        for case in [
            BadCase::Cycle,
            BadCase::EscapedCellStore,
            BadCase::NonNoopDropInArm,
            BadCase::ReturnInArm,
            BadCase::DivOp,
            BadCase::ThrowValue,
        ] {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut interns = Interns::new();
            let unit_ty = unit_types(&mut interns);
            let u32_ty = TypeId(1);
            let ref_u32_ty = TypeId(2);
            interns.intern_type(TypeDescriptor {
                id: u32_ty,
                name: None,
                kind: TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed: false,
                        bits: 32,
                        repr: IntRepr::Number,
                    }),
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: u32_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            interns.intern_type(TypeDescriptor {
                id: ref_u32_ty,
                name: None,
                kind: TypeKind::Reference {
                    pointee: u32_ty,
                    mutable: true,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: ref_u32_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });

            let real_drop_str = interns.intern_str("real_drop");
            let throwing_host = HostPayload::Marker {
                section: interns.intern_str("rcgjs.host.invoke"),
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 52,
                },
                helpers: Vec::new(),
            };

            let mut cb = FuncBuilder::new(
                item_id(22),
                Signature {
                    arguments: Vec::new(),
                    result: unit_ty,
                    abi: AbiSignature::default(),
                },
            );
            let unit_val = cb.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Unit),
                unit_ty,
                O,
            );
            let one = cb.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
                u32_ty,
                O,
            );
            let escaped_cell = cb.cell(u32_ty, true, O);
            if matches!(case, BadCase::EscapedCellStore) {
                cb.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(escaped_cell), O);
                let _escaped_ref = cb.op(
                    FuncBuilder::ENTRY,
                    OpKind::BorrowView {
                        root: Root::Cell(escaped_cell),
                        path: Vec::new(),
                        mutable: true,
                    },
                    ref_u32_ty,
                    O,
                );
            }
            cb.effect(
                FuncBuilder::ENTRY,
                OpKind::Host {
                    payload: throwing_host,
                    args: Vec::new(),
                },
                O,
            );
            let b_done = cb.block();
            let u0 = cb.block();
            let u1 = cb.block();
            let u2 = cb.block();
            cb.terminate(
                FuncBuilder::ENTRY,
                RTerm::Invoke {
                    normal: (b_done, Vec::new()),
                    unwind: (u0, Vec::new()),
                },
            );
            cb.terminate(b_done, RTerm::Return(None));

            match case {
                BadCase::Cycle => {
                    cb.terminate(
                        u0,
                        RTerm::Switch {
                            scrut: one,
                            scrut_ty: u32_ty,
                            cases: vec![(1, u1, Vec::new())],
                            default: (u2, Vec::new()),
                        },
                    );
                    cb.terminate(u1, RTerm::Goto(u0, Vec::new()));
                    cb.terminate(u2, RTerm::Throw(None));
                }
                BadCase::EscapedCellStore => {
                    cb.effect(
                        u0,
                        OpKind::CellStore {
                            cell: escaped_cell,
                            v: one,
                        },
                        O,
                    );
                    cb.terminate(u0, RTerm::Throw(None));
                    cb.terminate(u1, RTerm::Throw(None));
                    cb.terminate(u2, RTerm::Throw(None));
                }
                BadCase::NonNoopDropInArm => {
                    cb.terminate(
                        u0,
                        RTerm::Switch {
                            scrut: one,
                            scrut_ty: u32_ty,
                            cases: vec![(1, u1, Vec::new())],
                            default: (u2, Vec::new()),
                        },
                    );
                    cb.effect(
                        u1,
                        OpKind::Drop {
                            root: Root::Value(unit_val),
                            path: Vec::new(),
                            glue: DropGlue::Direct {
                                item: item_id(99),
                                link_name: real_drop_str,
                            },
                        },
                        O,
                    );
                    cb.terminate(u1, RTerm::Throw(None));
                    cb.terminate(u2, RTerm::Throw(None));
                }
                BadCase::ReturnInArm => {
                    cb.terminate(
                        u0,
                        RTerm::Switch {
                            scrut: one,
                            scrut_ty: u32_ty,
                            cases: vec![(1, u1, Vec::new())],
                            default: (u2, Vec::new()),
                        },
                    );
                    cb.terminate(u1, RTerm::Return(None));
                    cb.terminate(u2, RTerm::Throw(None));
                }
                BadCase::DivOp => {
                    let _q = cb.op(
                        u0,
                        OpKind::Binary {
                            op: BinaryOp::Div,
                            l: one,
                            r: one,
                            ty: u32_ty,
                        },
                        u32_ty,
                        O,
                    );
                    cb.terminate(u0, RTerm::Throw(None));
                    cb.terminate(u1, RTerm::Throw(None));
                    cb.terminate(u2, RTerm::Throw(None));
                }
                BadCase::ThrowValue => {
                    cb.terminate(u0, RTerm::Throw(Some(one)));
                    cb.terminate(u1, RTerm::Throw(None));
                    cb.terminate(u2, RTerm::Throw(None));
                }
            }

            let caller = cb.finish();
            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            let caller_item = test_item("caller", interns);
            let caller_interns_ref = caller_item.jir.interns().unwrap().clone();
            table.insert(JsName::new("caller".to_owned()), caller_item);

            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller.clone()));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(
                stats.modified, 0,
                "unsafe cleanup DAG {case:?} must NOT prune"
            );
            assert!(funcs[&JsName::new("caller".to_owned())]
                .as_ref()
                .unwrap()
                .has_invoke());

            let forged = [InvokePrunedSite {
                block: FuncBuilder::ENTRY,
                reason: InvokePruneReason::NoopUnwindChain,
            }];
            assert!(
                witness::check_invoke_pruning(
                    &caller,
                    &caller_interns_ref,
                    &forged,
                    &table,
                    &funcs,
                    &live,
                )
                .is_err(),
                "witness must reject forged NoopUnwindChain for {case:?}"
            );
        }
    }

    #[test]
    fn monomorphic_typed_array_nv_helpers_prune_with_witness_parity() {
        let nv_helpers = [
            "nv_grow_u8",
            "nv_grow_i8",
            "nv_grow_u16",
            "nv_grow_i16",
            "nv_grow_u32",
            "nv_grow_i32",
            "nv_grow_f32",
            "nv_grow_f64",
            "nv_with_cap_u8",
            "nv_with_cap_i8",
            "nv_with_cap_u16",
            "nv_with_cap_i16",
            "nv_with_cap_u32",
            "nv_with_cap_i32",
            "nv_with_cap_f32",
            "nv_with_cap_f64",
            "nv_zeroed_u8",
            "nv_zeroed_i8",
            "nv_zeroed_u16",
            "nv_zeroed_i16",
            "nv_zeroed_u32",
            "nv_zeroed_i32",
            "nv_zeroed_f32",
            "nv_zeroed_f64",
            "nv_resize_zero_ta",
        ];
        for sym in nv_helpers {
            let mut drop_interns = Interns::new();
            let unit_d = unit_types(&mut drop_interns);
            let real_drop = build_non_empty_drop(&mut drop_interns, unit_d);

            let mut caller_interns = Interns::new();
            let unit_c = unit_types(&mut caller_interns);
            let sym_id = caller_interns.intern_str(sym);
            let caller = build_caller_with_cleanup_drop(
                &mut caller_interns,
                unit_c,
                Callee::Runtime {
                    member: sym_id,
                    sysroot: true,
                },
                "real_drop",
            );

            let mut table = BTreeMap::new();
            table.insert(
                JsName::new("real_drop".to_owned()),
                test_item("real_drop", drop_interns),
            );
            table.insert(
                JsName::new("caller".to_owned()),
                test_item("caller", caller_interns),
            );
            let mut funcs = BTreeMap::new();
            funcs.insert(JsName::new("real_drop".to_owned()), Ok(real_drop));
            funcs.insert(JsName::new("caller".to_owned()), Ok(caller));

            let live: BTreeSet<JsName> = table.keys().cloned().collect();
            let stats = run_checked(&table, &mut funcs, &live);
            assert_eq!(
                stats.modified, 1,
                "monomorphic TypedArray helper `{sym}` must prune"
            );
            assert_eq!(
                stats.witnesses_failed, 0,
                "witness must accept monomorphic TypedArray helper `{sym}`"
            );
        }
    }
}
