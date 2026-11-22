//! The tcx side of the frozen v1 late IR: encoding a monomorphic MIR body into it.
//!
//! The schema itself — every type below [`LateIrAnalysisSummary`], the merge rules, the validators
//! and the postcard round trip — lives in [`rcgjs_object::late_ir`], because the frozen v1 text
//! optimizer in `jsc-link` consumes it without a compiler. What is left here is the [`Encoder`].
//!
//! The executable body contains stable semantic identities and no source paths or spans. Those
//! live in a separate diagnostics table, so two crates can merge the same monomorphic body even
//! when rustc describes it through different paths. Encoding is deliberately conservative: one
//! unsupported operation makes the whole item unavailable and keeps its printed JavaScript as the
//! executable fallback.

use std::collections::{BTreeMap, BTreeSet};

use rustc_abi::{Size, TagEncoding, Variants};
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::mir::interpret::{GlobalAlloc, Scalar};
use rustc_middle::mir::visit::{MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor};
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypeFoldable};
use rustc_span::{sym, Span, Symbol};

use crate::cgu::CguCx;
use crate::value::typing_env;

pub(crate) use rcgjs_object::late_ir::*;

/// The discriminant a `Variants::Multiple` tag encodes directly, when it fits the schema.
///
/// Stays here rather than beside its type: `Size` is `rustc_abi`'s.
fn physical_direct_discriminant(value: u128, size: Size) -> Option<NominalDiscriminant> {
    let discriminant = NominalDiscriminant {
        bits: size.truncate(value),
        bytes: u8::try_from(size.bytes()).ok()?,
    };
    discriminant.is_valid().then_some(discriminant)
}

// The tcx-side constructors of [`LateIrAnalysisSummary`].
//
// Free functions rather than inherent methods: the type is defined in `rcgjs-object`, and an
// inherent `impl` has to live with its type.

/// Encodes a monomorphic MIR body. Any unsupported operation rejects the complete body.
pub(crate) fn function<'tcx>(
    cgu: &CguCx<'tcx>,
    instance: Instance<'tcx>,
    mir: &mir::Body<'tcx>,
) -> LateIrAnalysisSummary {
    let carrier_bridges = encode_carrier_bridges(cgu, instance, mir);
    let (jir, jir_decline) = jir_lane(crate::jir::encode::function(cgu, instance, mir), || {
        cgu.namer.fn_name(instance).into_string()
    });
    let mut encoder = Encoder::new(cgu, instance, mir);
    match encoder.encode() {
        Ok((body, diagnostics)) => LateIrAnalysisSummary {
            version: LATE_IR_ANALYSIS_VERSION,
            availability: Availability::Available(body),
            carrier_bridges,
            diagnostics,
            allocator_provider: AllocatorProvider::Unknown,
            jir,
            jir_decline,
        },
        Err(reason) => match Encoder::new(cgu, instance, mir).semantic_header() {
            Ok((body, diagnostics)) if matches!(body.semantic, BodySemantic::Standard(_)) => {
                LateIrAnalysisSummary {
                    version: LATE_IR_ANALYSIS_VERSION,
                    availability: Availability::Semantic(body, reason),
                    carrier_bridges,
                    diagnostics,
                    allocator_provider: AllocatorProvider::Unknown,
                    jir,
                    jir_decline,
                }
            }
            _ => LateIrAnalysisSummary {
                version: LATE_IR_ANALYSIS_VERSION,
                availability: Availability::Unavailable(reason),
                carrier_bridges,
                diagnostics: encoder.root_diagnostics(),
                allocator_provider: AllocatorProvider::Unknown,
                jir,
                jir_decline,
            },
        },
    }
}

/// The JIR-S of the vtable `vtable.rs` builds for `ty as dyn trait_ref`.
///
/// The v1 lane has no vtable body, so `availability` stays the synthesized default and the
/// report is unchanged; only `jir` is set.
pub(crate) fn vtable<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    trait_ref: Option<ty::ExistentialTraitRef<'tcx>>,
) -> LateIrAnalysisSummary {
    let (jir, jir_decline) = jir_lane(crate::jir::encode::vtable(cgu, ty, trait_ref), || {
        cgu.namer.vtable_name(ty, trait_ref).into_string()
    });
    LateIrAnalysisSummary {
        jir,
        jir_decline,
        ..LateIrAnalysisSummary::default()
    }
}

/// Records the stable identity and type of a static.
///
/// Static initializers remain in the printed JavaScript body. This body kind lets whole-program
/// analysis identify the boundary without pretending that an initializer CFG exists.
pub(crate) fn static_item<'tcx>(cgu: &CguCx<'tcx>, def_id: DefId) -> LateIrAnalysisSummary {
    let tcx = cgu.tcx;
    let (jir, jir_decline) = jir_lane(crate::jir::encode::static_item(cgu, def_id), || {
        cgu.namer.static_name(def_id).into_string()
    });
    let ty =
        tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    let type_id = TypeId(tcx.type_id_hash(ty).as_u128());
    let body = LateBody {
        item: ItemId {
            definition: definition_id(tcx, def_id),
            instantiation: type_id.0,
        },
        kind: BodyKind::Static,
        semantic: BodySemantic::Ordinary,
        signature: Signature {
            arguments: Vec::new(),
            result: type_id,
            abi: AbiSignature::default(),
        },
        locals: Vec::new(),
        types: vec![TypeDefinition {
            id: type_id,
            kind: TypeKind::Opaque,
            metadata: type_metadata(tcx, ty),
        }],
        blocks: Vec::new(),
    };
    let mut diagnostics = DiagnosticInfo {
        item_path: Some(with_no_trimmed_paths!(tcx.def_path_str(def_id))),
        root: source(tcx, tcx.def_span(def_id)),
        type_names: vec![TypeDiagnostic {
            id: type_id,
            name: with_no_trimmed_paths!(ty.to_string()),
        }],
        locations: Vec::new(),
    };
    diagnostics.normalize();
    LateIrAnalysisSummary {
        version: LATE_IR_ANALYSIS_VERSION,
        availability: Availability::Available(body),
        carrier_bridges: Vec::new(),
        diagnostics,
        allocator_provider: AllocatorProvider::Unknown,
        jir,
        jir_decline,
    }
}

/// Splits an encode result into the summary's two JIR fields, printing the coverage line the
/// coverage scripts aggregate when `JSC_JIR_SELFCHECK=1`.
fn jir_lane(
    result: Result<crate::jir::JirItem, rcgjs_object::jir::decline::EncodeDecline>,
    name: impl FnOnce() -> String,
) -> (
    Option<crate::jir::JirItem>,
    Option<rcgjs_object::jir::Decline>,
) {
    let coverage = std::env::var_os("JSC_JIR_SELFCHECK").is_some_and(|value| value == "1");
    match result {
        Ok(item) => {
            if coverage {
                eprintln!("jir-coverage: {} ok {}", name(), item.body.kind.as_str());
            }
            (Some(item), None)
        }
        Err(decline) => {
            let decline = rcgjs_object::jir::Decline::Encode(decline);
            if coverage {
                eprintln!("jir-coverage: {} {} {decline}", name(), decline.code());
            }
            (None, Some(decline))
        }
    }
}
struct Encoder<'a, 'tcx> {
    cgu: &'a CguCx<'tcx>,
    instance: Instance<'tcx>,
    mir: &'a mir::Body<'tcx>,
    types: BTreeMap<TypeId, TypeKind>,
    type_metadata: BTreeMap<TypeId, TypeMetadata>,
    type_names: BTreeMap<TypeId, String>,
    locations: Vec<LocationDiagnostic>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CarrierLocalFacts {
    definitions: u32,
    shared_borrows: u32,
    shared_alias: Option<mir::Local>,
    drops: u32,
    invalid: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CarrierAliasFacts {
    definitions: u32,
    as_str_arguments: u32,
    invalid: bool,
}

struct CarrierUseCollector<'a> {
    carriers: &'a mut [CarrierLocalFacts],
    aliases: &'a mut [CarrierAliasFacts],
    producer_destinations: BTreeSet<mir::Local>,
    allowed_borrows: BTreeSet<(mir::Local, mir::Local)>,
    allowed_as_str_arguments: BTreeSet<mir::Local>,
    allowed_drops: BTreeSet<mir::Local>,
    cleanup_blocks: BTreeSet<mir::BasicBlock>,
}

impl<'tcx> Visitor<'tcx> for CarrierUseCollector<'_> {
    fn visit_place(
        &mut self,
        place: &mir::Place<'tcx>,
        context: PlaceContext,
        location: mir::Location,
    ) {
        let local = place.local;
        let projected = !place.projection.is_empty();
        if self.cleanup_blocks.contains(&location.block) {
            if let Some(facts) = self.carriers.get_mut(local.as_usize()) {
                facts.invalid = true;
            }
            if let Some(facts) = self.aliases.get_mut(local.as_usize()) {
                facts.invalid = true;
            }
            self.super_place(place, context, location);
            return;
        }
        if let Some(facts) = self.carriers.get_mut(local.as_usize()) {
            match context {
                PlaceContext::NonUse(_) => {}
                PlaceContext::MutatingUse(MutatingUseContext::Call)
                    if !projected && self.producer_destinations.contains(&local) =>
                {
                    facts.definitions += 1;
                }
                PlaceContext::MutatingUse(MutatingUseContext::Drop)
                    if !projected && self.allowed_drops.contains(&local) =>
                {
                    facts.drops += 1;
                }
                PlaceContext::NonMutatingUse(NonMutatingUseContext::SharedBorrow)
                    if !projected
                        && self
                            .allowed_borrows
                            .iter()
                            .any(|(carrier, _)| *carrier == local) =>
                {
                    facts.shared_borrows += 1;
                }
                PlaceContext::NonMutatingUse(NonMutatingUseContext::Projection)
                | PlaceContext::MutatingUse(MutatingUseContext::Projection) => {}
                _ => facts.invalid = true,
            }
            if projected {
                facts.invalid = true;
            }
        }
        if let Some(facts) = self.aliases.get_mut(local.as_usize()) {
            match context {
                PlaceContext::NonUse(_) => {}
                PlaceContext::MutatingUse(MutatingUseContext::Store)
                    if !projected
                        && self
                            .allowed_borrows
                            .iter()
                            .any(|(_, alias)| *alias == local) =>
                {
                    facts.definitions += 1;
                }
                PlaceContext::NonMutatingUse(
                    NonMutatingUseContext::Copy | NonMutatingUseContext::Move,
                ) if !projected && self.allowed_as_str_arguments.contains(&local) => {
                    facts.as_str_arguments += 1;
                }
                PlaceContext::NonMutatingUse(NonMutatingUseContext::Projection)
                | PlaceContext::MutatingUse(MutatingUseContext::Projection) => {}
                _ => facts.invalid = true,
            }
            if projected {
                facts.invalid = true;
            }
        }
        self.super_place(place, context, location);
    }

    fn visit_local(
        &mut self,
        _local: mir::Local,
        _context: PlaceContext,
        _location: mir::Location,
    ) {
    }
}

fn encode_carrier_bridges<'tcx>(
    cgu: &CguCx<'tcx>,
    instance: Instance<'tcx>,
    body: &mir::Body<'tcx>,
) -> Vec<CarrierBridge> {
    CarrierBridgeEncoder::new(cgu, instance, body).encode()
}

struct CarrierBridgeEncoder<'a, 'tcx> {
    cgu: &'a CguCx<'tcx>,
    instance: Instance<'tcx>,
    body: &'a mir::Body<'tcx>,
}

struct DirectFunctionUseCollector<'a, 'b, 'tcx> {
    encoder: &'a CarrierBridgeEncoder<'b, 'tcx>,
    occurrences: BTreeMap<DirectItemRef, u32>,
}

impl<'tcx> Visitor<'tcx> for DirectFunctionUseCollector<'_, '_, 'tcx> {
    fn visit_operand(&mut self, operand: &mir::Operand<'tcx>, location: mir::Location) {
        if let Some(item) = self.encoder.direct_item(operand) {
            *self.occurrences.entry(item).or_default() += 1;
        }
        self.super_operand(operand, location);
    }
}

impl<'a, 'tcx> CarrierBridgeEncoder<'a, 'tcx> {
    fn new(cgu: &'a CguCx<'tcx>, instance: Instance<'tcx>, body: &'a mir::Body<'tcx>) -> Self {
        Self {
            cgu,
            instance,
            body,
        }
    }

    fn encode(&self) -> Vec<CarrierBridge> {
        let mut candidates = Vec::new();
        let mut use_collector = DirectFunctionUseCollector {
            encoder: self,
            occurrences: BTreeMap::new(),
        };
        use_collector.visit_body(self.body);
        for data in self.body.basic_blocks.iter() {
            let mir::TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                ..
            } = &data.terminator().kind
            else {
                continue;
            };
            let Some(producer) = self.direct_item(func) else {
                continue;
            };
            if !exact_normal_call(
                target.is_some_and(|target| !self.body.basic_blocks[target].is_cleanup),
                unwind,
            ) || destination.as_local().is_none()
                || !self.is_global_string(destination.ty(&self.body.local_decls, self.cgu.tcx).ty)
                || !args
                    .iter()
                    .all(|argument| self.baseline_compatible_argument(&argument.node))
            {
                continue;
            }
            candidates.push((producer, destination.as_local().unwrap()));
        }

