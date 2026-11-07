//! Unsizing coercions: `&[T; N] -> &[T]` (slices) and `&T -> &dyn Trait` (trait objects).
//!
//! Representation contract (CONTRACT.md + plan):
//! - `&[T]` / `&mut [T]`  = `{ buf, off, len }`
//! - `&dyn Trait`         = `{ ptr, meta }` where `meta` is the vtable array
//! - a reference to a struct with an **unsized tail** = `{ ptr, meta }` where `ptr` is the record
//!   and `meta` measures the tail (`value.rs::unsized_tail`)
//! - dyn -> dyn upcasts consult `tcx.supertrait_vtable_slot`.
//!
//! The shape of the pass follows `rustc_codegen_cranelift`'s `src/unsize.rs`: strip the pointer,
//! compare the *tails* of the two pointee types (`struct_lockstep_tails_for_codegen`), and build
//! the metadata the target needs. What differs is that a fat pointer here is a JavaScript object
//! rather than a scalar pair, so a coercion is a small object literal and there is no `old_info`
//! to thread — a `dyn -> dyn` upcast simply reads `.meta` back off the value it was handed.
//!
//! A struct with an unsized tail is the one case where the lockstep tails are *not* what the
//! pointer becomes: the record stays whole and only its metadata is read off them.
//!
//! `&str` is deliberately absent: the contract keeps it a plain JavaScript string, so `&[u8] as
//! &str` and friends have no fat form to build.

use rustc_middle::ty::{self, Ty};

use crate::base::FnCx;
use crate::jsast::{self, Expr};
use crate::naming::Namer;
use crate::value::{is_zst, transparent_field, typing_env, unsized_tail};

/// `value as <to_ty>`, where the coercion makes a thin pointer fat (or re-points a fat one).
pub(crate) fn coerce_unsized<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    value: Expr,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
) -> Expr {
    let tcx = fx.tcx;
    // A pattern type is its base type in every respect that matters here. `NonNull<T>` is a struct
    // over `pattern_type!(*const T is !null)`, so every step of a `Box<T> -> Box<dyn Trait>`
    // coercion arrives wearing one.
    let from_ty = crate::value::peel_pattern(from_ty);
    let to_ty = crate::value::peel_pattern(to_ty);
    match (from_ty.kind(), to_ty.kind()) {
        // The pointer itself never changes; only the metadata beside it appears.
        (
            ty::Ref(_, source, _) | ty::RawPtr(source, _),
            ty::Ref(_, target, _) | ty::RawPtr(target, _),
        ) => {
            // A raw pointer to the sized pointee is a slot; a reference to it is the object
            // itself. The two are fattened from different places, so which one this is has to
            // come from the pointer type rather than from the tail.
            let raw = matches!(from_ty.kind(), ty::RawPtr(..));
            // A record with an unsized tail keeps the record and puts the tail's metadata beside
            // it. Its lockstep tails are the tail *field's* two types, which say how long the tail
            // is and nothing about where the record lives, so they are read for the metadata only.
            if unsized_tail(tcx, *target).is_some() {
                return fatten_tail(fx, value, *source, *target, raw);
            }
            let (source, target) =
                tcx.struct_lockstep_tails_for_codegen(*source, *target, typing_env());
            fatten(fx, value, source, target, raw)
        }
        // A wrapper that *is* the pointer inside it, which is every step of `Box<T>` down to the
        // raw pointer: `Box` to `Unique` to `NonNull` to `*const T`. Its value is that pointer's
        // value, so the coercion is the field's coercion and nothing is rebuilt around the result.
        (ty::Adt(..), ty::Adt(..))
            if transparent_field(tcx, from_ty).is_some()
                && transparent_field(tcx, to_ty).is_some() =>
        {
            let from_field = transparent_field(tcx, from_ty).unwrap();
            let to_field = transparent_field(tcx, to_ty).unwrap();
            coerce_unsized(fx, value, from_field, to_field)
        }
        // A wrapper around a pointer, coerced by coercing the one field that is not a 1-ZST.
        (ty::Adt(def_a, args_a), ty::Adt(def_b, args_b)) if def_a == def_b => {
            let variant = def_a.non_enum_variant();
            let mut entries = Vec::with_capacity(variant.fields.len());
            let mut coerced = false;

            for (idx, field) in variant.fields.iter_enumerated() {
                let key = Namer::field_name(variant, idx);
                let read = jsast::member(value.clone(), key.clone());
                let field_from =
                    tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args_a));
                let field_to = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args_b));

                // Everything but the pointer field is carried across unchanged.
                if field_from == field_to || is_1zst(fx, field_from) {
                    entries.push((key, read));
                    continue;
                }
                if coerced {
                    return fx.zombie(format!(
                        "`{from_ty}` has more than one field to unsize, which \
                         rustc_codegen_mires does not support"
                    ));
                }
                coerced = true;
                entries.push((key, coerce_unsized(fx, read, field_from, field_to)));
            }

            if coerced {
                jsast::object(entries)
            } else {
                fx.zombie(format!(
                    "cannot unsize `{from_ty}` to `{to_ty}`: no field of it changes type"
                ))
            }
        }
        _ => fx.zombie(format!(
            "unsizing coercions are not supported by rustc_codegen_mires (`{from_ty}` to `{to_ty}`)"
        )),
    }
}

