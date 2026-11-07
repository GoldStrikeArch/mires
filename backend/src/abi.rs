//! Calls: resolving the callee of a `Call` terminator and arranging its arguments.
//!
//! # Calling convention
//!
//! Every function takes its MIR arguments as positional JS parameters, in MIR order, and returns
//! its return local. There is no return pointer and no caller location: `#[track_caller]` adds a
//! hidden argument at the Rust ABI level, but nothing in the spike reads `Location::caller()`, so
//! the backend passes and binds only the arguments the MIR body actually declares.
//!
//! The one exception is the `"rust-call"` ABI, which closures and the `Fn*` trait methods use. Its
//! last argument is a tuple that the Rust ABI splats into one argument per element. The backend
//! splats it too (like `rustc_codegen_cranelift`'s `src/abi/mod.rs`), because the shims rustc
//! generates for `FnOnce::call_once` are *written against the splatted signature*: their MIR takes
//! the elements as separate locals. A closure body instead declares the tuple as a single local
//! and marks it `spread_arg`; `base.rs` rebuilds that local from the individual parameters in the
//! function prelude, so both shapes agree with the same call sites.

use rustc_abi::ExternAbi;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::mir::interpret::GlobalAlloc;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::jsast::{self, Expr, Stmt};
use crate::value::typing_env;

/// What a `Call` terminator's callee turned out to be.
pub(crate) enum Callee {
    /// A direct call to another function in this program.
    Direct(String),
    /// A call into the JavaScript runtime shim, `__rt.<name>(...)`.
    Runtime(String),
    /// A call through a value (a function pointer).
    Indirect(Expr),
    /// A call through a trait object's vtable, by absolute vtable slot. See [`VTABLE_DROP_SLOT`].
    Virtual(usize),
    /// Empty drop glue: the call is a no-op.
    Nop,
}

/// The vtable slot holding the drop glue, which `vtable.rs` fills and dropping a `dyn Trait`
/// reads. `rustc_middle::ty::vtable::COMMON_VTABLE_ENTRIES_DROPINPLACE`.
const VTABLE_DROP_SLOT: f64 = 0.0;

/// The parameter a closure thunk built inside a loop takes its environment as.
///
/// In the `$` namespace the backend reserves for its own names (`names.rs`), beside the `$aN` a
/// thunk forwards and the `$tN` a lowering hoists, so it can never shadow a local. See
/// [`FnCx::closure_thunk`].
const CAPTURED_ENVIRONMENT: &str = "$c";

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers a `Call` terminator's callee and arguments into the statements that perform it.
    ///
    /// Returns the statements to emit before the jump to `target`. The caller has already set the
    /// current span to the callee's `fn_span`; each argument narrows it to its own span.
    pub(crate) fn codegen_call(
        &self,
        func: &mir::Operand<'tcx>,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
        diverges: bool,
    ) -> Vec<Stmt> {
        let tcx = self.tcx;
        let func_ty = self.monomorphize(func.ty(&self.mir.local_decls, tcx));
        let fn_sig = func_ty.fn_sig(tcx);
        let is_rust_call = fn_sig.abi() == ExternAbi::RustCall;
        // Set below, once the callee resolves: a `#[track_caller]` callee takes the caller's
        // `Location` as a hidden trailing argument, and `base.rs`'s `params` binds it.
        let mut tracks_caller = false;

        let callee = match *func_ty.kind() {
            // A `view-abi` marker is recognized before anything else asks what the callee is: it
            // has a body, it resolves, and calling it would compile — it would just abort at run
            // time instead of building any DOM. See [`marker_section`].
            ty::FnDef(def_id, _) if marker_section(tcx, def_id).is_some() => {
                let marker = marker_section(tcx, def_id).expect("just matched");
                return self.codegen_marker(&marker, def_id, args, destination);
            }
            // A `mires-js` collection marker is recognized here for the same reason: it is an
            // ordinary generic Rust function, which a foreign declaration cannot be. See
            // `crate::map`.
            ty::FnDef(def_id, _) if crate::map::map_section(tcx, def_id).is_some() => {
                let marker = crate::map::map_section(tcx, def_id).expect("just matched");
                return self.codegen_map_marker(&marker, args, destination);
            }
            // A `#[js_extern]` declaration is recognized here for the same reason, and it has to be
            // here rather than later: its marker is an ordinary Rust function, so without this it
            // would resolve and emit a call to a body whose only statement is `unreachable!`.
            // See `crate::js_extern`.
            ty::FnDef(def_id, _)
                if crate::js_extern::extern_section(tcx, def_id).is_some() =>
            {
                let body = crate::js_extern::extern_section(tcx, def_id).expect("just matched");
                return self.codegen_js_extern(&body, args, destination);
            }
            // A `js!{}` block's marker, recognized here for exactly the same reason and told apart
            // from the two above by its section prefix alone. See `crate::js_block`.
            ty::FnDef(def_id, _) if crate::js_block::block_section(tcx, def_id).is_some() => {
                let body = crate::js_block::block_section(tcx, def_id).expect("just matched");
                return self.codegen_js_block(&body, args, destination);
            }
            ty::FnDef(def_id, generic_args) => {
                if tcx.is_foreign_item(def_id) {
                    let symbol = tcx.symbol_name(Instance::mono(tcx, def_id)).name;
                    // A foreign declaration with the *Rust* ABI is not a foreign function at all:
                    // it is a Rust symbol some other crate in this program defines, and the
                    // declaration exists only because the definition cannot be named directly.
                    // `core`'s `panic_impl` — the `#[panic_handler]` — is the one that matters
                    // here; the allocator shims are the same shape. Those are ordinary calls to
                    // an ordinary item, and routing them through the `__rt` shim object instead
                    // would look for a print-shim member that nobody defines.
                    //
                    // Everything else — `extern "C"` and friends — is the print shim contract in
                    // `CONTRACT.md`: a call to `__rt.<symbol>`.
                    if fn_sig.abi() == ExternAbi::Rust && jsast::is_plain_ident(symbol) {
                        Callee::Direct(symbol.to_owned())
                    } else {
                        Callee::Runtime(self.cgu.namer.rt_member(def_id, symbol))
                    }
                } else {
                    let Some(bound_free_args) = generic_args.no_bound_vars() else {
                        return vec![jsast::expr_stmt(self.zombie(format!(
                            "cannot call `{func_ty}`, whose generic arguments are not free of \
                             bound variables"
                        )))];
                    };
                    // Not `expect_resolve`: an unresolvable callee is a diagnostic on this call,
                    // not an ICE in the backend.
                    let instance = match Instance::try_resolve(
                        tcx,
                        typing_env(),
                        def_id,
                        bound_free_args,
                    ) {
                        Ok(Some(instance)) => instance,
                        Ok(None) | Err(_) => {
                            return vec![jsast::expr_stmt(
                                self.zombie(format!("cannot resolve a call to `{func_ty}`")),
                            )];
                        }
                    };
                    tracks_caller = instance.def.requires_caller_location(tcx);
                    match instance.def {
                        ty::InstanceKind::Intrinsic(def_id) => {
                            // A native lowering wins; it produces the whole statement list, the
                            // write to `destination` included.
                            if let Some(stmts) = crate::intrinsics::codegen_intrinsic(
                                self,
                                instance,
                                args,
                                &destination,
                            ) {
                                return stmts;
                            }
                            // Otherwise fall back to the intrinsic's own MIR body. What the
                            // collector emitted for it is an `InstanceKind::Item`, so the name has
                            // to be built from that and not from this `Intrinsic` instance, which
                            // no item is ever emitted under.
                            //
                            // The gate must mirror the mono collector's own rule: a
                            // `must_be_overridden` intrinsic is never collected, so naming its
                            // fallback here would emit a call to a function that does not exist
                            // (`is_mir_available` alone can disagree with the collector on those).
                            let must_override = tcx
                                .intrinsic(def_id)
                                .is_some_and(|intrinsic| intrinsic.must_be_overridden);
                            if !must_override && tcx.is_mir_available(def_id) {
                                let fallback = Instance::new_raw(def_id, instance.args);
                                Callee::Direct(self.cgu.namer.fn_name(fallback).into_string())
                            } else {
                                return vec![jsast::expr_stmt(self.zombie(format!(
                                    "intrinsic `{}` is not supported by rustc_codegen_mires",
                                    tcx.item_name(def_id)
                                )))];
                            }
                        }
                        // `idx` is an absolute vtable slot: `first_method_vtable_slot` counts the
                        // three header entries before the first method, so it indexes the array
                        // `vtable.rs` builds directly.
                        ty::InstanceKind::Virtual(_, idx) => Callee::Virtual(idx),
                        ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => Callee::Nop,
                        _ => Callee::Direct(self.cgu.namer.fn_name(instance).into_string()),
                    }
                }
            }
            ty::FnPtr(..) => Callee::Indirect(self.codegen_operand(func)),
            _ => {
                return vec![jsast::expr_stmt(
                    self.zombie(format!("cannot call a value of type `{func_ty}`")),
                )];
            }
        };

        if let Callee::Nop = callee {
            return Vec::new();
        }

        let mut arguments = if is_rust_call {
            self.splat_rust_call_args(args)
        } else {
            // Each argument reports against its own span rather than the callee's.
            args.iter().map(|arg| self.at(arg.span, || self.codegen_operand(&arg.node))).collect()
        };
        if tracks_caller {
            arguments.push(crate::intrinsics::caller_location_argument(self, self.span()));
        }

        // A virtual call reads its receiver three times (twice for the vtable, once for the
        // pointer), so the receiver goes into a hoisted temporary first.
        let mut out = Vec::new();
        let call = match callee {
            Callee::Direct(name) => jsast::call(jsast::id(name), arguments),
            Callee::Runtime(name) => jsast::rt_call(name, arguments),
            Callee::Indirect(target) => jsast::call(target, arguments),
            Callee::Virtual(idx) => {
                let Some((receiver, rest)) = arguments.split_first() else {
                    return vec![jsast::expr_stmt(
                        self.zombie("a virtual call takes at least a receiver".to_string()),
                    )];
                };
                let (bind, receiver) = self.temp(receiver.clone());
                out.push(bind);
                let mut call_args = vec![jsast::member(receiver.clone(), "ptr")];
                call_args.extend(rest.iter().cloned());
                jsast::call(vtable_slot(receiver, idx as f64), call_args)
            }
            Callee::Nop => unreachable!("handled above"),
        };

        if diverges {
            // A call that never returns: the shim's `js_abort` throws, so nothing follows it.
            out.push(jsast::expr_stmt(call));
            out.push(jsast::throw_error("rustc_codegen_mires: diverging call returned"));
        } else {
            out.extend(self.write_place(destination, call));
        }
        out
    }

    /// Dropping a value behind a `&dyn Trait`: the null checked call through vtable slot 0.
    ///
    /// The seam `base.rs`'s `TerminatorKind::Drop` uses once the place being dropped turns out to
    /// have a `dyn` type. `value` is the fat pointer; slot 0 is `null` exactly when the concrete
    /// type needs no drop, so the check is what keeps a trivially droppable type free.
    pub(crate) fn codegen_dyn_drop(&self, value: Expr) -> Vec<Stmt> {
        let (bind, receiver) = self.temp(value);
        let slot = vtable_slot(receiver.clone(), VTABLE_DROP_SLOT);
        vec![
            bind,
            jsast::if_(
                jsast::binary(jsast::BinOp::StrictNe, slot.clone(), jsast::null()),
                vec![jsast::expr_stmt(jsast::call(
                    slot,
                    vec![jsast::member(receiver, "ptr")],
                ))],
            ),
        ]
    }

    /// Splats the argument tuple of a `"rust-call"` callee into one argument per element.
    fn splat_rust_call_args(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Expr> {
        let (self_arg, pack) = match args {
            [pack] => (None, pack),
            [self_arg, pack] => (Some(self_arg), pack),
            _ => {
                return vec![
                    self.zombie("a `\"rust-call\"` function takes one or two arguments".to_string()),
                ];
            }
        };

        let mut out = Vec::new();
        out.extend(self_arg.map(|arg| self.at(arg.span, || self.codegen_operand(&arg.node))));

        self.at(pack.span, || {
            let pack = &pack.node;
            let pack_ty = self.monomorphize(pack.ty(&self.mir.local_decls, self.tcx));
            let ty::Tuple(element_tys) = pack_ty.kind() else {
                out.push(self.zombie(format!(
                    "the argument pack of a `\"rust-call\"` function is `{pack_ty}`, not a tuple"
                )));
                return;
            };

            // The pack is read as a place, so repeating the expression per element is free; each
            // element is copied exactly as `Operand::Copy` of that field would be.
            let pack_value = self.codegen_operand_aliased(pack);
            for (i, element_ty) in element_tys.iter().enumerate() {
                let element = jsast::index(pack_value.clone(), jsast::num(i as f64));
                out.push(if crate::value::needs_clone(self.tcx, element_ty) {
                    crate::value::clone_expr(self.tcx, element_ty, element)
                } else {
                    element
                });
            }
        });
        out
    }
}

