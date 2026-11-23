//! The `Vec` family emitters: construction, length, growth, indexing, insertion, removal, extend,
//! clone, truncate, resize, pop, slice-iter, replace, swap, box-array, and drop.
//!
//! The carrier is D5's [`Carrier::NativeVec`]: `{ buf, len, cap }`. Primitive numeric elements
//! (`u8`, `i8`, `u16`, `i16`, `u32`/`usize`, `i32`/`isize`, `f32`, `f64`) back `buf` with an
//! ES2018 growable `TypedArray` (kind tags `0..=7`); all other elements back `buf` with a
//! capacity-length plain JavaScript `Array` with elements initialized in `[0, len)` and released
//! slots cleared. Both storage forms share unified `__rt.nv_*` runtime helpers to minimize shim
//! footprint while keeping hot paths free of `ArrayBuffer.isView` checks.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{BodySemantic, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, EnumRepr, FieldKey, IteratorIdentity, NominalForm, RepresentationCandidate,
    TypeDescriptor, TypeId, TypeKind,
};
use crate::jsast::{self, BinOp, Expr, Stmt};
use crate::ptr;

use super::drop::require_element_glue;
use super::string::{member, receiver};
use super::{
    pointer_limit, slot_type, type_get, unsigned_bits, LowerCx, SemanticDecline, SignatureSlot,
};

/// ES2018 TypedArray constructor kind tag (`__rt._tv_ctors[k]`) for built-in primitive numeric
/// elements. 64/128-bit integers (`BigInt64Array`/`BigUint64Array`) are ES2020 and stay plain
/// `Array`; `bool` and `char` also stay plain `Array`.
pub(crate) fn typed_array_kind(element: &TypeDescriptor) -> Option<u8> {
    match element.kind {
        TypeKind::Integer {
            signed: false,
            bits: 8,
        } => Some(0),
        TypeKind::Integer {
            signed: true,
            bits: 8,
        } => Some(1),
        TypeKind::Integer {
            signed: false,
            bits: 16,
        } => Some(2),
        TypeKind::Integer {
            signed: true,
            bits: 16,
        } => Some(3),
        TypeKind::Integer {
            signed: false,
            bits: 32,
        } => Some(4),
        TypeKind::Integer {
            signed: true,
            bits: 32,
        } => Some(5),
        TypeKind::Float { bits: 32 } => Some(6),
        TypeKind::Float { bits: 64 } => Some(7),
        _ => None,
    }
}

/// Built-in primitive types whose `Clone` impl is guaranteed by the orphan rule to be a pure,
/// non-panicking, drop-free bitwise copy.
pub(super) fn is_trivial_clone_primitive(element: &TypeDescriptor) -> bool {
    matches!(
        element.kind,
        TypeKind::Bool | TypeKind::Char | TypeKind::Integer { .. } | TypeKind::Float { .. }
    ) && !element.layout.needs_drop
        && !element.layout.zst
        && element.layout.sized
}

/// `RawVec::min_non_zero_cap(size_of::<T>())` on this toolchain: 8 for 1-byte elements, 4 for
/// 2..=1024-byte elements, 1 for larger elements.
fn vec_min_cap(cx: &LowerCx<'_>) -> Result<u64, SemanticDecline> {
    let element = vec_element(cx)?;
    let size = element
        .layout
        .size
        .ok_or(SemanticDecline::UnknownElementLayout)?;
    if size == 0 {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    Ok(if size == 1 {
        8
    } else if size <= 1024 {
        4
    } else {
        1
    })
}

fn panic_handler_expr(
    cx: &LowerCx<'_>,
    panic_impl: &JsName,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Expr {
    refs.insert(panic_impl.clone());
    if cx.panic_reads_info {
        helpers.insert("_nv_pinfo");
        jsast::arrow(
            vec!["$m".to_owned(), "$l".to_owned()],
            jsast::rt_call(
                "_nv_pinfo",
                vec![
                    jsast::id("$m"),
                    jsast::id(panic_impl.as_str()),
                    jsast::id("$l"),
                ],
            ),
        )
    } else {
        jsast::id(panic_impl.as_str())
    }
}

fn push_panic_arg(
    cx: &LowerCx<'_>,
    args: &mut Vec<Expr>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
    loc_param: Option<&str>,
) {
    if let Some(panic_impl) = cx.panic_impl {
        args.push(panic_handler_expr(cx, panic_impl, helpers, refs));
        if cx.panic_reads_info {
            if let Some(loc) = loc_param {
                args.push(jsast::id(loc));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RangeShape {
    Full,
    Bounds,
    From,
    To,
}

fn vec_tv_kind(cx: &LowerCx<'_>) -> Result<Option<u8>, SemanticDecline> {
    Ok(typed_array_kind(vec_element(cx)?))
}

pub(super) fn vec_new(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(kind) = vec_tv_kind(cx)? {
        helpers.insert("tv_new");
        return Ok(vec![jsast::ret(jsast::rt_call(
            "tv_new",
            vec![jsast::num(f64::from(kind))],
        ))]);
    }
    Ok(vec![jsast::ret(jsast::object_of(vec![
        ("buf", jsast::array(vec![])),
        ("len", jsast::num(0)),
        ("cap", jsast::num(0)),
    ]))])
}

pub(super) fn vec_with_capacity(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    helpers.insert("nv_cap");
    let mut args = vec![receiver(), jsast::raw_num(maximum.to_string())];
    match (vec_tv_kind(cx)?, cx.panic_impl) {
        (Some(kind), Some(panic_impl)) => {
            args.push(jsast::num(f64::from(kind)));
            args.push(panic_handler_expr(cx, panic_impl, helpers, refs));
        }
        (Some(kind), None) => {
            args.push(jsast::num(f64::from(kind)));
        }
        (None, Some(panic_impl)) => {
            args.push(jsast::undefined());
            args.push(panic_handler_expr(cx, panic_impl, helpers, refs));
        }
        (None, None) => {}
    }
    Ok(vec![jsast::ret(jsast::rt_call("nv_cap", args))])
}

pub(super) fn vec_len(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("len"))])
}

pub(super) fn vec_capacity(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("cap"))])
}

