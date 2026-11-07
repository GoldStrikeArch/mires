//! Per-mono-item codegen: the MIR walk.
//!
//! One `Instance` becomes one [`JsItem`] holding one top level JavaScript function. Its MIR locals
//! become JS locals named `_0`, `_1`, ... — `_0` is the return place and `_1..=arg_count` are the
//! parameters — and its control flow graph becomes structured JavaScript:
//!
//! ```js
//! function name(_1, _2) {
//!   return _1 + _2 | 0;
//! }
//! ```
//!
//! The graph is rebuilt into loops, labeled blocks, ifs and switches by the `structurizer` crate:
//! `cfg_adapter.rs` turns the body into the graph it reads, this module runs it, `queue.rs`
//! rewrites the region tree that comes back, and `emit.rs` prints it.
//! `-Cllvm-args=js-structure=trampoline` forces the flat `switch (sel)` dispatch loop the spike
//! emitted for everything, which is also what an irreducible graph falls back to;
//! `-Cllvm-args=js-queue=off` turns the rewrite off, leaving one statement per MIR statement.
//!
//! All the cases of a dispatch's `switch` share one block scope, so every local is declared once,
//! at the top of the function, and the body only assigns to them. Hoisted temporaries
//! ([`FnCx::temp`]) and the structurizer's own (`emit.rs`) join that same declaration list, which
//! is why it is built *after* the body: by then the queue has run, and a local whose one
//! assignment moved into the one expression that read it has nothing left to declare.
//!
//! # The current span
//!
//! Lowering methods do not take a span. `FnCx` carries the span of the statement or terminator
//! being lowered in a `Cell`, and [`FnCx::at`] narrows it for the sub-expressions that know
//! better — a call argument, a constant. Threading a span parameter through fifteen signatures is
//! how the spike ended up reporting fn-level spans for operand errors: every caller that forgot to
//! narrow it silently passed `mir.span`.
//!
//! Places and operands live in `place.rs`, rvalues and operators in `rvalue.rs`, value
//! representation in `value.rs` and calls in `abi.rs`.

use std::cell::{Cell, RefCell};

use rustc_hir::LangItem;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{
    self, AssertKind, BasicBlock, BasicBlockData, Body, Local, START_BLOCK, Statement,
    StatementKind, SwitchTargets, Terminator, TerminatorKind,
};
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, Instance, TyCtxt, TypeFoldable};
use rustc_span::Span;
use structurizer::{Options as StructureOptions, Term};

use crate::cfg_adapter::{self, Blocks, Scrutinee};
use crate::cgu::CguCx;
use crate::item::{ItemKind, JsItem, ZombieKind, ZombieLog};
use crate::jsast::{self, Expr, Stmt};
use crate::names::{LineComments, LocalNames, Locs};
use crate::opts::Structure;
use crate::value::typing_env;

/// Lower a single monomorphized function.
pub(crate) fn codegen_fn<'tcx>(cgu: &CguCx<'tcx>, instance: Instance<'tcx>) -> JsItem {
    let tcx = cgu.tcx;
    let mir = tcx.instance_mir(instance.def);
    let fx = FnCx::new(cgu, instance, mir);
    let name = cgu.namer.fn_name(instance);

    let body = fx.codegen_body();
    let params = fx.params();
    // Only an entry point is described: a `.d.ts` is about the module's interface, and an internal
    // item is renamed by the minifier and exported by nothing.
    let dts = match cgu.namer.linkage_of(instance).exported {
        true => crate::dts::signature(tcx, instance, &params),
        false => None,
    };
    let decl = jsast::function(name.as_str(), params, body);
    let debug_path = with_no_trimmed_paths!(tcx.def_path_str(instance.def_id()));

    // What the source map is told this item's names stand for: the function itself, and every
    // local whose JavaScript spelling is not the Rust one (`names.rs`).
    let mut names = vec![(name.as_str().to_owned(), debug_path.clone())];
    names.extend(fx.names.originals().iter().cloned());

    JsItem::new(
        name,
        ItemKind::Fn,
        decl,
        fx.zombies.take(),
        cgu.namer.linkage_of(instance),
        debug_path,
    )
    .with_source(crate::names::location_of(tcx, mir.span), names)
    .with_dts(dts)
}

