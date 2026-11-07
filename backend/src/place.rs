//! Places and operands: the JavaScript that reads and writes a MIR `Place`.
//!
//! Value and reference representation is documented in `value.rs`.
//!
//! # Fat pointers
//!
//! Three pointee types are not a single JavaScript value, so a pointer to them carries metadata
//! beside the pointer (`unsize.rs` builds them, `vtable.rs` builds the vtable half):
//!
//! * `&[T]` / `&mut [T]` is `{ buf, off, len }` — a backing JS array, a start offset into it and a
//!   length. The offset is what makes `Subslice` a constant time projection: a subslice shares the
//!   same `buf` and only moves `off`/`len`, so a write through it is visible in the original.
//! * `&dyn Trait` is `{ ptr, meta }` — the concrete value and its vtable array.
//! * a reference to a **struct with an unsized tail** is `{ ptr, meta }` too -- the record and the
//!   length of the tail (`value.rs::unsized_tail`). This one is not the identity all the way
//!   through: a `Deref` names the record, and the projection that reads the tail field is where
//!   `meta` comes back to build the fat slice the tail's type is.
//!
//! `Deref` of the first two keeps the whole object: it is the place walk's *later* projections that
//! read `buf`/`off`/`len` or `ptr`. `&*p` where `*p` is unsized is therefore the identity, which is
//! exactly what MIR's `&raw const (fake) (*_1)` (`RawPtrKind::FakeForPtrMetadata`, the shape a
//! bounds check is built out of) needs it to be.
//!
//! # Slots
//!
//! Every other pointer is a `{ buf, off }` slot (`ptr.rs` owns the model), and a `Deref` of one is
//! `p.buf[p.off]` — an ordinary assignable JavaScript place, which is what lets the walk keep
//! going through a pointer without a special case for writing.
//!
//! Taking a reference is therefore the walk run *backwards*: the walk produces `A[B]` or `A.k`,
//! and `&place` takes it apart again into `{ buf: A, off: B }` or `{ buf: A, off: "k" }`. A slot
//! captures its key **by value** at the moment it is built, so `&mut xs[i]` keeps pointing at the
//! element `i` named then even if `i` moves afterwards; the accessor closures this replaced closed
//! over `i` instead, and needed the index hoisted into a temporary to be sound.
//!
//! A bare local has no container to be the `buf` of, so a local whose address is taken is
//! **boxed**: declared `let x = []`, used as `x[0]`, and pointed at with `{ buf: x, off: 0 }`. See
//! `uses.rs` for how that is decided and `base.rs` for where it is declared.

use rustc_abi::VariantIdx;
use rustc_middle::mir::{Operand, Place, PlaceElem};
use rustc_middle::ty::{self, Ty};

use crate::base::FnCx;
use crate::jsast::{self, BinOp as JsBinOp, Expr, Stmt};
use crate::ptr;
use crate::value::{FieldKey, clone_expr, field_expr, field_key, is_indirect, needs_clone};

/// A place, lowered to the JavaScript that reads and writes it.
pub(crate) enum JsPlace {
    /// A JS assignment target: a local, a property, an element, or the `p.buf[p.off]` a pointer
    /// names.
    Var(Expr),
    /// The pointee of a reference to an aggregate. Reading is the value itself; writing has to
    /// overwrite in place, because rebinding the JS variable would break the aliasing.
    Deref(Expr),
    /// The `N` elements a pointer to `[E; N]` names, reached through `__rt.read_array` and
    /// `__rt.write_array`. The expression is the *pointer*, not the place, because which elements
    /// it names is a question only the record can answer: a plain slot holds the whole array at
    /// `p.buf[p.off]`, and the window slot a `as *const [E; N]` cast produces spans `w`
    /// consecutive elements of somebody else's buffer (`ptr.rs`, "Forms").
    Array(Expr, u64),
}