pub(super) fn vec_is_empty(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(jsast::binary(
        BinOp::StrictEq,
        member("len"),
        jsast::num(0),
    ))])
}

/// `as_slice`/`as_mut_slice`: a fat slice sharing the native buffer — exactly the
/// [`Carrier::SliceView`] shape.
pub(super) fn vec_as_slice(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(ptr::fat_slice(
        member("buf"),
        jsast::num(0),
        member("len"),
    ))])
}

/// `Index<usize>` / `IndexMut<usize>`: inline bounds check + element reference so V8 keeps the
/// failure branch out-of-line (`throw`) and eliminates the `direct ? ... : ...` check on hot loops.
pub(super) fn vec_index(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let indirect = vec_element(cx)?.js.is_indirect;
    helpers.insert("_nv_fail");
    let mut fail_args = vec![Expr::Str("oob".to_owned())];
    push_panic_arg(cx, &mut fail_args, helpers, refs, Some("$native2"));
    let value = if indirect {
        jsast::index(member("buf"), jsast::id("$native1"))
    } else {
        ptr::slot(member("buf"), jsast::id("$native1"))
    };
    Ok(vec![
        jsast::if_(
            jsast::binary(BinOp::Ge, jsast::id("$native1"), member("len")),
            vec![jsast::throw(jsast::rt_call("_nv_fail", fail_args))],
        ),
        jsast::ret(value),
    ])
}

/// `Index<Range*<usize>>` / `IndexMut<Range*<usize>>`: subslice sharing the native buffer.
pub(super) fn vec_range_index(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let shape = range_shape(cx)?;
    match shape {
        RangeShape::Full => Ok(vec![jsast::ret(ptr::fat_slice(
            member("buf"),
            jsast::num(0),
            member("len"),
        ))]),
        RangeShape::Bounds => {
            helpers.insert("nv_range");
            let mut args = vec![
                receiver(),
                jsast::member(jsast::id("$native1"), "start"),
                jsast::member(jsast::id("$native1"), "end"),
            ];
            push_panic_arg(cx, &mut args, helpers, refs, Some("$native2"));
            Ok(vec![jsast::ret(jsast::rt_call("nv_range", args))])
        }
        RangeShape::From => {
            helpers.insert("nv_range");
            let mut args = vec![
                receiver(),
                jsast::member(jsast::id("$native1"), "start"),
                member("len"),
            ];
            push_panic_arg(cx, &mut args, helpers, refs, Some("$native2"));
            Ok(vec![jsast::ret(jsast::rt_call("nv_range", args))])
        }
        RangeShape::To => {
            helpers.insert("nv_range");
            let mut args = vec![
                receiver(),
                jsast::num(0),
                jsast::member(jsast::id("$native1"), "end"),
            ];
            push_panic_arg(cx, &mut args, helpers, refs, Some("$native2"));
            Ok(vec![jsast::ret(jsast::rt_call("nv_range", args))])
        }
    }
}

/// `Vec::remove`: bounds throw before mutation, then shift left and return the removed element.
pub(super) fn vec_remove(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("nv_remove");
    let mut args = vec![receiver(), jsast::id("$native1")];
    push_panic_arg(cx, &mut args, helpers, refs, Some("$native2"));
    Ok(vec![jsast::ret(jsast::rt_call("nv_remove", args))])
}

/// `Vec::swap_remove`: O(1) swap with last element and return the removed element.
pub(super) fn vec_swap_remove(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("nv_swap_remove");
    let mut args = vec![receiver(), jsast::id("$native1")];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![jsast::ret(jsast::rt_call("nv_swap_remove", args))])
}

/// `Vec::push`: exponential growth via shared runtime helper, capacity throw before mutation.
pub(super) fn vec_push(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    helpers.insert("nv_push");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_push", args)),
        jsast::ret_void(),
    ])
}

/// `Vec::push_mut(&mut self, value: T) -> &mut T`.
pub(super) fn vec_push_mut(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    let indirect = vec_element(cx)?.js.is_indirect;
    helpers.insert("nv_push");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    let push = jsast::expr_stmt(jsast::rt_call("nv_push", args));
    if indirect {
        Ok(vec![push, jsast::ret(jsast::id("$native1"))])
    } else {
        Ok(vec![
            jsast::const_("$nativeIdx", member("len")),
            push,
            jsast::ret(ptr::slot(member("buf"), jsast::id("$nativeIdx"))),
        ])
    }
}