        let mut bridges = candidates
            .into_iter()
            .filter(|(producer, _)| {
                unique_direct_function_use(use_collector.occurrences.get(producer))
            })
            .filter_map(|(producer, carrier)| self.certify(producer, carrier))
            .collect::<Vec<_>>();
        bridges.sort();
        bridges.dedup();
        bridges
    }

    fn certify(&self, producer: DirectItemRef, carrier: mir::Local) -> Option<CarrierBridge> {
        let mut borrow = None;
        let mut as_str = None;
        let mut drop = None;
        for data in self.body.basic_blocks.iter() {
            for statement in &data.statements {
                let mir::StatementKind::Assign(assignment) = &statement.kind else {
                    continue;
                };
                let Some(alias) = assignment.0.as_local() else {
                    continue;
                };
                if matches!(
                    &assignment.1,
                    mir::Rvalue::Ref(_, mir::BorrowKind::Shared, place)
                        if place.as_local() == Some(carrier)
                ) && borrow.replace(alias).is_some()
                {
                    return None;
                }
            }

            match &data.terminator().kind {
                mir::TerminatorKind::Call {
                    func,
                    args,
                    target,
                    unwind,
                    ..
                } if self.call_operation(func) == Some(StandardOperation::StringAsStr) => {
                    let Some(alias) = borrow else {
                        continue;
                    };
                    if exact_normal_call(
                        target.is_some_and(|target| !self.body.basic_blocks[target].is_cleanup),
                        unwind,
                    ) && args.len() == 1
                        && operand_root_local(&args[0].node) == Some(alias)
                        && as_str.replace(self.direct_item(func)?).is_some()
                    {
                        return None;
                    }
                }
                mir::TerminatorKind::Drop {
                    place,
                    target,
                    unwind,
                    replace,
                    drop: async_drop,
                } if place.as_local() == Some(carrier)
                    && (!matches!(unwind, mir::UnwindAction::Unreachable)
                        || self.body.basic_blocks[*target].is_cleanup
                        || *replace
                        || async_drop.is_some()
                        || drop.replace(self.drop_item(*place)?).is_some()) =>
                {
                    return None
                }
                _ => {}
            }
        }
        let (alias, as_str, drop) = (borrow?, as_str?, drop?);

        let mut carriers = vec![CarrierLocalFacts::default(); self.body.local_decls.len()];
        let mut aliases = vec![CarrierAliasFacts::default(); self.body.local_decls.len()];
        let carrier_slot = carriers.get_mut(carrier.as_usize())?;
        carrier_slot.shared_alias = Some(alias);
        let mut collector = CarrierUseCollector {
            carriers: &mut carriers,
            aliases: &mut aliases,
            producer_destinations: BTreeSet::from([carrier]),
            allowed_borrows: BTreeSet::from([(carrier, alias)]),
            allowed_as_str_arguments: BTreeSet::from([alias]),
            allowed_drops: BTreeSet::from([carrier]),
            cleanup_blocks: self
                .body
                .basic_blocks
                .iter_enumerated()
                .filter_map(|(block, data)| data.is_cleanup.then_some(block))
                .collect(),
        };
        collector.visit_body(self.body);
        let carrier_facts = carriers[carrier.as_usize()];
        let alias_facts = aliases[alias.as_usize()];
        if carrier_facts
            != (CarrierLocalFacts {
                definitions: 1,
                shared_borrows: 1,
                shared_alias: Some(alias),
                drops: 1,
                invalid: false,
            })
            || alias_facts
                != (CarrierAliasFacts {
                    definitions: 1,
                    as_str_arguments: 1,
                    invalid: false,
                })
        {
            return None;
        }
        Some(CarrierBridge {
            producer,
            as_str,
            drop,
        })
    }

    fn direct_item(&self, function: &mir::Operand<'tcx>) -> Option<DirectItemRef> {
        let function_ty = monomorphize(
            self.cgu.tcx,
            self.instance,
            function.ty(&self.body.local_decls, self.cgu.tcx),
        );
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return None;
        };
        if self.cgu.tcx.is_foreign_item(definition) {
            return None;
        }
        let arguments = arguments.no_bound_vars()?;
        let resolved =
            Instance::try_resolve(self.cgu.tcx, typing_env(), definition, arguments).ok()??;
        Some(DirectItemRef {
            item: item_id(self.cgu, resolved),
            link_name: self.cgu.namer.fn_name(resolved).into_string(),
        })
    }

    fn call_operation(&self, function: &mir::Operand<'tcx>) -> Option<StandardOperation> {
        let function_ty = monomorphize(
            self.cgu.tcx,
            self.instance,
            function.ty(&self.body.local_decls, self.cgu.tcx),
        );
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return None;
        };
        let arguments = arguments.no_bound_vars()?;
        let resolved =
            Instance::try_resolve(self.cgu.tcx, typing_env(), definition, arguments).ok()??;
        standard_operation(self.cgu.tcx, resolved.def_id(), resolved.args)
    }

    fn drop_item(&self, place: mir::Place<'tcx>) -> Option<DirectItemRef> {
        let ty = monomorphize(
            self.cgu.tcx,
            self.instance,
            place.ty(&self.body.local_decls, self.cgu.tcx).ty,
        );
        if !self.is_global_string(ty) {
            return None;
        }
        let glue = Instance::resolve_drop_glue(self.cgu.tcx, ty);
        if matches!(
            glue.def,
            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None))
        ) {
            return None;
        }
        Some(DirectItemRef {
            item: item_id(self.cgu, glue),
            link_name: self.cgu.namer.fn_name(glue).into_string(),
        })
    }

    fn is_global_string(&self, ty: Ty<'tcx>) -> bool {
        let ty = monomorphize(self.cgu.tcx, self.instance, ty);
        // The language String has no allocator parameter. Its backing Vec always uses Global.
        matches!(ty.kind(), ty::Adt(definition, _)
            if self.cgu.tcx.lang_items().string() == Some(definition.did()))
    }

    fn baseline_compatible_argument(&self, operand: &mir::Operand<'tcx>) -> bool {
        let ty = monomorphize(
            self.cgu.tcx,
            self.instance,
            operand.ty(&self.body.local_decls, self.cgu.tcx),
        );
        matches!(
            ty.kind(),
            ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Never
        ) || matches!(ty.kind(), ty::Tuple(fields) if fields.is_empty())
            || matches!(ty.kind(), ty::Ref(_, pointee, rustc_hir::Mutability::Not) if pointee.is_str())
    }
}

fn operand_root_local(operand: &mir::Operand<'_>) -> Option<mir::Local> {
    match operand {
        mir::Operand::Copy(place) | mir::Operand::Move(place) => place.as_local(),
        mir::Operand::Constant(_) | mir::Operand::RuntimeChecks(_) => None,
    }
}

fn exact_normal_call(target: bool, unwind: &mir::UnwindAction) -> bool {
    target && matches!(unwind, mir::UnwindAction::Unreachable)
}

fn unique_direct_function_use(occurrences: Option<&u32>) -> bool {
    occurrences == Some(&1)
}

#[cfg(test)]
fn certify_carrier_flow_for_test(
    locals: usize,
    carrier: usize,
    alias: usize,
    events: &[(usize, CarrierFlowEvent)],
) -> bool {
    let mut carriers = vec![CarrierLocalFacts::default(); locals];
    let mut aliases = vec![CarrierAliasFacts::default(); locals];
    let Some(carrier_facts) = carriers.get_mut(carrier) else {
        return false;
    };
    carrier_facts.shared_alias = u32::try_from(alias).ok().map(mir::Local::from_u32);
    for &(local, event) in events {
        match event {
            CarrierFlowEvent::Producer => {
                let Some(facts) = carriers.get_mut(local) else {
                    return false;
                };
                facts.definitions += 1;
            }
            CarrierFlowEvent::SharedBorrow => {
                let Some(facts) = carriers.get_mut(local) else {
                    return false;
                };
                facts.shared_borrows += 1;
            }
            CarrierFlowEvent::Drop => {
                let Some(facts) = carriers.get_mut(local) else {
                    return false;
                };
                facts.drops += 1;
            }
            CarrierFlowEvent::AliasDefinition => {
                let Some(facts) = aliases.get_mut(local) else {
                    return false;
                };
                facts.definitions += 1;
            }
            CarrierFlowEvent::AsStrArgument => {
                let Some(facts) = aliases.get_mut(local) else {
                    return false;
                };
                facts.as_str_arguments += 1;
            }
            CarrierFlowEvent::OtherCarrierUse
            | CarrierFlowEvent::ProjectedCarrierUse
            | CarrierFlowEvent::CleanupUse => {
                let Some(facts) = carriers.get_mut(local) else {
                    return false;
                };
                facts.invalid = true;
            }
            CarrierFlowEvent::OtherAliasUse => {
                let Some(facts) = aliases.get_mut(local) else {
                    return false;
                };
                facts.invalid = true;
            }
        }
    }
    let Some(alias_local) = u32::try_from(alias).ok().map(mir::Local::from_u32) else {
        return false;
    };
    carriers.get(carrier)
        == Some(&CarrierLocalFacts {
            definitions: 1,
            shared_borrows: 1,
            shared_alias: Some(alias_local),
            drops: 1,
            invalid: false,
        })
        && aliases.get(alias)
            == Some(&CarrierAliasFacts {
                definitions: 1,
                as_str_arguments: 1,
                invalid: false,
            })
}
impl<'a, 'tcx> Encoder<'a, 'tcx> {
    fn new(cgu: &'a CguCx<'tcx>, instance: Instance<'tcx>, mir: &'a mir::Body<'tcx>) -> Self {
        Self {
            cgu,
            instance,
            mir,
            types: BTreeMap::new(),
            type_metadata: BTreeMap::new(),
            type_names: BTreeMap::new(),
            locations: Vec::new(),
        }
    }

    fn root_diagnostics(&self) -> DiagnosticInfo {
        DiagnosticInfo {
            item_path: Some(with_no_trimmed_paths!(self
                .cgu
                .tcx
                .def_path_str(self.instance.def_id()))),
            root: source(self.cgu.tcx, self.mir.span)
                .or_else(|| source(self.cgu.tcx, self.cgu.tcx.def_span(self.instance.def_id()))),
            type_names: Vec::new(),
            locations: Vec::new(),
        }
    }

    fn encode(&mut self) -> Result<(LateBody, DiagnosticInfo), String> {
        let (item, semantic, signature, locals) = self.header()?;

        let mut blocks = Vec::with_capacity(self.mir.basic_blocks.len());
        for (block, data) in self.mir.basic_blocks.iter_enumerated() {
            blocks.push(self.basic_block(block, data)?);
        }

        self.finish(item, semantic, signature, locals, blocks)
    }

    fn semantic_header(&mut self) -> Result<(LateBody, DiagnosticInfo), String> {
        let (item, semantic, signature, locals) = self.header()?;
        self.finish(item, semantic, signature, locals, Vec::new())
    }

    fn header(&mut self) -> Result<(ItemId, BodySemantic, Signature, Vec<Local>), String> {
        let mut locals = Vec::with_capacity(self.mir.local_decls.len());
        for (local, declaration) in self.mir.local_decls.iter_enumerated() {
            let id = LocalId(local.as_usize() as u32);
            let ty = self.type_id(declaration.ty)?;
            locals.push(Local {
                id,
                role: if local == mir::RETURN_PLACE {
                    LocalRole::Return
                } else if local.as_usize() <= self.mir.arg_count {
                    LocalRole::Argument
                } else {
                    LocalRole::Temporary
                },
                ty,
            });
            self.record(DiagnosticSite::Local(id), declaration.source_info.span);
        }

        let signature = Signature {
            arguments: locals
                .iter()
                .filter(|local| local.role == LocalRole::Argument)
                .map(|local| local.ty)
                .collect(),
            result: locals[mir::RETURN_PLACE.as_usize()].ty,
            abi: self.abi_signature()?,
        };
        Ok((
            item_id(self.cgu, self.instance),
            self.body_semantic()?,
            signature,
            locals,
        ))
    }

    fn finish(
        &mut self,
        item: ItemId,
        semantic: BodySemantic,
        signature: Signature,
        locals: Vec<Local>,
        blocks: Vec<BasicBlock>,
    ) -> Result<(LateBody, DiagnosticInfo), String> {
        let body = LateBody {
            item,
            kind: BodyKind::Function,
            semantic,
            signature,
            locals,
            types: self
                .types
                .iter()
                .map(|(id, kind)| TypeDefinition {
                    id: *id,
                    kind: kind.clone(),
                    metadata: self.type_metadata[id],
                })
                .collect(),
            blocks,
        };
        let mut diagnostics = self.root_diagnostics();
        diagnostics.type_names = self
            .type_names
            .iter()
            .map(|(id, name)| TypeDiagnostic {
                id: *id,
                name: name.clone(),
            })
            .collect();
        diagnostics.locations = self.locations.clone();
        diagnostics.normalize();
        if body.blocks.is_empty() && matches!(body.semantic, BodySemantic::Standard(_)) {
            body.validate_semantic_header()?;
        } else {
            body.validate()?;
        }
        Ok((body, diagnostics))
    }

