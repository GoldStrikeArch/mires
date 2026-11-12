//! `Convert` lowering: the registry-owned adapters between a value's carriers (Phase 4, D6).
//!
//! An adapter is a small expression over the value being converted — never a statement, never a
//! change to the value's Rust type. The string family: `Baseline` (the `Vec<u8>` alloc-block
//! `String` struct), [`Carrier::HostString`] (an immutable JavaScript string) and
//! [`Carrier::StringBuilder`] (`{ text, cap, len }`, capacity and length in UTF-8 bytes). The
//! vec/slice family:
//! [`Carrier::NativeVec`] (`{ buf, len, cap }`, `buf` a capacity-length array initialized in
//! `[0, len)`) and [`Carrier::SliceView`] (`{ buf, off, len }` sharing a native buffer — the
//! same record a baseline fat slice is, so that direction is a plain re-record).
//!
//! The baseline block is reached through the type table, never through guessed field names: the
//! resolver walks the recorded `String` → `Vec<u8>` → raw-parts structure to the `{ buf, off }`
//! pointer leaf, the length, and the capacity, and declines any shape it does not recognize. The
//! byte-level marshalling itself is the existing shim's: `__rt.convert_block_str` decodes a block's
//! UTF-8 bytes (`__rt.bytes_str`'s decode) and consumes the block, as `__rt.convert_block_nv` does
//! for a `Vec`, and `__rt.convert_str_block` / `__rt.convert_sb_block` encode a fresh, unshared
//! block — fresh because `str_bytes` memoizes records that other `as_bytes` calls may alias, and
//! an owned baseline `String` must not share its allocation. Capacity is Rust-visible and survives
//! every string conversion that can carry it: a builder's `.cap` rides into the baseline cap leaf
//! (capacity-length backing block) and back. Only the immutable [`Carrier::HostString`] is
//! exact-fit, by design: capacity observation and every growth path force the builder carrier
//! (the planner's mutable/BUILDER_ONLY legality), so a host-string value's capacity is its
//! length.

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::Carrier;
use rcgjs_object::jir::types::{
    AllocatorIdentity, FieldKey, RepresentationCandidate, TypeId, TypeKind, TypeTable,
};
use rcgjs_object::jsast::{self, Expr};

use super::{decline, Res};

/// The expression converting `value` (of Rust type `ty`) from carrier `from` to carrier `to`.
///
/// `value` is evaluated exactly once: a conversion that reads it more than once wraps it in an
/// arrow unless it is already a bare identifier.
pub fn convert(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
    from: Carrier,
    to: Carrier,
) -> Res<Expr> {
    match (from, to) {
        (Carrier::StringBuilder, Carrier::HostString) => Ok(jsast::member(value, "text")),
        (Carrier::HostString, Carrier::StringBuilder) => {
            Ok(jsast::rt_call("convert_host_sb", vec![value]))
        }
        (Carrier::Baseline, Carrier::HostString) => baseline_to_host(table, interns, value, ty),
        (Carrier::HostString, Carrier::Baseline) => block_to_baseline(
            table,
            interns,
            ty,
            jsast::rt_call("convert_str_block", vec![value]),
        ),
        (Carrier::Baseline, Carrier::StringBuilder) => {
            baseline_to_builder(table, interns, value, ty)
        }
        (Carrier::StringBuilder, Carrier::Baseline) => block_to_baseline(
            table,
            interns,
            ty,
            jsast::rt_call("convert_sb_block", vec![value]),
        ),
        (Carrier::NativeVec, Carrier::Baseline) => {
            native_vec_to_baseline(table, interns, value, ty)
        }
        (Carrier::Baseline, Carrier::NativeVec) => {
            baseline_to_native_vec(table, interns, value, ty)
        }
        (Carrier::Baseline, Carrier::SliceView) => {
            baseline_vec_to_slice_view(table, interns, value, ty)
        }
        (Carrier::Baseline, Carrier::NativeBox) | (Carrier::NativeBox, Carrier::Baseline) => {
            Ok(value)
        }
        (Carrier::NativeVec, Carrier::SliceView) => {
            let (value, wrap) = once(value);
            Ok(wrap(jsast::object_of(vec![
                ("buf", jsast::member(value.clone(), "buf")),
                ("off", jsast::num(0)),
                ("len", jsast::member(value, "len")),
            ])))
        }
        (Carrier::SliceView, Carrier::NativeVec) => {
            Ok(jsast::rt_call("convert_sv_nv", vec![value]))
        }
        (Carrier::SliceView, Carrier::Baseline) => {
            // The baseline fat slice is the same `{ buf, off, len }` record; re-record so the
            // converted value never aliases the view's own mutations.
            let (value, wrap) = once(value);
            Ok(wrap(jsast::object_of(vec![
                ("buf", jsast::member(value.clone(), "buf")),
                ("off", jsast::member(value.clone(), "off")),
                ("len", jsast::member(value, "len")),
            ])))
        }
        _ => Err(decline(
            OpClass::Rvalue,
            format!("no Convert adapter from {from:?} to {to:?}"),
        )),
    }
}

