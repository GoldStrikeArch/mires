//! Interprocedural ASCII provenance analysis over optimized JIR functions (`js-str=bytes`).
//!
//! Computes a sound 2-phase fixpoint (greatest fixpoint for value flow, intersected with a least
//! fixpoint from constructive ASCII roots so ungrounded dead/diverging cycles cannot claim
//! `KnownAscii`). Anything not constructively proven to hold pure ASCII (`b < 0x80` on every
//! byte) stays `MaybeNonAscii`.
//!
//! Also locates the live, plain-ABI 4-parameter `core::str::slice_error_fail` item when present so
//! outlined `StrIndex` (`__rt.bstr_sub*`) can bypass `core::str::traits::<impl Index>::index`.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::refs::{self, RefScan};
use crate::item::{JsName, LinkItem};
use crate::jir::repr::{Carrier, CellId, Func, Op, OpKind, Proj, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, HostPayload, ItemId,
    StandardOperation, TypeId,
};
use crate::jir::types::{RepresentationCandidate, TypeKind, TypeTable, VariantDescriptor};
use crate::jir::{Interns, StrId};
use crate::plan::abi_flat::AbiPlan;
use crate::plan::representation::ReprOutcome;

/// Whole-program ASCII facts consumed while lowering JIR functions under `js-str=bytes`.
#[derive(Clone, Debug, Default)]
pub struct AsciiFacts {
    /// Per-function set of SSA `ValueId`s proven to contain only ASCII bytes (`&str` or
    /// `Carrier::HostString`). Anything absent is `MaybeNonAscii`.
    pub funcs: BTreeMap<JsName, BTreeSet<ValueId>>,
    /// Live `core::str::slice_error_fail` JS name when lowered with exact 4-arg `#[track_caller]`
    /// signature `(s, start, end, $loc)`.
    pub slice_error_fail: Option<JsName>,
    /// Authenticated `core::slice::<impl [T]>::reverse` item IDs.
    pub slice_reverse: BTreeSet<ItemId>,
    /// Authenticated `core::slice::<impl [T]>::reverse::revswap` item IDs.
    pub slice_revswap: BTreeSet<ItemId>,
    /// Authenticated `core::slice::<impl [u8]>::starts_with` item IDs (including `bstr_abi` clones).
    pub slice_starts_with: BTreeSet<ItemId>,
    /// Authenticated `core::slice::<impl [u8]>::ends_with` item IDs (including `bstr_abi` clones).
    pub slice_ends_with: BTreeSet<ItemId>,
}

/// Borrowed slice of [`AsciiFacts`] for one function being lowered.
#[derive(Clone, Copy, Debug, Default)]
pub struct FuncAsciiFacts<'a> {
    pub known: Option<&'a BTreeSet<ValueId>>,
    pub slice_error_fail: Option<&'a str>,
    pub slice_reverse: Option<&'a BTreeSet<ItemId>>,
    pub slice_revswap: Option<&'a BTreeSet<ItemId>>,
    pub slice_starts_with: Option<&'a BTreeSet<ItemId>>,
    pub slice_ends_with: Option<&'a BTreeSet<ItemId>>,
}

impl AsciiFacts {
    pub fn for_item(&self, name: &JsName) -> FuncAsciiFacts<'_> {
        FuncAsciiFacts {
            known: self.funcs.get(name),
            slice_error_fail: self.slice_error_fail.as_ref().map(|name| name.as_str()),
            slice_reverse: (!self.slice_reverse.is_empty()).then_some(&self.slice_reverse),
            slice_revswap: (!self.slice_revswap.is_empty()).then_some(&self.slice_revswap),
            slice_starts_with: (!self.slice_starts_with.is_empty())
                .then_some(&self.slice_starts_with),
            slice_ends_with: (!self.slice_ends_with.is_empty()).then_some(&self.slice_ends_with),
        }
    }
}

impl<'a> FuncAsciiFacts<'a> {
    pub fn is_known_ascii(&self, value: ValueId) -> bool {
        self.known.is_some_and(|set| set.contains(&value))
    }

    pub fn is_slice_reverse(&self, item: ItemId) -> bool {
        self.slice_reverse.is_some_and(|set| set.contains(&item))
    }

    pub fn is_slice_revswap(&self, item: ItemId) -> bool {
        self.slice_revswap.is_some_and(|set| set.contains(&item))
    }

    pub fn is_slice_starts_with(&self, item: ItemId) -> bool {
        self.slice_starts_with
            .is_some_and(|set| set.contains(&item))
    }

    pub fn is_slice_ends_with(&self, item: ItemId) -> bool {
        self.slice_ends_with.is_some_and(|set| set.contains(&item))
    }
}

/// What ASCII-trackable value shape an SSA value has in JIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AsciiType {
    /// Shared string slice `&str` (peeling transparent wrappers).
    SharedStr,
    /// Owned `String` lowered as `Carrier::HostString` (byte string holding UTF-8 code units).
    HostString,
    /// Owned `String` with `Carrier::Baseline` (`RepresentationCandidate::String`, including
    /// universal native `{ s, cap }` `String`). Tracked internally to propagate ASCII across
    /// `NativeStr*` / `String*` / `Option<String>` / `Result<String, E>`, never published into
    /// `AsciiFacts.funcs`.
    OwnedString,
    /// 2-variant enum (e.g. `Option<&str>`, `Option<String>`, `Result<String, E>`,
    /// `Result<String, &str>`) where each variant carries at most one live field and at least one
    /// variant carries a `SharedStr` or `OwnedString` field. Never published directly.
    EnumStr {
        /// `(field_index, is_owned_string)` for variant 0, if its live payload is a tracked string.
        v0: Option<(u32, bool)>,
        /// `(field_index, is_owned_string)` for variant 1, if its live payload is a tracked string.
        v1: Option<(u32, bool)>,
    },
    /// Any other type/carrier (`MaybeNonAscii` always).
    Other,
}

#[derive(Clone, Debug)]
enum Rule {
    Never,
    Root,
    Single(ValueId),
    /// Every listed dependency must be ASCII (Phase A) AND grounded (Phase B). Used for mutated
    /// local `String` cells / values that accumulate stores and pushed slices.
    All(Vec<ValueId>),
    BlockParam(Vec<ValueId>),
    FuncParam(Vec<(usize, ValueId)>),
    CallReturn(usize),
}

struct FuncState<'a> {
    name: JsName,
    func: &'a Func,
    val_types: Vec<AsciiType>,
    rules: Vec<Rule>,
    returns: Vec<ValueId>,
    is_open: bool,
    ascii: Vec<bool>,
    grounded: Vec<bool>,
    ret_ascii: bool,
    ret_grounded: bool,
}

