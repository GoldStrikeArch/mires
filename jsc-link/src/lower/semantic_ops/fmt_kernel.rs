//! The call-site format kernel (Phase 4, W4g): the recognition and inline emission of proven
//! `format_args!` sites, ported from v1 `native_kernel.rs`'s format pipeline.
//!
//! A *site* is a consumer call — the linked `alloc::fmt::format` item
//! ([`StandardOperation::FormatArgsToString`]) or the exact `String::write_fmt` op
//! ([`StandardOperation::StringFmtWriteFmt`]) — whose `Arguments` operand is built in the same
//! function by the linked `Arguments::new` item from a constant template and an array of linked
//! `Argument::new_display` calls over proven default-`Display` values. Identity is exact, never
//! shape: every callee resolves through the linked body's `BodySemantic::Standard` tag
//! ([`analyze`]'s resolver), so a user function with the same signature is never mistaken for
//! format machinery (v1 `OPTIMIZER.md`: "the kernel inlines a format site only on exact
//! identity").
//!
//! The planner ([`crate::plan::representation`]) runs [`analyze`] over each pre-commit `Func`:
//! a proven site frees the consumer's edges to the machinery items, prices the machinery the
//! commit would drop, and — when the destination labels native — **marks** the site in band by
//! committing carriers onto the constructor calls' results (`Arguments::new` → the concatenated
//! [`Carrier::HostString`]; each `Argument::new_display` → the argument's host text, at
//! [`Carrier::StringBuilder`] when the display source is a committed builder; each single-use
//! rename between those results and their consumers, [`Site::renames`], → the mark of the value
//! it renames). The marks travel
//! with the `Func` through phase C's optimizer, so [`intercept`] — the emission hook in
//! `lower/js/call.rs` — needs no side table: a marked constructor call lowers to its text, the
//! consumer call to the concatenation in the native `String` layout, and an *unmarked* machinery
//! call stays a plain call to the baseline item. A marked call the emitter can no longer prove
//! is a `Decline`, never a generic emission — the lane retry then prints the baseline program.
//!
//! v1 semantics preserved exactly: a producer call feeding a template keeps its emission (only
//! the constructor apparatus is recognized, and every recognized value must be used exactly once
//! along the spine, so a value the rest of the function reads keeps its baseline meaning); a
//! `String` display argument needs a planned native representation, not a runtime test; an
//! identity-proven `format!` machinery call the pipeline cannot prove declines the whole item's
//! format commit ([`Analysis::sites`] comes back empty), so nothing is elided out from under it.

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::interns::Interns;
use crate::jir::repr::{Carrier, Func, Op, OpKind, Proj, Root, ValueId};
use crate::jir::semantic::{CallSemantic, Callee, ConstTree, ItemId, StandardOperation};
use crate::jir::types::{RepresentationCandidate, TypeId, TypeKind, TypeTable};
use crate::jsast::{self, BinOp, Expr};

/// The operations the kernel consumes or elides at a call site.
pub(crate) fn is_format_machinery(operation: StandardOperation) -> bool {
    matches!(
        operation,
        StandardOperation::FormatArgsNew
            | StandardOperation::FormatArgumentNewDisplay
            | StandardOperation::FormatArgsToString
            | StandardOperation::StringFmtWriteFmt
    )
}

/// The constructor/consumer identities whose *unproven* appearance declines the item's format
/// commit (v1: "a format-machinery call the pipeline did not prove must not emit generically").
/// An unproven `String::write_fmt` call stays an ordinary call, exactly as v1 left it.
fn declines_item(operation: StandardOperation) -> bool {
    matches!(
        operation,
        StandardOperation::FormatArgsNew
            | StandardOperation::FormatArgumentNewDisplay
            | StandardOperation::FormatArgsToString
    )
}

/// The default-Display argument shapes the pipeline recognizes (v1 `FormatDisplayKind`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplayKind {
    Integer,
    Bool,
    Char,
    Str,
    F64,
    String,
}

/// One display argument of a site.
#[derive(Clone, Debug)]
pub(crate) struct DisplayArg {
    pub kind: DisplayKind,
    /// The `Argument::new_display` call's result: the site's text binding.
    pub value: ValueId,
    /// The place the display value is loaded from, all further derefs appended.
    pub root: Root,
    pub path: Vec<Proj>,
}

