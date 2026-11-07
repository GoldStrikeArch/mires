//! The pointer ABI: how a Rust pointer is represented in JavaScript, and the one place every
//! pointer producing part of the backend goes through.
//!
//! `CONTRACT.md`, section "Pointers", is the specification this module implements; the doc comment
//! on each function repeats the part of it that function owns.
//!
//! # The model
//!
//! A pointer is a **slot**: a container plus a key, `{ buf, off }`, where `buf` is a JavaScript
//! array or object and `off` is a number index or a string property key. `off` counts *units of
//! the pointee type*, not bytes. Reading through the pointer is `p.buf[p.off]`, which is an
//! assignable JavaScript place, so a write through a pointer is an ordinary assignment and every
//! alias sees it.
//!
//! Every pointer producing operation either names a real slot or degrades to a provenance free
//! number, and nothing in between.
//!
//! # Forms
//!
//! | form | shape | what it is |
//! |---|---|---|
//! | slot | `{ buf, off }` | a pointer to one place; dereferenceable |
//! | scaled slot | `{ buf, off, sc }` | a byte offset view of a `sc`-sized pointee, from an `as *const u8` cast; **not** dereferenceable |
//! | window slot | `{ buf, off, w }` | a pointer to `w` consecutive elements, from an `as *const [E; N]` cast |
//! | fat slice | `{ buf, off, len }` | `&[T]`, `*const [T]` |
//! | fat dyn | `{ ptr, meta }` | `&dyn Trait`, where `meta` is the vtable array |
//! | address | a JS number | `null()`, `NonNull::dangling()`, `without_provenance`, an integer that
//!   was transmuted to a pointer; never dereferenceable |
//! | object | the pointee's own JS value | a reference to an aggregate, which keeps its identity |
//! | string | a JS string | `&str` |
//!
//! A slot record is **immutable**: [`add`] returns a new record rather than moving `off`. That is
//! what lets `value::needs_clone` keep excluding pointers.
//!
//! # References and raw pointers
//!
//! The two differ for aggregate pointees only:
//!
//! * `&T` and `&mut T` of an aggregate are the pointee's own JS object. Dereferencing is the
//!   identity, and mutation works because JS objects have reference semantics.
//! * `*const T` and `*mut T` of an aggregate are a slot. [`ref_to_raw`] of a bare object goes
//!   through `__rt.box`, a `WeakMap` keyed identity map, so that two raw pointers made from one
//!   object compare equal; [`raw_to_ref`] is `p.buf[p.off]`.
//! * For a pointee that is a JS primitive (a number, a `bool`, a `char`, a function pointer) there
//!   is no object to point at, so *every* pointer to it is a slot and the pointed-at local is
//!   boxed.
//!
//! # Status
//!
//! Wave 0 landed this API with the behavior the backend had before the slot model, so that the
//! parallel pointer workstreams have a stable surface to build against. A function whose body is
//! not the specification says so, in a `Not implemented yet` paragraph that states what it owes.
//! No function panics: an operation the backend cannot express records a zombie instead.

// Wave 0 lands the surface ahead of its callers; the wiring arrives with the workstreams that own
// each operation.
#![allow(dead_code)]


use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::base::FnCx;
use crate::jsast::{self, BinOp as JsBinOp, Expr, Stmt};
use crate::value::{is_indirect, is_zst, peel_pattern, peel_transparent, typing_env};

/// The buffer half of a slot.
pub(crate) const BUF: &str = "buf";
/// The offset half of a slot, counted in units of the pointee type.
pub(crate) const OFF: &str = "off";
/// The element size a scaled slot's byte offset is measured against.
pub(crate) const SCALE: &str = "sc";
/// The element count a window slot spans.
pub(crate) const WINDOW: &str = "w";
/// The length of a fat slice pointer.
pub(crate) const LEN: &str = "len";
/// The data half of a fat trait object pointer.
pub(crate) const PTR: &str = "ptr";
/// The vtable half of a fat trait object pointer.
pub(crate) const META: &str = "meta";

// ---------------------------------------------------------------------------------------------
// Building and taking apart the records
// ---------------------------------------------------------------------------------------------

/// `{ buf, off }`: a pointer to `buf[off]`.
pub(crate) fn slot(buf: Expr, off: Expr) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off)])
}

/// `{ buf, off, sc }`: a byte offset view of a buffer whose elements are `scale` bytes wide.
///
/// This is what `p as *const u8` produces for a pointee larger than a byte. `off` is a *byte*
/// offset, so the element it names is `buf[off / sc]` and only an offset that is a multiple of
/// `sc` names one at all. A scaled slot is not dereferenceable: it is meant to be offset and then
/// cast back, which is what the `__rt.unscale` helper does.
pub(crate) fn scaled_slot(buf: Expr, off: Expr, scale: u64) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off), (SCALE, jsast::num(scale as f64))])
}

/// `{ buf, off, w }`: a pointer to the `width` elements starting at `buf[off]`.
///
/// This is what `p as *const [E; N]` produces. The window is how a pointer to an array of
/// elements stays inside the buffer those elements live in, rather than copying them out.
pub(crate) fn window_slot(buf: Expr, off: Expr, width: u64) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off), (WINDOW, jsast::num(width as f64))])
}

/// `{ buf, off, len }`: a fat pointer to `len` elements starting at `buf[off]`.
pub(crate) fn fat_slice(buf: Expr, off: Expr, len: Expr) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off), (LEN, len)])
}

/// `{ ptr, meta }`: a fat pointer to a trait object.
pub(crate) fn fat_dyn(data: Expr, meta: Expr) -> Expr {
    jsast::object_of(vec![(PTR, data), (META, meta)])
}

/// `p.buf`.
pub(crate) fn slot_buf(pointer: Expr) -> Expr {
    jsast::member(pointer, BUF)
}

/// `p.off`.
pub(crate) fn slot_off(pointer: Expr) -> Expr {
    jsast::member(pointer, OFF)
}

/// `p.sc`, the element size a scaled slot's offset is measured against.
pub(crate) fn slot_scale(pointer: Expr) -> Expr {
    jsast::member(pointer, SCALE)
}

/// `p.w`, the element count a window slot spans.
pub(crate) fn slot_window(pointer: Expr) -> Expr {
    jsast::member(pointer, WINDOW)
}

/// `p.len`, the length of a fat slice pointer.
pub(crate) fn slot_len(pointer: Expr) -> Expr {
    jsast::member(pointer, LEN)
}

/// `p.buf[p.off]`: the place a slot points at, as an assignable JavaScript expression.
///
/// `pointer` is evaluated twice, so a caller holding anything but a local or a field hoists it
/// into a temporary first ([`FnCx::temp`]).
pub(crate) fn slot_element(pointer: Expr) -> Expr {
    jsast::index(slot_buf(pointer.clone()), slot_off(pointer))
}

// ---------------------------------------------------------------------------------------------
// Reading and writing through a pointer
// ---------------------------------------------------------------------------------------------

/// The value behind `pointer`, which is a **raw** pointer to `pointee`.
///
/// This is the read half of `*p` for the intrinsics that take a pointer argument (`read_via_copy`,
/// `volatile_load`, the atomics, `typed_swap_nonoverlapping`), every one of which is declared over
/// `*const T`/`*mut T`. The place walk in `place.rs` reaches the same expression through a `Deref`
/// projection, where the pointer may also be a reference and the two forms part ways for an
/// aggregate pointee — see the module documentation.
///
/// A slot reads [`slot_element`], a fat pointer is its own record (the projections that follow read
/// `buf`/`off`/`len`), and a `&str` is the string. A scaled slot or an address does not name a
/// place at all, and reading one is a `TypeError` where the mistake is rather than a wrong answer.
pub(crate) fn deref_read<'tcx>(fx: &FnCx<'_, 'tcx>, pointee: Ty<'tcx>, pointer: Expr) -> Expr {
    if is_zst(fx.tcx, pointee) {
        return jsast::undefined();
    }
    // An array pointee is the one case where the slot alone does not say where the elements are:
    // a plain slot holds the whole array as its one element, while the window slot an
    // `as *const [E; N]` cast produced spans `N` elements of somebody else's buffer. `Box<[E; N]>`
    // is the second shape. `__rt.read_array` asks, exactly as the place walk in `place.rs` does.
    if let ty::Array(_, count) = peel_pattern(peel_transparent(fx.tcx, pointee)).kind() {
        if is_slot_pointee(fx.tcx, pointee) {
            let width = count.try_to_target_usize(fx.tcx).unwrap_or(0);
            return jsast::rt_call("read_array", vec![pointer, jsast::num(width as f64)]);
        }
    }
    if is_slot_pointee(fx.tcx, pointee) { slot_element(pointer) } else { pointer }
}

