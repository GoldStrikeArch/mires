//! Lowering of MIR constants to JavaScript literals.
//!
//! Scalars (integers, floats, `bool`, `char`), zero sized constants, the `&str` literals the print
//! shim takes, and the pointer constants that reach a `static` or a promoted aggregate. Anything
//! else the JS side would have to model as memory — a `&[u8]`, a vtable — becomes a zombie rather
//! than a silent miscompilation.
//!
//! Every function here takes the codegen unit (for the namer) and a [`ZombieLog`] rather than a
//! `FnCx`: constants are also lowered for `static` initializers, which have no function around
//! them. The span is threaded explicitly for the same reason.
//!
//! # Pointers into allocations
//!
//! A constant of pointer type arrives as an allocation plus an offset, and the allocation the
//! offset points *into* is named by the pointer's provenance. [`pointer_expr`] is the one place
//! that dispatches on it: a `static` becomes the module level binding it was emitted as, a
//! function becomes the emitted function, and an anonymous allocation is decoded back into the
//! value it encodes. Provenance also has to be consulted *inside* an allocation being decoded —
//! the initializer of `static S: &str` is a pointer and a length, and neither means anything
//! without knowing which allocation the pointer belongs to.

pub(crate) mod raw;

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::interpret::{AllocId, Allocation, GlobalAlloc, Scalar};
use rustc_middle::mir::{ConstOperand, ConstValue};
use rustc_middle::ty::layout::{LayoutCx, TyAndLayout};
use rustc_middle::ty::{self, Instance, ScalarInt, Ty};
use rustc_span::Span;

use crate::cgu::CguCx;
use crate::item::{ZombieKind, ZombieLog};
use crate::jsast::{self, Expr};
use crate::naming::Namer;
use crate::value::{self, is_indirect, typing_env};

/// The byte length from which a `&str` constant is hoisted into a shared `const`.
///
/// A hoisted string costs its name (19 characters before minification, two or three after) at
/// every use, plus one declaration; an inline one costs its own length at every use. So the
/// threshold is not a break-even point for a string used *once* — it is the point where the
/// strings that repeat, which are nearly all of them, start paying: panic messages, `unsafe`
/// precondition texts and file paths are written by `core` in one place and instantiated into
/// dozens of items, and identical text in two crates collapses to one constant at link time.
/// Below it the win is too small to be worth the indirection in the emitted JavaScript.
const HOIST_STRING_BYTES: usize = 32;

/// Reports a constant the backend cannot lower, standing in for it with a runtime abort.
fn zombie(cgu: &CguCx<'_>, log: &ZombieLog, span: Span, message: String) -> Expr {
    log.record(cgu.tcx, span, ZombieKind::Constant, message)
}

/// Evaluates one `Operand::Constant` and lowers it to a JavaScript expression.
pub(crate) fn codegen_constant<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    instance: Instance<'tcx>,
    constant: &ConstOperand<'tcx>,
) -> Expr {
    let tcx = cgu.tcx;
    let konst = instance.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        typing_env(),
        ty::EarlyBinder::bind(tcx, constant.const_),
    );
    let ty = konst.ty();
    match konst.eval(tcx, typing_env(), constant.span) {
        Ok(value) => codegen_const_value(cgu, log, value, ty, constant.span),
        Err(_) => {
            zombie(cgu, log, constant.span, format!("could not evaluate constant of type `{ty}`"))
        }
    }
}

/// Lowers the initializer of a `static` item.
///
/// The initializer is const evaluated to an allocation and decoded back into the value it encodes,
/// exactly like a promoted constant: there is no memory on the JavaScript side for the bytes to
/// live in, so the `static` becomes a module level binding holding the decoded value.
pub(crate) fn codegen_static_initializer<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    def_id: DefId,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let ty = tcx
        .normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    match tcx.eval_static_initializer(def_id) {
        Ok(alloc) => read_alloc(cgu, log, alloc.inner(), Size::ZERO, ty, span),
        // An `extern` static, or one whose initializer failed to evaluate (already reported).
        Err(_) => zombie(
            cgu,
            log,
            span,
            format!("the initializer of this `static` of type `{ty}` could not be evaluated"),
        ),
    }
}

