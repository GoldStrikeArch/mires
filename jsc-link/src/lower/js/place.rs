//! Places: the JavaScript that reads and writes the storage a [`Root`] and a [`Proj`] path name
//! (v1 `backend/src/place.rs`, over JIR-R roots instead of MIR locals).
//!
//! The walk is v1's exactly — `Deref` picks the pointer form from the type table, `Field` asks
//! `TypeTable::field`, `Index`/`ConstantIndex`/`Subslice` spell element and window reads,
//! `Downcast` is a no-op — and so are the four shapes it produces (`JsPlace`), the reference and
//! raw pointer built by running it backwards (`borrow_view`/`raw_view`), and the write-through
//! rules.

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{Proj, Root};
use rcgjs_object::jir::types::{FieldKey, TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt};
use rcgjs_object::ptr::{self, BUF, META, OFF, PTR};

use super::value::{self, field_expr, is_slot, is_slot_pointee};
use super::{decline, JsCx, Res};

/// A place, lowered to the JavaScript that reads and writes it (`place.rs::JsPlace`).
#[derive(Clone, Debug)]
pub enum JsPlace {
    /// A JS assignment target: a local, a property, an element, or the `p.buf[p.off]` a pointer
    /// names.
    Var(Expr),
    /// The pointee of a reference to an aggregate: reading is the value itself, writing overwrites
    /// in place.
    Deref(Expr),
    /// The pointee of a slot pointer (`{ buf, off }`, optionally carrying `.sc`/`.w`/`.es` or a
    /// null/dangling address). Kept as the pointer expression until a read, write, or non-transparent
    /// projection actually dereferences it, so `&*p` / `&raw *p` / `addr_of!((*p).0)` return `p`
    /// intact without allocating a temp or stripping metadata, while actual reads/writes hoist
    /// non-duplicable `pointer` expressions once via `reread`.
    Slot {
        pointer: Expr,
        indirect: bool,
        byte_signed: Option<bool>,
    },
    /// One byte element indexed out of a raw `*const [u8]` / `*mut [u8]` / `*const [i8]` /
    /// `*mut [i8]` slice when whole-program analysis sees potential wide-to-byte aliasing.
    ByteAt { buf: Expr, off: Expr, signed: bool },
    /// The `N` elements a pointer to `[E; N]` names, reached through `__rt.read_array` and
    /// `__rt.write_array`. The expression is the pointer.
    Array(Expr, u64),
    /// A place with no storage (`place.rs::JsPlace::Unit`): a zero-sized element of an array or
    /// slice, or anything projected out of a zero-sized array. Reading it is `undefined` and a
    /// write to it stores nothing. The expression is the pointer every reference or raw pointer to
    /// it is: the data pointer of the slice (its `buf`), or the reference form of the array.
    Unit(Expr),
}

/// The result of walking a place.
pub struct Walk {
    pub place: JsPlace,
    pub ty: TypeId,
    /// The pointer a `&`-of-this-place is, when the place is the pointee of a fat pointer.
    pub identity: Option<Expr>,
    /// Whether the walk holds this place as an aggregate *value* rather than as storage: the
    /// pointee of a reference to an aggregate, or an SSA value root of one, before any step
    /// reaches into storage. A reference to an aggregate is the object itself, so the place's
    /// expression is the reference value, spelled as wherever it was read from: `a.r`, `refs[i]`,
    /// a boxed cell's `c[0]`, or an inline value's whole definition. Its raw pointer is the
    /// object's `__rt.box` ([`JsCx::raw_view`]), never the slot of that expression, which would
    /// be the address of the storage *holding* the reference.
    pub object: bool,
    /// Whether a `str` place was reached by dereferencing a `FatSlice` `str` pointer (`&mut str`,
    /// `*const str`, `*mut str`, `NonNull<str>`, `Unique<str>`, `Box<str>`).
    pub str_mutable: bool,
    /// Whether that `str` pointer was specifically mutable (`&mut str` / `*mut str`), so it
    /// cannot alias a memoized `__rt.str_bytes` / `__rt.bstr_bytes` view.
    pub str_from_mut: bool,
}

