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

use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::repr::{Carrier, Func, Op, OpKind, Proj, Root, ValueId};
use crate::jir::semantic::{
    BodySemantic, CallSemantic, Callee, ConstTree, ItemId, StandardOperation,
};
use crate::jir::types::{RepresentationCandidate, TypeId, TypeKind, TypeTable};
use crate::jsast::{self, BinOp, Expr, Stmt};

use super::{type_get, SemanticDecline};

/// The operations the kernel consumes or elides at a call site.
pub(crate) fn is_format_machinery(operation: StandardOperation) -> bool {
    matches!(
        operation,
        StandardOperation::FormatArgsNew
            | StandardOperation::FormatArgumentNewDisplay
            | StandardOperation::FormatArgumentNewLowerHex
            | StandardOperation::FormatArgumentNewUpperHex
            | StandardOperation::FormatArgsToString
            | StandardOperation::StringFmtWriteFmt
    )
}

/// Concatenate static template pieces with already-lowered argument texts.
pub(crate) fn concat_format_pieces_with_bytes(
    pieces: &[String],
    texts: &[Expr],
    byte_strings: bool,
) -> Expr {
    let mut parts = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        if !piece.is_empty() {
            let lit = if byte_strings {
                jsast::string(crate::lower::bstr::byte_string(piece))
            } else {
                jsast::string(piece)
            };
            parts.push(lit);
        }
        if let Some(text) = texts.get(index) {
            parts.push(text.clone());
        }
    }
    parts
        .into_iter()
        .reduce(|left, right| jsast::binary(BinOp::Add, left, right))
        .unwrap_or_else(|| jsast::string(""))
}

/// Proves the return type is the canonical `Result<(), fmt::Error>` and spells its `Ok` value.
pub(crate) fn write_ok_expr(
    table: &TypeTable,
    interns: &Interns,
    result: TypeId,
) -> Result<Expr, SemanticDecline> {
    let (tag, key) = write_ok_variant(table, interns, result)?;
    if let Some(expr) = crate::lower::js::value::enum_value(
        table,
        interns,
        result,
        0,
        vec![(key.clone(), jsast::undefined())],
    ) {
        return Ok(expr);
    }
    Ok(jsast::object(vec![
        ("TAG".to_owned(), jsast::string(&tag)),
        (key, jsast::undefined()),
    ]))
}

fn write_ok_variant(
    table: &TypeTable,
    interns: &Interns,
    result_ty: TypeId,
) -> Result<(String, String), SemanticDecline> {
    let result = type_get(table, result_ty)?;
    let TypeKind::Adt {
        args,
        form: crate::jir::types::NominalForm::Enum,
        variants,
        ..
    } = &result.kind
    else {
        return Err(SemanticDecline::WrongReturnType);
    };
    if args.len() != 2 || variants.len() != 2 {
        return Err(SemanticDecline::WrongReturnType);
    }
    let [ok, error] = variants.as_slice() else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let (Some(ok_tag), Some(error_tag)) = (ok.physical_tag, error.physical_tag) else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let unit = type_get(table, args[0])?;
    let fmt_error = type_get(table, args[1])?;
    let field_tys = |variant: &crate::jir::types::VariantDescriptor| -> Vec<TypeId> {
        variant.fields.iter().map(|field| field.ty).collect()
    };
    if !ok.result_ok
        || error.result_ok
        || ok_tag.bits != 0
        || error_tag.bits != 1
        || ok_tag.bytes != error_tag.bytes
        || field_tys(ok) != [args[0]]
        || field_tys(error) != [args[1]]
        || !matches!(&unit.kind, TypeKind::Tuple(fields) if fields.is_empty())
        || !unit.layout.zst
        || !matches!(
            &fmt_error.kind,
            TypeKind::Adt {
                form: crate::jir::types::NominalForm::Struct,
                variants,
                ..
            } if variants.len() == 1 && variants[0].fields.is_empty()
        )
        || !fmt_error.layout.zst
        || !result.layout.sized
        || result.layout.zst
    {
        return Err(SemanticDecline::WrongReturnType);
    }
    let tag = interns
        .str(ok.name)
        .ok_or(SemanticDecline::WrongReturnType)?
        .to_owned();
    let key = match ok.fields[0].key {
        crate::jir::types::FieldKey::Name(id) => interns
            .str(id)
            .ok_or(SemanticDecline::WrongReturnType)?
            .to_owned(),
        crate::jir::types::FieldKey::Index(index) => format!("_{index}"),
        crate::jir::types::FieldKey::Transparent => {
            return Err(SemanticDecline::WrongReturnType);
        }
    };
    Ok((tag, key))
}