    fn abi_signature(&self) -> Result<AbiSignature, String> {
        let spread = match self.mir.spread_arg {
            Some(local) => {
                let ty = monomorphize(self.cgu.tcx, self.instance, self.mir.local_decls[local].ty);
                let ty::Tuple(elements) = ty.kind() else {
                    return Err("rust-call spread argument is not a tuple".to_owned());
                };
                Some(SpreadArgument {
                    local: LocalId(local.as_usize() as u32),
                    arity: u16::try_from(elements.len())
                        .map_err(|_| "rust-call spread argument has too many elements")?,
                })
            }
            None => None,
        };
        let caller_location = self.instance.def.requires_caller_location(self.cgu.tcx);
        let parameters = self
            .mir
            .arg_count
            .checked_sub(usize::from(spread.is_some()))
            .and_then(|count| count.checked_add(spread.map_or(0, |spread| spread.arity.into())))
            .and_then(|count| count.checked_add(usize::from(caller_location)))
            .and_then(|count| u16::try_from(count).ok())
            .ok_or_else(|| "function has too many JavaScript parameters".to_owned())?;
        Ok(AbiSignature {
            parameters,
            spread,
            caller_location,
        })
    }

    fn body_semantic(&mut self) -> Result<BodySemantic, String> {
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, Some(ty))) = self.instance.def {
            return Ok(BodySemantic::DropGlue(self.type_id(ty)?));
        }
        Ok(
            standard_operation(self.cgu.tcx, self.instance.def_id(), self.instance.args)
                .map_or(BodySemantic::Ordinary, BodySemantic::Standard),
        )
    }

    fn basic_block(
        &mut self,
        block: mir::BasicBlock,
        data: &mir::BasicBlockData<'tcx>,
    ) -> Result<BasicBlock, String> {
        let id = BlockId(block.as_usize() as u32);
        let mut statements = Vec::with_capacity(data.statements.len());
        for (index, statement) in data.statements.iter().enumerate() {
            self.record(
                DiagnosticSite::Statement {
                    block: id,
                    statement: index as u32,
                },
                statement.source_info.span,
            );
            let kind = self
                .statement(&statement.kind)
                .map_err(|reason| format!("block {} statement {index}: {reason}", id.0))?;
            statements.push(Statement {
                index: index as u32,
                kind,
            });
        }
        self.record(
            DiagnosticSite::Terminator(id),
            data.terminator().source_info.span,
        );
        let terminator = self
            .terminator(&data.terminator().kind)
            .map_err(|reason| format!("block {} terminator: {reason}", id.0))?;
        Ok(BasicBlock {
            id,
            cleanup: data.is_cleanup,
            statements,
            terminator,
        })
    }

    fn statement(&mut self, statement: &mir::StatementKind<'tcx>) -> Result<StatementKind, String> {
        Ok(match statement {
            mir::StatementKind::Assign(assignment) => StatementKind::Assign {
                destination: self.place(assignment.0)?,
                value: self.rvalue(&assignment.1)?,
            },
            mir::StatementKind::SetDiscriminant {
                place,
                variant_index,
            } => StatementKind::SetDiscriminant {
                place: self.place(**place)?,
                variant: variant_index.as_usize() as u32,
            },
            mir::StatementKind::StorageLive(local) => {
                StatementKind::StorageLive(LocalId(local.as_usize() as u32))
            }
            mir::StatementKind::StorageDead(local) => {
                StatementKind::StorageDead(LocalId(local.as_usize() as u32))
            }
            mir::StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
                mir::NonDivergingIntrinsic::Assume(value) => {
                    StatementKind::Assume(self.operand(value)?)
                }
                mir::NonDivergingIntrinsic::CopyNonOverlapping(copy) => {
                    StatementKind::CopyNonOverlapping {
                        source: self.operand(&copy.src)?,
                        destination: self.operand(&copy.dst)?,
                        count: self.operand(&copy.count)?,
                    }
                }
            },
            mir::StatementKind::FakeRead(..)
            | mir::StatementKind::PlaceMention(..)
            | mir::StatementKind::AscribeUserType(..)
            | mir::StatementKind::Coverage(..)
            | mir::StatementKind::ConstEvalCounter
            | mir::StatementKind::Nop
            | mir::StatementKind::BackwardIncompatibleDropHint { .. } => StatementKind::Nop,
        })
    }

    fn rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>) -> Result<Rvalue, String> {
        Ok(match rvalue {
            mir::Rvalue::Use(value, _) => Rvalue::Use(self.operand(value)?),
            mir::Rvalue::Repeat(value, count) => Rvalue::Repeat {
                value: self.operand(value)?,
                count: monomorphize(self.cgu.tcx, self.instance, *count)
                    .try_to_target_usize(self.cgu.tcx)
                    .ok_or_else(|| "array repeat count is not a target usize".to_owned())?,
            },
            mir::Rvalue::Ref(_, kind, place) => Rvalue::Reference {
                kind: borrow_kind(*kind),
                place: self.place(*place)?,
            },
            mir::Rvalue::RawPtr(kind, place) => Rvalue::RawPointer {
                kind: raw_pointer_kind(*kind),
                place: self.place(*place)?,
            },
            mir::Rvalue::Cast(kind, value, result) => Rvalue::Cast {
                kind: cast_kind(*kind),
                value: self.operand(value)?,
                result: self.type_id(*result)?,
            },
            mir::Rvalue::BinaryOp(op, operands) => Rvalue::Binary {
                op: binary_op(*op),
                left: self.operand(&operands.0)?,
                right: self.operand(&operands.1)?,
            },
            mir::Rvalue::UnaryOp(op, value) => Rvalue::Unary {
                op: unary_op(*op),
                value: self.operand(value)?,
            },
            mir::Rvalue::Discriminant(place) => Rvalue::Discriminant(self.place(*place)?),
            mir::Rvalue::Aggregate(kind, operands) => Rvalue::Aggregate {
                kind: self.aggregate_kind(kind)?,
                result: self.type_id(rvalue.ty(&self.mir.local_decls, self.cgu.tcx))?,
                operands: operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect::<Result<_, _>>()?,
            },
            mir::Rvalue::CopyForDeref(place) => Rvalue::CopyForDeref(self.place(*place)?),
            mir::Rvalue::WrapUnsafeBinder(value, _) => {
                Rvalue::WrapUnsafeBinder(self.operand(value)?)
            }
            mir::Rvalue::Reborrow(_, mutable, place) => Rvalue::Reborrow {
                mutable: matches!(mutable, rustc_hir::Mutability::Mut),
                place: self.place(*place)?,
            },
            mir::Rvalue::ThreadLocalRef(_) => {
                return Err("thread-local references are not representable".to_owned());
            }
        })
    }

    fn aggregate_kind(&mut self, kind: &mir::AggregateKind<'tcx>) -> Result<AggregateKind, String> {
        Ok(match kind {
            mir::AggregateKind::Array(element) => AggregateKind::Array(self.type_id(*element)?),
            mir::AggregateKind::Tuple => AggregateKind::Tuple,
            mir::AggregateKind::Adt(definition, variant, _, _, active_field) => {
                AggregateKind::Adt {
                    definition: definition_id(self.cgu.tcx, *definition),
                    variant: variant.as_usize() as u32,
                    active_field: active_field.map(|field| field.as_usize() as u32),
                }
            }
            mir::AggregateKind::Closure(definition, _) => {
                AggregateKind::Closure(definition_id(self.cgu.tcx, *definition))
            }
            mir::AggregateKind::Coroutine(definition, _) => {
                AggregateKind::Coroutine(definition_id(self.cgu.tcx, *definition))
            }
            mir::AggregateKind::CoroutineClosure(definition, _) => {
                AggregateKind::CoroutineClosure(definition_id(self.cgu.tcx, *definition))
            }
            mir::AggregateKind::RawPtr(pointee, mutable) => AggregateKind::RawPointer {
                pointee: self.type_id(*pointee)?,
                mutable: matches!(mutable, rustc_hir::Mutability::Mut),
            },
        })
    }

    fn terminator(&mut self, terminator: &mir::TerminatorKind<'tcx>) -> Result<Terminator, String> {
        Ok(match terminator {
            mir::TerminatorKind::Goto { target } => Terminator::Goto(block_id(*target)),
            mir::TerminatorKind::SwitchInt { discr, targets } => Terminator::Switch {
                discriminant: self.operand(discr)?,
                targets: targets
                    .iter()
                    .map(|(value, target)| (value, block_id(target)))
                    .collect(),
                otherwise: block_id(targets.otherwise()),
            },
            mir::TerminatorKind::Return => Terminator::Return,
            mir::TerminatorKind::Unreachable => Terminator::Unreachable,
            mir::TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                ..
            } => Terminator::Call(Call {
                function: self.operand(func)?,
                callee: self.callee(func)?,
                semantic: self.call_semantic(func),
                arguments: args
                    .iter()
                    .map(|argument| self.operand(&argument.node))
                    .collect::<Result<_, _>>()?,
                destination: self.place(*destination)?,
                target: target.map(block_id),
                unwind: unwind_action(*unwind),
            }),
            mir::TerminatorKind::Drop {
                place,
                target,
                unwind,
                replace,
                drop,
            } => Terminator::Drop {
                place: self.place(*place)?,
                target: block_id(*target),
                unwind: unwind_action(*unwind),
                replace: *replace,
                async_drop: drop.map(block_id),
                glue: self.drop_glue(*place),
            },
            mir::TerminatorKind::Assert {
                cond,
                expected,
                msg,
                target,
                unwind,
            } => Terminator::Assert {
                condition: self.operand(cond)?,
                expected: *expected,
                message: self.assert_message(msg)?,
                target: block_id(*target),
                unwind: unwind_action(*unwind),
            },
            mir::TerminatorKind::UnwindResume => Terminator::UnwindResume,
            mir::TerminatorKind::UnwindTerminate(reason) => {
                Terminator::UnwindTerminate(terminate_reason(*reason))
            }
            mir::TerminatorKind::FalseEdge {
                real_target,
                imaginary_target,
            } => Terminator::FalseEdge {
                real: block_id(*real_target),
                imaginary: block_id(*imaginary_target),
            },
            mir::TerminatorKind::FalseUnwind {
                real_target,
                unwind,
            } => Terminator::FalseUnwind {
                real: block_id(*real_target),
                unwind: unwind_action(*unwind),
            },
            other => return Err(format!("unsupported `{}` terminator", other.name())),
        })
    }

    fn assert_message(
        &mut self,
        message: &mir::AssertMessage<'tcx>,
    ) -> Result<AssertMessage, String> {
        Ok(match message {
            mir::AssertKind::BoundsCheck { len, index } => AssertMessage::BoundsCheck {
                length: self.operand(len)?,
                index: self.operand(index)?,
            },
            mir::AssertKind::Overflow(op, left, right) => AssertMessage::Overflow {
                op: binary_op(*op),
                left: self.operand(left)?,
                right: self.operand(right)?,
            },
            mir::AssertKind::OverflowNeg(value) => AssertMessage::OverflowNeg(self.operand(value)?),
            mir::AssertKind::DivisionByZero(value) => {
                AssertMessage::DivisionByZero(self.operand(value)?)
            }
            mir::AssertKind::RemainderByZero(value) => {
                AssertMessage::RemainderByZero(self.operand(value)?)
            }
            mir::AssertKind::ResumedAfterReturn(kind) => {
                AssertMessage::ResumedAfterReturn(coroutine_kind(*kind))
            }
            mir::AssertKind::ResumedAfterPanic(kind) => {
                AssertMessage::ResumedAfterPanic(coroutine_kind(*kind))
            }
            mir::AssertKind::ResumedAfterDrop(kind) => {
                AssertMessage::ResumedAfterDrop(coroutine_kind(*kind))
            }
            mir::AssertKind::MisalignedPointerDereference { required, found } => {
                AssertMessage::MisalignedPointer {
                    required: self.operand(required)?,
                    found: self.operand(found)?,
                }
            }
            mir::AssertKind::NullPointerDereference => AssertMessage::NullPointer,
            mir::AssertKind::NullReferenceConstructed => AssertMessage::NullReference,
            mir::AssertKind::InvalidEnumConstruction(value) => {
                AssertMessage::InvalidEnum(self.operand(value)?)
            }
        })
    }

    fn operand(&mut self, operand: &mir::Operand<'tcx>) -> Result<Operand, String> {
        let ty = self.type_id(operand.ty(&self.mir.local_decls, self.cgu.tcx))?;
        let kind = match operand {
            mir::Operand::Copy(place) => OperandKind::Copy(self.place(*place)?),
            mir::Operand::Move(place) => OperandKind::Move(self.place(*place)?),
            mir::Operand::Constant(constant) => OperandKind::Constant(self.constant(constant)?),
            mir::Operand::RuntimeChecks(check) => {
                OperandKind::RuntimeCheck(check.value(self.cgu.tcx.sess))
            }
        };
        Ok(Operand { ty, kind })
    }

    fn constant(&mut self, constant: &mir::ConstOperand<'tcx>) -> Result<Constant, String> {
        let tcx = self.cgu.tcx;
        let konst = monomorphize(tcx, self.instance, constant.const_);
        let ty = konst.ty();
        let value = konst
            .eval(tcx, typing_env(), constant.span)
            .map_err(|_| format!("could not evaluate constant of type `{ty}`"))?;
        match value {
            mir::ConstValue::ZeroSized => Ok(Constant::ZeroSized),
            mir::ConstValue::Scalar(Scalar::Int(value))
                if crate::constant::raw::is_primitive(ty)
                    || crate::constant::raw::is_pointer(ty) =>
            {
                Ok(Constant::Scalar {
                    bits: value.to_uint(value.size()),
                    bytes: value.size().bytes() as u8,
                })
            }
            // A non-primitive scalar constant (`Option<NonZero<T>>`, `Option<bool>`,
            // `Option<Ordering>`, `None::<&T>`, transparent wrappers, single-field structs,
            // single-scalar tuples/arrays/closures, fieldless enums) decodes from a scratch
            // allocation laid out for `ty`, matching `constant.rs::scalar_to_expr` (MIRES-006).
            mir::ConstValue::Scalar(Scalar::Int(value)) => {
                let scratch = crate::constant::raw::scratch_alloc(tcx, value, ty)
                    .ok_or_else(|| format!("no layout for `{ty}` in a constant"))?;
                Ok(Constant::Memory(self.const_tree(
                    &scratch,
                    rustc_abi::Size::ZERO,
                    ty,
                    0,
                )?))
            }
            // A pointer that is the one scalar of a type around it (`Option<&T>`,
            // `Option<NonNull<T>>`, `Option<fn()>`) decodes from a scratch allocation holding it,
            // provenance included, as that type stored in memory does.
            mir::ConstValue::Scalar(pointer @ Scalar::Ptr(..))
                if !crate::constant::raw::is_pointer(ty) =>
            {
                let scratch = crate::constant::raw::scratch_pointer_alloc(tcx, pointer, ty)
                    .ok_or_else(|| format!("no layout for `{ty}` in a constant"))?;
                Ok(Constant::Memory(self.const_tree(
                    &scratch,
                    rustc_abi::Size::ZERO,
                    ty,
                    0,
                )?))
            }
            mir::ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
                let (provenance, offset) = pointer.prov_and_relative_offset();
                if let GlobalAlloc::Memory(allocation) = tcx.global_alloc(provenance.alloc_id()) {
                    let pointee = ty
                        .builtin_deref(true)
                        .ok_or_else(|| format!("the pointer constant `{ty}` is not a reference"))?;
                    let allocation = allocation.inner();
                    self.check_const_memory(allocation)?;
                    return Ok(Constant::Memory(
                        self.const_memory_pointer(allocation, offset, pointee, None, 0)?,
                    ));
                }
                Ok(Constant::Pointer {
                    target: self.global_reference(provenance.alloc_id())?,
                    offset: offset.bytes(),
                })
            }
            mir::ConstValue::Slice { alloc_id, meta } if ty.peel_refs().is_str() => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err("string constant is not backed by memory".to_owned());
                };
                let text =
                    crate::constant::raw::read_str(allocation.inner(), rustc_abi::Size::ZERO, meta)
                        .map_err(|_| {
                            "string constant contains invalid UTF-8 or exceeds its allocation"
                        })?;
                Ok(Constant::String(text.to_owned()))
            }
            mir::ConstValue::Slice { alloc_id, meta } => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err("slice constant is not backed by memory".to_owned());
                };
                let ty::Slice(element) = ty.peel_refs().kind() else {
                    return Err(format!("slice constant `{ty}` has a non-slice pointee"));
                };
                let allocation = allocation.inner();
                self.check_const_memory(allocation)?;
                Ok(Constant::Memory(ConstTree::Slice(self.const_slice(
                    allocation,
                    rustc_abi::Size::ZERO,
                    *element,
                    meta,
                    0,
                )?)))
            }
            mir::ConstValue::Indirect { alloc_id, offset } => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err("indirect constant is not backed by memory".to_owned());
                };
                let allocation = allocation.inner();
                self.check_const_memory(allocation)?;
                Ok(Constant::Memory(
                    self.const_tree(allocation, offset, ty, 0)?,
                ))
            }
        }
    }

    fn check_const_memory(&self, allocation: &mir::interpret::Allocation) -> Result<(), String> {
        if allocation.mutability != rustc_middle::mir::Mutability::Not {
            return Err("mutable constant memory cannot be decoded into a value".to_owned());
        }
        if allocation.len() as u64 > MAX_CONST_MEMORY_BYTES {
            return Err("constant memory exceeds the encoded-constant size limit".to_owned());
        }
        Ok(())
    }

    /// Decodes the value a pointer into anonymous immutable memory stands for.
    ///
    /// A `str` pointee is its text and a slice pointee is its decoded elements; both take their
    /// length from the wide pointer's metadata. Every other pointee decodes at its own type behind
    /// a [`ConstTree::Reference`].
    fn const_memory_pointer(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        pointee: Ty<'tcx>,
        meta: Option<u64>,
        depth: u32,
    ) -> Result<ConstTree, String> {
        if pointee.is_str() {
            let length = meta.ok_or_else(|| "a `&str` constant has no length".to_owned())?;
            let text = crate::constant::raw::read_str(allocation, offset, length)
                .map_err(|_| "string constant contains invalid UTF-8 or exceeds its allocation")?;
            return Ok(ConstTree::Str(text.to_owned()));
        }
        if let ty::Slice(element) = pointee.kind() {
            let length = meta.ok_or_else(|| format!("a `&{pointee}` constant has no length"))?;
            return Ok(ConstTree::Slice(
                self.const_slice(allocation, offset, *element, length, depth)?,
            ));
        }
        Ok(ConstTree::Reference(Box::new(
            self.const_tree(allocation, offset, pointee, depth)?,
        )))
    }

    fn const_slice(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        element: Ty<'tcx>,
        length: u64,
        depth: u32,
    ) -> Result<Vec<ConstTree>, String> {
        let tcx = self.cgu.tcx;
        if length > MAX_CONST_MEMORY_BYTES {
            return Err("slice constant exceeds the encoded-constant length limit".to_owned());
        }
        let layout = tcx
            .layout_of(typing_env().as_query_input(element))
            .map_err(|_| format!("no layout for `{element}` in a slice constant"))?;
        let stride = layout.size.bytes();
        let length = usize::try_from(length)
            .map_err(|_| "slice constant length does not fit the host".to_owned())?;
        if !crate::constant::raw::slice_fits(allocation, offset, stride, length) {
            return Err(format!(
                "a `&[{element}]` constant runs past the end of its allocation"
            ));
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

    /// Rebuilds the value of type `ty` stored at `offset` in immutable constant memory.
    ///
    /// This is the owned-IR mirror of the printed-JavaScript constant reader: the same bytes
    /// decode to the same value shape, so a later lowering of the tree cannot disagree with the
    /// baseline text.
    fn const_tree(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, String> {
        if depth >= MAX_CONST_TREE_DEPTH {
            return Err("constant memory nests deeper than the encoded-constant limit".to_owned());
        }
        let depth = depth + 1;
        let tcx = self.cgu.tcx;
        let layout = tcx
            .layout_of(typing_env().as_query_input(ty))
            .map_err(|_| format!("no layout for `{ty}` in a constant"))?;

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
                let int = crate::constant::raw::read_scalar(tcx, allocation, offset, layout.size)
                    .ok_or_else(|| format!("could not read a `{ty}` out of a constant"))?;
                Ok(ConstTree::Scalar {
                    bits: int.to_uint(int.size()),
                    bytes: int.size().bytes() as u8,
                })
            }
            ty::Pat(base, _) => self.const_tree(allocation, offset, *base, depth),
            ty::Adt(def, args) if def.is_enum() => {
                let variant_index = match crate::constant::raw::variant_index(
                    tcx, allocation, offset, layout, *def,
                ) {
                    Ok(index) => index,
                    Err(_) => {
                        return Err(format!(
                            "could not read the variant of `{ty}` out of a constant"
                        ));
                    }
                };
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
                    return Err(
                        "array constant exceeds the encoded-constant length limit".to_owned()
                    );
                }
                let element_layout = tcx
                    .layout_of(typing_env().as_query_input(*element))
                    .map_err(|_| format!("no layout for `{element}` in a constant"))?;
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
            _ => Err(format!(
                "constants of type `{ty}` cannot be encoded from an allocation"
            )),
        }
    }

    /// Decodes a pointer stored inside constant memory.
    fn const_pointer_field(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, String> {
        let tcx = self.cgu.tcx;
        let pointee = ty.builtin_deref(true);
        let wide =
            pointee.is_some_and(|pointee| crate::constant::raw::is_wide_pointee(tcx, pointee));
        let pointer = crate::constant::raw::read_pointer(tcx, allocation, offset, wide);
        if let Some(pointee) = pointee {
            let peeled_pointee =
                crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee));
            if matches!(peeled_pointee.kind(), ty::Dynamic(..)) {
                return self.const_dyn_pointer_field(&pointer, ty, depth);
            }
        }
        let Some(alloc_id) = pointer.alloc_id() else {
            if wide {
                return Err(format!(
                    "a `{ty}` in a constant is an address with no provenance"
                ));
            }
            if !pointer.readable {
                return Err(format!("a `{ty}` in a constant is uninitialized"));
            }
            return Ok(ConstTree::Address(pointer.offset.bytes()));
        };
        match tcx.global_alloc(alloc_id) {
            GlobalAlloc::Memory(target) => {
                let pointee = pointee
                    .ok_or_else(|| format!("the pointer constant `{ty}` is not a reference"))?;
                let target = target.inner();
                self.check_const_memory(target)?;
                self.const_memory_pointer(target, pointer.offset, pointee, pointer.meta, depth)
            }
            GlobalAlloc::VTable(..) => {
                Err("pointer to a vtable constant is not encoded".to_owned())
            }
            _ if wide => Err(format!(
                "a wide `{ty}` to a named global is not encoded in a constant"
            )),
            _ => Ok(ConstTree::Global {
                target: self.global_reference(alloc_id)?,
                offset: pointer.offset.bytes(),
            }),
        }
    }

    fn const_dyn_pointer_field(
        &mut self,
        pointer: &crate::constant::raw::Pointer,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, String> {
        let tcx = self.cgu.tcx;
        let Some((concrete_ty, principal)) = crate::constant::raw::read_vtable(tcx, pointer) else {
            return Err("pointer to a vtable constant is not encoded".to_owned());
        };
        let _ = crate::vtable::vtable_expr(self.cgu, concrete_ty, principal);
        let vtable = self
            .cgu
            .namer
            .vtable_name(concrete_ty, principal)
            .into_string();
        let concrete = self.type_id(concrete_ty)?;
        let data = match pointer.alloc_id() {
            None => {
                if !pointer.readable {
                    return Err(format!("a `{ty}` in a constant is uninitialized"));
                }
                ConstTree::Address(pointer.offset.bytes())
            }
            Some(alloc_id) => match tcx.global_alloc(alloc_id) {
                GlobalAlloc::Memory(target) => {
                    let target = target.inner();
                    self.check_const_memory(target)?;
                    ConstTree::Reference(Box::new(self.const_tree(
                        target,
                        pointer.offset,
                        concrete_ty,
                        depth,
                    )?))
                }
                GlobalAlloc::Static(def_id) => {
                    if !crate::constant::raw::static_matches_pointee(
                        tcx,
                        def_id,
                        pointer.offset,
                        concrete_ty,
                    ) {
                        return Err(format!(
                            "a pointer into part of a `static` is not encoded in a constant \
                             (type `{ty}`)"
                        ));
                    }
                    ConstTree::Global {
                        target: self.global_reference(alloc_id)?,
                        offset: pointer.offset.bytes(),
                    }
                }
                _ => {
                    return Err(format!(
                        "a wide `{ty}` to a named global is not encoded in a constant"
                    ));
                }
            },
        };
        Ok(ConstTree::Dyn {
            data: Box::new(data),
            concrete,
            vtable,
        })
    }

    fn global_reference(
        &mut self,
        allocation: mir::interpret::AllocId,
    ) -> Result<GlobalReference, String> {
        Ok(match self.cgu.tcx.global_alloc(allocation) {
            GlobalAlloc::Function { instance } => GlobalReference::Function {
                item: item_id(self.cgu, instance),
                link_name: self.cgu.namer.fn_name(instance).into_string(),
            },
            GlobalAlloc::Static(definition) => {
                GlobalReference::Static(definition_id(self.cgu.tcx, definition))
            }
            GlobalAlloc::TypeId { ty } => GlobalReference::Type(self.type_id(ty)?),
            GlobalAlloc::Memory(_) => {
                return Err("pointer to anonymous constant memory is not encoded yet".to_owned());
            }
            GlobalAlloc::VTable(..) => {
                return Err("pointer to a vtable constant is not encoded yet".to_owned());
            }
        })
    }

    fn place(&mut self, place: mir::Place<'tcx>) -> Result<Place, String> {
        let mut projection = Vec::with_capacity(place.projection.len());
        for element in place.projection {
            projection.push(match element {
                mir::PlaceElem::Deref => Projection::Deref,
                mir::PlaceElem::Field(field, ty) => Projection::Field {
                    field: field.as_usize() as u32,
                    ty: self.type_id(ty)?,
                },
                mir::PlaceElem::Index(local) => Projection::Index(LocalId(local.as_usize() as u32)),
                mir::PlaceElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => Projection::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                },
                mir::PlaceElem::Subslice { from, to, from_end } => {
                    Projection::Subslice { from, to, from_end }
                }
                mir::PlaceElem::Downcast(_, variant) => Projection::Downcast {
                    variant: variant.as_usize() as u32,
                },
                mir::PlaceElem::OpaqueCast(ty) => Projection::OpaqueCast(self.type_id(ty)?),
                mir::PlaceElem::UnwrapUnsafeBinder(ty) => {
                    Projection::UnwrapUnsafeBinder(self.type_id(ty)?)
                }
            });
        }
        Ok(Place {
            local: LocalId(place.local.as_usize() as u32),
            projection,
            ty: self.type_id(place.ty(&self.mir.local_decls, self.cgu.tcx).ty)?,
        })
    }

    fn callee(&mut self, function: &mir::Operand<'tcx>) -> Result<Callee, String> {
        let tcx = self.cgu.tcx;
        let function_ty = monomorphize(tcx, self.instance, function.ty(&self.mir.local_decls, tcx));
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return Ok(Callee::Indirect {
                signature: self.type_id(function_ty)?,
            });
        };
        if tcx.is_foreign_item(definition) {
            return Ok(Callee::Foreign {
                definition: definition_id(tcx, definition),
                symbol: tcx
                    .symbol_name(Instance::mono(tcx, definition))
                    .name
                    .to_owned(),
            });
        }
        let Some(arguments) = arguments.no_bound_vars() else {
            return Ok(Callee::Indirect {
                signature: self.type_id(function_ty)?,
            });
        };
        match Instance::try_resolve(tcx, typing_env(), definition, arguments) {
            Ok(Some(instance)) => Ok(Callee::Direct {
                item: item_id(self.cgu, instance),
                link_name: self.cgu.namer.fn_name(instance).into_string(),
            }),
            Ok(None) | Err(_) => Ok(Callee::Indirect {
                signature: self.type_id(function_ty)?,
            }),
        }
    }

    fn call_semantic(&self, function: &mir::Operand<'tcx>) -> CallSemantic {
        let tcx = self.cgu.tcx;
        let function_ty = monomorphize(tcx, self.instance, function.ty(&self.mir.local_decls, tcx));
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return CallSemantic::Ordinary;
        };
        let Some(arguments) = arguments.no_bound_vars() else {
            return CallSemantic::Ordinary;
        };
        // MIR may name a trait method while codegen dispatches its resolved impl instance. Native
        // semantics belong to the monomorphic body that will actually be called, not to the
        // unresolved trait declaration (for example `Vec<T>::deref_mut`).
        if let Ok(Some(instance)) = Instance::try_resolve(tcx, typing_env(), definition, arguments)
        {
            if let Some(operation) = standard_operation(tcx, instance.def_id(), instance.args) {
                return call_semantic_for_operation(operation);
            }
        }
        if let Some(operation) = standard_operation(tcx, definition, arguments) {
            return call_semantic_for_operation(operation);
        }
        if crate::js_extern::extern_section(tcx, definition).is_some() {
            return CallSemantic::Host;
        }
        if crate::js_block::block_section(tcx, definition).is_some() {
            return CallSemantic::RawJavaScript;
        }
        if tcx
            .codegen_fn_attrs(definition)
            .link_section
            .is_some_and(|section| section.as_str().starts_with("rcgjs.tc."))
        {
            return CallSemantic::ViewAbi;
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

    fn drop_glue(&self, place: mir::Place<'tcx>) -> DropGlue {
        let tcx = self.cgu.tcx;
        let ty = monomorphize(tcx, self.instance, place.ty(&self.mir.local_decls, tcx).ty);
        let glue = Instance::resolve_drop_glue(tcx, ty);
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) = glue.def {
            DropGlue::None
        } else if matches!(ty.kind(), ty::Dynamic(..)) {
            DropGlue::Dynamic
        } else {
            DropGlue::Direct {
                item: item_id(self.cgu, glue),
                link_name: self.cgu.namer.fn_name(glue).into_string(),
            }
        }
    }

    fn type_id(&mut self, ty: Ty<'tcx>) -> Result<TypeId, String> {
        let tcx = self.cgu.tcx;
        let ty = monomorphize(tcx, self.instance, ty);
        let id = TypeId(tcx.type_id_hash(ty).as_u128());
        if self.types.contains_key(&id) {
            return Ok(id);
        }

        // Insert first so recursive nominal arguments cannot recurse forever.
        self.types.insert(id, TypeKind::Opaque);
        self.type_metadata.insert(id, type_metadata(tcx, ty));
        self.type_names
            .insert(id, with_no_trimmed_paths!(ty.to_string()));
        let kind = match ty.kind() {
            ty::Bool => TypeKind::Bool,
            ty::Char => TypeKind::Char,
            ty::Int(_) | ty::Uint(_) => {
                let (signed, bits) = crate::value::int_info(tcx, ty)
                    .ok_or_else(|| format!("could not determine integer width for `{ty}`"))?;
                TypeKind::Integer { signed, bits }
            }
            ty::Float(ty::FloatTy::F32) => TypeKind::Float { bits: 32 },
            ty::Float(ty::FloatTy::F64) => TypeKind::Float { bits: 64 },
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
                    .ok_or_else(|| format!("array length for `{ty}` is not a target usize"))?,
            },
            ty::Slice(element) => TypeKind::Slice(self.type_id(*element)?),
            ty::Ref(_, pointee, mutable) => TypeKind::Reference {
                pointee: self.type_id(*pointee)?,
                mutable: matches!(mutable, rustc_hir::Mutability::Mut),
            },
            ty::RawPtr(pointee, mutable) => TypeKind::RawPointer {
                pointee: self.type_id(*pointee)?,
                mutable: matches!(mutable, rustc_hir::Mutability::Mut),
            },
            ty::Adt(definition, arguments) => self.adt_type(ty, *definition, arguments)?,
            ty::Foreign(definition) => TypeKind::Nominal {
                kind: NominalKind::Foreign,
                definition: definition_id(tcx, *definition),
                arguments: Vec::new(),
                form: NominalForm::Opaque,
                variants: Vec::new(),
            },
            ty::FnDef(definition, arguments) => {
                self.nominal_type(NominalKind::Function, *definition, arguments.skip_binder())?
            }
            ty::Closure(definition, arguments) => {
                self.nominal_type(NominalKind::Closure, *definition, arguments)?
            }
            ty::Coroutine(definition, arguments) => {
                self.nominal_type(NominalKind::Coroutine, *definition, arguments)?
            }
            ty::CoroutineClosure(definition, arguments) => {
                self.nominal_type(NominalKind::CoroutineClosure, *definition, arguments)?
            }
            ty::Pat(base, _) => TypeKind::Pattern(self.type_id(*base)?),
            _ => TypeKind::Opaque,
        };
        self.types.insert(id, kind);
        Ok(id)
    }

    fn nominal_type(
        &mut self,
        kind: NominalKind,
        definition: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeKind, String> {
        let arguments = arguments
            .iter()
            .filter_map(|argument| argument.as_type())
            .map(|argument| self.type_id(argument))
            .collect::<Result<_, _>>()?;
        Ok(TypeKind::Nominal {
            kind,
            definition: definition_id(self.cgu.tcx, definition),
            arguments,
            form: NominalForm::Opaque,
            variants: Vec::new(),
        })
    }

    fn adt_type(
        &mut self,
        ty: Ty<'tcx>,
        definition: ty::AdtDef<'tcx>,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeKind, String> {
        let type_arguments = arguments
            .iter()
            .filter_map(|argument| argument.as_type())
            .map(|argument| self.type_id(argument))
            .collect::<Result<_, _>>()?;
        let direct_tag_size = if definition.is_enum() {
            self.cgu
                .tcx
                .layout_of(typing_env().as_query_input(ty))
                .ok()
                .and_then(|layout| match &layout.variants {
                    Variants::Multiple {
                        tag,
                        tag_encoding: TagEncoding::Direct,
                        ..
                    } => Some(tag.size(&self.cgu.tcx)),
                    Variants::Empty
                    | Variants::Single { .. }
                    | Variants::Multiple {
                        tag_encoding: TagEncoding::Niche { .. },
                        ..
                    } => None,
                })
        } else {
            None
        };
        let mut variants = Vec::with_capacity(definition.variants().len());
        for (variant_index, variant) in definition.variants().iter_enumerated() {
            let mut fields = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let ty = self
                    .cgu
                    .tcx
                    .normalize_erasing_regions(typing_env(), field.ty(self.cgu.tcx, arguments));
                fields.push(self.type_id(ty)?);
            }
            let discriminant = if let Some(tag_size) = direct_tag_size {
                let discriminant = ty
                    .discriminant_for_variant(self.cgu.tcx, variant_index)
                    .ok_or_else(|| {
                        format!(
                            "could not determine discriminant for variant {} of `{ty}`",
                            variant_index.as_usize()
                        )
                    })?;
                Some(
                    physical_direct_discriminant(discriminant.val, tag_size).ok_or_else(|| {
                        format!("physical enum tag for `{ty}` has an unsupported width")
                    })?,
                )
            } else {
                None
            };
            variants.push(NominalVariant {
                fields,
                discriminant,
                result_ok: self.cgu.tcx.lang_items().result_ok_variant() == Some(variant.def_id),
            });
        }
        let form = if definition.is_struct() {
            NominalForm::Struct
        } else if definition.is_enum() {
            NominalForm::Enum
        } else if definition.is_union() {
            NominalForm::Union
        } else {
            NominalForm::Opaque
        };
        Ok(TypeKind::Nominal {
            kind: NominalKind::Adt,
            definition: definition_id(self.cgu.tcx, definition.did()),
            arguments: type_arguments,
            form,
            variants,
        })
    }

    fn record(&mut self, site: DiagnosticSite, span: Span) {
        if let Some(source) = source(self.cgu.tcx, span) {
            self.locations.push(LocationDiagnostic { site, source });
        }
    }
}