/// The statements that store `value` through `pointer`, a **raw** pointer to `pointee`.
///
/// The write half of [`deref_read`], and its exact counterpart. Four cases, in this order:
///
/// 1. a zero sized pointee stores nothing, but `value` may still be a call worth making;
/// 2. `&mut str` has no mutable JavaScript form, so a write through one is a zombie;
/// 3. an aggregate pointee is overwritten in place by `__rt.overwrite` *at the slot it names*, so
///    that every alias of the aggregate sees the new contents; assigning the slot would rebind
///    only the one container the pointer came from;
/// 4. anything else is an assignment to [`slot_element`].
pub(crate) fn deref_write<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    pointer: Expr,
    value: Expr,
) -> Vec<Stmt> {
    if is_zst(fx.tcx, pointee) {
        // Nothing to store, but the value may still be a call worth making.
        return vec![jsast::expr_stmt(value)];
    }
    if !is_slot_pointee(fx.tcx, pointee) {
        // `str`, and the unsized pointees, whose pointer is the value itself.
        return write_indirect(fx, pointee, pointer, value);
    }
    // The write half of the array question above.
    if matches!(peel_pattern(peel_transparent(fx.tcx, pointee)).kind(), ty::Array(..)) {
        return vec![jsast::expr_stmt(jsast::rt_call("write_array", vec![pointer, value]))];
    }
    let element = slot_element(pointer);
    if is_indirect(fx.tcx, pointee) {
        return write_indirect(fx, pointee, element, value);
    }
    vec![jsast::assign_stmt(element, value)]
}

/// The statements that store `value` into `target`, where `target` is the pointee's own JS value.
///
/// This is what a write through a *reference* to an aggregate is, and the half of [`deref_write`]
/// that a raw pointer reaches after its slot has been resolved. `place.rs` comes here for a
/// `JsPlace::Deref`.
pub(crate) fn write_indirect<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    target: Expr,
    value: Expr,
) -> Vec<Stmt> {
    if is_zst(fx.tcx, pointee) {
        return vec![jsast::expr_stmt(value)];
    }
    // A `&str` is a JavaScript string, which has no mutable form to write through.
    if matches!(peel_pattern(pointee).kind(), ty::Str) {
        return vec![jsast::expr_stmt(
            fx.zombie("mutating a `&mut str` is not supported by rustc_codegen_mires".to_string()),
        )];
    }
    // Overwrite the aggregate in place so every alias sees the new contents. Not
    // `Object.assign`: writing a smaller enum variant over a larger one has to drop the keys
    // the old variant had, or the payload of the old one survives beside the new discriminant.
    //
    // Where the target is an assignable *place* rather than a bare value, `__rt._put` is the one
    // that answers, because such a place may hold no object at all yet. A heap block comes back
    // from `alloc` uninitialized and `Box::new` writes the whole pointee straight into it; there
    // is nothing there to overwrite, and the value has to be stored. `_put` overwrites in place
    // wherever there *is* an object, so the aliasing rule is unchanged.
    let call = match target {
        Expr::Index(base, key) => jsast::rt_call("_put", vec![*base, *key, value]),
        Expr::Member(base, name) => {
            jsast::rt_call("_put", vec![*base, jsast::string(name), value])
        }
        target => jsast::rt_call("overwrite", vec![target, value]),
    };
    vec![jsast::expr_stmt(call)]
}

// ---------------------------------------------------------------------------------------------
// Arithmetic, comparison and addresses
// ---------------------------------------------------------------------------------------------

/// `pointer.add(count)`: the pointer moved by `count` elements of `pointee`.
///
/// `MIR`'s `BinOp::Offset` and the `offset` intrinsic land here; the wrapping form,
/// `arith_offset`, is [`wrapping_add`].
///
/// A slot record is immutable, so the result is a *new* record over the same buffer,
/// `{ buf: p.buf, off: p.off + count }`, with the offset counted in pointee units. `count` is a
/// signed element count, so a subtraction reaches this with a negated operand. The inverse is
/// `ptr_offset_from`, which is `a.off - b.off` in elements and `(a.off - b.off) * size` in bytes.
///
/// A scaled slot's offset is already in bytes, and its pointee is a byte, so `off + count` is the
/// same expression — but `sc` has to survive the copy or the slot could never be unscaled again.
/// It is carried only where the pointee is one byte wide, which is the only pointee a scaled slot
/// can have.
///
/// **A pointer that is an address gets a record here rather than `address + count * size`**, and
/// that is a deliberate division of labour with [`wrapping_add`]. The three forms cannot be told
/// apart at compile time, so answering for all of them costs a run time test at every offset —
/// which also costs the peephole that folds `{ buf: p.buf, off: p.off + n }.buf[...]` down to
/// `p.buf[p.off + n]`, the shape every slice loop is made of. This function is the *strict* offset
/// (`ptr::add`, `ptr::offset`, `BinOp::Offset`), where Rust requires the pointer to be derived
/// from an allocated object whenever the count is non-zero: an address cannot reach it in a
/// program that is not already undefined behaviour, and a zero count folds to the pointer above.
/// The wrapping form, which Rust *does* allow on a provenance free pointer, pays for the test.
///
/// A degenerate record built here anyway — `null().add(1)` in a program that is already wrong —
/// is not silently answered: it has an `off` and no `buf`, and `__rt.addr`, `__rt.ptr_eq` and
/// `__rt.ptr_cmp` throw when they are handed one.
pub(crate) fn add<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    pointer: Expr,
    count: Expr,
) -> Expr {
    // `p.add(0)` is `p`, which is worth folding: an empty range's `end` is spelled that way.
    if count == jsast::num(0) {
        return pointer;
    }
    let size = size_of(fx.tcx, pointee);
    // Offsetting a pointer to a zero sized type moves it by no bytes at all.
    if size == Some(0) {
        return pointer;
    }
    let off = jsast::binary(JsBinOp::Add, slot_off(pointer.clone()), count);
    match size {
        Some(1) => jsast::object_of(vec![
            (BUF, slot_buf(pointer.clone())),
            (OFF, off),
            (SCALE, slot_scale(pointer)),
        ]),
        _ => slot(slot_buf(pointer), off),
    }
}

/// `pointer.wrapping_add(count)`: the same offset, on a pointer that may be an address.
///
/// `arith_offset` — which is what `ptr::wrapping_add`, `wrapping_offset` and `wrapping_byte_add`
/// call — is the offset Rust allows on a pointer with no provenance, so this is the one place that
/// has to tell a record from an address, and `__rt.offset(p, count, size)` does it at run time: a
/// number moves by `count * size` bytes, a slot gets a new record, and a scaled slot keeps its
/// `sc`. The strict form ([`add`]) stays inline for the sake of the fold; the division of labour
/// is spelled out there.
///
/// The address arm is not a curiosity: a slice of a zero sized element counts its length by
/// `wrapping_byte_add`ing a dangling pointer, and that is arithmetic on a number.
pub(crate) fn wrapping_add<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    pointer: Expr,
    count: Expr,
) -> Expr {
    if count == jsast::num(0) {
        return pointer;
    }
    // The size is the multiplier the address arm needs; a slot never looks at it. An unsized
    // pointee cannot reach a thin offset at all, so the fallback is only there to have one.
    let size = size_of(fx.tcx, pointee).unwrap_or(1);
    jsast::rt_call("offset", vec![pointer, count, jsast::num(size as f64)])
}