/// Lower a `static` item.
///
/// A `static` is a module level `let` holding the value its initializer const evaluates to. Always
/// a `let`, never a `const`: a `static mut` is assignable.
///
/// A `static` whose type is a JS primitive is **boxed**, `let S = [value]`, for the same reason a
/// local whose address is taken is: a reference to it is a `{ buf: S, off: 0 }` slot, and a bare
/// module level binding is not something a slot can name. `constant.rs` builds that reference; the
/// two have to agree, which is why the shape is decided by the same [`is_indirect`] question.
///
/// [`is_indirect`]: crate::value::is_indirect
pub(crate) fn codegen_static<'tcx>(cgu: &CguCx<'tcx>, def_id: DefId) -> JsItem {
    let tcx = cgu.tcx;
    let name = cgu.namer.static_name(def_id);
    let zombies = ZombieLog::default();
    let value =
        crate::constant::codegen_static_initializer(cgu, &zombies, def_id, tcx.def_span(def_id));
    let ty = tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
    let value =
        if crate::value::is_indirect(tcx, ty) { value } else { jsast::array(vec![value]) };
    let debug_path = with_no_trimmed_paths!(tcx.def_path_str(def_id));

    let names = vec![(name.as_str().to_owned(), debug_path.clone())];

    JsItem::new(
        name.clone(),
        ItemKind::Static,
        jsast::let_(name.as_str(), value),
        zombies.take(),
        cgu.namer.linkage_of(Instance::mono(tcx, def_id)),
        debug_path,
    )
    .with_source(crate::names::location_of(tcx, tcx.def_span(def_id)), names)
}

/// The state of one function's lowering.
pub(crate) struct FnCx<'a, 'tcx> {
    pub(crate) cgu: &'a CguCx<'tcx>,
    pub(crate) tcx: TyCtxt<'tcx>,
    pub(crate) instance: Instance<'tcx>,
    pub(crate) mir: &'tcx Body<'tcx>,
    pub(crate) zombies: ZombieLog,
    /// What each MIR local is called in the emitted JavaScript. See `names.rs`.
    pub(crate) names: LocalNames,
    /// The span lowering is currently at. See the module docs.
    span: Cell<Span>,
    /// The block lowering is currently at, for the lowerings that need to know whether the code
    /// they emit runs more than once. See [`FnCx::in_cycle`].
    block: Cell<BasicBlock>,
    /// Which blocks lie on a control flow cycle, indexed by block. See [`cyclic_blocks`].
    cyclic: Vec<bool>,
    /// The hoisted temporaries this body has asked for, in allocation order.
    temps: RefCell<Vec<String>>,
    /// What the body does with each of its locals. See `uses.rs`.
    pub(crate) uses: crate::uses::Uses,
    /// Which locals hold an enum tag rather than a discriminant number. See `tag.rs`.
    pub(crate) tag_discrs: crate::tag::TagDiscrs<'tcx>,
    /// Which locals are boxed, indexed by local. See [`FnCx::is_boxed`].
    boxed: Vec<bool>,
    /// Which locals only carry an argument to a `view-abi` marker that reads it instead of
    /// lowering it, indexed by local. See [`crate::abi::consumed_locals`].
    consumed: Vec<bool>,
    /// The `view!` templates this body has instantiated, by the local each one's root landed in.
    ///
    /// A template arrives as one marker call and its holes as one call each, so what the first
    /// call worked out — the cloner, the walk, where every hole writes — has to be waiting when
    /// the later ones are lowered. See `abi.rs`.
    pub(crate) templates: RefCell<std::collections::HashMap<Local, crate::abi::Instantiation>>,
}

impl<'a, 'tcx> FnCx<'a, 'tcx> {
    fn new(
        cgu: &'a CguCx<'tcx>,
        instance: Instance<'tcx>,
        mir: &'tcx Body<'tcx>,
    ) -> FnCx<'a, 'tcx> {
        let uses = crate::uses::Uses::of(cgu.tcx, instance, mir, &[]);
        let mut fx = FnCx {
            cgu,
            tcx: cgu.tcx,
            instance,
            mir,
            zombies: ZombieLog::default(),
            names: LocalNames::new(mir, crate::opts::get().names),
            span: Cell::new(mir.span),
            block: Cell::new(START_BLOCK),
            cyclic: cyclic_blocks(mir),
            temps: RefCell::new(Vec::new()),
            uses,
            tag_discrs: crate::tag::TagDiscrs::of(cgu.tcx, instance, mir),
            boxed: Vec::new(),
            consumed: Vec::new(),
            templates: RefCell::new(std::collections::HashMap::new()),
        };
        fx.consumed = crate::abi::consumed_locals(fx.tcx, mir, &fx.uses);
        // Counted a second time now that the dropped assignments are known, because a borrow one of
        // them takes is not in the emitted program and so needs no box to point at. The first round
        // is what named them, and it answers every other question the same way either round would:
        // only the boxing flag moves. See `uses.rs`.
        if fx.consumed.iter().any(|consumed| *consumed) {
            fx.uses = crate::uses::Uses::of(fx.tcx, instance, mir, &fx.consumed);
        }
        // A local whose address is taken and whose value is a JS primitive has no place a
        // `{ buf, off }` slot could name, so it is declared as a one element array instead. An
        // aggregate needs no box: its own object is the thing pointers point at.
        fx.boxed = mir
            .local_decls
            .indices()
            .map(|local| {
                let ty = fx.monomorphize(mir.local_decls[local].ty);
                fx.uses.bare_borrowed(local) && !crate::value::is_indirect(fx.tcx, ty)
            })
            .collect();
        fx
    }