/// Which `core::fmt::rt::Argument::new_*` constructor built a placeholder argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArgFormat {
    Display,
    LowerHex,
    UpperHex,
}

/// Decoded static formatting options of one template placeholder (`core/src/fmt/mod.rs`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PlaceholderSpec {
    pub sign_plus: bool,
    pub alternate: bool,
    pub zero_pad: bool,
    pub width: Option<u16>,
    pub precision: Option<u16>,
}

impl PlaceholderSpec {
    pub(crate) fn is_default(self) -> bool {
        !self.sign_plus
            && !self.alternate
            && !self.zero_pad
            && self.width.is_none()
            && self.precision.is_none()
    }
}

/// The proven argument value shapes the pipeline recognizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplayKind {
    Integer { signed: bool, bits: u32 },
    Bool,
    Char,
    Str,
    F64,
    F32,
    String,
}

/// One formatted argument of a site.
#[derive(Clone, Debug)]
pub(crate) struct DisplayArg {
    pub kind: DisplayKind,
    pub format: ArgFormat,
    pub spec: PlaceholderSpec,
    /// The `Argument::new_*` call's result: the site's text binding.
    pub value: ValueId,
    /// The place the formatted value is loaded from, all further derefs appended.
    pub root: Root,
    pub path: Vec<Proj>,
}

impl DisplayArg {
    /// The `__rt.*` member required to format this argument, if any.
    pub(crate) fn required_helper(&self) -> Option<&'static str> {
        required_runtime_helper(self.format, self.kind, self.spec)
    }
}

fn supported_arg_spec(format: ArgFormat, kind: DisplayKind, spec: PlaceholderSpec) -> bool {
    match format {
        ArgFormat::Display => {
            if spec.is_default() {
                true
            } else if matches!(kind, DisplayKind::F64 | DisplayKind::F32) {
                !spec.alternate
                    && !spec.zero_pad
                    && spec.width.is_none()
                    && spec.precision.is_some()
            } else if matches!(kind, DisplayKind::Integer { .. }) {
                !spec.alternate && spec.precision.is_none()
            } else {
                false
            }
        }
        ArgFormat::LowerHex | ArgFormat::UpperHex => {
            matches!(kind, DisplayKind::Integer { .. })
                && !spec.sign_plus
                && spec.precision.is_none()
        }
    }
}

fn can_inline_unsigned_hex(signed: bool, bits: u32, spec: PlaceholderSpec) -> bool {
    !signed && bits <= 32 && !spec.alternate && (spec.width.is_none() || spec.zero_pad)
}