/// `receiver.meta[slot]`: one entry of the vtable a fat pointer carries.
fn vtable_slot(receiver: Expr, slot: f64) -> Expr {
    jsast::index(jsast::member(receiver, "meta"), jsast::num(slot))
}

// -------------------------------------------------------------------------------------------
// `view-abi` markers
// -------------------------------------------------------------------------------------------
//
// A `view!` compiled for the client reaches codegen as calls to the marker functions in
// `view-abi`, each carrying a `link_section` naming what it is. The functions themselves are
// ordinary Rust with unreachable bodies: nothing about them stops a call from compiling, so the
// call is intercepted here, ahead of `is_foreign_item` and of resolution, and replaced with
// dom-expressions emission. See `crate::template` for what the payload holds.
//
// # The payload static is consumed, not referenced
//
// `template(&DATA)` is the one marker whose argument the backend reads at *compile* time. The
// emitted JavaScript therefore mentions nothing of `DATA`, and the `static` holding it should not
// be in the program at all. Dead code elimination is what removes it, and dead code elimination
// works on references: the MIR that feeds the call assigns the payload's address to a temporary
// first, and lowering that assignment is what would keep the `static` alive. So the assignment is
// dropped along with the call it fed. [`payload_locals`] is the rule, and it can only ever drop a
// store nothing else can observe.
//
// # Reaching a handler's body
//
// The monomorphization collector walks into a closure only through a call, so a closure handed to
// a marker is collected exactly when the marker's body calls it. `effect` and `each` do; `hole`
// cannot, because the value it takes is generic and an event handler is only one of the things it
// may be. So an event handler and everything it calls are emitted by [`FnCx::intern_reachable`],
// which walks the body the way the collector would. See `CONTRACT.md`, "Templates".

/// The `link_section` prefix every `view-abi` marker carries.
const MARKER_PREFIX: &str = "rcgjs.tc.";

/// The marker `def_id` is, by the `link_section` it carries.
fn marker_section(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(MARKER_PREFIX).map(str::to_owned)
}

/// How a `view_abi::text` value of type `ty` becomes a JavaScript string, or `None` for a type that
/// needs its own `Display`.
///
/// Only the shapes whose `Display` output the runtime reproduces exactly are listed. A number is
/// the case worth being careful about: `String(x)` agrees with Rust's `Display` for every integer,
/// and for a float only because the backend already spells float formatting the way Rust does (see
/// `CONTRACT.md`). An integer wider than 53 bits is a `BigInt`, whose `String()` is also exact.
///
/// A reference is peeled rather than rejected, which is what retires the `(*title)` workaround: a
/// `&&str` reaches this as a reference to a `str` reference, and reading through it is the whole
/// conversion.
fn text_conversion<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, value: Expr) -> Option<Expr> {
    match ty.kind() {
        // A `str` is already a JavaScript string. `alloc`'s `String` is NOT: it is a `Vec` of
        // bytes in the value model, so it is an `Adt` here and takes the `Display` path below.
        ty::Str => Some(value),
        ty::Ref(_, pointee, _) => {
            // Reading through the reference is the conversion, and which spelling that takes is
            // `ptr.rs`'s to answer: a reference to a value the model keeps in a slot is the slot
            // record, and a reference to anything else already IS the value. This is what retires
            // the `(*title)` workaround.
            let pointee = *pointee;
            let read = match crate::ptr::is_slot(tcx, ty) {
                true => crate::ptr::slot_element(value),
                false => value,
            };
            text_conversion(tcx, pointee, read)
        }
        // A `char` is a code point NUMBER in the value model, so `String(x)` would print the
        // number: `'J'` would render as `74`. Rust's `Display` for a `char` is the character.
        ty::Char => {
            Some(jsast::call(jsast::id("String.fromCodePoint".to_owned()), vec![value]))
        }
        // `String(x)` is Rust's `Display` for every one of these.
        ty::Bool | ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
            Some(jsast::call(jsast::id("String".to_owned()), vec![value]))
        }
        _ => None,
    }
}