impl<'a> JsCx<'a> {
    /// The place a root names, before any projection: an SSA value's storage or a cell.
    pub fn root_place(&self, root: &Root) -> Res<(JsPlace, TypeId)> {
        match root {
            Root::Value(v) => Ok((JsPlace::Var(self.expr_of(*v)?), self.ty_of(*v)?)),
            Root::Cell(cell) => Ok((JsPlace::Var(self.cell_expr(*cell)?), self.cell_ty(*cell)?)),
        }
    }

    /// The place walk (`place.rs::walk_place`).
    pub fn walk_place(&self, root: &Root, path: &[Proj]) -> Res<Walk> {
        let table = self.table;
        // `js-str=bytes`: one byte of a byte view held as its string is that string's code unit
        // (`JsCx::byte_string_view` admits only this path, and only where it is read).
        if let (
            Root::Value(view),
            [Proj::Deref, step @ (Proj::Index(_) | Proj::ConstantIndex { .. })],
        ) = (root, path)
        {
            if self.byte_string_view(*view) {
                let string = self.expr_of(*view)?;
                let index = match step {
                    Proj::Index(index) => self.expr_of(*index)?,
                    Proj::ConstantIndex {
                        offset,
                        from_end: false,
                        ..
                    } => jsast::num(*offset as f64),
                    Proj::ConstantIndex { offset, .. } => jsast::binary(
                        BinOp::Sub,
                        jsast::member(string.clone(), "length"),
                        jsast::num(*offset as f64),
                    ),
                    _ => unreachable!("matched above"),
                };
                let pointee = value::pointee_of(table, self.ty_of(*view)?)
                    .ok_or_else(|| decline(OpClass::Place, "a byte view that is not a pointer"))?;
                let element = self.element_ty(pointee)?;
                return Ok(Walk {
                    place: JsPlace::Var(jsast::method_call(string, "charCodeAt", vec![index])),
                    ty: element,
                    identity: None,
                    object: false,
                    str_mutable: false,
                    str_from_mut: false,
                });
            }
        }
        let (mut current, mut ty) = self.root_place(root)?;
        let mut variant: Option<u32> = None;
        let mut identity = None;
        let mut tail: Option<Expr> = None;
        let mut str_mutable = false;
        let mut str_from_mut = false;
        let mut raw_byte_slice: Option<bool> = None;
        // An SSA value has no storage of its own: an aggregate one is held as the object.
        let mut object = matches!(root, Root::Value(_)) && is_object(table, ty);

        for (step, proj) in path.iter().enumerate() {
            identity = None;
            let carried_tail = tail.take();
            let carried_str_mutable = std::mem::take(&mut str_mutable);
            let carried_str_from_mut = std::mem::take(&mut str_from_mut);
            let carried_raw_byte_slice = raw_byte_slice.take();
            // A downcast and a transparent field are the same place; every other step leaves it.
            object &= matches!(proj, Proj::Downcast { .. } | Proj::Field { .. });

            match proj {
                Proj::Deref => {
                    let Some(pointee) = value::pointee_of(table, ty) else {
                        return Err(decline(
                            OpClass::Place,
                            format!(
                                "deref of non-pointer {ty:?} at projection {step} from {root:?}"
                            ),
                        ));
                    };
                    // Nothing inside a place with no storage has storage either, and a pointer
                    // read out of no storage is `undefined`.
                    if matches!(current, JsPlace::Unit(_)) {
                        current = JsPlace::Unit(jsast::undefined());
                        ty = pointee;
                        variant = None;
                        continue;
                    }
                    let value = self.read_place(&current);
                    let shape = table.peeled(pointee);
                    current = match value::kind(table, shape) {
                        Some(TypeKind::Dynamic { .. }) => {
                            identity = Some(value.clone());
                            JsPlace::Deref(jsast::member(value, PTR))
                        }
                        Some(TypeKind::Slice(element)) => {
                            if self.byte_views.raw_byte_derefs && value::is_raw(table, ty) {
                                raw_byte_slice = match value::kind(table, *element) {
                                    Some(TypeKind::Integer { bits: 8, signed }) => Some(*signed),
                                    _ => None,
                                };
                            }
                            identity = Some(value.clone());
                            JsPlace::Deref(value)
                        }
                        Some(TypeKind::Str) => {
                            str_mutable = value::is_mut_str_ptr(table, ty);
                            str_from_mut = value::is_mutable_ptr(table, ty);
                            identity = Some(value.clone());
                            JsPlace::Deref(value)
                        }
                        _ if table.unsized_tail(shape).is_some() => {
                            let value = if value::is_raw(table, ty) {
                                self.raw_to_ref(pointee, value)
                            } else {
                                value
                            };
                            identity = Some(value.clone());
                            tail = Some(jsast::member(value.clone(), META));
                            JsPlace::Deref(jsast::member(value, PTR))
                        }
                        Some(TypeKind::Array { length, .. }) if is_slot(table, ty) => {
                            JsPlace::Array(value, *length)
                        }
                        _ if is_slot(table, ty) => JsPlace::Slot {
                            pointer: value,
                            indirect: table.is_indirect(pointee),
                            byte_signed: if self.byte_views.raw_byte_derefs {
                                match value::kind(table, shape) {
                                    Some(TypeKind::Integer { bits: 8, signed }) => Some(*signed),
                                    _ => None,
                                }
                            } else {
                                None
                            },
                        },
                        // A reference to an aggregate: the pointee is the reference value.
                        _ => {
                            object = is_object(table, pointee);
                            JsPlace::Deref(value)
                        }
                    };
                    ty = pointee;
                    variant = None;
                }
                Proj::Field {
                    field,
                    ty: field_ty,
                } => {
                    let key = match table.field(ty, variant, *field) {
                        Some(descriptor) => descriptor.key,
                        None => {
                            return Err(decline(
                                OpClass::Place,
                                format!("field {field} of a type with no such field"),
                            ))
                        }
                    };
                    object &= matches!(key, FieldKey::Transparent);
                    let field_ty = *field_ty;
                    if matches!(current, JsPlace::Unit(_)) {
                        // Still no storage, and still the same address.
                    } else if matches!(key, FieldKey::Transparent) {
                        tail = carried_tail;
                        str_mutable = carried_str_mutable;
                        str_from_mut = carried_str_from_mut;
                        raw_byte_slice = carried_raw_byte_slice;
                    } else if !table.get(field_ty).is_some_and(|d| d.layout.sized) {
                        let base = self.read_place(&current);
                        current = self.tail_place(base, key, field_ty, carried_tail)?;
                    } else {
                        let base = self.read_place(&current);
                        current = JsPlace::Var(field_expr(self.interns, base, key));
                    }
                    ty = field_ty;
                    variant = None;
                }
                Proj::Index(index) => {
                    let element = self.element_ty(ty)?;
                    if let Some(address) = self.unit_element(&current, ty, element) {
                        // The index is not spelled, so `fuse` never makes it an inline value
                        // ([`reaches_unit`]): it was evaluated where it was defined.
                        current = JsPlace::Unit(address);
                    } else {
                        let index = self.expr_of(*index)?;
                        current = match current {
                            JsPlace::Array(pointer, _) => {
                                JsPlace::Var(self.unwindow_element(pointer, index))
                            }
                            _ => {
                                let base = self.read_place(&current);
                                if let Some(signed) = carried_raw_byte_slice {
                                    let (setup, slot) = self.reread(base, &[BUF, OFF]);
                                    JsPlace::ByteAt {
                                        buf: super::ptr::sequenced(
                                            setup,
                                            ptr::slot_buf(slot.clone()),
                                        ),
                                        off: offset(ptr::slot_off(slot), index),
                                        signed,
                                    }
                                } else {
                                    JsPlace::Var(self.element_expr(ty, base, index))
                                }
                            }
                        };
                    }
                    ty = element;
                    variant = None;
                }
                Proj::ConstantIndex {
                    offset: idx_offset,
                    from_end,
                    ..
                } => {
                    let element = self.element_ty(ty)?;
                    if let Some(address) = self.unit_element(&current, ty, element) {
                        current = JsPlace::Unit(address);
                    } else {
                        current = match current {
                            JsPlace::Array(pointer, length) => {
                                let index = if *from_end {
                                    length.saturating_sub(*idx_offset)
                                } else {
                                    *idx_offset
                                };
                                JsPlace::Var(
                                    self.unwindow_element(pointer, jsast::num(index as f64)),
                                )
                            }
                            _ => {
                                let base = self.read_place(&current);
                                let index =
                                    self.constant_index(ty, base.clone(), *idx_offset, *from_end);
                                if let Some(signed) = carried_raw_byte_slice {
                                    let (setup, slot) = self.reread(base, &[BUF, OFF]);
                                    JsPlace::ByteAt {
                                        buf: super::ptr::sequenced(
                                            setup,
                                            ptr::slot_buf(slot.clone()),
                                        ),
                                        off: offset(ptr::slot_off(slot), index),
                                        signed,
                                    }
                                } else {
                                    JsPlace::Var(self.element_expr(ty, base, index))
                                }
                            }
                        };
                    }
                    ty = element;
                    variant = None;
                }
                // A run of a zero-sized array is a zero-sized array at the same address. A run of
                // a slice stays a fat slice whatever its element is, because its `len` is the
                // element count.
                Proj::Subslice { .. }
                    if matches!(current, JsPlace::Unit(_)) || table.is_zst(ty) =>
                {
                    if !matches!(current, JsPlace::Unit(_)) {
                        current = JsPlace::Unit(self.read_place(&current));
                    }
                    variant = None;
                }
                Proj::Subslice { from, to, from_end } => {
                    let array_source = match current {
                        JsPlace::Array(pointer, length) => Ok((pointer, length)),
                        other => match value::kind(table, ty) {
                            Some(TypeKind::Array { length, .. }) => {
                                Ok((self.read_place(&other), *length))
                            }
                            _ => Err(other),
                        },
                    };
                    match array_source {
                        Ok((source, length)) => {
                            let end = if *from_end {
                                length.saturating_sub(*to)
                            } else {
                                *to
                            };
                            let sub_len = end.saturating_sub(*from);
                            let (setup, slot) =
                                self.reread(jsast::rt_call("unwindow", vec![source]), &[BUF, OFF]);
                            let window = ptr::window_slot(
                                ptr::slot_buf(slot.clone()),
                                offset(ptr::slot_off(slot), jsast::num(*from as f64)),
                                sub_len,
                            );
                            current = JsPlace::Array(super::ptr::sequenced(setup, window), sub_len);
                            variant = None;
                        }
                        Err(other) => {
                            let base = self.read_place(&other);
                            let (value, subslice_ty) =
                                self.subslice(ty, base, *from, *to, *from_end)?;
                            current = JsPlace::Deref(value);
                            ty = subslice_ty;
                            variant = None;
                        }
                    }
                }
                Proj::Downcast { variant: index } => {
                    variant = Some(*index);
                    tail = carried_tail;
                    str_mutable = carried_str_mutable;
                    str_from_mut = carried_str_from_mut;
                    raw_byte_slice = carried_raw_byte_slice;
                }
            }
        }
        Ok(Walk {
            place: current,
            ty,
            identity,
            object,
            str_mutable,
            str_from_mut,
        })
    }