fn required_runtime_helper(
    format: ArgFormat,
    kind: DisplayKind,
    spec: PlaceholderSpec,
) -> Option<&'static str> {
    match (format, kind) {
        (ArgFormat::Display, DisplayKind::F64) => Some(if spec.precision.is_some() {
            "fmt_float_fixed"
        } else {
            "native_f64_to_string"
        }),
        (ArgFormat::Display, DisplayKind::F32) => Some(if spec.precision.is_some() {
            "fmt_float_fixed"
        } else {
            "native_f32_to_string"
        }),
        (ArgFormat::Display, DisplayKind::Integer { .. }) if !spec.is_default() => {
            Some("fmt_int_dec")
        }
        (ArgFormat::LowerHex | ArgFormat::UpperHex, DisplayKind::Integer { signed, bits }) => {
            if can_inline_unsigned_hex(signed, bits, spec) {
                None
            } else {
                Some("fmt_int_hex")
            }
        }
        _ => None,
    }
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
                        ConstTree::SliceAddress { length: 0, .. } => true,
                        ConstTree::Reference(inner) => {
                            matches!(
                                inner.as_ref(),
                                ConstTree::Aggregate(e) | ConstTree::Slice(e) if e.is_empty()
                            ) || matches!(
                                inner.as_ref(),
                                ConstTree::Unit | ConstTree::SliceAddress { length: 0, .. }
                            )
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
        let (root, mut path) = match self.def(reference).map(|op| &op.kind) {
            Some(OpKind::BorrowView {
                root,
                path,
                mutable: false,
            }) => (*root, path.clone()),
            _ => (Root::Value(reference), vec![Proj::Deref]),
        };
        let mut ty = pointee;
        loop {
            let descriptor = self.table.get(ty)?;
            match &descriptor.kind {
                TypeKind::Integer {
                    signed,
                    bits: bits @ (8 | 16 | 32 | 64 | 128),
                } => {
                    return Some((
                        root,
                        path,
                        DisplayKind::Integer {
                            signed: *signed,
                            bits: *bits,
                        },
                    ));
                }
                TypeKind::Bool => return Some((root, path, DisplayKind::Bool)),
                TypeKind::Char => return Some((root, path, DisplayKind::Char)),
                TypeKind::Float { bits: 64 } => return Some((root, path, DisplayKind::F64)),
                TypeKind::Float { bits: 32 } => return Some((root, path, DisplayKind::F32)),
                // The place holds the `str` (or the `&str` one deref up): reading it is the host
                // string when reached through a shared `&str`, NOT when dereferencing a `FatSlice`
                // `str` pointer (`&mut str`, `*const str`, `*mut str`, `NonNull<str>`, `Box<str>`).
                TypeKind::Str => {
                    if self.derefs_mut_str(root, &path) {
                        return None;
                    }
                    return Some((root, path, DisplayKind::Str));
                }
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

    /// Whether walking `(root, path)` reaches `str` by dereferencing a `FatSlice` `str` pointer
    /// (`&mut str` or any raw `str` pointer `*const str` / `*mut str` / `NonNull<str>` / `Box<str>`).
    fn derefs_mut_str(&self, root: Root, path: &[Proj]) -> bool {
        let mut ty = match root {
            Root::Value(v) => match self.ty(v) {
                Some(ty) => ty,
                None => return true,
            },
            Root::Cell(cell) => match self.func.cells.get(cell.0 as usize) {
                Some(c) => c.ty,
                None => return true,
            },
        };
        for proj in path {
            match proj {
                Proj::Deref => match self.kind_of(ty) {
                    Some(TypeKind::Reference { pointee, mutable }) => {
                        if *mutable && matches!(self.kind_of(*pointee), Some(TypeKind::Str)) {
                            return true;
                        }
                        ty = *pointee;
                    }
                    Some(TypeKind::RawPointer { pointee, .. }) => {
                        if matches!(self.kind_of(*pointee), Some(TypeKind::Str)) {
                            return true;
                        }
                        ty = *pointee;
                    }
                    _ => return true,
                },
                Proj::Field { ty: field_ty, .. } => ty = *field_ty,
                Proj::Downcast { .. } => {}
                _ => return false,
            }
        }
        false
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

/// `FormattingOptions::flags` bit layout (`core/src/fmt/mod.rs:318-345`).
const FILL_MASK: u32 = 0x1f_ffff;
const DEFAULT_FILL: u32 = ' ' as u32;
const SIGN_PLUS_FLAG: u32 = 1 << 21;
const SIGN_MINUS_FLAG: u32 = 1 << 22;
const ALTERNATE_FLAG: u32 = 1 << 23;
const SIGN_AWARE_ZERO_PAD_FLAG: u32 = 1 << 24;
const DEBUG_LOWER_HEX_FLAG: u32 = 1 << 25;
const DEBUG_UPPER_HEX_FLAG: u32 = 1 << 26;
const WIDTH_FLAG: u32 = 1 << 27;
const PRECISION_FLAG: u32 = 1 << 28;
const ALIGN_BITS: u32 = 0b11 << 29;
const ALIGN_UNKNOWN: u32 = 3 << 29;
const DEFAULT_FLAGS: u32 = DEFAULT_FILL | ALIGN_UNKNOWN;
const UNSUPPORTED_FLAGS_MASK: u32 =
    (1 << 31) | SIGN_MINUS_FLAG | DEBUG_LOWER_HEX_FLAG | DEBUG_UPPER_HEX_FLAG;

/// Decodes a Rust 1.99 `format_args!` template byte sequence (`core/src/fmt/mod.rs:662-702`):
/// literal runs length-prefixed (`0x80` for a two-byte length), `0xC0..=0xFF` placeholders,
/// `0x00` terminator. Fails closed (`None`) on any indirect width/precision, explicit `arg_index`,
/// custom fill character, explicit alignment (`<`/`>`/`^`), `-` flag, or debug hex flag.
pub(crate) fn decode_template(bytes: &[u8]) -> Option<(Vec<String>, Vec<PlaceholderSpec>)> {
    let mut pieces = vec![String::new()];
    let mut specs = Vec::new();
    let mut cursor = 0;
    loop {
        let header = *bytes.get(cursor)?;
        cursor += 1;
        match header {
            0 => return (cursor == bytes.len()).then_some((pieces, specs)),
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
            0xc0 => {
                specs.push(PlaceholderSpec::default());
                pieces.push(String::new());
            }
            0xc1..=0xff => {
                // Decline explicit `arg_index` (bit 3), indirect width (bit 4), indirect precision (bit 5).
                if header & 0x38 != 0 {
                    return None;
                }
                let flags = if header & 1 != 0 {
                    let raw = bytes.get(cursor..cursor + 4)?;
                    cursor += 4;
                    u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
                } else {
                    DEFAULT_FLAGS
                };
                let width = if header & 2 != 0 {
                    let raw = bytes.get(cursor..cursor + 2)?;
                    cursor += 2;
                    Some(u16::from_le_bytes([raw[0], raw[1]]))
                } else {
                    None
                };
                let precision = if header & 4 != 0 {
                    let raw = bytes.get(cursor..cursor + 2)?;
                    cursor += 2;
                    Some(u16::from_le_bytes([raw[0], raw[1]]))
                } else {
                    None
                };
                if (flags & FILL_MASK) != DEFAULT_FILL
                    || (flags & ALIGN_BITS) != ALIGN_UNKNOWN
                    || (flags & UNSUPPORTED_FLAGS_MASK) != 0
                    || ((flags & WIDTH_FLAG) != 0) != width.is_some()
                    || ((flags & PRECISION_FLAG) != 0) != precision.is_some()
                {
                    return None;
                }
                specs.push(PlaceholderSpec {
                    sign_plus: (flags & SIGN_PLUS_FLAG) != 0,
                    alternate: (flags & ALTERNATE_FLAG) != 0,
                    zero_pad: (flags & SIGN_AWARE_ZERO_PAD_FLAG) != 0,
                    width,
                    precision,
                });
                pieces.push(String::new());
            }
            _ => return None,
        }
    }
}

// -- plan-time recognition ---------------------------------------------------------------------

fn analyze_universal(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    resolve: &dyn Fn(&ItemId) -> Option<StandardOperation>,
) -> Vec<Site> {
    let view = View::new(func, table, interns);
    let uses = use_counts(func);
    let used_once = |v: ValueId| uses.get(&v).copied().unwrap_or(0) == 1;

    // Every identity-proven machinery call, by identity.
    let mut machinery: BTreeMap<(usize, usize), (StandardOperation, ItemId)> = BTreeMap::new();
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, op) in block.ops.iter().enumerate() {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                semantic,
                ..
            } = &op.kind
            else {
                continue;
            };
            let op_id = resolve(item)
                .filter(|op| is_format_machinery(*op))
                .or(match semantic {
                    CallSemantic::Standard(StandardOperation::StringFmtWriteFmt) => {
                        Some(StandardOperation::StringFmtWriteFmt)
                    }
                    _ => None,
                });
            if let Some(operation) = op_id {
                machinery.insert((b, i), (operation, *item));
            }
        }
    }

    let mut sites = Vec::new();
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
        sites.push(site);
    }
    sites
}

