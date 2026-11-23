//! `TypeDescriptor::of(tcx, ty)`: every tcx-derived fact the JavaScript lowering asks of a type,
//! evaluated once per type per codegen unit and stored in the unit's crate-scope
//! [`TypeTable`](rcgjs_object::jir::types::TypeTable).
//!
//! The answers are the ones v1's `value.rs`/`ptr.rs`/`names.rs`/`vtable.rs`/`coroutine_layout`/
//! `fn_sig` queries give — this module calls those very functions rather than reimplementing them,
//! so a descriptor can never disagree with the baseline lowering about a type.
//!
//! A descriptor is inserted as an [`TypeKind::Opaque`] placeholder before its components are
//! walked, so a recursive nominal type (`struct Node { next: Option<Box<Node>> }`) terminates, and
//! replaced by the finished descriptor afterwards. Every id inserted on behalf of one item is
//! recorded so a failed item can roll them back; a placeholder must never survive an item.

use rustc_abi::{FieldIdx, FieldsShape, Size, TagEncoding, VariantIdx, Variants};
use rustc_hir::def_id::DefId;
use rustc_middle::ty::layout::{LayoutCx, TyAndLayout};
use rustc_middle::ty::{self, Ty, TyCtxt};

use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::interns::{Interns, SigId, StrId};
use rcgjs_object::jir::semantic::{DefinitionId, DirectItemRef};
use rcgjs_object::jir::types::{
    AllocatorIdentity, EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, IteratorIdentity,
    JsFacts, LayoutFacts, Niche, NominalAbi, NominalDiscriminant, NominalForm, PinnedBoundary,
    PointerForm, RepresentationCandidate, SigAbi, SigDescriptor, TransparentField, TypeDescriptor,
    TypeId, TypeKind, TypeMeta, VariantDescriptor,
};

use crate::value::{self, typing_env};

/// The state one type-encoding session shares with the body encoder.
pub(crate) struct TypeCx<'a, 'tcx> {
    pub(crate) tcx: TyCtxt<'tcx>,
    /// The codegen unit, for the namer: a `FnDef` type carries the JavaScript name of the item
    /// it resolves to.
    pub(crate) cgu: &'a crate::cgu::CguCx<'tcx>,
    pub(crate) interns: &'a mut Interns,
    /// Every type id this session inserted, for rollback on failure.
    pub(crate) inserted: Vec<TypeId>,
}

impl<'a, 'tcx> TypeCx<'a, 'tcx> {
    pub(crate) fn new(cgu: &'a crate::cgu::CguCx<'tcx>, interns: &'a mut Interns) -> Self {
        Self {
            tcx: cgu.tcx,
            cgu,
            interns,
            inserted: Vec::new(),
        }
    }

    /// Records the type's printed Rust path on its descriptor, which is what
    /// `core::any::type_name` returns (v1 `intrinsics.rs`,
    /// `with_no_trimmed_paths!(format!("{ty}"))`).
    ///
    /// Called only where a body asks for one — the `type_name` intrinsic — so the strings are
    /// paid for by the programs that read them.
    pub(crate) fn record_type_name(&mut self, ty: Ty<'tcx>) -> Result<(), EncodeDecline> {
        let id = self.type_id(ty)?;
        let Some(descriptor) = self.interns.types.get(id) else {
            return Ok(());
        };
        if descriptor.name.is_some() {
            return Ok(());
        }
        let mut descriptor = descriptor.clone();
        let name = type_name(ty);
        descriptor.name = Some(self.str(&name));
        self.interns.types.replace(descriptor);
        Ok(())
    }