/// `Vec::insert(&mut self, index: usize, element: T)`.
pub(super) fn vec_insert(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    helpers.insert("nv_insert");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::id("$native2"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, Some("$native3"));
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_insert", args)),
        jsast::ret_void(),
    ])
}

/// `Vec::insert_mut(&mut self, index: usize, element: T) -> &mut T`.
pub(super) fn vec_insert_mut(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    let indirect = vec_element(cx)?.js.is_indirect;
    helpers.insert("nv_insert");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::id("$native2"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, Some("$native3"));
    let insert = jsast::expr_stmt(jsast::rt_call("nv_insert", args));
    let out = if indirect {
        jsast::id("$native2")
    } else {
        ptr::slot(member("buf"), jsast::id("$native1"))
    };
    Ok(vec![insert, jsast::ret(out)])
}

/// `Vec::reserve` / `Vec::reserve_exact`.
pub(super) fn vec_reserve(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    let exact = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecReserveExact)
    );
    helpers.insert("nv_reserve");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
        Expr::Bool(exact),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_reserve", args)),
        jsast::ret_void(),
    ])
}

/// `Vec::shrink_to_fit`.
pub(super) fn vec_shrink_to_fit(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("nv_shrink");
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_shrink", vec![receiver()])),
        jsast::ret_void(),
    ])
}

/// `Vec::pop(&mut self) -> Option<T>`: constructs the `Option<T>` enum literal directly in the
/// JS AST so dense-int enum tag encoding (`hoist_consts.rs`) sees the variant tags transparently.
pub(super) fn vec_pop(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let (none_tag, some_tag, some_field) = option_layout(cx)?;
    let is_tv = vec_tv_kind(cx)?.is_some();
    let mut stmts = vec![
        jsast::if_(
            jsast::binary(BinOp::StrictEq, member("len"), jsast::num(0)),
            vec![jsast::ret(jsast::object_of(vec![(
                "TAG",
                jsast::string(none_tag),
            )]))],
        ),
        jsast::const_(
            "$nativeIdx",
            jsast::binary(BinOp::Sub, member("len"), jsast::num(1)),
        ),
        jsast::assign_stmt(member("len"), jsast::id("$nativeIdx")),
    ];
    if is_tv {
        stmts.push(jsast::ret(jsast::object(vec![
            ("TAG".to_owned(), jsast::string(some_tag)),
            (
                some_field,
                jsast::index(member("buf"), jsast::id("$nativeIdx")),
            ),
        ])));
    } else {
        stmts.push(jsast::const_(
            "$nativeVal",
            jsast::index(member("buf"), jsast::id("$nativeIdx")),
        ));
        stmts.push(jsast::assign_stmt(
            jsast::index(member("buf"), jsast::id("$nativeIdx")),
            jsast::undefined(),
        ));
        stmts.push(jsast::ret(jsast::object(vec![
            ("TAG".to_owned(), jsast::string(some_tag)),
            (some_field, jsast::id("$nativeVal")),
        ])));
    }
    Ok(stmts)
}

/// `Vec::truncate(&mut self, len: usize)`.
pub(super) fn vec_truncate(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(glue) = require_element_glue(cx)? {
        refs.insert(glue.clone());
        return Ok(super::drop::dropful_vec_truncate(
            glue,
            cx.element_glue_nonthrowing,
            helpers,
        ));
    }
    if vec_tv_kind(cx)?.is_some() {
        return Ok(vec![
            jsast::if_(
                jsast::binary(BinOp::Lt, jsast::id("$native1"), member("len")),
                vec![jsast::assign_stmt(member("len"), jsast::id("$native1"))],
            ),
            jsast::ret_void(),
        ]);
    }
    Ok(vec![
        jsast::if_(
            jsast::binary(BinOp::Lt, jsast::id("$native1"), member("len")),
            vec![
                jsast::expr_stmt(jsast::method_call(
                    member("buf"),
                    "fill",
                    vec![jsast::undefined(), jsast::id("$native1"), member("len")],
                )),
                jsast::assign_stmt(member("len"), jsast::id("$native1")),
            ],
        ),
        jsast::ret_void(),
    ])
}

/// `Vec::resize(&mut self, new_len: usize, value: T)` for trivial-`Clone` primitive elements.
pub(super) fn vec_resize(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    helpers.insert("nv_resize");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::id("$native2"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_resize", args)),
        jsast::ret_void(),
    ])
}

/// `alloc::vec::from_elem(elem: T, n: usize) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn vec_from_elem(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    helpers.insert("nv_from_elem");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
    ];
    match (vec_tv_kind(cx)?, cx.panic_impl) {
        (Some(kind), Some(panic_impl)) => {
            args.push(jsast::num(f64::from(kind)));
            args.push(panic_handler_expr(cx, panic_impl, helpers, refs));
        }
        (Some(kind), None) => {
            args.push(jsast::num(f64::from(kind)));
        }
        (None, Some(panic_impl)) => {
            args.push(jsast::undefined());
            args.push(panic_handler_expr(cx, panic_impl, helpers, refs));
        }
        (None, None) => {}
    }
    Ok(vec![jsast::ret(jsast::rt_call("nv_from_elem", args))])
}

