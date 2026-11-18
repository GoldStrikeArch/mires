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
use crate::jir::repr::{Carrier, Func, OpKind, Proj, RTerm, Root, ValueId};
use crate::jir::semantic::{
    CallSemantic, Callee, ConstTree, HostPayload, ItemId, StandardOperation, TypeId,
};
use crate::jir::types::{TypeKind, TypeTable, VariantDescriptor};
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
}

/// Borrowed slice of [`AsciiFacts`] for one function being lowered.
#[derive(Clone, Copy, Debug, Default)]
pub struct FuncAsciiFacts<'a> {
    pub known: Option<&'a BTreeSet<ValueId>>,
    pub slice_error_fail: Option<&'a str>,
}

impl AsciiFacts {
    pub fn for_item(&self, name: &JsName) -> FuncAsciiFacts<'_> {
        FuncAsciiFacts {
            known: self.funcs.get(name),
            slice_error_fail: self.slice_error_fail.as_ref().map(|name| name.as_str()),
        }
    }
}

impl<'a> FuncAsciiFacts<'a> {
    pub fn is_known_ascii(&self, value: ValueId) -> bool {
        self.known.is_some_and(|set| set.contains(&value))
    }
}

/// What ASCII-trackable value shape an SSA value has in JIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AsciiType {
    /// Shared string slice `&str` (peeling transparent wrappers).
    SharedStr,
    /// `Option<&str>` with known `(none_variant, some_variant)` indices (tracked only to preserve
    /// ASCII across `StrGet`/`StrStrip*` -> `Option::Some` unwrapping; never exported directly).
    OptionStr { none: u32, some: u32 },
    /// Owned `String` lowered as `Carrier::HostString` (byte string holding UTF-8 code units).
    HostString,
    /// Any other type/carrier (`MaybeNonAscii` always).
    Other,
}

#[derive(Clone, Debug)]
enum Rule {
    Never,
    Root,
    Single(ValueId),
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