    /// Whether this local is boxed: declared `let x = []`, read as `x[0]`, pointed at with
    /// `{ buf: x, off: 0 }`.
    ///
    /// See `uses.rs` for the question this answers and `place.rs` for what it does to the walk.
    pub(crate) fn is_boxed(&self, local: Local) -> bool {
        self.boxed.get(local.as_usize()).copied().unwrap_or(false)
    }

    /// Whether this local only carries an argument to a `view-abi` marker that read it instead of
    /// lowering it, so that the assignment filling it has nothing left to feed.
    fn is_consumed(&self, local: Local) -> bool {
        self.consumed.get(local.as_usize()).copied().unwrap_or(false)
    }

    /// Substitutes the instance's generic arguments into a type read off the MIR body.
    pub(crate) fn monomorphize<T>(&self, value: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>> + Copy,
    {
        self.instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx,
            typing_env(),
            ty::EarlyBinder::bind(self.tcx, value),
        )
    }

    /// The span lowering is currently at.
    ///
    /// For the lowerings that have to hand a span to a rustc query rather than to a zombie.
    #[allow(dead_code)]
    pub(crate) fn span(&self) -> Span {
        self.span.get()
    }

    /// Whether the block being lowered lies on a control flow cycle, so that everything it emits
    /// may run more than once in one call of this function.
    ///
    /// Every local is declared once, in the function prelude, and a loop assigns to the same
    /// binding on every turn. A lowering that hands a *binding* to something outliving the
    /// iteration -- a closure over the environment of a `view!` handler -- therefore has to
    /// snapshot the value instead; see `abi.rs::closure_thunk`.
    pub(crate) fn in_cycle(&self) -> bool {
        self.cyclic.get(self.block.get().as_usize()).copied().unwrap_or(false)
    }

    /// Runs `f` with the current span narrowed to `span`.
    pub(crate) fn at<R>(&self, span: Span, f: impl FnOnce() -> R) -> R {
        let saved = self.span.replace(span);
        let result = f();
        self.span.set(saved);
        result
    }

    /// Records something the backend cannot lower, and stands in for it with a runtime abort.
    ///
    /// Nothing is reported here. If the finished program can reach this item, `link.rs` reports
    /// every zombie in it, with the chain of calls that made it reachable; if it cannot, the item
    /// is dropped and the zombie with it. That is what lets a crate define `impl Add for i64`
    /// without every program that links it failing to compile.
    pub(crate) fn zombie(&self, message: String) -> Expr {
        self.zombies.record(self.tcx, self.span.get(), ZombieKind::Unsupported, message)
    }

    /// Allocates a hoisted temporary holding `value`.
    ///
    /// Returns the statement that fills it and the expression that reads it. The name is declared
    /// in the function prelude rather than at the use site, because the cases of the trampoline's
    /// `switch` share one block scope.
    ///
    /// The lowerings that need one all need a subexpression evaluated exactly once: the index of
    /// `&mut arr[i]` (`place.rs`), a virtual call's receiver and a `dyn` drop's (`abi.rs`).
    pub(crate) fn temp(&self, value: Expr) -> (Stmt, Expr) {
        let name = {
            let mut temps = self.temps.borrow_mut();
            let name = format!("$t{}", temps.len());
            temps.push(name.clone());
            name
        };
        (jsast::assign_stmt(jsast::id(name.clone()), value), jsast::id(name))
    }

    /// What a MIR local is called in the emitted JavaScript.
    ///
    /// `_0`, `_1`, ... unless the body's `var_debug_info` says what the programmer called it; see
    /// `names.rs` for the rules and for why an item name can never collide with one of these.
    pub(crate) fn local_name(&self, local: Local) -> String {
        self.names.get(local)
    }

    /// The name of the JS parameter carrying element `i` of a splatted `"rust-call"` tuple.
    ///
    /// The `$e` marker keeps these out of the way of a readable local name's collision suffix: a
    /// closure whose argument tuple is called `args` takes `args$e0`, `args$e1`, ..., while a
    /// second local also called `args` would become `args$1`.
    fn spread_param(&self, local: Local, i: usize) -> String {
        format!("{}$e{i}", self.local_name(local))
    }

    /// The function's JavaScript parameter list.
    ///
    /// A `#[track_caller]` function takes one parameter more than its MIR declares: the caller's
    /// `Location`, which the Rust ABI passes as a hidden trailing argument and which the
    /// `caller_location` intrinsic reads back. `abi.rs` appends the matching argument at every call
    /// site — see `intrinsics::caller_location_argument`.
    fn params(&self) -> Vec<String> {
        let mut params = Vec::new();
        for local in self.mir.args_iter() {
            match self.spread_arity(local) {
                Some(arity) => params.extend((0..arity).map(|i| self.spread_param(local, i))),
                None => params.push(self.local_name(local)),
            }
        }
        if self.instance.def.requires_caller_location(self.tcx) {
            params.push(crate::intrinsics::CALLER_LOCATION_PARAM.to_string());
        }
        params
    }