fn definition_id(tcx: TyCtxt<'_>, definition: DefId) -> DefinitionId {
    let hash = tcx.def_path_hash(definition);
    DefinitionId {
        crate_id: hash.stable_crate_id().as_u64(),
        local_hash: hash.local_hash().as_u64(),
    }
}

pub(crate) fn standard_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    [
        ("vec_new", StandardOperation::VecNew),
        ("vec_with_capacity", StandardOperation::VecWithCapacity),
        ("vec_reserve", StandardOperation::VecReserve),
        ("vec_as_slice", StandardOperation::VecAsSlice),
        ("vec_as_mut_slice", StandardOperation::VecAsMutSlice),
        ("vec_pop", StandardOperation::VecPop),
        ("vec_is_empty", StandardOperation::VecIsEmpty),
        ("string_new", StandardOperation::StringNew),
        ("string_as_str", StandardOperation::StringAsStr),
        ("string_push_str", StandardOperation::StringPushStr),
    ]
    .into_iter()
    .find_map(|(name, operation)| {
        (tcx.get_diagnostic_item(Symbol::intern(name)) == Some(definition)).then_some(operation)
    })
    .or_else(|| format_to_string_operation(tcx, definition, arguments))
    .or_else(|| format_args_item_operation(tcx, definition, arguments))
    .or_else(|| {
        let associated = tcx.opt_associated_item(definition)?;
        let signature = tcx
            .fn_sig(definition)
            .instantiate(tcx, arguments)
            .skip_binder();
        let receiver = signature.inputs().first().map(|ty| ty.peel_refs());
        let result = signature.output().peel_refs();
        let symbol = associated.name();
        let name = symbol.as_str();
        let trait_id = associated
            .trait_item_def_id()
            .map(|trait_item| tcx.parent(trait_item));
        let exact_extend = associated.trait_item_def_id().is_some_and(|trait_item| {
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_extend"), trait_item)
        });
        let exact_fmt_write_fmt =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_fmt_write_fmt"), definition);
        let exact_fmt_spec_write_fmt =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_fmt_spec_write_fmt"), definition);
        let exact_spec_to_string =
            tcx.is_diagnostic_item(Symbol::intern("rcgjs_spec_to_string"), definition);
        let exact_to_string = associated.trait_item_def_id().is_some_and(|trait_item| {
            tcx.is_diagnostic_item(Symbol::intern("to_string_method"), trait_item)
        });
        let fmt_write = trait_id
            .is_some_and(|trait_id| tcx.is_diagnostic_item(Symbol::intern("FmtWrite"), trait_id));
        let inherent = matches!(associated.container, ty::AssocContainer::InherentImpl);
        // An unrelated impl can accept a standard value or return String. Only the
        // owning standard impl gives these method names their semantic identity.
        let owner = inherent.then(|| {
            tcx.normalize_erasing_regions(
                typing_env(),
                tcx.type_of(tcx.parent(definition))
                    .instantiate(tcx, arguments),
            )
        });
        let standard_impl = owner.and_then(|owner| standard_owner(tcx, owner));
        let global_string_result = tcx.lang_items().string().is_some_and(|string| {
            matches!(signature.output().kind(), ty::Adt(definition, _) if definition.did() == string)
        });
        let exact_f64_spec_to_string = f64_spec_to_string_operation(
            exact_spec_to_string,
            signature.inputs().len() == 1
                && signature.inputs().first().is_some_and(|input| {
                matches!(
                    input.kind(),
                    ty::Ref(_, pointee, rustc_hir::Mutability::Not)
                        if matches!(pointee.kind(), ty::Float(ty::FloatTy::F64))
                )
            }),
            global_string_result,
        );
        if exact_f64_spec_to_string.is_some() {
            return exact_f64_spec_to_string;
        }
        let exact_primitive_to_string = primitive_to_string_operation(
            exact_to_string,
            shared_primitive_display(signature.inputs()),
            global_string_result,
        );
        if exact_primitive_to_string.is_some() {
            return exact_primitive_to_string;
        }
        if owner.is_some_and(|owner| matches!(owner.kind(), ty::Slice(_)))
            && name == "swap"
            && receiver.is_some_and(|receiver| matches!(receiver.kind(), ty::Slice(_)))
        {
            return Some(StandardOperation::SliceSwap);
        }
        let receiver = receiver.and_then(|ty| standard_owner(tcx, ty));
        if let Some(operation) =
            fmt_adapter_operation(receiver, exact_fmt_write_fmt, exact_fmt_spec_write_fmt)
        {
            return Some(operation);
        }
        if let Some(operation) = fmt_write_operation(receiver, fmt_write, name) {
            return Some(operation);
        }
        match receiver {
            Some(RepresentationCandidate::Vec)
                if standard_impl == Some(RepresentationCandidate::Vec) => match name {
                "len" => Some(StandardOperation::VecLen),
                "capacity" => Some(StandardOperation::VecCapacity),
                "push" => Some(StandardOperation::VecPush),
                "clear" => Some(StandardOperation::VecClear),
                "remove" => Some(StandardOperation::VecRemove),
                "swap" => Some(StandardOperation::VecSwap),
                _ => None,
            },
            Some(RepresentationCandidate::Vec)
                if trait_id == tcx.lang_items().index_trait() && name == "index" =>
            {
                Some(StandardOperation::VecIndex)
            }
            Some(RepresentationCandidate::Vec)
                if trait_id == tcx.lang_items().index_mut_trait() && name == "index_mut" =>
            {
                Some(StandardOperation::VecIndexMut)
            }
            Some(RepresentationCandidate::Vec)
                if trait_id == tcx.lang_items().deref_trait() && name == "deref" =>
            {
                Some(StandardOperation::VecAsSlice)
            }
            Some(RepresentationCandidate::Vec)
                if trait_id == tcx.lang_items().deref_mut_trait() && name == "deref_mut" =>
            {
                Some(StandardOperation::VecAsMutSlice)
            }
            Some(RepresentationCandidate::Vec)
                if trait_id == tcx.lang_items().drop_trait() && name == "drop" =>
            {
                Some(StandardOperation::VecDrop)
            }
            Some(RepresentationCandidate::Vec) if exact_extend => {
                Some(StandardOperation::VecExtend)
            }
            Some(RepresentationCandidate::String)
                if standard_impl == Some(RepresentationCandidate::String) => match name {
                "len" => Some(StandardOperation::StringLen),
                "capacity" => Some(StandardOperation::StringCapacity),
                "push" => Some(StandardOperation::StringPush),
                "push_str" => Some(StandardOperation::StringPushStr),
                "clear" => Some(StandardOperation::StringClear),
                "as_str" => Some(StandardOperation::StringAsStr),
                _ => None,
            },
            Some(RepresentationCandidate::String)
                if trait_id == tcx.lang_items().deref_trait() && name == "deref" =>
            {
                Some(StandardOperation::StringAsStr)
            }
            Some(RepresentationCandidate::String)
                if trait_id == tcx.lang_items().drop_trait() && name == "drop" =>
            {
                Some(StandardOperation::StringDrop)
            }
            _ => match standard_owner(tcx, result) {
                Some(RepresentationCandidate::String)
                    if standard_impl == Some(RepresentationCandidate::String)
                        && name == "with_capacity" => {
                    Some(StandardOperation::StringWithCapacity)
                }
                _ => None,
            },
        }
    })
}