/// Where the unsized tail of a struct lives, carried from the `Deref` that named the struct to the
/// projection that reads the tail.
///
/// The two forms are the same reference/raw split every other aggregate pointee has, and they are
/// the two halves of the representation:
///
/// * a **reference** to such a struct is `{ ptr: <the record>, meta }`, and the tail is an
///   ordinary field of that record holding the elements. `core`'s `array::IntoIter` unsizes a
///   `PolymorphicIter<[T; N]>` this way;
/// * a **raw pointer** to one is `{ ptr: <a slot>, meta }` over a heap block laid out as the
///   header record at element 0 and the tail after it, which is what `Rc<[T]>` allocates
///   (`runtime/shim.js`, `retype_rc`). The record is `p.ptr.buf[p.ptr.off]` and the tail is the
///   run of elements that follows it, so the tail is a slice over the block itself.
enum Tail {
    /// The tail is a field of the record, holding the elements.
    Field { meta: Expr },
    /// The tail is the run of elements after the header in a heap block.
    Block { block: Expr, meta: Expr },
}

/// The result of walking a place: where it is and what type it has.
struct Walk<'tcx> {
    place: JsPlace,
    ty: Ty<'tcx>,
    /// The pointer a `&`-of-this-place is, when the place is the pointee of a fat pointer. `&*p`
    /// on an unsized pointee is `p` itself, metadata included.
    identity: Option<Expr>,
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Walks a place's projections, tracking the JS expression that reaches it and its type.
    pub(crate) fn codegen_place(&self, place: Place<'tcx>) -> (JsPlace, Ty<'tcx>) {
        let walk = self.walk_place(place);
        (walk.place, walk.ty)
    }

    /// The place walk.
    fn walk_place(&self, place: Place<'tcx>) -> Walk<'tcx> {
        let mut current = JsPlace::Var(self.local_expr(place.local));
        let mut ty = self.monomorphize(self.mir.local_decls[place.local].ty);
        let mut variant: Option<VariantIdx> = None;
        let mut identity = None;
        // Where the tail of the struct a `Deref` of a `{ ptr, meta }` named actually lives. Only
        // the projection that reads that tail needs it, and the steps that may sit in between -- a
        // transparent field, a downcast, a cast that changes nothing -- leave the place alone and
        // carry it along; every other projection drops it.
        let mut tail: Option<Tail> = None;

        let projection = place.projection;
        for (step, elem) in projection.iter().enumerate() {
            // Whether the place is settled here: nothing left in the projection produces any
            // JavaScript of its own. A field of a transparent wrapper is the wrapper (`value.rs`)
            // and a downcast is a no-op, so a `Deref` followed only by those still names what the
            // `Deref` named -- which is what makes `(*b).value = v` on a `Box<[E; N]>` reach the
            // array store rather than an assignment to one place.
            let settles = projection[step + 1..].iter().all(|elem| {
                matches!(
                    elem,
                    PlaceElem::Field(..)
                        | PlaceElem::Downcast(..)
                        | PlaceElem::OpaqueCast(_)
                        | PlaceElem::UnwrapUnsafeBinder(_)
                )
            });
            // Only a `Deref` of an unsized pointee sets this; every other projection clears it.
            identity = None;
            let carried_tail = tail.take();

            match elem {
                PlaceElem::Deref => {
                    let Some(pointee) = ty.builtin_deref(true) else {
                        return Walk {
                            place: JsPlace::Var(self.zombie(format!("cannot deref `{ty}`"))),
                            ty,
                            identity: None,
                        };
                    };
                    let value = self.read(&current);
                    // A transparent wrapper *is* its field (`value.rs`), so the shape questions
                    // below are asked of what the pointee's JavaScript value actually is.
                    // `MaybeUninit<[E; N]>` is how `Box::new([1, 2, 3])` spells the place it
                    // writes into, and it has to reach the array row rather than the scalar one.
                    let shape = crate::value::peel_pattern(
                        crate::value::peel_transparent(self.tcx, pointee),
                    );
                    current = match shape.kind() {
                        // A trait object: the concrete value lives behind `.ptr`, and the whole
                        // fat pointer is what a reference to the pointee is.
                        ty::Dynamic(..) => {
                            identity = Some(value.clone());
                            JsPlace::Deref(jsast::member(value, ptr::PTR))
                        }
                        // A slice: the place *is* the fat pointer, because the projections that
                        // follow need all three of `buf`, `off` and `len`.
                        ty::Slice(_) => {
                            identity = Some(value.clone());
                            JsPlace::Deref(value)
                        }
                        // A struct with an unsized tail: the record lives behind `.ptr`, exactly
                        // as a trait object's concrete value does, and `.meta` measures the tail.
                        // Where the tail itself is depends on which of the two the `.ptr` half is,
                        // and that is the same reference/raw split every other aggregate pointee
                        // has -- see [`Tail`].
                        _ if crate::value::unsized_tail(self.tcx, shape).is_some() => {
                            identity = Some(value.clone());
                            let meta = jsast::member(value.clone(), ptr::META);
                            let record = jsast::member(value, ptr::PTR);
                            match ptr::is_raw(self.tcx, ty) {
                                true => {
                                    tail = Some(Tail::Block { block: record.clone(), meta });
                                    JsPlace::Deref(ptr::slot_element(record))
                                }
                                false => {
                                    tail = Some(Tail::Field { meta });
                                    JsPlace::Deref(record)
                                }
                            }
                        }
                        // A pointer to an array, dereferenced as a whole: the elements it names
                        // are `p.buf[p.off]` for a plain slot and `buf[off .. off + w]` for the
                        // window slot an `as *const [E; N]` cast produced, and the two are told
                        // apart at run time by `__rt.read_array`/`__rt.write_array`. A projection
                        // *out of* the array keeps the plain slot form below: a pointer that
                        // reaches an element was never a window.
                        ty::Array(_, count) if settles && ptr::is_slot(self.tcx, ty) => {
                            let width = count.try_to_target_usize(self.tcx).unwrap_or(0);
                            JsPlace::Array(value, width)
                        }
                        // A slot whose pointee is an aggregate: the place is the object at
                        // `p.buf[p.off]`, and it is a `Deref` rather than a `Var` because a write
                        // of the *whole* pointee has to overwrite that object in place. Assigning
                        // `p.buf[p.off]` would rebind the one container the pointer came from —
                        // for a raw pointer taken from a local, the `__rt.box` array that nothing
                        // else reads — and the local it aliases would keep its old value. The
                        // projections that follow are unaffected: reading a `Deref` is the object,
                        // exactly as reading a `Var` was.
                        _ if ptr::is_slot(self.tcx, ty) && is_indirect(self.tcx, pointee) => {
                            JsPlace::Deref(ptr::slot_element(value))
                        }
                        // A slot to a primitive: `p.buf[p.off]` is the place, and it is assignable,
                        // so the projections that follow and a write both work without a special
                        // case.
                        _ if ptr::is_slot(self.tcx, ty) => {
                            JsPlace::Var(ptr::slot_element(value))
                        }
                        // A reference to an aggregate, which is the aggregate's own object.
                        _ => JsPlace::Deref(value),
                    };
                    ty = pointee;
                    variant = None;
                }
                PlaceElem::Field(field, field_ty) => {
                    let key = field_key(self.tcx, ty, variant, field).unwrap_or_else(|| {
                        self.zombie(format!("cannot take a field of `{ty}`"));
                        FieldKey::Index(field.as_usize())
                    });
                    let field_ty = self.monomorphize(field_ty);
                    // The one field of a transparent wrapper *is* the wrapper, so the projection
                    // leaves the place exactly as it was. Reading it and re-wrapping the result
                    // would not: it would turn a place into a value, and an array place read that
                    // way is a copy of the elements rather than the place they live in.
                    // `MaybeUninit::write` is the case -- `(*p).value = v` on a `Box<[E; N]>` has
                    // to reach the same store `*p = v` does.
                    if matches!(key, FieldKey::Transparent) {
                        tail = carried_tail;
                    } else if !field_ty.is_sized(self.tcx, crate::value::typing_env()) {
                        // The unsized tail of the record a `Deref` of a `{ ptr, meta }` named.
                        // Rebuilding the fat pointer the tail's type is needs the metadata and
                        // where the elements live, which is what the walk carried here.
                        let base = self.read(&current);
                        current = self.tail_place(base, key, field_ty, carried_tail);
                    } else {
                        let base = self.read(&current);
                        current = JsPlace::Var(field_expr(base, key));
                    }
                    ty = field_ty;
                    variant = None;
                }
                PlaceElem::Index(index) => {
                    let base = self.read(&current);
                    let index = self.local_expr(index);
                    current = JsPlace::Var(self.element_expr(ty, base, index));
                    ty = self.element_ty(ty);
                    variant = None;
                }
                PlaceElem::ConstantIndex { offset, min_length: _, from_end } => {
                    let base = self.read(&current);
                    let index = self.constant_index(ty, base.clone(), offset, from_end);
                    // Constant either way, so nothing to hoist: `len` cannot change under a
                    // reference that borrows the slice it was read from.
                    current = JsPlace::Var(self.element_expr(ty, base, index));
                    ty = self.element_ty(ty);
                    variant = None;
                }
                PlaceElem::Subslice { from, to, from_end } => {
                    let base = self.read(&current);
                    let (value, subslice_ty) = self.subslice(ty, base, from, to, from_end);
                    current = JsPlace::Deref(value);
                    ty = subslice_ty;
                    variant = None;
                }
                // Neither produces JavaScript of its own, so the place -- and the metadata of the
                // fat pointer it came through -- is exactly what it was.
                PlaceElem::Downcast(_, index) => {
                    variant = Some(index);
                    tail = carried_tail;
                }
                PlaceElem::OpaqueCast(cast_ty) | PlaceElem::UnwrapUnsafeBinder(cast_ty) => {
                    ty = self.monomorphize(cast_ty);
                    tail = carried_tail;
                }
            }
        }

        Walk { place: current, ty, identity }
    }

    /// The place a struct's **unsized tail** is, given the record it was projected out of, the key
    /// of the tail field, and where the walk says the tail lives.
    ///
    /// Either way the answer is the fat pointer the tail's type is, built out of the elements and
    /// the metadata the two halves carry between them. A slice tail is the only shape supported:
    /// the reference half can produce nothing else (`[T; N] -> [T]` is the only coercion that
    /// makes a record with a tail out of a sized one) and on the heap half a `str` or `dyn` tail
    /// is a block shape this backend does not build.
    fn tail_place(
        &self,
        record: Expr,
        key: FieldKey,
        tail_ty: Ty<'tcx>,
        tail: Option<Tail>,
    ) -> JsPlace {
        let Some(tail) = tail else {
            return JsPlace::Var(self.zombie(format!(
                "the `{tail_ty}` tail of a struct was projected out of a place that is not behind \
                 a `{{ ptr, meta }}` pointer, so its length is not known here"
            )));
        };
        if !matches!(tail_ty.kind(), ty::Slice(_)) {
            return JsPlace::Var(self.zombie(format!(
                "a struct with a `{tail_ty}` tail is not supported by rustc_codegen_mires; only a \
                 slice tail is, because only a run of elements can be laid out after the header \
                 (`examples/core-tests/guard_rc_str.rs`)"
            )));
        }
        match tail {
            // The field holds the elements, so the slice over them starts at zero.
            Tail::Field { meta } => {
                JsPlace::Deref(ptr::fat_slice(field_expr(record, key), jsast::num(0), meta))
            }
            // The block holds the header at the slot's own offset and the tail after it, so the
            // slice starts one element on. Every `{ buf, off }` the model builds then works over
            // the tail unchanged: it is an ordinary slice into the block.
            Tail::Block { block, meta } => JsPlace::Deref(ptr::fat_slice(
                ptr::slot_buf(block.clone()),
                jsast::binary(JsBinOp::Add, ptr::slot_off(block), jsast::num(1)),
                meta,
            )),
        }
    }

    /// How a local is read: `x`, or `x[0]` when it is boxed (see `uses.rs`).
    pub(crate) fn local_expr(&self, local: rustc_middle::mir::Local) -> Expr {
        let name = jsast::id(self.local_name(local));
        if self.is_boxed(local) { jsast::index(name, jsast::num(0)) } else { name }
    }

    /// One element of an array or a slice: `base[i]` or `base.buf[base.off + i]`.
    fn element_expr(&self, base_ty: Ty<'tcx>, base: Expr, index: Expr) -> Expr {
        match base_ty.kind() {
            ty::Slice(_) => jsast::index(
                ptr::slot_buf(base.clone()),
                offset(ptr::slot_off(base), index),
            ),
            _ => jsast::index(base, index),
        }
    }

    /// The index a `ConstantIndex` projection reads, counted from the far end when asked.
    fn constant_index(&self, base_ty: Ty<'tcx>, base: Expr, offset: u64, from_end: bool) -> Expr {
        if !from_end {
            return jsast::num(offset as f64);
        }
        // `[-k of n]` is the k-th element from the end. A slice's length is its own field; an
        // array's is the JS array's.
        let length = match base_ty.kind() {
            ty::Slice(_) => ptr::slot_len(base),
            _ => jsast::member(base, "length"),
        };
        jsast::binary(JsBinOp::Sub, length, jsast::num(offset as f64))
    }

    /// A `Subslice` projection: `xs[from .. len - to]`, as a fat pointer sharing `xs`'s buffer.
    fn subslice(
        &self,
        base_ty: Ty<'tcx>,
        base: Expr,
        from: u64,
        to: u64,
        from_end: bool,
    ) -> (Expr, Ty<'tcx>) {
        let tcx = self.tcx;
        match base_ty.kind() {
            ty::Slice(_) => {
                // `[from:-to]` keeps the same buffer and moves the window; the length shrinks by
                // `from` at the start and, when counted from the end, by `to` at the tail.
                let len = ptr::slot_len(base.clone());
                let len = if from_end {
                    let trimmed = from + to;
                    if trimmed == 0 {
                        len
                    } else {
                        jsast::binary(JsBinOp::Sub, len, jsast::num(trimmed as f64))
                    }
                } else {
                    jsast::num(to.saturating_sub(from) as f64)
                };
                let value = ptr::fat_slice(
                    ptr::slot_buf(base.clone()),
                    offset(ptr::slot_off(base), jsast::num(from as f64)),
                    len,
                );
                (value, base_ty)
            }
            ty::Array(_, size) => {
                // Subslicing a fixed size array yields another fixed size array, so the value is
                // a plain JS array rather than a fat pointer.
                let size = size.try_to_target_usize(tcx).unwrap_or(0);
                let end = if from_end { size.saturating_sub(to) } else { to };
                let element = self.element_ty(base_ty);
                let value = jsast::method_call(
                    base,
                    "slice",
                    vec![jsast::num(from as f64), jsast::num(end as f64)],
                );
                (value, Ty::new_array(tcx, element, end.saturating_sub(from)))
            }
            _ => (self.zombie(format!("cannot subslice `{base_ty}`")), base_ty),
        }
    }

    pub(crate) fn element_ty(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
        match ty.builtin_index() {
            Some(element) => self.monomorphize(element),
            None => {
                self.zombie(format!("cannot index `{ty}`"));
                ty
            }
        }
    }

    /// The expression that reads a place.
    pub(crate) fn read(&self, place: &JsPlace) -> Expr {
        match place {
            JsPlace::Var(expr) | JsPlace::Deref(expr) => expr.clone(),
            // A copy of the elements: a plain slot hands back the array it holds, a window slot
            // the `n` elements it spans.
            JsPlace::Array(pointer, width) => {
                jsast::rt_call("read_array", vec![pointer.clone(), jsast::num(*width as f64)])
            }
        }
    }

    /// The statements that write `value` to a place.
    ///
    /// A write to a local, a field, an element or a slot is an assignment; a write through a
    /// reference to an aggregate has to overwrite the object in place, which is
    /// [`crate::ptr::write_indirect`].
    pub(crate) fn write_place(&self, place: Place<'tcx>, value: Expr) -> Vec<Stmt> {
        let (target, ty) = self.codegen_place(place);
        match target {
            JsPlace::Var(target) => vec![jsast::assign_stmt(target, value)],
            JsPlace::Deref(target) => ptr::write_indirect(self, ty, target, value),
            // Element by element for a window, an in place overwrite for a plain slot: never a
            // rebind, because the elements live in a buffer other pointers share.
            JsPlace::Array(pointer, _) => {
                vec![jsast::expr_stmt(jsast::rt_call("write_array", vec![pointer, value]))]
            }
        }
    }

    /// `&place` / `&mut place`: a reference.
    ///
    /// A reference to an aggregate is the aggregate's own JavaScript object, which is what keeps
    /// the emitted code readable and what makes a `Deref` of it free. A reference to anything
    /// else is a slot naming the place the walk arrived at.
    pub(crate) fn codegen_ref(&self, place: Place<'tcx>) -> Expr {
        let walk = self.walk_place(place);
        // `&*p` where `*p` is unsized is `p`: the metadata belongs to the reference, not to the
        // pointee, so there is nothing to rebuild.
        if let Some(identity) = walk.identity {
            return identity;
        }
        // `&*p` where the pointee is an array: a reference to an aggregate is the aggregate's own
        // object, and whether the pointer names that object or names the array's elements is a
        // property of the record rather than of the type, so `__rt.array_ref` asks it — exactly as
        // `__rt.read_array` and `__rt.unwindow` ask their version of the same question. A plain
        // slot answers with the array it holds, which is what this always emitted; a pointer over
        // the elements themselves answers with an array that aliases them, which is what
        // `<&mut [T] as TryInto<&mut [T; N]>>` and every `as *mut [E; N]` cast at a non zero
        // offset need and what silently read one element instead.
        if let JsPlace::Array(pointer, width) = &walk.place {
            return jsast::rt_call("array_ref", vec![pointer.clone(), jsast::num(*width as f64)]);
        }
        if is_indirect(self.tcx, walk.ty) {
            return self.read(&walk.place);
        }
        self.slot_of(walk.place, walk.ty)
    }

    /// `&raw const place` / `&raw mut place`: a raw pointer.
    ///
    /// The same walk as [`FnCx::codegen_ref`], and the same answer for everything but an
    /// aggregate: a raw pointer to one is a slot too, because a raw pointer is a thing programs
    /// offset and compare and a bare object is neither. A place inside a container decomposes into
    /// a slot naming it; a bare object with no container goes through `__rt.box`.
    pub(crate) fn codegen_raw_ptr(&self, place: Place<'tcx>) -> Expr {
        let walk = self.walk_place(place);
        if let Some(identity) = walk.identity {
            return identity;
        }
        // `str`, a zero sized pointee: no slot form to build.
        if !ptr::is_slot_pointee(self.tcx, walk.ty) {
            return self.read(&walk.place);
        }
        self.slot_of(walk.place, walk.ty)
    }

    /// The slot naming a place: the place walk, run backwards.
    ///
    /// `A[B]` is `{ buf: A, off: B }` and `A.k` is `{ buf: A, off: "k" }`, both evaluating the key
    /// once, here, which is what makes a slot capture it by value. A place that is not a
    /// projection out of something is a value with nowhere to point: an aggregate is boxed into a
    /// slot by `__rt.box`, and a primitive local should have been boxed by the pre-pass, so
    /// reaching this with one is a bug rather than a shape to lower.
    fn slot_of(&self, place: JsPlace, ty: Ty<'tcx>) -> Expr {
        match place {
            // A reborrow, `&*p`: the place is `p.buf[p.off]`, and the slot naming it is `p`
            // itself. Rebuilding `{ buf: p.buf, off: p.off }` would answer the same for a
            // dereference and lose whatever else the record carried — a scale, a window.
            JsPlace::Var(Expr::Index(base, key)) if reborrowed(&base, &key).is_some() => {
                reborrowed(&base, &key).cloned().unwrap_or_else(jsast::undefined)
            }
            JsPlace::Var(Expr::Index(base, key)) => ptr::slot(*base, *key),
            JsPlace::Var(Expr::Member(base, name)) => ptr::slot(*base, jsast::string(name)),
            // The pointer a `__rt.read_array` place came from already names it: `&raw (*p)` is
            // `p`, window and all.
            JsPlace::Array(pointer, _) => pointer,
            JsPlace::Deref(value) | JsPlace::Var(value) => {
                if is_indirect(self.tcx, ty) {
                    ptr::ref_to_raw(self, ty, value)
                } else {
                    self.zombie(format!(
                        "rustc_codegen_mires cannot take the address of this `{ty}`: it is a \
                         JavaScript value rather than a place, and the pass that boxes the \
                         locals whose address is taken did not see this one"
                    ))
                }
            }
        }
    }

    /// An operand, cloning aggregates that are copied out of a place.
    pub(crate) fn codegen_operand(&self, operand: &Operand<'tcx>) -> Expr {
        match operand {
            Operand::Copy(place) => {
                let (source, ty) = self.codegen_place(*place);
                let value = self.read(&source);
                if needs_clone(self.tcx, ty) { clone_expr(self.tcx, ty, value) } else { value }
            }
            Operand::Move(place) => {
                let (source, _) = self.codegen_place(*place);
                self.read(&source)
            }
            Operand::Constant(constant) => self.at(constant.span, || {
                crate::constant::codegen_constant(self.cgu, &self.zombies, self.instance, constant)
            }),
            // `-Cdebug-assertions=off -Coverflow-checks=off`: every runtime check is disabled.
            Operand::RuntimeChecks(check) => jsast::boolean(check.value(self.tcx.sess)),
        }
    }

    /// An operand read without cloning, for callers that immediately project out of it.
    pub(crate) fn codegen_operand_aliased(&self, operand: &Operand<'tcx>) -> Expr {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (source, _) = self.codegen_place(*place);
                self.read(&source)
            }
            Operand::Constant(_) | Operand::RuntimeChecks(_) => self.codegen_operand(operand),
        }
    }

    /// The type of an operand, monomorphized.
    pub(crate) fn operand_ty(&self, operand: &Operand<'tcx>) -> Ty<'tcx> {
        self.monomorphize(operand.ty(&self.mir.local_decls, self.tcx))
    }

    /// The metadata half of a fat pointer: a slice's length, a trait object's vtable.
    ///
    /// `MIR`'s `PtrMetadata` unary operator and the `ptr_metadata` intrinsic (which the
    /// `LowerIntrinsics` pass rewrites into that operator) both land here.
    pub(crate) fn ptr_metadata(&self, pointer_ty: Ty<'tcx>, value: Expr) -> Expr {
        let Some(pointee) = pointer_ty.builtin_deref(true) else {
            return self.zombie(format!("`{pointer_ty}` has no pointer metadata"));
        };
        match pointee.kind() {
            ty::Slice(_) => ptr::slot_len(value),
            ty::Dynamic(..) => jsast::member(value, ptr::META),
            // A `&str` is a JavaScript string, whose metadata is the length of the bytes it would
            // encode to rather than a field of a record.
            ty::Str => ptr::str_len(value),
            // A thin pointer's metadata is `()`.
            _ if pointee.is_sized(self.tcx, crate::value::typing_env()) => jsast::undefined(),
            // A struct with an unsized tail carries the tail's metadata beside the record, the
            // same way a trait object carries its vtable (`value.rs`).
            _ if crate::value::unsized_tail(self.tcx, pointee).is_some() => {
                jsast::member(value, ptr::META)
            }
            _ => self.zombie(format!(
                "the pointer metadata of `{pointee}` is not supported by rustc_codegen_mires"
            )),
        }
    }
}

/// The pointer `p` of a `p.buf[p.off]` place, which is the place a slot already names.
fn reborrowed<'a>(base: &'a Expr, key: &'a Expr) -> Option<&'a Expr> {
    let (Expr::Member(buffer, buf), Expr::Member(offset, off)) = (base, key) else { return None };
    (buf == ptr::BUF && off == ptr::OFF && buffer == offset).then_some(&**buffer)
}

/// `base + index`, with the `+ 0` of an unshifted slice folded away.
fn offset(base: Expr, index: Expr) -> Expr {
    if index == jsast::num(0) {
        return base;
    }
    jsast::binary(JsBinOp::Add, base, index)
}