    /// The place a struct's unsized tail is (`place.rs::tail_place`).
    fn tail_place(
        &self,
        record: Expr,
        key: FieldKey,
        tail_ty: TypeId,
        tail: Option<Expr>,
    ) -> Res<JsPlace> {
        let Some(tail) = tail else {
            return Err(decline(
                OpClass::Place,
                "the unsized tail of a struct projected out of a place that is not behind a { ptr, meta } pointer",
            ));
        };
        if !matches!(value::kind(self.table, tail_ty), Some(TypeKind::Slice(_))) {
            return Err(decline(
                OpClass::Place,
                "a struct with a non-slice unsized tail",
            ));
        }
        Ok(JsPlace::Deref(jsast::rt_call(
            "tail_slice",
            vec![
                record,
                jsast::string(value::key_name(self.interns, key).expect("tail field key")),
                tail,
            ],
        )))
    }

    /// The address of an element of `base` (of type `base_ty`) when the element has no storage
    /// ([`element_is_unit`]), and `None` when it has. Every element of such a run is at one
    /// address: the one the base already had, the data pointer of a slice (its `buf`, which is
    /// what `as_ptr` answers), or the reference form of a zero-sized array. None of the three
    /// reads an element.
    fn unit_element(&self, base: &JsPlace, base_ty: TypeId, element: TypeId) -> Option<Expr> {
        if let JsPlace::Unit(address) = base {
            return Some(address.clone());
        }
        if !element_is_unit(self.table, base_ty, element) {
            return None;
        }
        let base = self.read_place(base);
        Some(match value::kind(self.table, base_ty) {
            Some(TypeKind::Slice(_)) => ptr::slot_buf(base),
            _ => base,
        })
    }

