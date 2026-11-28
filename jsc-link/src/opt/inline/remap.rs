//! Imports a callee's metadata without changing the provider or caller on failure.

use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::jir::interns::{BytesId, Interns, SigId, StrId};
use crate::jir::origin::{Origin, OriginId, OriginKind, OriginTable, SourceFactId};
use crate::jir::repr::{Func, OpKind, Proj, RTerm};
use crate::jir::semantic::{
    Callee, ConstTree, DirectItemRef, DropGlue, GlobalReference, HostPayload, StaticStep,
};
use crate::jir::summary::ItemSummary;
use crate::jir::types::{FieldDescriptor, FieldKey, SigAbi, TypeId, TypeKind, VariantDescriptor};
use rcgjs_object::object::Header;

use super::InlineDecline;

type Res<T> = Result<T, InlineDecline>;

#[derive(Clone)]
pub(crate) struct Metadata<'a> {
    pub universe: &'a Interns,
    pub interns: Cow<'a, Interns>,
    pub origins: Option<Cow<'a, OriginTable>>,
    pub header: Option<&'a Header>,
    pub summary: Option<Cow<'a, ItemSummary>>,
}

impl Metadata<'_> {
    pub fn compatible(&self, provider: &Self) -> bool {
        match (self.header, provider.header) {
            (Some(a), Some(b)) => {
                a.jir_schema == b.jir_schema
                    && a.rustc_version_hash == b.rustc_version_hash
                    && a.data_layout == b.data_layout
                    && a.emit_args == b.emit_args
            }
            (None, None) => std::ptr::eq(self.universe, provider.universe),
            _ => false,
        }
    }

    /// A failed import discards both the translated body and every appended table entry.
    pub fn import(&self, provider: &Self, body: &Func, call: OriginId) -> Res<(Func, Self)> {
        if !self.compatible(provider) {
            return Err(InlineDecline::ObjectIncompatible);
        }
        let mut imported = self.clone();
        let mut target = self.interns.as_ref().clone();
        target.rebuild_indices();
        let mut remap = Remap {
            source: provider.interns.as_ref(),
            target,
            pending: Vec::new(),
            seen: BTreeSet::new(),
        };
        let mut body = body.clone();
        remap.func(&mut body)?;
        remap.types()?;
        let origins = match (&self.origins, &provider.origins) {
            (Some(caller), Some(callee)) => {
                let mut caller = caller.as_ref().clone();
                let offset = caller.origins.len() as u32;
                if caller.origins.len().saturating_add(callee.origins.len()) > 65536 {
                    return Err(InlineDecline::OriginBudget);
                }
                let caller_orig_len = caller.origins.len();
                let map = |id: OriginId| -> Res<OriginId> {
                    if (id.0 as usize) < callee.origins.len() {
                        Ok(OriginId(offset + id.0))
                    } else if (call.0 as usize) < caller_orig_len {
                        Ok(call)
                    } else if caller_orig_len > 0 {
                        Ok(OriginId(0))
                    } else if id.0 == u32::MAX {
                        Ok(id)
                    } else {
                        Err(InlineDecline::OriginInvalid)
                    }
                };
                for origin in &callee.origins {
                    let mut origin: Origin = origin.clone();
                    origin.source = origin.source.map(|id| remap.fact(id)).transpose()?;
                    origin.local_name = origin.local_name.map(|id| remap.string(id)).transpose()?;
                    if let OriginKind::Generated(name) = &mut origin.kind {
                        *name = remap.string(*name)?;
                    }
                    origin.inline_stack = origin
                        .inline_stack
                        .into_iter()
                        .map(map)
                        .collect::<Res<_>>()?;
                    if (call.0 as usize) < offset as usize {
                        origin.inline_stack.push(call);
                    }
                    caller.origins.push(origin);
                }
                for def in &mut body.values {
                    def.origin = map(def.origin)?;
                }
                for cell in &mut body.cells {
                    cell.origin = map(cell.origin)?;
                }
                for op in body.blocks.iter_mut().flat_map(|block| &mut block.ops) {
                    op.origin = map(op.origin)?;
                }
                Some(Cow::Owned(caller))
            }
            // Without source tables no provider location can be invented. Runtime location
            // intrinsics and track-caller bodies are independently refused by the inliner.
            (None, None) => {
                for def in &mut body.values {
                    def.origin = call;
                }
                for cell in &mut body.cells {
                    cell.origin = call;
                }
                for op in body.blocks.iter_mut().flat_map(|block| &mut block.ops) {
                    op.origin = call;
                }
                None
            }
            _ => return Err(InlineDecline::OriginUnavailable),
        };
        imported.interns = Cow::Owned(remap.target);
        imported.origins = origins;
        Ok((body, imported))
    }
}

