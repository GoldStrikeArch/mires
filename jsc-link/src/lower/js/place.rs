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

/// Byte-aliasing access mode for a [`JsPlace::Slot`] dereference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotByteAccess {
    /// Regular slot element access (`pointer.buf[pointer.off]`).
    None,
    /// Aliasing unsigned byte read/write (`__rt.read_byte(p)` / `__rt.write_byte(p, v)`).
    Unsigned,
    /// Aliasing signed byte read/write (`__rt.read_byte(p, true)` / `__rt.write_byte(p, v)`).
    Signed,
}

/// How a `str` place was reached by dereferencing its pointer.
///
/// Eliminates the invalid `(fat: false, from_mut: true)` combination: an immutable `&str` is a
/// JavaScript string (`None`), while `&mut str` / `*mut str` (`FatMut`) and `*const str` /
/// `NonNull<str>` / `Unique<str>` / `Box<str>` (`FatConst`) are live `{ buf, off, len }` UTF-8
/// byte slices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrPlaceKind {
    /// Not a `FatSlice` `str` deref (either a non-`str` place or an immutable `&str` JS string).
    None,
    /// Dereferenced from an immutable `FatSlice` `str` pointer (`*const str`, `NonNull<str>`,
    /// `Unique<str>`, `Box<str>`), which may alias a memoized `__rt.str_bytes`/`__rt.bstr_bytes`
    /// view and must detach before mutating.
    FatConst,
    /// Dereferenced from a mutable `FatSlice` `str` pointer (`&mut str`, `*mut str`).
    FatMut,
}

/// Metadata carried from a `Proj::Deref` across transparent `Proj::Field` and `Proj::Downcast`
/// steps to the projection that consumes it. Keeping all carried variants in one enum ensures
/// transparent and downcast steps restore every piece of metadata atomically.
enum DerefCarry {
    None,
    UnsizedTail { meta: Expr, raw: Option<Expr> },
    Str(StrPlaceKind),
    RawByteSlice { signed: bool },
}

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
        byte_access: SlotByteAccess,
    },
    /// One byte element indexed out of a raw `*const [u8]` / `*mut [u8]` / `*const [i8]` /
    /// `*mut [i8]` slice when whole-program analysis sees potential wide-to-byte aliasing.
    ByteAt { buf: Expr, off: Expr, signed: bool },
    /// An element of a Structure-of-Arrays (SoA) `Vec<T>` / `&[T]` / `&T`: `buf` is the
    /// `{ f0: TA0, f1: TA1, ... }` columns object and `off` is the element index.
    SoaElem {
        buf: Expr,
        off: Expr,
        elem_ty: TypeId,
    },
    /// A `bool` field stored as `0`/`1` inside an SoA `Uint8Array` column.
    SoaBool { buf: Expr, col: String, off: Expr },
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
    /// Whether a `str` place was reached by dereferencing a `FatSlice` `str` pointer, and whether
    /// that pointer was mutable.
    pub str_kind: StrPlaceKind,
    /// Whether the final projection landed on a `heap.smi_u32_fields` struct field stored as
    /// signed `Int32`.
    pub smi_u32: bool,
    /// Whether the last storage-selecting step was a non-transparent `Proj::Field` (struct, tuple,
    /// or enum field, as opposed to an array/slice element).
    pub field_slot: bool,
}

/// Mutable walk cursor threaded across projection steps in [`JsCx::walk_place`].
struct WalkState {
    current: JsPlace,
    ty: TypeId,
    variant: Option<u32>,
    identity: Option<Expr>,
    carry: DerefCarry,
    object: bool,
    smi_u32: bool,
    field_slot: bool,
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
        // `js-str=bytes`: one byte of a byte view held as its string (or `{ buf, off, len }`
        // byte-string subslice) is that string's code unit.
        if let Some(walk) = self.try_walk_byte_view_place(root, path)? {
            return Ok(walk);
        }
        let (current, ty) = self.root_place(root)?;
        // An SSA value has no storage of its own: an aggregate one is held as the object.
        let object = matches!(root, Root::Value(_)) && is_object(self.table, ty);
        let mut state = WalkState {
            current,
            ty,
            variant: None,
            identity: None,
            carry: DerefCarry::None,
            object,
            smi_u32: false,
            field_slot: false,
        };