    /// `pointer.buf[pointer.off]`, hoisting non-duplicable `pointer` expressions once via `reread`.
    fn slot_element(&self, pointer: Expr) -> Expr {
        super::ptr::slot_element_unwrapped(self, pointer)
    }

    /// One element of a slot-pointed `[E; N]` (plain slot or window slot), normalized once
    /// through `__rt.unwindow` into a hoisted temporary so `buf` and `off` come from the same
    /// record without double evaluation.
    fn unwindow_element(&self, pointer: Expr, index: Expr) -> Expr {
        let (setup, slot) = self.reread(jsast::rt_call("unwindow", vec![pointer]), &[BUF, OFF]);
        jsast::index(
            super::ptr::sequenced(setup, ptr::slot_buf(slot.clone())),
            offset(ptr::slot_off(slot), index),
        )
    }

    /// One element of an array or a slice: `base[i]` or `base.buf[base.off + i]`.
    pub fn element_expr(&self, base_ty: TypeId, base: Expr, index: Expr) -> Expr {
        match value::kind(self.table, base_ty) {
            Some(TypeKind::Slice(_)) => jsast::index(
                ptr::slot_buf(base.clone()),
                offset(ptr::slot_off(base), index),
            ),
            _ => jsast::index(base, index),
        }
    }