    /// The number of elements of `local`'s tuple, if it is the splatted `"rust-call"` argument.
    fn spread_arity(&self, local: Local) -> Option<usize> {
        if self.mir.spread_arg != Some(local) {
            return None;
        }
        match self.monomorphize(self.mir.local_decls[local].ty).kind() {
            ty::Tuple(tys) => Some(tys.len()),
            _ => None,
        }
    }

    /// The whole function body: local declarations, then the structured control flow.
    fn codegen_body(&self) -> Vec<Stmt> {
        // The body is lowered first: it is what decides how many temporaries get hoisted, and —
        // once the expression queue has run — which locals are left to declare at all.
        let (body, structure_temps) = self.codegen_control_flow();
        // A shared effect buffers its holes until the last one arrives (`abi.rs`). One still
        // waiting means writes were buffered and never emitted, so the DOM this body builds is
        // missing them; that is a defect in the payload or in this pass, not an optimization that
        // did not fire, and it is reported rather than left silent.
        for instantiation in self.templates.borrow().values() {
            if let Some(group) = instantiation.incomplete_group() {
                self.span.set(self.mir.span);
                self.zombie(format!(
                    "the holes of `view!` effect group {group} are not all filled in this body, so \
                     the effect that carries them was never emitted"
                ));
            }
        }
        let mentioned = mentioned_names(&body);

        // Every local is declared once, up front: the cases of a dispatch's `switch` share a
        // block scope, so a `let` inside a case would collide with the same `let` in another.
        //
        // A boxed local is declared as the empty array that is its box; a boxed *parameter* is
        // already bound by the signature, so its box is built by a prelude statement instead.
        let mut declarations = Vec::new();
        let mut prelude = Vec::new();
        for local in self.mir.local_decls.indices() {
            let is_param = local.as_usize() >= 1 && local.as_usize() <= self.mir.arg_count;
            let boxed = self.is_boxed(local);
            match (is_param, self.spread_arity(local)) {
                // Rebuild the splatted `"rust-call"` tuple from the individual parameters.
                (true, Some(arity)) => {
                    let elements =
                        (0..arity).map(|i| jsast::id(self.spread_param(local, i))).collect();
                    declarations.push((self.local_name(local), Some(jsast::array(elements))));
                }
                (true, None) => {
                    if boxed {
                        let name = jsast::id(self.local_name(local));
                        prelude.push(jsast::assign_stmt(
                            name.clone(),
                            jsast::array(vec![name]),
                        ));
                    }
                }
                (false, _) => {
                    let init = boxed.then(|| jsast::array(Vec::new()));
                    declarations.push((self.local_name(local), init));
                }
            }
        }
        declarations.extend(self.temps.borrow().iter().map(|name| (name.clone(), None)));
        declarations.extend(structure_temps.into_iter().map(|name| (name, None)));

        // A name the finished body never mentions has nothing to declare: the expression queue
        // inlined the temporary that carried it, or destination passing took its assignment away.
        // Declaring a name the body still *reads* matters even when nothing assigns it, so the
        // test is on mentions rather than on assignments. A name the body declares for itself
        // (`js-scoped-lets`) must not be declared twice.
        if crate::opts::get().queue {
            let scoped = declared_names(&body);
            declarations.retain(|(name, _)| mentioned.contains(name) && !scoped.contains(name));
        }

        let mut out = Vec::new();
        if !declarations.is_empty() {
            out.push(jsast::lets(declarations));
        }
        out.extend(prelude);
        out.extend(body);
        out
    }

    /// The body's control flow graph, rebuilt as structured JavaScript.
    ///
    /// Returns the statements and the names of the temporaries the structurizer introduced, which
    /// the prelude declares alongside the locals.
    ///
    /// The structurizer handles the shapes it cannot nest — an irreducible graph, one that would
    /// nest past its depth limit — by falling back to a dispatch loop on its own, so an `Err` here
    /// means the *graph* was not one it can read, which would be a bug in `cfg_adapter`. Lowering
    /// the body again as a flat dispatch is the one thing left to try, and it is tried silently:
    /// there is nothing a user of the backend could do about it. The abandoned attempt leaves its
    /// hoisted temporaries declared and its zombies recorded, which costs a few unused `let`s and
    /// a duplicated diagnostic on a path that is not supposed to be reachable at all.
    fn codegen_control_flow(&self) -> (Vec<Stmt>, Vec<String>) {
        if !matches!(crate::opts::get().structure, Structure::Trampoline) {
            if let Some(body) = self.structure_body(false) {
                return body;
            }
        }
        if let Some(body) = self.structure_body(true) {
            return body;
        }

        self.span.set(self.mir.span);
        let message = "this function's control flow cannot be lowered by rustc_codegen_mires";
        (vec![jsast::expr_stmt(self.zombie(message.to_string()))], Vec::new())
    }