/// Builds the fat pointer for a pointee that is a **struct with an unsized tail**.
///
/// `&mut PolymorphicIter<[MaybeUninit<T>; N]> -> &mut PolymorphicIter<[MaybeUninit<T>]>` is the
/// coercion, and `{ ptr, meta }` is the answer: the record is unchanged and `meta` measures the
/// tail, exactly as a trait object's `meta` names its vtable. The record itself is *not* the tail,
/// which is what separates this from [`fatten`]: fattening the lockstep tails would hand back a
/// fat slice over the whole record, and the dereference on the other side reads the record as the
/// struct it is.
///
/// The data half is the **reference** form of the record, even when the pointer being coerced is a
/// raw one, for the same reason a trait object's is: everything past the coercion projects fields
/// out of a record rather than reading a slot.
fn fatten_tail<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    value: Expr,
    source: Ty<'tcx>,
    target: Ty<'tcx>,
    raw: bool,
) -> Expr {
    let tcx = fx.tcx;
    let (source_tail, target_tail) =
        tcx.struct_lockstep_tails_for_codegen(source, target, typing_env());
    let meta = match (source_tail.kind(), target_tail.kind()) {
        (ty::Array(_, len), ty::Slice(_)) => match len.try_to_target_usize(tcx) {
            Some(len) => jsast::num(len as f64),
            None => {
                return fx
                    .zombie(format!("the length of `{source_tail}` is not known at compile time"));
            }
        },
        _ => {
            return fx.zombie(format!(
                "cannot unsize `{source}` to `{target}`: a struct tail going from \
                 `{source_tail}` to `{target_tail}` is not supported by rustc_codegen_mires"
            ));
        }
    };
    let record = if raw { crate::ptr::raw_to_ref(fx, source, value) } else { value };
    crate::ptr::fat_dyn(record, meta)
}

/// Builds the fat pointer for a pointer whose pointee goes from `source` to `target`.
///
/// `source` and `target` are the *tails*: the sized type being pointed at and the unsized type it
/// is being seen as. `raw` says whether the value being coerced is a raw pointer, which for a
/// pointee with an object of its own is a `{ buf, off }` slot rather than that object.
fn fatten<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    value: Expr,
    source: Ty<'tcx>,
    target: Ty<'tcx>,
    raw: bool,
) -> Expr {
    let tcx = fx.tcx;
    match (source.kind(), target.kind()) {
        // `&[T; N] -> &[T]`: the array is the buffer, the length is static.
        (ty::Array(_, len), ty::Slice(_)) => {
            let len = match len.try_to_target_usize(tcx) {
                Some(len) => len,
                None => {
                    fx.zombie(format!("the length of `{source}` is not known at compile time"));
                    0
                }
            };
            if !raw {
                return crate::ptr::fat_slice(value, jsast::num(0), jsast::num(len as f64));
            }
            // From a raw pointer the array is not in hand, and where its elements live is a
            // question only the record can answer: a plain slot holds the whole array as its one
            // element, while a window slot already spans `N` elements of somebody else's buffer.
            // `Box<[T; N]>` is the second shape -- a heap block keyed to `T` -- so the two cannot
            // be told apart from the type. `__rt.unwindow_slice` asks, exactly as `__rt.unwindow`
            // does for the thin cast.
            jsast::rt_call("unwindow_slice", vec![value, jsast::num(len as f64)])
        }
        // `&dyn Sub -> &dyn Super`: a trait upcast, which swaps in a supertrait vtable.
        (ty::Dynamic(data_a, _), ty::Dynamic(data_b, _)) => {
            let principal_b = data_b.principal_def_id();
            if data_a.principal_def_id() == principal_b || principal_b.is_none() {
                // Same principal (or none to speak of): nothing about the value changes.
                return value;
            }
            match tcx.supertrait_vtable_slot((source, target)) {
                Some(slot) => jsast::object_of(vec![
                    ("ptr", jsast::member(value.clone(), "ptr")),
                    (
                        "meta",
                        jsast::index(jsast::member(value, "meta"), jsast::num(slot as f64)),
                    ),
                ]),
                // The supertrait's entries are a prefix of this vtable, so it doubles as its own
                // upcast: the slot numbering the callee uses is already right.
                None => value,
            }
        }
        // `&T -> &dyn Trait`: the concrete value is the pointer, the vtable is the metadata.
        //
        // The data half is the *reference*, even when the pointer being coerced is a raw one: a
        // vtable's entries take the receiver the way every other function does, and a virtual call
        // has no pointee type left to decide anything by.
        (_, ty::Dynamic(data, ..)) => {
            let principal = data
                .principal()
                .map(|principal| tcx.instantiate_bound_regions_with_erased(principal));
            let meta = crate::vtable::vtable_expr(fx.cgu, source, principal);
            let value = if raw { crate::ptr::raw_to_ref(fx, source, value) } else { value };
            crate::ptr::fat_dyn(value, meta)
        }
        _ => fx.zombie(format!("cannot unsize `{source}` to `{target}`")),
    }
}

/// Whether a field takes no space *and* imposes no alignment, the way `unsize_ptr` means it.
///
/// Alignment is not modeled anywhere else in the backend, so a zero sized field is treated as a
/// 1-ZST: an over-aligned marker field beside a pointer is not a shape this backend can build.
fn is_1zst<'tcx>(fx: &FnCx<'_, 'tcx>, ty: Ty<'tcx>) -> bool {
    is_zst(fx.tcx, ty)
}