/// `a.offset_from(b)`: how many `pointee`s apart two pointers into one buffer are.
///
/// `off` counts pointee units, so this is exactly `a.off - b.off` — no division, and no rounding
/// to worry about in either direction. It answers for a scaled slot too, whose `off` is in bytes
/// and whose pointee is a byte.
///
/// `ptr_offset_from_unsigned` is the same expression: Rust promises the difference is
/// non-negative, and a `usize` is the same JavaScript number a non-negative `isize` is.
///
/// Two pointers into *different* buffers are undefined behaviour in Rust, and the subtraction
/// answers with whatever the two unrelated offsets happen to be rather than paying for a check on
/// every call. A zero sized pointee makes the operation undefined behaviour as well (Rust divides
/// by the size); such a pointer is an address here, and subtracting the two addresses is both the
/// closest thing to an answer and what the zero sized slice iterator wants.
pub(crate) fn offset_from<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    lhs: Expr,
    rhs: Expr,
) -> Expr {
    if size_of(fx.tcx, pointee) == Some(0) {
        return jsast::binary(JsBinOp::Sub, lhs, rhs);
    }
    // An **aggregate** pointee goes through the helper, because one of the two records may not
    // know where it is. A reference to an aggregate is the object itself, so a raw pointer taken
    // from one that arrived as a value — a function parameter, the result of a call — is
    // `__rt.box`, a buffer of one, and subtracting its offset from a buffer offset answers
    // nonsense. `core`'s `choose_pivot` is written exactly that way: `median3` *returns* one of
    // three `&T`s and the caller asks which index it was, so every sort of a struct depends on it.
    // The helper finds the object in the buffer the other pointer names.
    if is_indirect(fx.tcx, pointee) {
        return jsast::rt_call("offset_from", vec![lhs, rhs]);
    }
    jsast::binary(JsBinOp::Sub, slot_off(lhs), slot_off(rhs))
}

/// `lhs == rhs` on two pointers of type `pointer_ty`.
///
/// `__rt.ptr_eq(a, b)` is `===` first (so two references to one object, and two numbers, answer
/// immediately), `false` when either side is not an object, and a comparison of `buf`, `off` and
/// `sc` otherwise. `len` is deliberately ignored, which makes comparing a fat pointer with a thin
/// one free.
///
/// Every `==` on a pointer routes here, including the `compares_by_value` checks in `rvalue.rs`
/// and `intrinsics.rs`.
pub(crate) fn eq<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointer_ty: Ty<'tcx>,
    lhs: Expr,
    rhs: Expr,
) -> Expr {
    let _ = (fx, pointer_ty);
    jsast::rt_call("ptr_eq", vec![lhs, rhs])
}

/// `lhs != rhs` on two pointers of type `pointer_ty`: the negation of [`eq`].
pub(crate) fn ne<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointer_ty: Ty<'tcx>,
    lhs: Expr,
    rhs: Expr,
) -> Expr {
    jsast::not(eq(fx, pointer_ty, lhs, rhs))
}

/// `lhs < rhs` and its siblings on two pointers of type `pointer_ty`. `op` is one of
/// [`JsBinOp::Lt`], [`JsBinOp::Le`], [`JsBinOp::Gt`] or [`JsBinOp::Ge`].
///
/// `__rt.ptr_cmp(a, b, size)` compares `off` when both sides share a buffer and their [`addr`]
/// otherwise, returns -1, 0 or 1, and is compared against zero with `op`.
pub(crate) fn cmp<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointer_ty: Ty<'tcx>,
    op: JsBinOp,
    lhs: Expr,
    rhs: Expr,
) -> Expr {
    let size = pointer_ty
        .builtin_deref(true)
        .and_then(|pointee| size_of(fx.tcx, pointee))
        .unwrap_or(1);
    jsast::binary(
        op,
        jsast::rt_call("ptr_cmp", vec![lhs, rhs, jsast::num(size as f64)]),
        jsast::num(0),
    )
}

/// The address of `pointer`, as the integer a `ptr as usize` cast produces.
///
/// Not implemented yet: the pointer value is handed back as it is, which is exact for an address
/// and meaningless for a record. The specification: `__rt.addr(p, size_of::<pointee>())`, which
/// assigns every buffer a synthetic base address of `4096 * n` on first sight (a `WeakMap`, so the
/// same buffer keeps its base) and returns `base + off * size`, or `base + off` for a scaled slot,
/// or the number itself for an address.
///
/// The scheme is chosen for four guarantees rather than convenience:
///
/// * `is_null()` is false for every real pointer and true only for `null()`, because a base is
///   never zero;
/// * `NonNull::dangling()` is an alignment, which is below 4096, so it never collides with a base;
/// * an address is a multiple of the element size, so alignment predicates answer correctly;
/// * `fmt::Arguments::as_str` tests `bits & 1`, and its element is 8 bytes wide, so the answer is
///   deterministic.
pub(crate) fn addr<'tcx>(fx: &FnCx<'_, 'tcx>, pointee: Ty<'tcx>, pointer: Expr) -> Expr {
    let size = size_of(fx.tcx, pointee).unwrap_or(1).max(1);
    jsast::rt_call("addr", vec![pointer, jsast::num(size as f64)])
}

// ---------------------------------------------------------------------------------------------
// Runs of elements
// ---------------------------------------------------------------------------------------------

/// `ptr::copy(src, dst, count)` and `ptr::copy_nonoverlapping(src, dst, count)`, as the statement
/// that performs the move. `overlapping` picks between the two.
///
/// The helper moves `count` *elements*, because `off` counts elements: `__rt.copy` is correct when
/// the two ranges overlap (it copies away from the overlap, or hands the whole run to
/// `copyWithin`), and `__rt.copy_nonoverlapping` is the same loop without the direction test.
///
/// A byte for byte copy of an aggregate produces an independent value, and assigning a JavaScript
/// object would produce an alias, so an aggregate element is *cloned* on the way across — the
/// helper takes the clone as a function, built here from `value::clone_expr`, and the argument is
/// absent for the element types that copy by value.
///
/// The two pointers must agree on their scale: a copy between a plain slot and the byte view of a
/// wider buffer has no meaning in a model where a buffer holds values rather than bytes, and the
/// helper throws rather than guessing.
pub(crate) fn copy<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    dst: Expr,
    src: Expr,
    count: Expr,
    overlapping: bool,
) -> Vec<Stmt> {
    let tcx = fx.tcx;
    // Moving a run of zero sized values moves nothing at all; the arguments are places, so there
    // is nothing left to evaluate either.
    if is_zst(tcx, pointee) {
        return Vec::new();
    }
    let name = if overlapping { "copy" } else { "copy_nonoverlapping" };
    let mut args = vec![dst, src, count];
    if crate::value::needs_clone(tcx, pointee) {
        args.push(jsast::arrow(
            vec![CLONE_PARAM.to_string()],
            crate::value::clone_expr(tcx, pointee, jsast::id(CLONE_PARAM)),
        ));
    }
    vec![jsast::expr_stmt(jsast::rt_call(name, args))]
}

/// The parameter of the clone function [`copy`] hands the runtime, named the way `value.rs` names
/// the one it builds for an array copy: `$`-prefixed, so it cannot shadow a local.
const CLONE_PARAM: &str = "$e";

