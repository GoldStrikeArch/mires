//! MIR -> JIR-S: the tcx side of the schema in `rcgjs-object`.
//!
//! [`EncodeCore`] holds the shared encoding state used by functions, statics, and vtables, while
//! [`FnEncoder`] wraps it with the concrete [`Instance`] and [`mir::Body`] needed for function
//! encoding so static and vtable encoders can never accidentally touch MIR.
//!
//! Every produced [`Body`] is verified against the codegen unit's crate-scope [`Interns`] before
//! being returned; a verification failure produces [`EncodeDecline::Invalid`], never a panic.

mod call;
mod constant;
mod mir;
mod origins;

use std::collections::BTreeSet;

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir as rustc_mir;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, ExistentialTraitRef, Instance, Ty, TyCtxt, TypeFoldable, VtblEntry};
use rustc_span::Span;

use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::interns::{Interns, StrId};
use rcgjs_object::jir::origin::{OriginId, OriginKind, OriginTable, SourceFactId};
use rcgjs_object::jir::semantic::{
    AbiSignature, Body, BodyKind, BodySemantic, ConstTree, DefinitionId, DirectItemRef, ItemId,
    Signature, TypeId, VtableEntry,
};

use super::types::{definition_id, type_name, TypeCx};
use super::JirItem;
use crate::cgu::CguCx;
use crate::item::JsName;
use crate::value::typing_env;

use self::constant::MAX_CONST_MEMORY_BYTES;
use self::origins::{local_names, OriginRecorder};

pub(crate) use self::origins::location_of;

/// The encoded JIR item together with any auxiliary link-level item references (vtables,
/// `#[js_extern]` imports, panic lang items, reified function items, host closure callbacks)
/// discovered while walking MIR.
pub(crate) struct EncodedJir {
    pub(crate) item: JirItem,
    pub(crate) extra_refs: BTreeSet<JsName>,
}

type PendingVtable<'tcx> = (Ty<'tcx>, Option<ExistentialTraitRef<'tcx>>);

/// Whether the debug-only self checks (postcard round trip of every body) run.
fn self_check() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("JSC_JIR_SELFCHECK").is_some_and(|value| value == "1")
}

/// Encodes a monomorphic function body.
pub(crate) fn function<'tcx>(
    cgu: &CguCx<'tcx>,
    instance: Instance<'tcx>,
    mir: &rustc_mir::Body<'tcx>,
) -> Result<EncodedJir, EncodeDecline> {
    let (result, pending_vtables) = {
        let mut interns = cgu.interns_mut();
        let mut encoder = FnEncoder::new(cgu, &mut interns, instance, mir);
        let body = encoder.function_body();
        encoder.finish_encoded(body)
    };
    for (ty, principal) in pending_vtables {
        let _ = crate::vtable::vtable_expr(cgu, ty, principal);
    }
    result
}

/// Encodes a `static`: its identity, type, and decoded initializer.
pub(crate) fn static_item<'tcx>(
    cgu: &CguCx<'tcx>,
    def_id: DefId,
) -> Result<EncodedJir, EncodeDecline> {
    let (result, pending_vtables) = {
        let mut interns = cgu.interns_mut();
        let mut encoder = EncodeCore::new(cgu, &mut interns);
        let body = encoder.static_body(def_id);
        encoder.finish_encoded(body)
    };
    for (ty, principal) in pending_vtables {
        let _ = crate::vtable::vtable_expr(cgu, ty, principal);
    }
    result
}

/// Encodes the vtable of `ty as dyn trait_ref` (`vtable.rs` builds the JavaScript array).
pub(crate) fn vtable<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    trait_ref: Option<ExistentialTraitRef<'tcx>>,
) -> Result<JirItem, EncodeDecline> {
    let mut interns = cgu.interns_mut();
    let mut encoder = EncodeCore::new(cgu, &mut interns);
    let result = encoder.vtable_body(ty, trait_ref);
    encoder.finish(result)
}

