//! Calls and drops: v1 `backend/src/abi.rs`.
//!
//! # The calling convention
//!
//! Every function takes its MIR arguments as positional JavaScript parameters in MIR order and
//! returns its return local. Two things sit beside that:
//!
//! * a `"rust-call"` callee (a closure body, an `Fn*` shim) takes the elements of its argument
//!   tuple as separate parameters. The **encoder** splats the caller side
//!   (`backend/src/jir/encode.rs::call_arguments`) exactly as it flattens the callee side into
//!   [`AbiSignature::parameters`], so a JIR-R call's arguments and its callee's parameters line up
//!   one to one and nothing here has to know the call site's Rust ABI;
//! * a `#[track_caller]` callee takes the caller's `Location` as a trailing argument. Which
//!   `Location` is the JIR-S call's to say: `track_caller: Some(fact)` is a concrete call site to
//!   spell as a constant, and `None` inside a `#[track_caller]` body means the body forwards its
//!   own [`CALLER_LOCATION_PARAM`] — a distinction only the callee's own ABI can make, which is
//!   what the driver's item index ([`super::host::ItemIndex`]) is consulted for.
//!
//! # Statements around an expression
//!
//! A call is an expression, but two of the shapes here need statements to run first: a virtual
//! call reads its receiver three times and so binds it to a temporary, and a native intrinsic
//! lowering may need a loop. [`Lowered`] is that pair, and `emit.rs` splices the statements in
//! before the assignment to the result. A value the plan inlined has no statement position of its
//! own, so a lowering that needs one there is either folded into a comma expression (when every
//! statement is an assignment) or declined.

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::repr::{Carrier, Op, OpKind, Proj, Root, ValueId};
use rcgjs_object::jir::semantic::{CallSemantic, Callee, DropGlue, ItemId, StandardOperation};
use rcgjs_object::jsast::{self, Expr, Stmt};
use rcgjs_object::ptr;

use super::ptr::sequenced;
use super::{decline, emit, host, intrinsics, value, JsCx, Res, CALLER_LOCATION_PARAM};

/// The vtable slot holding the drop glue (`abi.rs::VTABLE_DROP_SLOT`).
pub const VTABLE_DROP_SLOT: f64 = 0.0;

/// What one call-shaped op became: statements that have to run first, and either a value or
/// nothing.
pub enum Lowered {
    /// The statements run, then `1` is the op's value.
    Value(Vec<Stmt>, Expr),
    /// The statements capture a tagged call's scalar result lanes; `1` is its virtual canonical
    /// description and must be cached rather than assigned as a JavaScript object.
    Virtual(Vec<Stmt>, Expr),
    /// The statements run, then `1` is the lowered 64-bit integer `(lo, hi)` pair.
    Pair(Vec<Stmt>, (Expr, Expr)),
    /// The statements are the whole lowering; the op has no value.
    Done(Vec<Stmt>),
}

impl Lowered {
    /// A value with nothing to run before it.
    pub fn value(expr: Expr) -> Lowered {
        Lowered::Value(Vec::new(), expr)
    }

    /// The value of a lowering that needs no statement position, folding a prelude of plain
    /// assignments into a comma expression the way `ptr.rs::sequenced` does.
    pub fn into_expr(self) -> Option<Expr> {
        let (prelude, value) = match self {
            Lowered::Value(prelude, value) | Lowered::Virtual(prelude, value) => (prelude, value),
            Lowered::Pair(..) | Lowered::Done(_) => return None,
        };
        if prelude.is_empty() {
            return Some(value);
        }
        let mut items = Vec::with_capacity(prelude.len() + 1);
        for stmt in prelude {
            match stmt {
                Stmt::ExprStmt(expr) => items.push(expr),
                _ => return None,
            }
        }
        items.push(value);
        Some(jsast::seq(items))
    }
}

