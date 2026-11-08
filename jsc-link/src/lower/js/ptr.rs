//! The pointer ABI over the type table: reading and writing through a pointer, offsets,
//! comparisons, addresses, the pointer cast matrix, and the `str` hybrid (v1 `backend/src/ptr.rs`;
//! the record shape itself is `rcgjs_object::ptr`).

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::types::{FieldKey, TypeId, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt};
use rcgjs_object::ptr::{self, BUF, LEN, META, OFF, PTR};

use super::value::{self, is_slot_pointee, size_of, CLONE_PARAM};
use super::{decline, JsCx, Res};

/// The depth at which a cast stops peeling and reports (`ptr.rs::CAST_DEPTH`).
const CAST_DEPTH: u32 = 16;

// The read/write/offset/copy helpers below are the seams W8's intrinsics (`read_via_copy`,
// `offset`, `arith_offset`, `ptr_offset_from`, `copy_nonoverlapping`) lower through.
#[allow(dead_code)]
impl<'a> JsCx<'a> {
    /// The value behind `pointer`, a **raw** pointer to `pointee` (`ptr.rs::deref_read`).
    pub fn deref_read(&self, pointee: TypeId, pointer: Expr) -> Expr {
        let table = self.table;
        if table.is_zst(pointee) {
            return jsast::undefined();
        }
        if let Some(TypeKind::Array { length, .. }) = value::kind(table, pointee) {
            if is_slot_pointee(table, pointee) {
                return jsast::rt_call("read_array", vec![pointer, jsast::num(*length as f64)]);
            }
        }
        if is_slot_pointee(table, pointee) {
            ptr::slot_element(pointer)
        } else {
            pointer
        }
    }