/// Encoding state shared by functions, statics, and vtables.
struct EncodeCore<'a, 'tcx> {
    cgu: &'a CguCx<'tcx>,
    types: TypeCx<'a, 'tcx>,
    /// Extra item references discovered during encoding (panic lang items, runtime `Unsize`
    /// vtables, `#[js_extern]` imports, reified function items, host closure bodies).
    extra_refs: BTreeSet<JsName>,
    /// Vtables to intern into `cgu` after releasing `cgu.interns_mut()`.
    pending_vtables: Vec<PendingVtable<'tcx>>,
    origins: OriginRecorder,
}

impl<'a, 'tcx> EncodeCore<'a, 'tcx> {
    fn new(cgu: &'a CguCx<'tcx>, interns: &'a mut Interns) -> Self {
        Self {
            cgu,
            types: TypeCx::new(cgu, interns),
            extra_refs: BTreeSet::new(),
            pending_vtables: Vec::new(),
            origins: OriginRecorder::new(),
        }
    }

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.cgu.tcx
    }

    fn direct_item(&mut self, instance: Instance<'tcx>) -> DirectItemRef {
        DirectItemRef {
            item: item_id(self.cgu, instance),
            link_name: self.types.str(self.cgu.namer.fn_name(instance).as_str()),
        }
    }

    /// Normalizes an already-monomorphized type and interns its descriptor.
    fn type_id(&mut self, ty: Ty<'tcx>) -> Result<TypeId, EncodeDecline> {
        let ty = self
            .tcx()
            .normalize_erasing_regions(typing_env(), ty::Unnormalized::new_wip(ty));
        self.types.type_id(ty)
    }

    fn source_fact(&mut self, span: Span, mir_span: Option<Span>) -> Option<SourceFactId> {
        origins::source_fact(&mut self.types, span, mir_span)
    }

    fn finish_encoded(
        mut self,
        result: Result<Body, EncodeDecline>,
    ) -> (Result<EncodedJir, EncodeDecline>, Vec<PendingVtable<'tcx>>) {
        let extra_refs = std::mem::take(&mut self.extra_refs);
        let pending_vtables = std::mem::take(&mut self.pending_vtables);
        match self.finish(result) {
            Ok(item) => (Ok(EncodedJir { item, extra_refs }), pending_vtables),
            Err(decline) => (Err(decline), Vec::new()),
        }
    }

    /// Verifies the body and its origins, rolls the type table back on failure, and runs the
    /// debug-only postcard round trip.
    fn finish(mut self, result: Result<Body, EncodeDecline>) -> Result<JirItem, EncodeDecline> {
        let body = match result {
            Ok(body) => body,
            Err(decline) => {
                self.types.rollback();
                return Err(decline);
            }
        };
        let origins = self.origins.finish(&mut self.types);
        let verified = {
            let interns: &Interns = &*self.types.interns;
            body.verify(&interns.types, interns)
                .and_then(|()| body.verify_origins(&origins))
        };
        if let Err(error) = verified {
            self.types.rollback();
            return Err(EncodeDecline::Invalid {
                message: error.message,
            });
        }
        if self_check() {
            if let Err(message) = round_trip(&body, &origins) {
                self.types.rollback();
                return Err(EncodeDecline::Codec { message });
            }
        }
        Ok(JirItem { body, origins })
    }

    fn static_body(&mut self, def_id: DefId) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        let path = self
            .types
            .str(&with_no_trimmed_paths!(tcx.def_path_str(def_id)));
        self.origins.set_item_path(path);
        let root = self.source_fact(tcx.def_span(def_id), None);
        self.origins.set_root(root);
        let ty =
            tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
        let type_id = self.type_id(ty)?;
        let init = self.static_initializer(def_id)?;
        Ok(Body {
            item: ItemId {
                definition: definition_id(tcx, def_id),
                instantiation: type_id.0,
            },
            kind: BodyKind::Static {
                init,
                mutable: tcx.is_mutable_static(def_id),
            },
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: Vec::new(),
                result: type_id,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        })
    }

    /// The evaluated initializer of a `static`, decoded at the static's type.
    ///
    /// The static's own allocation may be mutable (a `static mut`, interior mutability); that is
    /// the value it starts with, not memory the program cannot write, so only the size limit
    /// applies to it. Memory it points into is checked as any constant memory is.
    fn static_initializer(&mut self, def_id: DefId) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let ty =
            tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
        let allocation = tcx
            .eval_static_initializer(def_id)
            .map_err(|_| EncodeDecline::ConstantEvaluation { ty: type_name(ty) })?;
        let allocation = allocation.inner();
        if allocation.len() as u64 > MAX_CONST_MEMORY_BYTES {
            return Err(EncodeDecline::ConstantMemoryTooLarge);
        }
        self.const_tree(allocation, Size::ZERO, ty, 0)
    }

    fn vtable_body(
        &mut self,
        ty: Ty<'tcx>,
        trait_ref: Option<ExistentialTraitRef<'tcx>>,
    ) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        let path = self.types.str(&with_no_trimmed_paths!(match trait_ref {
            Some(trait_ref) => format!("<{ty} as {trait_ref}> vtable"),
            None => format!("<{ty}> vtable"),
        }));
        self.origins.set_item_path(path);
        let root =
            trait_ref.and_then(|trait_ref| self.source_fact(tcx.def_span(trait_ref.def_id), None));
        self.origins.set_root(root);
        let concrete = self.type_id(ty)?;
        let entries: &[VtblEntry<'tcx>] = match trait_ref {
            Some(trait_ref) => {
                let trait_ref = trait_ref.with_self_ty(tcx, ty);
                tcx.vtable_entries(tcx.erase_and_anonymize_regions(trait_ref))
            }
            None => TyCtxt::COMMON_VTABLE_ENTRIES,
        };
        let (size, align) = match tcx.layout_of(typing_env().as_query_input(ty)) {
            Ok(layout) => (layout.size.bytes(), layout.align.abi.bytes()),
            Err(_) => (0, 1),
        };
        let mut encoded = Vec::with_capacity(entries.len());
        for entry in entries {
            encoded.push(match *entry {
                VtblEntry::MetadataDropInPlace => {
                    if !ty.needs_drop(tcx, typing_env()) {
                        VtableEntry::Drop(None)
                    } else {
                        let glue = Instance::resolve_drop_glue(tcx, ty);
                        match glue.def {
                            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => {
                                VtableEntry::Drop(None)
                            }
                            _ => VtableEntry::Drop(Some(self.direct_item(glue))),
                        }
                    }
                }
                VtblEntry::MetadataSize => VtableEntry::Size(size),
                VtblEntry::MetadataAlign => VtableEntry::Align(align),
                VtblEntry::Vacant => VtableEntry::Vacant,
                VtblEntry::Method(instance) => VtableEntry::Method(self.direct_item(instance)),
                VtblEntry::TraitVPtr(super_trait_ref) => {
                    let principal = ty::ExistentialTraitRef::erase_self_ty(tcx, super_trait_ref);
                    let name = self.cgu.namer.vtable_name(ty, Some(principal));
                    VtableEntry::TraitVPtr(self.types.str(name.as_str()))
                }
            });
        }
        Ok(Body {
            item: ItemId {
                definition: trait_ref
                    .map(|trait_ref| definition_id(tcx, trait_ref.def_id))
                    .unwrap_or(DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    }),
                instantiation: concrete.0,
            },
            kind: BodyKind::Vtable {
                concrete,
                trait_ref: trait_ref.map(|trait_ref| definition_id(tcx, trait_ref.def_id)),
                entries: encoded,
            },
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: Vec::new(),
                result: concrete,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        })
    }
}