/// Computes [`AsciiFacts`] across the program's post-optimization JIR functions.
pub fn compute(
    funcs: &BTreeMap<JsName, &Func>,
    table: &BTreeMap<JsName, LinkItem>,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    roots: &BTreeSet<JsName>,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> AsciiFacts {
    compute_with_interns(
        funcs,
        table,
        |name| table.get(name).and_then(|item| item.jir.interns()),
        repr,
        abi,
        roots,
        defined,
        live,
    )
}

#[allow(clippy::too_many_arguments)]
fn compute_with_interns<'a, F>(
    funcs: &BTreeMap<JsName, &'a Func>,
    table: &BTreeMap<JsName, LinkItem>,
    get_interns: F,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    roots: &BTreeSet<JsName>,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> AsciiFacts
where
    F: Fn(&JsName) -> Option<&'a Interns>,
{
    let mut name_to_idx: BTreeMap<JsName, usize> = BTreeMap::new();
    let mut names: Vec<JsName> = Vec::with_capacity(funcs.len());
    for name in funcs.keys() {
        name_to_idx.insert(name.clone(), names.len());
        names.push(name.clone());
    }

    // 1. Collect all non-direct references (`RefScan` + unlowered items + roots/exports).
    let mut scan = RefScan::default();
    let mut open_names: BTreeSet<JsName> = BTreeSet::new();

    for (name, func) in funcs {
        let types = get_interns(name).map(|i| &i.types);
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, types, &mut scan);
                if let OpKind::Host { payload, .. } = &op.kind {
                    for (_, helper) in payload.helpers() {
                        scan.named.insert(helper.item);
                    }
                    if let HostPayload::KeyedList(payload) = payload {
                        for callable in [&payload.collection, &payload.key, &payload.row] {
                            if let Some(body) = callable.body {
                                scan.named.insert(body.item);
                            }
                        }
                    }
                }
            }
        }
    }

    // Items outside `funcs` (statics, vtables, `NativeBody` overrides, declined/baseline items)
    // may call or take pointers to anything in their `refs`.
    for (name, item) in table {
        if item.linkage.exported
            || item.linkage.root
            || roots.contains(name)
            || !defined.contains(name)
            || !live.contains(name)
        {
            open_names.insert(name.clone());
        }
        if !funcs.contains_key(name) {
            open_names.insert(name.clone());
            for referenced in &item.refs {
                open_names.insert(referenced.clone());
            }
        }
    }

    let mut open_items = scan.ineligible();

    // Locate authentic `core::str::slice_error_fail` if live and keeping its standard 4-arg ABI,
    // plus authenticated `core::slice::{reverse,revswap,starts_with,ends_with}` items.
    let slice_error_fail =
        find_slice_error_fail(funcs, table, &get_interns, repr, abi, defined, live);
    if let Some(fail_name) = &slice_error_fail {
        // Outlined `StrIndex` jumps directly to `slice_error_fail` without modifying JIR calls.
        open_names.insert(fail_name.clone());
    }
    let core_slice_items =
        find_core_slice_items(funcs, table, &get_interns, repr, abi, defined, live);

    // 2. Build per-function value metadata, intra-procedural rules, and direct call edges.
    let mut incoming_calls: Vec<Vec<Vec<(usize, ValueId)>>> = funcs
        .values()
        .map(|f| vec![Vec::new(); f.params.len()])
        .collect();
    let mut force_open_idx = vec![false; funcs.len()];

    let mut states: Vec<FuncState<'_>> = Vec::with_capacity(funcs.len());
    for (caller_idx, name) in names.iter().enumerate() {
        let func = funcs[name];
        let Some(interns) = get_interns(name) else {
            return AsciiFacts {
                funcs: BTreeMap::new(),
                slice_error_fail,
                slice_reverse: core_slice_items.reverse,
                slice_revswap: core_slice_items.revswap,
                slice_starts_with: core_slice_items.starts_with,
                slice_ends_with: core_slice_items.ends_with,
            };
        };
        let type_table = &interns.types;
        let val_types: Vec<AsciiType> = func
            .values
            .iter()
            .map(|v| classify_value_type(type_table, interns, v.ty, v.carrier))
            .collect();
        let mut defs: Vec<Option<&Op>> = vec![None; func.values.len()];
        for block in &func.blocks {
            for op in &block.ops {
                if let Some(res) = op.result {
                    defs[res.0 as usize] = Some(op);
                }
            }
        }
        let owned_strings = analyze_owned_strings(func, &defs, &val_types, type_table);
        let disqualified = collect_disqualified(func, &owned_strings.accounted_mut_values);

        let mut rules = vec![Rule::Never; func.values.len()];

        // Incoming edges for every non-entry block parameter.
        let mut block_incoming: Vec<Vec<Vec<ValueId>>> = func
            .blocks
            .iter()
            .map(|b| vec![Vec::new(); b.params.len()])
            .collect();
        for block in &func.blocks {
            for (target, edge_args) in block.term.edges() {
                let target_idx = target.0 as usize;
                let Some(target_slots) = block_incoming.get_mut(target_idx) else {
                    continue;
                };
                if edge_args.len() != target_slots.len() {
                    for slots in target_slots.iter_mut() {
                        slots.clear();
                    }
                    continue;
                }
                for (slot, &arg) in target_slots.iter_mut().zip(edge_args.iter()) {
                    slot.push(arg);
                }
            }
        }

        // Entry block parameters match `func.params` (`FuncParam` rule assigned below).
        for (block_idx, block) in func.blocks.iter().enumerate() {
            if block_idx == 0 {
                continue;
            }
            for (param_idx, &val) in block.params.iter().enumerate() {
                if disqualified.contains(&val) || val_types[val.0 as usize] == AsciiType::Other {
                    continue;
                }
                let incoming = std::mem::take(&mut block_incoming[block_idx][param_idx]);
                if incoming.is_empty() {
                    continue;
                }
                let src_ty = val_types[val.0 as usize];
                if incoming
                    .iter()
                    .all(|&src| val_types[src.0 as usize] == src_ty)
                {
                    rules[val.0 as usize] = Rule::BlockParam(incoming);
                }
            }
        }

        // Ops and calls.
        let mut returns = Vec::new();
        for block in &func.blocks {
            for op in &block.ops {
                // Record direct call edges regardless of whether the result is trackable.
                if let OpKind::Call {
                    callee: Callee::Direct { item, link_name },
                    function: None,
                    args,
                    host: None,
                    ..
                } = &op.kind
                {
                    if let Some(target_name) =
                        resolve_callee(name, *item, *link_name, interns, repr, abi)
                    {
                        if let Some(&callee_idx) = name_to_idx.get(&target_name) {
                            let callee = funcs[&target_name];
                            let expected = args.len() + usize::from(callee.sig.abi.caller_location);
                            if callee.params.len() == expected {
                                for (i, &arg) in args.iter().enumerate() {
                                    incoming_calls[callee_idx][i].push((caller_idx, arg));
                                }
                            } else {
                                force_open_idx[callee_idx] = true;
                            }
                        }
                    } else {
                        open_items.insert(*item);
                    }
                }

                let Some(result) = op.result else { continue };
                let r_idx = result.0 as usize;
                let r_ty = val_types[r_idx];
                if disqualified.contains(&result) || r_ty == AsciiType::Other {
                    continue;
                }
                let mut rule = match &op.kind {
                    OpKind::Const(tree) => {
                        if is_ascii_const(type_table, interns, tree, r_ty) {
                            Rule::Root
                        } else {
                            Rule::Never
                        }
                    }
                    OpKind::Move(src) | OpKind::Copy(src) => {
                        if val_types[src.0 as usize] == r_ty {
                            Rule::Single(*src)
                        } else {
                            Rule::Never
                        }
                    }
                    OpKind::CellLoad(cell) if r_ty == AsciiType::OwnedString => owned_strings
                        .cell_deps
                        .get(cell)
                        .map(|deps| Rule::All(deps.clone()))
                        .unwrap_or(Rule::Never),
                    OpKind::BorrowView {
                        root: Root::Value(src),
                        path,
                        mutable: false,
                    } if r_ty == AsciiType::SharedStr
                        && val_types[src.0 as usize] == AsciiType::SharedStr
                        && matches!(path.as_slice(), [Proj::Deref]) =>
                    {
                        Rule::Single(*src)
                    }
                    OpKind::Aggregate {
                        variant,
                        active_field: None,
                        fields,
                        ..
                    } => match r_ty {
                        AsciiType::EnumStr { v0, v1 } => {
                            let spec = match *variant {
                                0 => v0,
                                1 => v1,
                                _ => {
                                    rules[r_idx] = Rule::Never;
                                    continue;
                                }
                            };
                            match spec {
                                None => Rule::Root,
                                Some((fidx, is_owned)) => {
                                    let expected_ty = if is_owned {
                                        AsciiType::OwnedString
                                    } else {
                                        AsciiType::SharedStr
                                    };
                                    if let Some(&fv) = fields.get(fidx as usize) {
                                        if val_types[fv.0 as usize] == expected_ty {
                                            Rule::Single(fv)
                                        } else {
                                            Rule::Never
                                        }
                                    } else {
                                        Rule::Never
                                    }
                                }
                            }
                        }
                        _ => Rule::Never,
                    },
                    OpKind::Field {
                        base,
                        variant: Some(variant),
                        field,
                    } if matches!(r_ty, AsciiType::SharedStr | AsciiType::OwnedString)
                        && !disqualified.contains(base) =>
                    {
                        match val_types[base.0 as usize] {
                            AsciiType::EnumStr { v0, v1 } => {
                                let spec = match *variant {
                                    0 => v0,
                                    1 => v1,
                                    _ => None,
                                };
                                match spec {
                                    Some((fidx, is_owned))
                                        if *field == fidx
                                            && r_ty
                                                == if is_owned {
                                                    AsciiType::OwnedString
                                                } else {
                                                    AsciiType::SharedStr
                                                } =>
                                    {
                                        Rule::Single(*base)
                                    }
                                    _ => Rule::Never,
                                }
                            }
                            _ => Rule::Never,
                        }
                    }
                    OpKind::Call {
                        callee,
                        function: None,
                        args,
                        semantic,
                        host: None,
                        ..
                    } => classify_call_result(
                        name,
                        func,
                        r_ty,
                        *semantic,
                        callee,
                        args,
                        &defs,
                        &disqualified,
                        &owned_strings.cell_deps,
                        &val_types,
                        interns,
                        repr,
                        abi,
                        &name_to_idx,
                    ),
                    _ => Rule::Never,
                };
                if r_ty == AsciiType::OwnedString {
                    if let Some(mut_deps) = owned_strings.val_mut_deps.get(&result) {
                        if !mut_deps.is_empty() {
                            rule = match rule {
                                Rule::Root => Rule::All(mut_deps.clone()),
                                Rule::Single(src) => {
                                    let mut all = Vec::with_capacity(1 + mut_deps.len());
                                    all.push(src);
                                    all.extend_from_slice(mut_deps);
                                    Rule::All(all)
                                }
                                Rule::All(mut existing) => {
                                    existing.extend_from_slice(mut_deps);
                                    Rule::All(existing)
                                }
                                _ => Rule::Never,
                            };
                        }
                    }
                }
                rules[r_idx] = rule;
            }

            if let RTerm::Return(Some(val)) = block.term {
                returns.push(val);
            }
        }

        let is_open = open_names.contains(name) || open_items.contains(&func.item);
        let num_vals = func.values.len();
        let ret_ascii = !returns.is_empty();
        states.push(FuncState {
            name: name.clone(),
            func,
            val_types,
            rules,
            returns,
            is_open,
            ascii: vec![false; num_vals],
            grounded: vec![false; num_vals],
            ret_ascii,
            ret_grounded: false,
        });
    }

    // Wire entry parameter rules now that every direct call site has been gathered.
    for (idx, state) in states.iter_mut().enumerate() {
        if force_open_idx[idx] {
            state.is_open = true;
        }
        for (param_idx, &val) in state.func.params.iter().enumerate() {
            let v_idx = val.0 as usize;
            let val_ty = state.val_types[v_idx];
            if state.is_open || val_ty == AsciiType::Other {
                continue;
            }
            let callers = std::mem::take(&mut incoming_calls[idx][param_idx]);
            if callers.is_empty() {
                continue;
            }
            if callers.iter().all(|&(c_idx, arg)| {
                states_pre_match(&names, funcs, c_idx, arg, val_ty, &get_interns)
            }) {
                state.rules[v_idx] = Rule::FuncParam(callers);
            }
        }
        for (v_idx, rule) in state.rules.iter().enumerate() {
            state.ascii[v_idx] = !matches!(rule, Rule::Never);
        }
    }

    // Phase A: Greatest Fixpoint (shrink optimistic `ascii` until stable).
    loop {
        let mut changed = false;
        for idx in 0..states.len() {
            let num_vals = states[idx].ascii.len();
            for v_idx in 0..num_vals {
                if !states[idx].ascii[v_idx] {
                    continue;
                }
                let ok = match &states[idx].rules[v_idx] {
                    Rule::Never => false,
                    Rule::Root => true,
                    Rule::Single(src) => states[idx].ascii[src.0 as usize],
                    Rule::All(deps) => {
                        !deps.is_empty()
                            && deps.iter().all(|&src| states[idx].ascii[src.0 as usize])
                    }
                    Rule::BlockParam(incoming) => {
                        !incoming.is_empty()
                            && incoming
                                .iter()
                                .all(|&src| states[idx].ascii[src.0 as usize])
                    }
                    Rule::FuncParam(callers) => {
                        !states[idx].is_open
                            && !callers.is_empty()
                            && callers
                                .iter()
                                .all(|&(c_idx, arg)| states[c_idx].ascii[arg.0 as usize])
                    }
                    Rule::CallReturn(callee_idx) => states[*callee_idx].ret_ascii,
                };
                if !ok {
                    states[idx].ascii[v_idx] = false;
                    changed = true;
                }
            }
            if states[idx].ret_ascii {
                let ret_ok = !states[idx].returns.is_empty()
                    && states[idx]
                        .returns
                        .iter()
                        .all(|&v| states[idx].ascii[v.0 as usize]);
                if !ret_ok {
                    states[idx].ret_ascii = false;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Phase B: Least Fixpoint for groundedness from constructive Roots over values proven in Phase A.
    loop {
        let mut changed = false;
        for idx in 0..states.len() {
            let num_vals = states[idx].ascii.len();
            for v_idx in 0..num_vals {
                if !states[idx].ascii[v_idx] || states[idx].grounded[v_idx] {
                    continue;
                }
                let is_grounded = match &states[idx].rules[v_idx] {
                    Rule::Never => false,
                    Rule::Root => true,
                    Rule::Single(src) => states[idx].grounded[src.0 as usize],
                    Rule::All(deps) => {
                        !deps.is_empty()
                            && deps.iter().all(|&src| states[idx].grounded[src.0 as usize])
                    }
                    Rule::BlockParam(incoming) => incoming
                        .iter()
                        .any(|&src| states[idx].grounded[src.0 as usize]),
                    Rule::FuncParam(callers) => callers
                        .iter()
                        .any(|&(c_idx, arg)| states[c_idx].grounded[arg.0 as usize]),
                    Rule::CallReturn(callee_idx) => states[*callee_idx].ret_grounded,
                };
                if is_grounded {
                    states[idx].grounded[v_idx] = true;
                    changed = true;
                }
            }
            if states[idx].ret_ascii
                && !states[idx].ret_grounded
                && states[idx]
                    .returns
                    .iter()
                    .any(|&v| states[idx].grounded[v.0 as usize])
            {
                states[idx].ret_grounded = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Publish only scalar `&str` and `Carrier::HostString` values proven in both phases.
    let mut out = BTreeMap::new();
    for state in &states {
        let mut known = BTreeSet::new();
        for (v_idx, &ty) in state.val_types.iter().enumerate() {
            if matches!(ty, AsciiType::SharedStr | AsciiType::HostString)
                && state.ascii[v_idx]
                && state.grounded[v_idx]
            {
                known.insert(ValueId(v_idx as u32));
            }
        }
        if !known.is_empty() {
            out.insert(state.name.clone(), known);
        }
    }

    AsciiFacts {
        funcs: out,
        slice_error_fail,
        slice_reverse: core_slice_items.reverse,
        slice_revswap: core_slice_items.revswap,
        slice_starts_with: core_slice_items.starts_with,
        slice_ends_with: core_slice_items.ends_with,
    }
}

fn states_pre_match<'a, F>(
    names: &[JsName],
    funcs: &BTreeMap<JsName, &'a Func>,
    caller_idx: usize,
    arg: ValueId,
    expected: AsciiType,
    get_interns: &F,
) -> bool
where
    F: Fn(&JsName) -> Option<&'a Interns>,
{
    let caller_name = &names[caller_idx];
    let caller_func = funcs[caller_name];
    let Some(interns) = get_interns(caller_name) else {
        return false;
    };
    let arg_def = caller_func.values[arg.0 as usize];
    let actual = classify_value_type(&interns.types, interns, arg_def.ty, arg_def.carrier);
    actual != AsciiType::Other && actual == expected
}

enum StringBorrowSource<'a> {
    Value(ValueId),
    Cell(&'a [ValueId]),
}

#[allow(clippy::too_many_arguments)]
fn classify_call_result(
    caller_name: &JsName,
    func: &Func,
    r_ty: AsciiType,
    semantic: CallSemantic,
    callee: &Callee,
    args: &[ValueId],
    defs: &[Option<&Op>],
    disqualified: &BTreeSet<ValueId>,
    cell_deps: &BTreeMap<CellId, Vec<ValueId>>,
    val_types: &[AsciiType],
    interns: &Interns,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    name_to_idx: &BTreeMap<JsName, usize>,
) -> Rule {
    let is_owned_ret = matches!(r_ty, AsciiType::HostString | AsciiType::OwnedString);
    if let CallSemantic::Standard(std_op) = semantic {
        match std_op {
            StandardOperation::IntToString
            | StandardOperation::BoolToString
            | StandardOperation::F64SpecToString
            | StandardOperation::NativeStrNew
            | StandardOperation::NativeStrWithCapacity
            | StandardOperation::StringNew
            | StandardOperation::StringWithCapacity
                if is_owned_ret =>
            {
                return Rule::Root;
            }
            StandardOperation::CharToString if is_owned_ret => {
                if let Some(&ch) = args.first() {
                    if is_proven_ascii_char(func, defs, &interns.types, ch) {
                        return Rule::Root;
                    }
                }
                return Rule::Never;
            }
            StandardOperation::StrToLowercase
            | StandardOperation::StrToUppercase
            | StandardOperation::NativeStrFromStr
            | StandardOperation::NativeStrToAsciiLowercase
            | StandardOperation::NativeStrToAsciiUppercase
            | StandardOperation::NativeStrRepeat
                if is_owned_ret =>
            {
                if let Some(&recv) = args.first() {
                    if val_types[recv.0 as usize] == AsciiType::SharedStr {
                        return Rule::Single(recv);
                    }
                }
                return Rule::Never;
            }
            StandardOperation::StrIndex
            | StandardOperation::StrTrim
            | StandardOperation::StrTrimStart
            | StandardOperation::StrTrimEnd
            | StandardOperation::StrSliceUnchecked
                if r_ty == AsciiType::SharedStr =>
            {
                if let Some(&recv) = args.first() {
                    if val_types[recv.0 as usize] == AsciiType::SharedStr {
                        return Rule::Single(recv);
                    }
                }
                return Rule::Never;
            }
            StandardOperation::StrGet
            | StandardOperation::StrStripPrefix
            | StandardOperation::StrStripSuffix
                if matches!(r_ty, AsciiType::EnumStr { .. }) =>
            {
                if let Some(&recv) = args.first() {
                    if val_types[recv.0 as usize] == AsciiType::SharedStr {
                        return Rule::Single(recv);
                    }
                }
                return Rule::Never;
            }
            // Immutable whole-value borrow of an SSA `HostString` or tracked `OwnedString`
            // value/cell preserves ASCII across `String::as_str` / `rcgjs_str_as_str`.
            StandardOperation::StringAsStr | StandardOperation::NativeStrAsStr
                if r_ty == AsciiType::SharedStr =>
            {
                if let Some(&recv) = args.first() {
                    match peel_shared_string_borrow(defs, val_types, disqualified, cell_deps, recv)
                    {
                        Some(StringBorrowSource::Value(v)) => return Rule::Single(v),
                        Some(StringBorrowSource::Cell(deps)) => return Rule::All(deps.to_vec()),
                        None => {}
                    }
                }
                return Rule::Never;
            }
            // Explicit safeguard: arbitrary `String::as_str`, `char::to_string`, `format!` remain unproven.
            StandardOperation::StringAsStr
            | StandardOperation::NativeStrAsStr
            | StandardOperation::CharToString
            | StandardOperation::FormatArgsToString => return Rule::Never,
            _ => {}
        }
    }

    let Callee::Direct { item, link_name } = callee else {
        return Rule::Never;
    };
    let Some(target) = resolve_callee(caller_name, *item, *link_name, interns, repr, abi) else {
        return Rule::Never;
    };
    let Some(&callee_idx) = name_to_idx.get(&target) else {
        return Rule::Never;
    };
    Rule::CallReturn(callee_idx)
}

fn peel_shared_string_borrow<'a>(
    defs: &[Option<&Op>],
    val_types: &[AsciiType],
    disqualified: &BTreeSet<ValueId>,
    cell_deps: &'a BTreeMap<CellId, Vec<ValueId>>,
    mut reference: ValueId,
) -> Option<StringBorrowSource<'a>> {
    for _ in 0..defs.len() {
        let def = (*defs.get(reference.0 as usize)?)?;
        match &def.kind {
            OpKind::Move(value) | OpKind::Copy(value) => reference = *value,
            OpKind::BorrowView {
                root: Root::Value(value),
                path,
                mutable: false,
            } if path.as_slice() == [Proj::Deref] && !disqualified.contains(value) => {
                reference = *value;
            }
            OpKind::BorrowView {
                root: Root::Value(value),
                path,
                mutable: false,
            } if path.is_empty()
                && !disqualified.contains(value)
                && matches!(
                    val_types.get(value.0 as usize).copied(),
                    Some(AsciiType::HostString | AsciiType::OwnedString)
                ) =>
            {
                return Some(StringBorrowSource::Value(*value));
            }
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path,
                mutable: false,
            } if path.is_empty() => {
                if let Some(deps) = cell_deps.get(cell) {
                    return Some(StringBorrowSource::Cell(deps.as_slice()));
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum StringTarget {
    Cell(CellId),
    Value(ValueId),
}

#[derive(Default)]
struct OwnedStringAnalysis {
    /// Non-disqualified `OwnedString` cells mapped to all stored values + pushed `&str` deps.
    cell_deps: BTreeMap<CellId, Vec<ValueId>>,
    /// Accounted `OwnedString` SSA values mapped to all pushed `&str` deps via mutable borrows.
    val_mut_deps: BTreeMap<ValueId, Vec<ValueId>>,
    /// SSA `OwnedString` values whose mutable borrows were all verified ASCII-safe.
    accounted_mut_values: BTreeSet<ValueId>,
}

fn analyze_owned_strings(
    func: &Func,
    defs: &[Option<&Op>],
    val_types: &[AsciiType],
    table: &TypeTable,
) -> OwnedStringAnalysis {
    let owned_cells: BTreeSet<CellId> = func
        .cells
        .iter()
        .enumerate()
        .filter_map(|(idx, cell)| is_owned_string_ty(table, cell.ty).then_some(CellId(idx as u32)))
        .collect();
    let has_owned_vals = val_types.contains(&AsciiType::OwnedString);
    if owned_cells.is_empty() && !has_owned_vals {
        return OwnedStringAnalysis::default();
    }

    let mut bad: BTreeSet<StringTarget> = BTreeSet::new();
    let mut cell_stores: BTreeMap<CellId, Vec<ValueId>> = BTreeMap::new();
    let mut mut_refs: BTreeMap<ValueId, StringTarget> = BTreeMap::new();
    let mut has_mut_borrow: BTreeSet<StringTarget> = BTreeSet::new();

    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::CellStore { cell, v } if owned_cells.contains(cell) => {
                    if val_types.get(v.0 as usize).copied() == Some(AsciiType::OwnedString) {
                        cell_stores.entry(*cell).or_default().push(*v);
                    } else {
                        bad.insert(StringTarget::Cell(*cell));
                    }
                }
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path,
                    mutable: true,
                } if owned_cells.contains(cell) => {
                    let target = StringTarget::Cell(*cell);
                    has_mut_borrow.insert(target);
                    if path.is_empty() {
                        if let Some(res) = op.result {
                            mut_refs.insert(res, target);
                        } else {
                            bad.insert(target);
                        }
                    } else {
                        bad.insert(target);
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(v),
                    path,
                    mutable: true,
                } if val_types.get(v.0 as usize).copied() == Some(AsciiType::OwnedString) => {
                    let target = StringTarget::Value(*v);
                    has_mut_borrow.insert(target);
                    if path.is_empty() {
                        if let Some(res) = op.result {
                            mut_refs.insert(res, target);
                        } else {
                            bad.insert(target);
                        }
                    } else {
                        bad.insert(target);
                    }
                }
                OpKind::RawView {
                    root: Root::Cell(cell),
                    ..
                } if owned_cells.contains(cell) => {
                    bad.insert(StringTarget::Cell(*cell));
                }
                OpKind::RawView {
                    root: Root::Value(v),
                    ..
                } if val_types.get(v.0 as usize).copied() == Some(AsciiType::OwnedString) => {
                    bad.insert(StringTarget::Value(*v));
                }
                OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. }
                    if val_types.get(base.0 as usize).copied() == Some(AsciiType::OwnedString) =>
                {
                    bad.insert(StringTarget::Value(*base));
                }
                _ => {}
            }
        }
    }

    // Transitively propagate mutable `&mut String` reference aliases (`Move`, `Copy`, `&mut *r`).
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                let Some(res) = op.result else { continue };
                if mut_refs.contains_key(&res) {
                    continue;
                }
                let src = match &op.kind {
                    OpKind::Move(v) | OpKind::Copy(v) => Some(*v),
                    OpKind::BorrowView {
                        root: Root::Value(v),
                        path,
                        mutable: true,
                    } if path.as_slice() == [Proj::Deref] => Some(*v),
                    _ => None,
                };
                if let Some(target) = src.and_then(|s| mut_refs.get(&s).copied()) {
                    mut_refs.insert(res, target);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Inspect EVERY use of EVERY mutable reference in `mut_refs`.
    let mut use_counts: BTreeMap<ValueId, usize> = BTreeMap::new();
    let mut mut_deps: BTreeMap<StringTarget, Vec<ValueId>> = BTreeMap::new();

    for block in &func.blocks {
        for used in block.term.uses() {
            if let Some(&target) = mut_refs.get(&used) {
                bad.insert(target);
            }
        }
        for op in &block.ops {
            let uses = op.kind.uses();
            let mut touched: Vec<(ValueId, StringTarget)> = Vec::new();
            for u in &uses {
                if let Some(&target) = mut_refs.get(u) {
                    touched.push((*u, target));
                }
            }
            if touched.is_empty() {
                continue;
            }
            match &op.kind {
                OpKind::Move(v) | OpKind::Copy(v) if touched.len() == 1 && touched[0].0 == *v => {
                    *use_counts.entry(*v).or_insert(0) += 1;
                }
                OpKind::BorrowView {
                    root: Root::Value(v),
                    path,
                    ..
                } if path.as_slice() == [Proj::Deref]
                    && touched.len() == 1
                    && touched[0].0 == *v =>
                {
                    *use_counts.entry(*v).or_insert(0) += 1;
                }
                OpKind::Call {
                    semantic: CallSemantic::Standard(std_op),
                    args,
                    host: None,
                    function: None,
                    ..
                } if touched.len() == 1 && args.first().copied() == Some(touched[0].0) => {
                    let (r, target) = touched[0];
                    *use_counts.entry(r).or_insert(0) += 1;
                    match std_op {
                        StandardOperation::NativeStrClear
                        | StandardOperation::StringClear
                        | StandardOperation::NativeStrTruncate
                        | StandardOperation::NativeStrReserve
                        | StandardOperation::NativeStrShrinkTo
                        | StandardOperation::NativeStrExtendWithin
                        | StandardOperation::NativeStrLen
                        | StandardOperation::NativeStrCapacity
                        | StandardOperation::StringLen
                        | StandardOperation::StringCapacity
                        | StandardOperation::NativeStrAsStr
                        | StandardOperation::StringAsStr
                        | StandardOperation::StringDrop => {}
                        StandardOperation::NativeStrPush
                        | StandardOperation::StringPush
                        | StandardOperation::StringFmtWriteChar => {
                            if args.len() != 2 || !is_proven_ascii_char(func, defs, table, args[1])
                            {
                                bad.insert(target);
                            }
                        }
                        StandardOperation::NativeStrPushStr
                        | StandardOperation::StringPushStr
                        | StandardOperation::StringFmtWriteStr => {
                            if args.len() == 2
                                && val_types.get(args[1].0 as usize).copied()
                                    == Some(AsciiType::SharedStr)
                            {
                                mut_deps.entry(target).or_default().push(args[1]);
                            } else {
                                bad.insert(target);
                            }
                        }
                        StandardOperation::NativeStrReplaceSlice => {
                            if args.len() == 4
                                && val_types.get(args[3].0 as usize).copied()
                                    == Some(AsciiType::SharedStr)
                            {
                                mut_deps.entry(target).or_default().push(args[3]);
                            } else {
                                bad.insert(target);
                            }
                        }
                        StandardOperation::NativeStrCloneFrom if args.len() == 2 => {
                            // Only track immutable whole-value borrow of an SSA OwnedString.
                            if let Some(src_v) = peel_owned_ssa_ref(defs, val_types, args[1]) {
                                mut_deps.entry(target).or_default().push(src_v);
                            } else {
                                bad.insert(target);
                            }
                        }
                        _ => {
                            bad.insert(target);
                        }
                    }
                }
                _ => {
                    for (_, target) in touched {
                        bad.insert(target);
                    }
                }
            }
        }
    }

    // Require every mutable borrow to have at least one accounted use (dead synthetic borrows
    // remain conservative).
    for (r, target) in &mut_refs {
        if use_counts.get(r).copied().unwrap_or(0) == 0 {
            bad.insert(*target);
        }
    }

    let mut out = OwnedStringAnalysis::default();
    for cell in owned_cells {
        let target = StringTarget::Cell(cell);
        if bad.contains(&target) {
            continue;
        }
        let Some(stores) = cell_stores.get(&cell) else {
            continue;
        };
        if stores.is_empty() {
            continue;
        }
        let mut deps = stores.clone();
        if let Some(extra) = mut_deps.get(&target) {
            deps.extend_from_slice(extra);
        }
        out.cell_deps.insert(cell, deps);
    }
    for target in has_mut_borrow {
        let StringTarget::Value(v) = target else {
            continue;
        };
        if bad.contains(&target) {
            continue;
        }
        out.accounted_mut_values.insert(v);
        if let Some(extra) = mut_deps.remove(&target) {
            out.val_mut_deps.insert(v, extra);
        }
    }
    out
}

fn peel_owned_ssa_ref(
    defs: &[Option<&Op>],
    val_types: &[AsciiType],
    mut reference: ValueId,
) -> Option<ValueId> {
    for _ in 0..defs.len() {
        let def = (*defs.get(reference.0 as usize)?)?;
        match &def.kind {
            OpKind::Move(v) | OpKind::Copy(v) => reference = *v,
            OpKind::BorrowView {
                root: Root::Value(v),
                path,
                mutable: false,
            } if path.as_slice() == [Proj::Deref] => reference = *v,
            OpKind::BorrowView {
                root: Root::Value(v),
                path,
                mutable: false,
            } if path.is_empty()
                && matches!(
                    val_types.get(v.0 as usize).copied(),
                    Some(AsciiType::HostString | AsciiType::OwnedString)
                ) =>
            {
                return Some(*v);
            }
            _ => return None,
        }
    }
    None
}

/// Proves that an SSA `char` (or unsigned integer) value is strictly in `0..=127` (ASCII).
fn is_proven_ascii_char(func: &Func, defs: &[Option<&Op>], table: &TypeTable, v: ValueId) -> bool {
    unsigned_upper_bound(func, defs, table, v, 0).is_some_and(|ub| ub <= 127)
}

fn unsigned_upper_bound(
    func: &Func,
    defs: &[Option<&Op>],
    table: &TypeTable,
    v: ValueId,
    depth: u8,
) -> Option<u64> {
    if depth > 8 {
        return None;
    }
    let ty = func.values.get(v.0 as usize)?.ty;
    let bit_width: u32 = match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Char) => 32,
        Some(TypeKind::Integer {
            signed: false,
            bits,
        }) if *bits <= 64 => *bits,
        _ => return None,
    };
    let type_max: u64 = if bit_width >= 64 {
        u64::MAX
    } else {
        (1u64 << bit_width) - 1
    };
    let def = (*defs.get(v.0 as usize)?)?;
    match &def.kind {
        OpKind::Const(ConstTree::Scalar { bits, .. }) => u64::try_from(*bits).ok(),
        OpKind::Move(src) | OpKind::Copy(src) => {
            unsigned_upper_bound(func, defs, table, *src, depth + 1)
        }
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: src,
            ..
        } => {
            let ub = unsigned_upper_bound(func, defs, table, *src, depth + 1)?;
            // Sound without truncation wraparound only when `ub <= type_max`.
            (ub <= type_max).then_some(ub)
        }
        OpKind::Binary { op, l, r, .. } => match op {
            BinaryOp::BitAnd => {
                let ul = unsigned_upper_bound(func, defs, table, *l, depth + 1);
                let ur = unsigned_upper_bound(func, defs, table, *r, depth + 1);
                match (ul, ur) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                }
            }
            BinaryOp::Rem => {
                let rhs = unsigned_upper_bound(func, defs, table, *r, depth + 1)?;
                // `Rem` divisor must be a constant scalar >= 1 so `x % c <= c - 1`.
                let is_const = matches!(
                    defs.get(r.0 as usize)
                        .and_then(|d| d.as_ref())
                        .map(|o| &o.kind),
                    Some(OpKind::Const(ConstTree::Scalar { .. }))
                );
                (is_const && rhs >= 1).then_some(rhs - 1)
            }
            BinaryOp::Shr => {
                let is_const = matches!(
                    defs.get(r.0 as usize)
                        .and_then(|d| d.as_ref())
                        .map(|o| &o.kind),
                    Some(OpKind::Const(ConstTree::Scalar { .. }))
                );
                let shift = unsigned_upper_bound(func, defs, table, *r, depth + 1)?;
                if !is_const || shift >= u64::from(bit_width) {
                    return None;
                }
                let lhs_ub =
                    unsigned_upper_bound(func, defs, table, *l, depth + 1).unwrap_or(type_max);
                Some(lhs_ub >> shift)
            }
            BinaryOp::Add => {
                let ul = unsigned_upper_bound(func, defs, table, *l, depth + 1)?;
                let ur = unsigned_upper_bound(func, defs, table, *r, depth + 1)?;
                let sum = ul.checked_add(ur)?;
                (sum <= type_max).then_some(sum)
            }
            _ => None,
        },
        _ => None,
    }
}