/// Lowers an already evaluated constant.
pub(crate) fn codegen_const_value<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    value: ConstValue,
    ty: Ty<'tcx>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    match value {
        ConstValue::ZeroSized => jsast::undefined(),
        ConstValue::Scalar(Scalar::Int(int)) => scalar_to_expr(cgu, log, int, ty, span),
        ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
            // A reference to a `static`, to a function, or to a promoted value like
            // `&Point { x: 1, y: 2 }`. A thin pointer, so there is no metadata to carry.
            let (provenance, offset) = pointer.prov_and_relative_offset();
            pointer_expr(cgu, log, provenance.alloc_id(), offset, ty, None, span)
        }
        ConstValue::Slice { alloc_id, meta } => {
            let GlobalAlloc::Memory(alloc) = tcx.global_alloc(alloc_id) else {
                return zombie(
                    cgu,
                    log,
                    span,
                    format!("the slice constant of type `{ty}` is not backed by an allocation"),
                );
            };
            let pointee = ty.peel_refs();
            if pointee.is_str() {
                return str_expr(cgu, log, alloc.inner(), Size::ZERO, Some(meta), span);
            }
            slice_expr(cgu, log, alloc.inner(), Size::ZERO, pointee, Some(meta), span)
        }
        // A constant whose value lives in an allocation rather than in a register: an aggregate
        // passed by value, `const C: Option<i32> = None` used directly. Decoded the same way a
        // `static`'s initializer is.
        ConstValue::Indirect { alloc_id, offset } => match tcx.global_alloc(alloc_id) {
            GlobalAlloc::Memory(alloc) => read_alloc(cgu, log, alloc.inner(), offset, ty, span),
            _ => zombie(
                cgu,
                log,
                span,
                format!("the constant of type `{ty}` is not backed by an allocation"),
            ),
        },
    }
}

/// A scalar constant, interpreted according to its Rust type.
fn scalar_to_expr<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    int: ScalarInt,
    ty: Ty<'tcx>,
    span: Span,
) -> Expr {
    let size = int.size();
    match ty.kind() {
        ty::Bool => jsast::boolean(int.to_uint(size) != 0),
        ty::Char => jsast::num(int.to_uint(size) as f64),
        // `value::scalar_int_literal` picks the representation: a number, or a `BigInt` literal
        // for the 64 and 128 bit types (S1-M2).
        ty::Int(_) | ty::Uint(_) => match value::scalar_int_literal(cgu.tcx, ty, int) {
            Some(literal) => literal,
            None => zombie(cgu, log, span, format!("`{ty}` is not an integer type")),
        },
        ty::Float(ty::FloatTy::F32) => jsast::num(f32::from_bits(int.to_uint(size) as u32) as f64),
        ty::Float(ty::FloatTy::F64) => jsast::num(f64::from_bits(int.to_uint(size) as u64)),
        // Not a primitive, but small enough that const evaluation kept it in a register rather
        // than an allocation: `None::<&mut i32>`, a one-field newtype, a fieldless enum. The bits
        // are laid out exactly as they would be in memory, so they are decoded by putting them
        // back into a scratch allocation and reading that.
        _ => match raw::scratch_alloc(cgu.tcx, int, ty) {
            Some(alloc) => read_alloc(cgu, log, &alloc, Size::ZERO, ty, span),
            None => zombie(
                cgu,
                log,
                span,
                format!("constants of type `{ty}` are not supported by rustc_codegen_mires"),
            ),
        },
    }
}