/// `ptr::write_bytes(dst, byte, count)`, as the statement that performs the fill.
///
/// A byte pattern only names a value in a model where a value is bytes. Two cases do survive the
/// translation, and they are the two the standard library actually uses:
///
/// * a one byte pointee, where the pattern *is* the value — sign extended for an `i8`, so that
///   `write_bytes(p, 0xff, n)` leaves `-1`s behind;
/// * a zero pattern on any other scalar, which is that scalar's zero — `0`, `0n`, `false`, or the
///   null address for a pointer. The test that the pattern really is zero is left to run time,
///   because the byte is rarely a literal by the time it arrives here.
///
/// An aggregate pointee is a zombie: `{ x: 0, y: 0 }` is not something a byte count can describe,
/// and a program that zeroes a struct through a raw pointer is doing something this model has no
/// answer for.
pub(crate) fn write_bytes<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    dst: Expr,
    byte: Expr,
    count: Expr,
) -> Vec<Stmt> {
    let tcx = fx.tcx;
    if is_zst(tcx, pointee) {
        return Vec::new();
    }
    let pointee = peel_pattern(pointee);
    let one_byte = size_of(tcx, pointee) == Some(1)
        && matches!(pointee.kind(), ty::Int(_) | ty::Uint(_) | ty::Bool);
    let args = if one_byte {
        // `bool` is one byte and its only valid patterns are 0 and 1, which `!!` maps to `false`
        // and `true`; an `i8` needs the pattern read as signed.
        let value = match pointee.kind() {
            ty::Bool => jsast::not(jsast::not(byte)),
            _ => match crate::value::int_info(tcx, pointee) {
                Some((true, bits)) => {
                    crate::value::mask(crate::value::int_repr(pointee), true, bits, byte)
                }
                _ => byte,
            },
        };
        vec![dst, value, count]
    } else {
        let Some(zero) = zero_value(tcx, pointee) else {
            return vec![jsast::expr_stmt(fx.zombie(format!(
                "`write_bytes` on `{pointee}` is not supported by rustc_codegen_mires: a byte \
                 pattern names a value only for a one byte pointee, and zero for a scalar one"
            )))];
        };
        vec![dst, byte, count, zero]
    };
    vec![jsast::expr_stmt(jsast::rt_call("write_bytes", args))]
}

/// The value a zeroed `ty` holds, for the types where zeroing means something here.
fn zero_value<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Expr> {
    match ty.kind() {
        ty::Int(_) | ty::Uint(_) => crate::value::int_literal(tcx, ty, 0),
        ty::Float(_) | ty::Char => Some(jsast::num(0)),
        ty::Bool => Some(jsast::boolean(false)),
        // A zeroed pointer is the null address, which is the number zero.
        ty::RawPtr(..) => Some(jsast::num(0)),
        _ => None,
    }
}

/// `compare_bytes(a, b, count)`: the lexicographic comparison of two runs of bytes, as a negative,
/// zero or positive number.
///
/// The intrinsic is declared over `*const u8`, so both pointers arrive as whatever the cast to a
/// byte pointer produced, and `__rt.compare_bytes` reads the answer off that: two unscaled byte
/// buffers compare element by element, because for a byte buffer the byte count *is* the element
/// count, which is what makes the bytewise `PartialEq` of `str` and `[u8]` exact; two scaled slots
/// of equal `sc` compare `count / sc` elements, which is what makes `[i32] == [i32]` work; and a
/// pair that agrees on neither throws rather than answering about bytes that do not exist.
pub(crate) fn compare_bytes(lhs: Expr, rhs: Expr, count: Expr) -> Expr {
    jsast::rt_call("compare_bytes", vec![lhs, rhs, count])
}

// ---------------------------------------------------------------------------------------------
// Converting between pointer forms
// ---------------------------------------------------------------------------------------------

/// `value as *const to_pointee`, where `value` points at `from_pointee`.
///
/// Every pointer to pointer cast goes through here: `CastKind::PtrToPtr`, `FnPtrToPtr`,
/// `ArrayToPointer`, `MutToConstPointer` and the pointer arms of a `transmute`.
///
/// The matrix, in the order the arms are tried:
///
/// | from -> to | result |
/// |---|---|
/// | either side is a `str` | [`cast_str`], the UTF-8 view; see below |
/// | the same pointee | identity, which covers `*const T` to `*mut T` |
/// | zero sized pointee on either side | identity |
/// | `[T]` to a sized pointee | the element's answer, with `len` riding along inert |
/// | `dyn Trait` to a sized pointee | `p.ptr`, the data half of the fat pointer |
/// | same size, same indirectness | identity |
/// | element to array of elements | a [`window_slot`] |
/// | array of elements to anything | `__rt.unwindow`, then the element's answer |
/// | aggregate to its single non zero sized field | reproject: `{ buf: p.buf[p.off], off: "<field>" }` |
/// | size N to size 1 | a [`scaled_slot`] over the same buffer |
/// | size 1 to size N | `__rt.unscale`: a fresh heap block is retyped to that element size, a scaled slot is re-keyed, anything else throws |
/// | anything else | a zombie naming both types, recorded at the cast |
///
/// A `str` on either side is decided before any of that, by [`cast_str`]: it is a JavaScript string
/// rather than a buffer, so its byte view is encoded rather than reinterpreted and the sizes the
/// rest of the matrix reasons about say nothing about it.
///
/// Three of the rows want their reasoning spelled out.
///
/// **A transparent union is peeled off both sides first**, so `*const MaybeUninit<u8>` to
/// `*const u8` is the identity rather than a reprojection: the union *is* its field (`value.rs`).
///
/// **The array rows come before the size rows, and `__rt.unwindow` is a run time question.** A
/// pointer to `[E; N]` is either a plain slot whose one element is the whole array or the window
/// slot an earlier `as *const [E; N]` cast produced over `N` consecutive elements of somebody
/// else's buffer, and nothing in the type says which. The helper looks: a record with a `w` keeps
/// its buffer and offset, a record without one steps into the array it holds.
///
/// **`[u8; N]` to `uN` is not a pointer cast.** A slot names a place, and no place in a byte
/// buffer holds the integer those bytes spell, so there is no record to answer with. The
/// conversion is a *value* operation and lives where the value is in hand — the transmute
/// lowering in `intrinsics.rs`, which is what `u32::from_le_bytes` and `fmt`'s
/// `cast_array().read()` reach. A pointer cast between the two lands on the last row and says so.
///
/// The rows that take a record apart read `value` twice, and hoist it into a temporary first when
/// re-reading it would not be free (`reread`).
pub(crate) fn cast_pointer<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_pointee: Ty<'tcx>,
    to_pointee: Ty<'tcx>,
    value: Expr,
) -> Expr {
    if let Some(converted) = cast_str(fx, from_pointee, to_pointee, value.clone()) {
        return converted;
    }
    cast_pointer_at(fx, from_pointee, to_pointee, value, 0)
}

/// The depth at which a cast stops peeling and reports. A window, a slice and a wrapper each strip
/// one layer off a finite type, so the bound is belt and braces against a shape this has not been
/// taught about.
const CAST_DEPTH: u32 = 16;