/// `alloc::boxed::box_assume_init_into_vec_unsafe(b: Box<MaybeUninit<[T; N]>>) -> Vec<T>` (`vec![...]`).
pub(super) fn vec_from_box_array(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let len = box_array_len(cx)?;
    helpers.insert("nv_from_box");
    let mut args = vec![receiver(), jsast::raw_num(len.to_string())];
    if let Some(kind) = vec_tv_kind(cx)? {
        args.push(jsast::num(f64::from(kind)));
    }
    Ok(vec![jsast::ret(jsast::rt_call("nv_from_box", args))])
}

/// `<&Vec<T> as IntoIterator>::into_iter` / `<&mut Vec<T> as IntoIterator>::into_iter`.
pub(super) fn vec_iter(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let fields = match slice_iter_layout(cx)? {
        SliceIterLayout::Legacy {
            ptr_key,
            end_key,
            marker_key,
        } => vec![
            (ptr_key, ptr::slot(member("buf"), jsast::num(0))),
            (end_key, ptr::slot(member("buf"), member("len"))),
            (marker_key, jsast::undefined()),
        ],
        SliceIterLayout::Indexed {
            ptr_key,
            pos_key,
            end_key,
            marker_key,
        } => vec![
            (ptr_key, ptr::slot(member("buf"), jsast::num(0))),
            (pos_key, jsast::num(0)),
            (end_key, member("len")),
            (marker_key, jsast::undefined()),
        ],
    };
    Ok(vec![jsast::ret(jsast::object(fields))])
}

/// `core::mem::replace(&mut Vec<T>, Vec<T>) -> Vec<T>`.
pub(super) fn vec_replace(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let src = jsast::id("$native1");
    Ok(vec![
        jsast::const_(
            "$nativeOld",
            jsast::object_of(vec![
                ("buf", member("buf")),
                ("len", member("len")),
                ("cap", member("cap")),
            ]),
        ),
        jsast::assign_stmt(member("buf"), jsast::member(src.clone(), "buf")),
        jsast::assign_stmt(member("len"), jsast::member(src.clone(), "len")),
        jsast::assign_stmt(member("cap"), jsast::member(src, "cap")),
        jsast::ret(jsast::id("$nativeOld")),
    ])
}

/// `core::mem::swap(&mut Vec<T>, &mut Vec<T>)`.
pub(super) fn vec_mem_swap(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let other = jsast::id("$native1");
    Ok(vec![
        jsast::const_("$nativeBuf", member("buf")),
        jsast::const_("$nativeLen", member("len")),
        jsast::const_("$nativeCap", member("cap")),
        jsast::assign_stmt(member("buf"), jsast::member(other.clone(), "buf")),
        jsast::assign_stmt(member("len"), jsast::member(other.clone(), "len")),
        jsast::assign_stmt(member("cap"), jsast::member(other.clone(), "cap")),
        jsast::assign_stmt(jsast::member(other.clone(), "buf"), jsast::id("$nativeBuf")),
        jsast::assign_stmt(jsast::member(other.clone(), "len"), jsast::id("$nativeLen")),
        jsast::assign_stmt(jsast::member(other, "cap"), jsast::id("$nativeCap")),
        jsast::ret_void(),
    ])
}

/// `Extend<Vec<T>>` of an owned source vector.
pub(super) fn vec_extend(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    helpers.insert("nv_extend");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_extend", args)),
        jsast::ret_void(),
    ])
}

/// `Vec::extend_from_slice(&mut self, other: &[T])` for trivial-`Clone` primitive elements.
pub(super) fn vec_extend_from_slice(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    let min_cap = vec_min_cap(cx)?;
    helpers.insert("nv_extend_slice");
    let mut args = vec![
        receiver(),
        jsast::id("$native1"),
        jsast::raw_num(maximum.to_string()),
        jsast::raw_num(min_cap.to_string()),
    ];
    push_panic_arg(cx, &mut args, helpers, refs, None);
    Ok(vec![
        jsast::expr_stmt(jsast::rt_call("nv_extend_slice", args)),
        jsast::ret_void(),
    ])
}

/// `<Vec<T> as Clone>::clone(&self) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn vec_clone(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("nv_clone");
    Ok(vec![jsast::ret(jsast::rt_call(
        "nv_clone",
        vec![receiver()],
    ))])
}

/// `<[T]>::to_vec(&self) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn slice_to_vec(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("nv_from_slice");
    let mut args = vec![receiver()];
    if let Some(kind) = vec_tv_kind(cx)? {
        args.push(jsast::num(f64::from(kind)));
    }
    Ok(vec![jsast::ret(jsast::rt_call("nv_from_slice", args))])
}

/// `Vec::clear`: the ordered element-drop body when the element needs drop, zero-helper reset
/// on TypedArray, else the length release helper.
pub(super) fn vec_clear(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(glue) = require_element_glue(cx)? {
        refs.insert(glue.clone());
        Ok(super::drop::dropful_vec_clear(
            glue,
            cx.element_glue_nonthrowing,
            helpers,
        ))
    } else if vec_tv_kind(cx)?.is_some() {
        Ok(vec![
            jsast::assign_stmt(member("len"), jsast::num(0)),
            jsast::ret_void(),
        ])
    } else {
        Ok(vec![
            jsast::expr_stmt(jsast::method_call(
                member("buf"),
                "fill",
                vec![jsast::undefined(), jsast::num(0), member("len")],
            )),
            jsast::assign_stmt(member("len"), jsast::num(0)),
            jsast::ret_void(),
        ])
    }
}