fn resolve_callee(
    caller_name: &JsName,
    item: ItemId,
    link_name: StrId,
    interns: &Interns,
    repr: &ReprOutcome,
    abi: &AbiPlan,
) -> Option<JsName> {
    if let Some(target) = abi.call_retargets(caller_name).get(&item) {
        return Some(target.clone());
    }
    if let Some(target) = repr
        .call_retargets
        .get(caller_name)
        .and_then(|m| m.get(&item))
    {
        return Some(target.clone());
    }
    interns.str(link_name).map(|s| JsName::new(s.to_owned()))
}

fn collect_disqualified(
    func: &Func,
    accounted_mut_values: &BTreeSet<ValueId>,
) -> BTreeSet<ValueId> {
    let mut bad = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. } => {
                    bad.insert(*base);
                }
                OpKind::BorrowView {
                    root: Root::Value(v),
                    mutable: true,
                    ..
                } => {
                    if !accounted_mut_values.contains(v) {
                        bad.insert(*v);
                    }
                }
                OpKind::RawView {
                    root: Root::Value(v),
                    ..
                } => {
                    bad.insert(*v);
                }
                _ => {}
            }
        }
    }
    bad
}

fn is_shared_str_ref(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference {
            mutable: false,
            pointee,
        }) if matches!(
            table.get(table.peeled(*pointee)).map(|d| &d.kind),
            Some(TypeKind::Str)
        )
    )
}

