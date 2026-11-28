//! Call-site encoding: callee resolution, ABI argument splatting, marker section decoding,
//! host payloads, `#[track_caller]` locations, and drop glue.

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, ScalarInt, Ty};
use rustc_span::{Span, Symbol};

use mires_marker_abi::block::PREFIX as BLOCK_PREFIX;
use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::{OriginKind, SourceFact, SourceFactId};
use rcgjs_object::jir::semantic::{
    BasicBlock, BlockId, Call, CallSemantic, Callee, ConstTree, Constant, DirectItemRef, DropGlue,
    HostPayload, Operand, OperandKind, StandardOperation, Terminator, Unwind,
};
use rcgjs_object::jsast;

use super::mir::{block_id, unwind_action};
use super::origins::resolve_caller_location_span;
use super::FnEncoder;
use crate::descriptor::{Descriptor, Root, PREFIX as EXTERN_PREFIX};
use crate::item::{ItemKind, JsItem, JsName, Linkage};
use crate::jir::types::{definition_id, type_name};
use crate::unwind_check::{is_sysroot_build, is_sysroot_crate};
use crate::value::typing_env;

const HOST_PREFIX: &str = "rcgjs.host.";
const MAP_PREFIX: &str = "rcgjs.map.";

/// Classification of a marker function's `#[link_section]` attribute, queried once per call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkerSection<'a> {
    Extern(&'a str),
    Block(&'a str),
    Host(&'a str),
    Map(&'a str),
    Other,
}

impl<'a> MarkerSection<'a> {
    pub(super) fn classify(section: Option<&'a str>) -> Self {
        let Some(section) = section else {
            return Self::Other;
        };
        if let Some(rest) = section.strip_prefix(EXTERN_PREFIX) {
            Self::Extern(rest)
        } else if let Some(rest) = section.strip_prefix(BLOCK_PREFIX) {
            Self::Block(rest)
        } else if let Some(rest) = section.strip_prefix(HOST_PREFIX) {
            Self::Host(rest)
        } else if let Some(rest) = section.strip_prefix(MAP_PREFIX) {
            Self::Map(rest)
        } else {
            Self::Other
        }
    }
}

impl<'a, 'tcx> FnEncoder<'a, 'tcx> {
    /// A binding used as a function value executes its host operation, just like a direct call.
    /// Its Rust placeholder contributes no locals or panic edges to the host body.
    pub(super) fn host_function_body(
        &mut self,
        descriptor: &str,
    ) -> Result<Vec<BasicBlock>, EncodeDecline> {
        self.intern_extern_import(descriptor);
        let instance = self.instance;
        let function_ty = Ty::new_fn_def(
            self.tcx(),
            instance.def_id(),
            ty::Binder::dummy(instance.args),
        );
        let function = Operand {
            ty: self.type_id(function_ty)?,
            kind: OperandKind::Constant(Constant::ZeroSized),
        };
        let (callee, _, _) = self.callee(function_ty)?;
        let arguments = self
            .mir
            .args_iter()
            .map(|local| self.operand(&mir::Operand::Copy(local.into())))
            .collect::<Result<Vec<_>, _>>()?;
        let origin = self.origin(
            OriginKind::Terminator,
            self.mir.basic_blocks[mir::START_BLOCK]
                .terminator()
                .source_info,
            None,
        );
        self.core.origins.extend_terminators([origin, origin]);
        let call = Call {
            function,
            callee,
            semantic: CallSemantic::Host,
            arguments,
            destination: self.place(mir::RETURN_PLACE.into())?,
            target: Some(BlockId(1)),
            unwind: Unwind::Unreachable,
            track_caller: None,
            host: Some(HostPayload::Extern(
                self.core.types.interns.intern_bytes(descriptor.as_bytes()),
            )),
        };
        Ok(vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(call),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
        ])
    }

    pub(super) fn call(
        &mut self,
        func: &mir::Operand<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
        target: Option<mir::BasicBlock>,
        unwind: mir::UnwindAction,
        source_info: mir::SourceInfo,
    ) -> Result<Call, EncodeDecline> {
        let tcx = self.tcx();
        let function_ty = self.monomorphize(func.ty(&self.mir.local_decls, tcx));
        let function = self.operand(func)?;
        let (callee, tracks_caller, resolved) = self.callee(function_ty)?;
        if let Callee::Intrinsic { name, .. } = &callee {
            if self.core.types.interns.str(*name) == Some("const_eval_select") {
                if let Some(arg) = args.get(2) {
                    let runtime_ty = crate::value::peel_pattern(crate::value::peel_transparent(
                        tcx,
                        self.monomorphize(arg.node.ty(&self.mir.local_decls, tcx)),
                    ));
                    if let ty::FnDef(def, fn_args) = *runtime_ty.kind() {
                        if let Some(Ok(Some(inst))) = fn_args
                            .no_bound_vars()
                            .map(|a| Instance::try_resolve(tcx, typing_env(), def, a))
                        {
                            self.core
                                .extra_refs
                                .insert(self.core.cgu.namer.fn_name(inst));
                        }
                    }
                }
            }
        }
        let link_section = match *function_ty.kind() {
            ty::FnDef(definition, _) => tcx.codegen_fn_attrs(definition).link_section,
            _ => None,
        };
        let marker = MarkerSection::classify(link_section.as_ref().map(Symbol::as_str));
        let semantic = self.call_semantic(function_ty, resolved, marker);
        let host = self.host_payload(function_ty, marker, args)?;
        let arguments = self.call_arguments(function_ty, args)?;
        // Resolve the caller location across any MIR-inlined `#[track_caller]` frames:
        // - `Some(fact)` materializes that concrete call site (including when a non-`#[track_caller]`
        //   helper was MIR-inlined into a `#[track_caller]` body).
        // - `Some(SourceFactId::FORWARD)` explicitly marks a `#[track_caller]` callee that forwards
        //   the enclosing body's `$loc` parameter.
        // - `Some(SourceFactId::NOT_TRACKED)` explicitly marks a non-`#[track_caller]` callee inside
        //   a `#[track_caller]` body so link-time lowering never needs to guess its ABI from
        //   `ItemIndex`.
        // - `None` is kept for non-`#[track_caller]` callers calling non-`#[track_caller]` callees
        //   and for un-inlined `caller_location` intrinsics forwarding `$loc`.
        let is_caller_location_intrinsic = matches!(
            &callee,
            Callee::Intrinsic { name, .. }
                if self.core.types.interns.str(*name) == Some("caller_location")
        );
        let track_caller = if tracks_caller {
            match resolve_caller_location_span(tcx, self.mir, self.instance, source_info) {
                Some(span) => Some(self.caller_location_fact(span)?),
                None if is_caller_location_intrinsic => None,
                None => Some(SourceFactId::FORWARD),
            }
        } else if self.instance.def.requires_caller_location(tcx) && !is_caller_location_intrinsic {
            Some(SourceFactId::NOT_TRACKED)
        } else {
            None
        };
        Ok(Call {
            function,
            callee,
            semantic,
            arguments,
            destination: self.place(destination)?,
            target: target.map(block_id),
            unwind: unwind_action(unwind),
            track_caller,
            host,
        })
    }

    /// The JavaScript argument list of a call: the MIR arguments in order, with a
    /// `"rust-call"` callee's trailing tuple already **splatted** into one argument per element
    /// (v1 `abi.rs::splat_rust_call_args`).
    ///
    /// The splat happens here rather than in the consumer because the call site's ABI is a fact
    /// only rustc has: `Callee::Direct` names an item, and a closure body, an `FnOnce::call_once`
    /// shim and an ordinary function taking a one-element tuple are indistinguishable from the
    /// item alone. Splatting at encode time also puts the caller side in the convention the
    /// callee side is already encoded in — `AbiSignature::parameters` counts a spread argument
    /// as its arity — so a JIR-S call's arguments and its callee's parameters line up one to one.
    ///
    /// Each element is read as `Operand::Copy` of the pack's field, which is exactly the element
    /// read v1 emits (a place read, cloned when the element type needs cloning).
    fn call_arguments(
        &mut self,
        function_ty: Ty<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
    ) -> Result<Vec<Operand>, EncodeDecline> {
        let tcx = self.tcx();
        let is_rust_call = matches!(function_ty.kind(), ty::FnDef(..) | ty::FnPtr(..))
            && function_ty.fn_sig(tcx).abi() == rustc_abi::ExternAbi::RustCall;
        if !is_rust_call {
            return args
                .iter()
                .map(|argument| self.operand(&argument.node))
                .collect();
        }
        let Some((pack, rest)) = args.split_last() else {
            return Err(EncodeDecline::SpreadNotTuple);
        };
        let mut out = Vec::with_capacity(rest.len() + 1);
        for argument in rest {
            out.push(self.operand(&argument.node)?);
        }
        let pack_ty = self.monomorphize(pack.node.ty(&self.mir.local_decls, tcx));
        let ty::Tuple(elements) = pack_ty.kind() else {
            return Err(EncodeDecline::SpreadNotTuple);
        };
        if elements.is_empty() {
            return Ok(out);
        }
        let (mir::Operand::Copy(place) | mir::Operand::Move(place)) = pack.node else {
            // A **constant** pack has no place to project out of, so it is split by decoding it:
            // a tuple constant is an aggregate tree whose fields are the arguments.
            let decoded = self.operand(&pack.node)?;
            // A ZERO-SIZED pack has no memory either, so `ConstValue::ZeroSized` is the whole of
            // it and there is no tree to take fields out of. A tuple is zero-sized exactly when
            // every element is, so the spread is one zero-sized constant per element. This is the
            // shape `Result::and_then`'s closure takes — `((),)` — and it is what left
            // `fmt::builders::DebugTuple::{field,finish}` without a JIR body until v2 W10.
            if matches!(decoded.kind, OperandKind::Constant(Constant::ZeroSized)) {
                for element_ty in elements.iter() {
                    out.push(Operand {
                        ty: self.type_id(element_ty)?,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    });
                }
                return Ok(out);
            }
            let fields = match decoded.kind {
                OperandKind::Constant(Constant::Memory(ConstTree::Aggregate(fields))) => fields,
                // A pack whose layout is ONE SCALAR is not memory either: const evaluation keeps
                // such a value in a register, so it arrives as its bits. A tuple is one scalar
                // when a single element is not zero-sized and that element is a scalar —
                // `(usize,)`, `(u32, ())` — which is what a closure called with one literal
                // argument passes once MIR optimization has folded the argument tuple
                // (MIRES-020). The bits are the tuple as memory would hold it, so they go back
                // into a scratch allocation laid out for the tuple, and the tuple is decoded from
                // there exactly as an in-memory pack is.
                OperandKind::Constant(Constant::Scalar { bits, bytes }) => {
                    let scratch = ScalarInt::try_from_uint(bits, Size::from_bytes(bytes))
                        .and_then(|int| crate::const_raw::scratch_alloc(tcx, int, pack_ty))
                        .ok_or_else(|| EncodeDecline::ConstantNoLayout {
                            ty: type_name(pack_ty),
                        })?;
                    let ConstTree::Aggregate(fields) =
                        self.core.const_tree(&scratch, Size::ZERO, pack_ty, 0)?
                    else {
                        return Err(EncodeDecline::SpreadNotTuple);
                    };
                    fields
                }
                _ => return Err(EncodeDecline::SpreadNotTuple),
            };
            if fields.len() != elements.len() {
                return Err(EncodeDecline::SpreadNotTuple);
            }
            for (field, element_ty) in fields.into_iter().zip(elements.iter()) {
                out.push(Operand {
                    ty: self.type_id(element_ty)?,
                    kind: OperandKind::Constant(Constant::Memory(field)),
                });
            }
            return Ok(out);
        };
        for (index, element_ty) in elements.iter().enumerate() {
            let element = place.project_deeper(
                &[mir::PlaceElem::Field(
                    rustc_abi::FieldIdx::from_usize(index),
                    element_ty,
                )],
                tcx,
            );
            out.push(self.operand(&mir::Operand::Copy(element))?);
        }
        Ok(out)
    }

    /// The callee, whether it takes a caller location, and the resolved `Instance` (when
    /// non-foreign and resolvable), exactly as `abi.rs::codegen_call` resolves it.
    fn callee(
        &mut self,
        function_ty: Ty<'tcx>,
    ) -> Result<(Callee, bool, Option<Instance<'tcx>>), EncodeDecline> {
        let tcx = self.tcx();
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return Ok((
                Callee::Indirect {
                    signature: self.type_id(function_ty)?,
                },
                false,
                None,
            ));
        };
        if tcx.is_foreign_item(definition) {
            let symbol = tcx.symbol_name(Instance::mono(tcx, definition)).name;
            let abi = function_ty.fn_sig(tcx).abi();
            // A Rust-ABI foreign declaration is a Rust symbol some other crate defines (`core`'s
            // `panic_impl`); everything else is the `__rt` print shim (`abi.rs`).
            let callee =
                if abi == rustc_abi::ExternAbi::Rust && crate::jsast::is_plain_ident(symbol) {
                    Callee::Foreign {
                        definition: definition_id(tcx, definition),
                        symbol: self.core.types.str(symbol),
                    }
                } else {
                    let sysroot = is_sysroot_build(tcx) || is_sysroot_crate(tcx, definition.krate);
                    Callee::Runtime {
                        member: self.core.types.str(&crate::jsast::sanitize_ident(symbol)),
                        sysroot,
                    }
                };
            return Ok((callee, false, None));
        }
        let Some(arguments) = arguments.no_bound_vars() else {
            return Ok((
                Callee::Indirect {
                    signature: self.type_id(function_ty)?,
                },
                false,
                None,
            ));
        };
        let instance = match Instance::try_resolve(tcx, typing_env(), definition, arguments) {
            Ok(Some(instance)) => instance,
            Ok(None) | Err(_) => {
                return Ok((
                    Callee::Indirect {
                        signature: self.type_id(function_ty)?,
                    },
                    false,
                    None,
                ));
            }
        };
        let mut tracks_caller = instance.def.requires_caller_location(tcx);
        let callee = match instance.def {
            ty::InstanceKind::Intrinsic(def_id) => {
                let must_override = tcx
                    .intrinsic(def_id)
                    .is_some_and(|intrinsic| intrinsic.must_be_overridden);
                let fallback = if !must_override && tcx.is_mir_available(def_id) {
                    let fallback = Instance::new_raw(def_id, instance.args);
                    Some(self.core.direct_item(fallback))
                } else {
                    None
                };
                let arguments: Vec<Ty<'tcx>> = instance
                    .args
                    .iter()
                    .filter_map(|argument| argument.as_type())
                    .collect();
                let types = arguments
                    .iter()
                    .map(|argument| self.type_id(*argument))
                    .collect::<Result<_, _>>()?;
                let name = tcx.item_name(def_id);
                if name.as_str() == "caller_location" {
                    self.describe_caller_location_type()?;
                    tracks_caller = true;
                }
                // `type_name`'s *value* is a type's printed path, which no other lowering reads;
                // the descriptor carries one only where a body asks for it.
                if name.as_str() == "type_name" {
                    for argument in &arguments {
                        self.core.types.record_type_name(*argument)?;
                    }
                }
                Callee::Intrinsic {
                    name: self.core.types.str(name.as_str()),
                    types,
                    fallback,
                }
            }
            ty::InstanceKind::Virtual(_, slot) => Callee::Virtual {
                slot: slot as u32,
                receiver: self.type_id(instance.args.type_at(0))?,
            },
            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => Callee::Nop,
            _ => {
                let item = self.core.direct_item(instance);
                Callee::Direct {
                    item: item.item,
                    link_name: item.link_name,
                }
            }
        };
        Ok((callee, tracks_caller, Some(instance)))
    }

    /// v1's `late_ir::Encoder::call_semantic`, unchanged in rule.
    fn call_semantic(
        &self,
        function_ty: Ty<'tcx>,
        resolved: Option<Instance<'tcx>>,
        marker: MarkerSection<'_>,
    ) -> CallSemantic {
        let tcx = self.tcx();
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return CallSemantic::Ordinary;
        };
        let Some(arguments) = arguments.no_bound_vars() else {
            return CallSemantic::Ordinary;
        };
        if let Some(instance) = resolved {
            if let Some(operation) =
                crate::jir::classify::standard_operation(tcx, instance.def_id(), instance.args)
            {
                return call_semantic_for_operation(operation);
            }
            if matches!(instance.def, ty::InstanceKind::Item(_)) {
                if let Some(operation) =
                    crate::jir::classify::str_op(tcx, instance.def_id(), instance.args)
                {
                    return CallSemantic::Standard(operation);
                }
            }
        }
        if resolved.is_none_or(|inst| inst.def_id() != definition || inst.args != arguments) {
            if let Some(operation) =
                crate::jir::classify::standard_operation(tcx, definition, arguments)
            {
                return call_semantic_for_operation(operation);
            }
        }
        match marker {
            MarkerSection::Extern(_) => return CallSemantic::Host,
            MarkerSection::Block(_) => return CallSemantic::RawJavaScript,
            MarkerSection::Host(_) | MarkerSection::Map(_) | MarkerSection::Other => {}
        }
        if tcx.is_foreign_item(definition) {
            let symbol = tcx.symbol_name(Instance::mono(tcx, definition));
            if matches!(
                symbol.name,
                "__rust_alloc"
                    | "__rust_alloc_zeroed"
                    | "__rust_dealloc"
                    | "__rust_realloc"
                    | "__rust_alloc_error_handler"
            ) {
                return CallSemantic::GlobalAllocator;
            }
        }
        let signature = function_ty.fn_sig(tcx).skip_binder();
        if signature
            .inputs_and_output
            .iter()
            .any(|ty| matches!(ty.kind(), ty::RawPtr(..)))
        {
            return CallSemantic::RawParts;
        }
        CallSemantic::Ordinary
    }

    /// The host-side payload of a call the lowering intercepts, decoded from the marker's
    /// `link_section` (see [`HostPayload`] for the convention).
    fn host_payload(
        &mut self,
        function_ty: Ty<'tcx>,
        marker: MarkerSection<'_>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
    ) -> Result<Option<HostPayload>, EncodeDecline> {
        let tcx = self.tcx();
        let ty::FnDef(definition, _) = *function_ty.kind() else {
            return Ok(None);
        };
        Ok(match marker {
            MarkerSection::Extern(descriptor) => {
                self.intern_extern_import(descriptor);
                Some(HostPayload::Extern(
                    self.core.types.interns.intern_bytes(descriptor.as_bytes()),
                ))
            }
            MarkerSection::Block(block) => Some(HostPayload::Block(
                self.core.types.interns.intern_bytes(block.as_bytes()),
            )),
            MarkerSection::Map(marker_name) => Some(HostPayload::Map {
                marker: self.core.types.str(marker_name),
            }),
            MarkerSection::Host(marker_name) => {
                self.record_host_marker_refs(args);
                let helpers = self.marker_helpers(definition, marker_name, args);
                Some(HostPayload::Marker {
                    section: self.core.types.str(&format!("{HOST_PREFIX}{marker_name}")),
                    def: definition_id(tcx, definition),
                    helpers,
                })
            }
            MarkerSection::Other => None,
        })
    }

    /// The sibling instances a marker's lowering calls, resolved at the marker's own generic
    /// argument (`host.rs::host_future_helper`).
    fn marker_helpers(
        &mut self,
        marker: DefId,
        marker_name: &str,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
    ) -> Vec<(StrId, DirectItemRef)> {
        let (helper, argument) = match marker_name {
            "spawn" => ("poll_once", 0),
            "futurepromise" | "futurenativepromise" => ("poll_result", 0),
            _ => return Vec::new(),
        };
        let Some(argument) = args.get(argument) else {
            return Vec::new();
        };
        let tcx = self.tcx();
        let ty = self.monomorphize(argument.node.ty(&self.mir.local_decls, tcx));
        let Some(item) = self.sibling_instance(marker, helper, ty) else {
            return Vec::new();
        };
        let name = self.core.types.str(helper);
        vec![(name, item)]
    }

    /// The instance of the item named `name` beside `sibling`, instantiated at `ty`.
    fn sibling_instance(
        &mut self,
        sibling: DefId,
        name: &str,
        ty: Ty<'tcx>,
    ) -> Option<DirectItemRef> {
        let tcx = self.tcx();
        let parent = tcx.parent(sibling);
        let helper = tcx
            .module_children(parent)
            .iter()
            .find(|child| child.ident.name.as_str() == name)
            .and_then(|child| child.res.opt_def_id())?;
        let arguments = tcx.mk_args(&[ty.into()]);
        let instance = Instance::try_resolve(tcx, typing_env(), helper, arguments)
            .ok()
            .flatten()?;
        Some(self.core.direct_item(instance))
    }

    /// The caller `Location` a `#[track_caller]` callee receives from this call site, as the
    /// source fact `intrinsics::caller_location_value` spells: the outermost macro expansion,
    /// 1-based line, 0-based display column (the `Location` column is this plus one).
    pub(super) fn caller_location_fact(
        &mut self,
        span: Span,
    ) -> Result<SourceFactId, EncodeDecline> {
        let tcx = self.tcx();
        let topmost = span.ctxt().outer_expn().expansion_cause().unwrap_or(span);
        let caller = tcx.sess.source_map().lookup_char_pos(topmost.lo());
        let file = caller
            .file
            .name
            .prefer_remapped_unconditionally()
            .to_string();
        let file = self.core.types.str(&file);
        self.describe_caller_location_type()?;
        Ok(self.core.types.interns.intern_source_fact(SourceFact {
            file,
            line: caller.line as u32,
            col: caller.col_display as u32,
        }))
    }

    /// Preserve the type Phase C needs to materialize a caller location. It is hidden from MIR's
    /// declared locals, so every semantic operation that can synthesize one records it explicitly.
    pub(super) fn describe_caller_location_type(&mut self) -> Result<(), EncodeDecline> {
        let tcx = self.tcx();
        if tcx
            .lang_items()
            .get(rustc_hir::LangItem::PanicLocation)
            .is_some()
        {
            self.type_id(tcx.caller_location_ty())?;
        }
        Ok(())
    }

    pub(super) fn drop_glue(&mut self, place: mir::Place<'tcx>) -> DropGlue {
        let tcx = self.tcx();
        let ty = self.monomorphize(place.ty(&self.mir.local_decls, tcx).ty);
        self.drop_glue_for_ty(ty)
    }

    fn drop_glue_for_ty(&mut self, ty: Ty<'tcx>) -> DropGlue {
        let tcx = self.tcx();
        let glue = Instance::resolve_drop_glue(tcx, ty);
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) = glue.def {
            DropGlue::None
        } else if matches!(ty.kind(), ty::Dynamic(..)) {
            DropGlue::Dynamic
        } else {
            let item = self.core.direct_item(glue);
            DropGlue::Direct {
                item: item.item,
                link_name: item.link_name,
            }
        }
    }

    /// Interns an `ItemKind::Import` item for a `#[js_extern(module = "...")]` descriptor and
    /// records the binding in `extra_refs` so link-time reachability keeps the import alive.
    fn intern_extern_import(&mut self, raw: &str) {
        let Ok(descriptor) = Descriptor::decode(raw) else {
            return;
        };
        if descriptor.root() != Root::Module {
            return;
        }
        let Some(module) = descriptor.module() else {
            return;
        };
        let head = descriptor
            .segments()
            .first()
            .copied()
            .unwrap_or_default()
            .to_owned();
        let local = crate::naming::extern_import(module, &head);
        let js_name = JsName::new(local.clone());
        let source = module.to_owned();
        self.core.cgu.intern(js_name.clone(), || {
            JsItem::new(
                js_name.clone(),
                ItemKind::Import,
                jsast::import_named(vec![(head.clone(), local.clone())], source.clone()),
                Vec::new(),
                Linkage {
                    fixed_name: Some(local.clone()),
                    root: false,
                    exported: false,
                    chunk: None,
                },
                source.clone(),
            )
        });
        self.core.extra_refs.insert(js_name);
    }

    /// Records closure/fn bodies and drop glue referenced by `rcgjs.host.*` markers (`callback*`,
    /// `try`, `spawn`, `futurepromise`, `futurenativepromise`, `promise`, `microtask`, `task`,
    /// `settled`, `localwaker`).
    fn record_host_marker_refs(&mut self, args: &[rustc_span::Spanned<mir::Operand<'tcx>>]) {
        let tcx = self.tcx();
        for arg in args {
            let arg_ty = self.monomorphize(arg.node.ty(&self.mir.local_decls, tcx));
            if arg_ty.needs_drop(tcx, typing_env()) {
                let glue = Instance::resolve_drop_glue(tcx, arg_ty);
                if !matches!(
                    glue.def,
                    ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None))
                ) {
                    self.core
                        .extra_refs
                        .insert(self.core.cgu.namer.fn_name(glue));
                }
            }
            let peeled = crate::value::peel_pattern(crate::value::peel_transparent(tcx, arg_ty));
            match *peeled.kind() {
                ty::Closure(def, cl_args) => {
                    let inst = Instance::new_raw(def, cl_args);
                    self.core
                        .extra_refs
                        .insert(self.core.cgu.namer.fn_name(inst));
                }
                ty::FnDef(def, fn_args) => {
                    if let Some(Ok(Some(inst))) = fn_args
                        .no_bound_vars()
                        .map(|a| Instance::try_resolve(tcx, typing_env(), def, a))
                    {
                        self.core
                            .extra_refs
                            .insert(self.core.cgu.namer.fn_name(inst));
                    }
                }
                _ => {}
            }
        }
    }
}

fn call_semantic_for_operation(operation: StandardOperation) -> CallSemantic {
    // Proven format sinks are inlined by the language kernel; keeping those calls Ordinary lets a
    // user function join a native String region without staging `format` itself (v1 rule).
    // Float `FromStr` bodies are replaced whole during lowering, not representation-planned.
    match operation {
        StandardOperation::FormatArgsToString
        | StandardOperation::FormatArgsNew
        | StandardOperation::FormatArgumentNewDisplay
        | StandardOperation::FormatArgumentNewLowerHex
        | StandardOperation::FormatArgumentNewUpperHex
        | StandardOperation::F64FromStr
        | StandardOperation::F32FromStr => CallSemantic::Ordinary,
        other => CallSemantic::Standard(other),
    }
}