/// `Vec`'s own drop: ordered element drops when required, zero-helper detach/clear.
pub(super) fn vec_drop(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(glue) = require_element_glue(cx)? {
        refs.insert(glue.clone());
        Ok(super::drop::dropful_vec_drop(
            glue,
            cx.element_glue_nonthrowing,
            helpers,
        ))
    } else if vec_tv_kind(cx)?.is_some() {
        Ok(vec![
            jsast::assign_stmt(member("buf"), jsast::null()),
            jsast::assign_stmt(member("len"), jsast::num(0)),
            jsast::assign_stmt(member("cap"), jsast::num(0)),
            jsast::ret_void(),
        ])
    } else {
        Ok(vec![
            jsast::assign_stmt(jsast::member(member("buf"), "length"), jsast::num(0)),
            jsast::assign_stmt(member("len"), jsast::num(0)),
            jsast::assign_stmt(member("cap"), jsast::num(0)),
            jsast::ret_void(),
        ])
    }
}

// Operation-specific legality arms.

pub(super) fn legality_with_capacity(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    if !matches!(
        type_get(cx.table, argument)?.kind,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        }
    ) {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    vec_capacity_limit(cx)?;
    Ok(())
}

pub(super) fn legality_unsigned_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(
        result.kind,
        TypeKind::Integer { signed: false, bits } if bits <= 32
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_bool_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    if !matches!(type_get(cx.table, cx.sig.result)?.kind, TypeKind::Bool) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_unit_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(&result.kind, TypeKind::Tuple(fields) if fields.is_empty()) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `push`'s value argument is exactly the vector's element type.
pub(super) fn legality_push(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let element = vec_element(cx)?;
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(1))? {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `push_mut` takes `T` and returns `&mut T`.
pub(super) fn legality_push_mut(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let element = vec_element(cx)?;
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(1))? {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    vec_capacity_limit(cx)?;
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: true,
        } if pointee == element.id
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `insert(&mut self, index: usize, element: T) -> ()`.
pub(super) fn legality_insert(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let element = vec_element(cx)?;
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(2))? {
        return Err(SemanticDecline::WrongArgumentType(2));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `insert_mut(&mut self, index: usize, element: T) -> &mut T`.
pub(super) fn legality_insert_mut(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let element = vec_element(cx)?;
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(2))? {
        return Err(SemanticDecline::WrongArgumentType(2));
    }
    vec_capacity_limit(cx)?;
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: true,
        } if pointee == element.id
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `reserve` / `reserve_exact` take `usize` and return `()`.
pub(super) fn legality_reserve(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `truncate` takes `usize` and returns `()`.
pub(super) fn legality_truncate(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    legality_unit_return(cx)
}

/// `pop` returns a 2-variant tagged `Option<T>` enum (`None` + `Some(T)`).
pub(super) fn legality_pop(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    option_layout(cx)?;
    Ok(())
}

/// `resize(&mut self, new_len: usize, value: T)` for trivial-`Clone` primitive elements.
pub(super) fn legality_resize(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let element = vec_element(cx)?;
    if !is_trivial_clone_primitive(element) {
        return Err(SemanticDecline::WrongArgumentType(2));
    }
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(2))? {
        return Err(SemanticDecline::WrongArgumentType(2));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `from_elem(elem: T, n: usize) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn legality_from_elem(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let element = vec_element(cx)?;
    if !is_trivial_clone_primitive(element) {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    if element.id != slot_type(cx.sig, SignatureSlot::Argument(0))? {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    legality_usize_argument_1(cx)?;
    vec_capacity_limit(cx)?;
    Ok(())
}

/// `box_assume_init_into_vec_unsafe(Box<MaybeUninit<[T; N]>>) -> Vec<T>`.
pub(super) fn legality_from_box_array(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    box_array_len(cx)?;
    Ok(())
}

/// `<&Vec<T> as IntoIterator>::into_iter` / `<&mut Vec<T> as IntoIterator>::into_iter`.
pub(super) fn legality_iter(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    slice_iter_layout(cx)?;
    Ok(())
}

/// `core::mem::replace(&mut Vec<T>, Vec<T>) -> Vec<T>`: all three slots are the exact same `Vec<T>`.
pub(super) fn legality_replace(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let receiver = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference {
        pointee,
        mutable: true,
    } = type_get(cx.table, receiver)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if pointee != slot_type(cx.sig, SignatureSlot::Argument(1))? {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    if pointee != cx.sig.result {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `core::mem::swap(&mut Vec<T>, &mut Vec<T>)`: both arguments are `&mut` to the exact same `Vec<T>`.
pub(super) fn legality_mem_swap(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let lhs = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let rhs = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    let TypeKind::Reference {
        pointee: lhs_ty,
        mutable: true,
    } = type_get(cx.table, lhs)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    let TypeKind::Reference {
        pointee: rhs_ty,
        mutable: true,
    } = type_get(cx.table, rhs)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(1));
    };
    if lhs_ty != rhs_ty {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    legality_unit_return(cx)
}

/// `extend`'s owned source is exactly the receiver's own vector type.
pub(super) fn legality_extend(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let receiver = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference { pointee, .. } = type_get(cx.table, receiver)?.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if pointee != slot_type(cx.sig, SignatureSlot::Argument(1))? {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `extend_from_slice(&mut self, other: &[T])` for trivial-`Clone` primitive elements.
pub(super) fn legality_extend_from_slice(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let element = vec_element(cx)?;
    if !is_trivial_clone_primitive(element) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    let slice_ref = type_get(cx.table, slot_type(cx.sig, SignatureSlot::Argument(1))?)?;
    let correct = matches!(
        slice_ref.kind,
        TypeKind::Reference {
            pointee,
            mutable: false,
        } if type_get(cx.table, pointee).is_ok_and(|p| matches!(p.kind, TypeKind::Slice(el) if el == element.id))
    );
    if !correct {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `<Vec<T> as Clone>::clone(&self) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn legality_clone(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let receiver = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference {
        pointee,
        mutable: false,
    } = type_get(cx.table, receiver)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if pointee != cx.sig.result {
        return Err(SemanticDecline::WrongReturnType);
    }
    let element = vec_element(cx)?;
    if !is_trivial_clone_primitive(element) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `<[T]>::to_vec(&self) -> Vec<T>` for trivial-`Clone` primitive elements.
pub(super) fn legality_slice_to_vec(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let element = vec_element(cx)?;
    if !is_trivial_clone_primitive(element) {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    let slice_ref = type_get(cx.table, slot_type(cx.sig, SignatureSlot::Argument(0))?)?;
    let correct = matches!(
        slice_ref.kind,
        TypeKind::Reference {
            pointee,
            mutable: false,
        } if type_get(cx.table, pointee).is_ok_and(|p| matches!(p.kind, TypeKind::Slice(el) if el == element.id))
    );
    if !correct {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    Ok(())
}

/// `as_slice`/`as_mut_slice` return a slice reference over the exact element, at the operation's
/// mutability.
pub(super) fn legality_as_slice(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecAsMutSlice)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let correct = matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: result_mutable,
        } if result_mutable == mutable
            && type_get(cx.table, pointee).is_ok_and(|pointee| {
                matches!(
                    pointee.kind,
                    TypeKind::Slice(element)
                        if vec_element(cx).is_ok_and(|expected| expected.id == element)
                )
            })
    );
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `Index<usize>`/`IndexMut<usize>` take a `usize` and return an element reference at the
/// operation's mutability.
pub(super) fn legality_index(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecIndexMut)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let correct = matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: result_mutable,
        } if result_mutable == mutable
            && vec_element(cx).is_ok_and(|element| element.id == pointee)
    );
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `Index<Range*<usize>>`/`IndexMut<Range*<usize>>` take a verified range struct and return a
/// slice reference over the vector's element type.
pub(super) fn legality_range_index(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    range_shape(cx)?;
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecRangeIndexMut)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let correct = matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: result_mutable,
        } if result_mutable == mutable
            && type_get(cx.table, pointee).is_ok_and(|pointee| {
                matches!(
                    pointee.kind,
                    TypeKind::Slice(element)
                        if vec_element(cx).is_ok_and(|expected| expected.id == element)
                )
            })
    );
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `remove`/`swap_remove` take a `usize` and return the element by value.
pub(super) fn legality_remove(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let correct = vec_element(cx).is_ok_and(|element| {
        element.id == cx.sig.result && element.layout.sized && !element.layout.zst
    });
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

fn legality_usize_argument_1(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !matches!(
        type_get(cx.table, argument)?.kind,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        }
    ) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    Ok(())
}

fn box_array_len(cx: &LowerCx<'_>) -> Result<u64, SemanticDecline> {
    let element = vec_element(cx)?;
    if !element.layout.sized || element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    let box_ty = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let box_desc = type_get(cx.table, box_ty)?;
    if box_desc.meta.candidate != RepresentationCandidate::Box
        || box_desc.meta.allocator != AllocatorIdentity::Global
    {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    let TypeKind::Adt { args: box_args, .. } = &box_desc.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    let inner = *box_args
        .first()
        .ok_or(SemanticDecline::WrongArgumentType(0))?;
    let array_ty = match &type_get(cx.table, cx.table.peeled(inner))?.kind {
        TypeKind::Array { .. } => cx.table.peeled(inner),
        TypeKind::Adt {
            form: NominalForm::Union,
            args,
            ..
        } => {
            let arg0 = *args.first().ok_or(SemanticDecline::WrongArgumentType(0))?;
            cx.table.peeled(arg0)
        }
        _ => return Err(SemanticDecline::WrongArgumentType(0)),
    };
    let TypeKind::Array {
        element: arr_elem,
        length,
    } = type_get(cx.table, array_ty)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if arr_elem != element.id {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    Ok(length)
}

enum SliceIterLayout {
    Legacy {
        ptr_key: String,
        end_key: String,
        marker_key: String,
    },
    Indexed {
        ptr_key: String,
        pos_key: String,
        end_key: String,
        marker_key: String,
    },
}

fn slice_iter_layout(cx: &LowerCx<'_>) -> Result<SliceIterLayout, SemanticDecline> {
    let element = vec_element(cx)?;
    if !element.layout.sized || element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecIterMut)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let expected_iter = if mutable {
        IteratorIdentity::SliceMut
    } else {
        IteratorIdentity::Slice
    };
    if result.meta.iterator != expected_iter {
        return Err(SemanticDecline::WrongReturnType);
    }
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &result.kind
    else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let [variant] = variants.as_slice() else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let key_str = |key: FieldKey| match key {
        FieldKey::Name(id) => cx
            .interns
            .str(id)
            .map(str::to_owned)
            .ok_or(SemanticDecline::WrongReturnType),
        _ => Err(SemanticDecline::WrongReturnType),
    };
    match variant.fields.as_slice() {
        [f0, f1, f2] => {
            let p0 = type_get(cx.table, cx.table.peeled(f0.ty))?;
            let p1 = type_get(cx.table, cx.table.peeled(f1.ty))?;
            let p2 = type_get(cx.table, f2.ty)?;
            if !matches!(p0.kind, TypeKind::RawPointer { pointee, .. } if pointee == element.id) {
                return Err(SemanticDecline::WrongReturnType);
            }
            if !matches!(
                p1.kind,
                TypeKind::RawPointer {
                    pointee,
                    mutable: end_mut,
                } if pointee == element.id && end_mut == mutable
            ) {
                return Err(SemanticDecline::WrongReturnType);
            }
            if !p2.layout.zst {
                return Err(SemanticDecline::WrongReturnType);
            }
            Ok(SliceIterLayout::Legacy {
                ptr_key: key_str(f0.key)?,
                end_key: key_str(f1.key)?,
                marker_key: key_str(f2.key)?,
            })
        }
        [f0, f1, f2, f3] => {
            let p0 = type_get(cx.table, cx.table.peeled(f0.ty))?;
            let p1 = type_get(cx.table, cx.table.peeled(f1.ty))?;
            let p2 = type_get(cx.table, cx.table.peeled(f2.ty))?;
            let p3 = type_get(cx.table, f3.ty)?;
            if !matches!(p0.kind, TypeKind::RawPointer { pointee, .. } if pointee == element.id) {
                return Err(SemanticDecline::WrongReturnType);
            }
            let is_usize = |k: &TypeKind| {
                matches!(
                    k,
                    TypeKind::Integer {
                        signed: false,
                        bits: 32,
                    }
                )
            };
            if !is_usize(&p1.kind) || !is_usize(&p2.kind) || !p3.layout.zst {
                return Err(SemanticDecline::WrongReturnType);
            }
            Ok(SliceIterLayout::Indexed {
                ptr_key: key_str(f0.key)?,
                pos_key: key_str(f1.key)?,
                end_key: key_str(f2.key)?,
                marker_key: key_str(f3.key)?,
            })
        }
        _ => Err(SemanticDecline::WrongReturnType),
    }
}

fn range_shape(cx: &LowerCx<'_>) -> Result<RangeShape, SemanticDecline> {
    let arg = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    let desc = type_get(cx.table, arg)?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(1));
    };
    let [variant] = variants.as_slice() else {
        return Err(SemanticDecline::WrongArgumentType(1));
    };
    let mut keys: Vec<&str> = Vec::with_capacity(variant.fields.len());
    for field in &variant.fields {
        if !matches!(
            type_get(cx.table, field.ty)?.kind,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            }
        ) {
            return Err(SemanticDecline::WrongArgumentType(1));
        }
        let FieldKey::Name(id) = field.key else {
            return Err(SemanticDecline::WrongArgumentType(1));
        };
        let Some(name) = cx.interns.str(id) else {
            return Err(SemanticDecline::WrongArgumentType(1));
        };
        keys.push(name);
    }
    match keys.as_slice() {
        [] => Ok(RangeShape::Full),
        ["start", "end"] => Ok(RangeShape::Bounds),
        ["start"] => Ok(RangeShape::From),
        ["end"] => Ok(RangeShape::To),
        _ => Err(SemanticDecline::WrongArgumentType(1)),
    }
}