fn is_owned_string_ty(table: &TypeTable, ty: TypeId) -> bool {
    table
        .get(table.peeled(ty))
        .is_some_and(|d| d.meta.candidate == RepresentationCandidate::String)
}

fn is_mut_slice_ref(table: &TypeTable, ty: TypeId) -> Option<TypeId> {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference {
            mutable: true,
            pointee,
        }) => match table.get(table.peeled(*pointee)).map(|d| &d.kind) {
            Some(TypeKind::Slice(elem)) => Some(*elem),
            _ => None,
        },
        _ => None,
    }
}

fn is_shared_u8_slice_ref(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference {
            mutable: false,
            pointee,
        }) if matches!(
            table.get(table.peeled(*pointee)).map(|d| &d.kind),
            Some(TypeKind::Slice(elem)) if matches!(
                table.get(table.peeled(*elem)).map(|d| &d.kind),
                Some(TypeKind::Integer {
                    signed: false,
                    bits: 8,
                })
            )
        )
    )
}

#[derive(Default)]
struct CoreSliceItems {
    reverse: BTreeSet<ItemId>,
    revswap: BTreeSet<ItemId>,
    starts_with: BTreeSet<ItemId>,
    ends_with: BTreeSet<ItemId>,
}

/// Authenticates live `core::slice::<impl [T]>::{reverse,reverse::revswap,starts_with,ends_with}`
/// items via an unforgeable unique `core` `StableCrateId`, origin metadata, and exact JIR
/// parameter/return types, propagating authenticated IDs to any `bstr_abi` synthetic clones.
fn find_core_slice_items<'a, F>(
    funcs: &BTreeMap<JsName, &'a Func>,
    table: &BTreeMap<JsName, LinkItem>,
    get_interns: &F,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> CoreSliceItems
