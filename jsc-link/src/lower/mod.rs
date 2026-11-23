//! The JIR-S → JavaScript lowerer: one item in, one declaration out.
//!
//! This module is the contract between the lane driver (`link/lane.rs`, W10) and the lowering
//! itself (`lower/{ssa,fuse,structure,plan}.rs` and `lower/js/*`, W6b–W9). A function runs the
//! pipeline `ssa::build → fuse::plan → structure::structure → js::function::lower`; a `static` and
//! a vtable are lowered from the body's own tree. As of the end of Phase 2 every item the driver
//! offers is lowered — 7,070 over the four fixture suites, 0 fallbacks — and the property the
//! contract exists to guarantee still holds in the other direction: a lowerer that declined
//! everything would link every program with 100 % fallback and byte-identical output to
//! `js-lane=baseline`. Both ends are pinned by `scripts/lane-shadow-test.sh` and by the unit tests
//! at the bottom of `link/lane.rs`.
//!
//! # The contract
//!
//! [`lower_item`] is **total**: every input either produces a [`LoweredItem`] or a [`Decline`].
//! It must never panic and must never emit a diagnostic — an item it cannot handle is a `Decline`
//! that the driver counts, reports and falls back from, so the program still links. The
//! declines are the vocabulary of `rcgjs_object::jir::decline`, and the lowerer's own reason is
//! [`LowerDecline::Other`], whose `op` names the class of construct that stopped it.
//!
//! # What the lowerer produces, and what the driver produces
//!
//! The lowerer produces an **AST**, never text. Printing is the driver's, for two reasons: the
//! printed form depends on link-time options the lowerer should not have to thread (`js-comments`,
//! `js-minify`, source maps), and printing is where a lane has to be identical to the baseline
//! path or a golden means nothing. So the driver builds an `rcgjs_object::item::JsItem` out of
//! what came back here and runs it through `minify::shorten_locals` and `JsItem::to_js_mapped` —
//! the same two calls the backend's own `driver.rs` makes on the baseline lane.
//!
//! What the lowerer therefore owes beyond the declaration is everything those two calls read that
//! cannot be recovered from the AST: the `names` table that maps an emitted local to the Rust
//! name behind it (the source map's `names` field), the item's own [`jsast::Loc`], its `.d.ts`
//! signature, and the zombies its body carried. `refs` is returned too, and the driver checks it
//! against the identifiers the printed declaration actually mentions: `refs` is what the
//! reachability walk runs on, so a lowering that under-reports one would drop a live item from
//! the program.
//!
//! What the driver holds equal between the lanes is not the whole of `refs` but the part of it
//! that names an item — the reachability walk's intersection with the item table, plus the
//! namer-shaped `$h` names the link step stubs. The locals are the lowerer's own business: it
//! names its temporaries `$vN` where the baseline printed `_N`, and that is the point of having
//! two lowerings rather than one.
//!
//! `dts` is `None` until the lowerer runs the type walk that produces a signature, and `None`
//! means "nothing to say": the driver keeps the baseline's rather than erasing it. `kind`,
//! `linkage` and `order` are not in [`LoweredItem`] at all, because they are what the item IS
//! and no lane may move an item in the program.

pub mod assembly;
pub mod bigint;
pub mod bstr;
pub mod fuse;
pub mod js;
pub mod plan;
pub mod semantic_ops;
pub mod ssa;
pub mod structure;

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{FlatZombie, JsName};
use crate::jir::decline::Decline;
use crate::jir::repr::Func;
use crate::jir::summary::ItemSummary;
use crate::jir::{Body, Interns, ItemId, OriginTable, TypeTable};
use crate::jsast;
use crate::opts::Opts;

/// Whether a function carries a nested native Vec/SliceView or String carrier inside an
/// aggregate, requiring `clone_expr` to handle both native and baseline shapes at `OpKind::Copy`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NestedClone {
    pub vec: bool,
    pub string: bool,
}

/// Link-time call-target overrides for one native-version caller.
pub type CallRetargets = BTreeMap<ItemId, JsName>;

/// The ordinary lowering contract: direct calls use their encoded link names.
pub static EMPTY_CALL_RETARGETS: &CallRetargets = &BTreeMap::new();