    /// The index a `ConstantIndex` projection reads.
    fn constant_index(&self, base_ty: TypeId, base: Expr, offset: u64, from_end: bool) -> Expr {
        if !from_end {
            return jsast::num(offset as f64);
        }
        let length = match value::kind(self.table, base_ty) {
            Some(TypeKind::Slice(_)) => ptr::slot_len(base),
            _ => jsast::member(base, "length"),
        };
        jsast::binary(BinOp::Sub, length, jsast::num(offset as f64))
    }

    /// A `Subslice` projection (`place.rs::subslice`).
    fn subslice(
        &self,
        base_ty: TypeId,
        base: Expr,
        from: u64,
        to: u64,
        from_end: bool,
    ) -> Res<(Expr, TypeId)> {
        match value::kind(self.table, base_ty) {
            Some(TypeKind::Slice(_)) => {
                let len = ptr::slot_len(base.clone());
                let len = if from_end {
                    let trimmed = from + to;
                    if trimmed == 0 {
                        len
                    } else {
                        jsast::binary(BinOp::Sub, len, jsast::num(trimmed as f64))
                    }
                } else {
                    jsast::num(to.saturating_sub(from) as f64)
                };
                let value = ptr::fat_slice(
                    ptr::slot_buf(base.clone()),
                    offset(ptr::slot_off(base), jsast::num(from as f64)),
                    len,
                );
                Ok((value, base_ty))
            }
            Some(TypeKind::Array { length, .. }) => {
                let size = *length;
                let end = if from_end {
                    size.saturating_sub(to)
                } else {
                    to
                };
                let value = jsast::method_call(
                    base,
                    "slice",
                    vec![jsast::num(from as f64), jsast::num(end as f64)],
                );
                // The subslice of a fixed size array is another fixed size array; its type is not
                // in the table unless the body mentions it, and nothing after a `Subslice` asks
                // for it but a further projection, which reads the same element type.
                Ok((value, base_ty))
            }
            _ => Err(decline(OpClass::Place, "subslice of a non-array place")),
        }
    }

    /// The element type of an array or slice.
    pub fn element_ty(&self, ty: TypeId) -> Res<TypeId> {
        match value::kind(self.table, ty) {
            Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => Ok(*element),
            _ => Err(decline(OpClass::Place, "index of a non-array place")),
        }
    }