    // Locate `core::str::slice_error_fail` if live and keeping its standard 4-arg ABI.
    let slice_error_fail = table.iter().find_map(|(name, item)| {
        if item.debug_path != "str::slice_error_fail"
            && item.debug_path != "core::str::slice_error_fail"
        {
            return None;
        }
        if !defined.contains(name) || !live.contains(name) || repr.bodies.contains_key(name) {
            return None;
        }
        let func = funcs.get(name)?;
        (func.sig.abi.caller_location
            && func.params.len() == 4
            && abi.call_signature(func.item).is_none())
        .then(|| name.clone())
    });
    if let Some(fail_name) = &slice_error_fail {
        // Outlined `StrIndex` jumps directly to `slice_error_fail` without modifying JIR calls.
        open_names.insert(fail_name.clone());
    }

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
            };
        };
        let type_table = &interns.types;
        let disqualified = collect_disqualified(func);
        let val_types: Vec<AsciiType> = func
            .values
            .iter()
            .map(|v| classify_value_type(type_table, interns, v.ty, v.carrier))
            .collect();

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
                            let expected =
                                args.len() + usize::from(callee.sig.abi.caller_location);
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
                let rule = match &op.kind {
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
                        AsciiType::OptionStr { none, some } => {
                            if *variant == none && fields.is_empty() {
                                Rule::Root
                            } else if *variant == some
                                && fields.len() == 1
                                && val_types[fields[0].0 as usize] == AsciiType::SharedStr
                            {
                                Rule::Single(fields[0])
                            } else {
                                Rule::Never
                            }
                        }
                        _ => Rule::Never,
                    },
                    OpKind::Field {
                        base,
                        variant: Some(variant),
                        field: 0,
                    } if r_ty == AsciiType::SharedStr && !disqualified.contains(base) => {
                        match val_types[base.0 as usize] {
                            AsciiType::OptionStr { some, .. } if *variant == some => {
                                Rule::Single(*base)
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
                        r_ty,
                        *semantic,
                        callee,
                        args,
                        &val_types,
                        interns,
                        repr,
                        abi,
                        &name_to_idx,
                    ),
                    _ => Rule::Never,
                };
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
            if callers
                .iter()
                .all(|&(c_idx, arg)| states_pre_match(&names, funcs, c_idx, arg, val_ty, &get_interns))
            {
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
                    Rule::BlockParam(incoming) => {
                        !incoming.is_empty()
                            && incoming.iter().all(|&src| states[idx].ascii[src.0 as usize])
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
    matches!(
        (actual, expected),
        (AsciiType::SharedStr, AsciiType::SharedStr)
            | (AsciiType::HostString, AsciiType::HostString)
            | (AsciiType::OptionStr { .. }, AsciiType::OptionStr { .. })
    )
}

#[allow(clippy::too_many_arguments)]
fn classify_call_result(
    caller_name: &JsName,
    r_ty: AsciiType,
    semantic: CallSemantic,
    callee: &Callee,
    args: &[ValueId],
    val_types: &[AsciiType],
    interns: &Interns,
    repr: &ReprOutcome,
    abi: &AbiPlan,
    name_to_idx: &BTreeMap<JsName, usize>,
) -> Rule {
    if let CallSemantic::Standard(std_op) = semantic {
        match std_op {
            StandardOperation::IntToString
            | StandardOperation::BoolToString
            | StandardOperation::F64SpecToString
                if r_ty == AsciiType::HostString =>
            {
                return Rule::Root;
            }
            StandardOperation::StrIndex
            | StandardOperation::StrTrim
            | StandardOperation::StrTrimStart
            | StandardOperation::StrTrimEnd
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
                if matches!(r_ty, AsciiType::OptionStr { .. }) =>
            {
                if let Some(&recv) = args.first() {
                    if val_types[recv.0 as usize] == AsciiType::SharedStr {
                        return Rule::Single(recv);
                    }
                }
                return Rule::Never;
            }
            // Explicit safeguard: `String::as_str`, `char::to_string`, `format!` remain unproven.
            StandardOperation::StringAsStr
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

fn collect_disqualified(func: &Func) -> BTreeSet<ValueId> {
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
                }
                | OpKind::RawView {
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

fn classify_value_type(
    table: &TypeTable,
    interns: &Interns,
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
    let peeled = table.peeled(ty);
    let variants = table.variants(peeled);
    if variants.len() == 2 {
        let variant_name = |v: &VariantDescriptor| interns.str(v.name).unwrap_or("");
        let none = variants.iter().position(|v| variant_name(v) == "None");
        let some = variants.iter().position(|v| variant_name(v) == "Some");
        if let (Some(none), Some(some)) = (none, some) {
            if variants[none].fields.is_empty()
                && variants[some].fields.len() == 1
                && is_shared_str_ref(table, variants[some].fields[0].ty)
            {
                return AsciiType::OptionStr {
                    none: none as u32,
                    some: some as u32,
                };
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
        AsciiType::OptionStr { none, some } => match tree {
            ConstTree::Variant { variant, fields } if *variant == none => fields.is_empty(),
            ConstTree::Variant { variant, fields } if *variant == some => {
                fields.len() == 1 && is_ascii_str_const(interns, &fields[0])
            }
            _ => false,
        },
        AsciiType::HostString | AsciiType::Other => false,
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
                bits: 64,
            },
        ));
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
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
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
        let as_str = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_id(91),
                    link_name: helper_link,
                },
                function: None,
                args: vec![int_str],
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

        // Also test live `core::str::slice_error_fail` detection.
        let mut fail_b = FuncBuilder::new(item_id(10), sig(vec![STR_REF, USIZE_TY, USIZE_TY], STR_REF, true));
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
            JsName::new("core$str$slice_error_fail$h123".to_owned()),
            link_item(
                "core$str$slice_error_fail$h123",
                Linkage::internal(),
                "str::slice_error_fail",
            ),
        );
        let roots = BTreeSet::from([JsName::new("caller".to_owned())]);
        let facts = run(&funcs, &table, &interns, &roots);

        let caller_facts = facts.for_item(&JsName::new("caller".to_owned()));
        assert!(caller_facts.is_known_ascii(lit_ascii));
        assert!(!caller_facts.is_known_ascii(lit_wide));
        assert!(caller_facts.is_known_ascii(int_str));
        assert!(!caller_facts.is_known_ascii(as_str)); // String::as_str must be MaybeNonAscii
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
}