fn f64_spec_to_string_operation(
    exact_marker: bool,
    shared_f64_input: bool,
    global_string_result: bool,
) -> Option<StandardOperation> {
    (exact_marker && shared_f64_input && global_string_result)
        .then_some(StandardOperation::F64SpecToString)
}

/// The primitive `Display` receivers whose `ToString` output JavaScript reproduces exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimitiveDisplay {
    Integer,
    Bool,
    Char,
}

/// The receiver class of a `fn(&P) -> String` signature, or `None` for anything else.
///
/// Only a shared reference to one fixed-width integer, `bool`, or `char` qualifies. Every other
/// receiver, including a wider reference chain, keeps the printed `Display` implementation.
fn shared_primitive_display<'tcx>(inputs: &[Ty<'tcx>]) -> Option<PrimitiveDisplay> {
    let [input] = inputs else {
        return None;
    };
    let ty::Ref(_, pointee, rustc_hir::Mutability::Not) = input.kind() else {
        return None;
    };
    match pointee.kind() {
        ty::Int(_) | ty::Uint(_) => Some(PrimitiveDisplay::Integer),
        ty::Bool => Some(PrimitiveDisplay::Bool),
        ty::Char => Some(PrimitiveDisplay::Char),
        _ => None,
    }
}

fn primitive_to_string_operation(
    exact_marker: bool,
    receiver: Option<PrimitiveDisplay>,
    global_string_result: bool,
) -> Option<StandardOperation> {
    if !exact_marker || !global_string_result {
        return None;
    }
    Some(match receiver? {
        PrimitiveDisplay::Integer => StandardOperation::IntToString,
        PrimitiveDisplay::Bool => StandardOperation::BoolToString,
        PrimitiveDisplay::Char => StandardOperation::CharToString,
    })
}