    /// One attempt at rebuilding the body, as regions or as one flat dispatch loop.
    ///
    /// The expression queue runs *between* the structurizer and `emit.rs`, on the region tree:
    /// that is the one representation in which the statements of a straight-line run are adjacent
    /// and the boundaries the queue has to stop at are explicit. See `queue.rs`.
    fn structure_body(&self, force_dispatch: bool) -> Option<(Vec<Stmt>, Vec<String>)> {
        let options = StructureOptions { force_dispatch, ..StructureOptions::default() };
        let structured = structurizer::structure(cfg_adapter::build(self)?, &options).ok()?;

        let region = if crate::opts::get().queue {
            crate::queue::run(self, &self.uses, structured.region)
        } else {
            structured.region
        };

        let (mut body, temps) = crate::emit::emit(self, region);
        // A function that falls off its end returns `undefined`, which is what a bare `return`
        // returns too, so the last one says nothing. A source map marker prints nothing either, so
        // it does not count as something following the `return`.
        if let Some(last) = body.iter().rposition(|stmt| !matches!(stmt, Stmt::Loc(_))) {
            if matches!(body[last], Stmt::Return(None)) {
                body.truncate(last);
            }
        }
        Some((body, temps))
    }

    /// One basic block, as the payload and the edge the structurizer takes it apart into.
    ///
    /// `bb` is the block's MIR number, which the lowerings read back through [`FnCx::in_cycle`].
    pub(crate) fn codegen_block(
        &self,
        bb: BasicBlock,
        data: &BasicBlockData<'tcx>,
        blocks: &Blocks,
    ) -> (Vec<Stmt>, Term<Scrutinee<'tcx>>) {
        self.block.set(bb);
        let mut stmts = Vec::new();
        // `-Cllvm-args=js-line-comments`: one `// file.rs:LINE` per run of statements that came
        // from the same line, which is where a source map would point. Emitted here, at the seam
        // where a statement's span is still in hand and the run it produced is a contiguous slice.
        let mut lines = LineComments::new(self.tcx);
        // `-Cllvm-args=js-source-map=on`: the same runs, marked rather than commented. A marker
        // prints nothing, so the two options are independent.
        let mut locs = Locs::new(self.tcx);
        for statement in &data.statements {
            lines.at(&mut stmts, statement.source_info.span);
            locs.at(&mut stmts, statement.source_info.span);
            stmts.extend(self.codegen_statement(statement));
        }
        // The two halves run in this order and neither lowers what the other does: a terminator's
        // statements and the value its edge branches on are disjoint parts of it.
        lines.at(&mut stmts, data.terminator().source_info.span);
        locs.at(&mut stmts, data.terminator().source_info.span);
        stmts.extend(self.terminator_stmts(data.terminator()));
        // A marker with nothing after it says nothing, and would only get in the way of the
        // peepholes that look at what a block ends with.
        while matches!(stmts.last(), Some(Stmt::Loc(_))) {
            stmts.pop();
        }
        (stmts, self.terminator_term(data.terminator(), blocks))
    }