        for (step, proj) in path.iter().enumerate() {
            state.identity = None;
            state.smi_u32 = false;
            let carried = std::mem::replace(&mut state.carry, DerefCarry::None);
            // A downcast and a transparent field are the same place; every other step leaves it.
            state.object &= matches!(proj, Proj::Downcast { .. } | Proj::Field { .. });
            state.field_slot &= matches!(proj, Proj::Downcast { .. } | Proj::Field { .. });

            match proj {
                Proj::Deref => self.step_deref(root, step, &mut state)?,
                Proj::Field {
                    field,
                    ty: field_ty,
                } => self.step_field(*field, *field_ty, carried, &mut state)?,
                Proj::Index(index) => self.step_index(root, step, *index, carried, &mut state)?,
                Proj::ConstantIndex {
                    offset: idx_offset,
                    from_end,
                    ..
                } => self.step_constant_index(
                    root,
                    step,
                    *idx_offset,
                    *from_end,
                    carried,
                    &mut state,
                )?,
                Proj::Subslice { from, to, from_end } => {
                    self.step_subslice(*from, *to, *from_end, &mut state)?;
                }
                Proj::Downcast { variant: index } => {
                    Self::step_downcast(*index, carried, &mut state);
                }
            }
        }
        let str_kind = match state.carry {
            DerefCarry::Str(kind) => kind,
            _ => StrPlaceKind::None,
        };
        Ok(Walk {
            place: state.current,
            ty: state.ty,
            identity: state.identity,
            object: state.object,
            str_kind,
            smi_u32: state.smi_u32,
            field_slot: state.field_slot,
        })
    }

    /// Fast path for indexing a `js-str=bytes` byte view (`FullSlice`, `VarSlice`, or array
    /// `ThinPtr`) directly via `base.charCodeAt(total)`.
    fn try_walk_byte_view_place(&self, root: &Root, path: &[Proj]) -> Res<Option<Walk>> {
        let (
            Root::Value(view),
            [Proj::Deref, step @ (Proj::Index(_) | Proj::ConstantIndex { .. })],
        ) = (root, path)
        else {
            return Ok(None);
        };
        let Some(bv) = self.byte_view(*view) else {
            return Ok(None);
        };
        let table = self.table;
        let (base, off, index) = match bv.mode {
            super::ByteViewMode::FullSlice => {
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
                (string, None, index)
            }
            super::ByteViewMode::VarSlice => {
                let base = self.byte_view_base_expr(bv, *view)?;
                let off = self.byte_view_off_expr(*view)?;
                let index = match step {
                    Proj::Index(index) => self.expr_of(*index)?,
                    Proj::ConstantIndex {
                        offset,
                        from_end: false,
                        ..
                    } => jsast::num(*offset as f64),
                    Proj::ConstantIndex { offset, .. } => {
                        let len = self.byte_view_len_expr(*view)?;
                        jsast::binary(
                            BinOp::UShr,
                            jsast::binary(BinOp::Sub, len, jsast::num(*offset as f64)),
                            jsast::num(0),
                        )
                    }
                    _ => unreachable!("matched above"),
                };
                (base, Some(off), index)
            }
            super::ByteViewMode::ThinPtr => {
                let Some(arr_len) = super::byte_view_analysis::is_immutable_byte_array_ptr(
                    table,
                    self.ty_of(*view)?,
                ) else {
                    return Ok(None);
                };
                let base = self.byte_view_base_expr(bv, *view)?;
                let off = self.byte_view_off_expr(*view)?;
                let index = match step {
                    Proj::Index(index) => self.expr_of(*index)?,
                    Proj::ConstantIndex {
                        offset,
                        from_end: false,
                        ..
                    } => jsast::num(*offset as f64),
                    Proj::ConstantIndex { offset, .. } => {
                        jsast::num(arr_len.saturating_sub(*offset) as f64)
                    }
                    _ => unreachable!("matched above"),
                };
                (base, Some(off), index)
            }
        };
        let total = match off {
            None => index,
            Some(Expr::Num(0.0)) => index,
            Some(off) if matches!(index, Expr::Num(n) if n == 0.0) => off,
            Some(off) => jsast::binary(
                BinOp::UShr,
                jsast::binary(BinOp::Add, off, index),
                jsast::num(0),
            ),
        };
        let pointee = value::pointee_of(table, self.ty_of(*view)?)
            .ok_or_else(|| decline(OpClass::Place, "a byte view that is not a pointer"))?;
        let element = self.element_ty(pointee)?;
        Ok(Some(Walk {
            place: JsPlace::Var(jsast::method_call(base, "charCodeAt", vec![total])),
            ty: element,
            identity: None,
            object: false,
            str_kind: StrPlaceKind::None,
            smi_u32: false,
            field_slot: false,
        }))
    }

    fn step_deref(&self, root: &Root, step: usize, state: &mut WalkState) -> Res<()> {
        let table = self.table;
        let Some(pointee) = value::pointee_of(table, state.ty) else {
            return Err(decline(
                OpClass::Place,
                format!(
                    "deref of non-pointer {:?} at projection {step} from {root:?}",
                    state.ty
                ),
            ));
        };
        // Nothing inside a place with no storage has storage either, and a pointer read out of no
        // storage is `undefined`.
        if matches!(state.current, JsPlace::Unit(_)) {
            state.current = JsPlace::Unit(jsast::undefined());
            state.ty = pointee;
            state.variant = None;
            return Ok(());
        }
        if self.soa_layout(pointee).is_some() {
            let cached = (step == 0)
                .then(|| match root {
                    Root::Value(v) => self.soa_refs.borrow().get(v).cloned(),
                    Root::Cell(_) => None,
                })
                .flatten();
            let (buf, off) = match cached {
                Some(pair) => pair,
                None => {
                    let value = self.read_place(&state.current);
                    self.soa_unpack_slot_expr_for_place(value)
                }
            };
            state.current = JsPlace::SoaElem {
                buf,
                off,
                elem_ty: pointee,
            };
            state.object = false;
            state.ty = pointee;
            state.variant = None;
            return Ok(());
        }
        let value = self.read_place(&state.current);
        let shape = table.peeled(pointee);
        state.current = match value::kind(table, shape) {
            Some(TypeKind::Dynamic { .. }) => {
                state.identity = Some(value.clone());
                JsPlace::Deref(jsast::member(value, PTR))
            }
            Some(TypeKind::Slice(element)) => {
                if self.byte_views.raw_byte_derefs && value::is_raw(table, state.ty) {
                    if let Some(TypeKind::Integer { bits: 8, signed }) =
                        value::kind(table, *element)
                    {
                        state.carry = DerefCarry::RawByteSlice { signed: *signed };
                    }
                }
                state.identity = Some(value.clone());
                JsPlace::Deref(value)
            }
            Some(TypeKind::Str) => {
                let str_kind = if value::is_mutable_ptr(table, state.ty) {
                    StrPlaceKind::FatMut
                } else if value::is_mut_str_ptr(table, state.ty) {
                    StrPlaceKind::FatConst
                } else {
                    StrPlaceKind::None
                };
                if str_kind != StrPlaceKind::None {
                    state.carry = DerefCarry::Str(str_kind);
                }
                state.identity = Some(value.clone());
                JsPlace::Deref(value)
            }
            _ if table.unsized_tail(shape).is_some() => {
                let is_raw_tail = value::is_raw(table, state.ty);
                let (raw, value) = if is_raw_tail {
                    (Some(value.clone()), self.raw_to_ref(pointee, value))
                } else {
                    (None, value)
                };
                state.identity = Some(value.clone());
                state.carry = DerefCarry::UnsizedTail {
                    meta: jsast::member(value.clone(), META),
                    raw,
                };
                JsPlace::Deref(jsast::member(value, PTR))
            }
            Some(TypeKind::Array { length, .. }) if is_slot(table, state.ty) => {
                JsPlace::Array(value, *length)
            }
            // Dereferencing a pointer to a zero-sized pointee (e.g. `*const ()`,
            // `&mut MaybeUninit<()>`, `Box<MaybeUninit<()>>`) has no storage; its address is
            // the pointer value itself, reading it is `undefined`, and writing it is a no-op.
            _ if table.is_zst(pointee) => JsPlace::Unit(value),
            _ if is_slot(table, state.ty) => JsPlace::Slot {
                pointer: value,
                indirect: table.is_indirect(pointee)
                    || (table.crosses_transparent_union(pointee)
                        && table.is_indirect(table.peeled(pointee))),
                byte_access: if self.byte_views.raw_byte_derefs {
                    match value::kind(table, shape) {
                        Some(TypeKind::Integer {
                            bits: 8,
                            signed: true,
                        }) => SlotByteAccess::Signed,
                        Some(TypeKind::Integer {
                            bits: 8,
                            signed: false,
                        }) => SlotByteAccess::Unsigned,
                        _ => SlotByteAccess::None,
                    }
                } else {
                    SlotByteAccess::None
                },
            },
            // A reference to an aggregate: the pointee is the reference value.
            _ => {
                state.object = is_object(table, pointee);
                JsPlace::Deref(value)
            }
        };
        state.ty = pointee;
        state.variant = None;
        Ok(())
    }

    fn step_field(
        &self,
        field: u32,
        field_ty: TypeId,
        carried: DerefCarry,
        state: &mut WalkState,
    ) -> Res<()> {
        let table = self.table;
        let (key, field_offset) = match table.field(state.ty, state.variant, field) {
            Some(descriptor) => (descriptor.key, descriptor.offset),
            None => {
                return Err(decline(
                    OpClass::Place,
                    format!("field {field} of a type with no such field"),
                ))
            }
        };
        state.object &= matches!(key, FieldKey::Transparent);
        state.smi_u32 = state.variant.is_none() && self.is_smi_u32_field(state.ty, field);
        if let JsPlace::SoaElem { buf, off, elem_ty } = &state.current {
            let (buf, off, elem_ty) = (buf.clone(), off.clone(), *elem_ty);
            if table.is_zst(field_ty) {
                state.current = JsPlace::Unit(jsast::undefined());
            } else if let Some(col) = self.soa_layout(elem_ty).and_then(|l| l.field(field)) {
                if col.scalar.is_bool() {
                    state.current = JsPlace::SoaBool {
                        buf,
                        col: col.col_name.clone(),
                        off,
                    };
                } else {
                    state.current =
                        JsPlace::Var(jsast::index(jsast::member(buf, col.col_name.clone()), off));
                }
            } else {
                return Err(decline(
                    OpClass::Place,
                    format!("SoA field {field} missing from layout"),
                ));
            }
            state.ty = field_ty;
            state.variant = None;
            state.field_slot = false;
            return Ok(());
        }
        if matches!(state.current, JsPlace::Unit(_)) {
            // Still no storage, and still the same address.
            state.field_slot = false;
        } else if table.is_zst(field_ty)
            && matches!(
                table.enum_repr(table.peeled(state.ty)),
                Some(
                    rcgjs_object::jir::types::EnumRepr::IntTag { fieldless: true }
                        | rcgjs_object::jir::types::EnumRepr::NullNiche { .. }
                        | rcgjs_object::jir::types::EnumRepr::SentinelNiche { .. }
                )
            )
        {
            state.current = JsPlace::Unit(jsast::undefined());
            state.field_slot = false;
        } else if matches!(key, FieldKey::Transparent) {
            state.carry = carried;
        } else if !table.get(field_ty).is_some_and(|d| d.layout.sized) {
            let (tail, raw_tail) = match carried {
                DerefCarry::UnsizedTail { meta, raw } => (Some(meta), raw),
                _ => (None, None),
            };
            let base = self.read_place(&state.current);
            state.current = self.tail_place(base, key, field_ty, tail, raw_tail)?;
            state.field_slot = false;
        } else if table.is_zst(field_ty) {
            // A zero-sized field inside a slot-pointed non-ZST container (such as
            // `ArcInner<Zst>::data` at byte offset 8) has no storage of its own, but taking
            // `&raw const (*ptr).data` must preserve the container's byte-offset address so
            // `Arc::from_raw` / `ptr::byte_sub` can recover the enclosing container pointer.
            let zst_addr = match &state.current {
                JsPlace::Slot { pointer, .. } => {
                    let parent_size = table
                        .get(table.peeled(state.ty))
                        .and_then(|d| d.layout.size)
                        .unwrap_or(0);
                    if parent_size > 0 {
                        let scaled = jsast::rt_call(
                            "scale",
                            vec![pointer.clone(), jsast::num(parent_size as f64)],
                        );
                        if field_offset == 0 {
                            Some(scaled)
                        } else {
                            let (setup, slot) = self.reread(scaled, &[BUF, OFF, ptr::SCALE]);
                            let num_branch = jsast::binary(
                                BinOp::UShr,
                                jsast::binary(
                                    BinOp::Add,
                                    slot.clone(),
                                    jsast::num(field_offset as f64),
                                ),
                                jsast::num(0),
                            );
                            let slot_branch = Expr::Object(vec![
                                (BUF.to_owned(), ptr::slot_buf(slot.clone())),
                                (
                                    OFF.to_owned(),
                                    jsast::binary(
                                        BinOp::Add,
                                        ptr::slot_off(slot.clone()),
                                        jsast::num(field_offset as f64),
                                    ),
                                ),
                                (
                                    ptr::SCALE.to_owned(),
                                    jsast::member(slot.clone(), ptr::SCALE),
                                ),
                            ]);
                            let shifted = jsast::cond(
                                jsast::binary(
                                    BinOp::StrictEq,
                                    jsast::type_of(slot),
                                    jsast::string("number"),
                                ),
                                num_branch,
                                slot_branch,
                            );
                            Some(super::ptr::sequenced(setup, shifted))
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(addr) = zst_addr {
                state.current = JsPlace::Unit(addr);
                state.field_slot = false;
            } else {
                let base = self.read_place(&state.current);
                state.current = JsPlace::Var(field_expr(self.interns, base, key));
                state.field_slot = true;
            }
        } else {
            let base = self.read_place(&state.current);
            state.current = JsPlace::Var(field_expr(self.interns, base, key));
            state.field_slot = true;
        }
        state.ty = field_ty;
        state.variant = None;
        Ok(())
    }

    fn step_index(
        &self,
        root: &Root,
        step: usize,
        index: rcgjs_object::jir::repr::ValueId,
        carried: DerefCarry,
        state: &mut WalkState,
    ) -> Res<()> {
        let element = self.element_ty(state.ty)?;
        if let Some(address) = self.unit_element(&state.current, state.ty, element) {
            // The index is not spelled, so `fuse` never makes it an inline value
            // ([`reaches_unit`]): it was evaluated where it was defined.
            state.current = JsPlace::Unit(address);
        } else {
            let index = self.expr_of(index)?;
            let current = std::mem::replace(&mut state.current, JsPlace::Unit(jsast::undefined()));
            state.current = match current {
                JsPlace::Array(pointer, _) => JsPlace::Var(self.unwindow_element(pointer, index)),
                other => {
                    let base = self.read_place(&other);
                    if let DerefCarry::RawByteSlice { signed } = carried {
                        let (setup, slot) = self.reread(base, &[BUF, OFF]);
                        JsPlace::ByteAt {
                            buf: super::ptr::sequenced(setup, ptr::slot_buf(slot.clone())),
                            off: offset(ptr::slot_off(slot), index),
                            signed,
                        }
                    } else if self.soa_slice_layout(state.ty).is_some() {
                        let (buf, base_off) = self.soa_unpack_slot_expr_for_place(base);
                        let total_off = if self.is_zero_slice_off(root, step, &base_off) {
                            index
                        } else {
                            offset(base_off, index)
                        };
                        JsPlace::SoaElem {
                            buf,
                            off: total_off,
                            elem_ty: element,
                        }
                    } else {
                        JsPlace::Var(self.element_expr(state.ty, base, index))
                    }
                }
            };
        }
        state.ty = element;
        state.variant = None;
        Ok(())
    }

    fn step_constant_index(
        &self,
        root: &Root,
        step: usize,
        idx_offset: u64,
        from_end: bool,
        carried: DerefCarry,
        state: &mut WalkState,
    ) -> Res<()> {
        let element = self.element_ty(state.ty)?;
        if let Some(address) = self.unit_element(&state.current, state.ty, element) {
            state.current = JsPlace::Unit(address);
        } else {
            let current = std::mem::replace(&mut state.current, JsPlace::Unit(jsast::undefined()));
            state.current = match current {
                JsPlace::Array(pointer, length) => {
                    let index = if from_end {
                        length.saturating_sub(idx_offset)
                    } else {
                        idx_offset
                    };
                    JsPlace::Var(self.unwindow_element(pointer, jsast::num(index as f64)))
                }
                other => {
                    let base = self.read_place(&other);
                    if self.soa_slice_layout(state.ty).is_some() {
                        let (buf, base_off, index) = if from_end {
                            let (setup, slot) = self.reread(base, &[BUF, OFF, ptr::LEN]);
                            let buf = super::ptr::sequenced(setup, ptr::slot_buf(slot.clone()));
                            let base_off = ptr::slot_off(slot.clone());
                            let index = jsast::binary(
                                BinOp::Sub,
                                ptr::slot_len(slot),
                                jsast::num(idx_offset as f64),
                            );
                            (buf, base_off, index)
                        } else {
                            let (buf, base_off) = self.soa_unpack_slot_expr_for_place(base);
                            (buf, base_off, jsast::num(idx_offset as f64))
                        };
                        let total_off = if self.is_zero_slice_off(root, step, &base_off) {
                            index
                        } else {
                            offset(base_off, index)
                        };
                        JsPlace::SoaElem {
                            buf,
                            off: total_off,
                            elem_ty: element,
                        }
                    } else {
                        let index =
                            self.constant_index(state.ty, base.clone(), idx_offset, from_end);
                        if let DerefCarry::RawByteSlice { signed } = carried {
                            let (setup, slot) = self.reread(base, &[BUF, OFF]);
                            JsPlace::ByteAt {
                                buf: super::ptr::sequenced(setup, ptr::slot_buf(slot.clone())),
                                off: offset(ptr::slot_off(slot), index),
                                signed,
                            }
                        } else {
                            JsPlace::Var(self.element_expr(state.ty, base, index))
                        }
                    }
                }
            };
        }
        state.ty = element;
        state.variant = None;
        Ok(())
    }

    fn step_subslice(&self, from: u64, to: u64, from_end: bool, state: &mut WalkState) -> Res<()> {
        // A run of a zero-sized array is a zero-sized array at the same address. A run of a slice
        // stays a fat slice whatever its element is, because its `len` is the element count.
        if matches!(state.current, JsPlace::Unit(_)) || self.table.is_zst(state.ty) {
            if !matches!(state.current, JsPlace::Unit(_)) {
                state.current = JsPlace::Unit(self.read_place(&state.current));
            }
            state.variant = None;
            return Ok(());
        }
        let current = std::mem::replace(&mut state.current, JsPlace::Unit(jsast::undefined()));
        let array_source = match current {
            JsPlace::Array(pointer, length) => Ok((pointer, length)),
            other => match value::kind(self.table, state.ty) {
                Some(TypeKind::Array { length, .. }) => Ok((self.read_place(&other), *length)),
                _ => Err(other),
            },
        };
        match array_source {
            Ok((source, length)) => {
                let end = if from_end {
                    length.saturating_sub(to)
                } else {
                    to
                };
                let sub_len = end.saturating_sub(from);
                let (setup, slot) =
                    self.reread(jsast::rt_call("unwindow", vec![source]), &[BUF, OFF]);
                let window = ptr::window_slot(
                    ptr::slot_buf(slot.clone()),
                    offset(ptr::slot_off(slot), jsast::num(from as f64)),
                    sub_len,
                );
                state.current = JsPlace::Array(super::ptr::sequenced(setup, window), sub_len);
                state.variant = None;
            }
            Err(other) => {
                let base = self.read_place(&other);
                let (value, subslice_ty) = self.subslice(state.ty, base, from, to, from_end)?;
                state.current = JsPlace::Deref(value);
                state.ty = subslice_ty;
                state.variant = None;
            }
        }
        Ok(())
    }

    fn step_downcast(variant: u32, carried: DerefCarry, state: &mut WalkState) {
        state.variant = Some(variant);
        state.carry = carried;
    }

    /// The place a struct's unsized tail is (`place.rs::tail_place`).
    fn tail_place(
        &self,
        record: Expr,
        key: FieldKey,
        tail_ty: TypeId,
        tail: Option<Expr>,
        raw_tail: Option<Expr>,
    ) -> Res<JsPlace> {
        let Some(tail) = tail else {
            return Err(decline(
                OpClass::Place,
                "the unsized tail of a struct projected out of a place that is not behind a { ptr, meta } pointer",
            ));
        };
        if !matches!(value::kind(self.table, tail_ty), Some(TypeKind::Slice(_))) {
            return Ok(JsPlace::Var(self.zombie(format!(
                "a struct with a `{}` tail is not supported by rustc_codegen_js; only a slice \
                 tail is, because only a run of elements can be laid out after the header",
                self.type_display(tail_ty),
            ))));
        }
        let key_name = value::key_name(self.interns, key).expect("tail field key");
        let heap = self.heap();
        if heap.typed_heap_enabled && !heap.raw_alloc_reachable {
            if let Some(raw_ptr) = raw_tail {
                let (setup, rp) = self.reread(raw_ptr, &[ptr::PTR, ptr::META]);
                let slot = jsast::member(rp.clone(), ptr::PTR);
                let rec = jsast::index(ptr::slot_buf(slot.clone()), ptr::slot_off(slot));
                let slice = ptr::fat_slice(
                    value::field_expr(self.interns, rec, key),
                    jsast::num(0),
                    jsast::member(rp, ptr::META),
                );
                return Ok(JsPlace::Deref(super::ptr::sequenced(setup, slice)));
            }
            return Ok(JsPlace::Deref(ptr::fat_slice(
                value::field_expr(self.interns, record, key),
                jsast::num(0),
                tail,
            )));
        }
        let key_str = jsast::string(key_name);
        if let Some(raw_ptr) = raw_tail {
            return Ok(JsPlace::Deref(jsast::rt_call(
                "tail_slice_raw",
                vec![raw_ptr, key_str],
            )));
        }
        Ok(JsPlace::Deref(jsast::rt_call(
            "tail_slice",
            vec![record, key_str, tail],
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
                byte_access: SlotByteAccess::Unsigned,
                ..
            } => jsast::rt_call("read_byte", vec![pointer.clone()]),
            JsPlace::Slot {
                pointer,
                byte_access: SlotByteAccess::Signed,
                ..
            } => jsast::rt_call("read_byte", vec![pointer.clone(), Expr::Bool(true)]),
            JsPlace::Slot {
                pointer,
                byte_access: SlotByteAccess::None,
                ..
            } => self.slot_element(pointer.clone()),
            JsPlace::ByteAt { buf, off, signed } => {
                let mut args = vec![buf.clone(), off.clone()];
                if *signed {
                    args.push(Expr::Bool(true));
                }
                jsast::rt_call("read_byte_at", args)
            }
            JsPlace::SoaElem { buf, off, elem_ty } => {
                if let Some(layout) = self.soa_layout(*elem_ty) {
                    self.soa_read_elem(layout, buf.clone(), off.clone())
                } else {
                    jsast::undefined()
                }
            }
            JsPlace::SoaBool { buf, col, off } => super::soa::decode_bool(jsast::index(
                jsast::member(buf.clone(), col.clone()),
                off.clone(),
            )),
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
            JsPlace::Var(Expr::Member(obj, key)) if key == "__proto__" => {
                Ok(vec![jsast::expr_stmt(value::define_proto_prop(
                    *obj, value,
                ))])
            }
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
                byte_access: SlotByteAccess::Unsigned | SlotByteAccess::Signed,
                ..
            } => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_byte",
                vec![pointer, value],
            ))]),
            JsPlace::Slot {
                pointer,
                indirect,
                byte_access: SlotByteAccess::None,
            } => {
                let mut target = self.slot_element(pointer);
                let mut stmts = Vec::new();
                loop {
                    match target {
                        Expr::Seq(mut items) => {
                            if let Some(tail) = items.pop() {
                                for setup in items {
                                    stmts.push(jsast::expr_stmt(setup));
                                }
                                target = tail;
                            } else {
                                target = Expr::Seq(items);
                                break;
                            }
                        }
                        other => {
                            target = other;
                            break;
                        }
                    }
                }
                if indirect {
                    stmts.extend(self.write_indirect(ty, target, value)?);
                    Ok(stmts)
                } else if let Expr::Member(obj, key) = target {
                    if key == "__proto__" {
                        stmts.push(jsast::expr_stmt(value::define_proto_prop(*obj, value)));
                        Ok(stmts)
                    } else {
                        let target = Expr::Member(obj, key);
                        if !is_lvalue(&target) {
                            return Err(decline(
                                OpClass::Place,
                                "a write to a place whose expression is not assignable (an inlined value used as storage)",
                            ));
                        }
                        stmts.push(jsast::assign_stmt(target, value));
                        Ok(stmts)
                    }
                } else {
                    if !is_lvalue(&target) {
                        return Err(decline(
                            OpClass::Place,
                            "a write to a place whose expression is not assignable (an inlined value used as storage)",
                        ));
                    }
                    stmts.push(jsast::assign_stmt(target, value));
                    Ok(stmts)
                }
            }
            JsPlace::ByteAt { buf, off, .. } => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_byte_at",
                vec![buf, off, value],
            ))]),
            JsPlace::SoaElem { buf, off, elem_ty } => {
                let Some(layout) = self.soa_layout(elem_ty) else {
                    return Err(decline(OpClass::Place, "SoA layout missing on write"));
                };
                self.soa_write_elem(layout, buf, off, value)
            }
            JsPlace::SoaBool { buf, col, off } => Ok(vec![jsast::assign_stmt(
                jsast::index(jsast::member(buf, col), off),
                super::soa::encode_bool(value),
            )]),
            JsPlace::Array(pointer, _) => {
                let mut args = vec![pointer, value];
                if let Some(TypeKind::Array { element, .. }) = value::kind(self.table, ty) {
                    if self.table.is_indirect(*element) {
                        args.push(Expr::Bool(true));
                        let heap = self.heap();
                        if heap.typed_heap_enabled
                            && !heap.non_rc_raw_alloc_reachable
                            && !heap.needs_box_tracking(self.table, *element)
                        {
                            args.push(Expr::Bool(true));
                        }
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
        if !mutable {
            if let Root::Cell(cell) = root {
                if path.is_empty() && self.map_key_unboxing().0.contains(cell) {
                    return self.cell_expr(*cell);
                }
            } else if let Root::Value(v) = root {
                if path == [Proj::Deref] && self.unboxed_map_key_cell(*v).is_some() {
                    return self.expr_of(*v);
                }
            }
        }
        let walk = self.walk_place(root, path)?;
        if value::is_str(self.table, walk.ty) {
            let value = walk
                .identity
                .unwrap_or_else(|| self.read_place(&walk.place));
            return Ok(match (walk.str_kind, mutable) {
                (StrPlaceKind::FatMut, true) => {
                    if self.needs_byte_slice_ref() {
                        jsast::rt_call("byte_slice_ref", vec![value])
                    } else {
                        value
                    }
                }
                (StrPlaceKind::FatConst, true) => {
                    let live = jsast::rt_call("detach_str_memo", vec![value]);
                    if self.needs_byte_slice_ref() {
                        jsast::rt_call("byte_slice_ref", vec![live])
                    } else {
                        live
                    }
                }
                (StrPlaceKind::FatConst | StrPlaceKind::FatMut, false) => self.bytes_to_str(value),
                (StrPlaceKind::None, false) => value,
                (StrPlaceKind::None, true) => self.str_to_bytes_mut(value),
            });
        }
        if let Some(identity) = walk.identity {
            if self.needs_byte_slice_ref() {
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
        if let JsPlace::SoaElem { buf, off, .. } = walk.place {
            return Ok(ptr::slot(buf, off));
        }
        if matches!(walk.place, JsPlace::SoaBool { .. }) {
            return Err(decline(OpClass::Place, "escaping borrow of SoA bool field"));
        }
        if self.table.is_indirect(walk.ty) {
            let value = self.read_place(&walk.place);
            if self.needs_byte_slice_ref() {
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
            return Ok(match (walk.str_kind, mutable) {
                (StrPlaceKind::FatConst, true) => jsast::rt_call("detach_str_memo", vec![value]),
                (StrPlaceKind::FatConst | StrPlaceKind::FatMut, _) => value,
                (StrPlaceKind::None, true) => self.str_to_bytes_mut(value),
                (StrPlaceKind::None, false) => self.str_to_bytes(value),
            });
        }
        if let Some(identity) = walk.identity {
            return Ok(self.ref_to_raw(walk.ty, identity));
        }
        if let JsPlace::Unit(address) = walk.place {
            return Ok(address);
        }
        if let JsPlace::SoaElem { buf, off, .. } = walk.place {
            return Ok(ptr::slot(buf, off));
        }
        if matches!(walk.place, JsPlace::SoaBool { .. }) {
            return Err(decline(
                OpClass::Place,
                "escaping raw pointer to SoA bool field",
            ));
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
        if walk.field_slot && self.table.is_indirect(walk.ty) {
            match walk.place {
                JsPlace::Var(Expr::Member(base, name)) => {
                    return Ok(ptr::owned_field_slot(*base, jsast::string(name)));
                }
                JsPlace::Var(Expr::Index(base, key)) => {
                    return Ok(ptr::owned_field_slot(*base, *key));
                }
                place => return self.slot_of(place, walk.ty),
            }
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
            JsPlace::SoaElem { buf, off, .. } => Ok(ptr::slot(buf, off)),
            JsPlace::SoaBool { .. } => Err(decline(
                OpClass::Place,
                "escaping slot pointer to SoA bool field",
            )),
            // No storage, so no slot either: the pointer is the address the run shares.
            JsPlace::Unit(address) => Ok(address),
            JsPlace::Deref(value) if self.table.is_zst(ty) => Ok(value),
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

    /// Whether `base_off` for an SoA slice indexing step is provably always `0`, allowing
    /// `0 + index` to fold directly to `index`.
    fn is_zero_slice_off(&self, root: &Root, step: usize, base_off: &Expr) -> bool {
        if super::ast_util::is_zero_num(base_off) {
            return true;
        }
        if step != 1 {
            return false;
        }
        let Root::Value(slice_v) = root else {
            return false;
        };
        if !self.plan.physical_slices.contains(slice_v) {
            return false;
        }
        let Some(def) = self.def_of(*slice_v) else {
            return false;
        };
        let is_vec_as_slice = matches!(
            &def.kind,
            rcgjs_object::jir::repr::OpKind::Call {
                semantic: rcgjs_object::jir::semantic::CallSemantic::Standard(
                    rcgjs_object::jir::semantic::StandardOperation::VecAsSlice
                        | rcgjs_object::jir::semantic::StandardOperation::NativeVecAsSlice
                        | rcgjs_object::jir::semantic::StandardOperation::NativeVecAsMutSlice,
                ),
                host: None,
                ..
            }
        );
        if !is_vec_as_slice {
            return false;
        }
        let Some(slice_name) = self.plan.names.get(slice_v) else {
            return false;
        };
        // Ensure no other SSA value was coalesced onto `slice_name` and no edge copy overwrites it.
        if self
            .plan
            .names
            .iter()
            .any(|(v, n)| v != slice_v && n == slice_name)
        {
            return false;
        }
        for copies in self.plan.copies.values() {
            for copy in copies {
                if let crate::lower::plan::CopyOperand::Value(dst) = copy.dst {
                    if dst == *slice_v || self.plan.names.get(&dst).is_some_and(|n| n == slice_name)
                    {
                        return false;
                    }
                }
            }
        }
        true
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
pub(super) fn is_object(table: &TypeTable, ty: TypeId) -> bool {
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
