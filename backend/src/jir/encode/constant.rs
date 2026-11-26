//! Constant-value and evaluated-memory decoding into JIR-S [`Constant`] and [`ConstTree`].

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::mir::interpret::{AllocId, Allocation, GlobalAlloc, Scalar};
use rustc_middle::ty::{self, Ty};

use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::semantic::{ConstTree, Constant, GlobalReference, StaticStep};

use super::{EncodeCore, FnEncoder};
use crate::const_raw::StaticPathStep;
use crate::jir::types::{definition_id, type_name};
use crate::value::typing_env;

/// The largest anonymous constant allocation decoded into an owned constant tree.
pub(super) const MAX_CONST_MEMORY_BYTES: u64 = 65536;

/// The deepest pointer chain followed while decoding constant memory.
const MAX_CONST_TREE_DEPTH: u32 = 64;

impl<'a, 'tcx> FnEncoder<'a, 'tcx> {
    pub(super) fn constant(
        &mut self,
        constant: &mir::ConstOperand<'tcx>,
    ) -> Result<Constant, EncodeDecline> {
        let tcx = self.tcx();
        let konst = self.monomorphize(constant.const_);
        let ty = konst.ty();
        let value = konst
            .eval(tcx, typing_env(), constant.span)
            .map_err(|_| EncodeDecline::ConstantEvaluation { ty: type_name(ty) })?;
        match value {
            mir::ConstValue::ZeroSized => Ok(Constant::ZeroSized),
            mir::ConstValue::Scalar(Scalar::Int(value))
                if crate::const_raw::is_primitive(ty) || crate::const_raw::is_pointer(ty) =>
            {
                Ok(Constant::Scalar {
                    bits: value.to_uint(value.size()),
                    bytes: value.size().bytes() as u8,
                })
            }
            // A non-primitive scalar constant (`Option<NonZero<T>>`, `Option<bool>`,
            // `Option<Ordering>`, `None::<&T>`, transparent wrappers, single-field structs,
            // single-scalar tuples/arrays/closures, fieldless enums) holds its memory layout in
            // one register. Decode it through a scratch allocation laid out for `ty`, matching
            // v1 `constant.rs::scalar_to_expr` and `ConstValue::Indirect` (MIRES-006).
            mir::ConstValue::Scalar(Scalar::Int(value)) => {
                let scratch = crate::const_raw::scratch_alloc(tcx, value, ty)
                    .ok_or_else(|| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
                Ok(Constant::Memory(self.core.const_tree(
                    &scratch,
                    Size::ZERO,
                    ty,
                    0,
                )?))
            }
            // A pointer that is the one scalar of a type around it (`Option<&T>`,
            // `Option<NonNull<T>>`, `Option<fn()>`) decodes from a scratch allocation holding it,
            // provenance included, as that type stored in memory does.
            mir::ConstValue::Scalar(pointer @ Scalar::Ptr(..))
                if !crate::const_raw::is_pointer(ty) =>
            {
                let scratch = crate::const_raw::scratch_pointer_alloc(tcx, pointer, ty)
                    .ok_or_else(|| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
                Ok(Constant::Memory(self.core.const_tree(
                    &scratch,
                    Size::ZERO,
                    ty,
                    0,
                )?))
            }
            mir::ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
                let (provenance, offset) = pointer.prov_and_relative_offset();
                match tcx.global_alloc(provenance.alloc_id()) {
                    GlobalAlloc::Memory(allocation) => {
                        let pointee = ty.builtin_deref(true).ok_or_else(|| {
                            EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                        })?;
                        let allocation = allocation.inner();
                        check_const_memory(allocation)?;
                        Ok(Constant::Memory(self.core.const_memory_pointer(
                            allocation, offset, pointee, None, 0,
                        )?))
                    }
                    GlobalAlloc::Static(def_id) => {
                        let pointee = ty.builtin_deref(true).ok_or_else(|| {
                            EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                        })?;
                        match self.core.static_pointer_const_tree(
                            provenance.alloc_id(),
                            def_id,
                            offset,
                            pointee,
                            ty,
                        )? {
                            ConstTree::Global { target, offset } => {
                                Ok(Constant::Pointer { target, offset })
                            }
                            other => Ok(Constant::Memory(other)),
                        }
                    }
                    _ => Ok(Constant::Pointer {
                        target: self.core.global_reference(provenance.alloc_id())?,
                        offset: offset.bytes(),
                    }),
                }
            }
            mir::ConstValue::Slice { alloc_id, meta }
                if crate::value::peel_pattern(crate::value::peel_transparent(tcx, ty))
                    .builtin_deref(true)
                    .is_some_and(|p| p.is_str()) =>
            {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::StringConstantNoMemory);
                };
                let text = crate::const_raw::read_str(allocation.inner(), Size::ZERO, meta)
                    .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                Ok(Constant::String(self.core.types.str(text)))
            }
            mir::ConstValue::Slice { alloc_id, meta } => {
                if let GlobalAlloc::Static(def_id) = tcx.global_alloc(alloc_id) {
                    let pointee =
                        crate::value::peel_pattern(crate::value::peel_transparent(tcx, ty))
                            .builtin_deref(true)
                            .unwrap_or_else(|| ty.peel_refs());
                    let (peeled, offset) = self.core.peel_transparent_at(pointee);
                    if let ty::Slice(element) = peeled.kind() {
                        return Ok(Constant::Memory(self.core.static_slice_const_tree(
                            alloc_id, def_id, offset, *element, meta, ty,
                        )?));
                    }
                    return Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) });
                }
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::SliceConstantNoMemory);
                };
                let allocation = allocation.inner();
                check_const_memory(allocation)?;
                let pointee = ty.peel_refs();
                // A transparent wrapper over the slice (`CStr` over `[c_char]`) is the slice it
                // wraps, at the wrapper's field offset.
                let (peeled, offset) = self.core.peel_transparent_at(pointee);
                if let ty::Slice(element) = peeled.kind() {
                    return Ok(Constant::Memory(ConstTree::Slice(
                        self.core
                            .const_slice(allocation, offset, *element, meta, 0)?,
                    )));
                }
                if peeled.is_str() {
                    let text = crate::const_raw::read_str(allocation, offset, meta)
                        .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                    return Ok(Constant::Memory(ConstTree::Str(self.core.types.str(text))));
                }
                // `&CStr`-shaped: a struct whose unsized tail takes the length. v1 prints a
                // zombie for these; the tree spells the record with its tail decoded.
                if crate::value::unsized_tail_indexed(tcx, pointee).is_some() {
                    return Ok(Constant::Memory(ConstTree::Reference(Box::new(
                        self.core
                            .const_tail_struct(allocation, Size::ZERO, pointee, meta, 0)?,
                    ))));
                }
                Err(EncodeDecline::SliceConstantNonSlicePointee { ty: type_name(ty) })
            }
            mir::ConstValue::Indirect { alloc_id, offset } => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::IndirectConstantNoMemory);
                };
                let allocation = allocation.inner();
                check_const_memory(allocation)?;
                Ok(Constant::Memory(
                    self.core.const_tree(allocation, offset, ty, 0)?,
                ))
            }
        }
    }
}