/// What the consumer call is, and the value its native labeling is judged by.
#[derive(Clone, Debug)]
pub(crate) enum SiteKind {
    /// `alloc::fmt::format(args) -> String`: `result` is the destination value.
    ToString { result: ValueId },
    /// `String::write_fmt(&mut s, args)`: `receiver` is the `&mut String` argument value.
    WriteFmt {
        receiver: ValueId,
        result: Option<ValueId>,
    },
}

/// One proven format site.
#[derive(Clone, Debug)]
pub(crate) struct Site {
    pub kind: SiteKind,
    /// `(block index, op index)` of the consumer call.
    pub consumer: (usize, usize),
    /// `(block index, op index)` of the `Arguments::new` call.
    pub new_call: (usize, usize),
    /// The `Arguments::new` call's result.
    pub args_value: ValueId,
    /// `(block index, op index)` of each `Argument::new_display` call.
    pub display_calls: Vec<(usize, usize)>,
    /// The single-use `Copy`/`Move` renames between a constructor call's result and the operand
    /// that consumes it, each beside the result it renames: the `Arguments` operand of the
    /// consumer back to `args_value`, and each argument-array element back to its
    /// `new_display` result. The runtime MIR `js-cleanup=on` prepares (rustc-cleanup: drop
    /// elaboration without the optimization passes) keeps them where optimized MIR's copy
    /// propagation would not; the commit gives each the mark of the value it renames, so the
    /// whole spine carries the native text the use-once check promised.
    pub renames: Vec<(ValueId, ValueId)>,
    /// The static template pieces; always `args.len() + 1` of them.
    pub pieces: Vec<String>,
    pub args: Vec<DisplayArg>,
    /// The linked machinery items the site's calls resolve to.
    pub callees: BTreeSet<ItemId>,
}

/// Every proven site of one function. Empty when the item's format commit declines.
#[derive(Debug, Default)]
pub(crate) struct Analysis {
    pub sites: Vec<Site>,
    /// The machinery items every identified call to which is part of a proven site — the
    /// references a committed rewrite stops printing.
    pub elidable: BTreeSet<ItemId>,
}

/// A read-only view of one function for the def chases, shared by the planner and the emitter.
pub(crate) struct View<'a> {
    pub func: &'a Func,
    pub table: &'a TypeTable,
    pub interns: &'a Interns,
    defs: BTreeMap<ValueId, (usize, usize)>,
}

impl<'a> View<'a> {
    pub fn new(func: &'a Func, table: &'a TypeTable, interns: &'a Interns) -> View<'a> {
        let mut defs = BTreeMap::new();
        for (b, block) in func.blocks.iter().enumerate() {
            for (i, op) in block.ops.iter().enumerate() {
                if let Some(result) = op.result {
                    defs.insert(result, (b, i));
                }
            }
        }
        View {
            func,
            table,
            interns,
            defs,
        }
    }

    fn def(&self, v: ValueId) -> Option<&'a Op> {
        let (b, i) = *self.defs.get(&v)?;
        Some(&self.func.blocks[b].ops[i])
    }

    fn def_at(&self, v: ValueId) -> Option<(usize, usize)> {
        self.defs.get(&v).copied()
    }

    fn ty(&self, v: ValueId) -> Option<TypeId> {
        self.func.ty(v)
    }