    /// `StatementKind::SetDiscriminant`: the variant of an already built enum place is changed
    /// without its fields being rewritten.
    ///
    /// Where the value *is* its tag -- a fieldless enum, whatever its `repr` -- there is nothing to
    /// change in place and the whole place is written instead. Where the value is an object, only
    /// the tag key moves; the keys of the variant the value used to hold stay where they are,
    /// because MIR writes the new variant's fields as separate statements and says nothing about
    /// the old ones.
    fn codegen_set_discriminant(
        &self,
        place: mir::Place<'tcx>,
        variant_index: rustc_abi::VariantIdx,
    ) -> Vec<Stmt> {
        let (target, ty) = self.codegen_place(place);
        let (Some(repr), Some(tag)) = (
            crate::value::enum_repr(self.tcx, ty),
            crate::value::variant_tag(self.tcx, ty, variant_index),
        ) else {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "the discriminant of `{ty}`, which is not an enum, cannot be set"
            )))];
        };
        match repr {
            // The two object shapes: a payload-carrying enum, and a coroutine, whose upvars and
            // saved locals sit beside the tag and outlive the state change.
            crate::value::EnumRepr::Tag { fieldless: false } | crate::value::EnumRepr::State => {
                vec![jsast::assign_stmt(
                    jsast::member(self.read(&target), crate::value::TAG),
                    tag,
                )]
            }
            crate::value::EnumRepr::Number | crate::value::EnumRepr::Tag { fieldless: true } => {
                self.write_place(place, tag)
            }
        }
    }

    fn codegen_statement(&self, statement: &Statement<'tcx>) -> Vec<Stmt> {
        self.span.set(statement.source_info.span);
        match &statement.kind {
            StatementKind::Assign(assignment) => {
                // An argument on its way to a marker that reads it at codegen time. Emitting it
                // would be a dead store, and a dead store naming a `static` is what keeps that
                // `static` in the program.
                if assignment.0.as_local().is_some_and(|local| self.is_consumed(local)) {
                    return Vec::new();
                }
                let value = self.codegen_rvalue(&assignment.1, assignment.0.as_local());
                self.write_place(assignment.0, value)
            }
            StatementKind::SetDiscriminant { place, variant_index } => {
                self.codegen_set_discriminant(**place, *variant_index)
            }
            StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
                // `assume` only guides optimizations.
                rustc_middle::mir::NonDivergingIntrinsic::Assume(_) => Vec::new(),
                // `LowerIntrinsics` rewrites a call to `copy_nonoverlapping` into this statement,
                // which is the form nearly every real caller arrives in; the intrinsic arm in
                // `intrinsics.rs` serves the crate that declares the intrinsic itself.
                rustc_middle::mir::NonDivergingIntrinsic::CopyNonOverlapping(copy) => {
                    let pointer_ty = self.operand_ty(&copy.src);
                    let Some(pointee) = pointer_ty.builtin_deref(true) else {
                        return vec![jsast::expr_stmt(self.zombie(format!(
                            "`copy_nonoverlapping` was called on `{pointer_ty}`, which is not a \
                             pointer"
                        )))];
                    };
                    let dst = self.codegen_operand(&copy.dst);
                    let src = self.codegen_operand(&copy.src);
                    let count = self.codegen_operand(&copy.count);
                    crate::ptr::copy(self, pointee, dst, src, count, false)
                }
            },
            StatementKind::StorageLive(_)
            | StatementKind::StorageDead(_)
            | StatementKind::ConstEvalCounter
            | StatementKind::Nop
            | StatementKind::FakeRead(..)
            | StatementKind::PlaceMention(..)
            | StatementKind::BackwardIncompatibleDropHint { .. }
            | StatementKind::AscribeUserType(..) => Vec::new(),
            // Everything above is a statement with nothing to emit. Anything else is a statement
            // the backend does not know, and skipping it silently would miscompile the body.
            other => vec![jsast::expr_stmt(self.zombie(format!(
                "`{}` statements are not supported by rustc_codegen_mires",
                other.name()
            )))],
        }
    }

    /// What a terminator runs before it transfers control: a call, an assertion, drop glue.
    ///
    /// The half of the old `codegen_terminator` that is *not* the edge. Terminators that only
    /// branch — `Goto`, `SwitchInt`, `Return` — contribute nothing here; the value a `SwitchInt`
    /// branches on is lowered by [`FnCx::terminator_term`], as part of the edge it belongs to.
    fn terminator_stmts(&self, terminator: &Terminator<'tcx>) -> Vec<Stmt> {
        self.span.set(terminator.source_info.span);
        match &terminator.kind {
            TerminatorKind::Goto { .. }
            | TerminatorKind::SwitchInt { .. }
            | TerminatorKind::Return
            | TerminatorKind::Unreachable
            | TerminatorKind::FalseEdge { .. }
            | TerminatorKind::FalseUnwind { .. }
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate(_) => Vec::new(),
            TerminatorKind::Call { func, args, destination, target, fn_span, .. } => {
                self.at(*fn_span, || self.codegen_call(func, args, *destination, target.is_none()))
            }
            TerminatorKind::Assert { cond, expected, msg, .. } => {
                if !self.tcx.sess.overflow_checks() && msg.is_optional_overflow_check() {
                    return Vec::new();
                }
                let condition = self.codegen_operand(cond);
                let failed = if *expected { jsast::not(condition) } else { condition };
                vec![jsast::if_(failed, self.codegen_assert_failure(msg))]
            }
            TerminatorKind::Drop { place, .. } => self.codegen_drop(*place),
            other => vec![jsast::expr_stmt(self.zombie(format!(
                "`{}` terminators are not supported by rustc_codegen_mires",
                other.name()
            )))],
        }
    }

    /// The edge a terminator takes: which block control goes to, or how the function ends.
    ///
    /// An edge into a block that is not in the graph can only be an edge into a cleanup block,
    /// which nothing but unwinding reaches. It becomes the same throw an unwind does rather than a
    /// jump to nowhere.
    fn terminator_term(
        &self,
        terminator: &Terminator<'tcx>,
        blocks: &Blocks,
    ) -> Term<Scrutinee<'tcx>> {
        self.span.set(terminator.source_info.span);
        let goto = |target: BasicBlock| match blocks.id(target) {
            Some(id) => Term::Goto(id),
            None => Term::Throw,
        };
        match &terminator.kind {
            TerminatorKind::Goto { target }
            | TerminatorKind::Drop { target, .. }
            | TerminatorKind::Assert { target, .. } => goto(*target),
            TerminatorKind::FalseEdge { real_target, .. }
            | TerminatorKind::FalseUnwind { real_target, .. } => goto(*real_target),
            // A call that does not come back ends the block. Control reaching the code after it
            // means the callee returned when its type says it cannot.
            TerminatorKind::Call { target, .. } => match target {
                Some(target) => goto(*target),
                None => Term::Unreachable,
            },
            TerminatorKind::Return => Term::Return,
            TerminatorKind::SwitchInt { discr, targets } => self.switch_term(discr, targets, blocks),
            TerminatorKind::UnwindResume | TerminatorKind::UnwindTerminate(_) => Term::Throw,
            // `Unreachable`, and every terminator the backend does not know: the statements above
            // have already aborted, so nothing can follow.
            _ => Term::Unreachable,
        }
    }

    /// The edge of a `SwitchInt`.
    ///
    /// A `bool` discriminant branches rather than switches — MIR lists the `false` arm as value 0
    /// and JavaScript's `switch` would compare it with `===` — and everything else becomes a
    /// multi way branch whose case values are biased for the structurizer (see `cfg_adapter`).
    fn switch_term(
        &self,
        discr: &mir::Operand<'tcx>,
        targets: &SwitchTargets,
        blocks: &Blocks,
    ) -> Term<Scrutinee<'tcx>> {
        let ty = self.operand_ty(discr);
        let tag_enum = self.tag_discrs.of_operand(discr);
        let on = Scrutinee {
            value: self.codegen_operand(discr),
            ty: cfg_adapter::ScrutTy { ty, tag_enum },
        };

        if matches!(ty.kind(), ty::Bool) {
            let target_for = |wanted: u128| {
                targets
                    .iter()
                    .find(|(value, _)| *value == wanted)
                    .map_or(targets.otherwise(), |(_, target)| target)
            };
            return match (blocks.id(target_for(1)), blocks.id(target_for(0))) {
                (Some(then_), Some(else_)) => Term::Cond { on, then_, else_ },
                _ => Term::Throw,
            };
        }

        let mut cases = Vec::with_capacity(targets.iter().len());
        for (value, target) in targets.iter() {
            let Some(id) = blocks.id(target) else { return Term::Throw };
            cases.push((cfg_adapter::encode(self.tcx, ty, value), id));
        }
        match blocks.id(targets.otherwise()) {
            Some(default) => Term::Switch { on, cases, default },
            None => Term::Throw,
        }
    }

    /// A `Drop` terminator: the call to the type's drop glue.
    ///
    /// The glue is resolved, never inspected. `Instance::resolve_drop_glue` answers with either
    /// the empty shim — nothing in this type needs dropping, so the terminator is a jump and
    /// nothing else — or the glue function rustc synthesizes, which arrives as an ordinary mono
    /// item and lowers like any other function. Recursive field glue, drop order and the call to
    /// `Drop::drop` itself are all inside that synthesized body; the backend never rebuilds them.
    ///
    /// The argument is the *reference* to the place. Every type that needs dropping is indirect,
    /// so that reference is the place's own JavaScript object and the `Deref` the glue's MIR
    /// starts with is a no-op.
    fn codegen_drop(&self, place: mir::Place<'tcx>) -> Vec<Stmt> {
        let ty = self.monomorphize(place.ty(&self.mir.local_decls, self.tcx).ty);
        let glue = Instance::resolve_drop_glue(self.tcx, ty);

        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) = glue.def {
            return Vec::new();
        }
        if let ty::Dynamic(..) = ty.kind() {
            // A trait object's glue is slot 0 of the receiver's vtable, called with the data
            // pointer and skipped when the slot is null. `&*place` on an unsized pointee is the
            // fat pointer itself, metadata included.
            return self.codegen_dyn_drop(self.codegen_ref(place));
        }

        let argument = self.codegen_ref(place);
        let glue_name = self.cgu.namer.fn_name(glue).into_string();
        vec![jsast::expr_stmt(jsast::call(jsast::id(glue_name), vec![argument]))]
    }

    /// The body of the `if` an `Assert` terminator guards: the panic it fails with.
    ///
    /// Rust's panics are lang items, so a failed assertion is an ordinary call to an ordinary
    /// function — `panic_bounds_check(index, len)` — whenever the program defines one. The
    /// `#[track_caller]` those functions carry adds a caller location argument at the Rust ABI
    /// level, but the backend passes and binds only the arguments a callee's MIR declares, on both
    /// sides of every call, so the two agree and the location is simply absent (see `abi.rs`).
    ///
    /// A program that declared no such lang item aborts through the shim with a static message.
    fn codegen_assert_failure(&self, msg: &mir::AssertKind<mir::Operand<'tcx>>) -> Vec<Stmt> {
        let (lang_item, args) = match msg {
            AssertKind::BoundsCheck { len, index } => (LangItem::PanicBoundsCheck, vec![index, len]),
            AssertKind::MisalignedPointerDereference { required, found } => {
                (LangItem::PanicMisalignedPointerDereference, vec![required, found])
            }
            AssertKind::InvalidEnumConstruction(source) => {
                (LangItem::PanicInvalidEnumConstruction, vec![source])
            }
            other => (other.panic_function(), Vec::new()),
        };

        // A lang item with generics of its own is not the shape the call below assumes; the shim
        // is the safe answer rather than a resolution that could go wrong.
        let panic = self
            .tcx
            .lang_items()
            .get(lang_item)
            .filter(|def_id| self.tcx.generics_of(*def_id).count() == 0);

        match panic {
            Some(def_id) => {
                let panic = Instance::mono(self.tcx, def_id);
                let name = self.cgu.namer.fn_name(panic).into_string();
                let mut arguments: Vec<Expr> =
                    args.iter().map(|arg| self.codegen_operand(arg)).collect();
                // The panic lang items are `#[track_caller]`, so they take the location of the
                // expression that failed as a hidden trailing argument.
                if panic.def.requires_caller_location(self.tcx) {
                    arguments.push(crate::intrinsics::caller_location_argument(self, self.span()));
                }
                vec![jsast::expr_stmt(jsast::call(jsast::id(name), arguments))]
            }
            None => vec![jsast::expr_stmt(jsast::rt_call(
                "js_abort",
                vec![jsast::string(assert_message(msg))],
            ))],
        }
    }

}