    /// The expression that reads a place.
    pub fn read_place(&self, place: &JsPlace) -> Expr {
        match place {
            JsPlace::Var(expr) | JsPlace::Deref(expr) => expr.clone(),
            JsPlace::Slot {
                pointer,
                byte_signed: Some(signed),
                ..
            } => {
                let mut args = vec![pointer.clone()];
                if *signed {
                    args.push(Expr::Bool(true));
                }
                jsast::rt_call("read_byte", args)
            }
            JsPlace::Slot { pointer, .. } => self.slot_element(pointer.clone()),
            JsPlace::ByteAt { buf, off, signed } => {
                let mut args = vec![buf.clone(), off.clone()];
                if *signed {
                    args.push(Expr::Bool(true));
                }
                jsast::rt_call("read_byte_at", args)
            }
            JsPlace::Array(pointer, width) => jsast::rt_call(
                "read_array",
                vec![pointer.clone(), jsast::num(*width as f64)],
            ),
            JsPlace::Unit(_) => jsast::undefined(),
        }
    }

    /// The statements that write `value` to a place of type `ty` (`place.rs::write_place`).
    pub fn write_place(&self, place: JsPlace, ty: TypeId, value: Expr) -> Res<Vec<Stmt>> {
        match place {
            JsPlace::Var(target) => {
                if !is_lvalue(&target) {
                    return Err(decline(
                        OpClass::Place,
                        "a write to a place whose expression is not assignable (an inlined value used as storage)",
                    ));
                }
                Ok(vec![jsast::assign_stmt(target, value)])
            }
            JsPlace::Deref(target) => self.write_indirect(ty, target, value),
            JsPlace::Slot {
                pointer,
                byte_signed: Some(_),
                ..
            } => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_byte",
                vec![pointer, value],
            ))]),
            JsPlace::Slot {
                pointer, indirect, ..
            } => {
                let target = self.slot_element(pointer);
                if indirect {
                    self.write_indirect(ty, target, value)
                } else {
                    if !is_lvalue(&target) {
                        return Err(decline(
                            OpClass::Place,
                            "a write to a place whose expression is not assignable (an inlined value used as storage)",
                        ));
                    }
                    Ok(vec![jsast::assign_stmt(target, value)])
                }
            }
            JsPlace::ByteAt { buf, off, .. } => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_byte_at",
                vec![buf, off, value],
            ))]),
            JsPlace::Array(pointer, _) => {
                let mut args = vec![pointer, value];
                if let Some(TypeKind::Array { element, .. }) = value::kind(self.table, ty) {
                    if self.table.is_indirect(*element) {
                        args.push(Expr::Bool(true));
                    }
                }
                Ok(vec![jsast::expr_stmt(jsast::rt_call("write_array", args))])
            }
            // Nothing to store. A value with effects of its own is still evaluated.
            JsPlace::Unit(_) => Ok(match value {
                Expr::Undefined => Vec::new(),
                value => vec![jsast::expr_stmt(value)],
            }),
        }
    }

    /// `&place` / `&mut place` (`place.rs::codegen_ref`).
    pub fn borrow_view(&self, root: &Root, path: &[Proj], mutable: bool) -> Res<Expr> {
        let walk = self.walk_place(root, path)?;
        if value::is_str(self.table, walk.ty) {
            let value = walk
                .identity
                .unwrap_or_else(|| self.read_place(&walk.place));
            return Ok(match (walk.str_mutable, mutable) {
                (true, true) => {
                    let live = if walk.str_from_mut {
                        value
                    } else {
                        jsast::rt_call("detach_str_memo", vec![value])
                    };
                    jsast::rt_call("byte_slice_ref", vec![live])
                }
                (true, false) => self.bytes_to_str(value),
                (false, false) => value,
                (false, true) => self.str_to_bytes_mut(value),
            });
        }
        if let Some(identity) = walk.identity {
            if let Some(TypeKind::Slice(element)) = value::kind(self.table, walk.ty) {
                if let Some(TypeKind::Integer { bits: 8, signed }) =
                    value::kind(self.table, *element)
                {
                    let mut args = vec![identity];
                    if *signed {
                        args.push(Expr::Bool(true));
                    }
                    return Ok(jsast::rt_call("byte_slice_ref", args));
                }
            }
            return Ok(identity);
        }
        if let JsPlace::Array(pointer, width) = &walk.place {
            if self.table.is_indirect(walk.ty) {
                let mut args = vec![pointer.clone(), jsast::num(*width as f64)];
                if let Some(TypeKind::Array { element, .. }) = value::kind(self.table, walk.ty) {
                    if let Some(elem_size) = self
                        .table
                        .get(*element)
                        .and_then(|d| d.layout.size)
                        .filter(|&s| s > 0)
                    {
                        args.push(jsast::num(elem_size as f64));
                    }
                }
                return Ok(jsast::rt_call("array_ref", args));
            }
            return Ok(pointer.clone());
        }
        if let JsPlace::Unit(address) = walk.place {
            return Ok(address);
        }
        if self.table.is_indirect(walk.ty) {
            let value = self.read_place(&walk.place);
            if let Some(TypeKind::Slice(element)) =
                value::kind(self.table, self.table.peeled(walk.ty))
            {
                if let Some(TypeKind::Integer { bits: 8, signed }) =
                    value::kind(self.table, self.table.peeled(*element))
                {
                    let mut args = vec![value];
                    if *signed {
                        args.push(Expr::Bool(true));
                    }
                    return Ok(jsast::rt_call("byte_slice_ref", args));
                }
            }
            return Ok(value);
        }
        self.slot_of(walk.place, walk.ty)
    }

    /// `&raw const place` / `&raw mut place` (`place.rs::codegen_raw_ptr`).
    pub fn raw_view(&self, root: &Root, path: &[Proj], mutable: bool) -> Res<Expr> {
        let walk = self.walk_place(root, path)?;
        if value::is_str(self.table, walk.ty) {
            let value = walk
                .identity
                .unwrap_or_else(|| self.read_place(&walk.place));
            return Ok(match (walk.str_mutable, mutable) {
                (true, true) if !walk.str_from_mut => {
                    jsast::rt_call("detach_str_memo", vec![value])
                }
                (true, _) => value,
                (false, true) => self.str_to_bytes_mut(value),
                (false, false) => self.str_to_bytes(value),
            });
        }
        if let Some(identity) = walk.identity {
            return Ok(self.ref_to_raw(walk.ty, identity));
        }
        if let JsPlace::Unit(address) = walk.place {
            return Ok(address);
        }
        if let JsPlace::Array(pointer, 0) = walk.place {
            let mut args = vec![pointer, jsast::num(0)];
            if let Some(TypeKind::Array { element, .. }) = value::kind(self.table, walk.ty) {
                if let Some(elem_size) = self
                    .table
                    .get(*element)
                    .and_then(|d| d.layout.size)
                    .filter(|&s| s > 0)
                {
                    args.push(jsast::num(elem_size as f64));
                }
            }
            return Ok(jsast::rt_call("array_ref", args));
        }
        if !is_slot_pointee(self.table, walk.ty) {
            return Ok(self.read_place(&walk.place));
        }
        // An aggregate held as a value: its box, whatever expression the value was read with.
        if walk.object {
            return Ok(jsast::rt_call("box", vec![self.read_place(&walk.place)]));
        }
        self.slot_of(walk.place, walk.ty)
    }

    /// The slot naming a place: the place walk, run backwards (`place.rs::slot_of`).
    fn slot_of(&self, place: JsPlace, ty: TypeId) -> Res<Expr> {
        match place {
            JsPlace::Var(Expr::Index(base, key)) if reborrowed(&base, &key).is_some() => {
                Ok(reborrowed(&base, &key)
                    .cloned()
                    .unwrap_or_else(jsast::undefined))
            }
            JsPlace::Var(Expr::Index(base, key)) => Ok(ptr::slot(*base, *key)),
            JsPlace::Var(Expr::Member(base, name)) => Ok(ptr::slot(*base, jsast::string(name))),
            JsPlace::Slot { pointer, .. } | JsPlace::Array(pointer, _) => Ok(pointer),
            JsPlace::ByteAt { buf, off, .. } => Ok(ptr::slot(buf, off)),
            // No storage, so no slot either: the pointer is the address the run shares.
            JsPlace::Unit(address) => Ok(address),
            JsPlace::Deref(value) | JsPlace::Var(value) => {
                if self.table.is_indirect(ty) {
                    Ok(self.ref_to_raw(ty, value))
                } else {
                    Err(decline(
                        OpClass::Place,
                        "the address of a direct value that is not in a cell (the place has no container to be the buf of)",
                    ))
                }
            }
        }
    }

    /// The metadata half of a fat pointer (`place.rs::ptr_metadata`).
    pub fn ptr_metadata(&self, pointer_ty: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        let Some(pointee) = value::pointee_of(table, pointer_ty) else {
            return Err(decline(OpClass::Rvalue, "PtrMetadata of a non-pointer"));
        };
        match value::kind(table, pointee) {
            Some(TypeKind::Slice(_)) => Ok(self.slice_len(value)),
            Some(TypeKind::Dynamic { .. }) => Ok(jsast::member(value, META)),
            Some(TypeKind::Str) => Ok(if value::is_mut_str_ptr(table, pointer_ty) {
                self.slice_len(value)
            } else {
                self.str_len(value)
            }),
            _ if table.get(pointee).is_some_and(|d| d.layout.sized) => Ok(jsast::undefined()),
            _ if table.unsized_tail(pointee).is_some() => Ok(jsast::member(value, META)),
            _ => Err(decline(
                OpClass::Rvalue,
                "PtrMetadata of a pointee whose metadata has no JavaScript form",
            )),
        }
    }
}