/// Rebuilds the value of type `ty` stored at `offset` in a constant allocation.
///
/// Promoted constants (`&Point { x: 1, y: 2 }`) reach codegen as bytes plus a type; since the
/// JavaScript side models values, not memory, the bytes are decoded field by field back into the
/// value they encode. Only the shapes the spike needs are decoded: scalars, structs, tuples and
/// arrays. Anything else — an enum, a nested reference — becomes a zombie.
fn read_alloc<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc: &Allocation,
    offset: Size,
    ty: Ty<'tcx>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
        return zombie(cgu, log, span, format!("no layout for `{ty}` in a constant"));
    };

    let field = |index: usize, field_ty: Ty<'tcx>| {
        read_alloc(cgu, log, alloc, offset + layout.fields.offset(index), field_ty, span)
    };

    // A transparent wrapper *is* its one non-1-ZST field (`value.rs`), so its bytes are that
    // field's bytes and its value is that field's value: a `NonNull<u8>` in a constant reads as
    // the pointer inside it, and a `MaybeUninit<u8>` as the byte. This arm comes first because it
    // covers unions, which have no shape of their own here at all.
    if let Some((index, field_ty)) = value::transparent_field_indexed(tcx, ty) {
        return field(index.as_usize(), field_ty);
    }

    match ty.kind() {
        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
            match raw::read_scalar(tcx, alloc, offset, layout.size) {
                Some(int) => scalar_to_expr(cgu, log, int, ty, span),
                None => {
                    zombie(cgu, log, span, format!("could not read a `{ty}` out of a constant"))
                }
            }
        }
        // A pattern only restricts which values inhabit the type; the bytes are the base type's.
        ty::Pat(base, _) => read_alloc(cgu, log, alloc, offset, *base, span),
        ty::Adt(def, _) if def.is_enum() => {
            read_enum(cgu, log, alloc, offset, ty, layout, span)
        }
        ty::Adt(def, args) if def.is_struct() => {
            let variant = def.non_enum_variant();
            let entries = variant
                .fields
                .iter_enumerated()
                .map(|(index, field_def)| {
                    let field_ty =
                        tcx.normalize_erasing_regions(typing_env(), field_def.ty(tcx, args));
                    (Namer::field_name(variant, index), field(index.as_usize(), field_ty))
                })
                .collect();
            jsast::object(entries)
        }
        ty::Tuple(element_tys) => {
            if element_tys.is_empty() {
                jsast::undefined()
            } else {
                jsast::array(element_tys.iter().enumerate().map(|(i, el)| field(i, el)).collect())
            }
        }
        ty::Array(element_ty, count) => {
            let count = count.try_to_target_usize(tcx).unwrap_or_default() as usize;
            jsast::array((0..count).map(|i| field(i, *element_ty)).collect())
        }
        // A pointer stored *inside* an allocation: `static S: &str`, `static P: &Point`.
        ty::Ref(..) | ty::RawPtr(..) | ty::FnPtr(..) => {
            read_pointer(cgu, log, alloc, offset, ty, span)
        }
        _ => zombie(
            cgu,
            log,
            span,
            format!("constants of type `{ty}` cannot be read out of an allocation"),
        ),
    }
}

/// Rebuilds an enum value out of a constant allocation.
///
/// Two steps, and the first is the interesting one: the bytes hold a *tag*, and which variant that
/// tag names depends on how the layout chose to encode it. A direct tag is the discriminant
/// itself; a niche tag is an otherwise-invalid value of some field, which is how `Option<&T>` fits
/// in one pointer. Once the variant is known its fields are read at that variant's own offsets,
/// and the value comes out shaped exactly like the one `rvalue.rs` builds for `Rvalue::Aggregate`,
/// through the same `value.rs` entry point.
fn read_enum<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc: &Allocation,
    offset: Size,
    ty: Ty<'tcx>,
    layout: TyAndLayout<'tcx>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let ty::Adt(def, args) = ty.kind() else {
        return zombie(cgu, log, span, format!("`{ty}` is not an enum"));
    };

    let variant_index = match raw::variant_index(tcx, alloc, offset, layout, *def) {
        Ok(index) => index,
        Err(raw::TagError::Uninhabited) => {
            return zombie(
                cgu,
                log,
                span,
                format!("a constant of the uninhabited type `{ty}` cannot be read"),
            );
        }
        Err(raw::TagError::Unreadable) => {
            return zombie(
                cgu,
                log,
                span,
                format!("could not read the discriminant of `{ty}` out of a constant"),
            );
        }
        Err(raw::TagError::NoSuchVariant(bits)) => {
            return zombie(
                cgu,
                log,
                span,
                format!(
                    "a constant of type `{ty}` has the discriminant `{bits}`, which names no \
                     variant"
                ),
            );
        }
    };

    let variant = value::variant_def(*def, Some(variant_index));
    let variant_layout = layout.for_variant(&LayoutCx::new(tcx, typing_env()), variant_index);

    let mut entries = Vec::with_capacity(variant.fields.len());
    for (index, field_def) in variant.fields.iter_enumerated() {
        let field_ty = tcx.normalize_erasing_regions(typing_env(), field_def.ty(tcx, args));
        let field_offset = offset + variant_layout.fields.offset(index.as_usize());
        entries.push((
            Namer::field_name(variant, index),
            read_alloc(cgu, log, alloc, field_offset, field_ty, span),
        ));
    }
    value::enum_value(tcx, ty, variant_index, entries)
        .unwrap_or_else(|| zombie(cgu, log, span, format!("`{ty}` is not an enum")))
}