/// The exact `fn(Arguments) -> String` that owns a `format!` result.
fn call_semantic_for_operation(operation: StandardOperation) -> CallSemantic {
    // Proven format sinks are inlined by the language kernel. Keeping those calls Ordinary
    // lets a user function join a native String region without staging `format` itself.
    match operation {
        StandardOperation::FormatArgsToString
        | StandardOperation::FormatArgsNew
        | StandardOperation::FormatArgumentNewDisplay => CallSemantic::Ordinary,
        other => CallSemantic::Standard(other),
    }
}

fn format_to_string_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    if tcx.opt_item_name(definition) != Some(Symbol::intern("format"))
        || tcx.crate_name(definition.krate).as_str() != "alloc"
    {
        return None;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let [input] = signature.inputs() else {
        return None;
    };
    let ty::Adt(result, _) = *signature.output().kind() else {
        return None;
    };
    let ty::Adt(arguments_ty, _) = *input.kind() else {
        return None;
    };
    (tcx.lang_items().string() == Some(result.did())
        && tcx.is_lang_item(arguments_ty.did(), rustc_hir::LangItem::FormatArguments))
    .then_some(StandardOperation::FormatArgsToString)
}

fn format_args_item_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    format_args_associated_operation(
        tcx,
        definition,
        arguments,
        tcx.opt_item_name(definition)?.as_str(),
    )
}

fn format_args_associated_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    name: &str,
) -> Option<StandardOperation> {
    if name == "new"
        && tcx
            .get_diagnostic_item(Symbol::intern("FmtArgumentsNew"))
            .is_some_and(|item| tcx.parent(definition) == item)
    {
        return Some(StandardOperation::FormatArgsNew);
    }
    if name != "new_display" {
        return None;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let is_format_argument = matches!(
        signature.output().kind(),
        ty::Adt(result, _)
            if tcx.is_lang_item(result.did(), rustc_hir::LangItem::FormatArgument)
    );
    if !is_format_argument {
        return None;
    }
    let [input] = signature.inputs() else {
        return None;
    };
    proven_display_ty(tcx, *input).then_some(StandardOperation::FormatArgumentNewDisplay)
}

/// Default `Display` types whose text this backend already spells exactly.
fn proven_display_ty<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> bool {
    while let ty::Ref(_, pointee, _) = *ty.kind() {
        ty = pointee;
    }
    match ty.kind() {
        ty::Str | ty::Char | ty::Bool | ty::Int(_) | ty::Uint(_) => true,
        ty::Float(ty::FloatTy::F64) => true,
        ty::Adt(definition, _) => tcx.lang_items().string() == Some(definition.did()),
        _ => false,
    }
}
fn standard_owner(tcx: TyCtxt<'_>, ty: Ty<'_>) -> Option<RepresentationCandidate> {
    let ty::Adt(definition, _) = ty.kind() else {
        return None;
    };
    if tcx.is_diagnostic_item(sym::Vec, definition.did()) {
        Some(RepresentationCandidate::Vec)
    } else if tcx.lang_items().string() == Some(definition.did()) {
        Some(RepresentationCandidate::String)
    } else {
        None
    }
}

pub(crate) fn type_metadata<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> TypeMetadata {
    let (candidate, abi, boundary, allocator) = match ty.kind() {
        ty::Adt(definition, arguments) => {
            let candidate = if definition.is_box() {
                RepresentationCandidate::Box
            } else if tcx.lang_items().string() == Some(definition.did()) {
                RepresentationCandidate::String
            } else if tcx.is_diagnostic_item(sym::Vec, definition.did()) {
                RepresentationCandidate::Vec
            } else {
                RepresentationCandidate::None
            };
            let abi = if definition.repr().transparent() {
                NominalAbi::Transparent
            } else if definition.repr().c() {
                NominalAbi::C
            } else {
                NominalAbi::Rust
            };
            let crate_name = tcx.crate_name(definition.did().krate);
            let boundary = match crate_name.as_str() {
                "view_abi" | "topcoat_dom" => PinnedBoundary::ViewAbi,
                "mires_js" => PinnedBoundary::Host,
                _ => PinnedBoundary::None,
            };
            let allocator = allocator_identity(candidate, allocator_matches_global(tcx, arguments));
            (candidate, abi, boundary, allocator)
        }
        _ => (
            RepresentationCandidate::None,
            NominalAbi::Rust,
            PinnedBoundary::None,
            AllocatorIdentity::NotApplicable,
        ),
    };
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok();
    TypeMetadata {
        candidate,
        abi,
        boundary,
        sized: ty.is_sized(tcx, typing_env()),
        zero_sized: crate::value::is_zst(tcx, ty),
        needs_drop: ty.needs_drop(tcx, typing_env()),
        has_own_drop: matches!(
            ty.kind(),
            ty::Adt(definition, _)
                if definition.has_dtor(tcx) || definition.async_destructor(tcx).is_some()
        ),
        allocator,
        size_bytes: layout.map(|layout| layout.size.bytes()),
        align_bytes: layout.map(|layout| layout.align.abi.bytes()),
    }
}

fn allocator_matches_global<'tcx>(
    tcx: TyCtxt<'tcx>,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<bool> {
    let allocator = arguments.types().nth(1)?;
    let global = tcx.lang_items().global_alloc_ty()?;
    Some(matches!(
        allocator.kind(),
        ty::Adt(definition, arguments)
            if definition.did() == global && arguments.is_empty()
    ))
}

fn allocator_identity(
    candidate: RepresentationCandidate,
    matches_global: Option<bool>,
) -> AllocatorIdentity {
    match candidate {
        RepresentationCandidate::None => AllocatorIdentity::NotApplicable,
        RepresentationCandidate::String => AllocatorIdentity::Global,
        RepresentationCandidate::Box | RepresentationCandidate::Vec => match matches_global {
            Some(true) => AllocatorIdentity::Global,
            Some(false) => AllocatorIdentity::Custom,
            None => AllocatorIdentity::Unknown,
        },
    }
}

fn item_id<'tcx>(cgu: &CguCx<'tcx>, instance: Instance<'tcx>) -> ItemId {
    let tcx = cgu.tcx;
    ItemId {
        definition: definition_id(tcx, instance.def_id()),
        instantiation: tcx.type_id_hash(instance.ty(tcx, typing_env())).as_u128(),
    }
}

fn source(tcx: TyCtxt<'_>, span: Span) -> Option<SourceFact> {
    crate::names::location_of(tcx, span).map(|location| SourceFact {
        file: location.file,
        line: location.line,
        column: location.col,
    })
}

fn monomorphize<'tcx, T>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>, value: T) -> T
where
    T: TypeFoldable<TyCtxt<'tcx>> + Copy,
{
    instance.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        typing_env(),
        ty::EarlyBinder::bind(tcx, value),
    )
}
fn block_id(block: mir::BasicBlock) -> BlockId {
    BlockId(block.as_usize() as u32)
}

fn borrow_kind(kind: mir::BorrowKind) -> BorrowKind {
    match kind {
        mir::BorrowKind::Shared => BorrowKind::Shared,
        mir::BorrowKind::Fake(mir::FakeBorrowKind::Shallow) => BorrowKind::FakeShallow,
        mir::BorrowKind::Fake(mir::FakeBorrowKind::Deep) => BorrowKind::FakeDeep,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::Default,
        } => BorrowKind::Mutable,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::TwoPhaseBorrow,
        } => BorrowKind::TwoPhase,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::ClosureCapture,
        } => BorrowKind::ClosureCapture,
    }
}

fn raw_pointer_kind(kind: mir::RawPtrKind) -> RawPointerKind {
    match kind {
        mir::RawPtrKind::Const => RawPointerKind::Const,
        mir::RawPtrKind::Mut => RawPointerKind::Mutable,
        mir::RawPtrKind::FakeForPtrMetadata => RawPointerKind::FakeForMetadata,
    }
}

fn cast_kind(kind: mir::CastKind) -> CastKind {
    match kind {
        mir::CastKind::PointerExposeProvenance => CastKind::PointerExposeProvenance,
        mir::CastKind::PointerWithExposedProvenance => CastKind::PointerWithExposedProvenance,
        mir::CastKind::PointerCoercion(..) => CastKind::PointerCoercion,
        mir::CastKind::IntToInt => CastKind::IntToInt,
        mir::CastKind::FloatToInt => CastKind::FloatToInt,
        mir::CastKind::FloatToFloat => CastKind::FloatToFloat,
        mir::CastKind::IntToFloat => CastKind::IntToFloat,
        mir::CastKind::PtrToPtr => CastKind::PtrToPtr,
        mir::CastKind::FnPtrToPtr => CastKind::FnPtrToPtr,
        mir::CastKind::Transmute => CastKind::Transmute,
        mir::CastKind::Subtype => CastKind::Subtype,
    }
}