fn option_layout(cx: &LowerCx<'_>) -> Result<(String, String, String), SemanticDecline> {
    let element = vec_element(cx)?;
    if !element.layout.sized || element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    let result = type_get(cx.table, cx.sig.result)?;
    if result.js.enum_repr != Some(EnumRepr::Tag { fieldless: false }) {
        return Err(SemanticDecline::WrongReturnType);
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &result.kind
    else {
        return Err(SemanticDecline::WrongReturnType);
    };
    if variants.len() != 2 {
        return Err(SemanticDecline::WrongReturnType);
    }
    let mut none_tag = None;
    let mut some_info = None;
    for variant in variants {
        let Some(tag) = cx.interns.str(variant.name) else {
            return Err(SemanticDecline::WrongReturnType);
        };
        match variant.fields.as_slice() {
            [] if none_tag.is_none() => none_tag = Some(tag.to_owned()),
            [field] if some_info.is_none() && field.ty == element.id => {
                let FieldKey::Name(key_id) = field.key else {
                    return Err(SemanticDecline::WrongReturnType);
                };
                let Some(field_name) = cx.interns.str(key_id) else {
                    return Err(SemanticDecline::WrongReturnType);
                };
                some_info = Some((tag.to_owned(), field_name.to_owned()));
            }
            _ => return Err(SemanticDecline::WrongReturnType),
        }
    }
    let (Some(none_tag), Some((some_tag, some_field))) = (none_tag, some_info) else {
        return Err(SemanticDecline::WrongReturnType);
    };
    Ok((none_tag, some_tag, some_field))
}

/// The element type of the operation's canonical `Vec<T>` — the monomorphic type argument, never
/// a name inference (v1 `vec_element`).
pub(super) fn vec_element<'a>(cx: &LowerCx<'a>) -> Result<&'a TypeDescriptor, SemanticDecline> {
    let vector_ty = match cx.semantic {
        BodySemantic::DropGlue(dropped) => dropped,
        BodySemantic::Standard(
            StandardOperation::VecNew
            | StandardOperation::VecWithCapacity
            | StandardOperation::VecFromElem
            | StandardOperation::VecFromBoxArray
            | StandardOperation::SliceToVec,
        ) => cx.sig.result,
        BodySemantic::Standard(
            StandardOperation::VecLen
            | StandardOperation::VecCapacity
            | StandardOperation::VecPush
            | StandardOperation::VecPushMut
            | StandardOperation::VecInsert
            | StandardOperation::VecInsertMut
            | StandardOperation::VecClear
            | StandardOperation::VecReserve
            | StandardOperation::VecReserveExact
            | StandardOperation::VecShrinkToFit
            | StandardOperation::VecTruncate
            | StandardOperation::VecPop
            | StandardOperation::VecResize
            | StandardOperation::VecAsSlice
            | StandardOperation::VecAsMutSlice
            | StandardOperation::VecIter
            | StandardOperation::VecIterMut
            | StandardOperation::VecReplace
            | StandardOperation::VecMemSwap
            | StandardOperation::VecIsEmpty
            | StandardOperation::VecIndex
            | StandardOperation::VecIndexMut
            | StandardOperation::VecRangeIndex
            | StandardOperation::VecRangeIndexMut
            | StandardOperation::VecRemove
            | StandardOperation::VecSwapRemove
            | StandardOperation::VecExtend
            | StandardOperation::VecExtendFromSlice
            | StandardOperation::VecClone
            | StandardOperation::VecDrop,
        ) => cx
            .sig
            .arguments
            .first()
            .copied()
            .ok_or(SemanticDecline::WrongArgumentType(0))?,
        BodySemantic::Standard(operation) => {
            return Err(SemanticDecline::UnsupportedOperation(operation));
        }
        BodySemantic::Ordinary | BodySemantic::PanicImpl => {
            return Err(SemanticDecline::NotAStandardOperation);
        }
    };
    element_of_vec_type(cx, vector_ty)
}