struct Remap<'a> {
    source: &'a Interns,
    target: Interns,
    pending: Vec<TypeId>,
    seen: BTreeSet<TypeId>,
}

impl Remap<'_> {
    fn string(&mut self, id: StrId) -> Res<StrId> {
        let value = self.source.str(id).ok_or(InlineDecline::StringInvalid)?;
        Ok(self.target.intern_str(value))
    }

    fn fact(&mut self, id: SourceFactId) -> Res<SourceFactId> {
        if id.is_sentinel() {
            return Ok(id);
        }
        let mut fact = self
            .source
            .source_fact(id)
            .ok_or(InlineDecline::SourceInvalid)?
            .clone();
        fact.file = self.string(fact.file)?;
        Ok(self.target.intern_source_fact(fact))
    }

    fn need(&mut self, id: TypeId) {
        if self.seen.insert(id) {
            self.pending.push(id);
        }
    }

    fn direct(&mut self, item: &mut DirectItemRef) -> Res<()> {
        item.link_name = self.string(item.link_name)?;
        Ok(())
    }

    fn global(&mut self, value: &mut GlobalReference) -> Res<()> {
        match value {
            GlobalReference::Function { link_name, .. }
            | GlobalReference::Static { link_name, .. } => *link_name = self.string(*link_name)?,
            GlobalReference::Type(ty) => self.need(*ty),
        }
        Ok(())
    }

    fn constant(&mut self, value: &mut ConstTree) -> Res<()> {
        match value {
            ConstTree::Str(id) => *id = self.string(*id)?,
            ConstTree::Aggregate(fields)
            | ConstTree::Variant { fields, .. }
            | ConstTree::Slice(fields) => {
                for field in fields {
                    self.constant(field)?;
                }
            }
            ConstTree::Reference(inner) => self.constant(inner)?,
            ConstTree::Dyn {
                data,
                concrete,
                vtable,
            } => {
                self.constant(data)?;
                self.need(*concrete);
                *vtable = self.string(*vtable)?;
            }
            ConstTree::Global { target, .. } => self.global(target)?,
            ConstTree::StaticField { target, path }
            | ConstTree::StaticSlice { target, path, .. } => {
                self.global(target)?;
                for step in path {
                    if let StaticStep::Field(name) = step {
                        *name = self.string(*name)?;
                    }
                }
            }
            ConstTree::Unit
            | ConstTree::Scalar { .. }
            | ConstTree::Address(_)
            | ConstTree::SliceAddress { .. } => {}
        }
        Ok(())
    }

    fn callee(&mut self, callee: &mut Callee) -> Res<()> {
        match callee {
            Callee::Direct { link_name, .. } => *link_name = self.string(*link_name)?,
            Callee::Foreign { symbol, .. } => *symbol = self.string(*symbol)?,
            Callee::Indirect { signature } => self.need(*signature),
            Callee::Virtual { receiver, .. } => self.need(*receiver),
            Callee::Runtime { member, .. } => *member = self.string(*member)?,
            Callee::Intrinsic {
                name,
                types,
                fallback,
            } => {
                *name = self.string(*name)?;
                for ty in types {
                    self.need(*ty);
                }
                if let Some(item) = fallback {
                    self.direct(item)?;
                }
            }
            Callee::Nop => {}
        }
        Ok(())
    }

    fn path(&mut self, path: &[Proj]) {
        for step in path {
            if let Proj::Field { ty, .. } = step {
                self.need(*ty);
            }
        }
    }

    fn func(&mut self, body: &mut Func) -> Res<()> {
        for ty in &body.sig.arguments {
            self.need(*ty);
        }
        self.need(body.sig.result);
        for value in &body.values {
            self.need(value.ty);
        }
        for cell in &body.cells {
            self.need(cell.ty);
        }
        for block in &mut body.blocks {
            if let RTerm::Switch { scrut_ty, .. } = block.term {
                self.need(scrut_ty);
            }
            for op in &mut block.ops {
                match &mut op.kind {
                    OpKind::Const(value) => self.constant(value)?,
                    OpKind::Aggregate { ty, .. }
                    | OpKind::Binary { ty, .. }
                    | OpKind::Unary { ty, .. } => self.need(*ty),
                    OpKind::Cast { from, to, .. } => {
                        self.need(*from);
                        self.need(*to);
                    }
                    OpKind::BorrowView { path, .. } | OpKind::RawView { path, .. } => {
                        self.path(path)
                    }
                    OpKind::Drop { path, glue, .. } => {
                        self.path(path);
                        if let DropGlue::Direct { link_name, .. } = glue {
                            *link_name = self.string(*link_name)?;
                        }
                    }
                    OpKind::Call {
                        callee,
                        track_caller,
                        host,
                        ..
                    } => {
                        if let Some(payload) = host {
                            self.host(payload)?;
                        }
                        self.callee(callee)?;
                        *track_caller = track_caller.map(|id| self.fact(id)).transpose()?;
                    }
                    OpKind::Intrinsic {
                        name,
                        types,
                        fallback,
                        track_caller,
                        ..
                    } => {
                        *name = self.string(*name)?;
                        for ty in types {
                            self.need(*ty);
                        }
                        if let Some(item) = fallback {
                            self.direct(item)?;
                        }
                        *track_caller = track_caller.map(|id| self.fact(id)).transpose()?;
                    }
                    OpKind::Assert { track_caller, .. } => {
                        *track_caller = track_caller.map(|id| self.fact(id)).transpose()?;
                    }
                    OpKind::Zombie(_) => return Err(InlineDecline::Zombie),
                    OpKind::Host { payload, .. } => self.host(payload)?,
                    OpKind::Convert { .. } | OpKind::SpillRead(_) | OpKind::SpillWrite { .. } => {
                        return Err(InlineDecline::CarrierBoundary)
                    }
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Repeat { .. }
                    | OpKind::Field { .. }
                    | OpKind::SetField { .. }
                    | OpKind::Discriminant(_)
                    | OpKind::SetDiscriminant { .. }
                    | OpKind::CellAlloc(_)
                    | OpKind::CellLoad(_)
                    | OpKind::CellStore { .. }
                    | OpKind::SliceView { .. }
                    | OpKind::Deref(_)
                    | OpKind::StoreThrough { .. }
                    | OpKind::Materialize(_) => {}
                }
            }
        }
        Ok(())
    }

    fn bytes(&mut self, id: BytesId) -> Res<BytesId> {
        let value = self.source.bytes(id).ok_or(InlineDecline::HostBoundary)?;
        Ok(self.target.intern_bytes(value))
    }

    fn host(&mut self, payload: &mut HostPayload) -> Res<()> {
        match payload {
            HostPayload::Extern(id) | HostPayload::Block(id) => {
                *id = self.bytes(*id)?;
            }
            HostPayload::Marker {
                section, helpers, ..
            } => {
                *section = self.string(*section)?;
                for (name, direct) in helpers {
                    *name = self.string(*name)?;
                    self.direct(direct)?;
                }
            }
            HostPayload::Map { marker } => {
                *marker = self.string(*marker)?;
            }
        }
        Ok(())
    }

    fn sig(&mut self, id: SigId) -> Res<SigId> {
        let mut sig = self
            .source
            .sig(id)
            .ok_or(InlineDecline::SignatureInvalid)?
            .clone();
        for ty in &sig.inputs {
            self.need(*ty);
        }
        self.need(sig.output);
        if let SigAbi::Other(name) = &mut sig.abi {
            *name = self.string(*name)?;
        }
        Ok(self.target.intern_sig(sig))
    }

    fn fields(&mut self, fields: &mut [FieldDescriptor]) -> Res<()> {
        for field in fields {
            self.need(field.ty);
            if let FieldKey::Name(name) = &mut field.key {
                *name = self.string(*name)?;
            }
        }
        Ok(())
    }

    fn variants(&mut self, variants: &mut [VariantDescriptor]) -> Res<()> {
        for variant in variants {
            variant.name = self.string(variant.name)?;
            for name in &mut variant.saved_local_keys {
                *name = self.string(*name)?;
            }
            self.fields(&mut variant.fields)?;
        }
        Ok(())
    }

    fn types(&mut self) -> Res<()> {
        while let Some(id) = self.pending.pop() {
            if self.seen.len() > 4096 {
                return Err(InlineDecline::TypeBudget);
            }
            let mut ty = self
                .source
                .ty(id)
                .ok_or(InlineDecline::TypeUnavailable)?
                .clone();
            ty.name = ty.name.map(|id| self.string(id)).transpose()?;
            match &mut ty.kind {
                TypeKind::Tuple(fields) | TypeKind::CoroutineClosure { upvars: fields, .. } => {
                    for ty in fields {
                        self.need(*ty);
                    }
                }
                TypeKind::Array { element, .. }
                | TypeKind::Slice(element)
                | TypeKind::Pattern(element)
                | TypeKind::Reference {
                    pointee: element, ..
                }
                | TypeKind::RawPointer {
                    pointee: element, ..
                } => self.need(*element),
                TypeKind::Adt { args, variants, .. } => {
                    for ty in args {
                        self.need(*ty);
                    }
                    self.variants(variants)?;
                }
                TypeKind::FnDef { sig, instance, .. } => {
                    *sig = self.sig(*sig)?;
                    if let Some(item) = instance {
                        self.direct(item)?;
                    }
                }
                TypeKind::FnPtr { sig } => *sig = self.sig(*sig)?,
                TypeKind::Closure {
                    upvars,
                    fn_pointer_shim,
                    ..
                } => {
                    for ty in upvars {
                        self.need(*ty);
                    }
                    if let Some(item) = fn_pointer_shim {
                        self.direct(item)?;
                    }
                }
                TypeKind::Coroutine {
                    upvars,
                    saved_locals,
                    variants,
                    ..
                } => {
                    for ty in upvars {
                        self.need(*ty);
                    }
                    for (_, ty) in saved_locals {
                        self.need(*ty);
                    }
                    self.variants(variants)?;
                }
                TypeKind::Opaque => return Err(InlineDecline::TypeOpaque),
                TypeKind::Bool
                | TypeKind::Char
                | TypeKind::Integer { .. }
                | TypeKind::Float { .. }
                | TypeKind::Str
                | TypeKind::Never
                | TypeKind::Dynamic { .. }
                | TypeKind::Foreign(_) => {}
            }
            self.need(ty.js.peeled);
            for field in [ty.js.transparent, ty.js.unsized_tail]
                .into_iter()
                .flatten()
            {
                self.need(field.ty);
            }
            self.fields(&mut ty.js.fields)?;
            if let Some(existing) = self.target.ty(id) {
                // Diagnostic type names may be absent in one producer. Every semantic and
                // representation field must agree under the same stable TypeId.
                ty.name = existing.name;
                if &ty != existing {
                    return Err(InlineDecline::TypeConflict);
                }
            } else {
                self.target.intern_type(ty);
            }
        }
        Ok(())
    }
}