/// Which arguments of a marker the backend reads instead of lowering.
///
/// A payload is decoded at codegen time, a hole index is a compile time constant, a `root` names
/// an instantiation the backend is holding, a list names one it is appending to, and a signal
/// ordinal is dropped. None of them appears in the emitted JavaScript, so nothing is left to
/// consume the MIR that computed them. Everything not listed here is an ordinary operand and is
/// lowered normally.
fn consumed_arguments(marker: &str) -> &'static [usize] {
    match marker {
        "template" => &[0],
        "hole" | "handler" | "effect" | "component" => &[0, 1],
        "signal" | "push" => &[0],
        "pushkeyed" => &[0],
        _ => &[],
    }
}

/// Which of a body's locals exist only to carry such an argument to its marker.
///
/// `template(&DATA)` reaches MIR as `_2 = <&DATA>` and a call that moves `_2`. The call emits
/// nothing that mentions `DATA`, so lowering that assignment would leave a dead store, and a dead
/// store naming a `static` is what keeps the `static` in the program. The `&root` of every hole is
/// the same shape without the `static`.
///
/// A local is reported here only when it is assigned once, read once and never borrowed: its one
/// reader is then the marker argument that was just consumed, and its assignment has nothing left
/// to feed. The walk follows the moves and borrows an expansion writes and stops at anything else,
/// so a body this pass does not fully understand keeps every store it had.
///
/// Returns one flag per local, indexed by [`mir::Local`].
pub(crate) fn consumed_locals<'tcx>(
    tcx: TyCtxt<'tcx>,
    mir: &mir::Body<'tcx>,
    uses: &crate::uses::Uses,
) -> Vec<bool> {
    /// How many moves to follow, for the same reason [`FnCx::constant_value`] has a bound.
    const MAX_STEPS: usize = 8;

    let mut consumed = vec![false; mir.local_decls.len()];
    for block in mir.basic_blocks.iter() {
        let mir::TerminatorKind::Call { func, args, .. } = &block.terminator().kind else {
            continue;
        };
        let ty::FnDef(def_id, _) = *func.ty(&mir.local_decls, tcx).kind() else { continue };
        let Some(marker) = marker_section(tcx, def_id) else { continue };

        for &index in consumed_arguments(&marker) {
            let Some(argument) = args.get(index) else { continue };
            let mut place = argument.node.place();
            for _ in 0..MAX_STEPS {
                let Some(local) = place.and_then(|place| place.as_local()) else { break };
                if !uses.inlinable(local) {
                    break;
                }
                consumed[local.as_usize()] = true;
                place = assigned_place(mir, local);
            }
        }
    }
    consumed
}

/// The place `local`'s one assignment reads, when that assignment is a move, copy or borrow.
///
/// Those are the three an expansion writes between a value and the marker argument it becomes.
/// Anything else ends the walk, which leaves the assignment standing.
fn assigned_place<'tcx>(mir: &mir::Body<'tcx>, local: mir::Local) -> Option<mir::Place<'tcx>> {
    for block in mir.basic_blocks.iter() {
        for statement in &block.statements {
            let mir::StatementKind::Assign(assignment) = &statement.kind else { continue };
            let (place, rvalue) = &**assignment;
            if place.as_local() != Some(local) {
                continue;
            }
            return match rvalue {
                mir::Rvalue::Use(operand, _) => operand.place(),
                mir::Rvalue::Ref(_, _, source) | mir::Rvalue::CopyForDeref(source) => Some(*source),
                _ => None,
            };
        }
    }
    None
}

/// The operand `local`'s one assignment reads, when that assignment is a plain move or copy.
fn assigned_operand<'tcx>(
    mir: &mir::Body<'tcx>,
    local: mir::Local,
) -> Option<mir::Operand<'tcx>> {
    for block in mir.basic_blocks.iter() {
        for statement in &block.statements {
            let mir::StatementKind::Assign(assignment) = &statement.kind else { continue };
            let (place, rvalue) = &**assignment;
            if place.as_local() != Some(local) {
                continue;
            }
            let mir::Rvalue::Use(operand, _) = rvalue else { return None };
            return Some(operand.clone());
        }
    }
    None
}

/// Where one hole of an instantiated template writes.
#[derive(Clone)]
struct HoleTarget {
    /// The node the hole's walk reaches.
    node: Expr,
    /// For a [`HoleKind::Child`] whose walk reached an anchor, the element it inserts into. `None`
    /// when the value is the element's sole child and `node` is that element.
    ///
    /// [`HoleKind::Child`]: crate::template::HoleKind::Child
    parent: Option<Expr>,
    /// For a hydratable [`HoleKind::Child`], the `_$getNextMarker` pair the walk named for it:
    /// the `<!/>` that closes the hole and the server nodes between the two markers.
    ///
    /// [`HoleKind::Child`]: crate::template::HoleKind::Child
    pair: Option<Expr>,
}

/// One template instantiated in this function, kept until its holes have been filled.
pub(crate) struct Instantiation {
    template: crate::template::Template,
    /// One entry per hole, in hole order. `Err` carries why that hole could not be located, so the
    /// zombie names the hole rather than the whole template.
    targets: Vec<Result<HoleTarget, String>>,
    /// The holes of each shared effect that have not been filled yet, by group. A group is here
    /// only while it has more than one member; see [`FnCx::codegen_fill`].
    awaited: std::collections::BTreeMap<u32, Vec<usize>>,
    /// The holes of each shared effect that have been filled, in the order they arrived.
    arrived: std::collections::BTreeMap<u32, Vec<Write>>,
}

impl Instantiation {
    /// Whether any shared effect is still waiting for a hole.
    ///
    /// Asked once the body has been lowered: a group that never completed is one whose writes were
    /// buffered and never emitted, which is a miscompilation rather than a missing optimization.
    pub(crate) fn incomplete_group(&self) -> Option<u32> {
        self.awaited.iter().find(|(_, holes)| !holes.is_empty()).map(|(group, _)| *group)
    }
}

/// One hole's write, held until the rest of its shared effect arrives.
struct Write {
    hole: crate::template::Hole,
    target: HoleTarget,
    fill: Fill,
}

/// What a hole is filled with: a value written once, or a closure the runtime re-runs.
enum Fill {
    /// The value of a `hole` call, already lowered.
    Once(Expr),
    /// The closure of an `effect` call, in the two forms a sink may want it in.
    Tracked {
        /// `() => closure$body(env)`, for a sink that subscribes to a function it is given.
        accessor: Expr,
        /// `closure$body(env)`, for a sink that takes a value and is called inside an effect.
        value: Expr,
    },
}

impl Fill {
    /// The expression to hand a sink that takes an accessor.
    ///
    /// `_$insert` is the one that does: it takes a value or a function and subscribes to the
    /// function itself, so a reactive child hole needs no `_$effect` around it. An event handler is
    /// the other: the sink calls what it is given.
    fn accessor(&self) -> Expr {
        match self {
            Fill::Once(value) | Fill::Tracked { accessor: value, .. } => value.clone(),
        }
    }

    /// The expression to hand a sink that takes a value.
    ///
    /// Every reactive sink other than `_$insert` is written inside an effect, and what it writes is
    /// the value the closure returned, not the closure.
    fn value(&self) -> Expr {
        match self {
            Fill::Once(value) | Fill::Tracked { value, .. } => value.clone(),
        }
    }

    /// Whether the runtime re-runs this fill, which is what puts a sink inside an `_$effect`.
    fn is_tracked(&self) -> bool {
        matches!(self, Fill::Tracked { .. })
    }
}