fn unary_op(op: mir::UnOp) -> UnaryOp {
    match op {
        mir::UnOp::Not => UnaryOp::Not,
        mir::UnOp::Neg => UnaryOp::Neg,
        mir::UnOp::PtrMetadata => UnaryOp::PtrMetadata,
    }
}

fn binary_op(op: mir::BinOp) -> BinaryOp {
    match op {
        mir::BinOp::Add => BinaryOp::Add,
        mir::BinOp::AddUnchecked => BinaryOp::AddUnchecked,
        mir::BinOp::AddWithOverflow => BinaryOp::AddWithOverflow,
        mir::BinOp::Sub => BinaryOp::Sub,
        mir::BinOp::SubUnchecked => BinaryOp::SubUnchecked,
        mir::BinOp::SubWithOverflow => BinaryOp::SubWithOverflow,
        mir::BinOp::Mul => BinaryOp::Mul,
        mir::BinOp::MulUnchecked => BinaryOp::MulUnchecked,
        mir::BinOp::MulWithOverflow => BinaryOp::MulWithOverflow,
        mir::BinOp::Div => BinaryOp::Div,
        mir::BinOp::Rem => BinaryOp::Rem,
        mir::BinOp::BitXor => BinaryOp::BitXor,
        mir::BinOp::BitAnd => BinaryOp::BitAnd,
        mir::BinOp::BitOr => BinaryOp::BitOr,
        mir::BinOp::Shl => BinaryOp::Shl,
        mir::BinOp::ShlUnchecked => BinaryOp::ShlUnchecked,
        mir::BinOp::Shr => BinaryOp::Shr,
        mir::BinOp::ShrUnchecked => BinaryOp::ShrUnchecked,
        mir::BinOp::Eq => BinaryOp::Eq,
        mir::BinOp::Lt => BinaryOp::Lt,
        mir::BinOp::Le => BinaryOp::Le,
        mir::BinOp::Ne => BinaryOp::Ne,
        mir::BinOp::Ge => BinaryOp::Ge,
        mir::BinOp::Gt => BinaryOp::Gt,
        mir::BinOp::Cmp => BinaryOp::Cmp,
        mir::BinOp::Offset => BinaryOp::Offset,
    }
}

fn coroutine_kind(kind: rustc_hir::CoroutineKind) -> CoroutineKind {
    match kind {
        rustc_hir::CoroutineKind::Desugared(kind, source) => CoroutineKind::Desugared {
            kind: match kind {
                rustc_hir::CoroutineDesugaring::Async => CoroutineDesugaring::Async,
                rustc_hir::CoroutineDesugaring::Gen => CoroutineDesugaring::Gen,
                rustc_hir::CoroutineDesugaring::AsyncGen => CoroutineDesugaring::AsyncGen,
            },
            source: match source {
                rustc_hir::CoroutineSource::Block => CoroutineSource::Block,
                rustc_hir::CoroutineSource::Closure => CoroutineSource::Closure,
                rustc_hir::CoroutineSource::Fn => CoroutineSource::Function,
            },
        },
        rustc_hir::CoroutineKind::Coroutine(movability) => CoroutineKind::Coroutine {
            movable: matches!(movability, rustc_hir::Movability::Movable),
        },
    }
}

fn unwind_action(action: mir::UnwindAction) -> Unwind {
    match action {
        mir::UnwindAction::Continue => Unwind::Continue,
        mir::UnwindAction::Unreachable => Unwind::Unreachable,
        mir::UnwindAction::Terminate(reason) => Unwind::Terminate(terminate_reason(reason)),
        mir::UnwindAction::Cleanup(block) => Unwind::Cleanup(block_id(block)),
    }
}