/// Function-specific encoder wrapping [`EncodeCore`] with the concrete [`Instance`] and MIR body.
struct FnEncoder<'a, 'tcx> {
    core: EncodeCore<'a, 'tcx>,
    instance: Instance<'tcx>,
    mir: &'a rustc_mir::Body<'tcx>,
}

impl<'a, 'tcx> FnEncoder<'a, 'tcx> {
    fn new(
        cgu: &'a CguCx<'tcx>,
        interns: &'a mut Interns,
        instance: Instance<'tcx>,
        mir: &'a rustc_mir::Body<'tcx>,
    ) -> Self {
        Self {
            core: EncodeCore::new(cgu, interns),
            instance,
            mir,
        }
    }

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.core.tcx()
    }

    /// Instantiates and normalizes `value` for the function being encoded.
    fn monomorphize<T>(&self, value: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>> + Copy,
    {
        self.instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx(),
            typing_env(),
            ty::EarlyBinder::bind(self.tcx(), value),
        )
    }

    fn type_id(&mut self, ty: Ty<'tcx>) -> Result<TypeId, EncodeDecline> {
        let ty = self.monomorphize(ty);
        self.core.types.type_id(ty)
    }

    fn source_fact(&mut self, span: Span) -> Option<SourceFactId> {
        self.core.source_fact(span, Some(self.mir.span))
    }

    fn origin(
        &mut self,
        kind: OriginKind,
        info: rustc_mir::SourceInfo,
        local_name: Option<StrId>,
    ) -> OriginId {
        self.core
            .origins
            .record(&mut self.core.types, self.mir, kind, info, local_name)
    }

    fn finish_encoded(
        self,
        result: Result<Body, EncodeDecline>,
    ) -> (Result<EncodedJir, EncodeDecline>, Vec<PendingVtable<'tcx>>) {
        self.core.finish_encoded(result)
    }
}