    fn kind_of(&self, ty: TypeId) -> Option<&'a TypeKind> {
        self.table.get(ty).map(|descriptor| &descriptor.kind)
    }

    /// Through `Copy`/`Move` renames to the value they read.
    fn chase_copies(&self, v: ValueId) -> ValueId {
        let mut at = v;
        loop {
            match self.def(at).map(|op| &op.kind) {
                Some(OpKind::Copy(inner) | OpKind::Move(inner)) => at = *inner,
                _ => return at,
            }
        }
    }

    /// Through `Copy`/`Move` and `Field`-of-`Aggregate` (the transparent `Argument` wrapper) to
    /// the underlying value.
    fn chase_ref(&self, v: ValueId) -> ValueId {
        let mut at = v;
        loop {
            match self.def(at).map(|op| &op.kind) {
                Some(OpKind::Copy(inner) | OpKind::Move(inner)) => at = *inner,
                Some(OpKind::Field {
                    base,
                    variant: None,
                    field,
                }) => {
                    let base = self.chase_copies(*base);
                    match self.def(base).map(|op| &op.kind) {
                        Some(OpKind::Aggregate {
                            fields,
                            active_field: None,
                            ..
                        }) if (*field as usize) < fields.len() => at = fields[*field as usize],
                        _ => return at,
                    }
                }
                _ => return at,
            }
        }
    }

    /// The constant bytes a template operand bottoms out in (v1 `follow_constant_bytes`).
    fn const_bytes(&self, v: ValueId) -> Option<Vec<u8>> {
        match self.def(v).map(|op| &op.kind)? {
            OpKind::Const(tree) => tree_bytes(tree, self.interns),
            OpKind::Copy(inner) | OpKind::Move(inner) => self.const_bytes(*inner),
            OpKind::BorrowView {
                root: Root::Value(inner),
                path,
                ..
            } if is_borrow_or_reborrow(path) => self.const_bytes(*inner),
            OpKind::Aggregate { fields, .. } => fields
                .iter()
                .map(|field| match self.def(*field).map(|op| &op.kind) {
                    Some(OpKind::Const(ConstTree::Scalar { bits, bytes: 1 })) => {
                        u8::try_from(*bits).ok()
                    }
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }

    /// The element values of the argument-list operand: `&[new_display; N]` chased to its array
    /// aggregate (v1 `follow_array_elements`). Returns the chain values crossed, for the
    /// use-once check.
    fn list_elements(&self, v: ValueId) -> Option<(Vec<ValueId>, Vec<ValueId>)> {
        let mut spine = Vec::new();
        let mut at = v;
        loop {
            spine.push(at);
            match self.def(at).map(|op| &op.kind)? {
                OpKind::Copy(inner) | OpKind::Move(inner) => at = *inner,
                OpKind::BorrowView {
                    root: Root::Value(inner),
                    path,
                    ..
                } if is_borrow_or_reborrow(path) => at = *inner,
                OpKind::Aggregate { fields, .. } => return Some((fields.clone(), spine)),
                OpKind::Const(tree) => {
                    // The empty argument list of `format!("literal")` is a constant.
                    let empty = match tree {
                        ConstTree::Aggregate(elements) | ConstTree::Slice(elements) => {
                            elements.is_empty()
                        }
                        ConstTree::Reference(inner) => {
                            matches!(
                                inner.as_ref(),
                                ConstTree::Aggregate(e) | ConstTree::Slice(e) if e.is_empty()
                            ) || matches!(inner.as_ref(), ConstTree::Unit)
                        }
                        ConstTree::Unit => true,
                        _ => false,
                    };
                    return empty.then(|| (Vec::new(), spine));
                }
                _ => return None,
            }
        }
    }

    /// The display source of a `new_display` reference operand: the borrowed place, extended by
    /// a deref per intermediate reference, and the proven display kind (v1
    /// `loaded_display_source` + `format_display_kind`).
    fn display_source(&self, reference: ValueId) -> Option<(Root, Vec<Proj>, DisplayKind)> {
        let reference = self.chase_ref(reference);
        let TypeKind::Reference {
            pointee,
            mutable: false,
        } = *self.kind_of(self.ty(reference)?)?
        else {
            return None;
        };
        let (root, mut path) = match self.def(reference).map(|op| &op.kind)? {
            OpKind::BorrowView {
                root,
                path,
                mutable: false,
            } => (*root, path.clone()),
            _ => return None,
        };
        let mut ty = pointee;
        loop {
            let descriptor = self.table.get(ty)?;
            match &descriptor.kind {
                TypeKind::Integer {
                    bits: 8 | 16 | 32 | 64 | 128,
                    ..
                } => {
                    return Some((root, path, DisplayKind::Integer));
                }
                TypeKind::Bool => return Some((root, path, DisplayKind::Bool)),
                TypeKind::Char => return Some((root, path, DisplayKind::Char)),
                TypeKind::Float { bits: 64 } => return Some((root, path, DisplayKind::F64)),
                // The place holds the `str` (or the `&str` one deref up): reading it is the host
                // string either way, so the walk stops without the last deref.
                TypeKind::Str => return Some((root, path, DisplayKind::Str)),
                TypeKind::Reference {
                    pointee: inner,
                    mutable: false,
                } => {
                    if matches!(self.kind_of(*inner)?, TypeKind::Str) {
                        return Some((root, path, DisplayKind::Str));
                    }
                    path.push(Proj::Deref);
                    ty = *inner;
                }
                TypeKind::Adt { .. }
                    if descriptor.meta.candidate == RepresentationCandidate::String =>
                {
                    return Some((root, path, DisplayKind::String));
                }
                _ => return None,
            }
        }
    }
}

/// A borrow path the template and argument-list chases see through: `&v` itself (`[]`), or the
/// reborrow `&*v` of a reference `v` (`[Deref]`), which names exactly what `v` already points at.
/// Optimized MIR folds the reborrow away (`InstSimplify`); the runtime MIR `js-cleanup=on`
/// prepares without the optimization passes keeps it, so a cleanup build's `Arguments::new`
/// receives its template as `&*template`.
fn is_borrow_or_reborrow(path: &[Proj]) -> bool {
    matches!(path, [] | [Proj::Deref])
}

/// [`ConstTree`] to raw bytes, references peeled (v1 `const_tree_bytes`).
fn tree_bytes(tree: &ConstTree, interns: &Interns) -> Option<Vec<u8>> {
    match tree {
        ConstTree::Reference(inner) => tree_bytes(inner, interns),
        ConstTree::Scalar { bits, bytes: 1 } => Some(vec![u8::try_from(*bits).ok()?]),
        ConstTree::Str(id) => Some(interns.str(*id)?.as_bytes().to_vec()),
        ConstTree::Aggregate(elements) | ConstTree::Slice(elements) => {
            let mut bytes = Vec::with_capacity(elements.len());
            for element in elements {
                bytes.extend(tree_bytes(element, interns)?);
            }
            Some(bytes)
        }
        ConstTree::Unit => Some(Vec::new()),
        _ => None,
    }
}

/// The default (`{}`/`{name}`) `format_args!` template byte encoding: literal runs
/// length-prefixed (`0x80` for a two-byte length), `0xC0` a placeholder for the next argument in
/// order, `0x00` the terminator. Anything else — width, precision, `{0}` reuse, `Debug` — is not
/// a default template and declines (v1 `decode_default_format_template`, byte for byte).
pub(crate) fn decode_template(bytes: &[u8]) -> Option<Vec<String>> {
    let mut pieces = vec![String::new()];
    let mut cursor = 0;
    loop {
        let header = *bytes.get(cursor)?;
        cursor += 1;
        match header {
            0 => return (cursor == bytes.len()).then_some(pieces),
            1..=0x7f => {
                let end = cursor
                    .checked_add(usize::from(header))
                    .filter(|end| *end <= bytes.len())?;
                let text = std::str::from_utf8(&bytes[cursor..end]).ok()?;
                pieces
                    .last_mut()
                    .expect("template starts with a piece")
                    .push_str(text);
                cursor = end;
            }
            0x80 => {
                let length = bytes
                    .get(cursor..cursor + 2)
                    .map(|length| u16::from_le_bytes([length[0], length[1]]) as usize)?;
                cursor += 2;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())?;
                let text = std::str::from_utf8(&bytes[cursor..end]).ok()?;
                pieces
                    .last_mut()
                    .expect("template starts with a piece")
                    .push_str(text);
                cursor = end;
            }
            0xc0 => pieces.push(String::new()),
            _ => return None,
        }
    }
}

// -- plan-time recognition ---------------------------------------------------------------------

/// Recognizes every proven format site of `func`. `resolve` answers the exact linked identity of
/// a direct callee — the callee body's `BodySemantic::Standard` tag — and nothing else.
///
/// The decline-whole-item rule: if any identity-proven `format!` machinery call is not part of a
/// proven site, the whole item's format commit declines (empty `sites`), so no site of it is
/// ever inlined while an unproven call still reads the constructors.
pub(crate) fn analyze(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    resolve: &dyn Fn(&ItemId) -> Option<StandardOperation>,
) -> Analysis {
    let view = View::new(func, table, interns);
    let uses = use_counts(func);
    let used_once = |v: ValueId| uses.get(&v).copied().unwrap_or(0) == 1;

    // Every identity-proven machinery call, by identity.
    let mut machinery: BTreeMap<(usize, usize), (StandardOperation, ItemId)> = BTreeMap::new();
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, op) in block.ops.iter().enumerate() {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                ..
            } = &op.kind
            else {
                continue;
            };
            if let Some(operation) = resolve(item).filter(|op| is_format_machinery(*op)) {
                machinery.insert((b, i), (operation, *item));
            }
        }
    }

    let mut sites = Vec::new();
    let mut proven: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (&position, &(operation, item)) in &machinery {
        let consumer_kind = match operation {
            StandardOperation::FormatArgsToString => 0,
            StandardOperation::StringFmtWriteFmt => 1,
            _ => continue,
        };
        let op = &func.blocks[position.0].ops[position.1];
        let OpKind::Call { args, .. } = &op.kind else {
            continue;
        };
        let (kind, arguments_operand) = if consumer_kind == 0 {
            let (Some(result), [arguments]) = (op.result, args.as_slice()) else {
                continue;
            };
            (SiteKind::ToString { result }, *arguments)
        } else {
            let [receiver, arguments] = args.as_slice() else {
                continue;
            };
            (
                SiteKind::WriteFmt {
                    receiver: *receiver,
                    result: op.result,
                },
                *arguments,
            )
        };
        let Some(mut site) = prove_site(&view, &machinery, &used_once, kind, arguments_operand)
        else {
            continue;
        };
        site.consumer = position;
        site.callees.insert(item);
        proven.insert(position);
        proven.insert(site.new_call);
        proven.extend(site.display_calls.iter().copied());
        sites.push(site);
    }

    // The decline-whole-item rule, plus which machinery items the committed rewrite may stop
    // referencing: those every identified call to which is proven.
    let mut call_counts: BTreeMap<ItemId, (u32, u32)> = BTreeMap::new();
    let mut declined = false;
    for (position, (operation, item)) in &machinery {
        let entry = call_counts.entry(*item).or_default();
        entry.0 += 1;
        if proven.contains(position) {
            entry.1 += 1;
        } else if declines_item(*operation) {
            declined = true;
        }
    }
    if declined {
        return Analysis::default();
    }
    // A machinery item mentioned other than by a proven direct call (a function constant, an
    // intrinsic fallback) keeps its reference and its baseline meaning.
    let machinery_items: BTreeSet<ItemId> = machinery.values().map(|(_, item)| *item).collect();
    let named = named_outside_calls(func, &machinery_items);
    let elidable = call_counts
        .iter()
        .filter(|(item, (all, ok))| all == ok && !named.contains(*item))
        .map(|(item, _)| *item)
        .collect();
    Analysis { sites, elidable }
}