fn element_of_vec_type<'a>(
    cx: &LowerCx<'a>,
    mut vector_ty: TypeId,
) -> Result<&'a TypeDescriptor, SemanticDecline> {
    if let TypeKind::Reference { pointee, .. } = type_get(cx.table, vector_ty)?.kind {
        vector_ty = pointee;
    }
    let vector = type_get(cx.table, vector_ty)?;
    if vector.meta.candidate != crate::jir::types::RepresentationCandidate::Vec {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Result));
    }
    let TypeKind::Adt { args, .. } = &vector.kind else {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Result));
    };
    let element = args
        .first()
        .copied()
        .ok_or(SemanticDecline::UnknownElementLayout)?;
    type_get(cx.table, element)
}

/// The capacity ceiling of a `Vec` operation: how many elements of this size and alignment fit
/// the producer's 32-bit address space (v1 `vec_capacity_limit`).
pub(super) fn vec_capacity_limit(cx: &LowerCx<'_>) -> Result<u64, SemanticDecline> {
    let element = vec_element(cx)?;
    if !element.layout.sized {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    if element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    let (Some(size), Some(align)) = (element.layout.size, element.layout.align) else {
        return Err(SemanticDecline::UnknownElementLayout);
    };
    if size == 0 || align == 0 {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    let arg_index = if matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecFromElem)
    ) {
        1
    } else {
        0
    };
    let argument = cx
        .sig
        .arguments
        .get(arg_index)
        .copied()
        .map(|ty| type_get(cx.table, ty))
        .transpose()?
        .ok_or(SemanticDecline::WrongArgumentType(arg_index as u16))?;
    let bits = match argument.kind {
        TypeKind::Reference { .. } => argument
            .layout
            .size
            .and_then(|bytes| bytes.checked_mul(8))
            .and_then(|bits| u32::try_from(bits).ok())
            .ok_or(SemanticDecline::UnknownElementLayout)?,
        _ => unsigned_bits(argument)
            .map_err(|_| SemanticDecline::WrongArgumentType(arg_index as u16))?,
    };
    let bytes = pointer_limit(bits)?
        .checked_add(1)
        .and_then(|limit| limit.checked_sub(align))
        .ok_or(SemanticDecline::UnknownElementLayout)?;
    Ok(bytes / size)
}