/// [`cast_pointer`] once the `str` rows are behind it, with the recursion depth the array and
/// wrapper rows spend.
fn cast_pointer_at<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_pointee: Ty<'tcx>,
    to_pointee: Ty<'tcx>,
    value: Expr,
    depth: u32,
) -> Expr {
    let tcx = fx.tcx;
    let unsupported = || {
        fx.zombie(format!(
            "rustc_codegen_mires cannot cast a pointer to `{from_pointee}` into a pointer to \
             `{to_pointee}`"
        ))
    };
    if depth >= CAST_DEPTH {
        return unsupported();
    }

    // Both sides are asked in the shape their JavaScript value actually has: a pattern type is its
    // base, and a transparent union is its field.
    let from = peel_pattern(peel_transparent(tcx, from_pointee));
    let to = peel_pattern(peel_transparent(tcx, to_pointee));

    // The same place named differently, which is `*const T` to `*mut T` and every cast whose two
    // pointees survive the peeling as one type.
    if from == to {
        return value;
    }
    // A pointer to a zero sized pointee names no place of its own, so what it carries is decided
    // by what it is *for*: it exists to be cast back, and the one thing every user of one does
    // with it is hand it to something expecting a reference. `*const T` to `*const ()` is
    // therefore the **reference form** of `T` — the object for an aggregate, the slot for a
    // primitive — and the way back re-derives the raw pointer from it.
    //
    // `core::fmt` is what makes this the rule rather than a nicety. `rt::Argument::new` erases
    // `&T` to a `NonNull<()>` and its `fn(&T, &mut Formatter)` to a `fn(NonNull<()>, ...)` with a
    // `transmute`, then *calls* the erased function with the erased value. The two are one bit
    // pattern on a real machine; here they are one value only if the erased pointer to nothing is
    // exactly what a `&T` is. Boxing an aggregate on the way in — which is right for a raw pointer
    // to it, because a raw pointer can be offset — would hand `<T as Display>::fmt` a slot where
    // it expects its `self`, and every field read would answer `undefined`.
    if is_zst(tcx, to) {
        let reference = raw_to_ref(fx, from, value);
        // What the erasure loses is the element size, and one caller needs it back:
        // `<*const T>::addr` is `transmute(self.cast::<()>())`, so by the time the address is
        // taken the pointee is `()` and the offset is still counted in `T`s. `__rt.erase` records
        // the size on the record, where it is inert to every other operation.
        match size_of(tcx, from) {
            Some(size) if size > 1 && !is_indirect(tcx, from) => {
                jsast::rt_call("erase", vec![reference, jsast::num(size as f64)])
            }
            _ => reference,
        }
    } else if is_zst(tcx, from) {
        ref_to_raw(fx, to, value)
    } else {
        cast_pointer_sized(fx, from_pointee, to_pointee, from, to, value, depth)
    }
}

/// [`cast_pointer_at`] once the rows that do not depend on the two sizes are behind it.
#[allow(clippy::too_many_arguments)]
fn cast_pointer_sized<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_pointee: Ty<'tcx>,
    to_pointee: Ty<'tcx>,
    from: Ty<'tcx>,
    to: Ty<'tcx>,
    value: Expr,
    depth: u32,
) -> Expr {
    let tcx = fx.tcx;
    let unsupported = || {
        fx.zombie(format!(
            "rustc_codegen_mires cannot cast a pointer to `{from_pointee}` into a pointer to \
             `{to_pointee}`"
        ))
    };
    // A **struct with an unsized tail** on the target side: the block is reshaped into the header
    // plus its tail, which is what `Rc<[T]>` and `Arc<[T]>` allocate. This comes before the
    // unsized-source rows below, because the length of the tail is exactly what the fat source
    // record carries and unwrapping it to its element would throw that away --
    // `allocate_for_slice` casts the fresh block to `T`, then to `[T]`, then to `*mut RcInner<[T]>`
    // and the `len` on the middle step is the whole of the metadata.
    if crate::value::unsized_tail(tcx, to).is_some() {
        return reshape_to_tail(fx, from, to, to_pointee, value);
    }
    // The way back, which is how such a block is freed: the fat pointer's `ptr` half is the slot
    // naming the block's start, and that is what `dealloc` reads. The header sits at that slot and
    // the tail after it, so there is no byte view of the whole to hand over and none is asked for.
    if crate::value::unsized_tail(tcx, from).is_some() {
        return jsast::member(value, PTR);
    }

    // The unsized sources. A fat record is a slot with an extra key, so the rows below read `buf`
    // and `off` off one without knowing it is fat, and `len` rides along inert.
    match (from.kind(), to.kind()) {
        (ty::Slice(from_element), ty::Slice(to_element)) => {
            // `[T]` to `[U]`: the length is a count of elements, so it means the same thing only
            // while the elements are the same size.
            return if size_of(tcx, *from_element) == size_of(tcx, *to_element) {
                value
            } else {
                unsupported()
            };
        }
        (ty::Slice(element), _) => return cast_pointer_at(fx, *element, to, value, depth + 1),
        // The data half of a trait object, which is the *reference* form of the concrete value —
        // an object for an aggregate, a slot for anything else. The cast asks for the raw pointer
        // that reference denotes, and the target type cannot say which form it has: the concrete
        // type is gone, and `to` is whatever the caller is casting to. `*mut dyn Trait as *mut u8`
        // is that case, and it is how a `Box<dyn Trait>` is freed. So the record is asked, the way
        // `__rt.unwindow` is asked.
        (ty::Dynamic(..), _) => return jsast::rt_call("unref", vec![jsast::member(value, PTR)]),
        // Nothing thin carries the metadata a fat pointer needs.
        (_, ty::Slice(_) | ty::Dynamic(..)) => return unsupported(),
        _ => {}
    }

    let (Some(from_size), Some(to_size)) = (size_of(tcx, from), size_of(tcx, to)) else {
        return unsupported();
    };

    // `p as *const [E; N]`: a window over the `N` elements at `p`, which is how `cast_array` and
    // the array reads built on it stay inside the buffer those elements live in.
    //
    // The array rows come first, ahead of the size row below, and a one element array is why: an
    // `[E; 1]` is the size of an `E` and both are indirect, so the size row would call the cast the
    // identity — and a record naming the place an *array* lives is not a record naming the place
    // its first element lives. `Arguments::new` casts `&[rt::Argument; 1]` to a
    // `NonNull<rt::Argument>` for every single-argument `write!` in a program, so that identity is
    // the difference between `args.buf` and `undefined`.
    if let ty::Array(element, count) = to.kind() {
        let element = peel_pattern(peel_transparent(tcx, *element));
        let width = count.try_to_target_usize(tcx).unwrap_or(0);
        // The window is over elements of `to`, so a byte view has to be keyed to the element
        // first. `Box::new([1, 2, 3])` is that cast: `alloc` hands back a byte granular block and
        // the very next thing done to it is a cast to `*mut [i32; 3]`. Keying it to the *array*
        // instead would leave the block holding one element that is the whole array, and the
        // slice `Box<[i32]>` unsizes to would name that inner array rather than the allocation.
        let element_size = size_of(tcx, element);
        let keyed = if element == from {
            Some(value.clone())
        } else if from_size == 1 && !is_indirect(tcx, from) && element_size.is_some() {
            Some(jsast::rt_call(
                "unscale",
                vec![
                    value.clone(),
                    jsast::num(element_size.unwrap_or(1) as f64),
                    crate::alloc_support::zero_factory(fx.cgu, element),
                ],
            ))
        } else {
            None
        };
        if let Some(keyed) = keyed {
            let (setup, pointer) = reread(fx, keyed, &[BUF, OFF]);
            let window = window_slot(slot_buf(pointer.clone()), slot_off(pointer), width);
            return sequenced(setup, window);
        }
    }
    // The other direction: a pointer to an array becomes a pointer to its first element, and only
    // the record knows whether that means stepping into the array it holds or keeping the window
    // it already is.
    if let ty::Array(element, _) = from.kind() {
        let element = *element;
        let unwindowed = jsast::rt_call("unwindow", vec![value]);
        return cast_pointer_at(fx, element, to, unwindowed, depth + 1);
    }

    // One byte to a different one byte. The record needs no re-keying, so this would be the
    // identity row below -- but a heap block is born byte granular, so a one byte element is the
    // one case where the *first* read of a block needs no re-keying either. A block from
    // `alloc_zeroed` holds the number zero in every byte, and that is only the right value if the
    // element's zero is the byte zero: `vec![false; n]` owes `false`. `__rt.retype` fills such a
    // block and is the identity for every other pointer.
    if from_size == 1 && to_size == 1 && !is_indirect(tcx, from) && !is_indirect(tcx, to) {
        if let Some(zero) = crate::alloc_support::non_byte_zero_factory(fx.cgu, to_pointee) {
            return jsast::rt_call("retype", vec![value, jsast::num(1), zero]);
        }
    }
    // The same number of bytes held in the same shape of JavaScript value: the record already
    // names a place of the target type.
    if from_size == to_size && is_indirect(tcx, from) == is_indirect(tcx, to) {
        return value;
    }

    // A wrapper to what it wraps: `*const NonNull<T>` to `*const *const T`. The slot moves from
    // the wrapper to the field inside it, which is a place of the target type — as opposed to the
    // other direction, from a scalar to a wrapper around it, which has no place to name and stays
    // on the unsupported list.
    if let Some(reprojected) = reproject_to_field(fx, from, to, &value, depth) {
        return reprojected;
    }

    // `p as *const u8`: a byte offset view of a buffer of `from_size` byte elements. Not
    // dereferenceable — it exists to be offset and cast back.
    //
    // `__rt.scale` rather than the record literal, because a pointer that is an *address* has to
    // come out an address: `ptr::null::<i32>()` is the number `0`, and `is_null` is written as
    // `self.cast::<u8>().addr() == 0`, so a record built out of `0..buf` would turn the one
    // pointer whose address is knowable into a record naming nothing.
    if to_size == 1 && !is_indirect(tcx, to) {
        return jsast::rt_call("scale", vec![value, jsast::num(from_size as f64)]);
    }
    // The way back. `__rt.unscale` answers for a scaled slot of exactly this element size whose
    // byte offset lands on an element, and for a **fresh heap block**, which it retypes to this
    // element size (`CONTRACT.md`, "Allocation"). Anything else throws — which is what makes an
    // aligned `byte_add` honest, and byte punning of a buffer that really is bytes a run time error
    // at the mistake rather than a wrong value.
    //
    // The third argument is how a *zeroed* block is filled once it is retyped: only the compiler
    // knows what a zero `to_pointee` looks like, so the cast carries a factory for one.
    if from_size == 1 && !is_indirect(tcx, from) {
        return jsast::rt_call(
            "unscale",
            vec![
                value,
                jsast::num(to_size as f64),
                crate::alloc_support::zero_factory(fx.cgu, to_pointee),
            ],
        );
    }

    unsupported()
}