    /// The `FnOnce::call_once` shim a `ClosureFnPointer` coercion of this closure names
    /// (v1 `rvalue.rs`: `Instance::resolve_closure(.., ClosureKind::FnOnce)`).
    ///
    /// Only a non-capturing closure coerces, but the shim is cheap to resolve and the coercion is
    /// not visible from the type, so it is recorded for every closure.
    fn closure_fn_pointer_shim(
        &mut self,
        definition: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Option<DirectItemRef> {
        let tcx = self.tcx;
        let instance =
            ty::Instance::resolve_closure(tcx, definition, arguments, ty::ClosureKind::FnOnce);
        let item = super::encode::item_id(self.cgu, instance);
        let name = self.cgu.namer.fn_name(instance).into_string();
        Some(DirectItemRef {
            item,
            link_name: self.str(&name),
        })
    }

    /// The item a `FnDef` type resolves to, with its emitted JavaScript name
    /// (v1 `rvalue.rs`'s `ReifyFnPointer` arm and `intrinsics.rs::const_eval_select`).
    fn fn_def_instance(
        &mut self,
        definition: DefId,
        arguments: ty::Binder<'tcx, ty::GenericArgsRef<'tcx>>,
    ) -> Option<DirectItemRef> {
        let tcx = self.tcx;
        let arguments = arguments.no_bound_vars()?;
        let instance = ty::Instance::try_resolve(tcx, typing_env(), definition, arguments)
            .ok()
            .flatten()?;
        let item = super::encode::item_id(self.cgu, instance);
        let name = self.cgu.namer.fn_name(instance).into_string();
        Some(DirectItemRef {
            item,
            link_name: self.str(&name),
        })
    }

    /// Removes every descriptor this session inserted. Called when the item fails to encode.
    pub(crate) fn rollback(&mut self) {
        for id in std::mem::take(&mut self.inserted) {
            self.interns.types.remove(id);
        }
    }

    pub(crate) fn str(&mut self, value: &str) -> StrId {
        self.interns.intern_str(value)
    }

    /// The id of `ty`, describing it (and everything it mentions) on first use.
    ///
    /// `ty` must already be monomorphic and normalized.
    pub(crate) fn type_id(&mut self, ty: Ty<'tcx>) -> Result<TypeId, EncodeDecline> {
        let tcx = self.tcx;
        let id = TypeId(tcx.type_id_hash(ty).as_u128());
        if self.interns.has_type(id) {
            return Ok(id);
        }
        // Insert first so recursive nominal arguments cannot recurse forever.
        self.interns.intern_type(placeholder(id));
        self.inserted.push(id);

        let kind = self.kind(ty)?;
        let layout = self.layout_facts(ty);
        let meta = type_meta(tcx, ty);
        let js = self.js_facts(ty, &kind)?;
        self.interns.types.replace(TypeDescriptor {
            id,
            // Filled on demand by `record_type_name`: a printed path is a large string and only
            // the `type_name` intrinsic ever reads one, so paying for every type would put ~18 %
            // on section S1 for a fact almost nothing asks (see the workstream report).
            name: None,
            kind,
            layout,
            meta,
            js,
        });
        Ok(id)
    }

    fn kind(&mut self, ty: Ty<'tcx>) -> Result<TypeKind, EncodeDecline> {
        let tcx = self.tcx;
        Ok(match ty.kind() {
            ty::Bool => TypeKind::Bool,
            ty::Char => TypeKind::Char,
            ty::Int(_) | ty::Uint(_) => {
                let (signed, bits) = value::int_info(tcx, ty)
                    .ok_or_else(|| EncodeDecline::IntegerWidth { ty: type_name(ty) })?;
                TypeKind::Integer { signed, bits }
            }
            ty::Float(ty::FloatTy::F16) => TypeKind::Float { bits: 16 },
            ty::Float(ty::FloatTy::F32) => TypeKind::Float { bits: 32 },
            ty::Float(ty::FloatTy::F64) => TypeKind::Float { bits: 64 },
            ty::Float(ty::FloatTy::F128) => TypeKind::Float { bits: 128 },
            ty::Str => TypeKind::Str,
            ty::Never => TypeKind::Never,
            ty::Tuple(elements) => TypeKind::Tuple(
                elements
                    .iter()
                    .map(|element| self.type_id(element))
                    .collect::<Result<_, _>>()?,
            ),
            ty::Array(element, length) => TypeKind::Array {
                element: self.type_id(*element)?,
                length: length
                    .try_to_target_usize(tcx)
                    .ok_or_else(|| EncodeDecline::ArrayLength { ty: type_name(ty) })?,
            },
            ty::Slice(element) => TypeKind::Slice(self.type_id(*element)?),
            ty::Ref(_, pointee, mutable) => TypeKind::Reference {
                pointee: self.type_id(*pointee)?,
                mutable: mutable.is_mut(),
            },
            ty::RawPtr(pointee, mutable) => TypeKind::RawPointer {
                pointee: self.type_id(*pointee)?,
                mutable: mutable.is_mut(),
            },
            ty::Adt(definition, arguments) => self.adt_kind(ty, *definition, arguments)?,
            ty::Foreign(definition) => TypeKind::Foreign(definition_id(tcx, *definition)),
            ty::FnDef(definition, arguments) => {
                let sig = self.sig(ty.fn_sig(tcx), Some(*definition))?;
                let instance = self.fn_def_instance(*definition, *arguments);
                TypeKind::FnDef {
                    def: definition_id(tcx, *definition),
                    sig,
                    instance,
                }
            }
            ty::FnPtr(..) => {
                let sig = self.sig(ty.fn_sig(tcx), None)?;
                TypeKind::FnPtr { sig }
            }
            ty::Dynamic(predicates, _) => TypeKind::Dynamic {
                principal: predicates
                    .principal_def_id()
                    .map(|principal| definition_id(tcx, principal)),
                auto: predicates
                    .auto_traits()
                    .map(|auto| definition_id(tcx, auto))
                    .collect(),
            },
            ty::Closure(definition, arguments) => TypeKind::Closure {
                def: definition_id(tcx, *definition),
                upvars: self.upvars(arguments.as_closure().upvar_tys())?,
                fn_pointer_shim: self.closure_fn_pointer_shim(*definition, arguments),
            },
            ty::CoroutineClosure(definition, arguments) => TypeKind::CoroutineClosure {
                def: definition_id(tcx, *definition),
                upvars: self.upvars(arguments.as_coroutine_closure().upvar_tys())?,
            },
            ty::Coroutine(definition, arguments) => self.coroutine_kind(*definition, arguments)?,
            ty::Pat(base, _) => TypeKind::Pattern(self.type_id(*base)?),
            ty::Alias(..)
            | ty::Param(_)
            | ty::Bound(..)
            | ty::Placeholder(_)
            | ty::Infer(_)
            | ty::Error(_)
            | ty::CoroutineWitness(..)
            | ty::UnsafeBinder(_) => TypeKind::Opaque,
        })
    }

    fn upvars(&mut self, upvars: &'tcx ty::List<Ty<'tcx>>) -> Result<Vec<TypeId>, EncodeDecline> {
        upvars.iter().map(|upvar| self.type_id(upvar)).collect()
    }

    /// The signature descriptor of `sig`, interned. `definition` is the `FnDef` the signature
    /// belongs to, when it has one, for its `#[track_caller]` and unwind facts.
    fn sig(
        &mut self,
        sig: ty::PolyFnSig<'tcx>,
        definition: Option<DefId>,
    ) -> Result<SigId, EncodeDecline> {
        let tcx = self.tcx;
        let sig = tcx.normalize_erasing_late_bound_regions(typing_env(), sig);
        let inputs = sig
            .inputs()
            .iter()
            .map(|input| self.type_id(*input))
            .collect::<Result<Vec<_>, _>>()?;
        let output = self.type_id(sig.output())?;
        let abi = match sig.abi() {
            rustc_abi::ExternAbi::Rust => SigAbi::Rust,
            rustc_abi::ExternAbi::RustCall => SigAbi::RustCall,
            rustc_abi::ExternAbi::C { .. } => SigAbi::C,
            other => SigAbi::Other(self.str(other.as_str())),
        };
        let spread_arity = match (sig.abi(), sig.inputs().last()) {
            (rustc_abi::ExternAbi::RustCall, Some(last)) => match last.kind() {
                ty::Tuple(elements) => {
                    Some(u16::try_from(elements.len()).map_err(|_| EncodeDecline::SpreadTooLarge)?)
                }
                _ => None,
            },
            _ => None,
        };
        let track_caller = definition.is_some_and(|definition| {
            tcx.codegen_fn_attrs(definition)
                .flags
                .contains(rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags::TRACK_CALLER)
        });
        let unwind = rustc_middle::ty::layout::fn_can_unwind(tcx, definition, sig.abi());
        Ok(self.interns.intern_sig(SigDescriptor {
            inputs,
            output,
            abi,
            spread_arity,
            track_caller,
            unwind,
        }))
    }

    fn adt_kind(
        &mut self,
        ty: Ty<'tcx>,
        definition: ty::AdtDef<'tcx>,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeKind, EncodeDecline> {
        let tcx = self.tcx;
        let args = arguments
            .iter()
            .filter_map(|argument| argument.as_type())
            .map(|argument| self.type_id(argument))
            .collect::<Result<_, _>>()?;
        let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok();
        let direct_tag_size = layout.and_then(|layout| match &layout.variants {
            Variants::Multiple {
                tag,
                tag_encoding: TagEncoding::Direct,
                ..
            } if definition.is_enum() => Some(tag.size(&tcx)),
            _ => None,
        });
        let form = if definition.is_struct() {
            NominalForm::Struct
        } else if definition.is_enum() {
            NominalForm::Enum
        } else if definition.is_union() {
            NominalForm::Union
        } else {
            NominalForm::Opaque
        };
        let layout_cx = LayoutCx::new(tcx, typing_env());
        let mut variants = Vec::with_capacity(definition.variants().len());
        for (variant_index, variant) in definition.variants().iter_enumerated() {
            let variant_layout = layout.map(|layout| {
                if definition.is_enum() {
                    layout.for_variant(&layout_cx, variant_index)
                } else {
                    layout
                }
            });
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field_index, field) in variant.fields.iter_enumerated() {
                let field_ty =
                    tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, arguments));
                let key = self.field_key(
                    ty,
                    definition.is_enum().then_some(variant_index),
                    field_index,
                );
                fields.push(FieldDescriptor {
                    key,
                    ty: self.type_id(field_ty)?,
                    offset: field_offset(variant_layout, field_index),
                });
            }
            let (logical_discr, physical_tag) = if definition.is_enum() {
                let discriminant =
                    ty.discriminant_for_variant(tcx, variant_index)
                        .ok_or_else(|| EncodeDecline::VariantDiscriminant {
                            variant: variant_index.as_u32(),
                            ty: type_name(ty),
                        })?;
                let logical = match value::int_info(tcx, discriminant.ty) {
                    Some((signed, bits)) => value::resign(signed, bits, discriminant.val),
                    None => discriminant.val as i128,
                };
                let physical = match direct_tag_size {
                    Some(tag_size) => Some(
                        physical_direct_discriminant(discriminant.val, tag_size)
                            .ok_or_else(|| EncodeDecline::EnumTagWidth { ty: type_name(ty) })?,
                    ),
                    None => None,
                };
                (logical, physical)
            } else {
                (0, None)
            };
            variants.push(VariantDescriptor {
                name: self.str(variant.name.as_str()),
                fields,
                logical_discr,
                physical_tag,
                result_ok: tcx.lang_items().result_ok_variant() == Some(variant.def_id),
                saved_local_keys: Vec::new(),
            });
        }
        let niche = layout.and_then(|layout| niche_facts(tcx, layout));
        Ok(TypeKind::Adt {
            def: definition_id(tcx, definition.did()),
            args,
            form,
            variants,
            niche,
        })
    }

    fn coroutine_kind(
        &mut self,
        definition: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeKind, EncodeDecline> {
        let tcx = self.tcx;
        let upvars = self.upvars(arguments.as_coroutine().upvar_tys())?;
        let (saved_locals, states, variants) = match tcx.coroutine_layout(definition, arguments) {
            Ok(layout) => {
                let mut saved_locals = Vec::with_capacity(layout.field_tys.len());
                for (index, saved) in layout.field_tys.iter_enumerated() {
                    let saved_ty = tcx.normalize_erasing_regions(
                        typing_env(),
                        ty::EarlyBinder::bind(tcx, saved.ty).instantiate(tcx, arguments),
                    );
                    saved_locals.push((index.as_u32(), self.type_id(saved_ty)?));
                }
                let mut states = Vec::with_capacity(layout.variant_fields.len());
                let mut variants = Vec::with_capacity(layout.variant_fields.len());
                for state_fields in layout.variant_fields.iter() {
                    let mut state = Vec::with_capacity(state_fields.len());
                    let mut fields = Vec::with_capacity(state_fields.len());
                    let mut keys = Vec::with_capacity(state_fields.len());
                    for saved in state_fields.iter() {
                        let index = saved.as_u32();
                        let key = self.str(&format!("$s{index}"));
                        state.push(index);
                        keys.push(key);
                        fields.push(FieldDescriptor {
                            key: FieldKey::Name(key),
                            ty: saved_locals[index as usize].1,
                            offset: 0,
                        });
                    }
                    states.push(state);
                    variants.push(VariantDescriptor {
                        name: self.str(""),
                        fields,
                        logical_discr: variants.len() as i128,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: keys,
                    });
                }
                (saved_locals, states, variants)
            }
            // A coroutine tainted by errors has no layout; it still describes as one with no
            // saved locals so the body that mentions it encodes.
            Err(_) => (Vec::new(), Vec::new(), Vec::new()),
        };
        Ok(TypeKind::Coroutine {
            def: definition_id(tcx, definition),
            upvars,
            saved_locals,
            states,
            variants,
        })
    }

    /// v1's `value::field_key`, as the schema spells it.
    fn field_key(
        &mut self,
        base: Ty<'tcx>,
        variant: Option<VariantIdx>,
        field: FieldIdx,
    ) -> FieldKey {
        match value::field_key(self.tcx, base, variant, field) {
            Some(value::FieldKey::Name(name)) => FieldKey::Name(self.str(&name)),
            Some(value::FieldKey::Index(index)) => FieldKey::Index(index as u32),
            Some(value::FieldKey::Transparent) => FieldKey::Transparent,
            // `field_key` answers `None` for a type with no fields to name; a descriptor field
            // never reaches this arm, and the answer that keeps the value model total is the
            // one a zero-sized aggregate gets.
            None => FieldKey::Transparent,
        }
    }

    fn layout_facts(&self, ty: Ty<'tcx>) -> LayoutFacts {
        let tcx = self.tcx;
        let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok();
        let sized = layout.is_some_and(|layout| layout.is_sized());
        LayoutFacts {
            size: layout.filter(|_| sized).map(|layout| layout.size.bytes()),
            // For an unsized type `layout.size` is where the tail begins — the sized prefix —
            // which is what `size_of_val` adds the measured tail to (v1 `intrinsics.rs`).
            unsized_prefix: layout.filter(|_| !sized).map(|layout| layout.size.bytes()),
            align: layout.map(|layout| layout.align.abi.bytes()),
            sized: ty.is_sized(tcx, typing_env()),
            zst: value::is_zst(tcx, ty),
            one_zst: layout.is_some_and(|layout| layout.is_1zst()),
            needs_drop: ty.needs_drop(tcx, typing_env()),
            has_own_drop: matches!(
                ty.kind(),
                ty::Adt(definition, _)
                    if definition.has_dtor(tcx) || definition.async_destructor(tcx).is_some()
            ),
        }
    }

    fn js_facts(&mut self, ty: Ty<'tcx>, kind: &TypeKind) -> Result<JsFacts, EncodeDecline> {
        let tcx = self.tcx;
        let peeled_ty = value::peel_pattern(value::peel_transparent(tcx, ty));
        let peeled = self.type_id(peeled_ty)?;
        let int = value::int_info(tcx, peeled_ty).map(|(signed, bits)| IntFacts {
            signed,
            bits,
            repr: match value::int_repr(peeled_ty) {
                value::IntRepr::Number => IntRepr::Number,
                value::IntRepr::BigInt => IntRepr::BigInt,
            },
        });
        let transparent = match value::transparent_field_indexed(tcx, ty) {
            Some((field, field_ty)) => Some(TransparentField {
                field: field.as_u32(),
                ty: self.type_id(field_ty)?,
            }),
            None => None,
        };
        let unsized_tail = match value::unsized_tail_indexed(tcx, ty) {
            Some((field, tail_ty)) => Some(TransparentField {
                field: field.as_u32(),
                ty: self.type_id(tail_ty)?,
            }),
            None => None,
        };
        let enum_repr = value::enum_repr(tcx, ty).map(|repr| match repr {
            value::EnumRepr::Number => {
                let (bits, signed) = value::direct_tag(tcx, ty).unwrap_or((0, false));
                EnumRepr::Number {
                    bits: bits as u32,
                    signed,
                }
            }
            value::EnumRepr::Tag { fieldless } => EnumRepr::Tag { fieldless },
            value::EnumRepr::State => EnumRepr::State,
        });
        let pointer_form = match ty.kind() {
            ty::Ref(_, pointee, mutbl) => Some(self.pointer_form(*pointee, false, mutbl.is_mut())),
            ty::RawPtr(pointee, mutbl) => Some(self.pointer_form(*pointee, true, mutbl.is_mut())),
            _ => None,
        };
        let fields = self.aggregate_fields(ty, kind)?;
        Ok(JsFacts {
            int,
            is_indirect: value::is_indirect(tcx, ty),
            needs_clone: value::needs_clone(tcx, ty),
            crosses_transparent_union: value::crosses_transparent_union(tcx, ty),
            transparent,
            peeled,
            unsized_tail,
            enum_repr,
            is_primitive_enum: value::is_primitive_enum(tcx, ty),
            pointer_form,
            byte_element: crate::ptr::ByteElement::of(tcx, ty).is_some(),
            fields,
        })
    }

    /// The JavaScript shape of a pointer to `pointee` (`ptr.rs`).
    ///
    /// Shared references `&str` are JavaScript strings (`PointerForm::Str`), while `&mut str` and
    /// every raw `str` pointer (`*const str`, `*mut str`, and transparent wrappers over them such
    /// as `NonNull<str>`, `Unique<str>`, `Box<str>`) are live `{ buf, off, len }` UTF-8 byte
    /// slices (`PointerForm::FatSlice`) aliasing their backing buffer.
    fn pointer_form(&self, pointee: Ty<'tcx>, raw: bool, mutable: bool) -> PointerForm {
        let tcx = self.tcx;
        // A transparent wrapper over an unsized type (`CStr` over `[c_char]`) is that type.
        let peeled = value::peel_pattern(value::peel_transparent(tcx, pointee));
        match peeled.kind() {
            ty::Str => {
                return if raw || mutable {
                    PointerForm::FatSlice
                } else {
                    PointerForm::Str
                };
            }
            ty::Slice(_) => return PointerForm::FatSlice,
            ty::Dynamic(..) => return PointerForm::FatDyn,
            _ => {}
        }
        if value::unsized_tail(tcx, pointee).is_some() {
            return PointerForm::FatTail;
        }
        if raw {
            if crate::ptr::is_slot_pointee(tcx, pointee) {
                PointerForm::Slot
            } else {
                PointerForm::Thin
            }
        } else if value::is_indirect(tcx, pointee) {
            PointerForm::Thin
        } else {
            PointerForm::Slot
        }
    }

    /// `JsFacts::fields`: a struct/union's one variant, a tuple's elements, an array's element,
    /// a closure's or coroutine's upvars.
    fn aggregate_fields(
        &mut self,
        ty: Ty<'tcx>,
        kind: &TypeKind,
    ) -> Result<Vec<FieldDescriptor>, EncodeDecline> {
        let tcx = self.tcx;
        Ok(match kind {
            TypeKind::Adt {
                form: NominalForm::Struct | NominalForm::Union,
                variants,
                ..
            } => variants
                .first()
                .map(|variant| variant.fields.clone())
                .unwrap_or_default(),
            TypeKind::Tuple(elements) => {
                let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok();
                elements
                    .iter()
                    .enumerate()
                    .map(|(index, element)| FieldDescriptor {
                        key: self.index_key(ty, index),
                        ty: *element,
                        offset: field_offset(layout, FieldIdx::from_usize(index)),
                    })
                    .collect()
            }
            TypeKind::Array { element, .. } => vec![FieldDescriptor {
                key: FieldKey::Index(0),
                ty: *element,
                offset: 0,
            }],
            TypeKind::Closure { upvars, .. }
            | TypeKind::CoroutineClosure { upvars, .. }
            | TypeKind::Coroutine { upvars, .. } => {
                let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok();
                upvars
                    .iter()
                    .enumerate()
                    .map(|(index, upvar)| FieldDescriptor {
                        key: self.index_key(ty, index),
                        ty: *upvar,
                        offset: field_offset(layout, FieldIdx::from_usize(index)),
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
    }

    /// The key of positional field `index` of `ty`: `Index`, unless the aggregate is zero-sized
    /// (every field is then `Transparent`, as `field_key` answers).
    fn index_key(&self, ty: Ty<'tcx>, index: usize) -> FieldKey {
        if value::is_zst(self.tcx, ty) {
            FieldKey::Transparent
        } else {
            FieldKey::Index(index as u32)
        }
    }
}

/// The placeholder inserted before a type's components are walked.
fn placeholder(id: TypeId) -> TypeDescriptor {
    TypeDescriptor {
        id,
        name: None,
        kind: TypeKind::Opaque,
        layout: LayoutFacts::default(),
        meta: TypeMeta::default(),
        js: JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: id,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        },
    }
}

/// The offset of field `index` in `layout`, or `0` when there is no layout to ask.
fn field_offset(layout: Option<TyAndLayout<'_>>, index: FieldIdx) -> u64 {
    let Some(layout) = layout else {
        return 0;
    };
    match &layout.fields {
        FieldsShape::Arbitrary { offsets, .. } => {
            offsets.get(index).map_or(0, |offset| offset.bytes())
        }
        FieldsShape::Array { stride, count } if (index.as_u32() as u64) < *count => {
            stride.bytes() * index.as_u32() as u64
        }
        FieldsShape::Primitive | FieldsShape::Union(_) | FieldsShape::Array { .. } => 0,
    }
}

/// The exact bits a logical discriminant occupies in a direct tag of `size` bytes.
pub(crate) fn physical_direct_discriminant(value: u128, size: Size) -> Option<NominalDiscriminant> {
    let bytes = u8::try_from(size.bytes()).ok()?;
    if bytes == 0 || bytes > 16 {
        return None;
    }
    Some(NominalDiscriminant {
        bits: size.truncate(value),
        bytes,
    })
}

/// The niche facts of a `Variants::Multiple` layout with a niche tag encoding.
fn niche_facts<'tcx>(tcx: TyCtxt<'tcx>, layout: TyAndLayout<'tcx>) -> Option<Niche> {
    let Variants::Multiple {
        tag,
        tag_encoding:
            TagEncoding::Niche {
                untagged_variant,
                niche_variants: _,
                niche_start,
            },
        tag_field,
        ..
    } = &layout.variants
    else {
        return None;
    };
    let tag_offset = layout.fields.offset(tag_field.as_usize());
    let layout_cx = LayoutCx::new(tcx, typing_env());
    let untagged = layout.for_variant(&layout_cx, *untagged_variant);
    let mut path = Vec::new();
    let mut current = untagged;
    let mut current_offset = Size::ZERO;
    // Walk down the untagged variant's fields to the scalar holding the niche. Bounded: a type
    // cannot nest deeper than its size in bytes, but a malformed layout must not spin.
    for _ in 0..64 {
        // A scalar is the end of the walk; a nested multi-variant layout has no field the niche
        // sits in from this model's point of view, so the path stops at it.
        if matches!(current.fields, FieldsShape::Primitive)
            || matches!(current.variants, Variants::Multiple { .. })
        {
            break;
        }
        let count = current.fields.count();
        let mut next = None;
        for index in 0..count {
            let offset = current_offset + current.fields.offset(index);
            let field = current.field(&layout_cx, index);
            let size = field.size;
            if size.bytes() > 0 && offset <= tag_offset && tag_offset < offset + size {
                next = Some((index, offset, field));
                break;
            }
        }
        let Some((index, offset, field)) = next else {
            break;
        };
        path.push(index as u32);
        current_offset = offset;
        current = field;
    }
    let range = tag.valid_range(&tcx);
    Some(Niche {
        untagged_variant: untagged_variant.as_u32(),
        niche_field: path,
        niche_start: *niche_start,
        valid_range: (range.start, range.end),
    })
}

/// v1's `late_ir::type_metadata`, as the schema's [`TypeMeta`].
pub(crate) fn type_meta<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> TypeMeta {
    let old = crate::late_ir::type_metadata(tcx, ty);
    TypeMeta {
        candidate: match old.candidate {
            crate::late_ir::RepresentationCandidate::None => RepresentationCandidate::None,
            crate::late_ir::RepresentationCandidate::Box => RepresentationCandidate::Box,
            crate::late_ir::RepresentationCandidate::Vec => RepresentationCandidate::Vec,
            crate::late_ir::RepresentationCandidate::String => RepresentationCandidate::String,
            crate::late_ir::RepresentationCandidate::BTreeMap => RepresentationCandidate::BTreeMap,
        },
        abi: match old.abi {
            crate::late_ir::NominalAbi::Rust => NominalAbi::Rust,
            crate::late_ir::NominalAbi::C => NominalAbi::C,
            crate::late_ir::NominalAbi::Transparent => NominalAbi::Transparent,
        },
        boundary: match old.boundary {
            crate::late_ir::PinnedBoundary::None => PinnedBoundary::None,
            crate::late_ir::PinnedBoundary::Host => PinnedBoundary::Host,
            crate::late_ir::PinnedBoundary::ViewAbi => PinnedBoundary::ViewAbi,
        },
        allocator: match old.allocator {
            crate::late_ir::AllocatorIdentity::NotApplicable => AllocatorIdentity::NotApplicable,
            crate::late_ir::AllocatorIdentity::Global => AllocatorIdentity::Global,
            crate::late_ir::AllocatorIdentity::Custom => AllocatorIdentity::Custom,
            crate::late_ir::AllocatorIdentity::Unknown => AllocatorIdentity::Unknown,
        },
        iterator: iterator_identity(tcx, ty),
    }
}

fn iterator_identity(tcx: TyCtxt<'_>, ty: Ty<'_>) -> IteratorIdentity {
    let ty::Adt(definition, _) = ty.kind() else {
        return IteratorIdentity::None;
    };
    if let Some(name) = tcx.get_diagnostic_name(definition.did()) {
        return match name.as_str() {
            "SliceIter" => IteratorIdentity::Slice,
            "Enumerate" => IteratorIdentity::Enumerate,
            "Take" => IteratorIdentity::Take,
            "Skip" => IteratorIdentity::Skip,
            _ => IteratorIdentity::None,
        };
    }
    // `core::slice::IterMut` has no `#[rustc_diagnostic_item]` on this toolchain,
    // so identify it unforgeably as the `IterMut` sibling inside `SliceIter`'s `core` module.
    if let Some(slice_iter) = tcx.get_diagnostic_item(rustc_span::Symbol::intern("SliceIter")) {
        if tcx.parent(definition.did()) == tcx.parent(slice_iter)
            && tcx.item_name(definition.did()).as_str() == "IterMut"
        {
            return IteratorIdentity::SliceMut;
        }
    }
    IteratorIdentity::None
}

pub(crate) fn definition_id(tcx: TyCtxt<'_>, definition: DefId) -> DefinitionId {
    let hash = tcx.def_path_hash(definition);
    DefinitionId {
        crate_id: hash.stable_crate_id().as_u64(),
        local_hash: hash.local_hash().as_u64(),
    }
}

/// The name of a type for a decline message, spelled with untrimmed paths.
pub(crate) fn type_name(ty: Ty<'_>) -> String {
    rustc_middle::ty::print::with_no_trimmed_paths!(ty.to_string())
}