/// Reads the pointer of type `ty` stored at `offset` in an allocation.
///
/// The bytes at `offset` are only half the pointer: they hold the offset *within* the allocation
/// it points at, and the allocation itself comes from the provenance recorded beside them. A wide
/// pointer's length follows in the next pointer sized slot.
fn read_pointer<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc: &Allocation,
    offset: Size,
    ty: Ty<'tcx>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let wide = ty.builtin_deref(true).is_some_and(raw::is_wide_pointee);
    let pointer = raw::read_pointer(tcx, alloc, offset, wide);

    let Some(alloc_id) = pointer.alloc_id() else {
        // No provenance: the bytes are an address and nothing else, which is exactly the address
        // form of the pointer model — `ptr::null()`, `without_provenance`, an integer cast to a
        // pointer. A thin one is that number. A wide one is not: `{ buf, off, len }` needs a
        // buffer, and there is none to name.
        if wide {
            return zombie(
                cgu,
                log,
                span,
                format!(
                    "a `{ty}` in a constant is an address with no provenance, and a wide pointer \
                     to nothing cannot be named by rustc_codegen_mires"
                ),
            );
        }
        return match pointer.readable {
            true => jsast::num(pointer.offset.bytes() as f64),
            false => zombie(
                cgu,
                log,
                span,
                format!("a `{ty}` in a constant is uninitialized, which rustc_codegen_mires cannot read"),
            ),
        };
    };

    pointer_expr(cgu, log, alloc_id, pointer.offset, ty, pointer.meta, span)
}

/// The JavaScript value standing in for a pointer of type `ty` into allocation `alloc_id`.
///
/// `offset` is the offset within *that* allocation, and `meta` the length of the pointee when the
/// pointer is wide.
fn pointer_expr<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc_id: AllocId,
    offset: Size,
    ty: Ty<'tcx>,
    meta: Option<u64>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let pointee = || {
        ty.builtin_deref(true).ok_or_else(|| {
            zombie(cgu, log, span, format!("the pointer constant `{ty}` is not a reference"))
        })
    };

    match tcx.global_alloc(alloc_id) {
        // A function pointer is the emitted function itself.
        GlobalAlloc::Function { instance, .. } => {
            jsast::id(cgu.namer.fn_name(instance).into_string())
        }
        // A reference to a `static`: the module level binding it was emitted as. For an indirect
        // type the binding's own JS value *is* the reference, and mutation through it is mutation
        // of that object. A primitive has no identity to point at, so such a `static` is emitted
        // *boxed* — `let S = [value]`, by `base::codegen_static` — and the reference to it is the
        // slot naming its one element.
        GlobalAlloc::Static(def_id) => {
            let pointee = match pointee() {
                Ok(pointee) => pointee,
                Err(poison) => return poison,
            };
            if offset != Size::ZERO {
                return zombie(
                    cgu,
                    log,
                    span,
                    format!(
                        "a pointer into the middle of a `static` is not supported by \
                         rustc_codegen_mires (type `{ty}`)"
                    ),
                );
            }
            let binding = jsast::id(cgu.namer.static_name(def_id).into_string());
            if is_indirect(cgu.tcx, pointee) {
                boxed_if_raw(cgu, ty, pointee, binding)
            } else {
                crate::ptr::slot(binding, jsast::num(0))
            }
        }
        // A reference to a promoted value: `&Point { x: 1, y: 2 }`, `&true`, `"text"`. The pointee
        // is rebuilt from the allocation's bytes, because the JS side has no memory to point into.
        GlobalAlloc::Memory(alloc) => {
            let pointee = match pointee() {
                Ok(pointee) => pointee,
                Err(poison) => return poison,
            };
            if pointee.is_str() {
                return str_expr(cgu, log, alloc.inner(), offset, meta, span);
            }
            if matches!(pointee.kind(), ty::Slice(_)) {
                return slice_expr(cgu, log, alloc.inner(), offset, pointee, meta, span);
            }
            let value = read_alloc(cgu, log, alloc.inner(), offset, pointee, span);
            if is_indirect(cgu.tcx, pointee) {
                boxed_if_raw(cgu, ty, pointee, value)
            } else {
                // A reference to a primitive is a slot, and a promoted value has no place of its
                // own to be a slot into — so it gets one, a fresh single element array. A write
                // through it goes nowhere, which is exactly what a promoted constant's lifetime
                // allows: nothing else can observe the array.
                crate::ptr::slot(jsast::array(vec![value]), jsast::num(0))
            }
        }
        // Vtables are the trait object milestone's; everything else has no JS meaning at all.
        _ => zombie(
            cgu,
            log,
            span,
            format!("pointer constants are not supported by rustc_codegen_mires (type `{ty}`)"),
        ),
    }
}