where
    F: Fn(&JsName) -> Option<&'a Interns>,
{
    let is_core_std_op = |op: StandardOperation| {
        op.is_str()
            || op.is_fast_str_op()
            || matches!(
                op,
                StandardOperation::SliceSwap
                    | StandardOperation::SliceSort
                    | StandardOperation::SliceSortCmp
                    | StandardOperation::F64FromStr
                    | StandardOperation::F32FromStr
                    | StandardOperation::FormatArgumentNewDisplay
                    | StandardOperation::FormatArgsNew
            )
    };
    let mut std_core_crates = BTreeSet::new();
    let mut named_core_crates = BTreeSet::new();
    for (name, func) in funcs {
        if !defined.contains(name) || !live.contains(name) || repr.bodies.contains_key(name) {
            continue;
        }
        if let Some(item) = table.get(name) {
            if matches!(
                item.jir.body.as_ref().map(|b| b.semantic),
                Some(BodySemantic::Standard(op)) if is_core_std_op(op)
            ) {
                std_core_crates.insert(func.item.definition.crate_id);
            }
            if item.crate_name == "core" {
                if let Some(interns) = get_interns(name) {
                    let (origin_crate, _) =
                        crate::lower::semantic_ops::string::item_origin_info(item, interns);
                    if origin_crate == "core" {
                        named_core_crates.insert(func.item.definition.crate_id);
                    }
                }
            }
        }
        for block in &func.blocks {
            for op in &block.ops {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    function: None,
                    semantic: CallSemantic::Standard(std_op),
                    host: None,
                    ..
                } = &op.kind
                {
                    if is_core_std_op(*std_op) {
                        std_core_crates.insert(item.definition.crate_id);
                    }
                }
            }
        }
    }
    let core_crate_ids = if !std_core_crates.is_empty() {
        std_core_crates
    } else {
        named_core_crates
    };
    if core_crate_ids.len() != 1 {
        return CoreSliceItems::default();
    }
    let core_crate_id = *core_crate_ids.iter().next().unwrap();

    // Map source item names to their `bstr_abi` / `abi_flat` synthetic clone `ItemId`s.
    let mut clones_of: BTreeMap<&JsName, Vec<ItemId>> = BTreeMap::new();
    for (clone_name, source_name) in abi.clones() {
        if let Some(&synth_id) = abi.synthetic_items.get(clone_name) {
            clones_of.entry(source_name).or_default().push(synth_id);
        }
    }

    let mut out = CoreSliceItems::default();
    for (name, func) in funcs {
        if !defined.contains(name) || !live.contains(name) || repr.bodies.contains_key(name) {
            continue;
        }
        if func.item.definition.crate_id != core_crate_id
            || func.sig.abi.caller_location
            || func.sig.abi.spread.is_some()
        {
            continue;
        }
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.crate_name != "core" {
            continue;
        }
        let Some(interns) = get_interns(name) else {
            continue;
        };
        let (origin_crate, origin_path) =
            crate::lower::semantic_ops::string::item_origin_info(item, interns);
        if origin_crate != "core" {
            continue;
        }
        let type_table = &interns.types;
        let dp = item.debug_path.as_str();
        let path_matches = |suffix: &str, core_suffix: &str| {
            (origin_path == suffix || origin_path == core_suffix)
                && (dp == suffix || dp == core_suffix)
        };
        let insert_with_clones = |set: &mut BTreeSet<ItemId>| {
            set.insert(func.item);
            if let Some(synth_ids) = clones_of.get(name) {
                for &sid in synth_ids {
                    set.insert(sid);
                }
            }
        };

        if path_matches(
            "slice::<impl [T]>::reverse",
            "core::slice::<impl [T]>::reverse",
        ) && func.params.len() == 1
            && type_table.is_zst(func.sig.result)
        {
            if let Some(p0_ty) = func.values.get(func.params[0].0 as usize).map(|v| v.ty) {
                if let Some(elem) = is_mut_slice_ref(type_table, p0_ty) {
                    if type_table.get(elem).is_some_and(|d| d.layout.sized) {
                        insert_with_clones(&mut out.reverse);
                    }
                }
            }
        } else if path_matches(
            "slice::<impl [T]>::reverse::revswap",
            "core::slice::<impl [T]>::reverse::revswap",
        ) && func.params.len() == 3
            && type_table.is_zst(func.sig.result)
        {
            let p0_ty = func.values.get(func.params[0].0 as usize).map(|v| v.ty);
            let p1_ty = func.values.get(func.params[1].0 as usize).map(|v| v.ty);
            let p2_ty = func.values.get(func.params[2].0 as usize).map(|v| v.ty);
            let is_usize = p2_ty.is_some_and(|ty| {
                matches!(
                    type_table.get(type_table.peeled(ty)).map(|d| &d.kind),
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: 32,
                    })
                )
            });
            if is_usize {
                if let (Some(e0), Some(e1)) = (
                    p0_ty.and_then(|t| is_mut_slice_ref(type_table, t)),
                    p1_ty.and_then(|t| is_mut_slice_ref(type_table, t)),
                ) {
                    if e0 == e1 && type_table.get(e0).is_some_and(|d| d.layout.sized) {
                        insert_with_clones(&mut out.revswap);
                    }
                }
            }
        } else if (path_matches(
            "slice::<impl [T]>::starts_with",
            "core::slice::<impl [T]>::starts_with",
        ) || path_matches(
            "slice::<impl [T]>::ends_with",
            "core::slice::<impl [T]>::ends_with",
        )) && func.params.len() == 2
            && matches!(
                type_table
                    .get(type_table.peeled(func.sig.result))
                    .map(|d| &d.kind),
                Some(TypeKind::Bool)
            )
        {
            let p0_ok = func
                .values
                .get(func.params[0].0 as usize)
                .is_some_and(|v| is_shared_u8_slice_ref(type_table, v.ty));
            let p1_ok = func
                .values
                .get(func.params[1].0 as usize)
                .is_some_and(|v| is_shared_u8_slice_ref(type_table, v.ty));
            if p0_ok && p1_ok {
                if origin_path.ends_with("starts_with") {
                    insert_with_clones(&mut out.starts_with);
                } else {
                    insert_with_clones(&mut out.ends_with);
                }
            }
        }
    }
    out
}

/// Locates the unique live `core::str::slice_error_fail` item authenticated by `StrIndex`
/// callees' `StableCrateId` (`core` by Rust coherence/orphan rules), origin metadata, and exact
/// 4-parameter `(s: &str, start: usize, end: usize, $loc) -> !` ABI without a flat call shape.
fn find_slice_error_fail<'a, F>(
    funcs: &BTreeMap<JsName, &'a Func>,
    table: &BTreeMap<JsName, LinkItem>,
    get_interns: &F,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> Option<JsName>
where
    F: Fn(&JsName) -> Option<&'a Interns>,
{
    let mut core_crate_ids = BTreeSet::new();
    for (name, func) in funcs {
        if !defined.contains(name) || !live.contains(name) || repr.bodies.contains_key(name) {
            continue;
        }
        for block in &func.blocks {
            for op in &block.ops {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    function: None,
                    semantic: CallSemantic::Standard(StandardOperation::StrIndex),
                    host: None,
                    ..
                } = &op.kind
                {
                    core_crate_ids.insert(item.definition.crate_id);
                }
            }
        }
    }
    if core_crate_ids.len() != 1 {
        return None;
    }
    let core_crate_id = *core_crate_ids.iter().next()?;

    let mut candidates = Vec::new();
    for (name, func) in funcs {
        if !defined.contains(name) || !live.contains(name) || repr.bodies.contains_key(name) {
            continue;
        }
        if func.item.definition.crate_id != core_crate_id {
            continue;
        }
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.crate_name != "core" {
            continue;
        }
        if item.debug_path != "str::slice_error_fail"
            && item.debug_path != "core::str::slice_error_fail"
        {
            continue;
        }
        let Some(interns) = get_interns(name) else {
            continue;
        };
        let (origin_crate, origin_path) =
            crate::lower::semantic_ops::string::item_origin_info(item, interns);
        if origin_crate != "core"
            || !matches!(
                origin_path,
                "str::slice_error_fail" | "core::str::slice_error_fail"
            )
        {
            continue;
        }
        if !func.sig.abi.caller_location
            || func.params.len() != 4
            || abi.call_signature(func.item).is_some()
        {
            continue;
        }
        let type_table = &interns.types;
        if !matches!(
            type_table
                .get(type_table.peeled(func.sig.result))
                .map(|d| &d.kind),
            Some(TypeKind::Never)
        ) {
            continue;
        }
        let [p0, p1, p2, _p3] = func.params.as_slice() else {
            continue;
        };
        let p0_ty = func.values.get(p0.0 as usize).map(|v| v.ty);
        let p1_ty = func.values.get(p1.0 as usize).map(|v| v.ty);
        let p2_ty = func.values.get(p2.0 as usize).map(|v| v.ty);
        let is_usize = |ty: Option<TypeId>| {
            ty.is_some_and(|ty| {
                matches!(
                    type_table.get(type_table.peeled(ty)).map(|d| &d.kind),
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: 32,
                    })
                )
            })
        };
        if !p0_ty.is_some_and(|ty| is_shared_str_ref(type_table, ty))
            || !is_usize(p1_ty)
            || !is_usize(p2_ty)
        {
            continue;
        }
        candidates.push(name.clone());
    }
    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

fn classify_value_type(
    table: &TypeTable,
    _interns: &Interns,
    ty: TypeId,
    carrier: Carrier,
) -> AsciiType {
    if carrier == Carrier::HostString {
        return AsciiType::HostString;
    }
    if carrier != Carrier::Baseline {
        return AsciiType::Other;
    }
    if is_shared_str_ref(table, ty) {
        return AsciiType::SharedStr;
    }
    if is_owned_string_ty(table, ty) {
        return AsciiType::OwnedString;
    }
    let peeled = table.peeled(ty);
    let variants = table.variants(peeled);
    if variants.len() == 2 {
        let variant_str_field = |v: &VariantDescriptor| -> Option<Option<(u32, bool)>> {
            let mut live = v
                .fields
                .iter()
                .enumerate()
                .filter(|(_, f)| !table.is_zst(f.ty));
            let Some((fidx, field)) = live.next() else {
                return Some(None);
            };
            if live.next().is_some() {
                return None;
            }
            if is_shared_str_ref(table, field.ty) {
                Some(Some((fidx as u32, false)))
            } else if is_owned_string_ty(table, field.ty) {
                Some(Some((fidx as u32, true)))
            } else {
                Some(None)
            }
        };
        if let (Some(v0), Some(v1)) = (
            variant_str_field(&variants[0]),
            variant_str_field(&variants[1]),
        ) {
            if v0.is_some() || v1.is_some() {
                return AsciiType::EnumStr { v0, v1 };
            }
        }
    }
    AsciiType::Other
}