/// `__rt.convert_block_str(<ptr>.buf, <ptr>.off, <len>)` over the baseline block.
fn baseline_to_host(table: &TypeTable, interns: &Interns, value: Expr, ty: TypeId) -> Res<Expr> {
    let layout = StringLayout::of(table, interns, ty)?;
    let (value, wrap) = once(value);
    Ok(wrap(host_text(&layout, value)))
}

/// The decoded text of a baseline `String` value, which consumes the value's block.
///
/// Both conversions that spell it (to the host string and to the builder) leave the baseline
/// direction, which the planner commits only for a dead, unaliased source or a value being
/// dropped (D2's dead-source rule; `analysis/valueflow.rs` `dead_unaliased`). The block is
/// therefore never read again, and no `dealloc` will come for it: the native carrier is dropped by
/// the engine. So the decode goes through `__rt.convert_block_str`, which gives the block's
/// synthetic address granule back exactly as `dealloc` would. A decode of bytes the program still
/// owns (`str::from_utf8_unchecked`, a `&str` from raw parts) stays `__rt.bytes_str`.
fn host_text(layout: &StringLayout, value: Expr) -> Expr {
    let pointer = layout.member(value.clone(), &layout.ptr);
    jsast::rt_call(
        "convert_block_str",
        vec![
            jsast::member(pointer.clone(), "buf"),
            jsast::member(pointer, "off"),
            layout.member(value, &layout.len),
        ],
    )
}

/// `{ text, cap, len }` straight off the baseline leaves: the decoded text, the cap leaf carried
/// into the builder's Rust-visible capacity (a moved `String` keeps its capacity), and the len
/// leaf — the decoded bytes' count, exactly the text's UTF-8 length — as its cached length.
fn baseline_to_builder(table: &TypeTable, interns: &Interns, value: Expr, ty: TypeId) -> Res<Expr> {
    let layout = StringLayout::of(table, interns, ty)?;
    let (value, wrap) = once(value);
    Ok(wrap(jsast::object_of(vec![
        ("text", host_text(&layout, value.clone())),
        ("cap", layout.member(value.clone(), &layout.cap)),
        ("len", layout.member(value, &layout.len)),
    ])))
}

/// A fresh baseline `String` struct over an owned `{ buf, off, len, cap }` byte block
/// (`__rt.convert_str_block` or `__rt.convert_sb_block`): the block's length and capacity land
/// on the recorded len and cap leaves, so a builder's excess capacity survives the conversion.
fn block_to_baseline(table: &TypeTable, interns: &Interns, ty: TypeId, block: Expr) -> Res<Expr> {
    let layout = StringLayout::of(table, interns, ty)?;
    let pointer = jsast::object_of(vec![
        ("buf", jsast::member(jsast::id("$cb"), "buf")),
        ("off", jsast::num(0)),
    ]);
    let length = jsast::member(jsast::id("$cb"), "len");
    let capacity = jsast::member(jsast::id("$cb"), "cap");
    let literal = wrap_paths(
        table,
        interns,
        ty,
        &layout.cap,
        &pointer,
        &length,
        &capacity,
    )?;
    Ok(jsast::call(
        jsast::arrow(vec!["$cb".to_owned()], literal),
        vec![block],
    ))
}

