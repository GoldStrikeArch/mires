//! The pointer ABI over the type table: reading and writing through a pointer, offsets,
//! comparisons, addresses, the pointer cast matrix, and the `str` hybrid (v1 `backend/src/ptr.rs`;
//! the record shape itself is `rcgjs_object::ptr`).

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, NominalForm, RepresentationCandidate, TypeId, TypeKind, TypeTable,
};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt};
use rcgjs_object::ptr::{self, BUF, LEN, META, OFF, PTR};

use super::ast_util::{is_one_num, is_zero_num, rt_member};
use super::value::{self, is_slot_pointee, size_of, CLONE_PARAM};
use super::{decline, JsCx, Res};

/// The depth at which a cast stops peeling and reports (`ptr.rs::CAST_DEPTH`).
const CAST_DEPTH: u32 = 16;

/// Arguments to [`JsCx::cast_pointer_sized`].
struct PointerCast {
    from_pointee: TypeId,
    to_pointee: TypeId,
    from: TypeId,
    to: TypeId,
    value: Expr,
    depth: u32,
}

/// Representation and mutability of one side of a `str`-involving pointer cast.
///
/// Mutable pointers (`&mut T`, `*mut T`) are always fat slices for `str`, so an immutable-style
/// non-fat mutable representation is unrepresentable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StrPtrRepr {
    /// Immutable non-raw reference (`&str` as JS string, or `&[u8]` / `&u8`).
    RefConst,
    /// Immutable raw/fat pointer (`*const str` / `NonNull<str>` / `Box<str>`, or `*const [u8]`).
    FatConst,
    /// Mutable reference or raw pointer (`&mut str` / `*mut str`, or `&mut [u8]` / `*mut [u8]`).
    FatMut,
}

impl StrPtrRepr {
    fn from_ptr_ty(table: &TypeTable, ty: TypeId) -> Self {
        if value::is_mutable_ptr(table, ty) {
            Self::FatMut
        } else if value::is_raw(table, ty) {
            Self::FatConst
        } else {
            Self::RefConst
        }
    }

    const fn is_fat(self) -> bool {
        matches!(self, Self::FatConst | Self::FatMut)
    }