fn terminate_reason(reason: mir::UnwindTerminateReason) -> TerminateReason {
    match reason {
        mir::UnwindTerminateReason::Abi => TerminateReason::Abi,
        mir::UnwindTerminateReason::InCleanup => TerminateReason::InCleanup,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U32: TypeId = TypeId(1);

    fn sample_with(file: &str) -> LateIrAnalysisSummary {
        LateIrAnalysisSummary {
            version: LATE_IR_ANALYSIS_VERSION,
            availability: Availability::Available(LateBody {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 2,
                        local_hash: 3,
                    },
                    instantiation: 4,
                },
                kind: BodyKind::Function,
                semantic: BodySemantic::Ordinary,
                signature: Signature {
                    arguments: Vec::new(),
                    result: U32,
                    abi: AbiSignature::default(),
                },
                locals: vec![Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: U32,
                }],
                types: vec![TypeDefinition {
                    id: U32,
                    kind: TypeKind::Integer {
                        signed: false,
                        bits: 32,
                    },
                    metadata: TypeMetadata {
                        candidate: RepresentationCandidate::None,
                        abi: NominalAbi::Rust,
                        boundary: PinnedBoundary::None,
                        sized: true,
                        zero_sized: false,
                        needs_drop: false,
                        has_own_drop: false,
                        allocator: AllocatorIdentity::NotApplicable,
                        size_bytes: Some(4),
                        align_bytes: Some(4),
                    },
                }],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: vec![Statement {
                        index: 0,
                        kind: StatementKind::Assign {
                            destination: Place {
                                local: LocalId(0),
                                projection: Vec::new(),
                                ty: U32,
                            },
                            value: Rvalue::Use(Operand {
                                ty: U32,
                                kind: OperandKind::Constant(Constant::Scalar {
                                    bits: 42,
                                    bytes: 4,
                                }),
                            }),
                        },
                    }],
                    terminator: Terminator::Return,
                }],
            }),
            carrier_bridges: Vec::new(),
            diagnostics: DiagnosticInfo {
                item_path: Some("crate::answer".to_owned()),
                root: Some(SourceFact {
                    file: file.to_owned(),
                    line: 3,
                    column: 4,
                }),
                type_names: vec![TypeDiagnostic {
                    id: U32,
                    name: "u32".to_owned(),
                }],
                locations: Vec::new(),
            },
            allocator_provider: AllocatorProvider::Unknown,
            jir: None,
            jir_decline: None,
        }
    }

    fn sample() -> LateIrAnalysisSummary {
        sample_with("src/lib.rs")
    }

    fn item(local_hash: u64, link_name: &str) -> DirectItemRef {
        DirectItemRef {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash,
                },
                instantiation: u128::from(local_hash),
            },
            link_name: link_name.to_owned(),
        }
    }

    fn bridge(producer_hash: u64) -> CarrierBridge {
        CarrierBridge {
            producer: item(producer_hash, &format!("producer_{producer_hash}")),
            as_str: item(20, "string_as_str"),
            drop: item(30, "drop_string"),
        }
    }

    #[test]
    fn owned_cfg_round_trips_deterministically() {
        let summary = sample().with_carrier_bridges_for_test(vec![bridge(10)]);
        let first = summary.to_bytes().unwrap();
        let second = summary.to_bytes().unwrap();
        assert_eq!(first, second);
        assert_eq!(LateIrAnalysisSummary::from_bytes(&first).unwrap(), summary);
        assert_eq!(summary.counts().locals, 1);
        assert_eq!(summary.carrier_bridges(), &[bridge(10)]);
    }

    #[test]
    fn carrier_bridges_validate_sorted_exact_item_references() {
        let unsorted = sample().with_carrier_bridges_for_test(vec![bridge(11), bridge(10)]);
        assert_eq!(
            unsorted.validate().unwrap_err(),
            "late-IR carrier bridges are not strictly sorted"
        );

        let mut empty = bridge(10);
        empty.as_str.link_name.clear();
        assert_eq!(
            sample()
                .with_carrier_bridges_for_test(vec![empty])
                .validate()
                .unwrap_err(),
            "late-IR direct item reference has an empty link name"
        );
    }

    #[test]
    fn duplicate_merge_intersects_carrier_certificates() {
        let shared = bridge(10);
        let left_only = bridge(11);
        let right_only = bridge(12);
        let mut forward =
            sample().with_carrier_bridges_for_test(vec![shared.clone(), left_only.clone()]);
        let right = sample().with_carrier_bridges_for_test(vec![shared.clone(), right_only]);
        let mut reverse = right.clone();
        let left = sample().with_carrier_bridges_for_test(vec![shared.clone(), left_only]);

        assert!(!forward.merge(&right));
        assert!(!reverse.merge(&left));
        assert_eq!(forward.carrier_bridges(), std::slice::from_ref(&shared));
        assert_eq!(reverse.carrier_bridges(), &[shared]);

        let mut disagreed = bridge(10);
        disagreed.as_str = item(21, "other_as_str");
        let mut left = sample().with_carrier_bridges_for_test(vec![bridge(10)]);
        let right = sample().with_carrier_bridges_for_test(vec![disagreed]);
        assert!(!left.merge(&right));
        assert!(left.carrier_bridges().is_empty());
    }

    #[test]
    fn exact_string_carrier_flow_is_certified() {
        assert!(certify_carrier_flow_for_test(
            4,
            2,
            3,
            &[
                (2, CarrierFlowEvent::Producer),
                (2, CarrierFlowEvent::SharedBorrow),
                (3, CarrierFlowEvent::AliasDefinition),
                (3, CarrierFlowEvent::AsStrArgument),
                (2, CarrierFlowEvent::Drop),
            ],
        ));
    }

    #[test]
    fn string_carrier_flow_rejects_every_extra_use_kind() {
        let exact = [
            (2, CarrierFlowEvent::Producer),
            (2, CarrierFlowEvent::SharedBorrow),
            (3, CarrierFlowEvent::AliasDefinition),
            (3, CarrierFlowEvent::AsStrArgument),
            (2, CarrierFlowEvent::Drop),
        ];
        for extra in [
            (2, CarrierFlowEvent::Producer),
            (2, CarrierFlowEvent::SharedBorrow),
            (2, CarrierFlowEvent::Drop),
            (3, CarrierFlowEvent::AliasDefinition),
            (3, CarrierFlowEvent::AsStrArgument),
            (2, CarrierFlowEvent::OtherCarrierUse),
            (2, CarrierFlowEvent::ProjectedCarrierUse),
            (3, CarrierFlowEvent::OtherAliasUse),
            (2, CarrierFlowEvent::CleanupUse),
        ] {
            let mut events = exact.to_vec();
            events.push(extra);
            assert!(!certify_carrier_flow_for_test(4, 2, 3, &events));
        }
    }

    #[test]
    fn carrier_bridge_requires_exact_normal_boundaries() {
        assert!(exact_normal_call(true, &mir::UnwindAction::Unreachable));
        assert!(!exact_normal_call(false, &mir::UnwindAction::Unreachable));
        assert!(!exact_normal_call(true, &mir::UnwindAction::Continue));
        assert!(!exact_normal_call(
            true,
            &mir::UnwindAction::Terminate(mir::UnwindTerminateReason::Abi),
        ));
        assert!(!exact_normal_call(
            true,
            &mir::UnwindAction::Cleanup(mir::BasicBlock::from_u32(0)),
        ));
    }

    #[test]
    fn carrier_bridge_rejects_any_extra_producer_function_use() {
        assert!(unique_direct_function_use(Some(&1)));
        assert!(!unique_direct_function_use(Some(&2)));
        assert!(!unique_direct_function_use(None));
    }

    #[test]
    fn f64_spec_to_string_is_appended_to_the_serialized_operation_enum() {
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::F64SpecToString).unwrap(),
            vec![31]
        );
    }

    #[test]
    fn primitive_to_string_operations_are_appended_to_the_serialized_operation_enum() {
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::IntToString).unwrap(),
            vec![32]
        );
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::BoolToString).unwrap(),
            vec![33]
        );
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::CharToString).unwrap(),
            vec![34]
        );
    }

    #[test]
    fn native_format_args_operations_are_appended_to_the_serialized_operation_enum() {
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::FormatArgumentNewDisplay).unwrap(),
            vec![35]
        );
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::FormatArgsNew).unwrap(),
            vec![36]
        );
        assert_eq!(
            postcard::to_allocvec(&StandardOperation::FormatArgsToString).unwrap(),
            vec![37]
        );
    }

    #[test]
    fn default_format_template_decodes_literal_pieces_and_placeholders() {
        assert_eq!(
            decode_default_format_template(b"\x02x=\xc0\0").unwrap(),
            vec!["x=".to_owned(), String::new()]
        );
        assert_eq!(
            decode_default_format_template(b"\xc0\x02px\0").unwrap(),
            vec![String::new(), "px".to_owned()]
        );
        assert_eq!(
            decode_default_format_template(b"\x05item:\xc0\x01:\xc0\0").unwrap(),
            vec!["item:".to_owned(), ":".to_owned(), String::new()]
        );
        assert_eq!(
            decode_default_format_template(&[0]).unwrap(),
            vec![String::new()]
        );
        assert_eq!(
            decode_default_format_template(&[0xc1, 0]).unwrap_err(),
            "format template uses flags, width, precision, or explicit argument selection"
        );
    }

    #[test]
    fn const_tree_bytes_reads_a_referenced_byte_aggregate() {
        let tree = ConstTree::Reference(Box::new(ConstTree::Aggregate(vec![
            ConstTree::Scalar {
                bits: b'x' as u128,
                bytes: 1,
            },
            ConstTree::Scalar {
                bits: b'=' as u128,
                bytes: 1,
            },
        ])));
        assert_eq!(const_tree_bytes(&tree), Some(b"x=".to_vec()));
        assert_eq!(const_tree_bytes(&ConstTree::Unit), None);
    }

    #[test]
    fn own_drop_distinguishes_structural_aggregate_cleanup() {
        let structural_row = TypeMetadata {
            needs_drop: true,
            ..sample().body().unwrap().types[0].metadata
        };
        let structural_store = structural_row;
        let custom_row = TypeMetadata {
            has_own_drop: true,
            ..structural_row
        };

        assert!(structural_row.needs_drop);
        assert!(!structural_row.has_own_drop);
        assert!(structural_store.needs_drop);
        assert!(!structural_store.has_own_drop);
        assert!(custom_row.needs_drop);
        assert!(custom_row.has_own_drop);
    }

    #[test]
    fn only_exact_fmt_write_method_names_have_string_write_semantics() {
        assert_eq!(
            fmt_write_operation(Some(RepresentationCandidate::String), true, "write_str"),
            Some(StandardOperation::StringFmtWriteStr)
        );
        assert_eq!(
            fmt_write_operation(Some(RepresentationCandidate::String), true, "write_char"),
            Some(StandardOperation::StringFmtWriteChar)
        );
        assert_eq!(
            fmt_write_operation(Some(RepresentationCandidate::String), true, "write_fmt"),
            None
        );
        assert_eq!(
            fmt_write_operation(None, true, "write_str"),
            None,
            "a custom writer is not the canonical String receiver"
        );
        assert_eq!(
            fmt_write_operation(Some(RepresentationCandidate::String), false, "write_str"),
            None,
            "a method with the same name is not the canonical FmtWrite trait item"
        );
    }

    #[test]
    fn only_exact_fmt_adapter_markers_have_string_adapter_semantics() {
        assert_eq!(
            fmt_adapter_operation(Some(RepresentationCandidate::String), true, false),
            Some(StandardOperation::StringFmtWriteFmt)
        );
        assert_eq!(
            fmt_adapter_operation(Some(RepresentationCandidate::String), false, true),
            Some(StandardOperation::StringFmtSpecWriteFmt)
        );
        assert_eq!(fmt_adapter_operation(None, true, false), None);
        assert_eq!(
            fmt_adapter_operation(Some(RepresentationCandidate::String), false, false),
            None
        );
        assert_eq!(
            fmt_adapter_operation(Some(RepresentationCandidate::String), true, true),
            None,
            "conflicting semantic markers are not canonical"
        );
    }

    #[test]
    fn f64_spec_to_string_requires_the_exact_marker_type_and_abi() {
        assert_eq!(
            f64_spec_to_string_operation(true, true, true),
            Some(StandardOperation::F64SpecToString)
        );
        assert_eq!(f64_spec_to_string_operation(false, true, true), None);
        assert_eq!(
            f64_spec_to_string_operation(true, false, true),
            None,
            "a marker instantiated for f32 or a non-shared receiver must stay baseline"
        );
        assert_eq!(f64_spec_to_string_operation(true, true, false), None);
    }

    #[test]
    fn primitive_to_string_requires_the_exact_marker_receiver_and_result() {
        assert_eq!(
            primitive_to_string_operation(true, Some(PrimitiveDisplay::Integer), true),
            Some(StandardOperation::IntToString)
        );
        assert_eq!(
            primitive_to_string_operation(true, Some(PrimitiveDisplay::Bool), true),
            Some(StandardOperation::BoolToString)
        );
        assert_eq!(
            primitive_to_string_operation(true, Some(PrimitiveDisplay::Char), true),
            Some(StandardOperation::CharToString)
        );
        assert_eq!(
            primitive_to_string_operation(false, Some(PrimitiveDisplay::Integer), true),
            None,
            "a method with the same name is not the canonical ToString trait item"
        );
        assert_eq!(
            primitive_to_string_operation(true, None, true),
            None,
            "a Display receiver this backend cannot spell stays baseline"
        );
        assert_eq!(
            primitive_to_string_operation(true, Some(PrimitiveDisplay::Integer), false),
            None,
            "only the global-allocator String result is native"
        );
    }

    #[test]
    fn own_drop_fact_round_trips_in_the_versioned_schema() {
        let mut summary = sample();
        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        body.types[0].metadata.needs_drop = true;
        body.types[0].metadata.has_own_drop = true;

        let bytes = summary.to_bytes().unwrap();
        let decoded = LateIrAnalysisSummary::from_bytes(&bytes).unwrap();
        assert!(decoded.body().unwrap().types[0].metadata.has_own_drop);
    }

    #[test]
    fn physical_direct_tags_round_trip_as_exact_target_bits() {
        let mut summary = sample();
        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        body.types.push(TypeDefinition {
            id: TypeId(2),
            kind: TypeKind::Nominal {
                kind: NominalKind::Adt,
                definition: DefinitionId {
                    crate_id: 3,
                    local_hash: 5,
                },
                arguments: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    NominalVariant {
                        fields: Vec::new(),
                        discriminant: Some(NominalDiscriminant {
                            bits: u8::MAX.into(),
                            bytes: 1,
                        }),
                        result_ok: false,
                    },
                    NominalVariant {
                        fields: vec![U32],
                        discriminant: Some(NominalDiscriminant { bits: 2, bytes: 1 }),
                        result_ok: false,
                    },
                ],
            },
            metadata: TypeMetadata {
                size_bytes: Some(8),
                align_bytes: Some(4),
                ..body.types[0].metadata
            },
        });

        let first = summary.to_bytes().unwrap();
        let second = summary.to_bytes().unwrap();
        assert_eq!(first, second);
        assert_eq!(LateIrAnalysisSummary::from_bytes(&first).unwrap(), summary);
    }

    #[test]
    fn a_physical_u8_tag_truncates_a_logical_isize_discriminant() {
        assert_eq!(
            physical_direct_discriminant(u128::MAX, Size::from_bytes(1)),
            Some(NominalDiscriminant {
                bits: u8::MAX.into(),
                bytes: 1,
            })
        );
    }

    #[test]
    fn niche_encoded_enums_have_no_scalar_tag_fact() {
        let mut summary = sample();
        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        body.types.push(TypeDefinition {
            id: TypeId(2),
            kind: TypeKind::Nominal {
                kind: NominalKind::Adt,
                definition: DefinitionId {
                    crate_id: 3,
                    local_hash: 5,
                },
                arguments: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    NominalVariant {
                        fields: Vec::new(),
                        discriminant: None,
                        result_ok: false,
                    },
                    NominalVariant {
                        fields: vec![U32],
                        discriminant: None,
                        result_ok: false,
                    },
                ],
            },
            metadata: body.types[0].metadata,
        });
        assert!(summary.validate().is_ok());

        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        let TypeKind::Nominal { variants, .. } = &mut body.types[1].kind else {
            unreachable!()
        };
        variants[0].discriminant = Some(NominalDiscriminant { bits: 0, bytes: 1 });
        assert_eq!(
            summary.validate().unwrap_err(),
            "late-IR enum variants have mixed physical direct-tag facts"
        );

        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        let TypeKind::Nominal { variants, .. } = &mut body.types[1].kind else {
            unreachable!()
        };
        variants[1].discriminant = Some(NominalDiscriminant { bits: 1, bytes: 1 });
        assert!(summary.validate().is_ok());

        let Availability::Available(body) = &mut summary.availability else {
            unreachable!()
        };
        let TypeKind::Nominal { form, .. } = &mut body.types[1].kind else {
            unreachable!()
        };
        *form = NominalForm::Struct;
        assert_eq!(
            summary.validate().unwrap_err(),
            "late-IR non-enum variant has enum-only semantic metadata"
        );
    }

    #[test]
    fn root_diagnostics_attribute_link_time_optimizer_misses() {
        let source = sample().diagnostics().root.as_ref().unwrap().clone();
        let miss = source.optimization_miss("native-representations", "fallback");

        assert_eq!(miss.source.as_deref(), Some("src/lib.rs"));
        assert_eq!(miss.location.as_deref(), Some("src/lib.rs:3:5"));
    }

    #[test]
    fn stale_versions_are_rejected_after_decoding() {
        let summary = sample().with_version_for_test(LATE_IR_ANALYSIS_VERSION + 1);
        let bytes = postcard::to_allocvec(&summary).unwrap();
        assert_eq!(
            LateIrAnalysisSummary::from_bytes(&bytes).unwrap_err(),
            format!(
                "late-IR analysis summary has format {}, but this backend reads format {}",
                LATE_IR_ANALYSIS_VERSION + 1,
                LATE_IR_ANALYSIS_VERSION
            )
        );
    }

    #[test]
    fn standard_heap_types_have_semantic_allocator_identities() {
        assert_eq!(
            allocator_identity(RepresentationCandidate::Vec, Some(true)),
            AllocatorIdentity::Global
        );
        assert_eq!(
            allocator_identity(RepresentationCandidate::Box, Some(false)),
            AllocatorIdentity::Custom
        );
        assert_eq!(
            allocator_identity(RepresentationCandidate::Vec, None),
            AllocatorIdentity::Unknown
        );
        assert_eq!(
            allocator_identity(RepresentationCandidate::String, None),
            AllocatorIdentity::Global
        );
    }

    #[test]
    fn allocator_provider_merge_is_conservative_and_order_independent() {
        assert_eq!(
            AllocatorProvider::Unknown.merge(AllocatorProvider::Default),
            AllocatorProvider::Default
        );
        assert_eq!(
            AllocatorProvider::Default.merge(AllocatorProvider::Custom),
            AllocatorProvider::Conflict
        );
        assert_eq!(
            AllocatorProvider::Custom.merge(AllocatorProvider::Default),
            AllocatorProvider::Conflict
        );
        assert_eq!(
            AllocatorProvider::Conflict.merge(AllocatorProvider::Default),
            AllocatorProvider::Conflict
        );
    }

    #[test]
    fn validation_rejects_missing_cfg_and_type_references() {
        let mut missing_block = sample();
        let Availability::Available(body) = &mut missing_block.availability else {
            unreachable!()
        };
        body.blocks[0].terminator = Terminator::Goto(BlockId(1));
        assert_eq!(
            missing_block.validate().unwrap_err(),
            "late-IR references missing block 1"
        );

        let mut missing_type = sample();
        let Availability::Available(body) = &mut missing_type.availability else {
            unreachable!()
        };
        body.locals[0].ty = TypeId(99);
        assert_eq!(
            missing_type.validate().unwrap_err(),
            "late-IR local references missing type 00000000000000000000000000000063"
        );
    }

    #[test]
    fn diagnostics_do_not_make_identical_bodies_conflict() {
        let mut left = sample_with("first/src/lib.rs");
        let right = sample_with("second/src/lib.rs");
        assert!(!left.merge(&right));
        assert_eq!(left.status(), "available");
        assert_eq!(
            left.diagnostics.root.as_ref().unwrap().file,
            "first/src/lib.rs"
        );
    }

    #[test]
    fn duplicate_merge_prefers_an_available_body_in_either_order() {
        let available = sample();
        let unavailable = LateIrAnalysisSummary::unavailable("no MIR here");

        let mut left = unavailable.clone();
        assert!(!left.merge(&available));
        let mut right = available.clone();
        assert!(!right.merge(&unavailable));

        assert_eq!(left.body(), available.body());
        assert_eq!(right.body(), available.body());
    }

    #[test]
    fn duplicate_merge_prefers_a_complete_body_over_its_semantic_header() {
        let mut available = sample();
        let mut semantic = available.clone();
        let Availability::Available(mut header) = semantic.availability else {
            unreachable!()
        };
        header.blocks.clear();
        semantic.availability = Availability::Semantic(header, "unsupported CFG detail".to_owned());
        let Availability::Available(body) = &mut available.availability else {
            unreachable!()
        };
        let metadata = body.types[0].metadata;
        body.types.push(TypeDefinition {
            id: TypeId(2),
            kind: TypeKind::Bool,
            metadata: TypeMetadata {
                size_bytes: Some(1),
                align_bytes: Some(1),
                ..metadata
            },
        });

        let mut forward = semantic.clone();
        assert!(!forward.merge(&available));
        let mut reverse = available.clone();
        assert!(!reverse.merge(&semantic));

        assert_eq!(forward, available);
        assert_eq!(reverse, available);
    }

    #[test]
    fn duplicate_semantic_headers_choose_a_deterministic_reason() {
        let available = sample();
        let Availability::Available(mut header) = available.availability else {
            unreachable!()
        };
        header.blocks.clear();
        let first = LateIrAnalysisSummary {
            availability: Availability::Semantic(header.clone(), "z reason".to_owned()),
            ..sample()
        };
        let second = LateIrAnalysisSummary {
            availability: Availability::Semantic(header, "a reason".to_owned()),
            ..sample()
        };

        let mut forward = first.clone();
        assert!(!forward.merge(&second));
        let mut reverse = second.clone();
        assert!(!reverse.merge(&first));

        assert_eq!(forward, second);
        assert_eq!(reverse, second);
    }

    #[test]
    fn conflicting_duplicate_bodies_remain_a_fallback_in_any_order() {
        let first = sample();
        let mut second = sample();
        let Availability::Available(body) = &mut second.availability else {
            unreachable!()
        };
        body.signature.result = TypeId(9);

        let mut forward = first.clone();
        assert!(forward.merge(&second));
        assert!(forward.merge(&first));
        let mut reverse = second;
        assert!(reverse.merge(&first));

        assert_eq!(forward.status(), "unavailable");
        assert_eq!(reverse.status(), "unavailable");
        assert_eq!(forward.fallback_reason(), Some(DUPLICATE_CONFLICT));
        assert_eq!(reverse.fallback_reason(), Some(DUPLICATE_CONFLICT));
    }
}