/// Every value's use count: op operands plus terminator/edge arguments.
fn use_counts(func: &Func) -> BTreeMap<ValueId, u32> {
    let mut uses: BTreeMap<ValueId, u32> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            for v in op.kind.uses() {
                *uses.entry(v).or_default() += 1;
            }
        }
        for v in block.term.uses() {
            *uses.entry(v).or_default() += 1;
        }
    }
    uses
}

/// The machinery items a non-call op names (a function constant): never elidable.
fn named_outside_calls(func: &Func, machinery: &BTreeSet<ItemId>) -> BTreeSet<ItemId> {
    fn tree(out: &mut BTreeSet<ItemId>, machinery: &BTreeSet<ItemId>, node: &ConstTree) {
        match node {
            ConstTree::Global {
                target: crate::jir::semantic::GlobalReference::Function { item, .. },
                ..
            } if machinery.contains(item) => {
                out.insert(*item);
            }
            ConstTree::Reference(inner) => tree(out, machinery, inner),
            ConstTree::Aggregate(elements) | ConstTree::Slice(elements) => {
                for element in elements {
                    tree(out, machinery, element);
                }
            }
            ConstTree::Variant { fields, .. } => {
                for field in fields {
                    tree(out, machinery, field);
                }
            }
            _ => {}
        }
    }
    let mut named = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Const(node) => tree(&mut named, machinery, node),
                OpKind::Intrinsic {
                    fallback: Some(fallback),
                    ..
                } if machinery.contains(&fallback.item) => {
                    named.insert(fallback.item);
                }
                _ => {}
            }
        }
    }
    named
}