/// The record key of the `index`th write of a shared effect (CONTRACT-DOM 6.3).
///
/// The alphabet is upstream's `getNumberedId`, frequency ordered, and the index is written in it
/// as a numeral of its length. (CONTRACT-DOM's heading says base 53 and the string it quotes is 54
/// characters long; the string is what upstream divides by, and it is what this follows.) The
/// record is private to one effect, so any unique scheme would be correct; this one makes the
/// emitted code diffable against the reference output.
fn record_key(index: usize) -> String {
    const ALPHABET: &[u8] = b"etaoinshrdlucwmfygpbTAOISWCBvkxjqzPHFMDRELNGUKVYJQZX_$";

    let mut out = Vec::new();
    let mut left = index;
    loop {
        out.push(ALPHABET[left % ALPHABET.len()]);
        left /= ALPHABET.len();
        if left == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("the alphabet is ASCII")
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers a call to one of the `view-abi` markers.
    fn codegen_marker(
        &self,
        marker: &str,
        def_id: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let poison = |message: String| vec![jsast::expr_stmt(self.zombie(message))];
        match marker {
            "template" => self.codegen_template(args, destination),
            "hole" | "handler" | "component" => self.codegen_fill(args, false),
            "effect" => self.codegen_fill(args, true),
            "signal" => self.codegen_signal(args, destination),
            "sget" => self.codegen_signal_read(args, destination),
            "sset" => self.codegen_signal_write(args),
            "etv" => self.codegen_event_target_value(args, destination),
            "epd" => self.codegen_event_prevent_default(args),
            "content" => self.codegen_content(args, destination),
            "text" => self.codegen_text(def_id, args, destination),
            "cond" => self.codegen_cond(args, destination),
            "list" => self.codegen_list(destination),
            "push" => self.codegen_push(args),
            "pushkeyed" => self.codegen_push_keyed(def_id, args),
            "micro" => self.codegen_microtask(args),
            "settled" => self.codegen_settled(args),
            "owner" => self.codegen_owner(destination),
            "withowner" => self.codegen_with_owner(args),
            "hostval" => self.codegen_host_value(args, destination),
            other => poison(format!(
                "`{MARKER_PREFIX}{other}` is not a `view-abi` marker this backend knows"
            )),
        }
    }

    /// `template(&DATA)`: the cloner, the walk, and the record the holes are filled through.
    fn codegen_template(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        use crate::template::{NodeKind, Step, Template, Walk, dom_import};

        // The imports below are ES module items. In script output they would be a syntax error in
        // the file rather than a wrong value, so the whole instantiation is refused instead.
        if crate::opts::get().modules != crate::opts::Modules::Esm {
            return vec![jsast::expr_stmt(self.zombie(
                "a `view!` template needs the dom-expressions runtime, which is imported: compile \
                 with `-Cllvm-args=js-modules=esm`"
                    .to_string(),
            ))];
        }

        let Some(def_id) = self.static_argument(args.first()) else {
            return vec![jsast::expr_stmt(self.zombie(
                "the first argument of `view_abi::template` is not a reference to a `static`"
                    .to_string(),
            ))];
        };
        let template = match Template::read(self.tcx, def_id) {
            Ok(template) => template,
            Err(err) => {
                let path = self.tcx.def_path_str(def_id);
                return vec![jsast::expr_stmt(self.zombie(format!(
                    "the `view!` template `{path}` cannot be read: {err}"
                )))];
            }
        };

        // The root: a clone of the interned cloner, or the server-rendered node it claims. The
        // cloner is declared where the `view!` that built it was written, which is the order the
        // reference compiler declares its own in.
        let cloner = template.cloner(self.cgu, self.source_order(def_id));
        let root_call = match template.hydratable {
            true => jsast::call(dom_import(self.cgu, "getNextElement"), vec![cloner]),
            false => jsast::call(cloner, Vec::new()),
        };
        let mut out = self.write_place(destination, root_call);
        let root = self.read(&self.codegen_place(destination).0);

        // Every walk the holes need: a hole's own node, and for an anchored child also the element
        // it inserts into. `Walk::plan` names the prefixes they share exactly once.
        let tree = template.tree();
        let mut wanted: Vec<&[Step]> = Vec::new();
        // A hydratable child hole also needs the `<!/>` closing its pair, which the walk names by
        // calling `_$getNextMarker`; asking for it here is what puts that call in the walk, where
        // any step taken after the pair can re-base on it. Kept alive for `wanted`'s borrows.
        let closing: Vec<Vec<Step>> = template
            .holes
            .iter()
            .filter(|hole| {
                hole.kind.inserts_a_node()
                    && tree.kind_at(&hole.path) == Some(NodeKind::MarkerOpen)
            })
            .map(|hole| {
                let mut path = hole.path.clone();
                path.push(Step::NextSibling);
                path
            })
            .collect();
        for hole in &template.holes {
            wanted.push(&hole.path);
            if hole.kind.inserts_a_node()
                && tree.kind_at(&hole.path).is_some_and(NodeKind::is_comment)
            {
                if let Some(parent) = hole.anchor_parent() {
                    wanted.push(parent);
                }
            }
        }
        wanted.extend(closing.iter().map(Vec::as_slice));
        // Both of the planner's callbacks emit a statement, so they share one buffer rather than
        // borrowing `out` twice.
        let steps = std::cell::RefCell::new(Vec::new());
        let walk = Walk::plan(
            root,
            &tree,
            &wanted,
            |expr| {
                let (bind, name) = self.temp(expr);
                steps.borrow_mut().push(bind);
                name
            },
            |expr| {
                let (bind, name) = self.temp(jsast::call(
                    dom_import(self.cgu, "getNextMarker"),
                    vec![expr],
                ));
                steps.borrow_mut().push(bind);
                name
            },
        );
        out.extend(steps.into_inner());

        let targets = template
            .holes
            .iter()
            .map(|hole| {
                let rendered = crate::template::render_path(&hole.path);
                // A walk that reaches no node is a payload whose HTML and whose paths disagree.
                // Reported, because guessing would silently write into the wrong node.
                let Some(kind) = tree.kind_at(&hole.path) else {
                    return Err(format!(
                        "its walk `{rendered}` leaves the template `{}`",
                        template.html
                    ));
                };
                let node = walk.at(&hole.path).expect("every hole's walk was planned");
                // A hole that inserts a node and reaches a comment is anchored before it; one
                // reaching anything else is the sole child of the element it reached.
                let anchored = hole.kind.inserts_a_node() && kind.is_comment();
                let parent = match anchored {
                    true => Some(hole.anchor_parent().and_then(|path| walk.at(path)).ok_or_else(
                        || {
                            format!(
                                "the anchor its walk `{rendered}` reaches is not inside an \
                                 element of the template `{}`",
                                template.html
                            )
                        },
                    )?),
                    false => None,
                };
                Ok(HoleTarget { node, parent, pair: walk.marker_pair(&hole.path) })
            })
            .collect();

        // The delegated events this template needs. Placed here, with the instantiation, rather
        // than at module scope: the reference compiler emits one union at the end of the module,
        // and a program this backend produces has no module scope to run code in.
        // The payload claims its events are deduplicated and sorted; they are canonicalized here
        // anyway, because a duplicate would register the same delegated listener twice.
        let events = crate::template::delegated_events(std::iter::once(&template));
        if !events.is_empty() {
            let names = events.into_iter().map(jsast::string).collect();
            out.push(jsast::expr_stmt(jsast::call(
                dom_import(self.cgu, "delegateEvents"),
                vec![jsast::array(names)],
            )));
        }

        // The shared effects this template's holes are meant to collapse into: one entry per group
        // with more than one member, holding the holes still to arrive. A group of one is written
        // as it arrives, like an ungrouped hole, because a record with one key buys nothing.
        let mut awaited: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
        for (index, hole) in template.holes.iter().enumerate() {
            if let Some(group) = hole.effect_group {
                awaited.entry(group).or_default().push(index);
            }
        }
        awaited.retain(|_, holes| holes.len() > 1);

        if let Some(local) = destination.as_local() {
            self.templates.borrow_mut().insert(
                local,
                Instantiation { template, targets, awaited, arrived: Default::default() },
            );
        }
        out
    }

    /// `hole(&root, index, value)` and `effect(&root, index, closure)`.
    fn codegen_fill(&self, args: &[Spanned<mir::Operand<'tcx>>], tracked: bool) -> Vec<Stmt> {
        use crate::template::HoleKind;

        let [root, index, value] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("a `view-abi` hole marker takes three arguments".to_string()),
            )];
        };

        let Some(local) = self.node_local(&root.node) else {
            return vec![jsast::expr_stmt(self.zombie(
                "the `root` of a `view-abi` hole marker does not name an instantiated template"
                    .to_string(),
            ))];
        };
        let Some(index) = self.constant_u32(&index.node) else {
            return vec![jsast::expr_stmt(
                self.zombie("the index of a `view-abi` hole marker is not a constant".to_string()),
            )];
        };

        let templates = self.templates.borrow();
        let Some(instantiation) = templates.get(&local) else {
            return vec![jsast::expr_stmt(self.zombie(
                "the `root` of a `view-abi` hole marker does not name an instantiated template"
                    .to_string(),
            ))];
        };
        let Some(hole) = instantiation.template.holes.get(index as usize) else {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "hole {index} is past the end of a template with {} holes",
                instantiation.template.holes.len()
            )))];
        };
        let target = match &instantiation.targets[index as usize] {
            Ok(target) => target.clone(),
            Err(err) => {
                return vec![jsast::expr_stmt(
                    self.zombie(format!("hole {index} cannot be filled: {err}")),
                )];
            }
        };
        let hole = hole.clone();
        let shared = hole.effect_group.filter(|group| instantiation.awaited.contains_key(group));
        drop(templates);

        // What the sink is decides how the value is lowered, not just whether the hole is
        // reactive: an event sink takes a JavaScript function whether or not it re-runs, and a
        // Rust closure is an environment object until something names its body. A component is the
        // third: `_$createComponent` calls what it is given, inside the nested hydration context it
        // installs, so the component call has to reach it as a function rather than as a value.
        let as_function = tracked
            || matches!(
                hole.kind,
                HoleKind::DelegatedEvent | HoleKind::Event | HoleKind::Component
            );
        let fill = match as_function {
            true => match self.closure_thunk(&value.node) {
                Ok((accessor, value)) => match tracked {
                    true => Fill::Tracked { accessor, value },
                    false => Fill::Once(accessor),
                },
                Err(err) => return vec![jsast::expr_stmt(self.zombie(err))],
            },
            false => Fill::Once(self.at(value.span, || self.codegen_operand(&value.node))),
        };

        // A hole of a shared effect waits for the rest of its group and the last one to arrive
        // emits all of them, because one effect cannot be written before every write it carries is
        // known. `codegen_body` reports a group that never completed.
        let Some(group) = shared else {
            return self.write_hole(&hole, &target, fill);
        };
        let mut templates = self.templates.borrow_mut();
        let Some(instantiation) = templates.get_mut(&local) else { return Vec::new() };
        instantiation.awaited.entry(group).or_default().retain(|waiting| *waiting != index as usize);
        instantiation.arrived.entry(group).or_default().push(Write { hole, target, fill });
        if !instantiation.awaited[&group].is_empty() {
            return Vec::new();
        }
        let writes = instantiation.arrived.remove(&group).unwrap_or_default();
        drop(templates);
        self.write_group(writes)
    }

    /// One `_$effect` carrying every write of a shared effect group, with its `_p$` record.
    ///
    /// CONTRACT-DOM 6.2: the values are read into locals first, and each write is guarded by a
    /// comparison against the key of the record the effect is seeded with, so a property whose
    /// value did not change is not written again. `classList` and `style` are the exception the
    /// same section names: they take the previous value and their result *is* what is recorded, so
    /// they are called unconditionally.
    fn write_group(&self, writes: Vec<Write>) -> Vec<Stmt> {
        use crate::template::{HoleKind, dom_import};

        /// The record parameter, and the name of the local holding hole `index`'s value. Both are
        /// upstream's spelling, so that a diff against the reference output is about the code.
        const RECORD: &str = "_p$";

        let value_name = |index: usize| match index {
            0 => "_v$".to_owned(),
            n => format!("_v${}", n + 1),
        };

        let mut body = Vec::new();
        let decls: Vec<(String, Option<Expr>)> = writes
            .iter()
            .enumerate()
            .map(|(index, write)| (value_name(index), Some(write.fill.value())))
            .collect();
        body.push(jsast::lets(decls));

        for (index, write) in writes.iter().enumerate() {
            let key = record_key(index);
            let slot = jsast::member(jsast::id(RECORD), key);
            let value = jsast::id(value_name(index));
            let node = write.target.node.clone();
            let name = jsast::string(write.hole.name.clone());
            match write.hole.kind {
                // The helper diffs against what it returned last time and the record keeps that,
                // never the value that was handed in.
                HoleKind::ClassList | HoleKind::Style => {
                    let helper = match write.hole.kind {
                        HoleKind::ClassList => "classList",
                        _ => "style",
                    };
                    let call = jsast::call(
                        dom_import(self.cgu, helper),
                        vec![node, value, slot.clone()],
                    );
                    body.push(jsast::assign_stmt(slot, call));
                }
                _ => {
                    let stored = jsast::assign(slot.clone(), value.clone());
                    let write_call = match write.hole.kind {
                        HoleKind::Property => jsast::assign(jsast::index(node, name), stored),
                        _ => jsast::call(
                            dom_import(self.cgu, "setAttribute"),
                            vec![node, name, stored],
                        ),
                    };
                    body.push(jsast::expr_stmt(jsast::binary(
                        jsast::BinOp::And,
                        jsast::binary(jsast::BinOp::StrictNe, value, slot),
                        write_call,
                    )));
                }
            }
        }
        body.push(jsast::ret(jsast::id(RECORD)));

        let seed = (0..writes.len())
            .map(|index| (record_key(index), jsast::undefined()))
            .collect();
        vec![jsast::expr_stmt(jsast::call(
            dom_import(self.cgu, "effect"),
            vec![jsast::arrow_block(vec![RECORD.to_owned()], body), jsast::object(seed)],
        ))]
    }

    /// The statements that fill one located hole.
    fn write_hole(&self, hole: &crate::template::Hole, target: &HoleTarget, fill: Fill) -> Vec<Stmt> {
        use crate::template::{HoleKind, dom_import};

        let node = target.node.clone();
        let name = || jsast::string(hole.name.clone());
        // A reactive sink other than `_$insert` is written inside an effect, and what it writes is
        // the value the closure returned. Holes that share an `effect_group` never reach here: one
        // effect carries all of them, and `write_group` is what builds it.
        let reactive = fill.is_tracked();
        let wrap = |body: Expr| match reactive {
            true => jsast::expr_stmt(jsast::call(
                dom_import(self.cgu, "effect"),
                vec![jsast::arrow(Vec::new(), body)],
            )),
            false => jsast::expr_stmt(body),
        };
        // `_$classList` and `_$style` diff against what they returned last time, so a reactive one
        // takes the effect's previous value as a third argument (CONTRACT-DOM 6.2).
        let wrap_diffing = |helper: &str| {
            let previous = "_$p";
            let mut arguments = vec![node.clone(), fill.value()];
            if reactive {
                arguments.push(jsast::id(previous));
            }
            let call = jsast::call(dom_import(self.cgu, helper), arguments);
            match reactive {
                true => jsast::expr_stmt(jsast::call(
                    dom_import(self.cgu, "effect"),
                    vec![jsast::arrow(vec![previous.to_owned()], call)],
                )),
                false => jsast::expr_stmt(call),
            }
        };

        match hole.kind {
            // `_$insert` takes a value or an accessor and subscribes to the accessor itself, so a
            // reactive child needs no effect around it.
            //
            // A component is inserted the same three ways, around a value that is the component
            // call rather than the hole's own. `_$createComponent(thunk)` is upstream's boundary:
            // it swaps `sharedConfig.context` for a nested one, runs the call, and restores the
            // parent context, which is what makes a component's hydration keys nest (CONTRACT-DOM
            // 9.5). The props object is inside the thunk, not an argument to `createComponent`:
            // a Topcoat client component is a plain Rust fn taking one props struct, and a struct
            // argument is a JS object keyed by field name, so the thunk is
            // `() => card$fn({ title: .., count: 3 })` and the callee destructures it. Passing no
            // second argument is what upstream's `Comp(props || {})` is written for.
            HoleKind::Child | HoleKind::Component => {
                let value = match hole.kind {
                    HoleKind::Component => jsast::call(
                        dom_import(self.cgu, "createComponent"),
                        vec![fill.accessor()],
                    ),
                    _ => fill.accessor(),
                };
                let Some(parent) = target.parent.clone() else {
                    return vec![jsast::expr_stmt(jsast::call(
                        dom_import(self.cgu, "insert"),
                        vec![node, value],
                    ))];
                };
                let Some(pair) = target.pair.clone() else {
                    return vec![jsast::expr_stmt(jsast::call(
                        dom_import(self.cgu, "insert"),
                        vec![parent, value, node],
                    ))];
                };
                // The walk reached the opening `<!$>` and named the `_$getNextMarker` pair for it:
                // the `<!/>` that closes the hole, and the server nodes between the two. `_$insert`
                // takes both, and adopts those nodes rather than replacing them.
                vec![jsast::expr_stmt(jsast::call(
                    dom_import(self.cgu, "insert"),
                    vec![
                        parent,
                        value,
                        jsast::index(pair.clone(), jsast::num(0)),
                        jsast::index(pair, jsast::num(1)),
                    ],
                ))]
            }
            HoleKind::Attribute => vec![wrap(jsast::call(
                dom_import(self.cgu, "setAttribute"),
                vec![node.clone(), name(), fill.value()],
            ))],
            HoleKind::Property => {
                vec![wrap(jsast::assign(jsast::index(node.clone(), name()), fill.value()))]
            }
            // A delegated handler is a property the runtime's one document listener reads, never
            // a listener of its own (CONTRACT-DOM 7.2).
            HoleKind::DelegatedEvent => vec![jsast::assign_stmt(
                jsast::index(node, jsast::string(format!("$${}", hole.name))),
                fill.accessor(),
            )],
            HoleKind::Event => vec![jsast::expr_stmt(jsast::method_call(
                node,
                "addEventListener",
                vec![name(), fill.accessor()],
            ))],
            HoleKind::ClassList => vec![wrap_diffing("classList")],
            HoleKind::Style => vec![wrap_diffing("style")],
            HoleKind::Spread => vec![jsast::expr_stmt(jsast::call(
                dom_import(self.cgu, "spread"),
                vec![node, fill.accessor(), jsast::boolean(false), jsast::boolean(false)],
            ))],
        }
    }

    /// `signal(ordinal, init)`: the runtime's `createSignal`, whose pair is the handle's value.
    ///
    /// The ordinal is dropped. It exists so every emitter numbers a view's declarations the same
    /// way, which is what hydration keys will be built from; nothing on the client reads it yet.
    fn codegen_signal(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [_ordinal, init] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::signal` takes two arguments".to_string()),
            )];
        };
        let init = self.at(init.span, || self.codegen_operand(&init.node));
        let call = jsast::call(crate::template::dom_import(self.cgu, "createSignal"), vec![init]);
        self.write_place(destination, call)
    }

    /// `sig_get(&sig)`: the read half of the pair, called.
    fn codegen_signal_read(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [sig] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::sig_get` takes one argument".to_string()),
            )];
        };
        let sig = self.at(sig.span, || self.codegen_operand(&sig.node));
        let read = jsast::call(jsast::index(sig, jsast::num(0)), Vec::new());
        self.write_place(destination, read)
    }

    /// `event_target_value(event)`: `event.target.value`, the text of the element the event
    /// came from.
    ///
    /// The argument is read as an operand and nothing else: a handler closure's `&Event` parameter
    /// holds the DOM event the runtime passed it, so the local IS the event.
    ///
    /// `|| ""` is what makes the answer a string for an element with no `value`, which Rust's
    /// `&str` return promises and `undefined` would break the moment anything read its length. `||`
    /// rather than `??` because the two cannot differ here: the only values they disagree on are
    /// falsy ones that are neither `null` nor `undefined`, and `target.value` is always a string,
    /// whose one falsy value is the empty string this substitutes anyway.
    fn codegen_event_target_value(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [event] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::event_target_value` takes one argument".to_string()),
            )];
        };
        let event = self.at(event.span, || self.codegen_operand(&event.node));
        let value = jsast::member(jsast::member(event, "target".to_owned()), "value".to_owned());
        let text = jsast::binary(jsast::BinOp::Or, value, jsast::string(""));
        self.write_place(destination, text)
    }

    /// `event_prevent_default(event)`: `event.preventDefault()`.
    fn codegen_event_prevent_default(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [event] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::event_prevent_default` takes one argument".to_string()),
            )];
        };
        let event = self.at(event.span, || self.codegen_operand(&event.node));
        let call = jsast::call(jsast::member(event, "preventDefault".to_owned()), Vec::new());
        vec![jsast::expr_stmt(call)]
    }

    /// `sig_set(&sig, value)`: the write half of the pair, called with the new value.
    fn codegen_signal_write(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [sig, value] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::sig_set` takes two arguments".to_string()),
            )];
        };
        let sig = self.at(sig.span, || self.codegen_operand(&sig.node));
        let value = self.at(value.span, || self.codegen_operand(&value.node));
        vec![jsast::expr_stmt(jsast::call(jsast::index(sig, jsast::num(1)), vec![value]))]
    }

    /// `content(value)`: the value a branch or a row contributes, erased to one type.
    ///
    /// `content(())` is the branch that renders nothing, and becomes `null`, which is what
    /// `_$insert` treats as no content. Anything else is its own value.
    fn codegen_content(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [value] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::content` takes one argument".to_string()),
            )];
        };
        let ty = self.operand_ty(&value.node);
        let value = match ty.kind() {
            ty::Tuple(elements) if elements.is_empty() => jsast::null(),
            _ => self.at(value.span, || self.codegen_operand(&value.node)),
        };
        self.write_place(destination, value)
    }

    /// `text(v)`: the string a hole's value renders as.
    ///
    /// The backend sees the monomorphized `T`, so a type it can convert in place needs no
    /// formatting machinery at all: [`text_conversion`] answers for the primitives, and a `T` whose
    /// JavaScript representation is already a string is the identity. Anything else is `T`'s own
    /// `Display`, which is ordinary Rust: the call goes to `view_abi::display_to_str::<T>`, the
    /// function the marker's own body names for exactly this purpose, and that body streams into
    /// the host string builder.
    fn codegen_text(
        &self,
        def_id: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [value] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::text` takes one argument".to_string()),
            )];
        };
        match self.codegen_text_value(def_id, value) {
            Ok(text) => self.write_place(destination, text),
            Err(err) => vec![jsast::expr_stmt(self.zombie(err))],
        }
    }

    /// The JavaScript string a `Display` value renders as: the shared half of `text` and
    /// `push_keyed`.
    ///
    /// `marker` is the marker whose argument this is, and is used only to reach `display_to_str`
    /// beside it.
    fn codegen_text_value(
        &self,
        marker: DefId,
        value: &Spanned<mir::Operand<'tcx>>,
    ) -> Result<Expr, String> {
        let ty = self.operand_ty(&value.node);
        let operand = self.at(value.span, || self.codegen_operand(&value.node));

        if let Some(converted) = text_conversion(self.tcx, ty, operand.clone()) {
            return Ok(converted);
        }

        // The general path. `display_to_str` is a sibling of the marker in `view-abi` and takes the
        // same one generic parameter, which is the value's own type.
        let helper = self.sibling_item(marker, "display_to_str").ok_or_else(|| {
            format!(
                "there is no conversion for `{ty}`, and `view_abi::display_to_str`, which formats \
                 one, is not in this program"
            )
        })?;
        let generic_args = self.tcx.mk_args(&[ty.into()]);
        let instance = Instance::try_resolve(self.tcx, typing_env(), helper, generic_args)
            .ok()
            .flatten()
            .ok_or_else(|| format!("`view_abi::display_to_str::<{ty}>` cannot be resolved"))?;
        let name = self.cgu.namer.fn_name(instance).into_string();
        Ok(jsast::call(jsast::id(name), vec![operand]))
    }

    /// The `DefId` of the item named `name` beside `sibling`.
    ///
    /// Used to reach a marker's helper without spelling a path: the two are declared together, so
    /// the parent module is the join.
    fn sibling_item(&self, sibling: DefId, name: &str) -> Option<DefId> {
        let parent = self.tcx.parent(sibling);
        self.tcx
            .module_children(parent)
            .iter()
            .find(|child| child.ident.name.as_str() == name)
            .and_then(|child| child.res.opt_def_id())
    }

    /// `cond(test, then, els)`: a conditional whose test is hoisted into `_$memo`.
    ///
    /// The value is an accessor `_$insert` subscribes to, and the memo it dispatches on is created
    /// once, beside it:
    ///
    /// ```js
    /// $c0 = _$memo(() => test(env));
    /// _$insert(root, () => $c0() ? then(env) : els(env));
    /// ```
    ///
    /// which is the reference compiler's conditional (`__dom_fixtures__/conditionalExpressions`)
    /// with its immediately invoked wrapper flattened: what that wrapper exists for is to make the
    /// memo's declaration an expression, and a statement position needs no wrapper. The reference
    /// also coerces the test with `!!`; here the test is a `Fn() -> bool`, so its value is already
    /// a JavaScript boolean and the coercion has nothing to do.
    ///
    /// The memo is what makes this worth having: without it a branch is rebuilt whenever anything
    /// the conditional read changes, and with it only when the test's answer does.
    fn codegen_cond(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        use crate::template::dom_import;

        let [test, then, els] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::cond` takes three closures".to_string()),
            )];
        };
        let mut calls = Vec::with_capacity(3);
        for closure in [test, then, els] {
            match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                // The call, not the arrow: each closure is called from inside the one accessor
                // this builds, rather than being handed over as a function of its own.
                Ok((_, call)) => calls.push(call),
                Err(err) => return vec![jsast::expr_stmt(self.zombie(err))],
            }
        }
        let [test, then, els] = <[Expr; 3]>::try_from(calls).expect("three closures, three calls");

        let (bind, memo) = self.temp(jsast::call(
            dom_import(self.cgu, "memo"),
            vec![jsast::arrow(Vec::new(), test)],
        ));
        let dispatch = jsast::arrow(
            Vec::new(),
            jsast::cond(jsast::call(memo, Vec::new()), then, els),
        );
        let mut out = vec![bind];
        out.extend(self.write_place(destination, dispatch));
        out
    }

    /// `list()`: the empty list of rows a `for` in a view body fills.
    ///
    /// A list of DOM nodes is a value `_$insert` takes, so the rows need no wrapper of their own.
    fn codegen_list(&self, destination: mir::Place<'tcx>) -> Vec<Stmt> {
        self.write_place(destination, jsast::array(Vec::new()))
    }

    /// `push(&list, row)`: one row appended to the list the loop is filling.
    fn codegen_push(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [list, row] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::push` takes a list and a row".to_string()),
            )];
        };
        let Some(local) = self.node_local(&list.node) else {
            return vec![jsast::expr_stmt(self.zombie(
                "the `list` of `view_abi::push` does not name a local".to_string(),
            ))];
        };
        let row = self.at(row.span, || self.codegen_operand(&row.node));
        vec![jsast::expr_stmt(jsast::method_call(
            self.local_expr(local),
            "push",
            vec![row],
        ))]
    }

    /// `push_keyed(&list, key, row)`: one row appended, matched to the row of the same key from
    /// the previous render.
    ///
    /// The key becomes a string exactly the way a `text` hole value does, so a key is compared by
    /// the text it renders as, and `__rt.keyed_row` holds one cache per loop. The site number is
    /// what separates two loops' caches; it is per call site, and a `for` has exactly one
    /// `push_keyed` in it, so per call site IS per loop.
    fn codegen_push_keyed(
        &self,
        def_id: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
    ) -> Vec<Stmt> {
        let [list, key, row] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::push_keyed` takes a list, a key and a row".to_string()),
            )];
        };
        let Some(local) = self.node_local(&list.node) else {
            return vec![jsast::expr_stmt(self.zombie(
                "the `list` of `view_abi::push_keyed` does not name a local".to_string(),
            ))];
        };
        let key = match self.codegen_text_value(def_id, key) {
            Ok(key) => key,
            Err(err) => return vec![jsast::expr_stmt(self.zombie(err))],
        };
        let row = self.at(row.span, || self.codegen_operand(&row.node));
        let site = jsast::string(self.cgu.next_keyed_site());
        let cached = jsast::rt_call("keyed_row".to_owned(), vec![site, key, row]);
        vec![jsast::expr_stmt(jsast::method_call(
            self.local_expr(local),
            "push",
            vec![cached],
        ))]
    }

    // --------------------------------------------------------------------------------------
    // The executor
    // --------------------------------------------------------------------------------------

    /// `microtask(f)`: `__rt.microtask(() => f())`.
    ///
    /// The scheduler, and the only part of the executor that is JavaScript. The poll loop, the
    /// task table and the owner policy are all Rust in `view-async`, because a loop on this side
    /// would have to call a monomorphized `Future::poll` through a `&mut F` and the value model
    /// hands out neither.
    fn codegen_microtask(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [f] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::microtask` takes one closure".to_string()),
            )];
        };
        match self.at(f.span, || self.closure_thunk(&f.node)) {
            Ok((arrow, _)) => {
                vec![jsast::expr_stmt(jsast::rt_call("microtask".to_owned(), vec![arrow]))]
            }
            Err(err) => vec![jsast::expr_stmt(self.zombie(err))],
        }
    }

    /// `on_settled(value, f)`: `__rt.settled(value, (x, ok) => f(x, ok))`.
    ///
    /// The closure takes two parameters, so the arrow forwards two, which is what
    /// [`FnCx::closure_thunk`] builds from the closure's own arity. The shim decides what
    /// settling means; see `runtime/shim.js`.
    fn codegen_settled(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [value, f] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::on_settled` takes a value and a closure".to_string()),
            )];
        };
        let value = self.at(value.span, || self.codegen_operand(&value.node));
        match self.at(f.span, || self.closure_thunk(&f.node)) {
            Ok((arrow, _)) => {
                vec![jsast::expr_stmt(jsast::rt_call("settled".to_owned(), vec![value, arrow]))]
            }
            Err(err) => vec![jsast::expr_stmt(self.zombie(err))],
        }
    }

    /// `owner()`: `_$getOwner()`, the reactive owner the caller is running under.
    fn codegen_owner(&self, destination: mir::Place<'tcx>) -> Vec<Stmt> {
        let call =
            jsast::call(crate::template::dom_import(self.cgu, "getOwner"), Vec::new());
        self.write_place(destination, call)
    }

    /// `with_owner(owner, f)`: `_$runWithOwner(owner, () => f())`.
    ///
    /// `runWithOwner` is the one name the client DOM module owes beyond the client ABI's 48 plus
    /// `createSignal`. It is exported by `solid-js` and not re-exported by `solid-js/web`, so a
    /// module that forwards only the web build has to add it.
    fn codegen_with_owner(&self, args: &[Spanned<mir::Operand<'tcx>>]) -> Vec<Stmt> {
        let [owner, f] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::with_owner` takes an owner and a closure".to_string()),
            )];
        };
        let owner = self.at(owner.span, || self.codegen_operand(&owner.node));
        match self.at(f.span, || self.closure_thunk(&f.node)) {
            Ok((arrow, _)) => vec![jsast::expr_stmt(jsast::call(
                crate::template::dom_import(self.cgu, "runWithOwner"),
                vec![owner, arrow],
            ))],
            Err(err) => vec![jsast::expr_stmt(self.zombie(err))],
        }
    }

    /// `host_value(value)`: the identity.
    ///
    /// A host value read as a `V`. In this model a JavaScript string IS a `&str` and a number IS
    /// an `f64`, so the only thing the marker adds is the caller's claim about which one this is.
    /// It is reachable only from `on_settled`'s body, which every call replaces, so this arm
    /// normally emits nothing at all; it is here so that a body which somehow survived is the
    /// identity rather than a zombie.
    fn codegen_host_value(
        &self,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let [value] = args else {
            return vec![jsast::expr_stmt(
                self.zombie("`view_abi::host_value` takes one value".to_string()),
            )];
        };
        let value = self.at(value.span, || self.codegen_operand(&value.node));
        self.write_place(destination, value)
    }

    // --------------------------------------------------------------------------------------
    // Reading a marker's arguments
    // --------------------------------------------------------------------------------------

    /// Where the `view!` that declared this payload was written.
    ///
    /// The payload is a `static` the expansion generated, so its span points into the expansion;
    /// [`rustc_span::Span::source_callsite`] walks that back to the macro call the programmer
    /// wrote, which is what "first use" means in the reference compiler's output.
    ///
    /// Every payload one expansion writes shares that position, and the tie-breaker between them is
    /// the index the emitter numbered the payload with: `__TC_TEMPLATE_2` is the third template the
    /// view mentions, which is where the reference compiler declares it. The span cannot answer
    /// this: a proc macro's output all carries the macro call's span, so every payload of one view
    /// has the same one. A payload whose name ends in no index sorts first among its view's, which
    /// leaves the name order that was there before.
    fn source_order(&self, def_id: DefId) -> Option<crate::item::SourceOrder> {
        let loc = crate::names::location_of(self.tcx, self.tcx.def_span(def_id).source_callsite())?;
        let name = self.tcx.item_name(def_id);
        let digits = name.as_str().trim_start_matches(|c: char| !c.is_ascii_digit());
        Some(crate::item::SourceOrder {
            file: loc.file,
            line: loc.line,
            col: loc.col,
            within: digits.parse().unwrap_or(0),
        })
    }

    /// The `static` a marker's first argument is a reference to.
    ///
    /// The reference is a pointer constant, and the allocation its provenance names is the static
    /// itself. Nothing else is accepted: a template payload built at run time is not a payload the
    /// backend can read at compile time.
    fn static_argument(&self, arg: Option<&Spanned<mir::Operand<'tcx>>>) -> Option<DefId> {
        let value = self.constant_value(&arg?.node)?;
        let mir::ConstValue::Scalar(mir::interpret::Scalar::Ptr(pointer, _)) = value else {
            return None;
        };
        let (provenance, _) = pointer.prov_and_relative_offset();
        match self.tcx.global_alloc(provenance.alloc_id()) {
            GlobalAlloc::Static(def_id) => Some(def_id),
            _ => None,
        }
    }

    /// The local an instantiated template's `&root` argument ultimately names.
    ///
    /// The argument is a reference, so the local the call receives is a temporary holding the
    /// borrow rather than the template's own local. Chasing the borrow back is what connects a
    /// hole to the `template` call that made it.
    fn node_local(&self, operand: &mir::Operand<'tcx>) -> Option<mir::Local> {
        /// How many assignments to follow. A borrow of a borrow of a local is already more than
        /// any expansion writes; the bound is what keeps a malformed body from looping.
        const MAX_STEPS: usize = 8;

        let mut local = operand.place()?.as_local()?;
        for _ in 0..MAX_STEPS {
            if self.templates.borrow().contains_key(&local) {
                return Some(local);
            }
            let Some(next) = self.assigned_from(local) else { return Some(local) };
            local = next;
        }
        Some(local)
    }

    /// The local `local`'s one assignment reads, when that assignment is a borrow or a move.
    fn assigned_from(&self, local: mir::Local) -> Option<mir::Local> {
        for block in self.mir.basic_blocks.iter() {
            for statement in &block.statements {
                let mir::StatementKind::Assign(assignment) = &statement.kind else { continue };
                let (place, rvalue) = &**assignment;
                if place.as_local() != Some(local) {
                    continue;
                }
                return match rvalue {
                    mir::Rvalue::Ref(_, _, source) | mir::Rvalue::CopyForDeref(source) => {
                        source.as_local()
                    }
                    mir::Rvalue::Use(operand, _) => operand.place().and_then(|place| place.as_local()),
                    _ => None,
                };
            }
        }
        None
    }

    /// A marker's hole index, which is always a literal in an expansion.
    fn constant_u32(&self, operand: &mir::Operand<'tcx>) -> Option<u32> {
        let mir::ConstValue::Scalar(mir::interpret::Scalar::Int(int)) =
            self.constant_value(operand)?
        else {
            return None;
        };
        u32::try_from(int.to_uint(int.size())).ok()
    }

    /// The value of a marker argument that is a constant, however MIR chose to spell it.
    ///
    /// A literal argument does not have to reach the call as an `Operand::Constant`: MIR routinely
    /// assigns it to a temporary first and passes that. Both spellings mean the same constant, so
    /// the temporary is followed back to the assignment that filled it.
    fn constant_value(&self, operand: &mir::Operand<'tcx>) -> Option<mir::ConstValue> {
        /// How many assignments to follow, for the same reason [`FnCx::node_local`] has a bound.
        const MAX_STEPS: usize = 8;

        let mut operand = operand.clone();
        for _ in 0..MAX_STEPS {
            if let Some(constant) = operand.constant() {
                let konst = self.monomorphize(constant.const_);
                return konst.eval(self.tcx, typing_env(), constant.span).ok();
            }
            operand = assigned_operand(self.mir, operand.place()?.as_local()?)?;
        }
        None
    }


    /// The two forms of the closure a marker was handed: `(...args) => body(env, ...args)` and the
    /// call `body(env)` alone.
    ///
    /// A `Fn() -> V` takes nothing past its environment and the arrow takes no parameters, which is
    /// the accessor `_$insert` subscribes to. A handler takes the event, and the arrow forwards it,
    /// which is the shape a DOM sink calls a listener with. Every other reactive sink is written
    /// inside an effect and wants the call rather than the arrow.
    ///
    /// # The environment is snapshotted inside a loop
    ///
    /// The arrow closes over whatever expression the environment is, and for a `move` closure that
    /// is the *binding* of a local. Every local is declared once in the function prelude
    /// (`base.rs`), so a `for` over the rows of a grid assigns a fresh environment array to the
    /// same binding on every turn and each row's handler would read the last row's environment.
    /// The array is rebuilt per iteration; only the name it lands in is shared.
    ///
    /// So a thunk built inside a control flow cycle takes the environment as a parameter of an
    /// immediately invoked wrapper, `(($c) => ($a0) => f($c, $a0))(_25)`, which gives every turn a
    /// binding of its own. Outside a cycle the arrow is emitted bare, because there is nothing to
    /// reassign it. The *call* form is unaffected either way: it runs before the loop moves on.
    ///
    /// # Errors
    ///
    /// Returns a message naming the type that was passed where a closure was expected.
    fn closure_thunk(&self, operand: &mir::Operand<'tcx>) -> Result<(Expr, Expr), String> {
        let ty = self.operand_ty(operand);
        let (instance, has_environment) = match *ty.kind() {
            ty::Closure(def_id, generic_args) => (Instance::new_raw(def_id, generic_args), true),
            ty::FnDef(def_id, generic_args) => {
                let args = generic_args.no_bound_vars().ok_or_else(|| {
                    format!("the function `{ty}` a hole was given is not free of bound variables")
                })?;
                let instance = Instance::try_resolve(self.tcx, typing_env(), def_id, args)
                    .ok()
                    .flatten()
                    .ok_or_else(|| {
                        format!("the function `{ty}` a hole was given cannot be resolved")
                    })?;
                (instance, false)
            }
            _ => {
                return Err(format!(
                    "a `view-abi` hole was given a `{ty}`, which is neither a closure nor a \
                     function this backend can call"
                ));
            }
        };

        if !self.tcx.is_mir_available(instance.def_id()) {
            return Err(format!("`{ty}` has no MIR, so its body cannot be emitted"));
        }
        // The body's arguments are its environment (for a closure) and then the ones a caller
        // splats out of the `"rust-call"` tuple.
        let mir = self.tcx.instance_mir(instance.def);
        let arity = mir.arg_count.saturating_sub(usize::from(has_environment));

        // Every marker that is handed a closure calls it (`cond`, `effect`, `handler`), so the
        // monomorphization collector has already walked into the body and everything it calls, and
        // the name below is of an item this program has.
        let name = self.cgu.namer.fn_name(instance);

        let params: Vec<String> = (0..arity).map(|index| format!("$a{index}")).collect();
        let name = name.into_string();
        let environment = has_environment.then(|| self.codegen_operand(operand));

        let mut arguments = Vec::with_capacity(arity + 1);
        arguments.extend(environment.clone());
        arguments.extend(params.iter().cloned().map(jsast::id));
        let call = jsast::call(jsast::id(name.clone()), arguments);

        let arrow = match environment {
            Some(environment) if self.in_cycle() => {
                let mut arguments = Vec::with_capacity(arity + 1);
                arguments.push(jsast::id(CAPTURED_ENVIRONMENT));
                arguments.extend(params.iter().cloned().map(jsast::id));
                let inner = jsast::arrow(params, jsast::call(jsast::id(name), arguments));
                jsast::call(
                    jsast::arrow(vec![CAPTURED_ENVIRONMENT.to_owned()], inner),
                    vec![environment],
                )
            }
            _ => jsast::arrow(params, call.clone()),
        };
        Ok((arrow, call))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_keys_are_upstreams_numerals() {
        // The first three of `getNumberedId`'s alphabet, which is what a two or three write group
        // spells and what `__dom_fixtures__/attributeExpressions` shows.
        assert_eq!(record_key(0), "e");
        assert_eq!(record_key(1), "t");
        assert_eq!(record_key(2), "a");
        // The last single character, then the wrap into two.
        assert_eq!(record_key(53), "$");
        assert_eq!(record_key(54), "te");
        assert_eq!(record_key(55), "tt");
    }

    #[test]
    fn record_keys_are_distinct() {
        let keys: std::collections::BTreeSet<String> = (0..200).map(record_key).collect();
        assert_eq!(keys.len(), 200, "a key is what tells one write of a group from another");
    }

    #[test]
    fn only_the_arguments_a_marker_reads_are_consumed() {
        // The payload of `template`, the `&root` and index of every hole marker, and the `&list` a
        // row is pushed onto. A signal's ordinal is dropped; its initializer is not. `content` and
        // `cond` lower every argument they have.
        assert_eq!(consumed_arguments("template"), [0]);
        assert_eq!(consumed_arguments("hole"), [0, 1]);
        assert_eq!(consumed_arguments("handler"), [0, 1]);
        assert_eq!(consumed_arguments("effect"), [0, 1]);
        assert_eq!(consumed_arguments("signal"), [0]);
        assert_eq!(consumed_arguments("push"), [0]);
        assert!(consumed_arguments("content").is_empty());
        assert!(consumed_arguments("cond").is_empty());
        assert!(consumed_arguments("list").is_empty());
        assert!(consumed_arguments("sget").is_empty());
        assert!(consumed_arguments("sset").is_empty());
    }
}