/// Postcard round trip of a body and its origins: the debug-only self check.
fn round_trip(body: &Body, origins: &OriginTable) -> Result<(), String> {
    let bytes = postcard::to_allocvec(body).map_err(|error| error.to_string())?;
    let decoded: Body = postcard::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *body {
        return Err("late-IR body does not survive a postcard round trip".to_owned());
    }
    let bytes = postcard::to_allocvec(origins).map_err(|error| error.to_string())?;
    let decoded: OriginTable = postcard::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *origins {
        return Err("late-IR origin table does not survive a postcard round trip".to_owned());
    }
    Ok(())
}

pub(crate) fn item_id<'tcx>(cgu: &CguCx<'tcx>, instance: Instance<'tcx>) -> ItemId {
    let tcx = cgu.tcx;
    ItemId {
        definition: definition_id(tcx, instance.def_id()),
        instantiation: tcx.type_id_hash(instance.ty(tcx, typing_env())).as_u128(),
    }
}

/// Decodes the `#[js_export]` `link_section` of a function item, if present.
pub(crate) fn fn_export_section<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
) -> Option<mires_marker_abi::export::ExportSection> {
    let ty::InstanceKind::Item(def_id) = instance.def else {
        return None;
    };
    if !matches!(
        tcx.def_kind(def_id),
        rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
    ) {
        return None;
    }
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    mires_marker_abi::export::parse_export_section(section.as_str())
}

/// Whether `instance` is a function tagged with marshaled `#[js_export]`.
pub(crate) fn is_js_export_fn<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> bool {
    fn_export_section(tcx, instance).is_some_and(|section| section.marshal)
}

/// Whether `instance` is an internal return-marshaling helper for `#[js_export] async fn`.
pub(crate) fn is_js_export_out_fn<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> bool {
    let ty::InstanceKind::Item(def_id) = instance.def else {
        return false;
    };
    if !matches!(
        tcx.def_kind(def_id),
        rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
    ) {
        return false;
    }
    tcx.codegen_fn_attrs(def_id)
        .link_section
        .is_some_and(|section| section.as_str() == mires_marker_abi::export::EXPORT_OUT_SECTION)
}

/// Emitted parameter names of `instance` for `.d.ts` signature synthesis.
pub(crate) fn dts_params<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    mir: &rustc_mir::Body<'tcx>,
) -> Vec<String> {
    let debug = local_names(mir);
    let mut params = Vec::new();
    for local in mir.args_iter() {
        if mir.spread_arg == Some(local) {
            let ty = instance.instantiate_mir_and_normalize_erasing_regions(
                tcx,
                typing_env(),
                ty::EarlyBinder::bind(tcx, mir.local_decls[local].ty),
            );
            if let ty::Tuple(tys) = ty.kind() {
                for i in 0..tys.len() {
                    params.push(format!("_{}$e{i}", local.as_usize()));
                }
                continue;
            }
        }
        let name = debug
            .get(local.as_usize())
            .and_then(|n| n.clone())
            .unwrap_or_else(|| format!("_{}", local.as_usize()));
        params.push(name);
    }
    if instance.def.requires_caller_location(tcx) {
        params.push("$loc".to_owned());
    }
    params
}