/// Proves one consumer's `Arguments` construction (v1 `native_format_text` +
/// `collect_format_site`). `consumer` and `callees` are filled by the caller.
fn prove_site(
    view: &View<'_>,
    machinery: &BTreeMap<(usize, usize), (StandardOperation, ItemId)>,
    used_once: &dyn Fn(ValueId) -> bool,
    kind: SiteKind,
    arguments: ValueId,
) -> Option<Site> {
    // The Arguments operand, through single-use renames to the `Arguments::new` call.
    let mut spine = vec![arguments];
    let mut chain = Vec::new();
    let mut at = arguments;
    let args_value = loop {
        match view.def(at).map(|op| &op.kind) {
            Some(OpKind::Copy(inner) | OpKind::Move(inner)) => {
                chain.push(at);
                at = *inner;
                spine.push(at);
            }
            Some(OpKind::Call { .. }) => break at,
            _ => return None,
        }
    };
    let mut renames: Vec<(ValueId, ValueId)> = chain
        .into_iter()
        .map(|rename| (rename, args_value))
        .collect();
    let new_call = view.def_at(args_value)?;
    let (StandardOperation::FormatArgsNew, new_item) = *machinery.get(&new_call)? else {
        return None;
    };
    let OpKind::Call { args: new_args, .. } = &view.func.blocks[new_call.0].ops[new_call.1].kind
    else {
        return None;
    };
    let [template, list] = new_args.as_slice() else {
        return None;
    };

    let pieces = decode_template(&view.const_bytes(*template)?)?;
    let (elements, list_spine) = view.list_elements(*list)?;
    spine.extend(list_spine);

    let mut args = Vec::with_capacity(elements.len());
    let mut display_calls = Vec::with_capacity(elements.len());
    let mut callees: BTreeSet<ItemId> = BTreeSet::new();
    callees.insert(new_item);
    for element in &elements {
        let mut at = *element;
        spine.push(at);
        let mut chain = Vec::new();
        let value = loop {
            match view.def(at).map(|op| &op.kind) {
                Some(OpKind::Copy(inner) | OpKind::Move(inner)) => {
                    chain.push(at);
                    at = *inner;
                    spine.push(at);
                }
                Some(OpKind::Call { .. }) => break at,
                _ => return None,
            }
        };
        renames.extend(chain.into_iter().map(|rename| (rename, value)));
        let display_call = view.def_at(value)?;
        let (StandardOperation::FormatArgumentNewDisplay, display_item) =
            *machinery.get(&display_call)?
        else {
            return None;
        };
        let OpKind::Call {
            args: display_args, ..
        } = &view.func.blocks[display_call.0].ops[display_call.1].kind
        else {
            return None;
        };
        let [reference] = display_args.as_slice() else {
            return None;
        };
        let (root, path, kind) = view.display_source(*reference)?;
        args.push(DisplayArg {
            kind,
            value,
            root,
            path,
        });
        display_calls.push(display_call);
        callees.insert(display_item);
    }
    if pieces.len() != args.len() + 1 {
        return None;
    }
    // Every spine value carries a native text after the commit, so anything else reading one
    // would read the wrong shape: each must be used exactly once, along the spine.
    if !spine.iter().all(|v| used_once(*v)) {
        return None;
    }
    // The write_fmt receiver must be the exact `&mut String` the builder contract expects.
    if let SiteKind::WriteFmt { receiver, .. } = kind {
        let TypeKind::Reference {
            pointee,
            mutable: true,
        } = *view.kind_of(view.ty(receiver)?)?
        else {
            return None;
        };
        let descriptor = view.table.get(pointee)?;
        if descriptor.meta.candidate != RepresentationCandidate::String {
            return None;
        }
    }
    Some(Site {
        kind,
        consumer: (0, 0),
        new_call,
        args_value,
        display_calls,
        renames,
        pieces,
        args,
        callees,
    })
}