    const fn is_mut(self) -> bool {
        matches!(self, Self::FatMut)
    }
}

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
        if let Some(layout) = self.soa_layout(pointee) {
            return self.soa_deref_read(layout, pointer);
        }
        if let Some(TypeKind::Array { length, .. }) = value::kind(table, pointee) {
            if is_slot_pointee(table, pointee) {
                return jsast::rt_call("read_array", vec![pointer, jsast::num(*length as f64)]);
            }
        }
        if is_slot_pointee(table, pointee) {
            if self.byte_views.raw_byte_derefs {
                if let Some(TypeKind::Integer { signed, bits: 8 }) =
                    value::kind(table, table.peeled(pointee))
                {
                    let mut args = vec![pointer];
                    if *signed {
                        args.push(Expr::Bool(true));
                    }
                    return jsast::rt_call("read_byte", args);
                }
            }
            slot_element_read(self, pointer)
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
        if let Some(layout) = self.soa_layout(pointee) {
            return self.soa_deref_write(layout, pointer, value);
        }
        if !is_slot_pointee(table, pointee) {
            return self.write_indirect(pointee, pointer, value);
        }
        if let Some(TypeKind::Array { element, .. }) = value::kind(table, pointee) {
            let mut args = vec![pointer, value];
            if table.is_indirect(*element) {
                args.push(Expr::Bool(true));
                let heap = self.heap();
                if heap.typed_heap_enabled
                    && !heap.non_rc_raw_alloc_reachable
                    && !heap.needs_box_tracking(table, *element)
                {
                    args.push(Expr::Bool(true));
                }
            }
            return Ok(vec![jsast::expr_stmt(jsast::rt_call("write_array", args))]);
        }
        if self.byte_views.raw_byte_derefs
            && matches!(
                value::kind(table, table.peeled(pointee)),
                Some(TypeKind::Integer { bits: 8, .. })
            )
        {
            return Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "write_byte",
                vec![pointer, value],
            ))]);
        }
        if table.is_indirect(pointee) {
            match unwrap_rt_box(pointer) {
                Ok((prefix, inner)) => {
                    let mut stmts: Vec<Stmt> = prefix.into_iter().map(jsast::expr_stmt).collect();
                    stmts.extend(self.overwrite_indirect(pointee, inner, value)?);
                    return Ok(stmts);
                }
                Err(pointer) => {
                    let element = slot_element_unwrapped(self, pointer);
                    return self.write_indirect(pointee, element, value);
                }
            }
        }
        let mut element = slot_element_unwrapped(self, pointer);
        let mut stmts = Vec::new();
        loop {
            match element {
                Expr::Seq(mut items) => {
                    if let Some(tail) = items.pop() {
                        for setup in items {
                            stmts.push(jsast::expr_stmt(setup));
                        }
                        element = tail;
                    } else {
                        element = Expr::Seq(items);
                        break;
                    }
                }
                other => {
                    element = other;
                    break;
                }
            }
        }
        stmts.push(jsast::assign_stmt(element, value));
        Ok(stmts)
    }

    /// Overwrites an indirect object value in place without rebinding any parent property/slot.
    fn overwrite_indirect(&self, pointee: TypeId, target: Expr, value: Expr) -> Res<Vec<Stmt>> {
        let table = self.table;
        if table.is_zst(pointee) {
            return Ok(vec![jsast::expr_stmt(value)]);
        }
        if value::is_str(table, pointee) {
            return Ok(vec![jsast::expr_stmt(self.zombie(
                "mutating a `&mut str` is not supported by rustc_codegen_js".to_owned(),
            ))]);
        }
        if self.heap().typed_heap_enabled {
            match self.try_inline_overwrite(pointee, target, value) {
                Ok(stmts) => Ok(stmts),
                Err((target, value)) => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                    "overwrite",
                    vec![target, value],
                ))]),
            }
        } else {
            Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "overwrite",
                vec![target, value],
            ))])
        }
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
        let heap = self.heap();
        let direct_plain_store = heap.typed_heap_enabled
            && !heap.non_rc_raw_alloc_reachable
            && !heap.needs_box_tracking(table, pointee);
        let put_helper = if heap.typed_heap_enabled && !heap.non_rc_raw_alloc_reachable {
            "_put_tracked"
        } else {
            "_put"
        };
        let call = match target {
            Expr::Index(base, key) => {
                if direct_plain_store {
                    return Ok(vec![jsast::assign_stmt(Expr::Index(base, key), value)]);
                }
                jsast::rt_call(put_helper, vec![*base, *key, value])
            }
            Expr::Member(base, name) => {
                if direct_plain_store {
                    return Ok(vec![jsast::assign_stmt(Expr::Member(base, name), value)]);
                }
                jsast::rt_call(put_helper, vec![*base, jsast::string(name), value])
            }
            target => {
                if heap.typed_heap_enabled {
                    match self.try_inline_overwrite(pointee, target, value) {
                        Ok(stmts) => return Ok(stmts),
                        Err((target, value)) => jsast::rt_call("overwrite", vec![target, value]),
                    }
                } else {
                    jsast::rt_call("overwrite", vec![target, value])
                }
            }
        };
        Ok(vec![jsast::expr_stmt(call)])
    }

    /// Lowers `*target = value` on a bare indirect reference (`&mut T`) to direct field/element
    /// assignments without calling `__rt.overwrite` (avoiding `for..in`, `delete`, V8 dictionary
    /// mode transitions, and temporary object literal allocations).
    fn try_inline_overwrite(
        &self,
        pointee: TypeId,
        target: Expr,
        value: Expr,
    ) -> Result<Vec<Stmt>, (Expr, Expr)> {
        let table = self.table;
        let peeled = table.peeled(pointee);
        let Some(desc) = table.get(peeled) else {
            return Err((target, value));
        };
        let mut stmts = Vec::new();
        match &desc.kind {
            TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } => {
                let flat_keys: Option<Vec<String>> = if self.heap().native_string_enabled
                    && desc.meta.candidate == RepresentationCandidate::String
                {
                    Some(vec!["s".to_owned(), "cap".to_owned()])
                } else if self.heap().native_vec_enabled
                    && crate::analysis::heap::is_direct_global_vec_ty(self.table, peeled)
                {
                    Some(vec!["buf".to_owned(), "len".to_owned()])
                } else {
                    None
                };
                if let Some(keys) = flat_keys {
                    if let Expr::Object(ref entries) = value {
                        let mut unique = std::collections::HashSet::with_capacity(entries.len());
                        let matches_exact = entries.len() == keys.len()
                            && entries
                                .iter()
                                .all(|(k, _)| keys.contains(k) && unique.insert(k.as_str()));
                        if matches_exact {
                            let Expr::Object(entries) = value else {
                                unreachable!()
                            };
                            let t = self.bind_temp_if_needed(target, &mut stmts);
                            let mut evaluated = Vec::with_capacity(entries.len());
                            for (k, v) in entries {
                                let val = self.bind_atom(v, &mut stmts);
                                evaluated.push((k, val));
                            }
                            for (k, v) in evaluated {
                                stmts.push(jsast::assign_stmt(jsast::member(t.clone(), k), v));
                            }
                            return Ok(stmts);
                        }
                    }
                    let t = self.bind_temp_if_needed(target, &mut stmts);
                    let v = self.bind_temp_if_needed(value, &mut stmts);
                    for k in keys {
                        stmts.push(jsast::assign_stmt(
                            jsast::member(t.clone(), k.clone()),
                            jsast::member(v.clone(), k),
                        ));
                    }
                    return Ok(stmts);
                }
                let candidate_safe = match desc.meta.candidate {
                    RepresentationCandidate::None => true,
                    RepresentationCandidate::Vec
                    | RepresentationCandidate::String
                    | RepresentationCandidate::Box => self.all_baseline_carriers(),
                    RepresentationCandidate::BTreeMap => {
                        !self.nested_clone.map && self.all_baseline_carriers()
                    }
                };
                let [variant] = variants.as_slice() else {
                    return Err((target, value));
                };
                if !candidate_safe || variant.fields.is_empty() || variant.fields.len() > 12 {
                    return Err((target, value));
                }
                let mut keys = Vec::with_capacity(variant.fields.len());
                for field in &variant.fields {
                    let Some(k) = value::key_name(self.interns, field.key) else {
                        return Err((target, value));
                    };
                    keys.push(k);
                }
                if let Expr::Object(ref entries) = value {
                    let mut unique = std::collections::HashSet::with_capacity(entries.len());
                    let matches_exact = entries.len() == keys.len()
                        && entries
                            .iter()
                            .all(|(k, _)| keys.contains(k) && unique.insert(k.as_str()));
                    if matches_exact {
                        let Expr::Object(entries) = value else {
                            unreachable!()
                        };
                        let t = self.bind_temp_if_needed(target, &mut stmts);
                        let mut evaluated = Vec::with_capacity(entries.len());
                        for (k, v) in entries {
                            let val = self.bind_atom(v, &mut stmts);
                            evaluated.push((k, val));
                        }
                        for (k, v) in evaluated {
                            stmts.push(jsast::assign_stmt(jsast::member(t.clone(), k), v));
                        }
                        return Ok(stmts);
                    }
                }
                let t = self.bind_temp_if_needed(target, &mut stmts);
                let v = self.bind_temp_if_needed(value, &mut stmts);
                for k in keys {
                    stmts.push(jsast::assign_stmt(
                        jsast::member(t.clone(), k.clone()),
                        jsast::member(v.clone(), k),
                    ));
                }
                Ok(stmts)
            }
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                ..
            } if matches!(
                table.enum_repr(peeled),
                Some(EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false })
            ) =>
            {
                let mut keys: Vec<String> = vec![value::TAG.to_owned()];
                for variant in variants {
                    for field in &variant.fields {
                        let Some(k) = value::key_name(self.interns, field.key) else {
                            return Err((target, value));
                        };
                        if !keys.contains(&k) {
                            keys.push(k);
                        }
                    }
                }
                if keys.len() <= 1 || keys.len() > 12 {
                    return Err((target, value));
                }
                if let Expr::Object(ref entries) = value {
                    let mut unique = std::collections::HashSet::with_capacity(entries.len());
                    let all_known = entries
                        .iter()
                        .all(|(k, _)| keys.contains(k) && unique.insert(k.as_str()));
                    if all_known {
                        let Expr::Object(entries) = value else {
                            unreachable!()
                        };
                        let t = self.bind_temp_if_needed(target, &mut stmts);
                        let mut by_key = std::collections::HashMap::with_capacity(entries.len());
                        for (k, v) in entries {
                            let val = self.bind_atom(v, &mut stmts);
                            by_key.insert(k, val);
                        }
                        for k in keys {
                            let rhs = by_key.remove(&k).unwrap_or_else(jsast::undefined);
                            stmts.push(jsast::assign_stmt(jsast::member(t.clone(), k), rhs));
                        }
                        return Ok(stmts);
                    }
                }
                let t = self.bind_temp_if_needed(target, &mut stmts);
                let v = self.bind_temp_if_needed(value, &mut stmts);
                for k in keys {
                    stmts.push(jsast::assign_stmt(
                        jsast::member(t.clone(), k.clone()),
                        jsast::member(v.clone(), k),
                    ));
                }
                Ok(stmts)
            }
            TypeKind::Tuple(elems) if !elems.is_empty() && elems.len() <= 8 => {
                let len = elems.len();
                if let Expr::Array(ref items) = value {
                    if items.len() == len {
                        let Expr::Array(items) = value else {
                            unreachable!()
                        };
                        let t = self.bind_temp_if_needed(target, &mut stmts);
                        let mut evaluated = Vec::with_capacity(len);
                        for item in items {
                            let val = self.bind_atom(item, &mut stmts);
                            evaluated.push(val);
                        }
                        for (i, val) in evaluated.into_iter().enumerate() {
                            stmts.push(jsast::assign_stmt(
                                jsast::index(t.clone(), jsast::num(i as f64)),
                                val,
                            ));
                        }
                        return Ok(stmts);
                    }
                }
                let t = self.bind_temp_if_needed(target, &mut stmts);
                let v = self.bind_temp_if_needed(value, &mut stmts);
                for i in 0..len {
                    let idx = jsast::num(i as f64);
                    stmts.push(jsast::assign_stmt(
                        jsast::index(t.clone(), idx.clone()),
                        jsast::index(v.clone(), idx),
                    ));
                }
                Ok(stmts)
            }
            _ => Err((target, value)),
        }
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
        let count_nonzero_const = matches!(count, Expr::Num(n) if n != 0.0);
        let keep_scale = size == Some(1) && !self.byte_views.elide_scale_scalar;
        let reads: &[&str] = if keep_scale {
            &[BUF, OFF, ptr::SCALE]
        } else {
            &[BUF, OFF]
        };
        let (p_setup, pointer) = self.reread(pointer, reads);
        let (c_setup, count) = if count_nonzero_const {
            (None, count)
        } else {
            self.reread(count, &[])
        };
        let off = jsast::binary(BinOp::Add, ptr::slot_off(pointer.clone()), count.clone());
        let slot = if keep_scale {
            jsast::object_of(vec![
                (BUF, ptr::slot_buf(pointer.clone())),
                (OFF, off),
                (ptr::SCALE, ptr::slot_scale(pointer.clone())),
            ])
        } else {
            ptr::slot(ptr::slot_buf(pointer.clone()), off)
        };
        let result = if count_nonzero_const {
            slot
        } else {
            jsast::cond(
                jsast::binary(BinOp::StrictEq, count, jsast::num(0)),
                pointer,
                slot,
            )
        };
        let mut seq = Vec::new();
        if let Some(s) = p_setup {
            seq.push(s);
        }
        if let Some(s) = c_setup {
            seq.push(s);
        }
        if seq.is_empty() {
            result
        } else {
            seq.push(result);
            jsast::seq(seq)
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
        jsast::rt_call("offset_from", vec![lhs, rhs])
    }

    /// `lhs == rhs` on two pointers (`ptr.rs::eq`).
    pub fn ptr_eq(&self, pointer_ty: TypeId, lhs: Expr, rhs: Expr) -> Expr {
        let helper = match value::pointee_of(self.table, pointer_ty) {
            Some(pointee)
                if matches!(value::kind(self.table, pointee), Some(TypeKind::Slice(_)))
                    || value::is_mut_str_ptr(self.table, pointer_ty) =>
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
        let (helper, size) = match value::pointee_of(self.table, pointer_ty) {
            Some(pointee)
                if matches!(value::kind(self.table, pointee), Some(TypeKind::Slice(_)))
                    || value::is_mut_str_ptr(self.table, pointer_ty) =>
            {
                let elem_size = match value::kind(self.table, pointee) {
                    Some(TypeKind::Slice(elem)) => size_of(self.table, *elem).unwrap_or(1),
                    _ => 1,
                };
                ("slice_ptr_cmp", elem_size)
            }
            Some(pointee)
                if matches!(
                    value::kind(self.table, pointee),
                    Some(TypeKind::Dynamic { .. })
                ) || self.table.unsized_tail(pointee).is_some() =>
            {
                ("fat_ptr_cmp", 1)
            }
            Some(pointee) => ("ptr_cmp", size_of(self.table, pointee).unwrap_or(1)),
            None => ("ptr_cmp", 1),
        };
        jsast::binary(
            op,
            jsast::rt_call(helper, vec![lhs, rhs, jsast::num(size as f64)]),
            jsast::num(0),
        )
    }

    /// The address of `pointer` (`ptr.rs::addr`).
    pub fn ptr_addr(&self, pointee: TypeId, pointer: Expr) -> Expr {
        let size = size_of(self.table, pointee).unwrap_or(1).max(1);
        jsast::rt_call("addr", vec![pointer, jsast::num(size as f64)])
    }

    /// Whether a single-element `copy`/`copy_nonoverlapping` for `pointee` can lower directly to
    /// an inline slot load/store (zero shim calls, zero slot-record allocations).
    pub(super) fn can_inline_copy_one(&self, pointee: TypeId) -> bool {
        let heap = self.heap();
        heap.typed_heap_enabled
            && !heap.non_rc_raw_alloc_reachable
            && !self.byte_views.raw_byte_derefs
            && self.byte_views.elide_scale_scalar
            && !heap.needs_box_tracking(self.table, pointee)
            && self.soa_layout(pointee).is_none()
            && is_slot_pointee(self.table, pointee)
            && !matches!(
                value::kind(self.table, pointee),
                Some(TypeKind::Array { .. })
            )
    }

    /// Lowers a single-element `copy`/`copy_nonoverlapping` whose `src` is a bare indirect
    /// reference (eliding `__rt.box`) directly into a clone + `deref_write` into `dst`.
    pub fn ptr_copy_one_from_ref(
        &self,
        pointee: TypeId,
        dst: Expr,
        src_ref: Expr,
    ) -> Res<Vec<Stmt>> {
        if self.table.is_zst(pointee) {
            return Ok(Vec::new());
        }
        if let Some(layout) = self.soa_layout(pointee) {
            return Ok(self.soa_ptr_copy(layout, dst, src_ref, jsast::num(1), true));
        }
        let mut stmts = Vec::new();
        let src_val = if self.table.needs_clone(pointee) {
            let (bind, tmp) = self.temp(src_ref);
            stmts.push(bind);
            value::clone_expr_with(self.table, self.interns, pointee, tmp, self.nested_clone)
        } else {
            src_ref
        };
        let dst_expr = if matches!(dst, Expr::Ident(_))
            || matches!(unwrap_offset_cond(dst.clone()), Expr::Object(_))
        {
            dst
        } else {
            let (bind, id) = self.temp(dst);
            stmts.push(bind);
            id
        };
        stmts.extend(self.deref_write(pointee, dst_expr, src_val)?);
        Ok(stmts)
    }

    /// `ptr::copy`/`copy_nonoverlapping` as a statement (`ptr.rs::copy`).
    pub fn ptr_copy(
        &self,
        pointee: TypeId,
        dst: Expr,
        src: Expr,
        count: Expr,
        overlapping: bool,
        count_is_one: bool,
    ) -> Vec<Stmt> {
        if self.table.is_zst(pointee) {
            return Vec::new();
        }
        if let Some(layout) = self.soa_layout(pointee) {
            return self.soa_ptr_copy(layout, dst, src, count, count_is_one);
        }
        if (count_is_one || count == jsast::num(1)) && self.can_inline_copy_one(pointee) {
            let src_elem = slot_element_read(self, src);
            let dst_elem = slot_element_unwrapped(self, dst);
            if self.table.needs_clone(pointee) {
                let (bind, val) = self.temp(src_elem);
                let cloned = value::clone_expr_with(
                    self.table,
                    self.interns,
                    pointee,
                    val,
                    self.nested_clone,
                );
                return vec![bind, jsast::assign_stmt(dst_elem, cloned)];
            }
            return vec![jsast::assign_stmt(dst_elem, src_elem)];
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
                value::clone_expr_with(
                    self.table,
                    self.interns,
                    pointee,
                    jsast::id(CLONE_PARAM),
                    self.nested_clone,
                ),
            ));
            let heap = self.heap();
            if heap.typed_heap_enabled
                && !heap.non_rc_raw_alloc_reachable
                && !heap.needs_box_tracking(self.table, pointee)
            {
                args.push(Expr::Bool(true));
            }
        }
        vec![jsast::expr_stmt(jsast::rt_call(name, args))]
    }

    /// `&value as *const T` (`ptr.rs::ref_to_raw`).
    pub fn ref_to_raw(&self, pointee: TypeId, reference: Expr) -> Expr {
        if self.soa_layout(pointee).is_some() {
            return reference;
        }
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
        if self.soa_layout(pointee).is_some() {
            return raw;
        }
        if self.table.unsized_tail(pointee).is_some() {
            let heap = self.heap();
            if heap.typed_heap_enabled
                && !heap.raw_alloc_reachable
                && !heap.needs_box_tracking(self.table, pointee)
            {
                let (setup, raw_ref) = self.reread(raw, &["ptr", "meta"]);
                let ptr_slot = jsast::member(raw_ref.clone(), "ptr");
                let record = jsast::index(ptr::slot_buf(ptr_slot.clone()), ptr::slot_off(ptr_slot));
                let fat = jsast::object_of(vec![
                    ("ptr", record),
                    ("meta", jsast::member(raw_ref, "meta")),
                ]);
                return sequenced(setup, fat);
            }
            return jsast::rt_call("tail_ref", vec![raw]);
        }
        if self.needs_byte_slice_ref() {
            if let Some(TypeKind::Slice(element)) =
                value::kind(self.table, self.table.peeled(pointee))
            {
                if let Some(TypeKind::Integer { bits: 8, signed }) =
                    value::kind(self.table, self.table.peeled(*element))
                {
                    let mut args = vec![raw];
                    if *signed {
                        args.push(Expr::Bool(true));
                    }
                    return jsast::rt_call("byte_slice_ref", args);
                }
            }
        }
        if !is_slot_pointee(self.table, pointee) || !self.table.is_indirect(pointee) {
            return raw;
        }
        if let Some(TypeKind::Array { element, length }) = value::kind(self.table, pointee) {
            let mut args = vec![raw, jsast::num(*length as f64)];
            if let Some(elem_size) = size_of(self.table, *element).filter(|&s| s > 0) {
                args.push(jsast::num(elem_size as f64));
            }
            return jsast::rt_call("array_ref", args);
        }
        slot_element_read(self, raw)
    }

    /// The zero element factory `__rt.unscale`/`retype` take (`alloc_support.rs::zero_factory`).
    pub(super) fn zero_factory(&self, ty: TypeId) -> Expr {
        match value::zero_value_for_heap(
            self.table,
            self.interns,
            ty,
            self.heap().native_string_enabled,
            self.heap().native_vec_enabled,
        ) {
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
                value::zero_value_for_heap(
                    table,
                    self.interns,
                    field.ty,
                    self.heap().native_string_enabled,
                    self.heap().native_vec_enabled,
                ),
                value::key_name(self.interns, field.key),
            ) else {
                return jsast::null();
            };
            entries.push((key, zero));
        }
        jsast::arrow(Vec::new(), jsast::object(entries))
    }

    /// A fat pointer built from its two halves (`ptr.rs::build_fat`).
    ///
    /// `AggregateKind::RawPtr` accepts any thin pointer as its data operand, so rustc's MIR
    /// optimizer routinely folds away intermediate `PtrToPtr` casts such as
    /// `*mut MaybeUninit<[T; N]> -> *mut MaybeUninit<T>` before building `*mut [MaybeUninit<T>]`.
    /// When `data_ty` is known, convert `data` from its actual pointee to the target data pointer's
    /// pointee through [`Self::cast_pointer`] before wrapping it in a slice, `str`, or thin result.
    pub fn build_fat(
        &self,
        pointee: TypeId,
        mutable: bool,
        data_ty: Option<TypeId>,
        data: Expr,
        meta: Expr,
    ) -> Res<Expr> {
        let table = self.table;
        let data_is_mut_str = data_ty.is_some_and(|dt| value::is_mut_str_ptr(table, dt));
        let (from_pointee, data) = if data_is_mut_str {
            let (setup, s) = self.reread(data, &[BUF, OFF]);
            let u8_ty = table
                .descriptors()
                .iter()
                .find(|d| {
                    matches!(
                        d.kind,
                        TypeKind::Integer {
                            signed: false,
                            bits: 8
                        }
                    )
                })
                .map(|d| d.id);
            (
                u8_ty,
                sequenced(setup, ptr::slot(ptr::slot_buf(s.clone()), ptr::slot_off(s))),
            )
        } else {
            (data_ty.and_then(|ty| value::pointee_of(table, ty)), data)
        };
        let raw_data = |data: Expr| -> Expr {
            if let (Some(dt), Some(fp)) = (data_ty, from_pointee) {
                if matches!(value::kind(table, dt), Some(TypeKind::Reference { .. }))
                    && !matches!(
                        value::kind(table, fp),
                        Some(TypeKind::Array { .. } | TypeKind::Slice(_) | TypeKind::Str)
                    )
                {
                    return self.ref_to_raw(fp, data);
                }
            }
            data
        };
        match value::kind(table, pointee) {
            Some(TypeKind::Dynamic { .. }) => Ok(ptr::fat_dyn(data, meta)),
            Some(TypeKind::Slice(element)) => match value::kind(table, *element) {
                // Zero-sized elements are stored nowhere, so the record keeps the data pointer
                // itself as its `buf`: what `as_ptr` and every element pointer answer, exactly as
                // a `SliceView` over a thin pointer spells it. Nothing is normalized or asked.
                _ if table.is_zst(*element) => Ok(ptr::fat_slice(data, jsast::num(0), meta)),
                Some(TypeKind::Array { length, .. }) => {
                    let data = match from_pointee {
                        Some(fp)
                            if table.peeled(fp) != table.peeled(*element)
                                && matches!(
                                    value::kind(table, fp),
                                    Some(TypeKind::Array { .. })
                                ) =>
                        {
                            self.cast_pointer(fp, *element, data)?
                        }
                        _ => data,
                    };
                    Ok(jsast::rt_call(
                        "chunk_slice",
                        vec![data, meta, jsast::num(*length as f64)],
                    ))
                }
                _ => {
                    let byte_tag = match value::kind(table, *element) {
                        Some(TypeKind::Integer {
                            signed: false,
                            bits: 8,
                        }) => Some(1.0),
                        Some(TypeKind::Integer {
                            signed: true,
                            bits: 8,
                        }) => Some(-1.0),
                        _ => None,
                    };
                    let data = match from_pointee {
                        Some(fp) => self.cast_slice_data(fp, *element, raw_data(data))?,
                        None if data_is_mut_str => {
                            let size = size_of(table, *element).unwrap_or(1);
                            let mut thin_args = vec![data, jsast::num(size as f64)];
                            if matches!(
                                value::kind(table, table.peeled(*element)),
                                Some(TypeKind::Integer {
                                    signed: true,
                                    bits: 8
                                })
                            ) {
                                thin_args.push(jsast::boolean(true));
                            }
                            jsast::rt_call("thin", thin_args)
                        }
                        None => data,
                    };
                    let use_sfp = self.heap().typed_heap_enabled
                        && self.byte_views.elide_scale_scalar
                        && !self.byte_views.raw_byte_derefs;
                    if use_sfp {
                        Ok(jsast::rt_call("sfp", vec![data, meta]))
                    } else {
                        let mut arguments = vec![data, meta];
                        if let Some(tag) = byte_tag {
                            arguments.push(jsast::num(tag));
                        }
                        Ok(jsast::rt_call("slice_from_parts", arguments))
                    }
                }
            },
            Some(TypeKind::Str) => {
                let data = match from_pointee {
                    Some(fp) => self.cast_to_byte_ptr(fp, raw_data(data), 0)?,
                    None => data,
                };
                if mutable {
                    let use_sfp = self.heap().typed_heap_enabled
                        && self.byte_views.elide_scale_scalar
                        && !self.byte_views.raw_byte_derefs;
                    if use_sfp {
                        Ok(jsast::rt_call("sfp", vec![data, meta]))
                    } else {
                        Ok(jsast::rt_call(
                            "slice_from_parts",
                            vec![data, meta, jsast::num(1.0)],
                        ))
                    }
                } else {
                    let (setup, data) = self.reread(data, &[BUF, OFF]);
                    let decoded = jsast::rt_call(
                        "bytes_str",
                        vec![ptr::slot_buf(data.clone()), ptr::slot_off(data), meta],
                    );
                    Ok(sequenced(setup, decoded))
                }
            }
            _ if table.get(pointee).is_some_and(|d| d.layout.sized) => match from_pointee {
                Some(fp) => self.cast_pointer(fp, pointee, raw_data(data)),
                None => {
                    let size = size_of(table, pointee).unwrap_or(0);
                    let mut args = vec![data, jsast::num(size as f64)];
                    if matches!(
                        value::kind(table, table.peeled(pointee)),
                        Some(TypeKind::Integer {
                            signed: true,
                            bits: 8
                        })
                    ) {
                        args.push(Expr::Bool(true));
                    }
                    Ok(jsast::rt_call("thin", args))
                }
            },
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

    /// Converts the thin data operand of a `Slice(element)` fat-pointer build from `from_pointee`
    /// to `element`.
    ///
    /// A 1-byte source pointer (`*mut u8` from `Global::allocate` / `Rc::allocate_for_slice` or a
    /// folded byte offset) uses `__rt.thin` rather than `__rt.unscale*` so a fresh heap block is
    /// not prematurely retyped or zero-filled before a subsequent `__rt.retype_rc` reshapes it,
    /// while scaled byte views (`p.sc !== undefined`) are still unscaled to element units.
    fn cast_slice_data(&self, from_pointee: TypeId, element: TypeId, data: Expr) -> Res<Expr> {
        let table = self.table;
        let mut from = table.peeled(from_pointee);
        let mut depth = 0;
        while let Some(TypeKind::Slice(inner)) = value::kind(table, from) {
            from = table.peeled(*inner);
            depth += 1;
            if depth >= CAST_DEPTH {
                break;
            }
        }
        let to = table.peeled(element);
        if from == to {
            return Ok(data);
        }
        if size_of(table, from) == Some(1) && !table.is_indirect(from) {
            if let Some(size) = size_of(table, to) {
                if size > 1 {
                    return Ok(jsast::rt_call("thin", vec![data, jsast::num(size as f64)]));
                }
            }
        }
        self.cast_pointer(from_pointee, element, data)
    }

    /// Converts a raw pointer with pointee `from_pointee` into a byte-element pointer for `str`
    /// reconstruction (`build_fat` on `TypeKind::Str`), without needing a standalone `u8`
    /// [`TypeId`] in the type table.
    fn cast_to_byte_ptr(&self, from_pointee: TypeId, value: Expr, depth: u32) -> Res<Expr> {
        let table = self.table;
        if depth >= CAST_DEPTH {
            return Err(decline(
                OpClass::Cast,
                format!(
                    "a pointer cast from pointee {:032x} to `u8` exceeded cast depth",
                    from_pointee.0
                ),
            ));
        }
        let from = table.peeled(from_pointee);
        if table.byte_element(from) {
            return Ok(value);
        }
        if table.is_zst(from) {
            return Ok(jsast::rt_call("thin", vec![value, jsast::num(1.0)]));
        }
        match value::kind(table, from) {
            Some(TypeKind::Str) => Ok(self.str_to_byte_ptr(value)),
            Some(TypeKind::Slice(element)) => self.cast_to_byte_ptr(*element, value, depth + 1),
            Some(TypeKind::Array { element, .. }) => {
                let unwindowed = jsast::rt_call("unwindow", vec![value]);
                self.cast_to_byte_ptr(*element, unwindowed, depth + 1)
            }
            _ => {
                if let Some((field_ty, key)) = self.sole_field(from) {
                    let projected = self.project_field(value, key);
                    return self.cast_to_byte_ptr(field_ty, projected, depth + 1);
                }
                if let Some(size) = size_of(table, from) {
                    if size > 1 {
                        return Ok(jsast::rt_call(
                            "scale",
                            vec![value, jsast::num(size as f64)],
                        ));
                    }
                }
                Ok(value)
            }
        }
    }

    /// `s.as_bytes()` (`ptr.rs::str_to_bytes`).
    ///
    /// When folding `__rt.bytes_str(buf, off, len)` straight back into a byte slice, wrap it in
    /// `__rt.byte_slice_ref(...)` so a fresh 1-byte heap allocation (`Box::<str>::from`) commits
    /// `block.fresh = false` before any downstream `__rt.unscale_scalar` (e.g. `is_ascii::runtime`).
    pub fn str_to_bytes(&self, string: Expr) -> Expr {
        let (pre, tail) = peel_seq(string);
        match bytes_str_args(tail) {
            Ok((buf, off, len)) => {
                let slice = ptr::fat_slice(buf, off, len);
                let slice = if self.needs_byte_slice_ref() {
                    jsast::rt_call("byte_slice_ref", vec![slice])
                } else {
                    slice
                };
                sequenced_vec(pre, slice)
            }
            Err(tail) => jsast::rt_call("str_bytes", vec![sequenced_vec(pre, tail)]),
        }
    }

    /// `s.as_ptr()` (`ptr.rs::str_to_byte_ptr`).
    pub fn str_to_byte_ptr(&self, string: Expr) -> Expr {
        let bytes = self.str_to_bytes(string);
        let (setup, sb) = self.reread(bytes, &[BUF, OFF]);
        sequenced(
            setup,
            ptr::slot(ptr::slot_buf(sb.clone()), ptr::slot_off(sb)),
        )
    }

    /// Fresh unmemoized byte view (`ptr.rs::str_to_bytes_mut`) for casting an immutable `&str` /
    /// `*const str` into a mutable byte or `str` pointer, so any write through the detached copy
    /// cannot surface in the memoized view of an equal string.
    pub fn str_to_bytes_mut(&self, string: Expr) -> Expr {
        jsast::rt_call("str_bytes_mut", vec![string])
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
        let (mut pre, tail) = peel_seq(string);
        match tail {
            Expr::Str(s) => {
                sequenced_vec(pre, jsast::num(crate::lower::bstr::literal_len(&s) as f64))
            }
            other => match bytes_str_args(other) {
                Ok((buf, off, len)) => {
                    if !jsast::is_droppable(&buf) {
                        pre.push(buf);
                    }
                    if !jsast::is_droppable(&off) {
                        pre.push(off);
                    }
                    sequenced_vec(pre, len)
                }
                Err(tail) => jsast::rt_call("str_len", vec![sequenced_vec(pre, tail)]),
            },
        }
    }

    /// The length of a fat slice (`ptr.rs::slice_len`): `slice.len`, except over a `&str`'s byte
    /// view, where `__rt.str_bytes(s).len` is `__rt.str_len(s)`. `str::len` is
    /// `self.as_bytes().len()`, and a length needs no bytes.
    pub fn slice_len(&self, slice: Expr) -> Expr {
        let (pre, tail) = peel_seq(slice);
        if !self.byte_views.raw_byte_derefs {
            if let Expr::Call(ref callee, ref args) = tail {
                if matches!(rt_member(callee), Some("sfp" | "slice_from_parts"))
                    && (args.len() == 2 || args.len() == 3)
                    && jsast::is_droppable(&args[0])
                    && args.get(2).is_none_or(jsast::is_droppable)
                {
                    let len = args[1].clone();
                    return sequenced_vec(pre, len);
                }
            }
        }
        match str_bytes_operand(tail) {
            Ok(string) => self.str_len(sequenced_vec(pre, string)),
            Err(Expr::Object(entries)) if jsast::folds_property(&entries, LEN) => {
                sequenced_vec(pre, ptr::slot_len(Expr::Object(entries)))
            }
            Err(tail) => ptr::slot_len(sequenced_vec(pre, tail)),
        }
    }

    /// The `str` arms of the pointer cast matrix over two pointee types (`ptr.rs::cast_str`);
    /// `None` when neither side is a `str`.
    pub fn cast_str(
        &self,
        from_pointee: TypeId,
        to_pointee: TypeId,
        value: Expr,
    ) -> Option<Res<Expr>> {
        self.cast_str_as(
            from_pointee,
            to_pointee,
            value,
            (StrPtrRepr::RefConst, StrPtrRepr::RefConst),
        )
    }

    /// [`Self::cast_str`], told the representation and mutability of both pointer types:
    /// - `&str` (`RefConst`) is a JavaScript string;
    /// - `&mut str`, `*const str`, `*mut str`, `NonNull<str>`, `Unique<str>`, `Box<str>`
    ///   (`FatConst` / `FatMut`) are live `{ buf, off, len }` UTF-8 byte slices.
    ///
    /// Crossing from an immutable `FatSlice` (`*const str` / `NonNull<str>` / `Box<str>`) to a
    /// mutable target (`*mut str` / `&mut str` / `*mut [u8]` / `&mut [u8]` / `*mut u8`) routes
    /// through `__rt.detach_str_memo`: live heap/stack buffers pass through unchanged, while
    /// memoized views from `__rt.str_bytes`/`__rt.bstr_bytes` detach so writes cannot corrupt the
    /// global memo for unrelated string uses.
    fn cast_str_as(
        &self,
        from_pointee: TypeId,
        to_pointee: TypeId,
        value: Expr,
        (from_repr, to_repr): (StrPtrRepr, StrPtrRepr),
    ) -> Option<Res<Expr>> {
        let table = self.table;
        let (from_str, to_str) = (
            value::is_str(table, from_pointee),
            value::is_str(table, to_pointee),
        );
        if !from_str && !to_str {
            return None;
        }
        let detach_if_const_to_mut = |v: Expr| {
            if !from_repr.is_mut() && to_repr.is_mut() {
                jsast::rt_call("detach_str_memo", vec![v])
            } else {
                v
            }
        };
        let unsupported = || {
            Ok(self.zombie(format!(
                "rustc_codegen_js cannot cast a pointer to `{}` into a pointer to `{}`: a `str` \
                 is a JavaScript string, whose only byte view is the UTF-8 one `as_bytes` produces",
                self.type_display(from_pointee),
                self.type_display(to_pointee),
            )))
        };
        Some(match (from_str, to_str) {
            (true, true) => Ok(match (from_repr, to_repr) {
                (StrPtrRepr::RefConst, StrPtrRepr::RefConst) => value,
                (StrPtrRepr::FatConst, StrPtrRepr::FatMut) => {
                    jsast::rt_call("detach_str_memo", vec![value])
                }
                (
                    StrPtrRepr::FatConst | StrPtrRepr::FatMut,
                    StrPtrRepr::FatConst | StrPtrRepr::FatMut,
                ) => value,
                (StrPtrRepr::FatConst | StrPtrRepr::FatMut, StrPtrRepr::RefConst) => {
                    self.bytes_to_str(value)
                }
                (StrPtrRepr::RefConst, StrPtrRepr::FatMut) => self.str_to_bytes_mut(value),
                (StrPtrRepr::RefConst, StrPtrRepr::FatConst) => self.str_to_bytes(value),
            }),
            (true, false) if from_repr.is_fat() => match value::kind(table, to_pointee) {
                Some(TypeKind::Slice(element)) if value::is_u8(table, *element) => {
                    Ok(detach_if_const_to_mut(value))
                }
                _ if value::is_u8(table, to_pointee) || table.is_zst(to_pointee) => {
                    let (setup, sb) = self.reread(detach_if_const_to_mut(value), &[BUF, OFF]);
                    Ok(sequenced(
                        setup,
                        ptr::slot(ptr::slot_buf(sb.clone()), ptr::slot_off(sb)),
                    ))
                }
                _ => unsupported(),
            },
            (true, false) => match value::kind(table, to_pointee) {
                Some(TypeKind::Slice(element)) if value::is_u8(table, *element) => {
                    Ok(if to_repr.is_mut() {
                        self.str_to_bytes_mut(value)
                    } else {
                        self.str_to_bytes(value)
                    })
                }
                _ if value::is_u8(table, to_pointee) => Ok(if to_repr.is_mut() {
                    let bytes = self.str_to_bytes_mut(value);
                    let (setup, sb) = self.reread(bytes, &[BUF, OFF]);
                    sequenced(
                        setup,
                        ptr::slot(ptr::slot_buf(sb.clone()), ptr::slot_off(sb)),
                    )
                } else {
                    self.str_to_byte_ptr(value)
                }),
                _ => unsupported(),
            },
            (false, true) => match value::kind(table, from_pointee) {
                Some(TypeKind::Slice(element)) if table.byte_element(*element) => {
                    Ok(if to_repr.is_fat() {
                        detach_if_const_to_mut(value)
                    } else {
                        self.bytes_to_str(value)
                    })
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
        let from_repr = StrPtrRepr::from_ptr_ty(self.table, from_ty);
        let to_repr = StrPtrRepr::from_ptr_ty(self.table, to_ty);
        self.cast_str_as(from_pointee, to_pointee, value, (from_repr, to_repr))
    }

    /// `value as *const to_pointee` (`ptr.rs::cast_pointer`).
    pub fn cast_pointer(&self, from_pointee: TypeId, to_pointee: TypeId, value: Expr) -> Res<Expr> {
        if let Some(converted) = self.cast_str(from_pointee, to_pointee, value.clone()) {
            return converted;
        }
        self.cast_pointer_at(from_pointee, to_pointee, value, 0)
    }

    /// Human-readable Rust type name for zombie diagnostics, falling back to the stable type hash
    /// when the producer did not record a printed path.
    pub(crate) fn type_display(&self, ty: TypeId) -> String {
        if let Some(name) = self
            .table
            .get(ty)
            .and_then(|descriptor| descriptor.name)
            .and_then(|id| self.interns.str(id))
        {
            return name.to_owned();
        }
        if matches!(value::kind(self.table, ty), Some(TypeKind::Str)) {
            return "str".to_owned();
        }
        format!("{:032x}", ty.0)
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
            Ok(self.zombie(format!(
                "rustc_codegen_js cannot cast a pointer to `{}` into a pointer to `{}`",
                self.type_display(from_pointee),
                self.type_display(to_pointee),
            )))
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
                    let mut arguments = vec![reference, jsast::num(size as f64)];
                    if let Some(kind) = self.scalar_view_kind(from_pointee) {
                        arguments.push(kind);
                    }
                    jsast::rt_call("erase", arguments)
                }
                _ => reference,
            });
        }
        if table.is_zst(from) {
            // Erasure can carry a scaled byte view or element size (`es`) through `*const ()`.
            // Restore its units just as a sized `aggregate_raw_ptr` does, rather than treating it as a reference.
            return Ok(match size_of(table, to) {
                Some(size) => {
                    let mut args = vec![value, jsast::num(size as f64)];
                    if matches!(
                        value::kind(table, to),
                        Some(TypeKind::Integer {
                            signed: true,
                            bits: 8
                        })
                    ) {
                        args.push(Expr::Bool(true));
                    }
                    jsast::rt_call("thin", args)
                }
                None => self.ref_to_raw(to, value),
            });
        }
        self.cast_pointer_sized(PointerCast {
            from_pointee,
            to_pointee,
            from,
            to,
            value,
            depth,
        })
    }

    fn cast_pointer_sized(&self, cast: PointerCast) -> Res<Expr> {
        let PointerCast {
            from_pointee,
            to_pointee,
            from,
            to,
            value,
            depth,
        } = cast;
        let table = self.table;
        let unsupported = || {
            Ok(self.zombie(format!(
                "rustc_codegen_js cannot cast a pointer to `{}` into a pointer to `{}`",
                self.type_display(from_pointee),
                self.type_display(to_pointee),
            )))
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
                    if let Some(clone) = self.non_scalar_clone_arg(to_element) {
                        arguments.push(clone);
                    }
                    "unscale_uninit"
                } else if let Some(kind) = self.scalar_view_kind(element) {
                    arguments.push(kind);
                    "unscale_scalar"
                } else {
                    if let Some(clone) = self.non_scalar_clone_arg(to_element) {
                        arguments.push(clone);
                    }
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
            match (value::kind(table, from), value::kind(table, to)) {
                (
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: 8,
                    }),
                    Some(TypeKind::Integer {
                        signed: true,
                        bits: 8,
                    }),
                ) => {
                    return Ok(jsast::rt_call("byte_sign", vec![value, Expr::Bool(true)]));
                }
                (
                    Some(TypeKind::Integer {
                        signed: true,
                        bits: 8,
                    }),
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: 8,
                    }),
                ) => {
                    return Ok(jsast::rt_call("byte_sign", vec![value, Expr::Bool(false)]));
                }
                _ => {}
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
            if !self.byte_views.elide_scale_scalar {
                if let Some(TypeKind::Integer { signed, bits: 8 }) = value::kind(table, to) {
                    if let Some(kind) = self.scalar_view_kind(from_pointee) {
                        let mut arguments = vec![value, jsast::num(from_size as f64), kind];
                        if *signed {
                            arguments.push(Expr::Bool(true));
                        }
                        return Ok(jsast::rt_call("scale_scalar", arguments));
                    }
                }
            }
            let mut arguments = vec![value, jsast::num(from_size as f64)];
            if let Some(clone) = self.non_scalar_clone_arg(from_pointee) {
                arguments.push(clone);
            }
            return Ok(jsast::rt_call("scale", arguments));
        }
        if from_size == 1 && !table.is_indirect(from) {
            let mut arguments = vec![
                value,
                jsast::num(to_size as f64),
                self.zero_factory(to_pointee),
            ];
            let member = if table.crosses_transparent_union(from_pointee) {
                if let Some(clone) = self.non_scalar_clone_arg(to_pointee) {
                    arguments.push(clone);
                }
                "unscale_uninit"
            } else if let Some(kind) = self.scalar_view_kind(to_pointee) {
                arguments.push(kind);
                "unscale_scalar"
            } else {
                if let Some(clone) = self.non_scalar_clone_arg(to_pointee) {
                    arguments.push(clone);
                }
                "unscale"
            };
            return Ok(jsast::rt_call(member, arguments));
        }
        unsupported()
    }

    /// The optional clone argument handed to `__rt.scale` / `__rt.unscale` / `__rt.unscale_uninit`
    /// for non-scalar element types (`ptr.rs::non_scalar_clone_arg`).
    fn non_scalar_clone_arg(&self, pointee: TypeId) -> Option<Expr> {
        if self.table.needs_clone(pointee) {
            Some(jsast::arrow(
                vec![CLONE_PARAM.to_string()],
                value::clone_expr_with(
                    self.table,
                    self.interns,
                    pointee,
                    jsast::id(CLONE_PARAM),
                    self.nested_clone,
                ),
            ))
        } else if self.table.is_indirect(pointee) {
            Some(Expr::Bool(true))
        } else {
            None
        }
    }

    /// A zero factory for a type whose zero is not the byte zero
    /// (`alloc_support.rs::non_byte_zero_factory`).
    fn non_byte_zero_factory(&self, ty: TypeId) -> Option<Expr> {
        let value = value::zero_value_for_heap(
            self.table,
            self.interns,
            ty,
            self.heap().native_string_enabled,
            self.heap().native_vec_enabled,
        )?;
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
            return Ok(self.zombie(format!(
                "a struct with a `{}` tail is not supported by rustc_codegen_js; only a slice \
                 tail is, because only a run of elements can be laid out after the header",
                self.type_display(tail.ty),
            )));
        };
        let Some(element_size) = size_of(table, *element) else {
            return Ok(self.zombie(format!(
                "the elements of `{}` have no size here",
                self.type_display(tail.ty),
            )));
        };
        if !matches!(value::kind(table, from), Some(TypeKind::Slice(_))) {
            return Ok(self.zombie(format!(
                "rustc_codegen_js cannot cast a pointer to `{}` into a pointer to `{}`: the \
                 length of the tail is not carried by the source pointer",
                self.type_display(from),
                self.type_display(to),
            )));
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

    /// Like [`Self::sole_field`], also returning the total field count (arity) of the wrapper
    /// aggregate so indexed containers (tuples, 1-element arrays, closures) preserve their exact
    /// length and slot position when rebuilt.
    pub fn sole_field_with_arity(&self, ty: TypeId) -> Option<(TypeId, FieldKey, usize)> {
        let table = self.table;
        let descriptor = table.get(table.peeled(ty))?;
        let (fields, arity): (&[rcgjs_object::jir::types::FieldDescriptor], usize) =
            match &descriptor.kind {
                TypeKind::Adt {
                    form: rcgjs_object::jir::types::NominalForm::Struct,
                    variants,
                    ..
                } => {
                    let fields = &variants.first()?.fields;
                    (fields, fields.len())
                }
                TypeKind::Tuple(elems) => (
                    &descriptor.js.fields,
                    elems.len().max(descriptor.js.fields.len()),
                ),
                TypeKind::Closure { upvars, .. } => (
                    &descriptor.js.fields,
                    upvars.len().max(descriptor.js.fields.len()),
                ),
                TypeKind::Array { element, length: 1 } if !table.is_zst(*element) => {
                    return Some((*element, FieldKey::Index(0), 1));
                }
                _ => return None,
            };
        let mut live = fields.iter().filter(|field| !table.is_zst(field.ty));
        let field = live.next()?;
        if live.next().is_some() {
            return None;
        }
        Some((field.ty, field.key, arity))
    }

    /// The one non-zero-sized field of an aggregate with exactly one, with its key
    /// (`intrinsics.rs::sole_field`).
    pub fn sole_field(&self, ty: TypeId) -> Option<(TypeId, FieldKey)> {
        self.sole_field_with_arity(ty).map(|(ty, key, _)| (ty, key))
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

/// Collapses a zero-offset guard `c === 0 ? p : { buf: p.buf, off: p.off + c }` emitted by
/// [`JsCx::ptr_add`] when the pointer is immediately dereferenced (a dereferenced pointer cannot
/// be dangling, so both arms evaluate to `p.buf[p.off + c]` with zero slot allocation).
pub(super) fn unwrap_offset_cond(expr: Expr) -> Expr {
    match expr {
        Expr::Cond(cond, _then_expr, else_expr)
            if matches!(
                cond.as_ref(),
                Expr::Binary(BinOp::StrictEq, _, zero) if **zero == jsast::num(0)
            ) && matches!(else_expr.as_ref(), Expr::Object(_)) =>
        {
            *else_expr
        }
        Expr::Seq(mut items) if !items.is_empty() => {
            let last = items.pop().expect("non-empty seq");
            items.push(unwrap_offset_cond(last));
            Expr::Seq(items)
        }
        other => other,
    }
}

/// If `expr` (optionally after a `Seq` setup) is `__rt.box(inner)`, returns `Ok((prefix, inner))`.
fn unwrap_rt_box(expr: Expr) -> Result<(Vec<Expr>, Expr), Expr> {
    let (prefix, tail) = peel_seq(expr);
    match tail {
        Expr::Call(callee, mut args) if args.len() == 1 && rt_member(&callee) == Some("box") => {
            Ok((prefix, args.pop().expect("1 arg")))
        }
        other => Err(sequenced_vec(prefix, other)),
    }
}

/// Lowers `pointer.buf[pointer.off]` for a read-only slot dereference or `raw_to_ref`, collapsing
/// ephemeral `__rt.box(x)` calls directly to `x` (since `__rt.box(x)` guarantees
/// `slot.buf[slot.off] === x`) before falling back to [`slot_element_unwrapped`].
pub(super) fn slot_element_read(cx: &JsCx<'_>, pointer: Expr) -> Expr {
    match unwrap_rt_box(unwrap_offset_cond(pointer)) {
        Ok((prefix, inner)) => sequenced_vec(prefix, inner),
        Err(pointer) => slot_element_unwrapped(cx, pointer),
    }
}

/// Lowers `pointer.buf[pointer.off]` for a dereferenced slot, collapsing any `ptr_add` zero-guard
/// and folding `{ buf, off }` object literals (including after a `Seq` setup) directly to
/// `buf[off]` without allocating a temporary slot object.
pub(super) fn slot_element_unwrapped(cx: &JsCx<'_>, pointer: Expr) -> Expr {
    let pointer = unwrap_offset_cond(pointer);
    let (mut prefix, tail) = peel_seq(pointer);
    if let Expr::Object(fields) = &tail {
        let buf = fields
            .iter()
            .find(|(k, _)| k == BUF)
            .map(|(_, v)| v.clone());
        let off = fields
            .iter()
            .find(|(k, _)| k == OFF)
            .map(|(_, v)| v.clone());
        if let (Some(buf), Some(off)) = (buf, off) {
            let buf_expr = if prefix.is_empty() {
                buf
            } else {
                prefix.push(buf);
                jsast::seq(prefix)
            };
            return jsast::index(buf_expr, off);
        }
    }
    let rebuilt = if prefix.is_empty() {
        tail
    } else {
        prefix.push(tail);
        jsast::seq(prefix)
    };
    let (setup, slot) = cx.reread(rebuilt, &[BUF, OFF]);
    sequenced(setup, ptr::slot_element(slot))
}

fn peel_seq(expr: Expr) -> (Vec<Expr>, Expr) {
    match expr {
        Expr::Seq(mut items) if !items.is_empty() => {
            let tail = items.pop().expect("non-empty seq");
            (items, tail)
        }
        other => (Vec::new(), other),
    }
}

fn sequenced_vec(mut pre: Vec<Expr>, tail: Expr) -> Expr {
    if pre.is_empty() {
        tail
    } else {
        pre.push(tail);
        jsast::seq(pre)
    }
}

/// If `expr` is `__rt.bytes_str(b, o, l)`, returns `Ok((b, o, l))`.
pub(super) fn bytes_str_args(expr: Expr) -> Result<(Expr, Expr, Expr), Expr> {
    match expr {
        Expr::Call(callee, mut args)
            if args.len() == 3 && rt_member(&callee) == Some("bytes_str") =>
        {
            let len = args.pop().expect("3 args");
            let off = args.pop().expect("3 args");
            let buf = args.pop().expect("3 args");
            Ok((buf, off, len))
        }
        other => Err(other),
    }
}

fn unwrap_addr_ptr(expr: &Expr) -> Option<(Expr, Expr)> {
    let inner = match expr {
        Expr::Binary(BinOp::UShr, lhs, rhs) if is_zero_num(rhs) => lhs.as_ref(),
        other => other,
    };
    let Expr::Call(callee, args) = inner else {
        return None;
    };
    if args.len() == 2 && rt_member(callee) == Some("addr") && jsast::is_droppable(&args[1]) {
        Some((args[0].clone(), args[1].clone()))
    } else {
        None
    }
}

/// Matches a strict comparison between `__rt.addr(ptr, align)` (optionally `>>> 0`) and `0`,
/// returning `__rt.ptr_is_null(ptr)` (or `__rt.ptr_is_null(ptr, align)` when `align != 1`) so
/// callers avoid `_addr_mint`.
pub fn simplify_addr_zero_eq(lhs: &Expr, rhs: &Expr) -> Option<Expr> {
    let (ptr, align) = if is_zero_num(rhs) {
        unwrap_addr_ptr(lhs)?
    } else if is_zero_num(lhs) {
        unwrap_addr_ptr(rhs)?
    } else {
        return None;
    };
    let args = if is_one_num(&align) {
        vec![ptr]
    } else {
        vec![ptr, align]
    };
    Some(jsast::rt_call("ptr_is_null", args))
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

/// The string a `__rt.str_bytes(s)` call encodes, or the expression back when it is anything else
/// (`ptr.rs::str_bytes_operand`). `str_bytes` has no effect a program can observe, so a consumer
/// that needs only what the string itself answers may drop the call.
fn str_bytes_operand(expr: Expr) -> Result<Expr, Expr> {
    match expr {
        Expr::Call(callee, mut args)
            if args.len() == 1 && rt_member(&callee) == Some("str_bytes") =>
        {
            Ok(args.pop().expect("one argument"))
        }
        other => Err(other),
    }
}