    /// The statements that store `value` through `pointer`, a **raw** pointer to `pointee`
    /// (`ptr.rs::deref_write`).
    pub fn deref_write(&self, pointee: TypeId, pointer: Expr, value: Expr) -> Res<Vec<Stmt>> {
        let table = self.table;
        if table.is_zst(pointee) {
            return Ok(vec![jsast::expr_stmt(value)]);
        }
        if !is_slot_pointee(table, pointee) {
            return self.write_indirect(pointee, pointer, value);
        }
        if matches!(value::kind(table, pointee), Some(TypeKind::Array { .. })) {
            return Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_array",
                vec![pointer, value],
            ))]);
        }
        let element = ptr::slot_element(pointer);
        if table.is_indirect(pointee) {
            return self.write_indirect(pointee, element, value);
        }
        Ok(vec![jsast::assign_stmt(element, value)])
    }

    /// The statements that store `value` into `target`, the pointee's own JS value
    /// (`ptr.rs::write_indirect`).
    pub fn write_indirect(&self, pointee: TypeId, target: Expr, value: Expr) -> Res<Vec<Stmt>> {
        let table = self.table;
        if table.is_zst(pointee) {
            return Ok(vec![jsast::expr_stmt(value)]);
        }
        if value::is_str(table, pointee) {
            return Ok(vec![jsast::expr_stmt(self.zombie(
                "mutating a `&mut str` is not supported by rustc_codegen_js".to_owned(),
            ))]);
        }
        let call = match target {
            Expr::Index(base, key) => jsast::rt_call("_put", vec![*base, *key, value]),
            Expr::Member(base, name) => {
                jsast::rt_call("_put", vec![*base, jsast::string(name), value])
            }
            target => jsast::rt_call("overwrite", vec![target, value]),
        };
        Ok(vec![jsast::expr_stmt(call)])
    }

    /// `pointer.add(count)` (`ptr.rs::add`).
    pub fn ptr_add(&self, pointee: TypeId, pointer: Expr, count: Expr) -> Expr {
        if count == jsast::num(0) {
            return pointer;
        }
        let size = size_of(self.table, pointee);
        if size == Some(0) {
            return pointer;
        }
        let off = jsast::binary(BinOp::Add, ptr::slot_off(pointer.clone()), count);
        match size {
            Some(1) => jsast::object_of(vec![
                (BUF, ptr::slot_buf(pointer.clone())),
                (OFF, off),
                (ptr::SCALE, ptr::slot_scale(pointer)),
            ]),
            _ => ptr::slot(ptr::slot_buf(pointer), off),
        }
    }

    /// `pointer.wrapping_add(count)` (`ptr.rs::wrapping_add`).
    pub fn ptr_wrapping_add(&self, pointee: TypeId, pointer: Expr, count: Expr) -> Expr {
        if count == jsast::num(0) {
            return pointer;
        }
        let size = size_of(self.table, pointee).unwrap_or(1);
        jsast::rt_call("offset", vec![pointer, count, jsast::num(size as f64)])
    }

    /// `a.offset_from(b)` (`ptr.rs::offset_from`).
    pub fn ptr_offset_from(&self, pointee: TypeId, lhs: Expr, rhs: Expr) -> Expr {
        if size_of(self.table, pointee) == Some(0) {
            return jsast::binary(BinOp::Sub, lhs, rhs);
        }
        if self.table.is_indirect(pointee) {
            return jsast::rt_call("offset_from", vec![lhs, rhs]);
        }
        jsast::binary(BinOp::Sub, ptr::slot_off(lhs), ptr::slot_off(rhs))
    }

    /// `lhs == rhs` on two pointers (`ptr.rs::eq`).
    pub fn ptr_eq(&self, pointer_ty: TypeId, lhs: Expr, rhs: Expr) -> Expr {
        let helper = match value::pointee_of(self.table, pointer_ty) {
            Some(pointee)
                if matches!(value::kind(self.table, pointee), Some(TypeKind::Slice(_))) =>
            {
                "slice_ptr_eq"
            }
            Some(pointee)
                if matches!(
                    value::kind(self.table, pointee),
                    Some(TypeKind::Dynamic { .. })
                ) || self.table.unsized_tail(pointee).is_some() =>
            {
                "fat_ptr_eq"
            }
            _ => "ptr_eq",
        };
        jsast::rt_call(helper, vec![lhs, rhs])
    }

    /// `lhs < rhs` and its siblings on two pointers (`ptr.rs::cmp`).
    pub fn ptr_cmp(&self, pointer_ty: TypeId, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let size = value::pointee_of(self.table, pointer_ty)
            .and_then(|pointee| size_of(self.table, pointee))
            .unwrap_or(1);
        jsast::binary(
            op,
            jsast::rt_call("ptr_cmp", vec![lhs, rhs, jsast::num(size as f64)]),
            jsast::num(0),
        )
    }

    /// The address of `pointer` (`ptr.rs::addr`).
    pub fn ptr_addr(&self, pointee: TypeId, pointer: Expr) -> Expr {
        let size = size_of(self.table, pointee).unwrap_or(1).max(1);
        jsast::rt_call("addr", vec![pointer, jsast::num(size as f64)])
    }

    /// `ptr::copy`/`copy_nonoverlapping` as a statement (`ptr.rs::copy`).
    pub fn ptr_copy(
        &self,
        pointee: TypeId,
        dst: Expr,
        src: Expr,
        count: Expr,
        overlapping: bool,
    ) -> Vec<Stmt> {
        if self.table.is_zst(pointee) {
            return Vec::new();
        }
        let name = if overlapping {
            "copy"
        } else {
            "copy_nonoverlapping"
        };
        let mut args = vec![dst, src, count];
        if self.table.needs_clone(pointee) {
            args.push(jsast::arrow(
                vec![CLONE_PARAM.to_string()],
                value::clone_expr(self.table, self.interns, pointee, jsast::id(CLONE_PARAM)),
            ));
        }
        vec![jsast::expr_stmt(jsast::rt_call(name, args))]
    }

    /// `&value as *const T` (`ptr.rs::ref_to_raw`).
    pub fn ref_to_raw(&self, pointee: TypeId, reference: Expr) -> Expr {
        if self.table.unsized_tail(pointee).is_some() {
            return jsast::rt_call("tail_raw", vec![reference]);
        }
        if !is_slot_pointee(self.table, pointee) || !self.table.is_indirect(pointee) {
            return reference;
        }
        match reference {
            Expr::Index(base, key) => ptr::slot(*base, *key),
            Expr::Member(base, name) => ptr::slot(*base, jsast::string(name)),
            reference => jsast::rt_call("box", vec![reference]),
        }
    }

    /// `&*raw` (`ptr.rs::raw_to_ref`).
    pub fn raw_to_ref(&self, pointee: TypeId, raw: Expr) -> Expr {
        if self.table.unsized_tail(pointee).is_some() {
            return jsast::rt_call("tail_ref", vec![raw]);
        }
        if !is_slot_pointee(self.table, pointee) {
            return raw;
        }
        if self.table.is_indirect(pointee) {
            ptr::slot_element(raw)
        } else {
            raw
        }
    }

    /// The zero element factory `__rt.unscale`/`retype` take (`alloc_support.rs::zero_factory`).
    fn zero_factory(&self, ty: TypeId) -> Expr {
        match value::zero_value(self.table, self.interns, ty) {
            Some(value) => jsast::arrow(Vec::new(), value),
            None => jsast::null(),
        }
    }

    /// The header factory `__rt.retype_rc` takes (`alloc_support.rs::header_factory`).
    fn header_factory(&self, ty: TypeId) -> Expr {
        let table = self.table;
        let Some(tail) = table.unsized_tail(ty) else {
            return jsast::null();
        };
        let peeled = table.peeled(ty);
        let Some(TypeKind::Adt { variants, .. }) = table.get(peeled).map(|d| &d.kind) else {
            return jsast::null();
        };
        let Some(variant) = variants.first() else {
            return jsast::null();
        };
        let mut entries = Vec::new();
        for (index, field) in variant.fields.iter().enumerate() {
            if index as u32 == tail.field || table.is_zst(field.ty) {
                continue;
            }
            let (Some(zero), Some(key)) = (
                value::zero_value(table, self.interns, field.ty),
                value::key_name(self.interns, field.key),
            ) else {
                return jsast::null();
            };
            entries.push((key, zero));
        }
        jsast::arrow(Vec::new(), jsast::object(entries))
    }

    /// A fat pointer built from its two halves (`ptr.rs::build_fat`).
    pub fn build_fat(&self, pointee: TypeId, data: Expr, meta: Expr) -> Res<Expr> {
        let table = self.table;
        match value::kind(table, pointee) {
            Some(TypeKind::Dynamic { .. }) => Ok(ptr::fat_dyn(data, meta)),
            Some(TypeKind::Slice(element)) => match value::kind(table, *element) {
                // Zero-sized elements are stored nowhere, so the record keeps the data pointer
                // itself as its `buf`: what `as_ptr` and every element pointer answer, exactly as
                // a `SliceView` over a thin pointer spells it. Nothing is normalized or asked.
                _ if table.is_zst(*element) => Ok(ptr::fat_slice(data, jsast::num(0), meta)),
                Some(TypeKind::Array { length, .. }) => Ok(jsast::rt_call(
                    "chunk_slice",
                    vec![data, meta, jsast::num(*length as f64)],
                )),
                _ => Ok(jsast::rt_call("slice_from_parts", vec![data, meta])),
            },
            Some(TypeKind::Str) => {
                let (setup, data) = self.reread(data, &[BUF, OFF]);
                let decoded = jsast::rt_call(
                    "bytes_str",
                    vec![ptr::slot_buf(data.clone()), ptr::slot_off(data), meta],
                );
                Ok(sequenced(setup, decoded))
            }
            _ if table.get(pointee).is_some_and(|d| d.layout.sized) => {
                let size = size_of(table, pointee).unwrap_or(0);
                Ok(jsast::rt_call("thin", vec![data, jsast::num(size as f64)]))
            }
            _ => {
                let element = table.unsized_tail(pointee).and_then(|tail| {
                    match value::kind(table, tail.ty) {
                        Some(TypeKind::Slice(element)) => size_of(table, *element),
                        _ => None,
                    }
                });
                match element {
                    Some(element_size) => Ok(jsast::rt_call(
                        "retype_rc",
                        vec![
                            data,
                            self.header_factory(pointee),
                            jsast::num(element_size as f64),
                            meta,
                        ],
                    )),
                    None => Err(decline(
                        OpClass::Rvalue,
                        "a fat pointer to a struct without a slice tail cannot be rebuilt from a thin data pointer",
                    )),
                }
            }
        }
    }

    /// `s.as_bytes()` (`ptr.rs::str_to_bytes`).
    pub fn str_to_bytes(&self, string: Expr) -> Expr {
        jsast::rt_call("str_bytes", vec![string])
    }

    /// `s.as_ptr()` (`ptr.rs::str_to_byte_ptr`).
    pub fn str_to_byte_ptr(&self, string: Expr) -> Expr {
        ptr::slot(ptr::slot_buf(self.str_to_bytes(string)), jsast::num(0))
    }

    /// `str::from_utf8_unchecked(v)` (`ptr.rs::bytes_to_str`).
    pub fn bytes_to_str(&self, bytes: Expr) -> Expr {
        let (setup, bytes) = self.reread(bytes, &[BUF, OFF, LEN]);
        let decoded = jsast::rt_call(
            "bytes_str",
            vec![
                ptr::slot_buf(bytes.clone()),
                ptr::slot_off(bytes.clone()),
                ptr::slot_len(bytes),
            ],
        );
        sequenced(setup, decoded)
    }

    /// `__rt.str_len(s)` (`ptr.rs::str_len`).
    pub fn str_len(&self, string: Expr) -> Expr {
        jsast::rt_call("str_len", vec![string])
    }

    /// The `str` arms of the pointer cast matrix over two pointee types (`ptr.rs::cast_str`);
    /// `None` when neither side is a `str`.
    pub fn cast_str(
        &self,
        from_pointee: TypeId,
        to_pointee: TypeId,
        value: Expr,
    ) -> Option<Res<Expr>> {
        let table = self.table;
        let (from_str, to_str) = (
            value::is_str(table, from_pointee),
            value::is_str(table, to_pointee),
        );
        if !from_str && !to_str {
            return None;
        }
        let unsupported = || {
            Err(decline(
                OpClass::Cast,
                "a pointer cast with a `str` on one side that is not the UTF-8 byte view",
            ))
        };
        Some(match (from_str, to_str) {
            (true, true) => Ok(value),
            (true, false) => match value::kind(table, to_pointee) {
                Some(TypeKind::Slice(element)) if value::is_u8(table, *element) => {
                    Ok(self.str_to_bytes(value))
                }
                _ if value::is_u8(table, to_pointee) => Ok(self.str_to_byte_ptr(value)),
                _ => unsupported(),
            },
            (false, true) => match value::kind(table, from_pointee) {
                Some(TypeKind::Slice(element)) if table.byte_element(*element) => {
                    Ok(self.bytes_to_str(value))
                }
                _ => unsupported(),
            },
            (false, false) => Ok(value),
        })
    }

    /// [`Self::cast_str`] over two **pointer** types (`ptr.rs::cast_str_ptr`).
    pub fn cast_str_ptr(&self, from_ty: TypeId, to_ty: TypeId, value: Expr) -> Option<Res<Expr>> {
        let from_pointee = value::pointee_of(self.table, from_ty)?;
        let to_pointee = value::pointee_of(self.table, to_ty)?;
        self.cast_str(from_pointee, to_pointee, value)
    }

    /// `value as *const to_pointee` (`ptr.rs::cast_pointer`).
    pub fn cast_pointer(&self, from_pointee: TypeId, to_pointee: TypeId, value: Expr) -> Res<Expr> {
        if let Some(converted) = self.cast_str(from_pointee, to_pointee, value.clone()) {
            return converted;
        }
        self.cast_pointer_at(from_pointee, to_pointee, value, 0)
    }

    fn cast_pointer_at(
        &self,
        from_pointee: TypeId,
        to_pointee: TypeId,
        value: Expr,
        depth: u32,
    ) -> Res<Expr> {
        let table = self.table;
        let unsupported = || {
            Err(decline(
                OpClass::Cast,
                format!(
                    "a pointer cast from pointee {:032x} to pointee {:032x} has no row in the cast matrix",
                    from_pointee.0, to_pointee.0
                ),
            ))
        };
        if depth >= CAST_DEPTH {
            return unsupported();
        }
        let from = table.peeled(from_pointee);
        let to = table.peeled(to_pointee);
        if from == to {
            return Ok(value);
        }
        if table.is_zst(to) {
            // A slice of zero-sized elements holds them nowhere: its `buf` is the data pointer it
            // was built from, which is the one address every element has (`place.rs`,
            // `JsPlace::Unit`). `as_ptr`, `cast::<()>` and `addr_eq` all answer it.
            if let Some(TypeKind::Slice(element)) = value::kind(table, from) {
                if table.is_zst(*element) {
                    return Ok(ptr::slot_buf(value));
                }
            }
            let reference = self.raw_to_ref(from, value);
            let reference = if table.unsized_tail(from).is_some()
                || matches!(value::kind(table, from), Some(TypeKind::Dynamic { .. }))
            {
                jsast::member(reference, PTR)
            } else {
                reference
            };
            return Ok(match size_of(table, from) {
                Some(size) if size > 1 && !table.is_indirect(from) => {
                    jsast::rt_call("erase", vec![reference, jsast::num(size as f64)])
                }
                _ => reference,
            });
        }
        if table.is_zst(from) {
            // Erasure can carry a scaled byte view through `*const ()`. Restore its units
            // just as a sized `aggregate_raw_ptr` does, rather than treating it as a reference.
            return Ok(match size_of(table, to) {
                Some(size) => jsast::rt_call("thin", vec![value, jsast::num(size as f64)]),
                None => self.ref_to_raw(to, value),
            });
        }
        self.cast_pointer_sized(from_pointee, to_pointee, from, to, value, depth)
    }

    #[allow(clippy::too_many_arguments)]
    fn cast_pointer_sized(
        &self,
        from_pointee: TypeId,
        to_pointee: TypeId,
        from: TypeId,
        to: TypeId,
        value: Expr,
        depth: u32,
    ) -> Res<Expr> {
        let table = self.table;
        let unsupported = || {
            Err(decline(
                OpClass::Cast,
                format!(
                    "a pointer cast from pointee {:032x} to pointee {:032x} has no row in the cast matrix",
                    from_pointee.0, to_pointee.0
                ),
            ))
        };
        if table.unsized_tail(to).is_some() {
            return self.reshape_to_tail(from, to, value);
        }
        if table.unsized_tail(from).is_some() {
            return Ok(jsast::member(value, PTR));
        }
        match (value::kind(table, from), value::kind(table, to)) {
            (Some(TypeKind::Slice(from_element)), Some(TypeKind::Slice(to_element))) => {
                return if size_of(table, *from_element) == size_of(table, *to_element) {
                    Ok(value)
                } else {
                    unsupported()
                };
            }
            (Some(TypeKind::Slice(element)), _) => {
                return self.cast_pointer_at(*element, to, value, depth + 1);
            }
            (Some(TypeKind::Dynamic { .. }), _) => {
                return Ok(jsast::rt_call("unref", vec![jsast::member(value, PTR)]));
            }
            (_, Some(TypeKind::Slice(_) | TypeKind::Dynamic { .. })) => return unsupported(),
            _ => {}
        }
        let (Some(from_size), Some(to_size)) = (size_of(table, from), size_of(table, to)) else {
            return unsupported();
        };
        if let Some(TypeKind::Array { element, length }) = value::kind(table, to) {
            let to_element = *element;
            let element = table.peeled(to_element);
            let width = *length;
            let element_size = size_of(table, element);
            let keyed = if element == from {
                Some(value.clone())
            } else if from_size == 1 && !table.is_indirect(from) && element_size.is_some() {
                let mut arguments = vec![
                    value.clone(),
                    jsast::num(element_size.unwrap_or(1) as f64),
                    self.zero_factory(element),
                ];
                let member = if table.crosses_transparent_union(from_pointee) {
                    "unscale_uninit"
                } else if let Some(kind) = self.scalar_view_kind(element) {
                    arguments.push(kind);
                    "unscale_scalar"
                } else {
                    "unscale"
                };
                Some(jsast::rt_call(member, arguments))
            } else {
                None
            };
            if let Some(keyed) = keyed {
                let (setup, pointer) = self.reread(keyed, &[BUF, OFF]);
                let window = ptr::window_slot(
                    ptr::slot_buf(pointer.clone()),
                    ptr::slot_off(pointer),
                    width,
                );
                return Ok(sequenced(setup, window));
            }
        }
        if let Some(TypeKind::Array { element, .. }) = value::kind(table, from) {
            let element = *element;
            let unwindowed = jsast::rt_call("unwindow", vec![value]);
            return self.cast_pointer_at(element, to, unwindowed, depth + 1);
        }
        if from_size == 1 && to_size == 1 && !table.is_indirect(from) && !table.is_indirect(to) {
            if let Some(zero) = self.non_byte_zero_factory(to_pointee) {
                return Ok(jsast::rt_call("retype", vec![value, jsast::num(1), zero]));
            }
        }
        if from_size == to_size && table.is_indirect(from) == table.is_indirect(to) {
            return Ok(value);
        }
        if let Some(reprojected) = self.reproject_to_field(from, to, &value, depth) {
            return Ok(reprojected);
        }
        if let Some(key) = self.zero_offset_field(from, to) {
            return Ok(jsast::rt_call(
                "prefix",
                vec![
                    value,
                    self.field_key_expr(key),
                    jsast::num(from_size as f64),
                ],
            ));
        }
        if let Some(key) = self.zero_offset_field(to, from) {
            return Ok(jsast::rt_call(
                "unproject",
                vec![value, self.field_key_expr(key)],
            ));
        }
        if to_size == 1 && !table.is_indirect(to) {
            return Ok(jsast::rt_call(
                "scale",
                vec![value, jsast::num(from_size as f64)],
            ));
        }
        if from_size == 1 && !table.is_indirect(from) {
            let mut arguments = vec![
                value,
                jsast::num(to_size as f64),
                self.zero_factory(to_pointee),
            ];
            let member = if table.crosses_transparent_union(from_pointee) {
                "unscale_uninit"
            } else if let Some(kind) = self.scalar_view_kind(to_pointee) {
                arguments.push(kind);
                "unscale_scalar"
            } else {
                "unscale"
            };
            return Ok(jsast::rt_call(member, arguments));
        }
        unsupported()
    }

    /// A zero factory for a type whose zero is not the byte zero
    /// (`alloc_support.rs::non_byte_zero_factory`).
    fn non_byte_zero_factory(&self, ty: TypeId) -> Option<Expr> {
        let value = value::zero_value(self.table, self.interns, ty)?;
        (jsast::expr_to_string(&value) != "0").then(|| jsast::arrow(Vec::new(), value))
    }

    /// The exact scalar representation an aliasing byte-storage view may decode.
    ///
    /// A missing kind keeps aggregates, pointers, enums, and other semantic values on the precise
    /// refusal path in `__rt.unscale`; their size alone does not describe a JavaScript value.
    fn scalar_view_kind(&self, ty: TypeId) -> Option<Expr> {
        let kind = match value::kind(self.table, ty) {
            Some(TypeKind::Integer { signed, bits }) if *bits > 8 => {
                Some(format!("{}{bits}", if *signed { 'i' } else { 'u' }))
            }
            Some(TypeKind::Float { bits: 32 }) => Some("f32".to_owned()),
            Some(TypeKind::Float { bits: 64 }) => Some("f64".to_owned()),
            Some(TypeKind::Char) => Some("u32".to_owned()),
            _ => None,
        };
        kind.map(jsast::string)
    }

    /// The reshape of a heap block into a header plus an unsized tail (`ptr.rs::reshape_to_tail`).
    fn reshape_to_tail(&self, from: TypeId, to: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        let Some(tail) = table.unsized_tail(to) else {
            return Err(decline(OpClass::Cast, "reshape to a struct without a tail"));
        };
        let Some(TypeKind::Slice(element)) = value::kind(table, tail.ty) else {
            return Err(decline(
                OpClass::Cast,
                "a pointer to a struct with a non-slice unsized tail",
            ));
        };
        let Some(element_size) = size_of(table, *element) else {
            return Err(decline(
                OpClass::Cast,
                "the elements of a slice tail have no size",
            ));
        };
        if !matches!(value::kind(table, from), Some(TypeKind::Slice(_))) {
            return Err(decline(
                OpClass::Cast,
                "a cast to a struct with an unsized tail from a pointer that carries no length",
            ));
        }
        let (setup, pointer) = self.reread(value, &[BUF, OFF, LEN]);
        let reshaped = jsast::rt_call(
            "retype_rc",
            vec![
                pointer.clone(),
                self.header_factory(to),
                jsast::num(element_size as f64),
                ptr::slot_len(pointer),
            ],
        );
        Ok(sequenced(setup, reshaped))
    }

    /// The reprojection row of the cast matrix (`ptr.rs::reproject_to_field`).
    fn reproject_to_field(
        &self,
        from: TypeId,
        to: TypeId,
        value: &Expr,
        depth: u32,
    ) -> Option<Expr> {
        let table = self.table;
        let mut chain = Vec::new();
        let mut current = from;
        for _ in depth..CAST_DEPTH {
            let (field_ty, key) = self.sole_field(current)?;
            chain.push(key);
            current = table.peeled(field_ty);
            if current != to {
                continue;
            }
            let mut pointer = value.clone();
            for key in chain {
                pointer = self.project_field(pointer, key);
            }
            return Some(pointer);
        }
        None
    }

    /// The non-zero-sized zero-offset field of a larger `aggregate` whose peeled type is `target`.
    /// A same-size one-field newtype is not a prefix: its standalone allocation has no containing
    /// pointer for `unproject` to recover.
    fn zero_offset_field(&self, aggregate: TypeId, target: TypeId) -> Option<FieldKey> {
        let table = self.table;
        let aggregate = table.peeled(aggregate);
        let target = table.peeled(target);
        let descriptor = table.get(aggregate)?;
        if descriptor.layout.size? <= table.get(target)?.layout.size? {
            return None;
        }
        match descriptor.kind {
            TypeKind::Adt {
                form: rcgjs_object::jir::types::NominalForm::Struct,
                ..
            }
            | TypeKind::Tuple(_) => descriptor
                .js
                .fields
                .iter()
                .find(|field| {
                    field.offset == 0 && !table.is_zst(field.ty) && table.peeled(field.ty) == target
                })
                .map(|field| field.key),
            _ => None,
        }
    }

    fn project_field(&self, pointer: Expr, key: FieldKey) -> Expr {
        match key {
            FieldKey::Name(name) => ptr::slot(
                ptr::slot_element(pointer),
                jsast::string(self.interns.str(name).unwrap_or("")),
            ),
            FieldKey::Index(index) => {
                ptr::slot(ptr::slot_element(pointer), jsast::num(index as f64))
            }
            FieldKey::Transparent => pointer,
        }
    }

    fn field_key_expr(&self, key: FieldKey) -> Expr {
        match key {
            FieldKey::Name(name) => jsast::string(self.interns.str(name).unwrap_or("")),
            FieldKey::Index(index) => jsast::num(index as f64),
            FieldKey::Transparent => jsast::undefined(),
        }
    }

    /// The one non-zero-sized field of an aggregate with exactly one, with its key
    /// (`intrinsics.rs::sole_field`).
    pub fn sole_field(&self, ty: TypeId) -> Option<(TypeId, FieldKey)> {
        let table = self.table;
        let descriptor = table.get(table.peeled(ty))?;
        let fields: &[rcgjs_object::jir::types::FieldDescriptor] = match &descriptor.kind {
            TypeKind::Adt {
                form: rcgjs_object::jir::types::NominalForm::Struct,
                variants,
                ..
            } => &variants.first()?.fields,
            TypeKind::Tuple(_) | TypeKind::Closure { .. } => &descriptor.js.fields,
            _ => return None,
        };
        let mut live = fields.iter().filter(|field| !table.is_zst(field.ty));
        let field = live.next()?;
        if live.next().is_some() {
            return None;
        }
        Some((field.ty, field.key))
    }

    /// An expression the caller reads `reads` off once each, and the assignment that makes that
    /// safe (`ptr.rs::reread`).
    pub fn reread(&self, value: Expr, reads: &[&str]) -> (Option<Expr>, Expr) {
        if duplicable(&value, reads) {
            return (None, value);
        }
        let (_, name) = self.temp(value.clone());
        (Some(jsast::assign(name.clone(), value)), name)
    }
}

/// `value`, preceded by the assignment `reread` asked for.
pub fn sequenced(setup: Option<Expr>, value: Expr) -> Expr {
    match setup {
        Some(setup) => jsast::seq(vec![setup, value]),
        None => value,
    }
}

/// Whether reading `reads` off `value` several times costs no more than reading it once.
fn duplicable(value: &Expr, reads: &[&str]) -> bool {
    match value {
        Expr::Ident(_) => true,
        Expr::Member(object, _) => duplicable(object, reads),
        Expr::Index(object, index) => {
            duplicable(object, reads)
                && matches!(&**index, Expr::Num(_) | Expr::Str(_) | Expr::Ident(_))
        }
        Expr::Object(entries) => reads
            .iter()
            .all(|name| jsast::folds_property(entries, name)),
        _ => false,
    }
}

/// `p.meta`, the metadata half of a fat pointer.
pub fn meta(pointer: Expr) -> Expr {
    jsast::member(pointer, META)
}