// -- emit-time inlining ------------------------------------------------------------------------

use crate::lower::js::call::Lowered;
use crate::lower::js::{decline, JsCx, Res};
use rcgjs_object::jir::decline::OpClass;

/// The kernel's emission hook (`lower/js/call.rs`): the inline lowering of a marked format call,
/// `None` for every call the plan did not commit.
///
/// A mark is a committed non-`Baseline` carrier on a constructor call's result — a shape only
/// [`crate::plan::representation`] produces, and only on identity-proven sites — so `off`,
/// `report` and an uncommitted `on` run never reach past the identity check here.
pub(crate) fn intercept(cx: &JsCx<'_>, op: &Op) -> Res<Option<Lowered>> {
    let OpKind::Call {
        callee,
        args,
        semantic,
        ..
    } = &op.kind
    else {
        return Ok(None);
    };
    let operation = match callee {
        Callee::Direct { item, .. } => match cx.items.get(item).and_then(|entry| entry.standard) {
            Some(operation) if is_format_machinery(operation) => operation,
            // The index may not know the item (a driverless lowering); the write_fmt identity is
            // also on the call itself.
            _ => match semantic {
                CallSemantic::Standard(StandardOperation::StringFmtWriteFmt) => {
                    StandardOperation::StringFmtWriteFmt
                }
                _ => return Ok(None),
            },
        },
        _ => return Ok(None),
    };
    let marked = |v: ValueId| cx.func.values[v.0 as usize].carrier != Carrier::Baseline;
    let view = View::new(cx.func, cx.table, cx.interns);
    match operation {
        StandardOperation::FormatArgumentNewDisplay => {
            let Some(result) = op.result.filter(|result| marked(*result)) else {
                return Ok(None);
            };
            let [reference] = args.as_slice() else {
                return Err(kernel_decline("marked new_display arity"));
            };
            let Some((root, path, kind)) = view.display_source(*reference) else {
                return Err(kernel_decline("marked new_display has no proven source"));
            };
            let text = display_text(
                cx,
                root,
                &path,
                kind,
                cx.func.values[result.0 as usize].carrier,
            )?;
            Ok(Some(Lowered::value(text)))
        }
        StandardOperation::FormatArgsNew => {
            let Some(result) = op.result.filter(|result| marked(*result)) else {
                return Ok(None);
            };
            let _ = result;
            let [template, list] = args.as_slice() else {
                return Err(kernel_decline("marked Arguments::new arity"));
            };
            let pieces = view
                .const_bytes(*template)
                .as_deref()
                .and_then(decode_template)
                .ok_or_else(|| kernel_decline("marked Arguments::new has no constant template"))?;
            let Some((elements, _)) = view.list_elements(*list) else {
                return Err(kernel_decline(
                    "marked Arguments::new has no argument array",
                ));
            };
            let mut texts = Vec::with_capacity(elements.len());
            for element in &elements {
                let value = view.chase_copies(*element);
                if !marked(value) {
                    return Err(kernel_decline(
                        "marked Arguments::new reads an unmarked argument",
                    ));
                }
                texts.push(cx.expr_of(value)?);
            }
            if pieces.len() != texts.len() + 1 {
                return Err(kernel_decline("marked format template arity"));
            }
            Ok(Some(Lowered::value(super::fmt::concat_format_pieces(
                &pieces, &texts,
            ))))
        }
        StandardOperation::FormatArgsToString => {
            let [arguments] = args.as_slice() else {
                return Ok(None);
            };
            if !marked(view.chase_copies(*arguments)) {
                return Ok(None);
            }
            let result = op
                .result
                .ok_or_else(|| kernel_decline("marked format! has no destination"))?;
            // Already the text the baseline `String` holds: every piece of it is well formed on
            // its own (see `display_text`), so the concatenation is too.
            let text = cx.expr_of(*arguments)?;
            match cx.func.values[result.0 as usize].carrier {
                Carrier::HostString => Ok(Some(Lowered::value(text))),
                Carrier::StringBuilder => {
                    // The exact-fit builder: the text measured once, for its length and its
                    // capacity both.
                    let (bind, name) = cx.temp(text);
                    let (bind_len, length) = cx.temp(jsast::rt_call("str_len", vec![name.clone()]));
                    Ok(Some(Lowered::Value(
                        vec![bind, bind_len],
                        super::string::exact_builder(name, length),
                    )))
                }
                _ => Err(kernel_decline("marked format! destination is not native")),
            }
        }
        StandardOperation::StringFmtWriteFmt => {
            let [receiver, arguments] = args.as_slice() else {
                return Ok(None);
            };
            if !marked(view.chase_copies(*arguments)) {
                return Ok(None);
            }
            // The receiver is a committed StringBuilder behind `&mut` (the plan's gate). The
            // append is `String::push_str`'s: the formatted text (well formed piece by piece, see
            // `display_text`), the required length is the cached length plus that text's (the
            // only text measured), the capacity grows to cover it, and the cached length advances
            // with the text.
            let walk = cx.pointee_place(*receiver)?;
            let recv = cx.read_place(&walk.place);
            // A format site's runtime members are the planner's to account (`representation`),
            // so the set is only a sink here.
            let mut helpers = BTreeSet::new();
            let (bind_text, text) = cx.temp(cx.expr_of(*arguments)?);
            let (bind_required, required) = cx.temp(jsast::binary(
                BinOp::Add,
                super::string::builder_len(recv.clone(), &mut helpers),
                jsast::rt_call("str_len", vec![text.clone()]),
            ));
            let mut stmts = vec![
                bind_text,
                bind_required,
                jsast::if_(
                    jsast::binary(
                        BinOp::Lt,
                        jsast::member(recv.clone(), "cap"),
                        required.clone(),
                    ),
                    vec![jsast::assign_stmt(
                        jsast::member(recv.clone(), "cap"),
                        required.clone(),
                    )],
                ),
                jsast::expr_stmt(jsast::compound_assign(
                    BinOp::Add,
                    jsast::member(recv.clone(), "text"),
                    text,
                )),
                jsast::assign_stmt(jsast::member(recv.clone(), "len"), required),
            ];
            stmts.extend(super::string::builder_checked(recv, &mut helpers));
            match op.result {
                None => Ok(Some(Lowered::Done(stmts))),
                Some(result) => {
                    let ok = super::fmt::write_ok_expr(cx.table, cx.interns, cx.ty_of(result)?)
                        .map_err(|_| {
                            kernel_decline("marked write_fmt result is not Result<(), fmt::Error>")
                        })?;
                    Ok(Some(Lowered::Value(stmts, ok)))
                }
            }
        }
        _ => Ok(None),
    }
}