/// The names a finished body declares for itself, at the level the prelude would declare them.
///
/// `js-scoped-lets` moves the declaration of a single-assignment local to its assignment; the
/// prelude then has to leave that name alone, or the two declarations collide.
fn declared_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::Const(name, _) | Stmt::Let(name, _) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Every identifier a finished body mentions, read or written.
///
/// What the prelude declares is filtered through this: after the expression queue has run, a local
/// whose one assignment was inlined into the one expression that read it is gone from the body,
/// and so is the `let` that would have declared it.
fn mentioned_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for stmt in body {
        jsast::visit_idents(stmt, &mut |name| {
            names.insert(name.to_string());
        });
    }
    names
}

/// Which blocks of a body lie on a control flow cycle, indexed by block number.
///
/// The graph is the one `cfg_adapter::build` hands the structurizer -- every non-cleanup block, and
/// only the edges between two of them -- so "inside a loop" here means the same thing as "inside a
/// loop" in the emitted JavaScript. Cleanup blocks are never lowered, so they are never asked
/// about, and they stay `false`.
///
/// Tarjan's algorithm, run iteratively because a MIR body can be deeper than the stack: a block is
/// on a cycle when its strongly connected component has more than one member, or when it is its own
/// successor.
fn cyclic_blocks(mir: &Body<'_>) -> Vec<bool> {
    let count = mir.basic_blocks.len();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (bb, data) in mir.basic_blocks.iter_enumerated() {
        if data.is_cleanup {
            continue;
        }
        for target in data.terminator().successors() {
            if !mir.basic_blocks[target].is_cleanup {
                successors[bb.as_usize()].push(target.as_usize());
            }
        }
    }

    let mut cyclic = vec![false; count];
    // The depth first numbering, `None` for a block the walk has not reached yet, and the lowest
    // number reachable from a block's subtree through at most one back edge.
    let mut index: Vec<Option<usize>> = vec![None; count];
    let mut low = vec![0usize; count];
    let mut on_component = vec![false; count];
    // The blocks whose component is still open, and the walk's own stack of `(block, successor)`.
    let mut component: Vec<usize> = Vec::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    let mut next = 0;

    for root in 0..count {
        if index[root].is_some() || mir.basic_blocks[BasicBlock::from_usize(root)].is_cleanup {
            continue;
        }
        index[root] = Some(next);
        low[root] = next;
        next += 1;
        component.push(root);
        on_component[root] = true;
        work.push((root, 0));

        while let Some((node, cursor)) = work.pop() {
            if let Some(&target) = successors[node].get(cursor) {
                work.push((node, cursor + 1));
                match index[target] {
                    None => {
                        index[target] = Some(next);
                        low[target] = next;
                        next += 1;
                        component.push(target);
                        on_component[target] = true;
                        work.push((target, 0));
                    }
                    Some(number) if on_component[target] => low[node] = low[node].min(number),
                    Some(_) => {}
                }
                continue;
            }

            // Every successor is done, so this block's component is complete when nothing below it
            // reached higher up than the block itself.
            if Some(low[node]) == index[node] {
                let start = component
                    .iter()
                    .rposition(|member| *member == node)
                    .expect("an open block is on the component stack");
                let members = component.split_off(start);
                for &member in &members {
                    on_component[member] = false;
                }
                if members.len() > 1 || successors[node].contains(&node) {
                    for member in members {
                        cyclic[member] = true;
                    }
                }
            }
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
        }
    }
    cyclic
}

/// What a failed assertion says when the program defines no panic lang item to say it.
fn assert_message<O>(msg: &AssertKind<O>) -> &'static str {
    match msg {
        AssertKind::BoundsCheck { .. } => "index out of bounds",
        AssertKind::DivisionByZero(..) => "attempt to divide by zero",
        AssertKind::RemainderByZero(..) => {
            "attempt to calculate the remainder with a divisor of zero"
        }
        AssertKind::OverflowNeg(..) => "attempt to negate with overflow",
        AssertKind::Overflow(..) => "attempt to compute with overflow",
        _ => "assertion failed",
    }
}