/// Everything the lowerer is given about one item.
///
/// Borrowed rather than owned: the driver holds the decoded body in the link item it is about to
/// replace, and the interns and the type table belong to the object the item was read from and
/// are shared by every item of that object.
pub struct LowerInput<'a> {
    /// The item's emitted JavaScript identifier — the name the declaration must bind.
    pub name: &'a JsName,
    /// The summary index entry: the ABI signature, the class, the linkage, the kind.
    pub summary: &'a ItemSummary,
    /// The decoded, verified JIR-S body.
    pub body: &'a Body,
    /// The body's origin table, positional with its statements. `None` when the object carried
    /// none, which is what a body encoded with origins disabled looks like.
    pub origins: Option<&'a OriginTable>,
    /// The strings, byte blobs and signatures the body's ids refer into.
    pub interns: &'a Interns,
    /// The crate-scope type table, out of the same interns. Passed separately because it is what
    /// nearly every lowering rule consults and threading `interns.types` at each use reads worse.
    pub table: &'a TypeTable,
    /// The options this link step runs under.
    pub opts: &'a Opts,
    /// Every item of the program by its [`crate::jir::semantic::ItemId`]: how the host lowering
    /// names the body of a closure a marker is handed (`js/host.rs`).
    pub items: &'a js::host::ItemIndex,
    /// The drop glue of every type the program drops anywhere, by that type: what a host marker
    /// that takes ownership of a closure or a future names (`js/host.rs`).
    pub drop_glue: &'a js::host::DropGlueIndex,
    /// Every vtable item of the program by the `(concrete type, trait)` pair it is the vtable of:
    /// what an unsizing coercion to `dyn` names (`js/unsize.rs`). v1 asked its namer for the
    /// name; a link-time lowering reads it off the vtable items the program already contains.
    pub vtables: &'a js::unsize::VtableIndex,
    /// The name the flat direct-call ABI gave each spill register, register 1 first
    /// (`plan::abi_flat::AbiPlan::spill_names`). A register is named around whatever the program
    /// already calls `$rN`, so the printer is told the name rather than deriving it. Empty when
    /// no plan was made, which is every mode but `js-lane=jir js-opt=on`.
    pub spills: &'a [JsName],
    /// The whole-program physical call ABI selected for this phase-C lowering. `None` while JIR
    /// is being built and while representation costing runs before ABI planning.
    pub abi: Option<&'a crate::plan::abi_flat::AbiPlan>,
    /// The value-local physical storage plan selected after representation and ABI planning.
    /// `None` while building JIR and during the earlier representation cost probes.
    pub physical: Option<&'a crate::plan::physical::PhysicalPlan>,
    /// A private native version's direct callees, keyed by their semantic item identity. The
    /// encoded `StrId` remains untouched; version names are a linker decision and never mutate an
    /// object file's shared intern table.
    pub call_retargets: &'a CallRetargets,
    /// The names of the program's module-level BigInt constants (`bigint.rs`). `Some` only for
    /// the lowering phase C installs on the ES2018 target, which then spells every BigInt constant
    /// of a function as its binding's name and widens an integer literal at compile time
    /// (`JsCx::to_bigint`); a `static` or vtable keeps its inline constants. `None` everywhere
    /// else, the whole-program cost trials included: they price the inline spelling.
    pub bigints: Option<&'a bigint::BigIntNames>,
    /// Whole-program lazy byte-view lowering policy (`analysis::byte_views`).
    pub byte_views: crate::analysis::byte_views::ByteViewPlan,
    /// Per-function nested native carrier clone guard (`value::clone_expr_with`).
    pub nested_clone: NestedClone,
}

/// One lowered item, as an AST plus what the printer needs around it.
pub struct LoweredItem {
    /// The top level declaration binding [`LowerInput::name`].
    pub decl: jsast::Stmt,
    /// Every identifier the declaration mentions, its own name excluded. Over-approximate exactly
    /// as the baseline's is: the reachability walk intersects it with the item table.
    pub refs: BTreeSet<JsName>,
    /// Emitted JavaScript name to the Rust name behind it, for the source map's `names` table.
    pub names: Vec<(String, String)>,
    /// Where the item was declared, which is the location its first line maps to. `None` for a
    /// synthesized item and whenever source maps are off.
    pub loc: Option<jsast::Loc>,
    /// The item's TypeScript signature, or `None`. Replaces the baseline's.
    pub dts: Option<crate::dts::Dts>,
    /// The unsupported constructs this lowering left in the body as a runtime throw. Replaces the
    /// baseline's, so a lowering that resolved a baseline zombie reports fewer.
    pub zombies: Vec<FlatZombie>,
    /// JIR provenance for allocation-bearing expressions in [`Self::decl`].
    pub js_alloc_origins: Vec<crate::analysis::js_alloc::Origin>,
    /// The digits of every BigInt constant [`Self::decl`] names by its module-level binding
    /// ([`LowerInput::bigints`]): the bindings the driver adds to the program. Empty unless the
    /// constants were hoisted.
    pub bigints: BTreeSet<String>,
}