/// A fresh, exact-fit baseline `Vec` struct copying the native carrier's `[0, len)` — a fresh
/// buffer because an owned baseline `Vec` must not share the native value's allocation.
fn native_vec_to_baseline(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
) -> Res<Expr> {
    let layout = VecLayout::of(table, interns, ty)?;
    let size = element_size(table, ty)?;
    let (value, wrap) = once(value);
    // A registered, element-typed heap block: baseline reads reach the buffer through
    // `unscale`'s block registry, so a bare `{ buf, off }` record would refuse the very first
    // deref (`__rt.convert_nv_block` mirrors an `alloc` + first-cast retype).
    let pointer = jsast::rt_call(
        "convert_nv_block",
        vec![value.clone(), jsast::raw_num(size.to_string())],
    );
    let length = jsast::member(value.clone(), "len");
    let capacity = jsast::member(value, "cap");
    wrap_paths(
        table,
        interns,
        ty,
        &layout.cap,
        &pointer,
        &length,
        &capacity,
    )
    .map(wrap)
}

/// The baseline struct literal with each leaf at its recorded path: the pointer slot, the
/// length, and — at `cap`, the recorded capacity path — the capacity kept exactly (D5's
/// Rust-visible capacity).
fn wrap_paths(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    cap: &[String],
    pointer: &Expr,
    length: &Expr,
    capacity: &Expr,
) -> Res<Expr> {
    #[allow(clippy::too_many_arguments)] // A recursive walker over exactly these leaves.
    fn build(
        table: &TypeTable,
        interns: &Interns,
        ty: TypeId,
        at: &mut Vec<String>,
        cap: &[String],
        pointer: &Expr,
        length: &Expr,
        capacity: &Expr,
    ) -> Res<Expr> {
        let no = |detail: &str| decline(OpClass::Rvalue, format!("Convert adapter: {detail}"));
        let fields = struct_fields(table, interns, ty).map_err(|detail| no(&detail))?;
        let mut entries = Vec::with_capacity(fields.len());
        for (key, field_ty) in fields {
            at.push(key.clone());
            let value = match classify(table, field_ty) {
                Leaf::Zst => jsast::undefined(),
                Leaf::Pointer => pointer.clone(),
                Leaf::Unsigned if at.as_slice() == cap => capacity.clone(),
                Leaf::Unsigned => length.clone(),
                Leaf::Recurse => {
                    build(table, interns, field_ty, at, cap, pointer, length, capacity)?
                }
            };
            at.pop();
            entries.push((key, value));
        }
        Ok(jsast::object(entries))
    }
    let mut at = Vec::new();
    build(table, interns, ty, &mut at, cap, pointer, length, capacity)
}

/// The element's byte size, from the vec's first type argument.
fn element_size(table: &TypeTable, ty: TypeId) -> Res<u64> {
    let no = |detail: &str| decline(OpClass::Rvalue, format!("vec Convert adapter: {detail}"));
    let descriptor = table
        .get(ty)
        .ok_or_else(|| no("the value's type is not in the table"))?;
    let TypeKind::Adt { args, .. } = &descriptor.kind else {
        return Err(no("the Vec is not an Adt"));
    };
    let element = args
        .first()
        .copied()
        .ok_or_else(|| no("the Vec has no element type argument"))?;
    let size = table
        .get(element)
        .and_then(|element| element.layout.size)
        .filter(|size| *size > 0)
        .ok_or_else(|| no("the element size is unavailable or zero"))?;
    Ok(size)
}

/// `__rt.convert_block_nv(<ptr>.buf, <ptr>.off, <len>, <cap>)`: a capacity-length native buffer
/// copied out of the baseline block, Rust-visible length and capacity preserved. Legal only
/// under D2's dead-source rule, which the planner enforces.
fn baseline_to_native_vec(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
) -> Res<Expr> {
    let layout = VecLayout::of(table, interns, ty)?;
    let (value, wrap) = once(value);
    let pointer = layout.member(value.clone(), &layout.ptr);
    let call = jsast::rt_call(
        "convert_block_nv",
        vec![
            jsast::member(pointer.clone(), "buf"),
            jsast::member(pointer, "off"),
            layout.member(value.clone(), &layout.len),
            layout.member(value, &layout.cap),
        ],
    );
    Ok(wrap(call))
}