fn kernel_decline(detail: &str) -> crate::jir::decline::Decline {
    decline(OpClass::Call, format!("format kernel: {detail}"))
}

/// The host text of one display argument (v1 `display_text_from_ref`): the place read, wrapped
/// by its proven kind. `mark` is the committed carrier of the `new_display` result, which for a
/// `String` source records the source's own carrier.
///
/// Every piece a site concatenates is well formed on its own, so the concatenation is exactly
/// the text the baseline `String` decodes from its bytes, and two adjacent pieces can never pair
/// into a scalar the baseline never held. A host `&str` is the one piece that may hold a lone
/// surrogate, so it is taken as Rust holds it (`__rt.native_str_wf`: U+FFFD) where it is read.
/// Everything else is well formed by construction: the template's literal pieces are decoded
/// UTF-8 (`decode_template`), the integer, `bool` and `f64` texts are ASCII, a `char` is one
/// scalar, and a native `String` source's text is itself only ever built from such pieces.
fn display_text(
    cx: &JsCx<'_>,
    root: Root,
    path: &[Proj],
    kind: DisplayKind,
    mark: Carrier,
) -> Res<Expr> {
    let walk = cx.walk_place(&root, path)?;
    let loaded = cx.read_place(&walk.place);
    Ok(match kind {
        DisplayKind::Integer => jsast::call(jsast::id("String"), vec![loaded]),
        DisplayKind::Bool => jsast::cond(loaded, jsast::string("true"), jsast::string("false")),
        DisplayKind::Char => jsast::method_call(jsast::id("String"), "fromCodePoint", vec![loaded]),
        // The planner accounts the members a site reaches (`representation`): the set is a sink.
        DisplayKind::Str => super::string::well_formed(loaded, &mut BTreeSet::new()),
        DisplayKind::F64 => {
            jsast::member(jsast::rt_call("native_f64_to_string", vec![loaded]), "text")
        }
        DisplayKind::String => match mark {
            Carrier::StringBuilder => jsast::member(loaded, "text"),
            _ => loaded,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::decode_template;

    #[test]
    fn the_default_template_encoding_decodes_to_v1_pieces() {
        assert_eq!(
            decode_template(b"\x02x=\xc0\0").unwrap(),
            vec!["x=".to_owned(), String::new()]
        );
        assert_eq!(
            decode_template(b"\xc0\x02px\0").unwrap(),
            vec![String::new(), "px".to_owned()]
        );
        assert_eq!(
            decode_template(b"\x05item:\xc0\x01:\xc0\0").unwrap(),
            vec!["item:".to_owned(), ":".to_owned(), String::new()]
        );
        assert_eq!(decode_template(&[0]).unwrap(), vec![String::new()]);
        // A non-default placeholder (width, `{0}` reuse, Debug) is not a template this decodes.
        assert!(decode_template(&[0xc1, 0]).is_none());
        assert!(decode_template(b"\x02x=").is_none());
    }
}
