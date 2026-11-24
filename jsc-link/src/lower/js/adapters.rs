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
//! byte-level marshalling itself is the existing shim's: `__rt.bytes_str` decodes a block's UTF-8
//! bytes, and `__rt.convert_str_block` / `__rt.convert_sb_block` encode a fresh, unshared block —
//! fresh because `str_bytes` memoizes records that other `as_bytes` calls may alias, and an owned
//! baseline `String` must not share its allocation. Capacity is Rust-visible and survives every
//! string conversion that can carry it: a builder's `.cap` rides into the baseline cap leaf
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
    convert_with_heap(
        table,
        interns,
        value,
        ty,
        from,
        to,
        &crate::analysis::heap::DEFAULT_HEAP_PLAN,
    )
}

pub fn convert_with_heap(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
    from: Carrier,
    to: Carrier,
    heap: &crate::analysis::heap::HeapPlan,
) -> Res<Expr> {
    let typed_tier_a = heap.typed_heap_enabled && !heap.non_rc_raw_alloc_reachable;
    match (from, to) {
        (Carrier::StringBuilder, Carrier::HostString) => Ok(jsast::member(value, "text")),
        (Carrier::HostString, Carrier::StringBuilder) => {
            Ok(jsast::rt_call("convert_host_sb", vec![value]))
        }
        (Carrier::Baseline, Carrier::HostString) => baseline_to_host(table, interns, value, ty),
        (Carrier::HostString, Carrier::Baseline) => {
            let helper = if typed_tier_a && crate::lower::bstr::enabled() {
                "bstr_block_typed"
            } else {
                "convert_str_block"
            };
            block_to_baseline(table, interns, ty, jsast::rt_call(helper, vec![value]))
        }
        (Carrier::Baseline, Carrier::StringBuilder) => {
            baseline_to_builder(table, interns, value, ty)
        }
        (Carrier::StringBuilder, Carrier::Baseline) => {
            let helper = if typed_tier_a && crate::lower::bstr::enabled() {
                "bstr_sb_block_typed"
            } else {
                "convert_sb_block"
            };
            block_to_baseline(table, interns, ty, jsast::rt_call(helper, vec![value]))
        }
        (Carrier::NativeVec, Carrier::Baseline) => {
            native_vec_to_baseline(table, interns, value, ty, heap)
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
            let mut args = vec![value];
            if let Some(kind) = vec_tv_kind(table, ty) {
                args.push(jsast::num(f64::from(kind)));
            }
            Ok(jsast::rt_call("convert_sv_nv", args))
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

/// `__rt.bytes_str(<ptr>.buf, <ptr>.off, <len>)` over the baseline block.
fn baseline_to_host(table: &TypeTable, interns: &Interns, value: Expr, ty: TypeId) -> Res<Expr> {
    let layout = StringLayout::of(table, interns, ty)?;
    let (value, wrap) = once(value);
    Ok(wrap(host_text(&layout, value)))
}

/// The decoded text of a baseline `String` value.
fn host_text(layout: &StringLayout, value: Expr) -> Expr {
    let pointer = layout.member(value.clone(), &layout.ptr);
    jsast::rt_call(
        "bytes_str",
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
    heap: &crate::analysis::heap::HeapPlan,
) -> Res<Expr> {
    let layout = VecLayout::of(table, interns, ty)?;
    let (size, scalar_kind, element) = element_info(table, ty)?;
    let (value, wrap) = once(value);
    let pointer = if heap.typed_heap_enabled && !heap.non_rc_raw_alloc_reachable {
        let mut args = vec![value.clone()];
        let tv_kind = vec_tv_kind(table, ty);
        let tracked = heap.needs_box_tracking(table, element);
        if let Some(k) = tv_kind {
            args.push(jsast::num(f64::from(k)));
        } else if tracked {
            args.push(jsast::undefined());
            args.push(Expr::Bool(true));
        }
        jsast::rt_call("convert_nv_typed", args)
    } else {
        // A registered, element-typed heap block: baseline reads reach the buffer through
        // `unscale`'s block registry, so a bare `{ buf, off }` record would refuse the very first
        // deref (`__rt.convert_nv_block` mirrors an `alloc` + first-cast retype).
        let mut args = vec![value.clone(), jsast::raw_num(size.to_string())];
        if let Some(kind) = scalar_kind {
            args.push(jsast::string(kind));
        } else if table.needs_clone(element) {
            args.push(jsast::arrow(
                vec![super::value::CLONE_PARAM.to_string()],
                super::value::clone_expr_with(
                    table,
                    interns,
                    element,
                    jsast::id(super::value::CLONE_PARAM),
                    crate::lower::NestedClone {
                        vec: true,
                        string: true,
                        map: true,
                        native_string: heap.native_string_enabled,
                    },
                ),
            ));
        } else if table.is_indirect(element) {
            args.push(Expr::Bool(true));
        }
        // Real patched `alloc` (`0018-typed-rawvec-and-box.patch`) stores `Unique<T>` directly in
        // `RawVecInner<T>.ptr`, so `RawVec::ptr()` never casts from `*mut u8` / unscales. Pass
        // `true` as the 4th arg so `convert_nv_block` returns the typed pointer rather than the
        // raw byte handle (while keeping the raw/typed pair registered for later byte views).
        let ptr_is_typed = size > 1
            && layout
                .ptr_pointee
                .and_then(|pt| super::value::size_of(table, table.peeled(pt)))
                == Some(size);
        if ptr_is_typed {
            if args.len() == 2 {
                args.push(jsast::undefined());
            }
            args.push(Expr::Bool(true));
        }
        jsast::rt_call("convert_nv_block", args)
    };
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
                Leaf::Pointer(_) => pointer.clone(),
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

/// The element's byte size, optional wider-scalar view kind, and element `TypeId`, from the
/// vec's first type argument.
fn element_info(table: &TypeTable, ty: TypeId) -> Res<(u64, Option<String>, TypeId)> {
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
    let scalar_kind = match table.get(table.peeled(element)).map(|d| &d.kind) {
        Some(TypeKind::Integer { signed, bits }) if *bits > 8 => {
            Some(format!("{}{bits}", if *signed { 'i' } else { 'u' }))
        }
        Some(TypeKind::Float { bits: 32 }) => Some("f32".to_owned()),
        Some(TypeKind::Float { bits: 64 }) => Some("f64".to_owned()),
        Some(TypeKind::Char) => Some("u32".to_owned()),
        _ => None,
    };
    Ok((size, scalar_kind, element))
}

fn vec_tv_kind(table: &TypeTable, ty: TypeId) -> Option<u8> {
    let TypeKind::Adt { args, .. } = &table.get(ty)?.kind else {
        return None;
    };
    let element = table.get(*args.first()?)?;
    crate::lower::semantic_ops::vec::typed_array_kind(element)
}

/// `__rt.convert_block_nv(<ptr>.buf, <ptr>.off, <len>, <cap>[, tvKind])`: a capacity-length native
/// buffer copied out of the baseline block, Rust-visible length and capacity preserved. Legal only
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
    let mut args = vec![
        jsast::member(pointer.clone(), "buf"),
        jsast::member(pointer, "off"),
        layout.member(value.clone(), &layout.len),
        layout.member(value, &layout.cap),
    ];
    if let Some(kind) = vec_tv_kind(table, ty) {
        args.push(jsast::num(f64::from(kind)));
    }
    let call = jsast::rt_call("convert_block_nv", args);
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
    let (size, _, _) = element_info(table, ty)?;
    let (value, wrap) = once(value);
    let pointer = layout.member(value.clone(), &layout.ptr);
    if size > 1 {
        return Ok(wrap(jsast::rt_call(
            "convert_block_sv",
            vec![pointer, layout.member(value, &layout.len)],
        )));
    }
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
pub(crate) struct StringLayout {
    /// To the `{ buf, off }` data-pointer slot.
    pub(crate) ptr: Vec<String>,
    /// To the length in bytes.
    pub(crate) len: Vec<String>,
    /// To the capacity in bytes, inside the raw-parts aggregate.
    pub(crate) cap: Vec<String>,
}

/// The recorded member paths from a baseline `Vec` value to its raw parts.
struct VecLayout {
    /// To the `{ buf, off }` data-pointer slot.
    ptr: Vec<String>,
    /// Pointee type of the `ptr` leaf (`T` under `0018-typed-rawvec-and-box.patch`, `u8` under
    /// untyped `RawVecInner`).
    ptr_pointee: Option<TypeId>,
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
        let mut ptr_pointee = None;
        walk(table, interns, ty, &[], &mut |path, leaf| match leaf {
            Leaf::Unsigned if path.len() == 1 => len = Some(path.to_vec()),
            Leaf::Unsigned => cap = Some(path.to_vec()),
            Leaf::Pointer(pointee) => {
                ptr = Some(path.to_vec());
                ptr_pointee = Some(pointee);
            }
            _ => {}
        })
        .map_err(|detail| no(&detail))?;
        match (ptr, len, cap) {
            (Some(ptr), Some(len), Some(cap)) => Ok(Self {
                ptr,
                ptr_pointee,
                len,
                cap,
            }),
            _ => Err(no(
                "the Vec structure has no pointer, length and capacity leaves",
            )),
        }
    }

    pub(super) fn member(&self, base: Expr, path: &[String]) -> Expr {
        member_path(base, path)
    }
}

/// What a leaf field of the walk is.
#[derive(Clone, Copy, PartialEq)]
enum Leaf {
    Pointer(TypeId),
    Unsigned,
    Recurse,
    Zst,
}

impl StringLayout {
    /// Resolves the canonical `String { vec: Vec<u8> }` structure, or declines.
    pub(crate) fn of(table: &TypeTable, interns: &Interns, ty: TypeId) -> Res<Self> {
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
                Leaf::Pointer(_) => ptr = Some(path.to_vec()),
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

    pub(crate) fn member(&self, base: Expr, path: &[String]) -> Expr {
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
        Some(TypeKind::RawPointer { pointee, .. } | TypeKind::Reference { pointee, .. }) => {
            Leaf::Pointer(*pointee)
        }
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

/// Canonical positional field indices of `alloc::string::String { vec: Vec<u8> }`
/// (`Vec { buf: RawVec, len }`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StringParts {
    pub(crate) vec_field: u32,
    pub(crate) len_field: u32,
    pub(crate) buf_field: u32,
}

/// Resolves the canonical `String { vec: Vec<u8> { buf, len } }` field indices when `ty` matches.
pub(crate) fn string_parts(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
) -> Option<StringParts> {
    StringLayout::of(table, interns, ty).ok()?;
    let descriptor = table.get(ty)?;
    let TypeKind::Adt { variants, .. } = &descriptor.kind else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    let mut real_str = variant
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !table.is_zst(f.ty));
    let (vec_idx, vec_f) = real_str.next()?;
    if real_str.next().is_some() {
        return None;
    }
    let vec_desc = table.get(vec_f.ty)?;
    let TypeKind::Adt {
        variants: vec_vars, ..
    } = &vec_desc.kind
    else {
        return None;
    };
    let [vec_var] = vec_vars.as_slice() else {
        return None;
    };
    let mut len_f = None;
    let mut buf_f = None;
    for (idx, f) in vec_var.fields.iter().enumerate() {
        if table.is_zst(f.ty) {
            continue;
        }
        let f_desc = table.get(f.ty)?;
        match &f_desc.kind {
            TypeKind::Integer { signed: false, .. } if len_f.is_none() => {
                len_f = Some(idx as u32);
            }
            TypeKind::Adt { .. } if buf_f.is_none() => {
                buf_f = Some(idx as u32);
            }
            _ => return None,
        }
    }
    Some(StringParts {
        vec_field: vec_idx as u32,
        len_field: len_f?,
        buf_field: buf_f?,
    })
}