impl<'a, 'tcx> EncodeCore<'a, 'tcx> {
    /// Decodes the value a pointer into anonymous immutable memory stands for.
    fn const_memory_pointer(
        &mut self,
        allocation: &Allocation,
        offset: Size,
        pointee: Ty<'tcx>,
        meta: Option<u64>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if pointee.is_str() {
            let length = meta.ok_or(EncodeDecline::StrConstantNoLength)?;
            let text = crate::const_raw::read_str(allocation, offset, length)
                .map_err(|_| EncodeDecline::StringConstantInvalid)?;
            return Ok(ConstTree::Str(self.types.str(text)));
        }
        if let ty::Slice(element) = pointee.kind() {
            let length = meta.ok_or_else(|| EncodeDecline::SliceConstantNoLength {
                pointee: type_name(pointee),
            })?;
            return Ok(ConstTree::Slice(
                self.const_slice(allocation, offset, *element, length, depth)?,
            ));
        }
        Ok(ConstTree::Reference(Box::new(
            self.const_tree(allocation, offset, pointee, depth)?,
        )))
    }

    /// `ty` with its transparent wrappers peeled, and the byte offset of the value that remains
    /// inside a value of `ty` (`value::peel_transparent`, keeping the offsets).
    fn peel_transparent_at(&self, mut ty: Ty<'tcx>) -> (Ty<'tcx>, Size) {
        let tcx = self.tcx();
        let mut offset = Size::ZERO;
        for _ in 0..16 {
            let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) else {
                break;
            };
            let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
                break;
            };
            offset += layout.fields.offset(index.as_usize());
            ty = field_ty;
        }
        (crate::value::peel_pattern(ty), offset)
    }

    /// A struct with an unsized tail (`CStr`, `Path`'s inner) at `offset`: its sized fields
    /// decode as usual and the tail takes `length` from the wide pointer's metadata.
    fn const_tail_struct(
        &mut self,
        allocation: &Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        length: u64,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if depth >= MAX_CONST_TREE_DEPTH {
            return Err(EncodeDecline::ConstantTooDeep);
        }
        let depth = depth + 1;
        let tcx = self.tcx();
        let layout = tcx
            .layout_of(typing_env().as_query_input(ty))
            .map_err(|_| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
        if let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) {
            let field_offset = offset + layout.fields.offset(index.as_usize());
            return self.const_tail_struct(allocation, field_offset, field_ty, length, depth);
        }
        let ty::Adt(def, args) = ty.kind() else {
            return Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) });
        };
        if !def.is_struct() {
            return Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) });
        }
        let variant = def.non_enum_variant();
        let last = variant.fields.len().checked_sub(1);
        let mut fields = Vec::with_capacity(variant.fields.len());
        for (index, field) in variant.fields.iter_enumerated() {
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
            let field_offset = offset + layout.fields.offset(index.as_usize());
            let tree = if Some(index.as_usize()) == last && !field_ty.is_sized(tcx, typing_env()) {
                match field_ty.kind() {
                    ty::Str => {
                        let text = crate::const_raw::read_str(allocation, field_offset, length)
                            .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                        ConstTree::Str(self.types.str(text))
                    }
                    ty::Slice(element) => ConstTree::Slice(self.const_slice(
                        allocation,
                        field_offset,
                        *element,
                        length,
                        depth,
                    )?),
                    _ => {
                        self.const_tail_struct(allocation, field_offset, field_ty, length, depth)?
                    }
                }
            } else {
                self.const_tree(allocation, field_offset, field_ty, depth)?
            };
            fields.push(tree);
        }
        Ok(ConstTree::Aggregate(fields))
    }

    fn const_slice(
        &mut self,
        allocation: &Allocation,
        offset: Size,
        element: Ty<'tcx>,
        length: u64,
        depth: u32,
    ) -> Result<Vec<ConstTree>, EncodeDecline> {
        let tcx = self.tcx();
        if length > MAX_CONST_MEMORY_BYTES {
            return Err(EncodeDecline::SliceConstantTooLong);
        }
        let layout = tcx
            .layout_of(typing_env().as_query_input(element))
            .map_err(|_| EncodeDecline::SliceElementNoLayout {
                element: type_name(element),
            })?;
        let stride = layout.size.bytes();
        let length =
            usize::try_from(length).map_err(|_| EncodeDecline::SliceConstantLengthOverflow)?;
        if !crate::const_raw::slice_fits(allocation, offset, stride, length) {
            return Err(EncodeDecline::SliceConstantOverrun {
                element: type_name(element),
            });
        }
        (0..length)
            .map(|index| {
                self.const_tree(
                    allocation,
                    offset + Size::from_bytes(stride * index as u64),
                    element,
                    depth,
                )
            })
            .collect()
    }

    /// Rebuilds the value of type `ty` stored at `offset` in constant memory (v1's
    /// `late_ir::Encoder::const_tree`, the owned mirror of the printed-JavaScript reader).
    pub(super) fn const_tree(
        &mut self,
        allocation: &Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if depth >= MAX_CONST_TREE_DEPTH {
            return Err(EncodeDecline::ConstantTooDeep);
        }
        let depth = depth + 1;
        let tcx = self.tcx();
        let layout = tcx
            .layout_of(typing_env().as_query_input(ty))
            .map_err(|_| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;

        if let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) {
            return self.const_tree(
                allocation,
                offset + layout.fields.offset(index.as_usize()),
                field_ty,
                depth,
            );
        }

        match ty.kind() {
            ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
                let int = crate::const_raw::read_scalar(tcx, allocation, offset, layout.size)
                    .ok_or_else(|| EncodeDecline::ConstantUnreadable { ty: type_name(ty) })?;
                Ok(ConstTree::Scalar {
                    bits: int.to_uint(int.size()),
                    bytes: int.size().bytes() as u8,
                })
            }
            ty::Pat(base, _) => self.const_tree(allocation, offset, *base, depth),
            ty::Adt(def, args) if def.is_enum() => {
                let variant_index =
                    crate::const_raw::variant_index(tcx, allocation, offset, layout, *def)
                        .map_err(|_| EncodeDecline::ConstantVariantUnreadable {
                            ty: type_name(ty),
                        })?;
                let variant = def.variant(variant_index);
                let variant_layout = layout.for_variant(
                    &rustc_middle::ty::layout::LayoutCx::new(tcx, typing_env()),
                    variant_index,
                );
                let fields = variant
                    .fields
                    .iter_enumerated()
                    .map(|(index, field)| {
                        let field_ty =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                        self.const_tree(
                            allocation,
                            offset + variant_layout.fields.offset(index.as_usize()),
                            field_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Variant {
                    variant: variant_index.as_u32(),
                    fields,
                })
            }
            ty::Adt(def, args) if def.is_struct() => {
                let variant = def.non_enum_variant();
                let fields = variant
                    .fields
                    .iter_enumerated()
                    .map(|(index, field)| {
                        let field_ty =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index.as_usize()),
                            field_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Tuple(elements) if elements.is_empty() => Ok(ConstTree::Unit),
            ty::Tuple(elements) => {
                let fields = elements
                    .iter()
                    .enumerate()
                    .map(|(index, element)| {
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index),
                            element,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Array(element, count) => {
                let count = count.try_to_target_usize(tcx).unwrap_or_default() as usize;
                if count as u64 > MAX_CONST_MEMORY_BYTES {
                    return Err(EncodeDecline::ArrayConstantTooLong);
                }
                let element_layout = tcx
                    .layout_of(typing_env().as_query_input(*element))
                    .map_err(|_| EncodeDecline::ConstantNoLayout {
                        ty: type_name(*element),
                    })?;
                let stride = element_layout.size.bytes();
                let fields = (0..count)
                    .map(|index| {
                        self.const_tree(
                            allocation,
                            offset + Size::from_bytes(stride * index as u64),
                            *element,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Closure(_, args) => {
                let fields = args
                    .as_closure()
                    .upvar_tys()
                    .iter()
                    .enumerate()
                    .map(|(index, upvar_ty)| {
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index),
                            upvar_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Ref(..) | ty::RawPtr(..) | ty::FnPtr(..) => {
                self.const_pointer_field(allocation, offset, ty, depth)
            }
            ty::Adt(def, _) if def.is_union() && layout.is_zst() => Ok(ConstTree::Unit),
            ty::FnDef(..) => Ok(ConstTree::Unit),
            _ => Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) }),
        }
    }

    /// Decodes a pointer stored inside constant memory.
    fn const_pointer_field(
        &mut self,
        allocation: &Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let pointee = ty.builtin_deref(true);
        let wide = pointee.is_some_and(|pointee| crate::const_raw::is_wide_pointee(tcx, pointee));
        let pointer = crate::const_raw::read_pointer(tcx, allocation, offset, wide);
        if let Some(pointee) = pointee {
            let peeled_pointee =
                crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee));
            if matches!(peeled_pointee.kind(), ty::Dynamic(..)) {
                return self.const_dyn_pointer_field(&pointer, ty, depth);
            }
        }
        let Some(alloc_id) = pointer.alloc_id() else {
            let Some(ptr_offset) = pointer.offset else {
                return Err(EncodeDecline::ConstantUninitialized { ty: type_name(ty) });
            };
            if wide {
                if let (Some(pointee), Some(length)) = (pointee, pointer.meta_len()) {
                    let peeled =
                        crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee));
                    let address = ptr_offset.bytes();
                    match peeled.kind() {
                        ty::Slice(_) => {
                            return Ok(ConstTree::SliceAddress { address, length });
                        }
                        ty::Str if length == 0 && address != 0 => {
                            return Ok(ConstTree::Str(self.types.str("")));
                        }
                        _ => {}
                    }
                }
                return Err(EncodeDecline::ConstantAddressNoProvenance { ty: type_name(ty) });
            }
            return Ok(ConstTree::Address(ptr_offset.bytes()));
        };
        let ptr_offset = pointer.offset.unwrap_or(Size::ZERO);
        match tcx.global_alloc(alloc_id) {
            GlobalAlloc::Memory(target) => {
                let pointee = pointee.ok_or_else(|| {
                    EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                })?;
                let target = target.inner();
                check_const_memory(target)?;
                self.const_memory_pointer(target, ptr_offset, pointee, pointer.meta_len(), depth)
            }
            GlobalAlloc::VTable(..) => Err(EncodeDecline::VtablePointerConstant),
            GlobalAlloc::Static(def_id) if wide => {
                let pointee = pointee.ok_or_else(|| {
                    EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                })?;
                let peeled =
                    crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee));
                if let (Some(length), ty::Slice(element)) = (pointer.meta_len(), peeled.kind()) {
                    return self.static_slice_const_tree(
                        alloc_id, def_id, ptr_offset, *element, length, ty,
                    );
                }
                Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) })
            }
            _ if wide => Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) }),
            GlobalAlloc::Static(def_id) => {
                let pointee = pointee.ok_or_else(|| {
                    EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                })?;
                self.static_pointer_const_tree(alloc_id, def_id, ptr_offset, pointee, ty)
            }
            _ => Ok(ConstTree::Global {
                target: self.global_reference(alloc_id)?,
                offset: ptr_offset.bytes(),
            }),
        }
    }

    /// Decodes a fat trait-object pointer (`&dyn Trait` / `*const dyn Trait`) stored inside
    /// constant memory (`constant.rs::read_dyn_pointer`).
    fn const_dyn_pointer_field(
        &mut self,
        pointer: &crate::const_raw::Pointer,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let Some((concrete_ty, principal)) = crate::const_raw::read_vtable(tcx, pointer) else {
            return Err(EncodeDecline::VtablePointerConstant);
        };
        self.pending_vtables.push((concrete_ty, principal));
        let vtable_name = self.cgu.namer.vtable_name(concrete_ty, principal);
        self.extra_refs.insert(vtable_name.clone());
        let vtable = self.types.str(vtable_name.as_str());
        let concrete = self.type_id(concrete_ty)?;
        let data = match pointer.alloc_id() {
            None => {
                let Some(ptr_offset) = pointer.offset else {
                    return Err(EncodeDecline::ConstantUninitialized { ty: type_name(ty) });
                };
                ConstTree::Address(ptr_offset.bytes())
            }
            Some(alloc_id) => {
                let ptr_offset = pointer.offset.unwrap_or(Size::ZERO);
                match tcx.global_alloc(alloc_id) {
                    GlobalAlloc::Memory(target) => {
                        let target = target.inner();
                        check_const_memory(target)?;
                        ConstTree::Reference(Box::new(self.const_tree(
                            target,
                            ptr_offset,
                            concrete_ty,
                            depth,
                        )?))
                    }
                    GlobalAlloc::Static(def_id) => self.static_pointer_const_tree(
                        alloc_id,
                        def_id,
                        ptr_offset,
                        concrete_ty,
                        ty,
                    )?,
                    _ => {
                        return Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) });
                    }
                }
            }
        };
        Ok(ConstTree::Dyn {
            data: Box::new(data),
            concrete,
            vtable,
        })
    }

    /// Resolves a pointer constant into `static` `def_id` at `offset` for target type `pointee`,
    /// returning `ConstTree::Global` for the whole static, `ConstTree::StaticField` for a valid
    /// struct/tuple/array projection, or declining fail-closed otherwise.
    fn static_pointer_const_tree(
        &mut self,
        alloc_id: AllocId,
        def_id: DefId,
        offset: Size,
        pointee: Ty<'tcx>,
        outer_ty: Ty<'tcx>,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let Some(steps) = crate::const_raw::resolve_static_path(tcx, def_id, offset, pointee)
        else {
            return Err(EncodeDecline::StaticSubPointerConstant {
                ty: type_name(outer_ty),
            });
        };
        let target = self.global_reference(alloc_id)?;
        if steps.is_empty() {
            Ok(ConstTree::Global { target, offset: 0 })
        } else {
            let path = self.intern_static_path(steps);
            Ok(ConstTree::StaticField { target, path })
        }
    }

    fn static_slice_const_tree(
        &mut self,
        alloc_id: AllocId,
        def_id: DefId,
        offset: Size,
        element_ty: Ty<'tcx>,
        length: u64,
        outer_ty: Ty<'tcx>,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let Some((steps, start)) =
            crate::const_raw::resolve_static_slice(tcx, def_id, offset, element_ty, length)
        else {
            return Err(EncodeDecline::StaticSubPointerConstant {
                ty: type_name(outer_ty),
            });
        };
        let target = self.global_reference(alloc_id)?;
        let path = self.intern_static_path(steps);
        Ok(ConstTree::StaticSlice {
            target,
            path,
            start,
            length,
        })
    }

    fn intern_static_path(&mut self, steps: Vec<StaticPathStep>) -> Vec<StaticStep> {
        steps
            .into_iter()
            .map(|step| match step {
                StaticPathStep::Field(name) => StaticStep::Field(self.types.str(&name)),
                StaticPathStep::Index(idx) => StaticStep::Index(idx),
            })
            .collect()
    }

    fn global_reference(&mut self, allocation: AllocId) -> Result<GlobalReference, EncodeDecline> {
        let tcx = self.tcx();
        Ok(match tcx.global_alloc(allocation) {
            GlobalAlloc::Function { instance } => {
                let item = self.direct_item(instance);
                GlobalReference::Function {
                    item: item.item,
                    link_name: item.link_name,
                }
            }
            GlobalAlloc::Static(definition) => GlobalReference::Static {
                definition: definition_id(tcx, definition),
                link_name: self
                    .types
                    .str(self.cgu.namer.static_name(definition).as_str()),
            },
            GlobalAlloc::TypeId { ty } => GlobalReference::Type(self.type_id(ty)?),
            GlobalAlloc::Memory(_) => return Err(EncodeDecline::AnonymousPointerConstant),
            GlobalAlloc::VTable(..) => return Err(EncodeDecline::VtablePointerConstantYet),
        })
    }
}

fn check_const_memory(allocation: &Allocation) -> Result<(), EncodeDecline> {
    if allocation.mutability != mir::Mutability::Not {
        return Err(EncodeDecline::MutableConstantMemory);
    }
    if allocation.len() as u64 > MAX_CONST_MEMORY_BYTES {
        return Err(EncodeDecline::ConstantMemoryTooLarge);
    }
    Ok(())
}