/// A borrowed view of a baseline `Vec`'s initialized range. The slice carrier deliberately
/// shares the source allocation; consumers must keep the Rust borrow's lifetime and may not
/// mutate or retain the view beyond it.
fn baseline_vec_to_slice_view(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
) -> Res<Expr> {
    let layout = VecLayout::of(table, interns, ty)?;
    let (value, wrap) = once(value);
    let pointer = layout.member(value.clone(), &layout.ptr);
    Ok(wrap(jsast::object_of(vec![
        ("buf", jsast::member(pointer.clone(), "buf")),
        ("off", jsast::member(pointer, "off")),
        ("len", layout.member(value, &layout.len)),
    ])))
}

/// Evaluate `value` once: a bare identifier is used as is, anything else binds an arrow parameter.
#[allow(clippy::type_complexity)]
fn once(value: Expr) -> (Expr, Box<dyn FnOnce(Expr) -> Expr>) {
    if matches!(&value, Expr::Ident(_)) {
        (value, Box::new(|body| body))
    } else {
        (
            jsast::id("$cv"),
            Box::new(move |body| {
                jsast::call(jsast::arrow(vec!["$cv".to_owned()], body), vec![value])
            }),
        )
    }
}

/// The recorded member paths from a baseline `String` value to its raw parts.
struct StringLayout {
    /// To the `{ buf, off }` data-pointer slot.
    ptr: Vec<String>,
    /// To the length in bytes.
    len: Vec<String>,
    /// To the capacity in bytes, inside the raw-parts aggregate.
    cap: Vec<String>,
}

/// The recorded member paths from a baseline `Vec` value to its raw parts.
struct VecLayout {
    /// To the `{ buf, off }` data-pointer slot.
    ptr: Vec<String>,
    /// To the length in elements — the `Vec`'s own top-level field.
    len: Vec<String>,
    /// To the capacity in elements, inside the raw-parts aggregate.
    cap: Vec<String>,
}

impl VecLayout {
    /// Resolves the canonical `Vec { buf: RawVec, len }` structure, or declines.
    fn of(table: &TypeTable, interns: &Interns, ty: TypeId) -> Res<Self> {
        let no = |detail: &str| decline(OpClass::Rvalue, format!("vec Convert adapter: {detail}"));
        let descriptor = table
            .get(ty)
            .ok_or_else(|| no("the value's type is not in the table"))?;
        if descriptor.meta.candidate != RepresentationCandidate::Vec
            || descriptor.meta.allocator != AllocatorIdentity::Global
        {
            return Err(no("the type is not the canonical global-allocator Vec"));
        }

        // At the Vec level: one unsigned leaf (`len`) and one aggregate to descend (the raw
        // parts, holding the pointer and the capacity), in whichever declaration order the table
        // records.
        let mut len = None;
        let mut cap = None;
        let mut ptr = None;
        walk(table, interns, ty, &[], &mut |path, leaf| match leaf {
            Leaf::Unsigned if path.len() == 1 => len = Some(path.to_vec()),
            Leaf::Unsigned => cap = Some(path.to_vec()),
            Leaf::Pointer => ptr = Some(path.to_vec()),
            _ => {}
        })
        .map_err(|detail| no(&detail))?;
        match (ptr, len, cap) {
            (Some(ptr), Some(len), Some(cap)) => Ok(Self { ptr, len, cap }),
            _ => Err(no(
                "the Vec structure has no pointer, length and capacity leaves",
            )),
        }
    }

    fn member(&self, base: Expr, path: &[String]) -> Expr {
        member_path(base, path)
    }
}

/// What a leaf field of the walk is.
#[derive(Clone, Copy, PartialEq)]
enum Leaf {
    Pointer,
    Unsigned,
    Recurse,
    Zst,
}