impl JsCx<'_> {
    /// The whole lowering of one call-shaped op: `Call`, `Intrinsic`, `Host` or `Drop`.
    pub(super) fn lower_call_like(&self, op: &Op) -> Res<Lowered> {
        match &op.kind {
            OpKind::Call { .. } => self.lower_call(op),
            OpKind::Intrinsic {
                name,
                types,
                fallback,
                args,
            } => intrinsics::lower(self, op, *name, types, *fallback, args),
            OpKind::Host { .. } => host::lower_host(self, op).map(Lowered::value),
            OpKind::Drop { root, path, glue } => self.lower_drop(root, path, *glue),
            _ => Err(decline(OpClass::Call, "not a call")),
        }
    }

    /// A `Call` op (`abi.rs::codegen_call`).
    fn lower_call(&self, op: &Op) -> Res<Lowered> {
        let OpKind::Call {
            callee,
            function,
            args,
            track_caller,
            semantic,
            host,
        } = &op.kind
        else {
            return Err(decline(OpClass::Call, "not a call"));
        };

        // Everything host-shaped is W9's, and is recognized before anything asks what the callee
        // is: a marker resolves to an ordinary Rust body whose only statement is `unreachable!`,
        // so calling it would compile and abort at run time (`abi.rs`, "view-abi markers"). The
        // exception `host::intercepts` makes is v1's: an async marker with a safe Rust body is an
        // ordinary call unless the future-poller pass applies.
        let _ = host;
        if host::intercepts(self, op) {
            return host::lower_call(self, op).map(Lowered::value);
        }

        // Phase 12D's opt-in exact numeric storage is value-local, so it is intercepted at the
        // prepared caller rather than changing the shared `Vec::with_capacity`/`Vec::push`
        // bodies for every vector of the same element type. The plan has proved a constant
        // logical capacity, a maximum length, no observable capacity/layout, and no boundary or
        // retained-view escape. `buf` is allocated once at the greater of the two lengths while
        // `cap` continues to carry Rust's logical capacity independently. An admitted immutable
        // slice is constructed inline; its only consumers have selected `buf, off, len` lanes,
        // whose member folding removes the temporary slice object.
        if matches!(
            semantic,
            CallSemantic::Standard(StandardOperation::VecWithCapacity)
        ) {
            let Some(result) = op.result else {
                return Err(decline(
                    OpClass::Call,
                    "selected typed Vec construction has no result",
                ));
            };
            if let Some(storage) = self.typed_storage(result) {
                let [capacity] = args.as_slice() else {
                    return Err(decline(
                        OpClass::Call,
                        "selected typed Vec construction has the wrong arity",
                    ));
                };
                let value = jsast::object_of(vec![
                    (
                        "buf",
                        jsast::new(
                            jsast::id(storage.constructor),
                            vec![jsast::raw_num(storage.allocation_length.to_string())],
                        ),
                    ),
                    ("len", jsast::num(0)),
                    ("cap", self.expr_of(*capacity)?),
                ]);
                return Ok(Lowered::value(value));
            }
        }
        if matches!(
            semantic,
            CallSemantic::Standard(StandardOperation::VecAsSlice)
        ) {
            let [receiver] = args.as_slice() else {
                return Err(decline(
                    OpClass::Call,
                    "selected typed Vec::as_slice has the wrong arity",
                ));
            };
            if self.typed_storage(*receiver).is_some() {
                let receiver = self.expr_of(*receiver)?;
                return Ok(Lowered::value(ptr::fat_slice(
                    jsast::member(receiver.clone(), "buf"),
                    jsast::num(0),
                    jsast::member(receiver, "len"),
                )));
            }
        }
        if matches!(semantic, CallSemantic::Standard(StandardOperation::VecPush)) {
            let [receiver, value] = args.as_slice() else {
                return Err(decline(
                    OpClass::Call,
                    "selected typed Vec push has the wrong arity",
                ));
            };
            if self.typed_storage(*receiver).is_some() {
                return Ok(Lowered::value(jsast::rt_call(
                    "typed_vec_push",
                    vec![self.expr_of(*receiver)?, self.expr_of(*value)?],
                )));
            }
        }
        // `CallSemantic` is not a codegen distinction on the baseline lane: an allocator shim, a
        // `from_raw_parts` and a `Vec::push` are ordinary calls to the items they resolve to (v1
        // synthesizes the allocator's bodies as items, `alloc_support.rs`, and never intercepts
        // the call site). It is the representation phases that key on it.
        let _ = semantic;

        // A compiler-tagged `String::deref` at a committed carrier is a boundary operation, not
        // a reason to enter (or replace) the shared `alloc` body. Keeping this at the call site
        // lets baseline callers retain the public body while a prepared JIR caller reads its
        // native value directly.
        if matches!(
            semantic,
            CallSemantic::Standard(StandardOperation::StringAsStr)
        ) {
            let [receiver] = args.as_slice() else {
                return Err(decline(OpClass::Call, "String::deref with the wrong arity"));
            };
            if let Some(carrier) = self.borrowed_carrier(*receiver) {
                let receiver = self.expr_of(*receiver)?;
                return match carrier {
                    Carrier::HostString => Ok(Lowered::value(receiver)),
                    Carrier::StringBuilder => Ok(Lowered::value(jsast::member(receiver, "text"))),
                    _ => Err(decline(
                        OpClass::Call,
                        "String::deref marked with an unsupported carrier",
                    )),
                };
            }
        }

        // A format call the representation plan committed (Phase 4, W4g): the constructor's text
        // or the consumer's concatenation, inlined at the site. Inert unless the plan marked the
        // site's carriers, so `off`/`report` and every uncommitted call stay plain calls.
        if let Some(lowered) = crate::lower::semantic_ops::fmt_kernel::intercept(self, op)? {
            return Ok(lowered);
        }

        if matches!(callee, Callee::Nop) {
            // `drop_in_place::<T>` for a `T` with empty glue: the arguments are place reads with
            // no effect, so the whole call is nothing.
            return Ok(Lowered::Done(Vec::new()));
        }

        let physical = match (callee, self.abi) {
            (Callee::Direct { item, .. }, Some(abi)) => abi.call_signature(*item),
            _ => None,
        };
        let slice_positions = physical.map_or_else(Vec::new, |sig| sig.slice_jir_positions());
        let tagged_positions = physical.map_or_else(Vec::new, |sig| sig.tagged_jir_positions());
        let recursive_positions =
            physical.map_or_else(Vec::new, |sig| sig.recursive_jir_positions());
        let i64_positions = physical.map_or_else(Vec::new, |sig| sig.i64_pair_jir_positions());
        let mut arguments = Vec::with_capacity(args.len());
        let mut owned_virtual_receiver = false;
        for (position, arg) in args.iter().enumerate() {
            if i64_positions.contains(&position) {
                let (lo, hi) = self.pair_of(*arg)?;
                arguments.extend([lo, hi]);
                continue;
            }
            let value = if position == 0 && matches!(callee, Callee::Virtual { .. }) {
                let (value, owned) = self.virtual_receiver(*arg)?;
                owned_virtual_receiver = owned;
                value
            } else {
                self.expr_of(*arg)?
            };
            let slice = slice_positions.contains(&position);
            if slice {
                // `member` folds these reads through an inline `{buf,off,len}` literal. A chain
                // of private slice-lane calls therefore never creates the fat-pointer object;
                // canonical or indirect calls do not consult the physical signature.
                arguments.extend([
                    ptr::slot_buf(value.clone()),
                    ptr::slot_off(value.clone()),
                    ptr::slot_len(value),
                ]);
            } else if let Some((_, shape)) = tagged_positions
                .iter()
                .find(|(tagged_position, _)| *tagged_position == position)
            {
                let Some(keys) = value::tagged_product_keys(self.table, self.interns, shape) else {
                    return Err(decline(
                        OpClass::Call,
                        "tagged argument has an unprintable product shape",
                    ));
                };
                let reads = keys.iter().map(String::as_str).collect::<Vec<_>>();
                let (setup, value) = self.reread(value, &reads);
                let Some(mut lanes) =
                    value::tagged_product_lanes(self.table, self.interns, shape, value)
                else {
                    return Err(decline(
                        OpClass::Call,
                        "tagged argument lanes do not agree with its type",
                    ));
                };
                if let Some(first) = lanes.first_mut() {
                    *first = sequenced(setup, first.clone());
                }
                arguments.extend(lanes);
            } else if let Some((_, shape)) = recursive_positions
                .iter()
                .find(|(recursive_position, _)| *recursive_position == position)
            {
                let Some(keys) = value::recursive_product_keys(self.table, self.interns, shape)
                else {
                    return Err(decline(
                        OpClass::Call,
                        "recursive argument has an unprintable product shape",
                    ));
                };
                let reads = keys.iter().map(String::as_str).collect::<Vec<_>>();
                let (setup, value) = self.reread(value, &reads);
                let Some(mut lanes) =
                    value::recursive_product_lanes(self.table, self.interns, shape, value)
                else {
                    return Err(decline(
                        OpClass::Call,
                        "recursive argument lanes do not agree with its type",
                    ));
                };
                if let Some(first) = lanes.first_mut() {
                    *first = sequenced(setup, first.clone());
                }
                arguments.extend(lanes);
            } else {
                arguments.push(value);
            }
        }
        if let Some(location) = self.caller_location_argument(callee, args.len(), *track_caller)? {
            arguments.push(location);
        }

        match callee {
            Callee::Direct { item, link_name } => {
                let name = match self.call_retargets.get(item) {
                    Some(name) => name.as_str().to_owned(),
                    None => self.link_name(*link_name)?,
                };
                let call = jsast::call(jsast::id(name), arguments);
                let tagged = physical
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.tagged.as_ref());
                let recursive = physical
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.recursive.as_ref());
                let i64_pair = physical
                    .and_then(|sig| sig.result.as_ref())
                    .is_some_and(|result| result.i64_pair);
                if tagged.is_none() && recursive.is_none() && !i64_pair {
                    return Ok(Lowered::value(call));
                }
                if op.result.is_none() {
                    return Ok(Lowered::Done(vec![jsast::expr_stmt(call)]));
                }
                if i64_pair {
                    let (bind_lo, lo) = self.temp(call);
                    let (bind_hi, hi) = self.temp(jsast::id(emit::spill_name_of(self.spills, 1)));
                    return Ok(Lowered::Pair(vec![bind_lo, bind_hi], (lo, hi)));
                }
                // The call runs before any spill read, then every lane is copied into a local:
                // another flattened call may reuse the module-scope registers before this value
                // is consumed. The ordinary object below is only a virtual semantic description;
                // `emit` caches it, and member folding erases it on the field-only/forwarded uses
                // whose proof admitted this result ABI.
                let (bind, tag) = self.temp(call);
                let mut prelude = vec![bind];
                let fields = tagged.map_or_else(
                    || recursive.expect("one physical result shape").fields(),
                    |tagged| tagged.fields(),
                );
                let payloads: Vec<Expr> = (1..fields)
                    .map(|index| {
                        let (bind, payload) =
                            self.temp(jsast::id(emit::spill_name_of(self.spills, index)));
                        prelude.push(bind);
                        payload
                    })
                    .collect();
                let value = match (tagged, recursive) {
                    (Some(tagged), None) => {
                        value::tagged_product_value(self.table, self.interns, tagged, tag, payloads)
                    }
                    (None, Some(recursive)) => {
                        let mut lanes = Vec::with_capacity(fields as usize);
                        lanes.push(tag);
                        lanes.extend(payloads);
                        value::recursive_product_value(self.table, self.interns, recursive, lanes)
                    }
                    _ => None,
                };
                let Some(value) = value else {
                    return Err(decline(
                        OpClass::Call,
                        "physical result has an unprintable product shape",
                    ));
                };
                Ok(Lowered::Virtual(prelude, value))
            }
            Callee::Foreign { symbol, .. } => Ok(Lowered::value(jsast::call(
                jsast::id(self.link_name(*symbol)?),
                arguments,
            ))),
            Callee::Runtime { member } => Ok(Lowered::value(jsast::rt_call(
                self.link_name(*member)?,
                arguments,
            ))),
            Callee::Indirect { .. } => {
                let Some(function) = function else {
                    return Err(decline(
                        OpClass::Call,
                        "an indirect call with no function value",
                    ));
                };
                Ok(Lowered::value(jsast::call(
                    self.expr_of(*function)?,
                    arguments,
                )))
            }
            // `slot` is absolute: `first_method_vtable_slot` counts the three header entries, so
            // it indexes the array `vtable.rs` builds directly.
            Callee::Virtual { slot, .. } => {
                let Some((receiver, rest)) = arguments.split_first() else {
                    return Err(decline(
                        OpClass::Call,
                        "a virtual call takes at least a receiver",
                    ));
                };
                let (setup, receiver) = self.reread(receiver.clone(), &["meta", "ptr"]);
                let data = jsast::member(receiver.clone(), ptr::PTR);
                let data = if owned_virtual_receiver {
                    jsast::rt_call("unref", vec![data])
                } else {
                    data
                };
                let mut call_args = vec![data];
                call_args.extend(rest.iter().cloned());
                let call = jsast::call(vtable_slot(receiver, f64::from(*slot)), call_args);
                let prelude = setup.map(jsast::expr_stmt).into_iter().collect();
                Ok(Lowered::Value(prelude, call))
            }
            // `ssa.rs` turns a `Callee::Intrinsic` into an `OpKind::Intrinsic`; a `Call` carrying
            // one is a body this lowering does not understand.
            Callee::Intrinsic { .. } => Err(decline(
                OpClass::Call,
                "an intrinsic callee on a `Call` op rather than an `Intrinsic` op",
            )),
            Callee::Nop => unreachable!("handled above"),
        }
    }

    /// The fat pointer a virtual call dispatches through.
    ///
    /// An owned trait-object receiver reaches JIR-R as `Deref(BorrowView(..))`: the `Deref` is the
    /// unsized `dyn Trait` place rustc passes, while the synthetic view is its `{ ptr, meta }` ABI
    /// value. Ordinary dereferences read the concrete value behind `ptr`; virtual dispatch needs
    /// the view itself so it can select `meta[slot]` and pass `ptr` to the selected method.
    fn virtual_receiver(&self, receiver: ValueId) -> Res<(Expr, bool)> {
        let mut value = receiver;
        for _ in 0..8 {
            let Some(def) = self.def_of(value) else {
                break;
            };
            match &def.kind {
                OpKind::Move(inner) | OpKind::Copy(inner) => value = *inner,
                OpKind::Deref(pointer)
                    if matches!(
                        value::kind(self.table, self.ty_of(value)?),
                        Some(rcgjs_object::jir::types::TypeKind::Dynamic { .. })
                    ) =>
                {
                    return Ok((self.expr_of(*pointer)?, true));
                }
                _ => break,
            }
        }
        Ok((self.expr_of(receiver)?, false))
    }

    /// The carrier of the place a reference value borrows, through alias-preserving renames.
    fn borrowed_carrier(&self, mut reference: rcgjs_object::jir::repr::ValueId) -> Option<Carrier> {
        for _ in 0..self.func.values.len() {
            let def = self.def_of(reference)?;
            match &def.kind {
                OpKind::Move(value) | OpKind::Copy(value) => reference = *value,
                OpKind::BorrowView {
                    root: Root::Value(value),
                    path,
                    ..
                } if path.as_slice() == [Proj::Deref] => reference = *value,
                OpKind::BorrowView {
                    root: Root::Value(value),
                    path,
                    ..
                } if path.is_empty() => {
                    let carrier = self.func.values.get(value.0 as usize)?.carrier;
                    return (carrier != Carrier::Baseline).then_some(carrier);
                }
                _ => return None,
            }
        }
        None
    }

    /// The trailing caller `Location` argument of a call, or `None` when the callee takes none
    /// (`intrinsics.rs::caller_location_argument`).
    ///
    /// `Some(fact)` is a concrete call site the encoder resolved. `None` is only ambiguous inside
    /// a `#[track_caller]` body, where a callee that tracks its caller is handed this body's own
    /// `$loc` instead: the driver's item index answers which, and its parameter count is checked
    /// against the argument list so a disagreement declines rather than mis-shapes the call.
    fn caller_location_argument(
        &self,
        callee: &Callee,
        args: usize,
        track_caller: Option<SourceFactId>,
    ) -> Res<Option<Expr>> {
        if let Some(fact) = track_caller {
            let Some(fact) = self.interns.source_fact(fact) else {
                return Err(decline(
                    OpClass::Call,
                    "a `#[track_caller]` call whose caller location fact is not in the interns",
                ));
            };
            return Ok(Some(self.caller_location_value(fact)?));
        }
        if !self.func.sig.abi.caller_location {
            return Ok(None);
        }
        // Inside a `#[track_caller]` body. Only a resolved Rust item can itself be
        // `#[track_caller]`: a foreign symbol, a `__rt` member and a function pointer never are
        // (v1 sets `tracks_caller` only on the resolved `FnDef` path).
        let item =
            match callee {
                Callee::Direct { item, .. } => *item,
                Callee::Foreign { .. } | Callee::Runtime { .. } | Callee::Indirect { .. } => {
                    return Ok(None)
                }
                _ => return Err(decline(
                    OpClass::Call,
                    "a call inside a `#[track_caller]` body whose callee ABI is not in the index",
                )),
            };
        match self.callee_takes_caller_location(item, args)? {
            true => Ok(Some(jsast::id(CALLER_LOCATION_PARAM))),
            false => Ok(None),
        }
    }

    /// Whether the callee item takes a trailing caller `Location`, checked against its parameter
    /// count.
    fn callee_takes_caller_location(&self, item: ItemId, args: usize) -> Res<bool> {
        let Some(entry) = self.items.get(&item) else {
            // The callee is not in the program at all: every call that reaches here is a
            // `ub_checks` precondition shim the mono collector never emitted, left behind in a
            // branch neither lane can execute. The baseline emits the call too — naming an
            // identifier nothing defines — so emitting it is what keeps the two lanes' reference
            // sets equal, and the trailing `$loc` is what the ABI of a `#[track_caller]` shim
            // takes. Which of the two it is cannot change behaviour: reaching the call is a
            // `ReferenceError` either way.
            return Ok(true);
        };
        // A missing ABI is distinct from a known zero-argument function. Never infer
        // caller-location forwarding from the default parameter count.
        if !entry.abi_known {
            return Err(decline(
                OpClass::Call,
                "a call inside a `#[track_caller]` body whose callee records no parameter shape",
            ));
        }
        if entry.params != args + usize::from(entry.caller_location) {
            return Err(decline(
                OpClass::Call,
                format!(
                    "a call passing {args} arguments to a callee that takes {} parameters",
                    entry.params
                ),
            ));
        }
        Ok(entry.caller_location)
    }

    /// An interned link name, as a `String`.
    fn link_name(&self, id: StrId) -> Res<String> {
        self.interns.str(id).map(str::to_owned).ok_or_else(|| {
            decline(
                OpClass::Call,
                "a callee whose link name is not in the interns",
            )
        })
    }

    /// A `Drop` op (`base.rs::codegen_drop`): the glue called with a reference to the place.
    fn lower_drop(&self, root: &Root, path: &[Proj], glue: DropGlue) -> Res<Lowered> {
        if let Root::Value(value) = root {
            if path.is_empty() && self.typed_storage(*value).is_some() {
                // Eligibility proves a POD element and an unobservable global allocator. The
                // exact backing store becomes unreachable with this value, so running the
                // generic NativeVec release helper can have no Rust-visible effect.
                return Ok(Lowered::Done(Vec::new()));
            }
        }
        match glue {
            // Empty glue is not emitted as an item, so naming it would dangle.
            DropGlue::None => Ok(Lowered::Done(Vec::new())),
            DropGlue::Direct { item, link_name } => {
                let argument = self.borrow_view(root, path)?;
                let target = self
                    .call_retargets
                    .get(&item)
                    .map(|name| name.as_str().to_owned())
                    .map(Ok)
                    .unwrap_or_else(|| self.link_name(link_name))?;
                Ok(Lowered::Done(vec![jsast::expr_stmt(jsast::call(
                    jsast::id(target),
                    vec![argument],
                ))]))
            }
            // A trait object's glue is slot 0 of the receiver's vtable, called with the data
            // pointer and skipped when the slot is null (`abi.rs::codegen_dyn_drop`).
            DropGlue::Dynamic => {
                let value = self.borrow_view(root, path)?;
                let (setup, receiver) = self.reread(value, &["meta", "ptr"]);
                let slot = vtable_slot(receiver.clone(), VTABLE_DROP_SLOT);
                let mut out: Vec<Stmt> = setup.map(jsast::expr_stmt).into_iter().collect();
                out.push(jsast::if_(
                    jsast::binary(jsast::BinOp::StrictNe, slot.clone(), jsast::null()),
                    vec![jsast::expr_stmt(jsast::call(
                        slot,
                        vec![jsast::member(receiver, ptr::PTR)],
                    ))],
                ));
                Ok(Lowered::Done(out))
            }
        }
    }
}

/// `receiver.meta[slot]`: one entry of the vtable a fat pointer carries (`abi.rs::vtable_slot`).
pub fn vtable_slot(receiver: Expr, slot: f64) -> Expr {
    jsast::index(super::ptr::meta(receiver), jsast::num(slot))
}