/// Lowers one item's JIR-S body to a JavaScript declaration.
///
/// A `static` and a vtable are lowered from the body's own tree; a function runs the pipeline
/// `ssa::build → fuse::plan → structure::structure → js::function::lower`.
pub fn lower_item(input: &LowerInput<'_>) -> Result<LoweredItem, Decline> {
    use crate::jir::BodyKind;
    match &input.body.kind {
        BodyKind::Static { .. } => js::function::lower_static(input, &plan::Plan::default()),
        BodyKind::Vtable { .. } => js::function::lower_vtable(input),
        BodyKind::Function => lower_function(input),
    }
}

/// Test-only: `RCGJS_REPR_FORCE=<substring>=<carrier>[,...]` replaces a matching
/// producer-tagged standard-operation body with the semantic-op registry's native lowering at the
/// forced carrier — the Phase 4 wave-1 harness that exercises the emitters before the
/// representation solver exists to select carriers. `<carrier>` is `host-string`,
/// `string-builder`, `native-vec`, `native-box` or `slice-view`. Inert without the environment
/// variable; a forced item that fails the registry's proof gate is a loud `Decline`, never a silent
/// baseline fallback. No element drop glue is resolved here, so a vec of drop-needing elements
/// declines loudly too.
fn forced_semantic_rewrite(input: &LowerInput<'_>) -> Option<Result<LoweredItem, Decline>> {
    use crate::jir::decline::{LowerDecline, OpClass};
    use crate::jir::repr::Carrier;

    let force = std::env::var("RCGJS_REPR_FORCE").ok()?;
    let name = input.name.as_str();
    let carrier = force.split(',').find_map(|pair| {
        let (key, carrier) = pair.split_once('=')?;
        if !key.is_empty() && name.contains(key) {
            Some(match carrier {
                "host-string" => Ok(Carrier::HostString),
                "string-builder" => Ok(Carrier::StringBuilder),
                "native-vec" => Ok(Carrier::NativeVec),
                "native-box" => Ok(Carrier::NativeBox),
                "slice-view" => Ok(Carrier::SliceView),
                other => Err(other.to_owned()),
            })
        } else {
            None
        }
    })?;
    let decline_with = |detail: String| {
        Some(Err(Decline::Lower(LowerDecline::Other {
            op: OpClass::Other,
            detail,
        })))
    };
    let carrier = match carrier {
        Ok(carrier) => carrier,
        Err(other) => {
            return decline_with(format!(
                "RCGJS_REPR_FORCE names unknown carrier `{other}` (host-string, string-builder, \
                 native-vec, slice-view)"
            ));
        }
    };

    let cx = semantic_ops::LowerCx {
        semantic: input.body.semantic,
        sig: &input.body.signature,
        table: input.table,
        interns: input.interns,
        element_glue: None,
        element_glue_nonthrowing: false,
        panic_impl: None,
    };
    let (op, slots) = match semantic_ops::support_of(&cx) {
        Ok(support) => support,
        Err(decline) => {
            return decline_with(format!(
                "RCGJS_REPR_FORCE matched `{name}` but {}",
                decline.reason()
            ));
        }
    };
    let selected: Vec<Carrier> = slots.iter().map(|_| carrier).collect();
    match semantic_ops::rewrite(&cx, &selected) {
        Ok(rewrite) => {
            let decl = jsast::function(name, rewrite.parameters, rewrite.statements);
            // Refs exactly as `js::function::finish` reports them: every identifier the
            // declaration mentions, the item's own name excluded (the driver demands equality).
            let mut refs = BTreeSet::new();
            jsast::visit_idents(&decl, &mut |ident| {
                if ident != name {
                    refs.insert(JsName::new(ident.to_owned()));
                }
            });
            Some(Ok(LoweredItem {
                decl,
                refs,
                names: Vec::new(),
                loc: None,
                dts: None,
                zombies: Vec::new(),
                js_alloc_origins: Vec::new(),
                bigints: BTreeSet::new(),
            }))
        }
        Err(decline) => decline_with(format!(
            "RCGJS_REPR_FORCE could not lower `{name}` as {:?} at {carrier:?}: {}",
            op.id,
            decline.reason()
        )),
    }
}