fn is_ascii_str_const(interns: &Interns, tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Str(id) => interns.str(*id).is_some_and(|s| s.is_ascii()),
        ConstTree::Reference(inner) => matches!(
            inner.as_ref(),
            ConstTree::Str(id) if interns.str(*id).is_some_and(|s| s.is_ascii())
        ),
        _ => false,
    }
}

fn is_ascii_const(
    _table: &TypeTable,
    interns: &Interns,
    tree: &ConstTree,
    expected: AsciiType,
) -> bool {
    match expected {
        AsciiType::SharedStr => is_ascii_str_const(interns, tree),
        AsciiType::EnumStr { v0, v1 } => match tree {
            ConstTree::Variant { variant, fields } => {
                let spec = match *variant {
                    0 => v0,
                    1 => v1,
                    _ => return false,
                };
                match spec {
                    None => true,
                    Some((fidx, false)) => fields
                        .get(fidx as usize)
                        .is_some_and(|f| is_ascii_str_const(interns, f)),
                    Some((_, true)) => false,
                }
            }
            _ => false,
        },
        AsciiType::HostString | AsciiType::OwnedString | AsciiType::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgjs_object::item::{ItemKind, ItemMap, LinkJir};
    use rcgjs_object::jir::repr::{FuncBuilder, RTerm};
    use rcgjs_object::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
    use rcgjs_object::jir::types::{
        FieldDescriptor, FieldKey, JsFacts, LayoutFacts, NominalForm, TypeDescriptor, TypeMeta,
    };
    use rcgjs_object::jir::OriginId;
    use rcgjs_object::object::Linkage;

    const STR_PRIM: TypeId = TypeId(1);
    const STR_REF: TypeId = TypeId(2);
    const STRING_TY: TypeId = TypeId(3);
    const BOOL_TY: TypeId = TypeId(4);
    const OPT_STR: TypeId = TypeId(5);
    const USIZE_TY: TypeId = TypeId(6);
    const NEVER_TY: TypeId = TypeId(7);
    const CORE_CRATE_ID: u64 = 77;

    fn desc(id: TypeId, kind: TypeKind) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts::default(),
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
        }
    }

    fn test_interns() -> (Interns, StrId, StrId, StrId, StrId) {
        let mut interns = Interns::new();
        interns.strings.push("ascii".to_owned());
        let ascii_str = StrId(0);
        interns.strings.push("é😀".to_owned());
        let non_ascii_str = StrId(1);
        interns.strings.push("None".to_owned());
        let none_str = StrId(2);
        interns.strings.push("Some".to_owned());
        let some_str = StrId(3);
        interns.strings.push("callee".to_owned());
        let callee_link = StrId(4);
        interns.strings.push("helper".to_owned());
        let helper_link = StrId(5);

        interns.types.insert(desc(STR_PRIM, TypeKind::Str));
        interns.types.insert(desc(
            STR_REF,
            TypeKind::Reference {
                pointee: STR_PRIM,
                mutable: false,
            },
        ));
        interns.types.insert(desc(
            STRING_TY,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 10,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: Vec::new(),
                niche: None,
            },
        ));
        interns.types.insert(desc(BOOL_TY, TypeKind::Bool));
        interns.types.insert(desc(
            USIZE_TY,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
        ));
        interns.types.insert(desc(NEVER_TY, TypeKind::Never));
        interns.types.insert(desc(
            OPT_STR,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 11,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: none_str,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: some_str,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: STR_REF,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
        ));
        (interns, ascii_str, non_ascii_str, callee_link, helper_link)
    }

    fn item_id(hash: u64) -> ItemId {
        item_id_in(1, hash)
    }

    fn item_id_in(crate_id: u64, hash: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id,
                local_hash: hash,
            },
            instantiation: hash as u128,
        }
    }

    fn sig(args: Vec<TypeId>, result: TypeId, caller_location: bool) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: args.len() as u16 + u16::from(caller_location),
                spread: None,
                caller_location,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments: args,
            result,
        }
    }

    fn link_item(name: &str, linkage: Linkage, debug_path: &str) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage,
            debug_path: debug_path.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: LinkJir::default(),
        }
    }

    fn run(
        funcs: &BTreeMap<JsName, Func>,
        table: &BTreeMap<JsName, LinkItem>,
        interns: &Interns,
        roots: &BTreeSet<JsName>,
    ) -> AsciiFacts {
        let refs: BTreeMap<JsName, &Func> = funcs.iter().map(|(k, v)| (k.clone(), v)).collect();
        let defined: BTreeSet<JsName> = table.keys().cloned().collect();
        let live: BTreeSet<JsName> = table.keys().cloned().collect();
        compute_with_interns(
            &refs,
            table,
            |_| Some(interns),
            &ReprOutcome::default(),
            &AbiPlan::default(),
            roots,
            &defined,
            &live,
        )
    }

    #[test]
    fn sound_across_callers_loops_exports_cycles_and_preserve_ops() {
        let (mut interns, ascii_id, wide_id, callee_link, helper_link) = test_interns();
        let cycle_str = StrId(interns.strings.len() as u32);
        interns.strings.push("cycle".to_owned());
        let mixed_str = StrId(interns.strings.len() as u32);
        interns.strings.push("mixed".to_owned());
        let exported_str = StrId(interns.strings.len() as u32);
        interns.strings.push("exported".to_owned());
        let o = OriginId(0);

        // `callee(p: &str) -> &str`: called only with ASCII, trims & returns &str -> KnownAscii.
        let mut callee_b = FuncBuilder::new(item_id(1), sig(vec![STR_REF], STR_REF, false));
        let p = callee_b.param(STR_REF, o);
        let trimmed = callee_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(99),
                    link_name: helper_link,
                },
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StrTrim),
                host: None,
            },
            STR_REF,
            o,
        );
        callee_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(trimmed)));
        let callee_func = callee_b.finish();

        // `mixed(p: &str) -> &str`: called once with "ascii" and once with "é😀" -> MUST NOT be KnownAscii.
        let mut mixed_b = FuncBuilder::new(item_id(2), sig(vec![STR_REF], STR_REF, false));
        let mp = mixed_b.param(STR_REF, o);
        mixed_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(mp)));
        let mixed_func = mixed_b.finish();

        // `cycle() -> &str`: pure cycle calling itself -> ungrounded, MUST NOT be KnownAscii.
        let mut cycle_b = FuncBuilder::new(item_id(3), sig(vec![], STR_REF, false));
        let cy_res = cycle_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(3),
                    link_name: cycle_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STR_REF,
            o,
        );
        cycle_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(cy_res)));
        let cycle_func = cycle_b.finish();

        // `exported(p: &str)`: exported root -> param MUST stay MaybeNonAscii even if internal caller passes ASCII.
        let mut exp_b = FuncBuilder::new(item_id(4), sig(vec![STR_REF], STR_REF, false));
        let ep = exp_b.param(STR_REF, o);
        exp_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(ep)));
        let exp_func = exp_b.finish();

        // `caller()`: tests literals, IntToString(HostString), StringAsStr, StrStripPrefix + Field unwrap,
        // loop block param with non-ASCII back-edge.
        let mut caller_b = FuncBuilder::new(item_id(5), sig(vec![], STR_REF, false));
        let lit_ascii = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(ascii_id)),
            STR_REF,
            o,
        );
        let lit_wide = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(wide_id)),
            STR_REF,
            o,
        );
        let slice_idx_arg = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE_TY,
            o,
        );
        caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id_in(CORE_CRATE_ID, 200),
                    link_name: helper_link,
                },
                function: None,
                args: vec![lit_ascii, slice_idx_arg],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StrIndex),
                host: None,
            },
            STR_REF,
            o,
        );
        // Test `String::as_str`: Baseline (`other_str`) and mutated `HostString` stay MaybeNonAscii;
        // immutable whole-value borrow of `IntToString` (`HostString`) preserves KnownAscii.
        let mut other_str_b = FuncBuilder::new(item_id(11), sig(vec![], STR_REF, false));
        let base_str = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(90),
                    link_name: helper_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::IntToString),
                host: None,
            },
            STRING_TY,
            o,
        );
        let base_borrow = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(base_str),
                path: Vec::new(),
                mutable: false,
            },
            STRING_TY,
            o,
        );
        let base_as_str = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(91),
                    link_name: helper_link,
                },
                function: None,
                args: vec![base_borrow],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StringAsStr),
                host: None,
            },
            STR_REF,
            o,
        );
        let host_mutated = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(90),
                    link_name: helper_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::IntToString),
                host: None,
            },
            STRING_TY,
            o,
        );
        other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(host_mutated),
                path: Vec::new(),
                mutable: true,
            },
            STRING_TY,
            o,
        );
        let host_mut_borrow = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(host_mutated),
                path: Vec::new(),
                mutable: false,
            },
            STRING_TY,
            o,
        );
        let host_mut_as_str = other_str_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(91),
                    link_name: helper_link,
                },
                function: None,
                args: vec![host_mut_borrow],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StringAsStr),
                host: None,
            },
            STR_REF,
            o,
        );
        other_str_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(base_as_str)));
        let mut string_cases_func = other_str_b.finish();
        string_cases_func.values[host_mutated.0 as usize].carrier = Carrier::HostString;

        let int_str = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(90),
                    link_name: helper_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::IntToString),
                host: None,
            },
            STRING_TY,
            o,
        );
        let int_str_ref = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(int_str),
                path: Vec::new(),
                mutable: false,
            },
            STRING_TY,
            o,
        );
        let as_str = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(91),
                    link_name: helper_link,
                },
                function: None,
                args: vec![int_str_ref],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StringAsStr),
                host: None,
            },
            STR_REF,
            o,
        );
        let opt_strip = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(92),
                    link_name: helper_link,
                },
                function: None,
                args: vec![lit_ascii, lit_wide],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StrStripPrefix),
                host: None,
            },
            OPT_STR,
            o,
        );
        let unwrapped = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: opt_strip,
                variant: Some(1),
                field: 0,
            },
            STR_REF,
            o,
        );
        let call_callee = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(1),
                    link_name: callee_link,
                },
                function: None,
                args: vec![unwrapped],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STR_REF,
            o,
        );
        caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(2),
                    link_name: mixed_str,
                },
                function: None,
                args: vec![lit_ascii],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STR_REF,
            o,
        );
        caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(2),
                    link_name: mixed_str,
                },
                function: None,
                args: vec![lit_wide],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STR_REF,
            o,
        );
        caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(4),
                    link_name: exported_str,
                },
                function: None,
                args: vec![lit_ascii],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STR_REF,
            o,
        );
        let loop_b = caller_b.block();
        let loop_phi = caller_b.block_param(loop_b, STR_REF, o);
        caller_b.terminate(FuncBuilder::ENTRY, RTerm::Goto(loop_b, vec![lit_ascii]));
        // Back-edge feeds `lit_wide` into `loop_phi` -> `loop_phi` MUST NOT be KnownAscii.
        let cond = caller_b.op(
            loop_b,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
            BOOL_TY,
            o,
        );
        caller_b.terminate(
            loop_b,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: BOOL_TY,
                cases: vec![(0, loop_b, vec![lit_wide])],
                default: (loop_b, vec![loop_phi]),
            },
        );
        let mut caller_func = caller_b.finish();
        caller_func.values[int_str.0 as usize].carrier = Carrier::HostString;

        let fail_path = StrId(interns.strings.len() as u32);
        interns.strings.push("str::slice_error_fail".to_owned());

        // Also test live `core::str::slice_error_fail` detection.
        let mut fail_b = FuncBuilder::new(
            item_id_in(CORE_CRATE_ID, 10),
            sig(vec![STR_REF, USIZE_TY, USIZE_TY], NEVER_TY, true),
        );
        let _ = (
            fail_b.param(STR_REF, o),
            fail_b.param(USIZE_TY, o),
            fail_b.param(USIZE_TY, o),
            fail_b.param(USIZE_TY, o),
        );
        fail_b.terminate(FuncBuilder::ENTRY, RTerm::Unreachable);
        let fail_func = fail_b.finish();

        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("callee".to_owned()), callee_func);
        funcs.insert(JsName::new("mixed".to_owned()), mixed_func);
        funcs.insert(JsName::new("cycle".to_owned()), cycle_func);
        funcs.insert(JsName::new("exported".to_owned()), exp_func);
        funcs.insert(JsName::new("caller".to_owned()), caller_func);
        funcs.insert(JsName::new("string_cases".to_owned()), string_cases_func);
        funcs.insert(
            JsName::new("core$str$slice_error_fail$h123".to_owned()),
            fail_func,
        );

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("callee".to_owned()),
            link_item("callee", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("mixed".to_owned()),
            link_item("mixed", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("cycle".to_owned()),
            link_item("cycle", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("exported".to_owned()),
            link_item("exported", Linkage::exported("exported".to_owned()), ""),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            link_item("caller", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("string_cases".to_owned()),
            link_item("string_cases", Linkage::internal(), ""),
        );
        let mut fail_item = link_item(
            "core$str$slice_error_fail$h123",
            Linkage::internal(),
            "str::slice_error_fail",
        );
        fail_item.crate_name = "core".to_owned();
        fail_item.jir.origins = Some(rcgjs_object::jir::OriginTable {
            origins: Vec::new(),
            statements: Vec::new(),
            terminators: Vec::new(),
            locals: Vec::new(),
            item_path: fail_path,
            root: None,
        });
        table.insert(
            JsName::new("core$str$slice_error_fail$h123".to_owned()),
            fail_item,
        );
        let roots = BTreeSet::from([
            JsName::new("caller".to_owned()),
            JsName::new("string_cases".to_owned()),
        ]);
        let facts = run(&funcs, &table, &interns, &roots);

        let caller_facts = facts.for_item(&JsName::new("caller".to_owned()));
        assert!(caller_facts.is_known_ascii(lit_ascii));
        assert!(!caller_facts.is_known_ascii(lit_wide));
        assert!(caller_facts.is_known_ascii(int_str));
        assert!(caller_facts.is_known_ascii(as_str)); // immutable HostString borrow preserves KnownAscii
        let sc_facts = facts.for_item(&JsName::new("string_cases".to_owned()));
        assert!(!sc_facts.is_known_ascii(base_as_str)); // Baseline String::as_str stays MaybeNonAscii
        assert!(!sc_facts.is_known_ascii(host_mut_as_str)); // mut-borrowed HostString stays MaybeNonAscii
        assert!(!caller_facts.is_known_ascii(opt_strip)); // Option<&str> is not scalar &str
        assert!(caller_facts.is_known_ascii(unwrapped));
        assert!(caller_facts.is_known_ascii(call_callee));
        assert!(!caller_facts.is_known_ascii(loop_phi)); // non-ASCII back-edge poisons loop param

        let callee_facts = facts.for_item(&JsName::new("callee".to_owned()));
        assert!(callee_facts.is_known_ascii(p));
        assert!(callee_facts.is_known_ascii(trimmed));

        let mixed_facts = facts.for_item(&JsName::new("mixed".to_owned()));
        assert!(!mixed_facts.is_known_ascii(mp));

        let cycle_facts = facts.for_item(&JsName::new("cycle".to_owned()));
        assert!(!cycle_facts.is_known_ascii(cy_res));

        let exp_facts = facts.for_item(&JsName::new("exported".to_owned()));
        assert!(!exp_facts.is_known_ascii(ep));

        assert_eq!(
            facts.slice_error_fail,
            Some(JsName::new("core$str$slice_error_fail$h123".to_owned()))
        );
    }

    #[test]
    fn slice_error_fail_rejects_lexical_and_crate_name_spoofs() {
        let (mut interns, ascii_id, _, _, helper_link) = test_interns();
        let fail_path = StrId(interns.strings.len() as u32);
        interns.strings.push("str::slice_error_fail".to_owned());
        let core_fail_path = StrId(interns.strings.len() as u32);
        interns
            .strings
            .push("core::str::slice_error_fail".to_owned());
        let o = OriginId(0);

        let mut caller_b = FuncBuilder::new(item_id(5), sig(vec![], STR_REF, false));
        let lit = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(ascii_id)),
            STR_REF,
            o,
        );
        let idx = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE_TY,
            o,
        );
        caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id_in(CORE_CRATE_ID, 200),
                    link_name: helper_link,
                },
                function: None,
                args: vec![lit, idx],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StrIndex),
                host: None,
            },
            STR_REF,
            o,
        );
        caller_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(lit)));
        let caller_func = caller_b.finish();

        let make_fail_fn = |crate_id, hash, ret_ty| {
            let mut b = FuncBuilder::new(
                item_id_in(crate_id, hash),
                sig(vec![STR_REF, USIZE_TY, USIZE_TY], ret_ty, true),
            );
            let _ = (
                b.param(STR_REF, o),
                b.param(USIZE_TY, o),
                b.param(USIZE_TY, o),
                b.param(USIZE_TY, o),
            );
            b.terminate(FuncBuilder::ENTRY, RTerm::Unreachable);
            b.finish()
        };
        let make_fail_item = |name: &str, crate_name: &str, debug_path: &str, path_id| {
            let mut item = link_item(name, Linkage::internal(), debug_path);
            item.crate_name = crate_name.to_owned();
            item.jir.origins = Some(rcgjs_object::jir::OriginTable {
                origins: Vec::new(),
                statements: Vec::new(),
                terminators: Vec::new(),
                locals: Vec::new(),
                item_path: path_id,
                root: None,
            });
            item
        };

        let lex_spoof_name = JsName::new("aaa_audit$str$slice_error_fail$h1".to_owned());
        let crate_spoof_name = JsName::new("core$str$slice_error_fail$h0_spoof".to_owned());
        let real_name = JsName::new("core$str$slice_error_fail$h999".to_owned());

        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("caller".to_owned()), caller_func);
        // Lexical spoof sorting before `core`.
        funcs.insert(lex_spoof_name.clone(), make_fail_fn(2, 1, STR_REF));
        // `--crate-name core` spoof matching `Never`, 4 params, `#[track_caller]`, `"core"` path.
        funcs.insert(crate_spoof_name.clone(), make_fail_fn(3, 2, NEVER_TY));

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("caller".to_owned()),
            link_item("caller", Linkage::exported("caller".to_owned()), "caller"),
        );
        table.insert(
            lex_spoof_name.clone(),
            make_fail_item(
                lex_spoof_name.as_str(),
                "aaa_audit",
                "str::slice_error_fail",
                fail_path,
            ),
        );
        table.insert(
            crate_spoof_name.clone(),
            make_fail_item(
                crate_spoof_name.as_str(),
                "core",
                "core::str::slice_error_fail",
                core_fail_path,
            ),
        );

        let roots = BTreeSet::from([JsName::new("caller".to_owned())]);
        // Without the authentic `core` item, both spoofs must be rejected.
        assert_eq!(run(&funcs, &table, &interns, &roots).slice_error_fail, None);

        // With the authentic `core` item present, it wins over both lexicographically earlier spoofs.
        funcs.insert(real_name.clone(), make_fail_fn(CORE_CRATE_ID, 3, NEVER_TY));
        table.insert(
            real_name.clone(),
            make_fail_item(
                real_name.as_str(),
                "core",
                "str::slice_error_fail",
                fail_path,
            ),
        );
        assert_eq!(
            run(&funcs, &table, &interns, &roots).slice_error_fail,
            Some(real_name.clone())
        );

        // Non-unique authentic `core` candidates must fall back to `None`.
        let dup_name = JsName::new("core$str$slice_error_fail$h999b".to_owned());
        funcs.insert(dup_name.clone(), make_fail_fn(CORE_CRATE_ID, 4, NEVER_TY));
        table.insert(
            dup_name.clone(),
            make_fail_item(
                dup_name.as_str(),
                "core",
                "str::slice_error_fail",
                fail_path,
            ),
        );
        assert_eq!(run(&funcs, &table, &interns, &roots).slice_error_fail, None);
    }

    #[test]
    fn owned_string_cells_result_enums_and_core_slice_auth() {
        let (mut interns, ascii_id, wide_id, _, helper_link) = test_interns();
        let o = OriginId(0);
        let native_str_ty = TypeId(20);
        let ref_native_str_ty = TypeId(21);
        let mut_ref_native_str_ty = TypeId(22);
        let res_str_ty = TypeId(23);
        let char_ty = TypeId(24);
        let u8_ty = TypeId(25);
        let slice_u8_ty = TypeId(26);
        let ref_slice_u8_ty = TypeId(27);
        let mut_slice_u8_ty = TypeId(28);
        let unit_ty = TypeId(29);

        let mut ns_desc = desc(
            native_str_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 20,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: Vec::new(),
                niche: None,
            },
        );
        ns_desc.meta.candidate = RepresentationCandidate::String;
        interns.types.insert(ns_desc);
        interns.types.insert(desc(
            ref_native_str_ty,
            TypeKind::Reference {
                pointee: native_str_ty,
                mutable: false,
            },
        ));
        interns.types.insert(desc(
            mut_ref_native_str_ty,
            TypeKind::Reference {
                pointee: native_str_ty,
                mutable: true,
            },
        ));
        let ok_name = interns.intern_str("Ok");
        let err_name = interns.intern_str("Err");
        interns.types.insert(desc(
            res_str_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 23,
                },
                args: vec![native_str_ty, STR_REF],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: ok_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: native_str_ty,
                            offset: 0,
                        }],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: true,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: err_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: STR_REF,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
        ));
        interns.types.insert(desc(char_ty, TypeKind::Char));
        let mut u8_desc = desc(
            u8_ty,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
        );
        u8_desc.layout.sized = true;
        u8_desc.layout.size = Some(1);
        interns.types.insert(u8_desc);
        interns
            .types
            .insert(desc(slice_u8_ty, TypeKind::Slice(u8_ty)));
        interns.types.insert(desc(
            ref_slice_u8_ty,
            TypeKind::Reference {
                pointee: slice_u8_ty,
                mutable: false,
            },
        ));
        interns.types.insert(desc(
            mut_slice_u8_ty,
            TypeKind::Reference {
                pointee: slice_u8_ty,
                mutable: true,
            },
        ));
        let mut unit_desc = desc(unit_ty, TypeKind::Tuple(Vec::new()));
        unit_desc.layout.zst = true;
        unit_desc.layout.sized = true;
        unit_desc.layout.size = Some(0);
        interns.types.insert(unit_desc);

        let build_link = interns.intern_str("build_ascii");
        let build_bad_link = interns.intern_str("build_bad");
        let rev_path = interns.intern_str("slice::<impl [T]>::reverse");
        let sw_path = interns.intern_str("slice::<impl [T]>::starts_with");

        // `build_ascii() -> Result<String, &str>`: creates String cell, pushes '-', pushes `(48 + (b & 7)) as char`,
        // pushes `"ascii"`, returns `Ok(load(cell))`.
        let mut b = FuncBuilder::new(item_id(50), sig(vec![u8_ty], res_str_ty, false));
        let byte_p = b.param(u8_ty, o);
        let cell = b.cell(native_str_ty, false, o);
        b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), o);
        let init = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(200),
                    link_name: helper_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrNew),
                host: None,
            },
            native_str_ty,
            o,
        );
        b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: init }, o);
        let mref1 = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: true,
            },
            mut_ref_native_str_ty,
            o,
        );
        let ch_dash = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar {
                bits: u128::from(b'-'),
                bytes: 4,
            }),
            char_ty,
            o,
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(201),
                    link_name: helper_link,
                },
                function: None,
                args: vec![mref1, ch_dash],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrPush),
                host: None,
            },
            o,
        );
        let mask7 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 1 }),
            u8_ty,
            o,
        );
        let masked = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::BitAnd,
                l: byte_p,
                r: mask7,
                ty: u8_ty,
            },
            u8_ty,
            o,
        );
        let zero_b = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar {
                bits: u128::from(b'0'),
                bytes: 1,
            }),
            u8_ty,
            o,
        );
        let digit_b = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: zero_b,
                r: masked,
                ty: u8_ty,
            },
            u8_ty,
            o,
        );
        let digit_ch = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: digit_b,
                from: u8_ty,
                to: char_ty,
            },
            char_ty,
            o,
        );
        let mref2 = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: true,
            },
            mut_ref_native_str_ty,
            o,
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(201),
                    link_name: helper_link,
                },
                function: None,
                args: vec![mref2, digit_ch],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrPush),
                host: None,
            },
            o,
        );
        let lit_ascii = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(ascii_id)),
            STR_REF,
            o,
        );
        let mref3 = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: true,
            },
            mut_ref_native_str_ty,
            o,
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(202),
                    link_name: helper_link,
                },
                function: None,
                args: vec![mref3, lit_ascii],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrPushStr),
                host: None,
            },
            o,
        );
        let loaded = b.op(FuncBuilder::ENTRY, OpKind::CellLoad(cell), native_str_ty, o);
        let ok_res = b.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: res_str_ty,
                variant: 0,
                active_field: None,
                fields: vec![loaded],
            },
            res_str_ty,
            o,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(ok_res)));
        let build_ascii_func = b.finish();

        // `build_bad()`: pushes a non-ASCII `"é😀"` -> MUST NOT be KnownAscii.
        let mut bb = FuncBuilder::new(item_id(51), sig(vec![], res_str_ty, false));
        let bcell = bb.cell(native_str_ty, false, o);
        let binit = bb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(200),
                    link_name: helper_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrNew),
                host: None,
            },
            native_str_ty,
            o,
        );
        bb.effect(
            FuncBuilder::ENTRY,
            OpKind::CellStore {
                cell: bcell,
                v: binit,
            },
            o,
        );
        let bmref = bb.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(bcell),
                path: Vec::new(),
                mutable: true,
            },
            mut_ref_native_str_ty,
            o,
        );
        let lit_wide = bb.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(wide_id)),
            STR_REF,
            o,
        );
        bb.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(202),
                    link_name: helper_link,
                },
                function: None,
                args: vec![bmref, lit_wide],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrPushStr),
                host: None,
            },
            o,
        );
        let bloaded = bb.op(
            FuncBuilder::ENTRY,
            OpKind::CellLoad(bcell),
            native_str_ty,
            o,
        );
        let bok = bb.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: res_str_ty,
                variant: 0,
                active_field: None,
                fields: vec![bloaded],
            },
            res_str_ty,
            o,
        );
        bb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(bok)));
        let build_bad_func = bb.finish();

        // `caller()`: unwraps `build_ascii` + `NativeStrAsStr` (KnownAscii!) vs `build_bad` (NOT KnownAscii!).
        let mut cb = FuncBuilder::new(item_id(52), sig(vec![u8_ty], STR_REF, false));
        let cp = cb.param(u8_ty, o);
        // Anchor real `core` crate ID via an authenticated `StrLen` call.
        let clit = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Str(ascii_id)),
            STR_REF,
            o,
        );
        cb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id_in(CORE_CRATE_ID, 300),
                    link_name: helper_link,
                },
                function: None,
                args: vec![clit],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StrLen),
                host: None,
            },
            USIZE_TY,
            o,
        );
        let good_res = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(50),
                    link_name: build_link,
                },
                function: None,
                args: vec![cp],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            res_str_ty,
            o,
        );
        let good_str_obj = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: good_res,
                variant: Some(0),
                field: 0,
            },
            native_str_ty,
            o,
        );
        let good_ref = cb.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(good_str_obj),
                path: Vec::new(),
                mutable: false,
            },
            ref_native_str_ty,
            o,
        );
        let good_as_str = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(203),
                    link_name: helper_link,
                },
                function: None,
                args: vec![good_ref],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrAsStr),
                host: None,
            },
            STR_REF,
            o,
        );
        let bad_res = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(51),
                    link_name: build_bad_link,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            res_str_ty,
            o,
        );
        let bad_str_obj = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: bad_res,
                variant: Some(0),
                field: 0,
            },
            native_str_ty,
            o,
        );
        let bad_ref = cb.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(bad_str_obj),
                path: Vec::new(),
                mutable: false,
            },
            ref_native_str_ty,
            o,
        );
        let bad_as_str = cb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(203),
                    link_name: helper_link,
                },
                function: None,
                args: vec![bad_ref],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeStrAsStr),
                host: None,
            },
            STR_REF,
            o,
        );
        cb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(good_as_str)));
        let caller_func = cb.finish();

        // Authenticated core::slice::reverse + starts_with, plus a spoofed crate's reverse.
        let real_rev_id = item_id_in(CORE_CRATE_ID, 401);
        let mut rev_b = FuncBuilder::new(real_rev_id, sig(vec![mut_slice_u8_ty], unit_ty, false));
        let _ = rev_b.param(mut_slice_u8_ty, o);
        rev_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let real_rev_func = rev_b.finish();

        let spoof_rev_id = item_id_in(999, 402);
        let mut srev_b = FuncBuilder::new(spoof_rev_id, sig(vec![mut_slice_u8_ty], unit_ty, false));
        let _ = srev_b.param(mut_slice_u8_ty, o);
        srev_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let spoof_rev_func = srev_b.finish();

        let real_sw_id = item_id_in(CORE_CRATE_ID, 403);
        let mut sw_b = FuncBuilder::new(
            real_sw_id,
            sig(vec![ref_slice_u8_ty, ref_slice_u8_ty], BOOL_TY, false),
        );
        let _ = (
            sw_b.param(ref_slice_u8_ty, o),
            sw_b.param(ref_slice_u8_ty, o),
        );
        let f_const = sw_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
            BOOL_TY,
            o,
        );
        sw_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(f_const)));
        let real_sw_func = sw_b.finish();

        let mk_core_item = |name: &str, crate_name: &str, dp: &str, path_id| {
            let mut item = link_item(name, Linkage::internal(), dp);
            item.crate_name = crate_name.to_owned();
            item.jir.origins = Some(rcgjs_object::jir::OriginTable {
                origins: Vec::new(),
                statements: Vec::new(),
                terminators: Vec::new(),
                locals: Vec::new(),
                item_path: path_id,
                root: None,
            });
            item
        };

        let mut funcs = BTreeMap::new();
        funcs.insert(JsName::new("build_ascii".to_owned()), build_ascii_func);
        funcs.insert(JsName::new("build_bad".to_owned()), build_bad_func);
        funcs.insert(JsName::new("caller".to_owned()), caller_func);
        funcs.insert(
            JsName::new("core$slice$impl_0$reverse$h1".to_owned()),
            real_rev_func,
        );
        funcs.insert(
            JsName::new("core$slice$impl_0$reverse$h_spoof".to_owned()),
            spoof_rev_func,
        );
        funcs.insert(
            JsName::new("core$slice$impl_0$starts_with$h1".to_owned()),
            real_sw_func,
        );

        let mut table = BTreeMap::new();
        table.insert(
            JsName::new("build_ascii".to_owned()),
            link_item("build_ascii", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("build_bad".to_owned()),
            link_item("build_bad", Linkage::internal(), ""),
        );
        table.insert(
            JsName::new("caller".to_owned()),
            link_item("caller", Linkage::exported("caller".to_owned()), ""),
        );
        table.insert(
            JsName::new("core$slice$impl_0$reverse$h1".to_owned()),
            mk_core_item(
                "core$slice$impl_0$reverse$h1",
                "core",
                "slice::<impl [T]>::reverse",
                rev_path,
            ),
        );
        table.insert(
            JsName::new("core$slice$impl_0$reverse$h_spoof".to_owned()),
            mk_core_item(
                "core$slice$impl_0$reverse$h_spoof",
                "core",
                "slice::<impl [T]>::reverse",
                rev_path,
            ),
        );
        table.insert(
            JsName::new("core$slice$impl_0$starts_with$h1".to_owned()),
            mk_core_item(
                "core$slice$impl_0$starts_with$h1",
                "core",
                "slice::<impl [T]>::starts_with",
                sw_path,
            ),
        );

        let roots = BTreeSet::from([JsName::new("caller".to_owned())]);
        let facts = run(&funcs, &table, &interns, &roots);
        let cf = facts.for_item(&JsName::new("caller".to_owned()));
        assert!(cf.is_known_ascii(good_as_str));
        assert!(!cf.is_known_ascii(bad_as_str));
        assert!(!cf.is_known_ascii(good_str_obj)); // OwnedString never directly published
        assert!(cf.is_slice_reverse(real_rev_id));
        assert!(!cf.is_slice_reverse(spoof_rev_id));
        assert!(cf.is_slice_starts_with(real_sw_id));
    }
}