/// Marks all proven `format!`/`write!` sites directly for universal `NativeString`
/// (`heap.native_string_enabled`).
///
/// Under universal `NativeString` (`js-str=bytes` + `{ s, cap }`), every `String` stays
/// `Carrier::Baseline` universally and every `&str` is already a UTF-8 byte string. Proven sites
/// mark only their use-once `Arguments::new`, `Argument::new_*`, and spine renames with
/// `Carrier::HostString`, leaving any unrelated unproven format calls in the same function
/// untouched. Each rewritten function is carrier-verified before installation.
///
/// Returns the names of every modified function.
pub(crate) fn commit_universal_native_string_sites(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    skip: &BTreeSet<JsName>,
    items: &crate::lower::js::host::ItemIndex,
) -> BTreeSet<JsName> {
    let resolve = |item: &ItemId| items.get(item).and_then(|entry| entry.standard);
    let mut touched = BTreeSet::new();
    for (name, entry) in funcs.iter_mut() {
        if !live.contains(name) || skip.contains(name) {
            continue;
        }
        let Ok(func) = entry else {
            continue;
        };
        let Some(link_item) = table.get(name) else {
            continue;
        };
        if !matches!(
            link_item.jir.body.as_ref().map(|body| body.semantic),
            Some(BodySemantic::Ordinary | BodySemantic::JsExport)
        ) {
            continue;
        }
        let Some(interns) = link_item.jir.interns() else {
            continue;
        };
        let sites = analyze_universal(func, &interns.types, interns, &resolve);
        if sites.is_empty() {
            continue;
        }
        let mut candidate = func.clone();
        let mut applied_any = false;
        for site in &sites {
            if candidate.values[site.args_value.0 as usize].carrier != Carrier::Baseline {
                continue;
            }
            // Pre-check that `write_fmt`'s return type is the canonical `Result<(), fmt::Error>`
            // so `intercept` can never decline later in Phase C.
            if let SiteKind::WriteFmt {
                result: Some(res), ..
            } = site.kind
            {
                let res_ty = candidate.values[res.0 as usize].ty;
                if write_ok_expr(&interns.types, interns, res_ty).is_err() {
                    continue;
                }
            }
            candidate.values[site.args_value.0 as usize].carrier = Carrier::HostString;
            for arg in &site.args {
                candidate.values[arg.value.0 as usize].carrier = Carrier::HostString;
            }
            for (rename, renamed) in &site.renames {
                candidate.values[rename.0 as usize].carrier =
                    candidate.values[renamed.0 as usize].carrier;
            }
            applied_any = true;
        }
        if !applied_any {
            continue;
        }
        if candidate
            .verify_carriers(&interns.types, crate::opt::observed_carriers(&candidate))
            .is_ok()
        {
            *func = candidate;
            touched.insert(name.clone());
        }
    }
    touched
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

    let (pieces, specs) = decode_template(&view.const_bytes(*template)?)?;
    let (elements, list_spine) = view.list_elements(*list)?;
    spine.extend(list_spine);
    if specs.len() != elements.len() || pieces.len() != elements.len() + 1 {
        return None;
    }

    let mut args = Vec::with_capacity(elements.len());
    let mut display_calls = Vec::with_capacity(elements.len());
    let mut callees: BTreeSet<ItemId> = BTreeSet::new();
    callees.insert(new_item);
    for (element, &spec) in elements.iter().zip(&specs) {
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
        let (arg_op, display_item) = *machinery.get(&display_call)?;
        let format = match arg_op {
            StandardOperation::FormatArgumentNewDisplay => ArgFormat::Display,
            StandardOperation::FormatArgumentNewLowerHex => ArgFormat::LowerHex,
            StandardOperation::FormatArgumentNewUpperHex => ArgFormat::UpperHex,
            _ => return None,
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
        if !supported_arg_spec(format, kind, spec) {
            return None;
        }
        args.push(DisplayArg {
            kind,
            format,
            spec,
            value,
            root,
            path,
        });
        display_calls.push(display_call);
        callees.insert(display_item);
    }
    // Every spine value carries a native text after the commit, so anything else reading one
    // would read the wrong shape: each must be used exactly once, along the spine.
    if !spine.iter().all(|v| used_once(*v)) {
        return None;
    }
    // The write_fmt receiver must be the exact `&mut String` the builder contract expects.
    if let SiteKind::WriteFmt { receiver, .. } = kind {
        let recv_ty = view.table.peeled(view.ty(receiver)?);
        let TypeKind::Reference {
            pointee,
            mutable: true,
        } = *view.kind_of(recv_ty)?
        else {
            return None;
        };
        let descriptor = view.table.get(view.table.peeled(pointee))?;
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
        StandardOperation::FormatArgumentNewDisplay
        | StandardOperation::FormatArgumentNewLowerHex
        | StandardOperation::FormatArgumentNewUpperHex => {
            let Some(result) = op.result.filter(|result| marked(*result)) else {
                return Ok(None);
            };
            let [reference] = args.as_slice() else {
                return Err(kernel_decline("marked new_argument arity"));
            };
            let Some((root, path, kind)) = view.display_source(*reference) else {
                return Err(kernel_decline("marked new_argument has no proven source"));
            };
            let format = match operation {
                StandardOperation::FormatArgumentNewDisplay => ArgFormat::Display,
                StandardOperation::FormatArgumentNewLowerHex => ArgFormat::LowerHex,
                StandardOperation::FormatArgumentNewUpperHex => ArgFormat::UpperHex,
                _ => unreachable!(),
            };
            let Some(spec) = find_placeholder_spec(&view, cx.items, result) else {
                return Err(kernel_decline(
                    "marked new_argument has no unique Arguments::new consumer",
                ));
            };
            if !supported_arg_spec(format, kind, spec) {
                return Err(kernel_decline(
                    "marked new_argument placeholder spec unsupported",
                ));
            }
            let (stmts, text) = format_text(
                cx,
                root,
                &path,
                kind,
                format,
                spec,
                cx.func.values[result.0 as usize].carrier,
            )?;
            if stmts.is_empty() {
                Ok(Some(Lowered::value(text)))
            } else {
                Ok(Some(Lowered::Value(stmts, text)))
            }
        }
        StandardOperation::FormatArgsNew => {
            let Some(result) = op.result.filter(|result| marked(*result)) else {
                return Ok(None);
            };
            let _ = result;
            let [template, list] = args.as_slice() else {
                return Err(kernel_decline("marked Arguments::new arity"));
            };
            let (pieces, _) = view
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
            let byte_strings = cx.heap().native_string_enabled || cx.bstr_enabled();
            Ok(Some(Lowered::value(concat_format_pieces_with_bytes(
                &pieces,
                &texts,
                byte_strings,
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
            let text = cx.expr_of(*arguments)?;
            match cx.func.values[result.0 as usize].carrier {
                Carrier::Baseline if cx.heap().native_string_enabled => {
                    let (bind, name) = cx.temp(text);
                    Ok(Some(Lowered::Value(
                        vec![bind],
                        jsast::object_of(vec![
                            ("s", name.clone()),
                            ("cap", jsast::member(name, "length")),
                        ]),
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
            let walk = cx.pointee_place(*receiver)?;
            let recv = cx.read_place(&walk.place);
            if !cx.heap().native_string_enabled {
                return Err(kernel_decline(
                    "marked write_fmt requires universal native String",
                ));
            }
            let text = cx.expr_of(*arguments)?;
            let stmts = vec![jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::member(recv, "s"),
                text,
            ))];
            match op.result {
                None => Ok(Some(Lowered::Done(stmts))),
                Some(result) => {
                    let ok =
                        write_ok_expr(cx.table, cx.interns, cx.ty_of(result)?).map_err(|_| {
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

/// Finds the unique `Arguments::new` call whose element array consumes `arg_result` and returns
/// its decoded [`PlaceholderSpec`] at that element's index. Fails closed (`None`) unless
/// `arg_result` appears in exactly one element of one marked `Arguments::new` call.
fn find_placeholder_spec(
    view: &View<'_>,
    items: &crate::lower::js::host::ItemIndex,
    arg_result: ValueId,
) -> Option<PlaceholderSpec> {
    let mut found: Option<PlaceholderSpec> = None;
    for block in &view.func.blocks {
        for op in &block.ops {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                args,
                ..
            } = &op.kind
            else {
                continue;
            };
            if items.get(item).and_then(|e| e.standard) != Some(StandardOperation::FormatArgsNew) {
                continue;
            }
            let Some(res) = op.result else {
                continue;
            };
            if view.func.values[res.0 as usize].carrier == Carrier::Baseline {
                continue;
            }
            let [template, list] = args.as_slice() else {
                continue;
            };
            let Some((elements, _)) = view.list_elements(*list) else {
                continue;
            };
            let Some((_, specs)) = view
                .const_bytes(*template)
                .as_deref()
                .and_then(decode_template)
            else {
                continue;
            };
            if specs.len() != elements.len() {
                continue;
            }
            for (elem, &spec) in elements.iter().zip(&specs) {
                if view.chase_copies(*elem) == arg_result {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(spec);
                }
            }
        }
    }
    found
}

/// The host text of one formatted argument: the place read, wrapped by its proven kind, format,
/// and placeholder options. `mark` is the committed carrier of the `new_*` result, which for a
/// `String` source records the source's own carrier.
///
/// Every piece a site concatenates is well formed on its own, so the concatenation is exactly
/// the text the baseline `String` decodes from its bytes, and two adjacent pieces can never pair
/// into a scalar the baseline never held. A host `&str` is the one piece that may hold a lone
/// surrogate, so it is taken as Rust holds it (`__rt.native_str_wf`: U+FFFD) where it is read.
/// Everything else is well formed by construction: the template's literal pieces are decoded
/// UTF-8 (`decode_template`), the integer, `bool`, `f64`, and `f32` texts are ASCII, a `char` is
/// one scalar, and a native `String` source's text is itself only ever built from such pieces.
fn format_text(
    cx: &JsCx<'_>,
    root: Root,
    path: &[Proj],
    kind: DisplayKind,
    format: ArgFormat,
    spec: PlaceholderSpec,
    mark: Carrier,
) -> Res<(Vec<Stmt>, Expr)> {
    let walk = cx.walk_place(&root, path)?;
    let loaded = cx.read_place(&walk.place);
    let byte_strings = cx.heap().native_string_enabled || cx.bstr_enabled();
    let mut setup = Vec::new();
    let expr = match (format, kind) {
        (ArgFormat::Display, DisplayKind::Integer { .. }) => {
            if spec.is_default() {
                jsast::call(jsast::id("String"), vec![loaded])
            } else {
                jsast::rt_call(
                    "fmt_int_dec",
                    vec![
                        loaded,
                        jsast::num(f64::from(spec.width.unwrap_or(0))),
                        jsast::boolean(spec.zero_pad),
                        jsast::boolean(spec.sign_plus),
                    ],
                )
            }
        }
        (ArgFormat::LowerHex | ArgFormat::UpperHex, DisplayKind::Integer { signed, bits }) => {
            let upper = format == ArgFormat::UpperHex;
            if can_inline_unsigned_hex(signed, bits, spec) {
                let mut expr = jsast::method_call(loaded, "toString", vec![jsast::num(16)]);
                if upper {
                    expr = jsast::method_call(expr, "toUpperCase", Vec::new());
                }
                if let Some(width) = spec.width.filter(|w| *w > 0) {
                    expr = jsast::method_call(
                        expr,
                        "padStart",
                        vec![jsast::num(f64::from(width)), jsast::string("0")],
                    );
                }
                expr
            } else {
                jsast::rt_call(
                    "fmt_int_hex",
                    vec![
                        loaded,
                        jsast::num(f64::from(bits)),
                        jsast::boolean(upper),
                        jsast::num(f64::from(spec.width.unwrap_or(0))),
                        jsast::boolean(spec.zero_pad),
                        jsast::boolean(spec.alternate),
                    ],
                )
            }
        }
        (ArgFormat::Display, DisplayKind::Bool) => {
            jsast::cond(loaded, jsast::string("true"), jsast::string("false"))
        }
        (ArgFormat::Display, DisplayKind::Char) => {
            if byte_strings {
                let ch = if matches!(loaded, Expr::Ident(_) | Expr::Num(_)) {
                    loaded
                } else {
                    let (bind, id) = cx.temp(loaded);
                    setup.push(bind);
                    id
                };
                jsast::cond(
                    jsast::binary(BinOp::Lt, ch.clone(), jsast::num(128)),
                    jsast::method_call(jsast::id("String"), "fromCharCode", vec![ch.clone()]),
                    jsast::rt_call("bstr_char", vec![ch]),
                )
            } else {
                jsast::method_call(jsast::id("String"), "fromCodePoint", vec![loaded])
            }
        }
        // Under `js-str=bytes` / universal `NativeString`, every `&str` is already a well-formed
        // UTF-8 byte string.
        (ArgFormat::Display, DisplayKind::Str) => {
            if byte_strings {
                loaded
            } else {
                return Err(kernel_decline("Str display requires byte-string mode"));
            }
        }
        (ArgFormat::Display, DisplayKind::F64) => match spec.precision {
            Some(prec) => jsast::rt_call(
                "fmt_float_fixed",
                vec![
                    loaded,
                    jsast::num(f64::from(prec)),
                    jsast::boolean(spec.sign_plus),
                ],
            ),
            None => jsast::member(jsast::rt_call("native_f64_to_string", vec![loaded]), "text"),
        },
        (ArgFormat::Display, DisplayKind::F32) => match spec.precision {
            Some(prec) => jsast::rt_call(
                "fmt_float_fixed",
                vec![
                    loaded,
                    jsast::num(f64::from(prec)),
                    jsast::boolean(spec.sign_plus),
                ],
            ),
            None => jsast::member(jsast::rt_call("native_f32_to_string", vec![loaded]), "text"),
        },
        (ArgFormat::Display, DisplayKind::String) => match mark {
            Carrier::StringBuilder => jsast::member(loaded, "text"),
            Carrier::HostString if cx.heap().native_string_enabled => jsast::member(loaded, "s"),
            _ => loaded,
        },
        _ => return Err(kernel_decline("unsupported format/kind combination")),
    };
    Ok((setup, expr))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_template, PlaceholderSpec, ALIGN_UNKNOWN, ALTERNATE_FLAG, DEFAULT_FILL,
        PRECISION_FLAG, SIGN_AWARE_ZERO_PAD_FLAG, SIGN_PLUS_FLAG, WIDTH_FLAG,
    };

    #[test]
    fn the_default_template_encoding_decodes_to_v1_pieces() {
        assert_eq!(
            decode_template(b"\x02x=\xc0\0").unwrap(),
            (
                vec!["x=".to_owned(), String::new()],
                vec![PlaceholderSpec::default()]
            )
        );
        assert_eq!(
            decode_template(b"\xc0\x02px\0").unwrap(),
            (
                vec![String::new(), "px".to_owned()],
                vec![PlaceholderSpec::default()]
            )
        );
        assert_eq!(
            decode_template(b"\x05item:\xc0\x01:\xc0\0").unwrap(),
            (
                vec!["item:".to_owned(), ":".to_owned(), String::new()],
                vec![PlaceholderSpec::default(), PlaceholderSpec::default()]
            )
        );
        assert_eq!(
            decode_template(&[0]).unwrap(),
            (vec![String::new()], Vec::new())
        );
        // Truncated or malformed templates decline.
        assert!(decode_template(&[0xc1, 0]).is_none());
        assert!(decode_template(b"\x02x=").is_none());
    }

    #[test]
    fn extended_template_decodes_precision_hex_zero_pad_and_declines_unsupported() {
        // `{:.3}`: header = 0xC0 | 1 (flags) | 4 (precision) = 0xC5
        let mut prec3 = vec![0xc5];
        prec3.extend_from_slice(&(DEFAULT_FILL | ALIGN_UNKNOWN | PRECISION_FLAG).to_le_bytes());
        prec3.extend_from_slice(&3u16.to_le_bytes());
        prec3.push(0);
        assert_eq!(
            decode_template(&prec3).unwrap(),
            (
                vec![String::new(), String::new()],
                vec![PlaceholderSpec {
                    precision: Some(3),
                    ..PlaceholderSpec::default()
                }]
            )
        );

        // `{:#08x}`: header = 0xC0 | 1 (flags) | 2 (width) = 0xC3
        let mut hex08 = vec![0xc3];
        hex08.extend_from_slice(
            &(DEFAULT_FILL
                | ALIGN_UNKNOWN
                | ALTERNATE_FLAG
                | SIGN_AWARE_ZERO_PAD_FLAG
                | WIDTH_FLAG)
                .to_le_bytes(),
        );
        hex08.extend_from_slice(&8u16.to_le_bytes());
        hex08.push(0);
        assert_eq!(
            decode_template(&hex08).unwrap(),
            (
                vec![String::new(), String::new()],
                vec![PlaceholderSpec {
                    alternate: true,
                    zero_pad: true,
                    width: Some(8),
                    ..PlaceholderSpec::default()
                }]
            )
        );

        // `{:+05}`: sign_plus + zero_pad + width
        let mut dec_plus = vec![0xc3];
        dec_plus.extend_from_slice(
            &(DEFAULT_FILL
                | ALIGN_UNKNOWN
                | SIGN_PLUS_FLAG
                | SIGN_AWARE_ZERO_PAD_FLAG
                | WIDTH_FLAG)
                .to_le_bytes(),
        );
        dec_plus.extend_from_slice(&5u16.to_le_bytes());
        dec_plus.push(0);
        assert_eq!(
            decode_template(&dec_plus).unwrap(),
            (
                vec![String::new(), String::new()],
                vec![PlaceholderSpec {
                    sign_plus: true,
                    zero_pad: true,
                    width: Some(5),
                    ..PlaceholderSpec::default()
                }]
            )
        );

        // Explicit arg_index (bit 3 = 0x08), indirect width (0x10), indirect precision (0x20) decline.
        assert!(decode_template(&[0xc8, 0, 0, 0]).is_none());
        assert!(decode_template(&[0xd2, 0, 0, 0]).is_none());
        assert!(decode_template(&[0xe4, 0, 0, 0]).is_none());

        // Custom fill char declines.
        let mut custom_fill = vec![0xc3];
        custom_fill.extend_from_slice(&('*' as u32 | ALIGN_UNKNOWN | WIDTH_FLAG).to_le_bytes());
        custom_fill.extend_from_slice(&4u16.to_le_bytes());
        custom_fill.push(0);
        assert!(decode_template(&custom_fill).is_none());

        // Explicit alignment (ALIGN_LEFT = 0 << 29) declines.
        let mut explicit_align = vec![0xc3];
        explicit_align.extend_from_slice(&(DEFAULT_FILL | WIDTH_FLAG).to_le_bytes());
        explicit_align.extend_from_slice(&4u16.to_le_bytes());
        explicit_align.push(0);
        assert!(decode_template(&explicit_align).is_none());
    }
}