/// A pointer constant to an aggregate, as the pointer type it has.
///
/// A *reference* to an aggregate is the aggregate's own object; a *raw* pointer to one is a slot,
/// so that it can be offset and compared like any other raw pointer. `__rt.box` is what turns the
/// object into the one slot that stands for it — see `ptr::ref_to_raw`, whose rule this is.
fn boxed_if_raw<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    pointee: Ty<'tcx>,
    value: Expr,
) -> Expr {
    let raw = matches!(value::peel_pattern(ty).kind(), ty::RawPtr(..));
    if raw && crate::ptr::is_slot_pointee(cgu.tcx, pointee) {
        return jsast::rt_call("box", vec![value]);
    }
    value
}

/// The `&[T]` whose `meta` elements start at `offset` in `alloc`.
///
/// A slice constant is a fat pointer, and the JavaScript side has no memory for it to point into,
/// so the elements are decoded one by one into a fresh array and the slice is
/// `{ buf: [...], off: 0, len }` over it. `&[u8]` lookup tables — `core::fmt`'s hex digits, every
/// `str` classification table — are what reach this, and `fmt::Arguments`'s `pieces: &[&str]` is
/// the same shape one level up.
///
/// The array is per constant rather than per allocation: two constants naming one allocation get
/// arrays that compare unequal by `ptr::eq`. That is the compromise every promoted value in this
/// module already makes, and a constant is immutable, so no write can make the two disagree.
fn slice_expr<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc: &Allocation,
    offset: Size,
    pointee: Ty<'tcx>,
    meta: Option<u64>,
    span: Span,
) -> Expr {
    let tcx = cgu.tcx;
    let (Some(length), ty::Slice(element_ty)) = (meta, pointee.kind()) else {
        return zombie(cgu, log, span, format!("a `&{pointee}` constant with no length"));
    };
    // The slice type reached codegen monomorphized, so its element type is normalized already.
    let element_ty = *element_ty;
    let Ok(element) = tcx.layout_of(typing_env().as_query_input(element_ty)) else {
        return zombie(cgu, log, span, format!("no layout for `{element_ty}` in a slice constant"));
    };
    let stride = element.size.bytes();
    let length = usize::try_from(length).unwrap_or(usize::MAX);
    if !raw::slice_fits(alloc, offset, stride, length) {
        return zombie(
            cgu,
            log,
            span,
            format!("a `&{pointee}` constant runs past the end of its allocation"),
        );
    }
    let elements = (0..length)
        .map(|index| {
            let at = offset + Size::from_bytes(stride * index as u64);
            read_alloc(cgu, log, alloc, at, element_ty, span)
        })
        .collect();
    crate::ptr::fat_slice(jsast::array(elements), jsast::num(0), jsast::num(length as f64))
}

/// The `&str` whose bytes start at `offset` in `alloc` and run for `meta` bytes.
fn str_expr<'tcx>(
    cgu: &CguCx<'tcx>,
    log: &ZombieLog,
    alloc: &Allocation,
    offset: Size,
    meta: Option<u64>,
    span: Span,
) -> Expr {
    let Some(length) = meta else {
        return zombie(cgu, log, span, "a `&str` constant with no length".to_string());
    };
    match raw::read_str(alloc, offset, length) {
        Ok(text) if text.len() >= HOIST_STRING_BYTES => {
            // The text is its own interning key: two crates that lower the same literal name the
            // same constant, and a JavaScript string is a value, so sharing one can never be
            // observed. See `CguCx::hoist`.
            cgu.hoist(
                crate::naming::STRING_PREFIX,
                text,
                format!("string constant {}", abbreviated(text)),
                || jsast::string(text),
            )
        }
        Ok(text) => jsast::string(text),
        Err(raw::StrError::PastEnd) => zombie(
            cgu,
            log,
            span,
            "a `&str` constant runs past the end of its allocation".to_string(),
        ),
        Err(raw::StrError::NotUtf8) => {
            zombie(cgu, log, span, "string constant is not valid UTF-8".to_string())
        }
    }
}

/// A string as a one line header comment: escaped, and cut short if it is long.
///
/// The escaping is what keeps a literal containing a newline from ending the comment early and
/// spilling the rest of the text into the program as JavaScript.
fn abbreviated(text: &str) -> String {
    const LIMIT: usize = 48;
    let mut end = LIMIT.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    match end == text.len() {
        true => format!("{text:?}"),
        false => format!("{:?}...", &text[..end]),
    }
}