/// The function pipeline: `ssa::build → fuse::plan → structure::structure → js::function::lower`.
///
/// # Why runtime checks are not folded here
///
/// `ssa::build_with` can take the decided edge of a `SwitchInt` on a `RuntimeCheck` operand
/// (`ub_checks`), which under `-Cdebug-assertions=off` deletes every precondition check in the
/// program. The baseline does not: `base.rs::switch_term` builds a `Term::Cond` on the boolean
/// literal and leaves both edges in the CFG, and nothing in `js-opt=off` folds it afterwards, so
/// v1 prints `if (false) { .. }` (135 of them in `core-tests/14_str` alone). v1 folds it only
/// under `ast-dce`, which is an `js-opt=on` pass — and `js-lane=jir` with `js-opt=on` is refused
/// in `opts.rs`, because two rewriters may not own one item.
///
/// So the fold is the OPTIMIZER's decision and the lane inherits it rather than making one of its
/// own. `opts.applies(AstDce)` is false in every mode the lane can run in today, which is the
/// answer: do what the baseline did. When Phase 3 re-lands `ast-dce` as a JIR pass the predicate
/// becomes true and the fold turns on with it, and the driver's subset rule (`link/lane.rs`)
/// already permits the smaller reference set it produces.
fn lower_function(input: &LowerInput<'_>) -> Result<LoweredItem, Decline> {
    lower_func(input, &build_func(input)?)
}

/// The front half of the function pipeline: the JIR-R [`Func`] of `input`'s body.
///
/// Split out from [`lower_func`] so the lane driver can hold every item's `Func` at once — which
/// is what a whole-program pass (`plan::abi_flat`) and a per-function one (`opt::optimize`) run
/// between the two halves. `lower_item` still runs both back to back.
pub fn build_func(input: &LowerInput<'_>) -> Result<Func, Decline> {
    let fold_runtime_checks = input.opts.applies(crate::opts::OptimizerPass::AstDce);
    ssa::build_with(
        input.body,
        input.origins,
        input.table,
        input.interns,
        fold_runtime_checks,
    )
}

/// The back half: `fuse::plan → structure::structure → js::function::lower` over a built [`Func`].
pub fn lower_func(input: &LowerInput<'_>, func: &Func) -> Result<LoweredItem, Decline> {
    lower_func_with_ascii(input, func, None)
}

/// [`lower_func`] carrying whole-program ASCII facts (`js-str=bytes`).
pub fn lower_func_with_ascii(
    input: &LowerInput<'_>,
    func: &Func,
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'_>>,
) -> Result<LoweredItem, Decline> {
    if let Some(forced) = forced_semantic_rewrite(input) {
        return forced;
    }
    if let crate::jir::semantic::BodySemantic::Standard(
        op @ (crate::jir::semantic::StandardOperation::F64FromStr
        | crate::jir::semantic::StandardOperation::F32FromStr),
    ) = input.body.semantic
    {
        if let Some(lowered) = semantic_ops::num::lower_float_from_str(
            input,
            op == crate::jir::semantic::StandardOperation::F32FromStr,
        ) {
            return lowered;
        }
    }
    let plan = fuse::plan(func, input.origins, input.table, input.interns, input.opts);
    let force_dispatch = matches!(input.opts.structure, crate::opts::Structure::Trampoline);
    let structured = structure::structure(func, &plan, input.table, force_dispatch)?;
    js::function::lower_with_ascii(input, func, &plan, structured.region, ascii)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lowering_decline_carries_a_code_the_lane_report_can_group_by() {
        use crate::jir::decline::{LowerDecline, OpClass};
        let decline = Decline::Lower(LowerDecline::Other {
            op: OpClass::Other,
            detail: "jir lowering not implemented".to_owned(),
        });
        assert_eq!(decline.code(), "lower.other");
        assert_eq!(decline.category(), "lower");
    }
}