/// The row of [`cast_pointer`] that reshapes a heap block into a **header plus an unsized tail**.
///
/// `Rc<[T]>` and `Arc<[T]>` are the whole of it: `allocate_for_slice` allocates a byte granular
/// block, casts it to the element type, rebuilds it as a `*mut [T]` of the tail's length and then
/// casts *that* to a `*mut RcInner<[T]>`. So the length arrives on the source record and nowhere
/// else -- a thin source has no metadata at all, and there is nothing to reshape by.
///
/// `__rt.retype_rc` does the reshape and returns the `{ ptr, meta }` a pointer to such a struct is;
/// `runtime/shim.js` documents the block layout and the two arrival states it accepts.
fn reshape_to_tail<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from: Ty<'tcx>,
    to: Ty<'tcx>,
    to_pointee: Ty<'tcx>,
    value: Expr,
) -> Expr {
    let tail = crate::value::unsized_tail(fx.tcx, to).expect("the caller matched on the tail");
    let ty::Slice(element) = tail.kind() else {
        return fx.zombie(format!(
            "rustc_codegen_mires cannot build a pointer to `{to_pointee}`: only a slice tail is \
             supported, and a `{tail}` tail is not"
        ));
    };
    let Some(element_size) = size_of(fx.tcx, *element) else {
        return fx.zombie(format!("the elements of `{tail}` have no size here"));
    };
    if !matches!(from.kind(), ty::Slice(_)) {
        return fx.zombie(format!(
            "rustc_codegen_mires cannot cast a pointer to `{from}` into a pointer to `{to_pointee}`: \
             the length of the tail is not carried by the source pointer"
        ));
    }
    let (setup, pointer) = reread(fx, value, &[BUF, OFF, LEN]);
    let reshaped = jsast::rt_call(
        "retype_rc",
        vec![
            pointer.clone(),
            crate::alloc_support::header_factory(fx.cgu, to),
            jsast::num(element_size as f64),
            slot_len(pointer),
        ],
    );
    sequenced(setup, reshaped)
}

/// The reprojection row of [`cast_pointer`]: a pointer to a wrapper, cast to a pointer to what the
/// wrapper holds.
///
/// The chain of single non zero sized fields is walked to see whether `to` is *on* it before
/// anything is emitted, so a cast whose target the chain never reaches falls through to the size
/// rows rather than ending up pointing at a field of the wrong type. Each step is
/// `{ buf: p.buf[p.off], off: "<field>" }`: the slot moves inside the aggregate the outer slot
/// names.
fn reproject_to_field<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from: Ty<'tcx>,
    to: Ty<'tcx>,
    value: &Expr,
    depth: u32,
) -> Option<Expr> {
    let tcx = fx.tcx;
    let mut chain = Vec::new();
    let mut current = from;
    for _ in depth..CAST_DEPTH {
        let (field_ty, key) = crate::intrinsics::sole_field(fx, current)?;
        chain.push(key);
        current = peel_pattern(peel_transparent(tcx, field_ty));
        if current != to {
            continue;
        }
        let mut pointer = value.clone();
        for key in chain {
            pointer = match key {
                crate::value::FieldKey::Name(name) => {
                    slot(slot_element(pointer), jsast::string(name))
                }
                crate::value::FieldKey::Index(index) => {
                    slot(slot_element(pointer), jsast::num(index as f64))
                }
                // A transparent union is its field already: the slot does not move.
                crate::value::FieldKey::Transparent => pointer,
            };
        }
        return Some(pointer);
    }
    None
}

/// `value`, preceded by the assignment [`reread`] asked for, if it asked for one.
fn sequenced(setup: Option<Expr>, value: Expr) -> Expr {
    match setup {
        Some(setup) => jsast::seq(vec![setup, value]),
        None => value,
    }
}

/// `&value as *const T`: the raw pointer a reference to `pointee` becomes.
///
/// A reference that is already a slot is one. A reference to an aggregate is a bare object, whose
/// raw pointer is `__rt.box(x)`: that helper keeps one slot per object in a `WeakMap`, so two raw
/// pointers taken from one object are the same record and `ptr::eq` on them is true, and its
/// `buf[0]` *is* the object, so a write through the pointer reaches every alias.
///
/// A reference to something that has no slot form at all — a `&str`, an unsized pointee, a zero
/// sized one — is handed back unchanged.
///
/// The box is the *last* resort. When the reference is a read out of a container —
/// `points[i]`, `pair.first` — the slot naming that place is `{ buf: points, off: i }`, and
/// building it instead keeps the pointer inside the buffer its element lives in. That is what
/// makes `ptr::copy` of a run of aggregates work at all: a boxed element is a buffer of one, and
/// offsetting away from it names nothing. Two raw pointers taken this way from one element are
/// still equal, because `__rt.ptr_eq` compares the buffer and the key rather than the record.
pub(crate) fn ref_to_raw<'tcx>(fx: &FnCx<'_, 'tcx>, pointee: Ty<'tcx>, reference: Expr) -> Expr {
    if !is_slot_pointee(fx.tcx, pointee) {
        return reference;
    }
    if !is_indirect(fx.tcx, pointee) {
        return reference;
    }
    match reference {
        Expr::Index(base, key) => slot(*base, *key),
        Expr::Member(base, name) => slot(*base, jsast::string(name)),
        reference => jsast::rt_call("box", vec![reference]),
    }
}

/// `&*raw`: the reference a raw pointer to `pointee` becomes.
///
/// The inverse of [`ref_to_raw`]. A reference to an aggregate is the object the slot names,
/// [`slot_element`]; a reference to anything else is the slot itself.
pub(crate) fn raw_to_ref<'tcx>(fx: &FnCx<'_, 'tcx>, pointee: Ty<'tcx>, raw: Expr) -> Expr {
    if !is_slot_pointee(fx.tcx, pointee) {
        return raw;
    }
    if is_indirect(fx.tcx, pointee) { slot_element(raw) } else { raw }
}