/// Whether a raw pointer to a place of type `ty` held as a value is the object's `__rt.box`
/// (`ptr.rs::ref_to_raw`): a sized, non-zero-sized aggregate whose reference is the object itself.
fn is_object(table: &TypeTable, ty: TypeId) -> bool {
    table.is_indirect(ty) && is_slot_pointee(table, ty) && table.unsized_tail(ty).is_none()
}

/// Whether an element of an array or slice of type `base_ty` has no storage to name
/// ([`JsPlace::Unit`], `place.rs::element_is_unit`): the base is a zero-sized array, which is
/// `undefined` like every zero-sized value, or the element is zero-sized, which a slice holds
/// nowhere at all: its `buf` is only the data pointer it was built from (`undefined` when it was
/// unsized from such an array, a dangling address under a collection that never allocated).
pub fn element_is_unit(table: &TypeTable, base_ty: TypeId, element: TypeId) -> bool {
    table.is_zst(base_ty) || table.is_zst(element)
}

/// Whether the walk of `path` from a root of type `root_ty` reaches a place with no storage. Such
/// a place does not spell the index that selected it, and is not spelled at all where it is only
/// read or written, so the plan keeps every operand of the view (its root value and its indices)
/// out of the inline set: an inline value is evaluated where it is spelled or not at all. A step
/// whose type the table cannot answer counts as reaching one, which costs a name, never an
/// evaluation.
pub fn reaches_unit(table: &TypeTable, root_ty: TypeId, path: &[Proj]) -> bool {
    let mut ty = Some(root_ty);
    for proj in path {
        match proj {
            Proj::Deref => ty = ty.and_then(|ty| value::pointee_of(table, ty)),
            Proj::Field { ty: field, .. } => ty = Some(*field),
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                let element = ty.and_then(|ty| match value::kind(table, ty) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                        Some(*element)
                    }
                    _ => None,
                });
                match (ty, element) {
                    (Some(base), Some(element)) if !element_is_unit(table, base, element) => {
                        ty = Some(element);
                    }
                    _ => return true,
                }
            }
            Proj::Subslice { .. } if ty.is_none_or(|ty| table.is_zst(ty)) => return true,
            Proj::Subslice { .. } | Proj::Downcast { .. } => {}
        }
    }
    false
}

/// The pointer `p` of a `p.buf[p.off]` place.
fn reborrowed<'e>(base: &'e Expr, key: &'e Expr) -> Option<&'e Expr> {
    let (Expr::Member(buffer, buf), Expr::Member(offset, off)) = (base, key) else {
        return None;
    };
    (buf == BUF && off == OFF && buffer == offset).then_some(&**buffer)
}

/// `base + index`, with the `+ 0` of an unshifted slice folded away.
pub fn offset(base: Expr, index: Expr) -> Expr {
    if index == jsast::num(0) {
        return base;
    }
    jsast::binary(BinOp::Add, base, index)
}

/// Whether an expression can stand on the left of `=`.
pub fn is_lvalue(expr: &Expr) -> bool {
    matches!(expr, Expr::Ident(_) | Expr::Member(..) | Expr::Index(..))
}