impl StringLayout {
    /// Resolves the canonical `String { vec: Vec<u8> }` structure, or declines.
    fn of(table: &TypeTable, interns: &Interns, ty: TypeId) -> Res<Self> {
        let no =
            |detail: &str| decline(OpClass::Rvalue, format!("string Convert adapter: {detail}"));
        let descriptor = table
            .get(ty)
            .ok_or_else(|| no("the value's type is not in the table"))?;
        if descriptor.meta.candidate != RepresentationCandidate::String
            || descriptor.meta.allocator != AllocatorIdentity::Global
        {
            return Err(no("the type is not the canonical global-allocator String"));
        }
        let vec_field = single_real_field(table, interns, ty)
            .ok_or_else(|| no("the String struct does not have exactly one real field"))?;
        let vector = table
            .get(vec_field.1)
            .ok_or_else(|| no("the Vec field's type is not in the table"))?;
        let element_is_byte = match &vector.kind {
            TypeKind::Adt { args, .. } => args
                .first()
                .is_some_and(|element| table.byte_element(*element)),
            _ => false,
        };
        if vector.meta.candidate != RepresentationCandidate::Vec || !element_is_byte {
            return Err(no("the String's field is not a byte Vec"));
        }

        // At the Vec level: one unsigned leaf (`len`) and one aggregate to descend (the raw
        // parts, holding the pointer and the capacity), in whichever declaration order the
        // table records.
        let mut len = None;
        let mut cap = None;
        let mut ptr = None;
        let base = vec![vec_field.0.clone()];
        walk(
            table,
            interns,
            vec_field.1,
            &base,
            &mut |path, leaf| match leaf {
                Leaf::Unsigned if path.len() == 2 => len = Some(path.to_vec()),
                Leaf::Unsigned => cap = Some(path.to_vec()),
                Leaf::Pointer => ptr = Some(path.to_vec()),
                _ => {}
            },
        )
        .map_err(|detail| no(&detail))?;
        match (ptr, len, cap) {
            (Some(ptr), Some(len), Some(cap)) => Ok(Self { ptr, len, cap }),
            _ => Err(no(
                "the Vec structure has no pointer, length and capacity leaves",
            )),
        }
    }

    fn member(&self, base: Expr, path: &[String]) -> Expr {
        member_path(base, path)
    }
}

fn member_path(base: Expr, path: &[String]) -> Expr {
    path.iter()
        .fold(base, |expr, key| jsast::member(expr, key.clone()))
}

/// Depth-first over the recorded struct tree, reporting each leaf with its member path.
fn walk(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    path: &[String],
    visit: &mut impl FnMut(&[String], Leaf),
) -> Result<(), String> {
    if path.len() > 8 {
        return Err("the String structure is deeper than the canonical layout".to_owned());
    }
    for (key, field_ty) in struct_fields(table, interns, ty)? {
        let mut at = path.to_vec();
        at.push(key);
        match classify(table, field_ty) {
            Leaf::Recurse => walk(table, interns, field_ty, &at, visit)?,
            leaf => visit(&at, leaf),
        }
    }
    Ok(())
}

fn classify(table: &TypeTable, ty: TypeId) -> Leaf {
    if table.is_zst(ty) {
        return Leaf::Zst;
    }
    let peeled = table.peeled(ty);
    match table.get(peeled).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::RawPointer { .. } | TypeKind::Reference { .. }) => Leaf::Pointer,
        Some(TypeKind::Integer { signed: false, .. }) => Leaf::Unsigned,
        Some(TypeKind::Adt { .. }) => Leaf::Recurse,
        _ => Leaf::Recurse,
    }
}

/// The JavaScript keys and types of a single-variant struct's fields, in declaration order.
fn struct_fields(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
) -> Result<Vec<(String, TypeId)>, String> {
    let descriptor = table
        .get(ty)
        .ok_or_else(|| "a field's type is not in the table".to_owned())?;
    let TypeKind::Adt { variants, .. } = &descriptor.kind else {
        return Err("a field of the String structure is not a struct".to_owned());
    };
    let [variant] = variants.as_slice() else {
        return Err("a field of the String structure is not single-variant".to_owned());
    };
    variant
        .fields
        .iter()
        .map(|field| {
            let key = match field.key {
                FieldKey::Name(id) => interns
                    .str(id)
                    .map(str::to_owned)
                    .ok_or_else(|| "a field key is not interned".to_owned())?,
                FieldKey::Index(index) => format!("_{index}"),
                FieldKey::Transparent => {
                    return Err("a transparent field key has no member spelling".to_owned());
                }
            };
            Ok((key, field.ty))
        })
        .collect()
}

/// The one non-zero-sized field of a single-variant struct.
fn single_real_field(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<(String, TypeId)> {
    let fields = struct_fields(table, interns, ty).ok()?;
    let mut real = fields
        .into_iter()
        .filter(|(_, field_ty)| !table.is_zst(*field_ty));
    let first = real.next()?;
    real.next().is_none().then_some(first)
}