/// A fat pointer built from its two halves: `aggregate_raw_ptr(data, meta)`, and the tail of an
/// unsizing coercion.
///
/// `pointee` is the unsized pointee: `[T]` or `dyn Trait`.
///
/// A slice pointer keeps the buffer and offset of the data pointer and takes the length from the
/// metadata, `{ buf: d.buf, off: d.off, len: m }`, which is what makes `from_raw_parts` on a slot
/// work.
///
/// A **chunk granularity** rebuild — the element type of the result is itself an array, which is
/// what `as_chunks` asks for — is a question the type cannot answer, so it is asked at run time by
/// `__rt.chunk_slice`. A pointer into a buffer whose elements really are those arrays rebuilds
/// into a slice over them; a pointer into a flat buffer (the window slot an `as *const [E; N]`
/// cast produced, or a buffer of bare elements) has no such elements in it and throws. That is the
/// same shape as [`__rt.unwindow`](cast_pointer), and it is deliberately *not* a zombie: the
/// unreachable-at-run-time half of `str::count::do_count_chars` is on the static call graph of
/// every `Display for str`, so a zombie there would reject `write!` itself. The cast that feeds it
/// (`*const u8` to `*const usize`) throws first, so nothing silently reads bytes as words.
///
/// A `str` is a JavaScript string rather than a buffer, so rebuilding one from a byte pointer is
/// `__rt.bytes_str` over the data slot and the length — a decode rather than a re-wrapping, which
/// is the price of the hybrid.
///
/// A **sized** pointee is not a rebuild at all: its metadata is `()`, and the result is the data
/// pointer, keyed back out of whatever erased form it arrived in.
pub(crate) fn build_fat<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    pointee: Ty<'tcx>,
    data: Expr,
    meta: Expr,
) -> Expr {
    match pointee.kind() {
        ty::Dynamic(..) => fat_dyn(data, meta),
        ty::Slice(element) if matches!(element.kind(), ty::Array(..)) => {
            let width = match element.kind() {
                ty::Array(_, count) => count.try_to_target_usize(fx.tcx).unwrap_or(0),
                _ => 0,
            };
            jsast::rt_call("chunk_slice", vec![data, meta, jsast::num(width as f64)])
        }
        ty::Slice(_) => fat_slice(slot_buf(data.clone()), slot_off(data), meta),
        // `str::from_raw_parts(p, n)`: the `n` bytes at `p`, decoded back into the JavaScript
        // string a `&str` is.
        ty::Str => {
            let (setup, data) = reread(fx, data, &[BUF, OFF]);
            let decoded =
                jsast::rt_call("bytes_str", vec![slot_buf(data.clone()), slot_off(data), meta]);
            sequenced(setup, decoded)
        }
        // A **sized** pointee has `()` for its metadata, so nothing is being rebuilt: a thin
        // pointer is its data pointer, and the metadata operand is `undefined`.
        //
        // The work is in the erasure the data pointer arrived through. `from_raw_parts` takes a
        // `*const impl Thin`, and `with_metadata_of` -- which is how `byte_offset`, `byte_add` and
        // `byte_sub` are all written -- passes `self as *const ()`. For a `*const i32` that value
        // is the scaled byte slot `cast::<u8>()` produced, and keying it back into `i32`s is the
        // whole of the round trip; for a `*const u8` it is already the slot. Nothing in the type
        // says which, so `__rt.thin` asks the record, the same way `__rt.unwindow` does.
        _ if pointee.is_sized(fx.tcx, typing_env()) => {
            let size = size_of(fx.tcx, pointee).unwrap_or(0);
            jsast::rt_call("thin", vec![data, jsast::num(size as f64)])
        }
        // A struct with an unsized tail, rebuilt from the fresh `*mut u8` its block was allocated
        // as. `with_metadata_of` is the spelling -- `allocate_for_ptr_in` hands the raw block and
        // the metadata of the value being copied into it -- and the reshape is the same one the
        // pointer cast performs, from the other arrival state (`runtime/shim.js`).
        _ => {
            let element = crate::value::unsized_tail(fx.tcx, pointee)
                .and_then(|tail| match tail.kind() {
                    ty::Slice(element) => size_of(fx.tcx, *element),
                    _ => None,
                });
            match element {
                Some(element_size) => jsast::rt_call(
                    "retype_rc",
                    vec![
                        data,
                        crate::alloc_support::header_factory(fx.cgu, pointee),
                        jsast::num(element_size as f64),
                        meta,
                    ],
                ),
                None => fx.zombie(format!(
                    "a pointer to `{pointee}` cannot be rebuilt from a thin data pointer by \
                     rustc_codegen_mires"
                )),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The `str` hybrid
// ---------------------------------------------------------------------------------------------

/// `s.as_bytes()`: the `{ buf, off, len }` byte view of a `&str`.
///
/// A `&str` is a JavaScript string, which has no bytes to point at, so the byte view is *made* —
/// `__rt.str_bytes` encodes the string to UTF-8 and hands back a fat slice over a plain array. That
/// is a conversion, not a reinterpretation, and it is why this is the one pointer cast in the model
/// that costs anything at run time.
///
/// The helper memoizes the last few strings, so two `as_bytes()` calls on one string give back the
/// same record and comparing their pointers answers `true`. Identity across independent calls is a
/// convenience and not a guarantee: nothing here may depend on it, and `CONTRACT.md` says so.
pub(crate) fn str_to_bytes(string: Expr) -> Expr {
    jsast::rt_call("str_bytes", vec![string])
}

/// `s.as_ptr()`: the thin `*const u8` at the start of a `&str`'s bytes.
///
/// The same byte view as [`str_to_bytes`], reduced to a slot: a thin pointer carries no length, and
/// leaving one on the record would put a `len` on a pointer that is about to be offset past it.
pub(crate) fn str_to_byte_ptr(string: Expr) -> Expr {
    slot(slot_buf(str_to_bytes(string)), jsast::num(0))
}

/// `str::from_utf8_unchecked(v)`: the JavaScript string `v`'s bytes spell.
///
/// `bytes` is a fat slice of one byte elements, so the three halves of it are what the decoder
/// reads. The record is read three times, which is free for the place expression an operand always
/// is and goes through a temporary for anything else.
///
/// `form` picks the decoder, because a one byte element is not always a number; see
/// [`ByteElement`].
pub(crate) fn bytes_to_str<'tcx>(fx: &FnCx<'_, 'tcx>, form: ByteElement, bytes: Expr) -> Expr {
    let (setup, bytes) = reread(fx, bytes, &[BUF, OFF, LEN]);
    let decoded = jsast::rt_call(
        form.decoder(),
        vec![slot_buf(bytes.clone()), slot_off(bytes.clone()), slot_len(bytes)],
    );
    sequenced(setup, decoded)
}

/// `__rt.str_len(s)`: the UTF-8 byte length of a `&str`, which is its pointer metadata.
pub(crate) fn str_len(string: Expr) -> Expr {
    jsast::rt_call("str_len", vec![string])
}

/// The `str` arms of the pointer cast matrix, over the two **pointee** types.
///
/// `None` when neither side is a `str`, which is the answer that lets [`cast_pointer`] go on to the
/// rest of its matrix; everything else is decided here, including the zombie for a cast this model
/// gives no meaning to.
///
/// | from -> to | result |
/// |---|---|
/// | `str` -> `str` | identity, which covers `*const str` to `*mut str` |
/// | `str` -> `[u8]` | [`str_to_bytes`] |
/// | `str` -> `u8` | [`str_to_byte_ptr`] |
/// | `[E]` -> `str`, `E` one byte | [`bytes_to_str`] |
/// | anything else with a `str` on one side | a zombie naming both types |
///
/// The element of the slice being decoded is any [one byte element](ByteElement) rather than a
/// `u8`, and `core` needs it to be: `char`'s `Debug` escapes through a buffer of `AsciiChar`, whose
/// `as_str` is this cast. Every panic that names a `char` is downstream of that, which is `&s[a..b]`
/// on a `str`, `contains`, `find` with a `&str` pattern, and the float formatter.
pub(crate) fn cast_str<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_pointee: Ty<'tcx>,
    to_pointee: Ty<'tcx>,
    value: Expr,
) -> Option<Expr> {
    let from = peel_pattern(from_pointee);
    let to = peel_pattern(to_pointee);
    let (from_str, to_str) = (matches!(from.kind(), ty::Str), matches!(to.kind(), ty::Str));
    if !from_str && !to_str {
        return None;
    }
    let unsupported = || {
        fx.zombie(format!(
            "rustc_codegen_mires cannot cast a pointer to `{from_pointee}` into a pointer to \
             `{to_pointee}`: a `str` is a JavaScript string, whose only byte view is the UTF-8 \
             one `as_bytes` produces"
        ))
    };
    Some(match (from_str, to_str) {
        (true, true) => value,
        (true, false) => match to.kind() {
            ty::Slice(element) if is_u8(*element) => str_to_bytes(value),
            _ if is_u8(to) => str_to_byte_ptr(value),
            _ => unsupported(),
        },
        (false, true) => match from.kind() {
            ty::Slice(element) => match ByteElement::of(fx.tcx, *element) {
                Some(form) => bytes_to_str(fx, form, value),
                None => unsupported(),
            },
            _ => unsupported(),
        },
        (false, false) => unreachable!("neither side is a `str`"),
    })
}

/// [`cast_str`] over the two **pointer** types, for the callers that have those in hand.
///
/// `rvalue.rs`'s cast arm and `intrinsics.rs`'s `transmute` both reach the `str` conversions
/// through here, so that neither has to take a pointer type apart itself. `None` when either side
/// is not a pointer at all, or when neither pointee is a `str`.
pub(crate) fn cast_str_ptr<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
    value: Expr,
) -> Option<Expr> {
    let from_pointee = peel_pattern(from_ty).builtin_deref(true)?;
    let to_pointee = peel_pattern(to_ty).builtin_deref(true)?;
    cast_str(fx, from_pointee, to_pointee, value)
}

/// Whether `ty` is `u8`, the element of every byte view.
fn is_u8(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), ty::Uint(ty::UintTy::U8))
}

