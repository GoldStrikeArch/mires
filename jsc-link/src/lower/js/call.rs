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

enum StrRange {
    Full,
    From { start: Expr, range: Expr },
    To { end: Expr, range: Expr },
    Bounds { start: Expr, end: Expr, range: Expr },
}

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

        // `js-str=bytes`: a `str` method with a producer-tagged identity is the JavaScript string
        // operation it is on a byte string (`lower/bstr.rs`).
        if let CallSemantic::Standard(operation) = semantic {
            if operation.is_str() && crate::lower::bstr::enabled() {
                if let Some(lowered) =
                    self.lower_str_call(op, *operation, callee, args, *track_caller)?
                {
                    return Ok(lowered);
                }
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
            Callee::Runtime { member, .. } => {
                // An `extern "C"` item is the host's (`__rt.<symbol>`): under `js-str=bytes` every
                // `str` an argument reaches is decoded to the host text it encodes and every `str`
                // the result reaches is encoded into the byte string Rust holds (`lower/bstr.rs`).
                if crate::lower::bstr::enabled() {
                    for (argument, arg) in arguments.iter_mut().zip(args.iter()) {
                        if self.is_known_ascii(*arg) {
                            continue;
                        }
                        *argument = host::crossing(
                            self,
                            self.ty_of(*arg)?,
                            argument.clone(),
                            crate::lower::bstr::Crossing::Out,
                        )?;
                    }
                }
                let call = jsast::rt_call(self.link_name(*member)?, arguments);
                match op.result {
                    Some(result) if crate::lower::bstr::enabled() => {
                        Ok(Lowered::value(host::crossing(
                            self,
                            self.ty_of(result)?,
                            call,
                            crate::lower::bstr::Crossing::In,
                        )?))
                    }
                    _ => Ok(Lowered::value(call)),
                }
            }
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

    /// A call to one of the producer-tagged `str` identities under `js-str=bytes`, spelled as the
    /// JavaScript string operation it is on byte strings: a length is `length`, a `&str` or `char`
    /// pattern is a substring search (UTF-8 is self-synchronizing, so a byte-string match of
    /// well-formed text is a match at a char boundary, which is what Rust's searcher reports),
    /// equality is `===` and order is `<` (code-unit order of byte strings is UTF-8 byte order,
    /// which is Rust's). A slice is `substring` behind the char-boundary checks; when they fail
    /// the original call runs instead, so the panic, its message and its location are Rust's own.
    ///
    /// `None` (an ordinary call) for anything the lowering does not see exactly: a callee the
    /// flat ABI reshaped, an unused result, or argument types other than the producer's.
    fn lower_str_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        callee: &Callee,
        args: &[ValueId],
        track_caller: Option<SourceFactId>,
    ) -> Res<Option<Lowered>> {
        let Callee::Direct { item, link_name } = callee else {
            return Ok(None);
        };
        if self.abi.is_some_and(|abi| {
            abi.call_signature(*item)
                .is_some_and(|sig| sig.result.is_some())
        }) {
            return Ok(None);
        }
        let Some(result) = op.result else {
            return Ok(None);
        };
        let table = self.table;
        let is_str = |value: ValueId| -> Res<bool> {
            Ok(crate::lower::bstr::is_str_pointer(
                table,
                self.ty_of(value)?,
            ))
        };
        let is_char = |value: ValueId| -> Res<bool> {
            Ok(matches!(
                value::kind(table, self.ty_of(value)?),
                Some(rcgjs_object::jir::types::TypeKind::Char)
            ))
        };
        let mut prelude = Vec::new();
        // Each argument is evaluated once, in order; one read more than once is held.
        let mut hold = |expr: Expr, prelude: &mut Vec<Stmt>| -> Expr {
            if matches!(expr, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
                expr
            } else {
                let (bind, held) = self.temp(expr);
                prelude.push(bind);
                held
            }
        };
        let string = |value: ValueId,
                      prelude: &mut Vec<Stmt>,
                      hold: &mut dyn FnMut(Expr, &mut Vec<Stmt>) -> Expr|
         -> Res<Expr> { Ok(hold(self.expr_of(value)?, prelude)) };
        let needle = |value: ValueId,
                      prelude: &mut Vec<Stmt>,
                      hold: &mut dyn FnMut(Expr, &mut Vec<Stmt>) -> Expr|
         -> Res<Option<Expr>> {
            if is_str(value)? {
                return Ok(Some(hold(self.expr_of(value)?, prelude)));
            }
            if is_char(value)? {
                return Ok(Some(match self.expr_of(value)? {
                    Expr::Num(point) if (0.0..128.0).contains(&point) => {
                        jsast::string(char::from(point as u8).to_string())
                    }
                    point => hold(crate::lower::bstr::char_text(point), prelude),
                }));
            }
            Ok(None)
        };
        let value = match (operation, args) {
            (StandardOperation::StrLen, [s]) if is_str(*s)? => {
                jsast::member(string(*s, &mut prelude, &mut hold)?, "length")
            }
            (StandardOperation::StrIsCharBoundary, [s, index]) if is_str(*s)? => {
                let s = string(*s, &mut prelude, &mut hold)?;
                let index = hold(self.expr_of(*index)?, &mut prelude);
                jsast::rt_call("bstr_boundary", vec![s, index])
            }
            (
                StandardOperation::StrStartsWith
                | StandardOperation::StrEndsWith
                | StandardOperation::StrContains
                | StandardOperation::StrFind
                | StandardOperation::StrRFind,
                [s, pattern],
            ) if is_str(*s)? && (is_str(*pattern)? || is_char(*pattern)?) => {
                let find = matches!(
                    operation,
                    StandardOperation::StrFind | StandardOperation::StrRFind
                );
                let variants = match find {
                    true => match self.option_variants(result)? {
                        Some(variants) => Some(variants),
                        None => return Ok(None),
                    },
                    false => None,
                };
                let s = string(*s, &mut prelude, &mut hold)?;
                let Some(pattern) = needle(*pattern, &mut prelude, &mut hold)? else {
                    return Ok(None);
                };
                let method = match operation {
                    StandardOperation::StrStartsWith => "startsWith",
                    StandardOperation::StrEndsWith => "endsWith",
                    StandardOperation::StrContains => "includes",
                    StandardOperation::StrFind => "indexOf",
                    _ => "lastIndexOf",
                };
                let found = jsast::method_call(s, method, vec![pattern]);
                match variants {
                    Some((none, some)) => {
                        let (bind, at) = self.temp(found);
                        prelude.push(bind);
                        let ty = self.ty_of(result)?;
                        let (Some(none), Some(some)) = (
                            value::enum_value(table, self.interns, ty, none, Vec::new()),
                            value::enum_value(
                                table,
                                self.interns,
                                ty,
                                some.0,
                                vec![(some.1, at.clone())],
                            ),
                        ) else {
                            return Ok(None);
                        };
                        jsast::cond(
                            jsast::binary(jsast::BinOp::Lt, at, jsast::num(0)),
                            none,
                            some,
                        )
                    }
                    _ => found,
                }
            }
            (StandardOperation::StrEq, [a, b]) if is_str(*a)? && is_str(*b)? => {
                let a = string(*a, &mut prelude, &mut hold)?;
                let b = string(*b, &mut prelude, &mut hold)?;
                jsast::binary(jsast::BinOp::StrictEq, a, b)
            }
            (StandardOperation::StrCmp, [a, b]) if is_str(*a)? && is_str(*b)? => {
                let ordering_ty = self.ty_of(result)?;
                let a = string(*a, &mut prelude, &mut hold)?;
                let b = string(*b, &mut prelude, &mut hold)?;
                let Some(cmp) = self.str_cmp_expr(ordering_ty, a, b) else {
                    return Ok(None);
                };
                cmp
            }
            (StandardOperation::StrPartialCmp, [a, b]) if is_str(*a)? && is_str(*b)? => {
                let Some((_, some)) = self.option_variants(result)? else {
                    return Ok(None);
                };
                let result_ty = self.ty_of(result)?;
                let Some(ordering_field) = table.field(result_ty, Some(some.0), 0) else {
                    return Ok(None);
                };
                let a = string(*a, &mut prelude, &mut hold)?;
                let b = string(*b, &mut prelude, &mut hold)?;
                let Some(cmp) = self.str_cmp_expr(ordering_field.ty, a, b) else {
                    return Ok(None);
                };
                let Some(wrapped) =
                    value::enum_value(table, self.interns, result_ty, some.0, vec![(some.1, cmp)])
                else {
                    return Ok(None);
                };
                wrapped
            }
            (
                StandardOperation::StrTrim
                | StandardOperation::StrTrimStart
                | StandardOperation::StrTrimEnd,
                [s],
            ) if is_str(*s)? => {
                let helper = match operation {
                    StandardOperation::StrTrim => "bstr_trim",
                    StandardOperation::StrTrimStart => "bstr_trim_start",
                    _ => "bstr_trim_end",
                };
                jsast::rt_call(helper, vec![string(*s, &mut prelude, &mut hold)?])
            }
            (StandardOperation::StrStripPrefix | StandardOperation::StrStripSuffix, [s, pattern])
                if is_str(*s)? && (is_str(*pattern)? || is_char(*pattern)?) =>
            {
                let Some((none, some)) = self.option_variants(result)? else {
                    return Ok(None);
                };
                let s = string(*s, &mut prelude, &mut hold)?;
                let Some(pattern) = needle(*pattern, &mut prelude, &mut hold)? else {
                    return Ok(None);
                };
                let ty = self.ty_of(result)?;
                let (method, rest) = if matches!(operation, StandardOperation::StrStripPrefix) {
                    (
                        "startsWith",
                        jsast::method_call(
                            s.clone(),
                            "slice",
                            vec![jsast::member(pattern.clone(), "length")],
                        ),
                    )
                } else {
                    let end = jsast::binary(
                        jsast::BinOp::Sub,
                        jsast::member(s.clone(), "length"),
                        jsast::member(pattern.clone(), "length"),
                    );
                    ("endsWith", jsast::method_call(s.clone(), "slice", vec![jsast::num(0), end]))
                };
                let (Some(none), Some(some)) = (
                    value::enum_value(table, self.interns, ty, none, Vec::new()),
                    value::enum_value(table, self.interns, ty, some.0, vec![(some.1, rest)]),
                ) else {
                    return Ok(None);
                };
                jsast::cond(jsast::method_call(s, method, vec![pattern]), some, none)
            }
            (StandardOperation::StrGet, [s, range]) if is_str(*s)? => {
                let Some((none, some)) = self.option_variants(result)? else {
                    return Ok(None);
                };
                let ty = self.ty_of(result)?;
                let s = string(*s, &mut prelude, &mut hold)?;
                let Some(shape) = self.decompose_str_range(*range, &mut prelude, &mut hold)? else {
                    return Ok(None);
                };
                let Some(none_val) = value::enum_value(table, self.interns, ty, none, Vec::new())
                else {
                    return Ok(None);
                };
                let got = match shape {
                    StrRange::Full => {
                        let Some(some_val) =
                            value::enum_value(table, self.interns, ty, some.0, vec![(some.1, s)])
                        else {
                            return Ok(None);
                        };
                        return Ok(Some(Lowered::Value(prelude, some_val)));
                    }
                    StrRange::From { start, .. } => jsast::rt_call("bstr_get_from", vec![s, start]),
                    StrRange::To { end, .. } => jsast::rt_call("bstr_get_to", vec![s, end]),
                    StrRange::Bounds { start, end, .. } => {
                        jsast::rt_call("bstr_get", vec![s, start, end])
                    }
                };
                let (bind, at) = self.temp(got);
                prelude.push(bind);
                let Some(some_val) =
                    value::enum_value(table, self.interns, ty, some.0, vec![(some.1, at.clone())])
                else {
                    return Ok(None);
                };
                jsast::cond(
                    jsast::binary(jsast::BinOp::StrictEq, at, jsast::undefined()),
                    none_val,
                    some_val,
                )
            }
            (StandardOperation::StrIndex, [s, range]) if is_str(*s)? => {
                let s = string(*s, &mut prelude, &mut hold)?;
                let Some(shape) = self.decompose_str_range(*range, &mut prelude, &mut hold)? else {
                    return Ok(None);
                };
                let (helper, mut sub_args, range_expr) = match shape {
                    StrRange::Full => return Ok(Some(Lowered::Value(prelude, s))),
                    StrRange::Bounds { start, end, range } => {
                        ("bstr_sub", vec![s.clone(), start, end], range)
                    }
                    StrRange::From { start, range } => {
                        ("bstr_sub_from", vec![s.clone(), start], range)
                    }
                    StrRange::To { end, range } => ("bstr_sub_to", vec![s.clone(), end], range),
                };
                let location = self.caller_location_argument(callee, args.len(), track_caller)?;
                match (self.ascii().and_then(|a| a.slice_error_fail), location) {
                    (Some(fail_fn), Some(loc)) => {
                        sub_args.push(jsast::id(fail_fn));
                        sub_args.push(loc);
                    }
                    (_, loc) => {
                        if self
                            .abi
                            .is_some_and(|abi| abi.call_signature(*item).is_some())
                        {
                            return Ok(None);
                        }
                        let name = match self.call_retargets.get(item) {
                            Some(name) => name.as_str().to_owned(),
                            None => self.link_name(*link_name)?,
                        };
                        let mut orig_args = vec![s, range_expr];
                        if let Some(loc) = loc {
                            orig_args.push(loc);
                        }
                        sub_args.push(jsast::arrow(
                            Vec::new(),
                            jsast::call(jsast::id(name), orig_args),
                        ));
                    }
                }
                jsast::rt_call(helper, sub_args)
            }
            _ => return Ok(None),
        };
        Ok(Some(Lowered::Value(prelude, value)))
    }

    /// Three-way `Ordering` result expression `a < b ? Less : a === b ? Equal : Greater`.
    fn str_cmp_expr(&self, ordering_ty: rcgjs_object::jir::types::TypeId, a: Expr, b: Expr) -> Option<Expr> {
        let table = self.table;
        let variant = |name: &str| -> Option<Expr> {
            let index = table
                .variants(table.peeled(ordering_ty))
                .iter()
                .position(|v| value::variant_name(self.interns, v) == name)?;
            value::enum_value(table, self.interns, ordering_ty, index as u32, Vec::new())
        };
        let (Some(less), Some(equal), Some(greater)) =
            (variant("Less"), variant("Equal"), variant("Greater"))
        else {
            return None;
        };
        Some(jsast::cond(
            jsast::binary(jsast::BinOp::Lt, a.clone(), b.clone()),
            less,
            jsast::cond(jsast::binary(jsast::BinOp::StrictEq, a, b), equal, greater),
        ))
    }

    /// Decomposes a `Range`, `RangeFrom`, `RangeTo`, or `RangeFull` value into held endpoints plus
    /// a lazy fallback range expression.
    fn decompose_str_range(
        &self,
        range: ValueId,
        prelude: &mut Vec<Stmt>,
        hold: &mut dyn FnMut(Expr, &mut Vec<Stmt>) -> Expr,
    ) -> Res<Option<StrRange>> {
        let table = self.table;
        let range_ty = table.peeled(self.ty_of(range)?);
        let variants = table.variants(range_ty);
        let [variant] = variants else {
            return Ok(None);
        };
        let mut keys = Vec::new();
        for field in &variant.fields {
            match value::key_name(self.interns, field.key) {
                Some(key) => keys.push(key),
                None => return Ok(None),
            }
        }
        let key_strs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let (fields, range_expr) = match self.expr_of(range)? {
            Expr::Object(entries)
                if entries
                    .iter()
                    .map(|(k, _)| k.as_str())
                    .eq(key_strs.iter().copied()) =>
            {
                let held: Vec<(String, Expr)> = entries
                    .into_iter()
                    .map(|(key, value)| (key, hold(value, prelude)))
                    .collect();
                (held.clone(), Expr::Object(held))
            }
            other => {
                let held = hold(other, prelude);
                let fields = key_strs
                    .iter()
                    .map(|key| ((*key).to_owned(), jsast::member(held.clone(), *key)))
                    .collect();
                (fields, held)
            }
        };
        let field = |key: &str| {
            fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(jsast::undefined)
        };
        Ok(match key_strs.as_slice() {
            [] => Some(StrRange::Full),
            ["start", "end"] => Some(StrRange::Bounds {
                start: field("start"),
                end: field("end"),
                range: range_expr,
            }),
            ["start"] => Some(StrRange::From {
                start: field("start"),
                range: range_expr,
            }),
            ["end"] => Some(StrRange::To {
                end: field("end"),
                range: range_expr,
            }),
            _ => None,
        })
    }

    /// The `(None, (Some, field key))` variant indices of an `Option` result.
    fn option_variants(&self, result: ValueId) -> Res<Option<(u32, (u32, String))>> {
        let table = self.table;
        let peeled = table.peeled(self.ty_of(result)?);
        let variants = table.variants(peeled);
        let index = |name: &str| {
            variants
                .iter()
                .position(|variant| value::variant_name(self.interns, variant) == name)
        };
        let (Some(none), Some(some)) = (index("None"), index("Some")) else {
            return Ok(None);
        };
        let Some(key) = variants[some]
            .fields
            .first()
            .and_then(|field| value::key_name(self.interns, field.key))
        else {
            return Ok(None);
        };
        Ok(Some((none as u32, (some as u32, key))))
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
            if path.is_empty() {
                if self.typed_storage(*value).is_some() {
                    // Eligibility proves a POD element and an unobservable global allocator. The
                    // exact backing store becomes unreachable with this value, so running the
                    // generic NativeVec release helper can have no Rust-visible effect.
                    return Ok(Lowered::Done(Vec::new()));
                }
                match self.dropped_string_kind(*value) {
                    Some(emit::DroppedStringKind::NoOp) => {
                        let is_host_str = self.func.values.get(value.0 as usize).map(|d| d.carrier)
                            == Some(Carrier::HostString);
                        if is_host_str || self.only_used_by_empty_drop(*value) {
                            let mut stmts = Vec::new();
                            if self.is_inline(*value) {
                                let target = match self.def_of(*value).map(|op| &op.kind) {
                                    Some(OpKind::Convert { v: src, .. }) => *src,
                                    _ => *value,
                                };
                                let expr = self.expr_of(target)?;
                                if !jsast::is_droppable(&expr) {
                                    stmts.push(jsast::expr_stmt(expr));
                                }
                            }
                            return Ok(Lowered::Done(stmts));
                        }
                    }
                    Some(emit::DroppedStringKind::BaselineDealloc { src, layout })
                        if self.is_inline(*value) || self.only_used_by_adjacent_drop(*value) =>
                    {
                        let src_expr = self.expr_of(src)?;
                        let mut stmts = Vec::new();
                        let base = if matches!(src_expr, Expr::Ident(_)) {
                            src_expr
                        } else {
                            let (bind, name) = self.temp(src_expr);
                            stmts.push(bind);
                            name
                        };
                        let cap = layout.member(base.clone(), &layout.cap);
                        let ptr = layout.member(base, &layout.ptr);
                        if cap != jsast::num(0) {
                            stmts.push(jsast::if_(
                                jsast::binary(jsast::BinOp::StrictNe, cap.clone(), jsast::num(0)),
                                vec![jsast::expr_stmt(jsast::rt_call(
                                    "dealloc",
                                    vec![ptr, cap, jsast::num(1)],
                                ))],
                            ));
                        }
                        return Ok(Lowered::Done(stmts));
                    }
                    _ => {}
                }
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