/// A one byte element, for the slices [`bytes_to_str`] decodes.
///
/// A slice of one byte elements is the one buffer whose element count and byte count are the same
/// number, which is what lets a `len` in elements be read as a `len` in bytes. Two types qualify,
/// and both are a JavaScript number:
///
/// * `u8` or `i8`. A signed element is accepted because the byte is the same byte either way; the
///   decoder reads it through a `Uint8Array`, which is where the sign goes.
/// * a **fieldless enum with a one byte integer `repr`**, which *is* its discriminant
///   (`value::EnumRepr`). `AsciiChar` is exactly that, and `[AsciiChar]::as_str` is the cast that
///   made this arm necessary: it is on the static call graph of every `char` formatted with
///   `Debug`, and so of every `str` slicing panic.
///
/// A fieldless enum *without* an integer `repr` is a variant name rather than a byte, and is not a
/// byte element at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ByteElement;

impl ByteElement {
    /// Whether `ty` is a one byte element, in which case a buffer of them is a buffer of bytes.
    ///
    /// The type is asked in the shape its JavaScript value has, so a transparent wrapper around a
    /// `u8` and a pattern type over one both answer the way the `u8` inside them does.
    pub(crate) fn of<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<ByteElement> {
        if matches!(
            peel_pattern(peel_transparent(tcx, ty)).kind(),
            ty::Uint(ty::UintTy::U8) | ty::Int(ty::IntTy::I8)
        ) {
            return Some(ByteElement);
        }
        matches!(crate::value::direct_tag(tcx, ty), Some((8, _))).then_some(ByteElement)
    }

    /// The `__rt` decoder that reads a buffer of these elements as a string.
    fn decoder(self) -> &'static str {
        "bytes_str"
    }
}

/// An expression a caller reads `reads` off, once each, and the assignment that makes that safe.
///
/// A place expression — a name, or a chain of fixed field and element reads over one — and a record
/// literal every one of those properties folds out of both re-read for nothing, so they come back
/// unchanged. Anything else is evaluated once into a hoisted temporary, and the caller writes the
/// returned assignment before its own expression in a sequence: a place has no statement channel to
/// put it in, and `FnCx::temp` declares the name in the prelude either way.
fn reread<'tcx>(fx: &FnCx<'_, 'tcx>, value: Expr, reads: &[&str]) -> (Option<Expr>, Expr) {
    if duplicable(&value, reads) {
        return (None, value);
    }
    let (_, name) = fx.temp(value.clone());
    (Some(jsast::assign(name.clone(), value)), name)
}

/// Whether reading `reads` off `value` several times costs no more than reading it once and does
/// the same thing.
fn duplicable(value: &Expr, reads: &[&str]) -> bool {
    match value {
        Expr::Ident(_) => true,
        Expr::Member(object, _) => duplicable(object, reads),
        Expr::Index(object, index) => {
            duplicable(object, reads)
                && matches!(&**index, Expr::Num(_) | Expr::Str(_) | Expr::Ident(_))
        }
        Expr::Object(entries) => {
            reads.iter().all(|name| jsast::folds_property(entries, name))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

/// Whether a *raw* pointer to `pointee` is a slot record.
///
/// True for every sized pointee except a zero sized one (a pointer to which is an address) and
/// `str` (which is a JavaScript string); false for an unsized pointee, whose pointer is fat.
///
/// A *reference* is narrower: it is a slot exactly when `!is_indirect(pointee)`, because a
/// reference to an aggregate keeps the aggregate's own identity instead. [`is_slot`] asks the
/// question of a whole pointer type, which is the form the place walk has in hand.
pub(crate) fn is_slot_pointee<'tcx>(tcx: TyCtxt<'tcx>, pointee: Ty<'tcx>) -> bool {
    let pointee = peel_pattern(pointee);
    if matches!(pointee.kind(), ty::Str) {
        return false;
    }
    match tcx.layout_of(typing_env().as_query_input(pointee)) {
        Ok(layout) => layout.is_sized() && !layout.is_zst(),
        Err(_) => false,
    }
}

/// Whether `pointer_ty` is a **raw** pointer rather than a reference.
///
/// The two differ for an aggregate pointee only, and the difference is the whole of the ref/raw
/// split: a reference to an aggregate is the aggregate's own object, while a raw pointer to one is
/// a slot naming the place it lives. A wrapper that *is* the pointer inside it answers the way that
/// pointer does, which is what makes a `NonNull<T>` and a `Box<T>` raw.
pub(crate) fn is_raw<'tcx>(tcx: TyCtxt<'tcx>, pointer_ty: Ty<'tcx>) -> bool {
    matches!(
        peel_pattern(crate::value::peel_transparent(tcx, pointer_ty)).kind(),
        ty::RawPtr(..)
    )
}

/// Whether a value of pointer type `pointer_ty` is a slot record, so that `*p` is `p.buf[p.off]`.
///
/// The two halves of the ref/raw split, in one question: a raw pointer is a slot wherever
/// [`is_slot_pointee`] says so, and a reference is a slot only where the pointee has no JS object
/// of its own to be. Anything that is not a pointer is not a slot.
///
/// The type is asked in the shape its JavaScript value has, so a wrapper that *is* the pointer
/// inside it answers the way that pointer does: a `Box<T>` is the raw pointer it owns
/// (`value.rs`), and `*b` is therefore `b.buf[b.off]` like any other dereference of one.
pub(crate) fn is_slot<'tcx>(tcx: TyCtxt<'tcx>, pointer_ty: Ty<'tcx>) -> bool {
    match peel_pattern(crate::value::peel_transparent(tcx, pointer_ty)).kind() {
        ty::Ref(_, pointee, _) => !is_indirect(tcx, *pointee) && is_slot_pointee(tcx, *pointee),
        ty::RawPtr(pointee, _) => is_slot_pointee(tcx, *pointee),
        _ => false,
    }
}

/// The size of `ty` in bytes, or `None` when it has no layout or no statically known size.
///
/// This is the scale a byte offset is measured in and the multiplier [`addr`] applies, so the
/// operations that need it need it exactly.
pub(crate) fn size_of<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<u64> {
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    layout.is_sized().then(|| layout.size.bytes())
}
