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
use rcgjs_object::jir::repr::{Op, OpKind, Proj, Root, ValueId};
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
                track_caller,
            } => {
                if let Some(lowered) = self.lower_soa_intrinsic(op, *name, types, args)? {
                    return Ok(lowered);
                }
                intrinsics::lower(self, op, *name, types, *fallback, args, *track_caller)
            }
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
        if matches!(
            semantic,
            CallSemantic::Standard(StandardOperation::VecAsSlice)
        ) && self.heap().native_vec_enabled
        {
            if let Some(lowered) =
                self.lower_soa_vec_call(op, StandardOperation::NativeVecAsSlice, args)?
            {
                return Ok(lowered);
            }
            if let Some(lowered) =
                self.lower_native_vec_call(op, StandardOperation::NativeVecAsSlice, args)?
            {
                return Ok(lowered);
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
            if self.heap().native_string_enabled {
                let receiver = self.expr_of(*receiver)?;
                return Ok(Lowered::value(jsast::member(receiver, "s")));
            }
        }

        // Phase 5 fast `core::str` comparison/search hooks (`ReleaseSpeed`, plus exact JS string
        // predicates `BstrEq`/`BstrStartsWith`/`BstrEndsWith`/`BstrContains` whenever `js-opt=on`,
        // since UTF-8 and UTF-16 equality/prefix/suffix/substring predicates agree identically).
        if let CallSemantic::Standard(operation) = semantic {
            let release_speed = self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed)
                || crate::opts::get().profile == Some(crate::opts::Profile::ReleaseSpeed);
            let exact_str_pred = self.opts.applies(crate::opts::OptimizerPass::AstDce)
                && matches!(
                    operation,
                    StandardOperation::BstrEq
                        | StandardOperation::BstrStartsWith
                        | StandardOperation::BstrEndsWith
                        | StandardOperation::BstrContains
                );
            if operation.is_fast_str_op() && (release_speed || exact_str_pred) {
                if let Some(lowered) = self.lower_fast_str_call(op, *operation, args)? {
                    return Ok(lowered);
                }
            }
        }

        // Phase 5 universal native `{ s, cap }` `String` hooks (`ReleaseSpeed` + `js-str=bytes`).
        if let CallSemantic::Standard(operation) = semantic {
            if self.heap().native_string_enabled
                && (operation.is_native_string()
                    || matches!(
                        operation,
                        StandardOperation::StrToLowercase | StandardOperation::StrToUppercase
                    ))
            {
                if let Some(lowered) = self.lower_native_string_call(*operation, args)? {
                    return Ok(lowered);
                }
            }
        }

        // Mires v4 Phase 3 universal flat `Vec<T, Global>` hooks (`ReleaseSpeed` + `Default`
        // allocator + `!byte_views` + `!non_rc_raw_alloc_reachable`).
        if let CallSemantic::Standard(operation) = semantic {
            if self.heap().native_vec_enabled {
                if operation.is_native_vec() {
                    if let Some(lowered) = self.lower_soa_vec_call(op, *operation, args)? {
                        return Ok(lowered);
                    }
                    if let Some(lowered) = self.lower_native_vec_call(op, *operation, args)? {
                        return Ok(lowered);
                    }
                } else if *operation == StandardOperation::VecResize {
                    if let Some(lowered) = self.lower_soa_vec_resize(args)? {
                        return Ok(lowered);
                    }
                    if let Some(lowered) = self.lower_native_vec_resize(args)? {
                        return Ok(lowered);
                    }
                }
            }
        }

        // Tier A typed managed heap (`ReleaseSpeed && Default allocator && !byte_views`): lower
        // `Global` `RawVec`/`Box`/`BTreeMap`/`Rc` allocation hooks directly to zero-WeakMap
        // TypedArrays / plain Arrays and no-op deallocations.
        if let CallSemantic::Standard(operation) = semantic {
            if operation.is_typed_heap() && self.heap().typed_heap_enabled {
                if let Some(lowered) = self.lower_typed_heap_call(op, *operation, args)? {
                    return Ok(lowered);
                }
            }
        }

        // `js-str=bytes`: a `str` method with a producer-tagged identity is the JavaScript string
        // operation it is on a byte string (`lower/bstr.rs`).
        if let CallSemantic::Standard(operation) = semantic {
            if operation.is_str()
                && (crate::lower::bstr::enabled()
                    || self.opts.str_repr == crate::opts::StrRepr::Bytes)
            {
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

        // Phase 2C: Native ByteStr `core::slice::<impl [u8]>::{starts_with,ends_with}` and inline
        // `core::slice::<impl [T]>::{reverse,revswap}` under `ReleaseSpeed`.
        if (self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed)
            || crate::opts::get().profile == Some(crate::opts::Profile::ReleaseSpeed))
            && track_caller.is_none()
        {
            if crate::lower::bstr::enabled() || self.opts.str_repr == crate::opts::StrRepr::Bytes {
                if let Some(lowered) = self.lower_bstr_slice_search(op, callee, args)? {
                    return Ok(lowered);
                }
            }
            if let Some(lowered) = self.lower_slice_reverse(op, callee, args)? {
                return Ok(lowered);
            }
        }

        let physical = match (callee, self.abi) {
            (Callee::Direct { item, .. }, Some(abi)) => abi.call_signature(*item),
            _ => None,
        };
        let slice_positions = physical.map_or_else(Vec::new, |sig| sig.slice_jir_positions());
        let byte_str_positions = physical.map_or_else(Vec::new, |sig| sig.byte_str_jir_positions());
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
            if let Some(&(_, bs_mode)) = byte_str_positions
                .iter()
                .find(|(bs_pos, _)| *bs_pos == position)
            {
                let Some(bv) = self.byte_view(*arg) else {
                    return Err(decline(
                        OpClass::Call,
                        "byte_str ABI argument did not survive byte_view_analysis",
                    ));
                };
                match bs_mode {
                    crate::plan::abi_flat::ByteStrParam::Full => {
                        arguments.push(self.byte_view_to_str_expr(*arg)?);
                    }
                    crate::plan::abi_flat::ByteStrParam::Sub => {
                        let base = self.byte_view_base_expr(bv, *arg)?;
                        let off = self.byte_view_off_expr(*arg)?;
                        let len = self.byte_view_len_expr(*arg)?;
                        arguments.extend([base, off, len]);
                    }
                }
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
                    Expr::Num(point)
                        if point.fract() == 0.0 && (0.0..=0x10_FFFF as f64).contains(&point) =>
                    {
                        if let Some(ch) = char::from_u32(point as u32) {
                            let mut buf = [0u8; 4];
                            jsast::string(crate::lower::bstr::byte_string(ch.encode_utf8(&mut buf)))
                        } else {
                            hold(crate::lower::bstr::char_text(Expr::Num(point)), prelude)
                        }
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
            (StandardOperation::StrByteAt, [s, index]) if is_str(*s)? => {
                let s = string(*s, &mut prelude, &mut hold)?;
                let index = hold(self.expr_of(*index)?, &mut prelude);
                jsast::method_call(s, "charCodeAt", vec![index])
            }
            (StandardOperation::StrSliceUnchecked, [s, start, end]) if is_str(*s)? => {
                let s = string(*s, &mut prelude, &mut hold)?;
                let start = hold(self.expr_of(*start)?, &mut prelude);
                let end = hold(self.expr_of(*end)?, &mut prelude);
                jsast::method_call(s, "slice", vec![start, end])
            }
            // A byte string is ASCII when every code unit is; `core`'s word-at-a-time scan would
            // first build the string's byte array and then read it through scalar views.
            (StandardOperation::StrIsAscii, [s]) if is_str(*s)? => {
                let expr = self.expr_of(*s)?;
                if self.is_known_ascii(*s) {
                    if !jsast::is_droppable(&expr) {
                        prelude.push(jsast::expr_stmt(expr));
                    }
                    Expr::Bool(true)
                } else {
                    jsast::rt_call("_bstr_is_ascii", vec![hold(expr, &mut prelude)])
                }
            }
            // The same answer one byte at a time over a `{ buf, off, len }` slice (or directly
            // over a `js-str=bytes` byte-string view).
            (StandardOperation::SliceIsAscii, [bytes])
                if matches!(
                    table.get(table.peeled(self.ty_of(*bytes)?)).map(|d| &d.kind),
                    Some(rcgjs_object::jir::types::TypeKind::Reference {
                        pointee,
                        mutable: false,
                    }) if matches!(
                        table.get(table.peeled(*pointee)).map(|d| &d.kind),
                        Some(rcgjs_object::jir::types::TypeKind::Slice(elem))
                            if table.byte_element(*elem)
                    )
                ) =>
            {
                if let Some(bv) = self.byte_view(*bytes) {
                    if matches!(
                        bv.mode,
                        super::ByteViewMode::FullSlice | super::ByteViewMode::VarSlice
                    ) && bv.base_str.is_some_and(|b| self.is_known_ascii(b))
                    {
                        let expr = self.expr_of(*bytes)?;
                        if !jsast::is_droppable(&expr) {
                            prelude.push(jsast::expr_stmt(expr));
                        }
                        Expr::Bool(true)
                    } else {
                        match bv.mode {
                            super::ByteViewMode::FullSlice => {
                                jsast::rt_call("_bstr_is_ascii", vec![self.expr_of(*bytes)?])
                            }
                            super::ByteViewMode::VarSlice => {
                                let (base, off, len) =
                                    self.extract_byte_view_var_slice(bv, *bytes, &mut prelude)?;
                                jsast::rt_call("_bstr_is_ascii_range", vec![base, off, len])
                            }
                            super::ByteViewMode::ThinPtr => {
                                jsast::rt_call("bytes_is_ascii", vec![self.expr_of(*bytes)?])
                            }
                        }
                    }
                } else {
                    jsast::rt_call("bytes_is_ascii", vec![self.expr_of(*bytes)?])
                }
            }
            (StandardOperation::StrIsCharBoundary, [s, index]) if is_str(*s)? => {
                let s_expr = string(*s, &mut prelude, &mut hold)?;
                let index_expr = hold(self.expr_of(*index)?, &mut prelude);
                if self.is_known_ascii(*s) {
                    let is_u32 = matches!(
                        value::kind(table, table.peeled(self.ty_of(*index)?)),
                        Some(rcgjs_object::jir::types::TypeKind::Integer {
                            signed: false,
                            bits: ..=32
                        })
                    );
                    let le_len = jsast::binary(
                        jsast::BinOp::Le,
                        index_expr.clone(),
                        jsast::member(s_expr, "length"),
                    );
                    if is_u32 {
                        le_len
                    } else {
                        jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::Ge, index_expr, jsast::num(0)),
                            le_len,
                        )
                    }
                } else {
                    jsast::rt_call("bstr_boundary", vec![s_expr, index_expr])
                }
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
                if matches!(
                    operation,
                    StandardOperation::StrStartsWith
                        | StandardOperation::StrEndsWith
                        | StandardOperation::StrContains
                ) {
                    if let Expr::Str(ref lit) = pattern {
                        if lit.is_empty() {
                            if !jsast::is_droppable(&s) {
                                prelude.push(jsast::expr_stmt(s));
                            }
                            return Ok(Some(Lowered::Value(prelude, Expr::Bool(true))));
                        }
                        if matches!(
                            operation,
                            StandardOperation::StrStartsWith | StandardOperation::StrEndsWith
                        ) && lit.encode_utf16().count() == 1
                        {
                            let code = f64::from(lit.encode_utf16().next().unwrap());
                            let idx = if matches!(operation, StandardOperation::StrStartsWith) {
                                jsast::num(0)
                            } else {
                                jsast::binary(
                                    jsast::BinOp::Sub,
                                    jsast::member(s.clone(), "length"),
                                    jsast::num(1),
                                )
                            };
                            return Ok(Some(Lowered::Value(
                                prelude,
                                jsast::binary(
                                    jsast::BinOp::StrictEq,
                                    jsast::method_call(s, "charCodeAt", vec![idx]),
                                    jsast::num(code),
                                ),
                            )));
                        }
                    }
                }
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
            (
                StandardOperation::StrStripPrefix | StandardOperation::StrStripSuffix,
                [s, pattern],
            ) if is_str(*s)? && (is_str(*pattern)? || is_char(*pattern)?) => {
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
                    (
                        "endsWith",
                        jsast::method_call(s.clone(), "slice", vec![jsast::num(0), end]),
                    )
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
                let s_known_ascii = self.is_known_ascii(*s);
                let s = string(*s, &mut prelude, &mut hold)?;
                let Some(shape) = self.decompose_str_range(*range, &mut prelude, &mut hold)? else {
                    return Ok(None);
                };
                let Some(none_val) = value::enum_value(table, self.interns, ty, none, Vec::new())
                else {
                    return Ok(None);
                };
                if s_known_ascii {
                    let (cond, sub) = match shape {
                        StrRange::Full => {
                            let Some(some_val) = value::enum_value(
                                table,
                                self.interns,
                                ty,
                                some.0,
                                vec![(some.1, s)],
                            ) else {
                                return Ok(None);
                            };
                            return Ok(Some(Lowered::Value(prelude, some_val)));
                        }
                        StrRange::From { start, .. } => (
                            jsast::binary(
                                jsast::BinOp::And,
                                jsast::binary(jsast::BinOp::Ge, start.clone(), jsast::num(0)),
                                jsast::binary(
                                    jsast::BinOp::Le,
                                    start.clone(),
                                    jsast::member(s.clone(), "length"),
                                ),
                            ),
                            jsast::method_call(s, "slice", vec![start]),
                        ),
                        StrRange::To { end, .. } => (
                            jsast::binary(
                                jsast::BinOp::And,
                                jsast::binary(jsast::BinOp::Ge, end.clone(), jsast::num(0)),
                                jsast::binary(
                                    jsast::BinOp::Le,
                                    end.clone(),
                                    jsast::member(s.clone(), "length"),
                                ),
                            ),
                            jsast::method_call(s, "slice", vec![jsast::num(0), end]),
                        ),
                        StrRange::Bounds { start, end, .. } => (
                            jsast::binary(
                                jsast::BinOp::And,
                                jsast::binary(
                                    jsast::BinOp::And,
                                    jsast::binary(jsast::BinOp::Ge, start.clone(), jsast::num(0)),
                                    jsast::binary(jsast::BinOp::Le, start.clone(), end.clone()),
                                ),
                                jsast::binary(
                                    jsast::BinOp::Le,
                                    end.clone(),
                                    jsast::member(s.clone(), "length"),
                                ),
                            ),
                            jsast::method_call(s, "slice", vec![start, end]),
                        ),
                    };
                    let Some(some_val) =
                        value::enum_value(table, self.interns, ty, some.0, vec![(some.1, sub)])
                    else {
                        return Ok(None);
                    };
                    return Ok(Some(Lowered::Value(
                        prelude,
                        jsast::cond(cond, some_val, none_val),
                    )));
                }
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

    /// Lowers the five Phase 3 Tier A typed-heap `Global` hooks (`StandardOperation::is_typed_heap`)
    /// directly at call sites when `self.heap().typed_heap_enabled` holds.
    fn lower_typed_heap_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        use super::ast_util::TA_CTORS;
        let table = self.table;
        let heap = self.heap();

        let const_u128 = |v: ValueId| -> Option<u128> {
            match self.def_of(v).map(|d| &d.kind) {
                Some(OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar {
                    bits,
                    ..
                })) => Some(*bits),
                _ => None,
            }
        };

        let result_pointee = || -> Option<rcgjs_object::jir::types::TypeId> {
            let res = op.result?;
            let ty = self.ty_of(res).ok()?;
            value::pointee_of(table, ty)
        };

        match operation {
            StandardOperation::BoxNew => {
                let [val] = args else {
                    return Ok(None);
                };
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                let peeled = table.peeled(elem);
                let expr = self.expr_of(*val)?;
                let tracked =
                    heap.needs_box_tracking(table, elem) && value::is_slot_pointee(table, peeled);
                let slot = if tracked && table.is_indirect(peeled) {
                    jsast::rt_call("t_box_new_tracked", vec![expr])
                } else {
                    ptr::slot(jsast::array(vec![expr]), jsast::num(0))
                };
                Ok(Some(Lowered::value(slot)))
            }
            StandardOperation::TypedAlloc => {
                let [cap_v, zeroed_v] = args else {
                    return Ok(None);
                };
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                let peeled = table.peeled(elem);
                let cap = self.expr_of(*cap_v)?;
                if let Some(k) = table
                    .get(elem)
                    .and_then(crate::lower::semantic_ops::typed_array_kind)
                {
                    let buf = jsast::new(jsast::id(TA_CTORS[k as usize]), vec![cap]);
                    return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
                }
                let tracked =
                    heap.needs_box_tracking(table, elem) && value::is_slot_pointee(table, peeled);
                let needs_shell = table.is_indirect(peeled)
                    || matches!(
                        value::kind(table, peeled),
                        Some(rcgjs_object::jir::types::TypeKind::Array { .. })
                    );
                if const_u128(*zeroed_v) == Some(0) {
                    if !needs_shell || const_u128(*cap_v).is_some_and(|n| n != 1) {
                        let buf = jsast::new(jsast::id("Array"), vec![cap]);
                        return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
                    }
                    let shell_expr = uninit_shell(table, self.interns, elem, 0, heap);
                    if const_u128(*cap_v) == Some(1) {
                        if let Some(shell) = shell_expr {
                            let slot = if tracked {
                                jsast::rt_call("t_box_new_tracked", vec![shell])
                            } else {
                                ptr::slot(jsast::array(vec![shell]), jsast::num(0))
                            };
                            return Ok(Some(Lowered::value(slot)));
                        }
                        let buf = jsast::new(jsast::id("Array"), vec![jsast::num(1)]);
                        return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
                    }
                    if let Some(shell) = shell_expr {
                        let mut call_args = vec![cap, jsast::arrow(Vec::new(), shell)];
                        if tracked {
                            call_args.push(Expr::Bool(true));
                        }
                        return Ok(Some(Lowered::value(jsast::rt_call(
                            "t_alloc_arr",
                            call_args,
                        ))));
                    }
                    let buf = jsast::new(jsast::id("Array"), vec![cap]);
                    return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
                }
                let zeroed = self.expr_of(*zeroed_v)?;
                let mk = match value::zero_value_for_heap_plan(table, self.interns, elem, heap) {
                    Some(v) => jsast::arrow(Vec::new(), v),
                    None => jsast::null(),
                };
                let mut call_args = vec![cap, zeroed, mk];
                if tracked {
                    call_args.push(Expr::Bool(true));
                }
                Ok(Some(Lowered::value(jsast::rt_call("t_alloc_z", call_args))))
            }
            StandardOperation::TypedRealloc => {
                let [ptr_v, old_cap_v, new_cap_v] = args else {
                    return Ok(None);
                };
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                let peeled = table.peeled(elem);
                let p = self.expr_of(*ptr_v)?;
                let old_cap = self.expr_of(*old_cap_v)?;
                let new_cap = self.expr_of(*new_cap_v)?;
                let checked = heap.non_rc_raw_alloc_reachable;
                if let Some(k) = table
                    .get(elem)
                    .and_then(crate::lower::semantic_ops::typed_array_kind)
                {
                    let helper = if checked {
                        "t_realloc_ta_checked"
                    } else {
                        "t_realloc_ta"
                    };
                    return Ok(Some(Lowered::value(jsast::rt_call(
                        helper,
                        vec![p, old_cap, new_cap, jsast::num(f64::from(k))],
                    ))));
                }
                if !checked {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "t_realloc_arr",
                        vec![p, new_cap],
                    ))))
                } else {
                    let es = table
                        .get(peeled)
                        .and_then(|d| d.layout.size)
                        .unwrap_or(1)
                        .max(1);
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "t_realloc_arr_checked",
                        vec![p, old_cap, new_cap, jsast::num(es as f64)],
                    ))))
                }
            }
            StandardOperation::TypedDealloc => {
                let Some(&ptr_v) = args.first() else {
                    return Ok(None);
                };
                if !heap.non_rc_raw_alloc_reachable {
                    return Ok(Some(Lowered::Done(Vec::new())));
                }
                let p = self.expr_of(ptr_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                    "t_dealloc_maybe",
                    vec![p],
                ))])))
            }
            StandardOperation::TypedDeallocRaw => {
                let Some(&ptr_v) = args.first() else {
                    return Ok(None);
                };
                if !heap.raw_alloc_reachable {
                    return Ok(Some(Lowered::Done(Vec::new())));
                }
                let p = self.expr_of(ptr_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                    "t_dealloc_maybe",
                    vec![p],
                ))])))
            }
            StandardOperation::RcAllocSlice | StandardOperation::ArcAllocSlice => {
                let [len_v] = args else {
                    return Ok(None);
                };
                let Some(rc_inner_ty) = result_pointee() else {
                    return Ok(None);
                };
                let Some(tail_info) = table.unsized_tail(rc_inner_ty) else {
                    return Ok(None);
                };
                let tail_idx = tail_info.field as usize;
                let Some(rcgjs_object::jir::types::TypeKind::Adt { variants, .. }) =
                    value::kind(table, rc_inner_ty)
                else {
                    return Ok(None);
                };
                let [variant] = variants.as_slice() else {
                    return Ok(None);
                };
                let Some(tail_field) = variant.fields.get(tail_idx) else {
                    return Ok(None);
                };
                let Some(rcgjs_object::jir::types::TypeKind::Slice(elem_ty)) =
                    value::kind(table, tail_field.ty)
                else {
                    return Ok(None);
                };
                let elem_peeled = table.peeled(*elem_ty);
                let len_expr = self.expr_of(*len_v)?;
                let (setup, len_ref) = self.reread(len_expr, &[]);
                let tail_buf = if table.is_zst(elem_peeled) {
                    let align = table
                        .get(*elem_ty)
                        .and_then(|d| d.layout.align)
                        .unwrap_or(1)
                        .max(1);
                    jsast::num(align as f64)
                } else if let Some(k) = table
                    .get(*elem_ty)
                    .and_then(crate::lower::semantic_ops::typed_array_kind)
                {
                    jsast::new(jsast::id(TA_CTORS[k as usize]), vec![len_ref.clone()])
                } else {
                    jsast::new(jsast::id("Array"), vec![len_ref.clone()])
                };

                let mut fields: Vec<(String, Expr)> = Vec::with_capacity(variant.fields.len());
                for (idx, field) in variant.fields.iter().enumerate() {
                    let Some(key_name) = value::key_name(self.interns, field.key) else {
                        return Ok(None);
                    };
                    if idx == tail_idx {
                        fields.push((key_name, tail_buf.clone()));
                        continue;
                    }
                    // Header counter field (`Cell<usize>` peels to `usize` -> `1`;
                    // `AtomicUsize { v: Atomic<usize>(UnsafeCell<usize>) }` -> `{ v: { _0: 1 } }`).
                    fn init_counter_one(
                        table: &rcgjs_object::jir::types::TypeTable,
                        interns: &rcgjs_object::jir::Interns,
                        ty: rcgjs_object::jir::types::TypeId,
                        depth: u8,
                    ) -> Option<Expr> {
                        if depth > 4 {
                            return None;
                        }
                        let peeled = table.peeled(ty);
                        match value::kind(table, peeled) {
                            Some(rcgjs_object::jir::types::TypeKind::Integer { .. }) => {
                                Some(jsast::num(1))
                            }
                            Some(rcgjs_object::jir::types::TypeKind::Adt { variants, .. }) => {
                                let [v] = variants.as_slice() else {
                                    return None;
                                };
                                let mut real_fields =
                                    v.fields.iter().filter(|f| !table.is_zst(f.ty));
                                let inner_f = real_fields.next()?;
                                if real_fields.next().is_some() {
                                    return None;
                                }
                                let ik = value::key_name(interns, inner_f.key)?;
                                let iv = init_counter_one(table, interns, inner_f.ty, depth + 1)?;
                                Some(Expr::Object(vec![(ik, iv)]))
                            }
                            _ => None,
                        }
                    }
                    let Some(init) = init_counter_one(table, self.interns, field.ty, 0) else {
                        return Ok(None);
                    };
                    fields.push((key_name, init));
                }

                let record = Expr::Object(fields);
                let slot = if heap.needs_box_tracking(table, rc_inner_ty) {
                    jsast::rt_call("t_box_new_tracked", vec![record])
                } else {
                    ptr::slot(jsast::array(vec![record]), jsast::num(0))
                };
                let fat = jsast::object_of(vec![("ptr", slot), ("meta", len_ref)]);
                Ok(Some(Lowered::value(super::ptr::sequenced(setup, fat))))
            }
            _ => Ok(None),
        }
    }

    /// Lowers Phase 5 `alloc::string::rcgjs_str_*` hooks and `str::to_{lower,upper}case` directly
    /// against the universal `{ s: byteString, cap: number }` `String` representation.
    fn lower_native_string_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let const_u128 = |v: ValueId| -> Option<u128> {
            match self.def_of(v).map(|def| &def.kind) {
                Some(OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar {
                    bits,
                    ..
                })) => Some(*bits),
                _ => None,
            }
        };
        let bind_if_needed = |e: Expr, setup: &mut Vec<Stmt>| -> Expr {
            if matches!(e, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
                e
            } else {
                let (stmt, id) = self.temp(e);
                setup.push(stmt);
                id
            }
        };
        match operation {
            StandardOperation::NativeStrNew => Ok(Some(Lowered::value(jsast::object_of(vec![
                ("s", jsast::string("")),
                ("cap", jsast::num(0)),
            ])))),
            StandardOperation::NativeStrWithCapacity => {
                let [cap_v] = args else {
                    return Ok(None);
                };
                if let Some(c) = const_u128(*cap_v) {
                    if c <= 2_147_483_647 {
                        return Ok(Some(Lowered::value(jsast::object_of(vec![
                            ("s", jsast::string("")),
                            ("cap", jsast::num(c as f64)),
                        ]))));
                    }
                }
                let cap = self.expr_of(*cap_v)?;
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nstr_with_cap",
                    vec![cap],
                ))))
            }
            StandardOperation::NativeStrFromStr => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let obj =
                    jsast::object_of(vec![("s", s.clone()), ("cap", jsast::member(s, "length"))]);
                Ok(Some(Lowered::Value(setup, obj)))
            }
            StandardOperation::NativeStrAsStr => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                Ok(Some(Lowered::value(jsast::member(s, "s"))))
            }
            StandardOperation::NativeStrLen => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                Ok(Some(Lowered::value(jsast::member(
                    jsast::member(s, "s"),
                    "length",
                ))))
            }
            StandardOperation::NativeStrCapacity => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let cap = jsast::member(s.clone(), "cap");
                let len = jsast::member(jsast::member(s, "s"), "length");
                let expr = jsast::cond(
                    jsast::binary(jsast::BinOp::Gt, cap.clone(), len.clone()),
                    cap,
                    len,
                );
                Ok(Some(Lowered::Value(setup, expr)))
            }
            StandardOperation::NativeStrPushStr => {
                let [s_v, rhs_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let rhs = self.expr_of(*rhs_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(
                    jsast::compound_assign(jsast::BinOp::Add, jsast::member(s, "s"), rhs),
                )])))
            }
            StandardOperation::NativeStrPush => {
                let [s_v, ch_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                if let Some(c) = const_u128(*ch_v) {
                    if let Some(ch) = u32::try_from(c).ok().and_then(char::from_u32) {
                        let mut buf = [0u8; 4];
                        let piece = crate::lower::bstr::literal(ch.encode_utf8(&mut buf));
                        return Ok(Some(Lowered::Done(vec![jsast::expr_stmt(
                            jsast::compound_assign(jsast::BinOp::Add, jsast::member(s, "s"), piece),
                        )])));
                    }
                }
                let mut setup = Vec::new();
                let s = bind_if_needed(s, &mut setup);
                let ch = bind_if_needed(self.expr_of(*ch_v)?, &mut setup);
                let piece = jsast::cond(
                    jsast::binary(jsast::BinOp::Lt, ch.clone(), jsast::num(128)),
                    jsast::method_call(jsast::id("String"), "fromCharCode", vec![ch.clone()]),
                    jsast::rt_call("bstr_char", vec![ch]),
                );
                setup.push(jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    jsast::member(s, "s"),
                    piece,
                )));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrClear => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                Ok(Some(Lowered::Done(vec![jsast::assign_stmt(
                    jsast::member(s, "s"),
                    jsast::string(""),
                )])))
            }
            StandardOperation::NativeStrTruncate => {
                let [s_v, len_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let new_len = bind_if_needed(self.expr_of(*len_v)?, &mut setup);
                let field = jsast::member(s, "s");
                setup.push(jsast::if_(
                    jsast::binary(
                        jsast::BinOp::Lt,
                        new_len.clone(),
                        jsast::member(field.clone(), "length"),
                    ),
                    vec![jsast::assign_stmt(
                        field.clone(),
                        jsast::method_call(field, "slice", vec![jsast::num(0), new_len]),
                    )],
                ));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrReserve => {
                let [s_v, add_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let add = self.expr_of(*add_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                    "nstr_reserve",
                    vec![s, add],
                ))])))
            }
            StandardOperation::NativeStrShrinkTo => {
                let [s_v, min_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let min_cap = bind_if_needed(self.expr_of(*min_v)?, &mut setup);
                let (b_len, len) =
                    self.temp(jsast::member(jsast::member(s.clone(), "s"), "length"));
                setup.push(b_len);
                let (b_t, target) = self.temp(jsast::cond(
                    jsast::binary(jsast::BinOp::Gt, min_cap.clone(), len.clone()),
                    min_cap,
                    len,
                ));
                setup.push(b_t);
                let cap_field = jsast::member(s, "cap");
                setup.push(jsast::if_(
                    jsast::binary(jsast::BinOp::Lt, target.clone(), cap_field.clone()),
                    vec![jsast::assign_stmt(cap_field, target)],
                ));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrFromVec => {
                let [vec_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let v = bind_if_needed(self.expr_of(*vec_v)?, &mut setup);
                let len = jsast::member(v.clone(), "len");
                if self.heap().native_vec_enabled {
                    let buf = jsast::member(v, "buf");
                    let s_expr = jsast::cond(
                        jsast::binary(jsast::BinOp::StrictEq, len.clone(), jsast::num(0)),
                        jsast::string(""),
                        jsast::rt_call("bstr_of", vec![buf.clone(), jsast::num(0), len]),
                    );
                    let obj = jsast::object_of(vec![
                        ("s", s_expr),
                        ("cap", jsast::member(buf, "length")),
                    ]);
                    return Ok(Some(Lowered::Value(setup, obj)));
                }
                let inner = jsast::member(jsast::member(v, "buf"), "inner");
                let ptr = jsast::member(inner.clone(), "ptr");
                let s_expr = jsast::cond(
                    jsast::binary(jsast::BinOp::StrictEq, len.clone(), jsast::num(0)),
                    jsast::string(""),
                    jsast::rt_call(
                        "bstr_of",
                        vec![
                            jsast::member(ptr.clone(), "buf"),
                            jsast::member(ptr, "off"),
                            len,
                        ],
                    ),
                );
                let obj =
                    jsast::object_of(vec![("s", s_expr), ("cap", jsast::member(inner, "cap"))]);
                Ok(Some(Lowered::Value(setup, obj)))
            }
            StandardOperation::NativeStrIntoBytes => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let helper = if self.heap().native_vec_enabled {
                    "nstr_into_nvec"
                } else {
                    "nstr_into_vec"
                };
                Ok(Some(Lowered::value(jsast::rt_call(helper, vec![s]))))
            }
            StandardOperation::NativeStrReplaceSlice => {
                let [s_v, start_v, end_v, rep_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let start = bind_if_needed(self.expr_of(*start_v)?, &mut setup);
                let end = bind_if_needed(self.expr_of(*end_v)?, &mut setup);
                let rep = bind_if_needed(self.expr_of(*rep_v)?, &mut setup);
                let field = jsast::member(s, "s");
                let head = jsast::method_call(field.clone(), "slice", vec![jsast::num(0), start]);
                let tail = jsast::method_call(field.clone(), "slice", vec![end]);
                let concat = jsast::binary(
                    jsast::BinOp::Add,
                    jsast::binary(jsast::BinOp::Add, head, rep),
                    tail,
                );
                setup.push(jsast::assign_stmt(field, concat));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrExtendWithin => {
                let [s_v, start_v, end_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = bind_if_needed(self.expr_of(*s_v)?, &mut setup);
                let start = bind_if_needed(self.expr_of(*start_v)?, &mut setup);
                let end = bind_if_needed(self.expr_of(*end_v)?, &mut setup);
                let field = jsast::member(s, "s");
                let sub = jsast::method_call(field.clone(), "slice", vec![start, end]);
                setup.push(jsast::expr_stmt(jsast::compound_assign(
                    jsast::BinOp::Add,
                    field,
                    sub,
                )));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrCloneFrom => {
                let [dst_v, src_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let dst = bind_if_needed(self.expr_of(*dst_v)?, &mut setup);
                let src = bind_if_needed(self.expr_of(*src_v)?, &mut setup);
                let src_s = jsast::member(src, "s");
                setup.push(jsast::assign_stmt(
                    jsast::member(dst.clone(), "s"),
                    src_s.clone(),
                ));
                let dst_cap = jsast::member(dst, "cap");
                let src_len = jsast::member(src_s, "length");
                setup.push(jsast::if_(
                    jsast::binary(jsast::BinOp::Gt, src_len.clone(), dst_cap.clone()),
                    vec![jsast::assign_stmt(dst_cap, src_len)],
                ));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeStrToAsciiLowercase => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nstr_ascii_lower",
                    vec![s],
                ))))
            }
            StandardOperation::NativeStrToAsciiUppercase => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nstr_ascii_upper",
                    vec![s],
                ))))
            }
            StandardOperation::NativeStrRepeat => {
                let [s_v, n_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let n = self.expr_of(*n_v)?;
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nstr_repeat",
                    vec![s, n],
                ))))
            }
            StandardOperation::StrToLowercase | StandardOperation::StrToUppercase => {
                let [s_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let converted = if self.is_known_ascii(*s_v) {
                    let method = if operation == StandardOperation::StrToLowercase {
                        "toLowerCase"
                    } else {
                        "toUpperCase"
                    };
                    jsast::method_call(s, method, Vec::new())
                } else {
                    let helper = if operation == StandardOperation::StrToLowercase {
                        "bstr_to_lowercase"
                    } else {
                        "bstr_to_uppercase"
                    };
                    jsast::rt_call(helper, vec![s])
                };
                let (bind, t) = self.temp(converted);
                let obj =
                    jsast::object_of(vec![("s", t.clone()), ("cap", jsast::member(t, "length"))]);
                Ok(Some(Lowered::Value(vec![bind], obj)))
            }
            _ => Ok(None),
        }
    }

    /// Lowers `Vec::resize(&mut self, new_len, value)` on numeric `TypedArray`-backed
    /// `Vec<T, Global>` directly to reserve + native `TypedArray.prototype.fill` (C++ `memset`).
    fn lower_native_vec_resize(&self, args: &[ValueId]) -> Res<Option<Lowered>> {
        const TA_SUFFIXES: [&str; 8] = ["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];
        let [v_v, new_len_v, val_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let arg_ty = self.ty_of(*v_v)?;
        let Some(vec_ty) = value::pointee_of(table, arg_ty) else {
            return Ok(None);
        };
        let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, vec_ty) else {
            return Ok(None);
        };
        let Some(elem_desc) = table.get(elem_ty) else {
            return Ok(None);
        };
        if elem_desc.layout.zst {
            return Ok(None);
        }
        let Some(k) = crate::analysis::heap::native_vec_ta_kind(table, elem_ty) else {
            return Ok(None);
        };
        let elem_size = elem_desc.layout.size.unwrap_or(1).max(1);
        let min_cap: u64 = if elem_size == 1 {
            8
        } else if elem_size <= 1024 {
            4
        } else {
            1
        };
        let max_elems: u64 = 2_147_483_647u64 / elem_size;
        let suffix = TA_SUFFIXES[k as usize];

        let mut setup = Vec::new();
        let bind_atom = |expr: Expr, setup: &mut Vec<Stmt>| -> Expr {
            if matches!(
                expr,
                Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_)
            ) {
                expr
            } else {
                let (bind, id) = self.temp(expr);
                setup.push(bind);
                id
            }
        };
        let v = bind_atom(self.expr_of(*v_v)?, &mut setup);
        let nl = bind_atom(self.expr_of(*new_len_v)?, &mut setup);
        let val = bind_atom(self.expr_of(*val_v)?, &mut setup);

        let (b_ol, ol) = self.temp(jsast::member(v.clone(), "len"));
        setup.push(b_ol);
        let grow_body = vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::Gt,
                    nl.clone(),
                    jsast::member(jsast::member(v.clone(), "buf"), "length"),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    format!("nv_reserve_{suffix}"),
                    vec![
                        v.clone(),
                        ol.clone(),
                        jsast::binary(jsast::BinOp::Sub, nl.clone(), ol.clone()),
                        jsast::num(min_cap as f64),
                        Expr::Bool(false),
                        jsast::num(max_elems as f64),
                    ],
                ))],
            ),
            jsast::expr_stmt(jsast::method_call(
                jsast::member(v.clone(), "buf"),
                "fill",
                vec![val, ol.clone(), nl.clone()],
            )),
        ];
        setup.push(jsast::if_(
            jsast::binary(jsast::BinOp::Gt, nl.clone(), ol),
            grow_body,
        ));
        setup.push(jsast::assign_stmt(jsast::member(v, "len"), nl));
        Ok(Some(Lowered::Done(setup)))
    }

    /// Lowers Mires v4 Phase 3 universal flat `Vec<T, Global>` hooks
    /// (`StandardOperation::is_native_vec`) directly at call sites when
    /// `self.heap().native_vec_enabled` holds.
    fn lower_native_vec_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        use super::ast_util::TA_CTORS;
        use rcgjs_object::jir::types::{NominalForm, PointerForm, TypeKind};
        const TA_SUFFIXES: [&str; 8] = ["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];

        let table = self.table;
        let heap = self.heap();

        let const_u128 = |v: ValueId| -> Option<u128> {
            match self.def_of(v).map(|d| &d.kind) {
                Some(OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar {
                    bits,
                    ..
                })) => Some(*bits),
                _ => None,
            }
        };

        let bind_if_needed = |expr: Expr, setup: &mut Vec<Stmt>| -> Expr {
            if matches!(expr, Expr::Ident(_)) {
                expr
            } else {
                let (bind, id) = self.temp(expr);
                setup.push(bind);
                id
            }
        };
        let bind_atom = |expr: Expr, setup: &mut Vec<Stmt>| -> Expr {
            if matches!(
                expr,
                Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_)
            ) {
                expr
            } else {
                let (bind, id) = self.temp(expr);
                setup.push(bind);
                id
            }
        };

        let vec_ty = match operation {
            StandardOperation::NativeVecNew
            | StandardOperation::NativeVecWithCapacity
            | StandardOperation::NativeVecFromElemZeroed
            | StandardOperation::NativeVecFromArray
            | StandardOperation::NativeVecFromRawParts => {
                let Some(res) = op.result else {
                    return Ok(None);
                };
                self.ty_of(res)?
            }
            StandardOperation::NativeVecIntoBoxedSlice => {
                let Some(&v0) = args.first() else {
                    return Ok(None);
                };
                self.ty_of(v0)?
            }
            _ => {
                let Some(&v0) = args.first() else {
                    return Ok(None);
                };
                let arg_ty = self.ty_of(v0)?;
                let Some(pt) = value::pointee_of(table, arg_ty) else {
                    return Ok(None);
                };
                pt
            }
        };

        let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, vec_ty) else {
            return Ok(None);
        };
        let Some(elem_desc) = table.get(elem_ty) else {
            return Ok(None);
        };
        let is_zst = elem_desc.layout.zst;
        let elem_size = elem_desc.layout.size.unwrap_or(0);
        let align = elem_desc.layout.align.unwrap_or(1).max(1);
        let ta_kind = if is_zst {
            None
        } else {
            crate::analysis::heap::native_vec_ta_kind(table, elem_ty)
        };
        let packed_arr = !is_zst && ta_kind.is_none() && heap.is_packed_arr_vec(table, vec_ty);
        let min_cap: u64 = if elem_size == 1 {
            8
        } else if elem_size <= 1024 {
            4
        } else {
            1
        };
        let max_elems: u64 = 2_147_483_647u64
            .checked_div(elem_size)
            .unwrap_or(4_294_967_295);
        let peeled_elem = table.peeled(elem_ty);
        let tracked = !is_zst
            && ta_kind.is_none()
            && heap.needs_box_tracking(table, elem_ty)
            && table.is_indirect(peeled_elem)
            && value::is_slot_pointee(table, peeled_elem);

        match operation {
            StandardOperation::NativeVecNew => {
                let Some(empty) = value::empty_native_vec_expr_for_heap(table, vec_ty, heap) else {
                    return Ok(None);
                };
                Ok(Some(Lowered::value(empty)))
            }
            StandardOperation::NativeVecWithCapacity => {
                let [cap_v] = args else {
                    return Ok(None);
                };
                if is_zst {
                    let cap_e = self.expr_of(*cap_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&cap_e) {
                        setup.push(jsast::expr_stmt(cap_e));
                    }
                    return Ok(Some(Lowered::Value(
                        setup,
                        jsast::object_of(vec![("buf", jsast::undefined()), ("len", jsast::num(0))]),
                    )));
                }
                if let Some(c) = const_u128(*cap_v) {
                    if c == 0 {
                        let Some(empty) =
                            value::empty_native_vec_expr_for_heap(table, vec_ty, heap)
                        else {
                            return Ok(None);
                        };
                        return Ok(Some(Lowered::value(empty)));
                    }
                    if c <= u128::from(max_elems) {
                        if let Some(k) = ta_kind {
                            return Ok(Some(Lowered::value(jsast::object_of(vec![
                                (
                                    "buf",
                                    jsast::new(
                                        jsast::id(TA_CTORS[k as usize]),
                                        vec![jsast::num(c as f64)],
                                    ),
                                ),
                                ("len", jsast::num(0)),
                            ]))));
                        }
                        if packed_arr {
                            return Ok(Some(Lowered::value(jsast::object_of(vec![
                                ("buf", jsast::array(Vec::new())),
                                ("len", jsast::num(0)),
                            ]))));
                        }
                        return Ok(Some(Lowered::value(jsast::object_of(vec![
                            (
                                "buf",
                                jsast::new(jsast::id("Array"), vec![jsast::num(c as f64)]),
                            ),
                            ("len", jsast::num(0)),
                        ]))));
                    }
                }
                let cap = self.expr_of(*cap_v)?;
                if let Some(k) = ta_kind {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        format!("nv_with_cap_{}", TA_SUFFIXES[k as usize]),
                        vec![cap, jsast::num(max_elems as f64)],
                    ))))
                } else if packed_arr {
                    let mut setup = Vec::new();
                    let cap = bind_atom(cap, &mut setup);
                    setup.push(jsast::if_(
                        jsast::binary(jsast::BinOp::Gt, cap, jsast::num(max_elems as f64)),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "js_abort",
                            vec![jsast::string("capacity overflow")],
                        ))],
                    ));
                    Ok(Some(Lowered::Value(
                        setup,
                        jsast::object_of(vec![
                            ("buf", jsast::array(Vec::new())),
                            ("len", jsast::num(0)),
                        ]),
                    )))
                } else {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "nv_with_cap_arr",
                        vec![cap, jsast::num(align as f64), jsast::num(max_elems as f64)],
                    ))))
                }
            }
            StandardOperation::NativeVecFromElemZeroed => {
                let [n_v] = args else {
                    return Ok(None);
                };
                if is_zst {
                    let n = self.expr_of(*n_v)?;
                    return Ok(Some(Lowered::value(jsast::object_of(vec![
                        ("buf", jsast::undefined()),
                        ("len", n),
                    ]))));
                }
                if let Some(k) = ta_kind {
                    if let Some(c) = const_u128(*n_v) {
                        if c == 0 {
                            let Some(empty) =
                                value::empty_native_vec_expr_for_heap(table, vec_ty, heap)
                            else {
                                return Ok(None);
                            };
                            return Ok(Some(Lowered::value(empty)));
                        }
                        if c <= u128::from(max_elems) {
                            return Ok(Some(Lowered::value(jsast::object_of(vec![
                                (
                                    "buf",
                                    jsast::new(
                                        jsast::id(TA_CTORS[k as usize]),
                                        vec![jsast::num(c as f64)],
                                    ),
                                ),
                                ("len", jsast::num(c as f64)),
                            ]))));
                        }
                    }
                    let n = self.expr_of(*n_v)?;
                    return Ok(Some(Lowered::value(jsast::rt_call(
                        format!("nv_zeroed_{}", TA_SUFFIXES[k as usize]),
                        vec![n, jsast::num(max_elems as f64)],
                    ))));
                }
                let Some(zero) =
                    value::zero_value_for_heap_plan(table, self.interns, elem_ty, heap)
                else {
                    return Ok(None);
                };
                let n = self.expr_of(*n_v)?;
                if !table.is_indirect(peeled_elem) {
                    if packed_arr {
                        Ok(Some(Lowered::value(jsast::rt_call(
                            "nv_zeroed_arr_p",
                            vec![n, jsast::num(max_elems as f64), zero],
                        ))))
                    } else {
                        Ok(Some(Lowered::value(jsast::rt_call(
                            "nv_zeroed_arr",
                            vec![
                                n,
                                jsast::num(align as f64),
                                jsast::num(max_elems as f64),
                                zero,
                            ],
                        ))))
                    }
                } else if packed_arr {
                    let helper = if tracked {
                        "nv_zeroed_arr_fn_tracked_p"
                    } else {
                        "nv_zeroed_arr_fn_p"
                    };
                    Ok(Some(Lowered::value(jsast::rt_call(
                        helper,
                        vec![
                            n,
                            jsast::num(max_elems as f64),
                            jsast::arrow(Vec::new(), zero),
                        ],
                    ))))
                } else {
                    let helper = if tracked {
                        "nv_zeroed_arr_fn_tracked"
                    } else {
                        "nv_zeroed_arr_fn"
                    };
                    Ok(Some(Lowered::value(jsast::rt_call(
                        helper,
                        vec![
                            n,
                            jsast::num(align as f64),
                            jsast::num(max_elems as f64),
                            jsast::arrow(Vec::new(), zero),
                        ],
                    ))))
                }
            }
            StandardOperation::NativeVecFromArray => {
                let [arr_v] = args else {
                    return Ok(None);
                };
                let arr_ty = self.ty_of(*arr_v)?;
                let Some(TypeKind::Array { length, .. }) =
                    table.get(table.peeled(arr_ty)).map(|d| &d.kind)
                else {
                    return Ok(None);
                };
                let len = *length;
                if is_zst {
                    let arr_e = self.expr_of(*arr_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&arr_e) {
                        setup.push(jsast::expr_stmt(arr_e));
                    }
                    return Ok(Some(Lowered::Value(
                        setup,
                        jsast::object_of(vec![
                            ("buf", jsast::undefined()),
                            ("len", jsast::num(len as f64)),
                        ]),
                    )));
                }
                if len == 0 {
                    let arr_e = self.expr_of(*arr_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&arr_e) {
                        setup.push(jsast::expr_stmt(arr_e));
                    }
                    let Some(empty) = value::empty_native_vec_expr_for_heap(table, vec_ty, heap)
                    else {
                        return Ok(None);
                    };
                    return Ok(Some(Lowered::Value(setup, empty)));
                }
                let arr_expr = self.expr_of(*arr_v)?;
                if let Some(k) = ta_kind {
                    return Ok(Some(Lowered::value(jsast::object_of(vec![
                        (
                            "buf",
                            jsast::new(jsast::id(TA_CTORS[k as usize]), vec![arr_expr]),
                        ),
                        ("len", jsast::num(len as f64)),
                    ]))));
                }
                let fresh_owned = {
                    let mut cur = *arr_v;
                    for _ in 0..8 {
                        if let Some(OpKind::Move(inner)) = self.def_of(cur).map(|d| &d.kind) {
                            cur = *inner;
                        } else {
                            break;
                        }
                    }
                    match self.def_of(cur).map(|d| &d.kind) {
                        Some(OpKind::Aggregate { .. } | OpKind::Repeat { .. }) => true,
                        // `OpKind::Copy` already deep-clones `[T; N]` in `emit.rs` when `needs_clone` holds.
                        Some(OpKind::Copy(_)) => table.needs_clone(arr_ty),
                        _ => false,
                    }
                };
                let mut owned_arr = if fresh_owned {
                    arr_expr
                } else {
                    value::clone_expr_with(table, self.interns, arr_ty, arr_expr, self.nested_clone)
                };
                if tracked {
                    owned_arr = jsast::rt_call("nv_box_track_arr", vec![owned_arr]);
                }
                Ok(Some(Lowered::value(jsast::object_of(vec![
                    ("buf", owned_arr),
                    ("len", jsast::num(len as f64)),
                ]))))
            }
            StandardOperation::NativeVecFromRawParts => {
                let [ptr_v, len_v, cap_v] = args else {
                    return Ok(None);
                };
                let ptr = self.expr_of(*ptr_v)?;
                let len = self.expr_of(*len_v)?;
                let cap = self.expr_of(*cap_v)?;
                if is_zst {
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&ptr) {
                        setup.push(jsast::expr_stmt(ptr));
                    }
                    if !jsast::is_droppable(&cap) {
                        setup.push(jsast::expr_stmt(cap));
                    }
                    return Ok(Some(Lowered::Value(
                        setup,
                        jsast::object_of(vec![("buf", jsast::undefined()), ("len", len)]),
                    )));
                }
                if let Some(k) = ta_kind {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "nv_from_raw_ta",
                        vec![ptr, len, cap, jsast::num(f64::from(k))],
                    ))))
                } else {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "nv_from_raw_arr",
                        vec![ptr, len, cap, jsast::num(align as f64)],
                    ))))
                }
            }
            StandardOperation::NativeVecCapacity => {
                let [v_v] = args else {
                    return Ok(None);
                };
                if is_zst {
                    let ve = self.expr_of(*v_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&ve) {
                        setup.push(jsast::expr_stmt(ve));
                    }
                    return Ok(Some(Lowered::Value(setup, jsast::num(4_294_967_295.0))));
                }
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(jsast::member(
                    jsast::member(v, "buf"),
                    "length",
                ))))
            }
            StandardOperation::NativeVecAsPtr | StandardOperation::NativeVecAsMutPtr => {
                let [v_v] = args else {
                    return Ok(None);
                };
                if is_zst {
                    let ve = self.expr_of(*v_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&ve) {
                        setup.push(jsast::expr_stmt(ve));
                    }
                    return Ok(Some(Lowered::Value(setup, jsast::num(align as f64))));
                }
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(ptr::slot(
                    jsast::member(v, "buf"),
                    jsast::num(0),
                ))))
            }
            StandardOperation::NativeVecAsSlice | StandardOperation::NativeVecAsMutSlice => {
                let [v_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let v = bind_if_needed(self.expr_of(*v_v)?, &mut setup);
                let buf = if is_zst {
                    jsast::num(align as f64)
                } else {
                    jsast::member(v.clone(), "buf")
                };
                let slice = jsast::object_of(vec![
                    ("buf", buf),
                    ("off", jsast::num(0)),
                    ("len", jsast::member(v, "len")),
                ]);
                Ok(Some(Lowered::Value(setup, slice)))
            }
            StandardOperation::NativeVecReserve | StandardOperation::NativeVecReserveExact => {
                let [v_v, used_v, add_v] = args else {
                    return Ok(None);
                };
                let exact = operation == StandardOperation::NativeVecReserveExact;
                let v = self.expr_of(*v_v)?;
                let used = self.expr_of(*used_v)?;
                let add = self.expr_of(*add_v)?;
                if is_zst {
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&v) {
                        setup.push(jsast::expr_stmt(v));
                    }
                    setup.push(jsast::if_(
                        jsast::binary(
                            jsast::BinOp::Gt,
                            jsast::binary(jsast::BinOp::Add, used, add),
                            jsast::num(4_294_967_295.0),
                        ),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "js_abort",
                            vec![jsast::string("capacity overflow")],
                        ))],
                    ));
                    return Ok(Some(Lowered::Done(setup)));
                }
                if packed_arr {
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&v) {
                        setup.push(jsast::expr_stmt(v));
                    }
                    setup.push(jsast::if_(
                        jsast::binary(
                            jsast::BinOp::Gt,
                            jsast::binary(jsast::BinOp::Add, used, add),
                            jsast::num(max_elems as f64),
                        ),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "js_abort",
                            vec![jsast::string("capacity overflow")],
                        ))],
                    ));
                    return Ok(Some(Lowered::Done(setup)));
                }
                let mut setup = Vec::new();
                let v = bind_if_needed(v, &mut setup);
                let used = bind_atom(used, &mut setup);
                let add = bind_atom(add, &mut setup);
                let call = if let Some(k) = ta_kind {
                    jsast::rt_call(
                        format!("nv_reserve_{}", TA_SUFFIXES[k as usize]),
                        vec![
                            v.clone(),
                            used.clone(),
                            add.clone(),
                            jsast::num(min_cap as f64),
                            Expr::Bool(exact),
                            jsast::num(max_elems as f64),
                        ],
                    )
                } else {
                    jsast::rt_call(
                        "nv_reserve_arr",
                        vec![
                            v.clone(),
                            used.clone(),
                            add.clone(),
                            jsast::num(min_cap as f64),
                            Expr::Bool(exact),
                            jsast::num(max_elems as f64),
                        ],
                    )
                };
                setup.push(jsast::if_(
                    jsast::binary(
                        jsast::BinOp::Gt,
                        jsast::binary(jsast::BinOp::Add, used, add),
                        jsast::member(jsast::member(v, "buf"), "length"),
                    ),
                    vec![jsast::expr_stmt(call)],
                ));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeVecTryReserve
            | StandardOperation::NativeVecTryReserveExact => {
                let [v_v, used_v, add_v] = args else {
                    return Ok(None);
                };
                let exact = operation == StandardOperation::NativeVecTryReserveExact;
                let v = self.expr_of(*v_v)?;
                let used = self.expr_of(*used_v)?;
                let add = self.expr_of(*add_v)?;
                if is_zst || packed_arr {
                    let limit = if is_zst {
                        4_294_967_295.0
                    } else {
                        max_elems as f64
                    };
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&v) {
                        setup.push(jsast::expr_stmt(v));
                    }
                    return Ok(Some(Lowered::Value(
                        setup,
                        jsast::binary(
                            jsast::BinOp::Le,
                            jsast::binary(jsast::BinOp::Add, used, add),
                            jsast::num(limit),
                        ),
                    )));
                }
                if let Some(k) = ta_kind {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "nv_try_reserve_ta",
                        vec![
                            v,
                            used,
                            add,
                            jsast::num(min_cap as f64),
                            Expr::Bool(exact),
                            jsast::num(f64::from(k)),
                            jsast::num(max_elems as f64),
                        ],
                    ))))
                } else {
                    Ok(Some(Lowered::value(jsast::rt_call(
                        "nv_try_reserve_arr",
                        vec![
                            v,
                            used,
                            add,
                            jsast::num(min_cap as f64),
                            Expr::Bool(exact),
                            jsast::num(max_elems as f64),
                        ],
                    ))))
                }
            }
            StandardOperation::NativeVecShrinkTo | StandardOperation::NativeVecTryShrinkTo => {
                let [v_v, target_v] = args else {
                    return Ok(None);
                };
                let is_try = operation == StandardOperation::NativeVecTryShrinkTo;
                let v = self.expr_of(*v_v)?;
                let target = self.expr_of(*target_v)?;
                let mut setup = Vec::new();
                if is_zst || packed_arr {
                    if !jsast::is_droppable(&v) {
                        setup.push(jsast::expr_stmt(v));
                    }
                    if !jsast::is_droppable(&target) {
                        setup.push(jsast::expr_stmt(target));
                    }
                } else if let Some(k) = ta_kind {
                    setup.push(jsast::expr_stmt(jsast::rt_call(
                        "nv_shrink_ta",
                        vec![v, target, jsast::num(f64::from(k))],
                    )));
                } else {
                    setup.push(jsast::expr_stmt(jsast::rt_call(
                        "nv_shrink_arr",
                        vec![v, target, jsast::num(align as f64)],
                    )));
                }
                if is_try {
                    Ok(Some(Lowered::Value(setup, Expr::Bool(true))))
                } else {
                    Ok(Some(Lowered::Done(setup)))
                }
            }
            StandardOperation::NativeVecGrowOne => {
                let [v_v] = args else {
                    return Ok(None);
                };
                let v = self.expr_of(*v_v)?;
                if is_zst || packed_arr {
                    let limit = if is_zst {
                        4_294_967_295.0
                    } else {
                        max_elems as f64
                    };
                    return Ok(Some(Lowered::Done(vec![jsast::if_(
                        jsast::binary(jsast::BinOp::Ge, jsast::member(v, "len"), jsast::num(limit)),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "js_abort",
                            vec![jsast::string("capacity overflow")],
                        ))],
                    )])));
                }
                if let Some(k) = ta_kind {
                    Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                        format!("nv_grow_{}", TA_SUFFIXES[k as usize]),
                        vec![v, jsast::num(min_cap as f64), jsast::num(max_elems as f64)],
                    ))])))
                } else {
                    Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                        "nv_grow_one_arr",
                        vec![v, jsast::num(min_cap as f64), jsast::num(max_elems as f64)],
                    ))])))
                }
            }
            StandardOperation::NativeVecPush | StandardOperation::NativeVecPushMut => {
                let [v_v, val_v] = args else {
                    return Ok(None);
                };
                let is_mut = operation == StandardOperation::NativeVecPushMut;
                let mut setup = Vec::new();
                let v = bind_if_needed(self.expr_of(*v_v)?, &mut setup);
                let val_expr = self.expr_of(*val_v)?;
                if is_zst {
                    if !jsast::is_droppable(&val_expr) {
                        setup.push(jsast::expr_stmt(val_expr));
                    }
                    let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
                    setup.push(bl);
                    setup.push(jsast::if_(
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            l.clone(),
                            jsast::num(4_294_967_295.0),
                        ),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "js_abort",
                            vec![jsast::string("capacity overflow")],
                        ))],
                    ));
                    setup.push(jsast::assign_stmt(
                        jsast::member(v, "len"),
                        jsast::binary(
                            jsast::BinOp::UShr,
                            jsast::binary(jsast::BinOp::Add, l, jsast::num(1)),
                            jsast::num(0),
                        ),
                    ));
                    return if is_mut {
                        Ok(Some(Lowered::Value(setup, jsast::num(align as f64))))
                    } else {
                        Ok(Some(Lowered::Done(setup)))
                    };
                }
                if packed_arr {
                    if !is_mut && !tracked {
                        let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
                        setup.push(bb);
                        setup.push(jsast::expr_stmt(jsast::method_call(
                            b.clone(),
                            "push",
                            vec![val_expr],
                        )));
                        setup.push(jsast::assign_stmt(
                            jsast::member(v, "len"),
                            jsast::member(b, "length"),
                        ));
                        return Ok(Some(Lowered::Done(setup)));
                    }
                    let val = bind_if_needed(val_expr, &mut setup);
                    let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
                    setup.push(bb);
                    let (bl, l) = self.temp(jsast::member(b.clone(), "length"));
                    setup.push(bl);
                    setup.push(jsast::expr_stmt(jsast::method_call(
                        b.clone(),
                        "push",
                        vec![val.clone()],
                    )));
                    setup.push(jsast::assign_stmt(
                        jsast::member(v, "len"),
                        jsast::binary(jsast::BinOp::Add, l.clone(), jsast::num(1)),
                    ));
                    if tracked {
                        setup.push(jsast::expr_stmt(jsast::rt_call(
                            "nv_box_track_slot",
                            vec![b.clone(), l.clone(), val.clone()],
                        )));
                    }
                    if !is_mut {
                        return Ok(Some(Lowered::Done(setup)));
                    }
                    let Some(res) = op.result else {
                        return Ok(None);
                    };
                    let ret_ty = self.ty_of(res)?;
                    let ret_expr = match table.pointer_form(ret_ty) {
                        Some(PointerForm::Thin) => val,
                        Some(PointerForm::Slot) => ptr::slot(b, l),
                        _ => return Ok(None),
                    };
                    return Ok(Some(Lowered::Value(setup, ret_expr)));
                }
                let val = if is_mut || tracked {
                    bind_if_needed(val_expr, &mut setup)
                } else {
                    val_expr
                };
                let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
                setup.push(bl);
                if let Some(k) = ta_kind {
                    setup.push(jsast::if_(
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            l.clone(),
                            jsast::member(jsast::member(v.clone(), "buf"), "length"),
                        ),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            format!("nv_grow_{}", TA_SUFFIXES[k as usize]),
                            vec![
                                v.clone(),
                                jsast::num(min_cap as f64),
                                jsast::num(max_elems as f64),
                            ],
                        ))],
                    ));
                    setup.push(jsast::assign_stmt(
                        jsast::index(jsast::member(v.clone(), "buf"), l.clone()),
                        val.clone(),
                    ));
                } else {
                    setup.push(jsast::if_(
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            l.clone(),
                            jsast::member(jsast::member(v.clone(), "buf"), "length"),
                        ),
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "nv_grow_one_arr",
                            vec![
                                v.clone(),
                                jsast::num(min_cap as f64),
                                jsast::num(max_elems as f64),
                            ],
                        ))],
                    ));
                    if tracked {
                        setup.push(jsast::expr_stmt(jsast::rt_call(
                            "nv_box_track_slot",
                            vec![jsast::member(v.clone(), "buf"), l.clone(), val.clone()],
                        )));
                    } else {
                        setup.push(jsast::assign_stmt(
                            jsast::index(jsast::member(v.clone(), "buf"), l.clone()),
                            val.clone(),
                        ));
                    }
                }
                setup.push(jsast::assign_stmt(
                    jsast::member(v.clone(), "len"),
                    jsast::binary(jsast::BinOp::Add, l.clone(), jsast::num(1)),
                ));
                if !is_mut {
                    return Ok(Some(Lowered::Done(setup)));
                }
                let Some(res) = op.result else {
                    return Ok(None);
                };
                let ret_ty = self.ty_of(res)?;
                let ret_expr = match table.pointer_form(ret_ty) {
                    Some(PointerForm::Thin) => val,
                    Some(PointerForm::Slot) => ptr::slot(jsast::member(v, "buf"), l),
                    _ => return Ok(None),
                };
                Ok(Some(Lowered::Value(setup, ret_expr)))
            }
            StandardOperation::NativeVecPop => {
                let [v_v] = args else {
                    return Ok(None);
                };
                let Some(res) = op.result else {
                    return Ok(None);
                };
                let ret_ty = self.ty_of(res)?;
                let peeled_ret = table.peeled(ret_ty);
                let Some(TypeKind::Adt {
                    form: NominalForm::Enum,
                    variants,
                    ..
                }) = table.get(peeled_ret).map(|d| &d.kind)
                else {
                    return Ok(None);
                };
                let mut none_idx = None;
                let mut some_info = None;
                for (i, var) in variants.iter().enumerate() {
                    if var.fields.is_empty() {
                        none_idx = Some(i as u32);
                    } else if var.fields.len() == 1 {
                        some_info = Some((i as u32, var.fields[0].key));
                    }
                }
                let (Some(none_idx), Some((some_idx, some_key))) = (none_idx, some_info) else {
                    return Ok(None);
                };
                let Some(none_expr) =
                    value::enum_value(table, self.interns, ret_ty, none_idx, Vec::new())
                else {
                    return Ok(None);
                };

                let mut setup = Vec::new();
                let v = bind_if_needed(self.expr_of(*v_v)?, &mut setup);
                let (_, res_id) = self.temp(jsast::undefined());

                let mut some_body = Vec::new();
                let elem_expr = if packed_arr {
                    let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
                    some_body.push(bb);
                    let (be, e) = self.temp(jsast::method_call(b.clone(), "pop", Vec::new()));
                    some_body.push(be);
                    some_body.push(jsast::assign_stmt(
                        jsast::member(v.clone(), "len"),
                        jsast::member(b, "length"),
                    ));
                    e
                } else {
                    let (bl, l) = self.temp(jsast::binary(
                        jsast::BinOp::UShr,
                        jsast::binary(
                            jsast::BinOp::Sub,
                            jsast::member(v.clone(), "len"),
                            jsast::num(1),
                        ),
                        jsast::num(0),
                    ));
                    some_body.push(bl);
                    some_body.push(jsast::assign_stmt(
                        jsast::member(v.clone(), "len"),
                        l.clone(),
                    ));
                    if is_zst {
                        jsast::undefined()
                    } else {
                        let (be, e) =
                            self.temp(jsast::index(jsast::member(v.clone(), "buf"), l.clone()));
                        some_body.push(be);
                        if ta_kind.is_none() && elem_desc.layout.needs_drop {
                            some_body.push(jsast::assign_stmt(
                                jsast::index(jsast::member(v.clone(), "buf"), l),
                                jsast::undefined(),
                            ));
                        }
                        e
                    }
                };
                let some_fields = match value::key_name(self.interns, some_key) {
                    Some(k) => vec![(k, elem_expr)],
                    None => Vec::new(),
                };
                let Some(some_expr) =
                    value::enum_value(table, self.interns, ret_ty, some_idx, some_fields)
                else {
                    return Ok(None);
                };
                some_body.push(jsast::assign_stmt(res_id.clone(), some_expr));

                setup.push(jsast::if_else(
                    jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::member(v, "len"),
                        jsast::num(0),
                    ),
                    vec![jsast::assign_stmt(res_id.clone(), none_expr)],
                    some_body,
                ));
                Ok(Some(Lowered::Value(setup, res_id)))
            }
            StandardOperation::NativeVecIntoBoxedSlice => {
                let [v_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let v = bind_if_needed(self.expr_of(*v_v)?, &mut setup);
                if is_zst {
                    return Ok(Some(Lowered::Value(
                        setup,
                        jsast::object_of(vec![
                            ("buf", jsast::num(align as f64)),
                            ("off", jsast::num(0)),
                            ("len", jsast::member(v, "len")),
                        ]),
                    )));
                }
                let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
                setup.push(bl);
                let buf = if let Some(k) = ta_kind {
                    let (bb, b) = self.temp(jsast::member(v, "buf"));
                    setup.push(bb);
                    let (bo, out) = self.temp(jsast::cond(
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::member(b.clone(), "length"),
                            l.clone(),
                        ),
                        b.clone(),
                        jsast::cond(
                            jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                            jsast::rt_call("_nv_et", vec![jsast::num(f64::from(k))]),
                            jsast::method_call(b, "slice", vec![jsast::num(0), l.clone()]),
                        ),
                    ));
                    setup.push(bo);
                    out
                } else if packed_arr {
                    jsast::cond(
                        jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                        jsast::rt_call("_nv_ea", vec![jsast::num(align as f64)]),
                        jsast::member(v, "buf"),
                    )
                } else {
                    let (_, b_id) = self.temp(jsast::undefined());
                    setup.push(jsast::if_else(
                        jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                        vec![jsast::assign_stmt(
                            b_id.clone(),
                            jsast::rt_call("_nv_ea", vec![jsast::num(align as f64)]),
                        )],
                        vec![
                            jsast::assign_stmt(b_id.clone(), jsast::member(v, "buf")),
                            jsast::if_(
                                jsast::binary(
                                    jsast::BinOp::Gt,
                                    jsast::member(b_id.clone(), "length"),
                                    l.clone(),
                                ),
                                vec![jsast::assign_stmt(
                                    jsast::member(b_id.clone(), "length"),
                                    l.clone(),
                                )],
                            ),
                        ],
                    ));
                    b_id
                };
                Ok(Some(Lowered::Value(
                    setup,
                    jsast::object_of(vec![("buf", buf), ("off", jsast::num(0)), ("len", l)]),
                )))
            }
            _ => Ok(None),
        }
    }

    /// Lowers Phase 5 `core::str` fast comparison/search hooks (`patches/0021-bstr-str-ops.patch`).
    fn lower_fast_str_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let is_bstr =
            crate::lower::bstr::enabled() || self.opts.str_repr == crate::opts::StrRepr::Bytes;
        match operation {
            StandardOperation::BstrEq => {
                let [a_v, b_v] = args else {
                    return Ok(None);
                };
                let a = self.expr_of(*a_v)?;
                let b = self.expr_of(*b_v)?;
                Ok(Some(Lowered::value(jsast::binary(
                    jsast::BinOp::StrictEq,
                    a,
                    b,
                ))))
            }
            StandardOperation::BstrCmp if is_bstr => {
                let [a_v, b_v] = args else {
                    return Ok(None);
                };
                let Some(res) = op.result else {
                    return Ok(None);
                };
                let res_ty = self.ty_of(res)?;
                let a_expr = self.expr_of(*a_v)?;
                let b_expr = self.expr_of(*b_v)?;
                let mut prelude = Vec::new();
                let a = if matches!(a_expr, Expr::Ident(_) | Expr::Str(_)) {
                    a_expr
                } else {
                    let (bind, id) = self.temp(a_expr);
                    prelude.push(bind);
                    id
                };
                let b = if matches!(b_expr, Expr::Ident(_) | Expr::Str(_)) {
                    b_expr
                } else {
                    let (bind, id) = self.temp(b_expr);
                    prelude.push(bind);
                    id
                };
                let Some(cmp) = self.str_cmp_expr(res_ty, a, b) else {
                    return Ok(None);
                };
                Ok(Some(Lowered::Value(prelude, cmp)))
            }
            StandardOperation::BstrStartsWith => {
                let [h_v, n_v] = args else {
                    return Ok(None);
                };
                let h = self.expr_of(*h_v)?;
                let n = self.expr_of(*n_v)?;
                if let Expr::Str(ref lit) = n {
                    if lit.is_empty() {
                        let mut prelude = Vec::new();
                        if !jsast::is_droppable(&h) {
                            prelude.push(jsast::expr_stmt(h));
                        }
                        return Ok(Some(Lowered::Value(prelude, Expr::Bool(true))));
                    }
                    if lit.encode_utf16().count() == 1 {
                        let code = f64::from(lit.encode_utf16().next().unwrap());
                        return Ok(Some(Lowered::value(jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::method_call(h, "charCodeAt", vec![jsast::num(0)]),
                            jsast::num(code),
                        ))));
                    }
                }
                Ok(Some(Lowered::value(jsast::method_call(
                    h,
                    "startsWith",
                    vec![n],
                ))))
            }
            StandardOperation::BstrEndsWith => {
                let [h_v, n_v] = args else {
                    return Ok(None);
                };
                let h = self.expr_of(*h_v)?;
                let n = self.expr_of(*n_v)?;
                if let Expr::Str(ref lit) = n {
                    if lit.is_empty() {
                        let mut prelude = Vec::new();
                        if !jsast::is_droppable(&h) {
                            prelude.push(jsast::expr_stmt(h));
                        }
                        return Ok(Some(Lowered::Value(prelude, Expr::Bool(true))));
                    }
                    if lit.encode_utf16().count() == 1 {
                        let code = f64::from(lit.encode_utf16().next().unwrap());
                        let mut prelude = Vec::new();
                        let h = if matches!(h, Expr::Ident(_) | Expr::Str(_)) {
                            h
                        } else {
                            let (bind, id) = self.temp(h);
                            prelude.push(bind);
                            id
                        };
                        let last = jsast::binary(
                            jsast::BinOp::Sub,
                            jsast::member(h.clone(), "length"),
                            jsast::num(1),
                        );
                        return Ok(Some(Lowered::Value(
                            prelude,
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::method_call(h, "charCodeAt", vec![last]),
                                jsast::num(code),
                            ),
                        )));
                    }
                }
                Ok(Some(Lowered::value(jsast::method_call(
                    h,
                    "endsWith",
                    vec![n],
                ))))
            }
            StandardOperation::BstrContains => {
                let [h_v, n_v] = args else {
                    return Ok(None);
                };
                let h = self.expr_of(*h_v)?;
                let n = self.expr_of(*n_v)?;
                if let Expr::Str(ref lit) = n {
                    if lit.is_empty() {
                        let mut prelude = Vec::new();
                        if !jsast::is_droppable(&h) {
                            prelude.push(jsast::expr_stmt(h));
                        }
                        return Ok(Some(Lowered::Value(prelude, Expr::Bool(true))));
                    }
                }
                Ok(Some(Lowered::value(jsast::method_call(
                    h,
                    "includes",
                    vec![n],
                ))))
            }
            StandardOperation::BstrFindChar
            | StandardOperation::BstrRfindChar
            | StandardOperation::BstrFindStr
            | StandardOperation::BstrRfindStr
                if is_bstr =>
            {
                let [h_v, s_v, e_v, n_v] = args else {
                    return Ok(None);
                };
                let h = self.expr_of(*h_v)?;
                let s = self.expr_of(*s_v)?;
                let e = self.expr_of(*e_v)?;
                let n = self.expr_of(*n_v)?;
                let const_pat: Option<(Expr, usize)> = match operation {
                    StandardOperation::BstrFindChar | StandardOperation::BstrRfindChar => {
                        match &n {
                            Expr::Num(point)
                                if point.fract() == 0.0
                                    && (0.0..=0x10_FFFF as f64).contains(point) =>
                            {
                                char::from_u32(*point as u32).map(|ch| {
                                    let mut buf = [0u8; 4];
                                    let utf8 = ch.encode_utf8(&mut buf);
                                    (
                                        jsast::string(crate::lower::bstr::byte_string(utf8)),
                                        utf8.len(),
                                    )
                                })
                            }
                            _ => None,
                        }
                    }
                    StandardOperation::BstrFindStr | StandardOperation::BstrRfindStr => match &n {
                        Expr::Str(lit) => Some((n.clone(), lit.chars().count())),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((pat, plen)) = const_pat {
                    let mut prelude = Vec::new();
                    if plen == 0 {
                        for expr in [h, s, e] {
                            if !jsast::is_droppable(&expr) {
                                prelude.push(jsast::expr_stmt(expr));
                            }
                        }
                        return Ok(Some(Lowered::Value(prelude, jsast::num(4_294_967_295.0))));
                    }
                    let hold = |expr: Expr, prelude: &mut Vec<Stmt>| -> Expr {
                        if matches!(expr, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
                            expr
                        } else {
                            let (bind, id) = self.temp(expr);
                            prelude.push(bind);
                            id
                        }
                    };
                    let h = hold(h, &mut prelude);
                    let s = hold(s, &mut prelude);
                    let e = hold(e, &mut prelude);
                    let is_forward = matches!(
                        operation,
                        StandardOperation::BstrFindChar | StandardOperation::BstrFindStr
                    );
                    if is_forward {
                        let search_args = if matches!(s, Expr::Num(v) if v == 0.0) {
                            vec![pat]
                        } else {
                            vec![pat, s]
                        };
                        let (bi, i) = self.temp(jsast::method_call(h, "indexOf", search_args));
                        prelude.push(bi);
                        let end_pos =
                            jsast::binary(jsast::BinOp::Add, i.clone(), jsast::num(plen as f64));
                        let cond = jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::StrictNe, i.clone(), jsast::num(-1.0)),
                            jsast::binary(jsast::BinOp::Le, end_pos, e),
                        );
                        return Ok(Some(Lowered::Value(
                            prelude,
                            jsast::cond(cond, i, jsast::num(4_294_967_295.0)),
                        )));
                    }
                    let (blim, lim) =
                        self.temp(jsast::binary(jsast::BinOp::Sub, e, jsast::num(plen as f64)));
                    prelude.push(blim);
                    let (bi, i) = self.temp(jsast::cond(
                        jsast::binary(jsast::BinOp::Lt, lim.clone(), s.clone()),
                        jsast::num(-1.0),
                        jsast::method_call(h, "lastIndexOf", vec![pat, lim]),
                    ));
                    prelude.push(bi);
                    return Ok(Some(Lowered::Value(
                        prelude,
                        jsast::cond(
                            jsast::binary(jsast::BinOp::Ge, i.clone(), s),
                            i,
                            jsast::num(4_294_967_295.0),
                        ),
                    )));
                }
                let helper = match operation {
                    StandardOperation::BstrFindChar => "bstr_find_char",
                    StandardOperation::BstrRfindChar => "bstr_rfind_char",
                    StandardOperation::BstrFindStr => "bstr_find_str",
                    StandardOperation::BstrRfindStr => "bstr_rfind_str",
                    _ => unreachable!(),
                };
                Ok(Some(Lowered::value(jsast::rt_call(
                    helper,
                    vec![h, s, e, n],
                ))))
            }
            _ => Ok(None),
        }
    }

    /// Extracts `(base, off, len)` for a `VarSlice` byte-view while evaluating non-atomic
    /// expressions at most once.
    fn extract_byte_view_var_slice(
        &self,
        bv: super::ByteView,
        v: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<(Expr, Expr, Expr)> {
        let mut hold = |expr: Expr| -> Expr {
            if matches!(expr, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
                expr
            } else {
                let (bind, id) = self.temp(expr);
                prelude.push(bind);
                id
            }
        };
        if bv.base_str.is_none() && bv.base_cell.is_none() {
            let expr = self.expr_of(v)?;
            if let Expr::Object(ref entries) = expr {
                let field = |k: &str| {
                    entries
                        .iter()
                        .find(|(ek, _)| ek == k)
                        .map(|(_, ev)| ev.clone())
                };
                if let (Some(b), Some(o), Some(l)) = (field("buf"), field("off"), field("len")) {
                    let b = hold(b);
                    let o = hold(o);
                    let l = hold(l);
                    return Ok((b, o, l));
                }
            }
            let held = hold(expr);
            let b = hold(jsast::member(held.clone(), "buf"));
            let o = hold(jsast::member(held.clone(), "off"));
            let l = hold(jsast::member(held, "len"));
            return Ok((b, o, l));
        }
        let base = hold(self.byte_view_base_expr(bv, v)?);
        let off = hold(self.byte_view_off_expr(v)?);
        let len = hold(self.byte_view_len_expr(v)?);
        Ok((base, off, len))
    }

    /// Extracts constant raw bytes when `v` is defined purely by a constant `&[u8]` / `&[u8; N]`
    /// / `&str` literal without side effects.
    fn const_byte_slice(&self, mut v: ValueId) -> Option<Vec<u8>> {
        use rcgjs_object::jir::semantic::{CastKind, ConstTree};
        for _ in 0..8 {
            let def = self.def_of(v)?;
            match &def.kind {
                OpKind::Copy(src)
                | OpKind::Move(src)
                | OpKind::Cast {
                    kind:
                        CastKind::PointerCoercion
                        | CastKind::PtrToPtr
                        | CastKind::Transmute
                        | CastKind::Subtype,
                    v: src,
                    ..
                } => v = *src,
                OpKind::BorrowView {
                    root: Root::Value(src),
                    path,
                    ..
                } if path.is_empty() => v = *src,
                OpKind::Const(tree) => {
                    fn decode(
                        interns: &rcgjs_object::jir::interns::Interns,
                        tree: &ConstTree,
                    ) -> Option<Vec<u8>> {
                        match tree {
                            ConstTree::Reference(inner) => decode(interns, inner),
                            ConstTree::Slice(items) | ConstTree::Aggregate(items) => {
                                let mut out = Vec::with_capacity(items.len());
                                for item in items {
                                    let ConstTree::Scalar { bits, bytes: 1 } = item else {
                                        return None;
                                    };
                                    out.push(u8::try_from(*bits).ok()?);
                                }
                                Some(out)
                            }
                            ConstTree::SliceAddress { length: 0, .. } => Some(Vec::new()),
                            ConstTree::Str(id) => Some(interns.str(*id)?.as_bytes().to_vec()),
                            _ => None,
                        }
                    }
                    return decode(self.interns, tree);
                }
                _ => return None,
            }
        }
        None
    }

    /// Phase 2C: Lowers authenticated `core::slice::<impl [u8]>::{starts_with,ends_with}` over
    /// `ByteStr` (`js-str=bytes`) views directly to JS `charCodeAt` / `startsWith` / `endsWith`.
    fn lower_bstr_slice_search(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let Callee::Direct { item, .. } = callee else {
            return Ok(None);
        };
        let Some(ascii) = self.ascii() else {
            return Ok(None);
        };
        let is_starts = ascii.is_slice_starts_with(*item);
        let is_ends = !is_starts && ascii.is_slice_ends_with(*item);
        if !is_starts && !is_ends {
            return Ok(None);
        }
        let [h_v, n_v] = args else {
            return Ok(None);
        };
        let Some(res) = op.result else {
            return Ok(None);
        };
        if !matches!(
            value::kind(self.table, self.table.peeled(self.ty_of(res)?)),
            Some(rcgjs_object::jir::types::TypeKind::Bool)
        ) {
            return Ok(None);
        }
        let Some(hbv) = self.byte_view(*h_v) else {
            return Ok(None);
        };
        let nbv = self.byte_view(*n_v);
        self.lower_bstr_slice_search_views(is_starts, *h_v, hbv, *n_v, nbv)
    }

    fn lower_bstr_slice_search_views(
        &self,
        is_starts: bool,
        h_v: ValueId,
        hbv: super::ByteView,
        n_v: ValueId,
        nbv: Option<super::ByteView>,
    ) -> Res<Option<Lowered>> {
        if !matches!(
            hbv.mode,
            super::ByteViewMode::FullSlice | super::ByteViewMode::VarSlice
        ) {
            return Ok(None);
        }
        let mut prelude = Vec::new();
        let hold = |expr: Expr, prelude: &mut Vec<Stmt>| -> Expr {
            if matches!(expr, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
                expr
            } else {
                let (bind, id) = self.temp(expr);
                prelude.push(bind);
                id
            }
        };

        // Case 1: Constant `&[u8]` needle literal (`b"..."`, `&[b';']`, etc.).
        if let Some(bytes) = self.const_byte_slice(n_v) {
            let nlen = bytes.len();
            if nlen == 0 {
                let h_expr = self.expr_of(h_v)?;
                if !jsast::is_droppable(&h_expr) {
                    prelude.push(jsast::expr_stmt(h_expr));
                }
                return Ok(Some(Lowered::Value(prelude, Expr::Bool(true))));
            }
            let pat_str: String = bytes.iter().copied().map(char::from).collect();
            let pat = jsast::string(pat_str);
            let expr = match hbv.mode {
                super::ByteViewMode::FullSlice => {
                    let h = hold(self.expr_of(h_v)?, &mut prelude);
                    if nlen == 1 {
                        let code = f64::from(bytes[0]);
                        let idx = if is_starts {
                            jsast::num(0)
                        } else {
                            jsast::binary(
                                jsast::BinOp::Sub,
                                jsast::member(h.clone(), "length"),
                                jsast::num(1),
                            )
                        };
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::method_call(h, "charCodeAt", vec![idx]),
                            jsast::num(code),
                        )
                    } else if is_starts {
                        jsast::method_call(h, "startsWith", vec![pat])
                    } else {
                        jsast::method_call(h, "endsWith", vec![pat])
                    }
                }
                super::ByteViewMode::VarSlice => {
                    let (base, off, len) =
                        self.extract_byte_view_var_slice(hbv, h_v, &mut prelude)?;
                    if nlen == 1 {
                        let code = f64::from(bytes[0]);
                        let idx = if is_starts {
                            off
                        } else {
                            jsast::binary(
                                jsast::BinOp::Sub,
                                jsast::binary(jsast::BinOp::Add, off, len.clone()),
                                jsast::num(1),
                            )
                        };
                        jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::Gt, len, jsast::num(0)),
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::method_call(base, "charCodeAt", vec![idx]),
                                jsast::num(code),
                            ),
                        )
                    } else if is_starts {
                        jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::Ge, len, jsast::num(nlen as f64)),
                            jsast::method_call(base, "startsWith", vec![pat, off]),
                        )
                    } else {
                        let end = jsast::binary(jsast::BinOp::Add, off, len.clone());
                        jsast::binary(
                            jsast::BinOp::And,
                            jsast::binary(jsast::BinOp::Ge, len, jsast::num(nlen as f64)),
                            jsast::method_call(base, "endsWith", vec![pat, end]),
                        )
                    }
                }
                super::ByteViewMode::ThinPtr => unreachable!(),
            };
            return Ok(Some(Lowered::Value(prelude, expr)));
        }

        // Case 2: Dynamic `ByteView` needle (`FullSlice` or `VarSlice`).
        let Some(nbv) = nbv else {
            return Ok(None);
        };
        if !matches!(
            nbv.mode,
            super::ByteViewMode::FullSlice | super::ByteViewMode::VarSlice
        ) {
            return Ok(None);
        }
        let expr = match (hbv.mode, nbv.mode) {
            (super::ByteViewMode::FullSlice, super::ByteViewMode::FullSlice) => {
                let h = hold(self.expr_of(h_v)?, &mut prelude);
                let n = hold(self.expr_of(n_v)?, &mut prelude);
                let method = if is_starts { "startsWith" } else { "endsWith" };
                jsast::method_call(h, method, vec![n])
            }
            (super::ByteViewMode::FullSlice, super::ByteViewMode::VarSlice) => {
                let h = hold(self.expr_of(h_v)?, &mut prelude);
                let (nb, no, nl) = self.extract_byte_view_var_slice(nbv, n_v, &mut prelude)?;
                let nsub = jsast::method_call(
                    nb,
                    "slice",
                    vec![no.clone(), jsast::binary(jsast::BinOp::Add, no, nl.clone())],
                );
                let method = if is_starts { "startsWith" } else { "endsWith" };
                jsast::binary(
                    jsast::BinOp::And,
                    jsast::binary(jsast::BinOp::Ge, jsast::member(h.clone(), "length"), nl),
                    jsast::method_call(h, method, vec![nsub]),
                )
            }
            (super::ByteViewMode::VarSlice, super::ByteViewMode::FullSlice) => {
                let (hb, ho, hl) = self.extract_byte_view_var_slice(hbv, h_v, &mut prelude)?;
                let n = hold(self.expr_of(n_v)?, &mut prelude);
                let check = if is_starts {
                    jsast::method_call(hb, "startsWith", vec![n.clone(), ho])
                } else {
                    jsast::method_call(
                        hb,
                        "endsWith",
                        vec![n.clone(), jsast::binary(jsast::BinOp::Add, ho, hl.clone())],
                    )
                };
                jsast::binary(
                    jsast::BinOp::And,
                    jsast::binary(jsast::BinOp::Ge, hl, jsast::member(n, "length")),
                    check,
                )
            }
            (super::ByteViewMode::VarSlice, super::ByteViewMode::VarSlice) => {
                let (hb, ho, hl) = self.extract_byte_view_var_slice(hbv, h_v, &mut prelude)?;
                let (nb, no, nl) = self.extract_byte_view_var_slice(nbv, n_v, &mut prelude)?;
                let nsub = jsast::method_call(
                    nb,
                    "slice",
                    vec![no.clone(), jsast::binary(jsast::BinOp::Add, no, nl.clone())],
                );
                let check = if is_starts {
                    jsast::method_call(hb, "startsWith", vec![nsub, ho])
                } else {
                    jsast::method_call(
                        hb,
                        "endsWith",
                        vec![nsub, jsast::binary(jsast::BinOp::Add, ho, hl.clone())],
                    )
                };
                jsast::binary(
                    jsast::BinOp::And,
                    jsast::binary(jsast::BinOp::Ge, hl, nl),
                    check,
                )
            }
            _ => unreachable!(),
        };
        Ok(Some(Lowered::Value(prelude, expr)))
    }

    /// Phase 2C: Lowers authenticated `core::slice::<impl [T]>::reverse` and
    /// `core::slice::reverse::revswap` to a tight inline two-pointer swap loop when element moves
    /// cannot observe `_boxes` tracking or raw byte views.
    fn lower_slice_reverse(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        use rcgjs_object::jir::types::TypeKind;
        let Callee::Direct { item, link_name } = callee else {
            return Ok(None);
        };
        let (is_reverse, is_revswap) = if let Some(ascii) = self.ascii() {
            (ascii.is_slice_reverse(*item), ascii.is_slice_revswap(*item))
        } else {
            let core_ok = self.items.get(item).is_some_and(|e| e.is_core);
            let name = self.interns.str(*link_name).unwrap_or("");
            (
                core_ok && is_core_slice_reverse_link_name(name),
                core_ok && is_core_slice_revswap_link_name(name),
            )
        };
        if !is_reverse && !is_revswap {
            return Ok(None);
        }
        if op
            .result
            .is_some_and(|r| self.ty_of(r).ok().is_none_or(|t| !self.table.is_zst(t)))
        {
            return Ok(None);
        }
        if let Some(lowered) = self.lower_soa_slice_reverse(is_reverse, args)? {
            return Ok(Some(lowered));
        }

        let table = self.table;
        let check_elem = |elem_ty: rcgjs_object::jir::types::TypeId| -> Option<bool> {
            let desc = table.get(elem_ty)?;
            if !desc.layout.sized {
                return None;
            }
            if desc.layout.zst {
                return Some(true);
            }
            let peeled = table.peeled(elem_ty);
            if self.byte_views.raw_byte_derefs
                || !self.byte_views.elide_scale_scalar
                || !value::is_slot_pointee(table, peeled)
                || matches!(value::kind(table, peeled), Some(TypeKind::Array { .. }))
            {
                return None;
            }
            if table.is_indirect(peeled) && !self.can_inline_copy_one(elem_ty) {
                return None;
            }
            Some(false)
        };

        if is_reverse {
            let [slice_v] = args else {
                return Ok(None);
            };
            let slice_ty = table.peeled(self.ty_of(*slice_v)?);
            let Some(TypeKind::Reference {
                pointee,
                mutable: true,
            }) = value::kind(table, slice_ty)
            else {
                return Ok(None);
            };
            let Some(TypeKind::Slice(elem_ty)) = value::kind(table, *pointee) else {
                return Ok(None);
            };
            let Some(is_zst) = check_elem(*elem_ty) else {
                return Ok(None);
            };
            let slice_expr = self.expr_of(*slice_v)?;
            if is_zst {
                let mut setup = Vec::new();
                if !jsast::is_droppable(&slice_expr) {
                    setup.push(jsast::expr_stmt(slice_expr));
                }
                return Ok(Some(Lowered::Done(setup)));
            }

            let mut setup = Vec::new();
            let (buf_raw, off_raw, len_raw) = match slice_expr {
                Expr::Object(ref entries) if entries.len() == 3 => {
                    let find = |k: &str| {
                        entries
                            .iter()
                            .find(|(ek, _)| ek == k)
                            .map(|(_, ev)| ev.clone())
                    };
                    if let (Some(b), Some(o), Some(l)) = (find("buf"), find("off"), find("len")) {
                        (b, o, l)
                    } else {
                        let (bs, s) = self.temp(slice_expr);
                        setup.push(bs);
                        (
                            ptr::slot_buf(s.clone()),
                            ptr::slot_off(s.clone()),
                            ptr::slot_len(s),
                        )
                    }
                }
                Expr::Ident(_) => (
                    ptr::slot_buf(slice_expr.clone()),
                    ptr::slot_off(slice_expr.clone()),
                    ptr::slot_len(slice_expr),
                ),
                other => {
                    let (bs, s) = self.temp(other);
                    setup.push(bs);
                    (
                        ptr::slot_buf(s.clone()),
                        ptr::slot_off(s.clone()),
                        ptr::slot_len(s),
                    )
                }
            };
            let b = if matches!(buf_raw, Expr::Ident(_)) {
                buf_raw
            } else {
                let (bb, id) = self.temp(buf_raw);
                setup.push(bb);
                id
            };
            let off_is_zero = matches!(off_raw, Expr::Num(v) if v == 0.0);
            let (bl, l) = self.temp(off_raw);
            setup.push(bl);
            let r_init = if off_is_zero {
                jsast::binary(jsast::BinOp::Sub, len_raw, jsast::num(1))
            } else {
                jsast::binary(
                    jsast::BinOp::Sub,
                    jsast::binary(jsast::BinOp::Add, l.clone(), len_raw),
                    jsast::num(1),
                )
            };
            let (br, r) = self.temp(r_init);
            setup.push(br);
            let (_, tmp) = self.temp(jsast::undefined());
            setup.push(jsast::while_(
                jsast::binary(jsast::BinOp::Lt, l.clone(), r.clone()),
                vec![
                    jsast::assign_stmt(tmp.clone(), jsast::index(b.clone(), l.clone())),
                    jsast::assign_stmt(
                        jsast::index(b.clone(), l.clone()),
                        jsast::index(b.clone(), r.clone()),
                    ),
                    jsast::assign_stmt(jsast::index(b, r.clone()), tmp),
                    jsast::expr_stmt(jsast::compound_assign(jsast::BinOp::Add, l, jsast::num(1))),
                    jsast::expr_stmt(jsast::compound_assign(jsast::BinOp::Sub, r, jsast::num(1))),
                ],
            ));
            return Ok(Some(Lowered::Done(setup)));
        }

        // `core::slice::reverse::revswap::<T>(a: *mut T, b: *mut T, n: usize)`
        let [a_v, b_v, n_v] = args else {
            return Ok(None);
        };
        let (
            Some(TypeKind::RawPointer {
                pointee: elem_a,
                mutable: true,
            }),
            Some(TypeKind::RawPointer {
                pointee: elem_b,
                mutable: true,
            }),
        ) = (
            value::kind(table, table.peeled(self.ty_of(*a_v)?)),
            value::kind(table, table.peeled(self.ty_of(*b_v)?)),
        )
        else {
            return Ok(None);
        };
        if table.peeled(*elem_a) != table.peeled(*elem_b) {
            return Ok(None);
        }
        let Some(is_zst) = check_elem(*elem_a) else {
            return Ok(None);
        };
        let a_expr = self.expr_of(*a_v)?;
        let b_expr = self.expr_of(*b_v)?;
        let n_expr = self.expr_of(*n_v)?;
        if is_zst {
            let mut setup = Vec::new();
            for expr in [a_expr, b_expr, n_expr] {
                if !jsast::is_droppable(&expr) {
                    setup.push(jsast::expr_stmt(expr));
                }
            }
            return Ok(Some(Lowered::Done(setup)));
        }

        let mut setup = Vec::new();
        let mut unpack_slot = |slot: Expr| -> (Expr, Expr) {
            let (buf_raw, off_raw) = match slot {
                Expr::Object(ref entries) if entries.len() == 2 => {
                    let find = |k: &str| {
                        entries
                            .iter()
                            .find(|(ek, _)| ek == k)
                            .map(|(_, ev)| ev.clone())
                    };
                    if let (Some(b), Some(o)) = (find("buf"), find("off")) {
                        (b, o)
                    } else {
                        let (bs, s) = self.temp(slot);
                        setup.push(bs);
                        (ptr::slot_buf(s.clone()), ptr::slot_off(s))
                    }
                }
                Expr::Ident(_) => (ptr::slot_buf(slot.clone()), ptr::slot_off(slot)),
                other => {
                    let (bs, s) = self.temp(other);
                    setup.push(bs);
                    (ptr::slot_buf(s.clone()), ptr::slot_off(s))
                }
            };
            let b = if matches!(buf_raw, Expr::Ident(_)) {
                buf_raw
            } else {
                let (bb, id) = self.temp(buf_raw);
                setup.push(bb);
                id
            };
            (b, off_raw)
        };
        let (ab, ao) = unpack_slot(a_expr);
        let (bb, bo) = unpack_slot(b_expr);
        let n = if matches!(n_expr, Expr::Ident(_) | Expr::Num(_)) {
            n_expr
        } else {
            let (bn, id) = self.temp(n_expr);
            setup.push(bn);
            id
        };
        let (bl, l) = self.temp(ao);
        setup.push(bl);
        let (bend, end) = self.temp(jsast::binary(jsast::BinOp::Add, l.clone(), n.clone()));
        setup.push(bend);
        let (br, r) = self.temp(jsast::binary(
            jsast::BinOp::Sub,
            jsast::binary(jsast::BinOp::Add, bo, n),
            jsast::num(1),
        ));
        setup.push(br);
        let (_, tmp) = self.temp(jsast::undefined());
        setup.push(jsast::while_(
            jsast::binary(jsast::BinOp::Lt, l.clone(), end),
            vec![
                jsast::assign_stmt(tmp.clone(), jsast::index(ab.clone(), l.clone())),
                jsast::assign_stmt(
                    jsast::index(ab.clone(), l.clone()),
                    jsast::index(bb.clone(), r.clone()),
                ),
                jsast::assign_stmt(jsast::index(bb, r.clone()), tmp),
                jsast::expr_stmt(jsast::compound_assign(jsast::BinOp::Add, l, jsast::num(1))),
                jsast::expr_stmt(jsast::compound_assign(jsast::BinOp::Sub, r, jsast::num(1))),
            ],
        ));
        Ok(Some(Lowered::Done(setup)))
    }

    /// Three-way `Ordering` result expression `a < b ? Less : a === b ? Equal : Greater`.
    fn str_cmp_expr(
        &self,
        ordering_ty: rcgjs_object::jir::types::TypeId,
        a: Expr,
        b: Expr,
    ) -> Option<Expr> {
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
        let drop_noop_nstr = self.heap().native_string_enabled
            && crate::analysis::heap::drop_targets_native_string(self.func, self.table, root, path);
        let drop_noop_nvec = self.heap().native_vec_enabled
            && !self.heap().non_rc_raw_alloc_reachable
            && crate::analysis::heap::drop_targets_noop_native_vec(
                self.func,
                self.table,
                self.interns,
                root,
                path,
                self.heap().native_string_enabled,
                true,
            );
        if drop_noop_nstr || drop_noop_nvec {
            let mut stmts = Vec::new();
            if path.is_empty() {
                if let Root::Value(v) = root {
                    if self.is_inline(*v) {
                        let expr = self.expr_of(*v)?;
                        if !jsast::is_droppable(&expr) {
                            stmts.push(jsast::expr_stmt(expr));
                        }
                    }
                }
            }
            return Ok(Lowered::Done(stmts));
        }
        match glue {
            // Empty glue is not emitted as an item, so naming it would dangle. Evaluate any
            // inlined non-droppable root/index operand as defense-in-depth so effects never drop.
            DropGlue::None => {
                let mut stmts = Vec::new();
                if let Root::Value(v) = root {
                    if self.is_inline(*v) {
                        let expr = self.expr_of(*v)?;
                        if !jsast::is_droppable(&expr) {
                            stmts.push(jsast::expr_stmt(expr));
                        }
                    }
                }
                for step in path {
                    if let Proj::Index(idx) = step {
                        if self.is_inline(*idx) {
                            let expr = self.expr_of(*idx)?;
                            if !jsast::is_droppable(&expr) {
                                stmts.push(jsast::expr_stmt(expr));
                            }
                        }
                    }
                }
                Ok(Lowered::Done(stmts))
            }
            DropGlue::Direct { item, link_name } => {
                let argument = self.borrow_view(root, path, true)?;
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
                let value = self.borrow_view(root, path, true)?;
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

fn has_16_hex_hash(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_core_slice_reverse_link_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("core$slice$impl_") else {
        return false;
    };
    let Some((digits, tail)) = rest.split_once("$reverse$h") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && has_16_hex_hash(tail)
}

fn is_core_slice_revswap_link_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("core$slice$reverse$revswap$h") else {
        return false;
    };
    has_16_hex_hash(rest)
}

/// Lightweight uninitialized container shell for `Box::<T>::new_uninit()` (`TypedAlloc` with
/// `zeroed == 0`). Provides arrays for fixed-array fields (so `BTreeMap` `LeafNode` / `vec![...]`
/// element writes have a target container) without deep zero-initializing element structs.
fn uninit_shell(
    table: &rcgjs_object::jir::types::TypeTable,
    interns: &rcgjs_object::jir::interns::Interns,
    ty: rcgjs_object::jir::types::TypeId,
    depth: u32,
    heap: &crate::analysis::heap::HeapPlan,
) -> Option<Expr> {
    use rcgjs_object::jir::types::{NominalForm, TypeKind};
    if depth > 4 {
        return None;
    }
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.layout.zst {
        return Some(jsast::undefined());
    }
    if heap.native_string_enabled
        && desc.meta.candidate == rcgjs_object::jir::types::RepresentationCandidate::String
    {
        return Some(jsast::object_of(vec![
            ("s", jsast::string("")),
            ("cap", jsast::num(0)),
        ]));
    }
    if heap.native_vec_enabled && crate::analysis::heap::is_direct_global_vec_ty(table, peeled) {
        if let Some(plan) = crate::plan::soa_vec::current_plan() {
            if let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, peeled) {
                if let Some(layout) = plan.get(table, elem_ty) {
                    return Some(super::soa::empty_vec_expr(layout));
                }
            }
        }
        if let Some(empty) = value::empty_native_vec_expr_for_heap(table, peeled, heap) {
            return Some(empty);
        }
    }
    match &desc.kind {
        TypeKind::Array { element, length } => {
            if depth == 0 {
                if let Some(k) = table
                    .get(*element)
                    .and_then(crate::lower::semantic_ops::typed_array_kind)
                {
                    return Some(jsast::new(
                        jsast::id(super::ast_util::TA_CTORS[k as usize]),
                        vec![jsast::num(*length as f64)],
                    ));
                }
            }
            if *length <= 4 {
                Some(jsast::array(
                    (0..*length).map(|_| jsast::undefined()).collect(),
                ))
            } else {
                Some(jsast::new(
                    jsast::id("Array"),
                    vec![jsast::num(*length as f64)],
                ))
            }
        }
        TypeKind::Tuple(elems) => {
            if elems.len() <= 4 {
                Some(jsast::array(
                    elems
                        .iter()
                        .map(|e| uninit_field_shell(table, interns, *e, depth + 1, heap))
                        .collect(),
                ))
            } else {
                Some(jsast::new(
                    jsast::id("Array"),
                    vec![jsast::num(elems.len() as f64)],
                ))
            }
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let variant = variants.first()?;
            let mut entries = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let key = value::key_name(interns, field.key)?;
                entries.push((
                    key,
                    uninit_field_shell(table, interns, field.ty, depth + 1, heap),
                ));
            }
            Some(jsast::object(entries))
        }
        TypeKind::Adt {
            form: NominalForm::Enum | NominalForm::Union,
            ..
        } if desc.js.is_indirect => Some(jsast::object(Vec::new())),
        _ => None,
    }
}

fn uninit_field_shell(
    table: &rcgjs_object::jir::types::TypeTable,
    interns: &rcgjs_object::jir::interns::Interns,
    ty: rcgjs_object::jir::types::TypeId,
    depth: u32,
    heap: &crate::analysis::heap::HeapPlan,
) -> Expr {
    use rcgjs_object::jir::types::{NominalForm, TypeKind};
    let peeled = table.peeled(ty);
    match table.get(peeled).map(|d| &d.kind) {
        Some(
            TypeKind::Array { .. }
            | TypeKind::Tuple(_)
            | TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            },
        ) => uninit_shell(table, interns, peeled, depth, heap).unwrap_or_else(jsast::undefined),
        _ => jsast::undefined(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rcgjs_object::item::{Availability, ItemSummary, JsName, SemanticKey};
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::origin::OriginId;
    use rcgjs_object::jir::repr::{
        Carrier, Func, Op, OpKind, RBlock, RBlockId, RTerm, ValueDef, ValueId,
    };
    use rcgjs_object::jir::semantic::{
        AbiSignature, Body, BodyKind, BodySemantic, CallSemantic, Callee, ConstTree, DefinitionId,
        ItemId, Signature, StandardOperation,
    };
    use rcgjs_object::jir::types::{
        AllocatorIdentity, EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts,
        LayoutFacts, NominalForm, PointerForm, RepresentationCandidate, TypeDescriptor, TypeId,
        TypeKind, TypeMeta, VariantDescriptor,
    };
    use rcgjs_object::jir::ItemClass;
    use rcgjs_object::jsast;
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};

    use crate::analysis::byte_views::ByteViewPlan;
    use crate::analysis::heap::HeapPlan;
    use crate::lower::js::{host, unsize};
    use crate::lower::{LowerInput, NestedClone};
    use crate::opts::{Opts, Profile};

    fn mk_desc(id: u128, kind: TypeKind, size: u64, is_indirect: bool) -> TypeDescriptor {
        let tid = TypeId(id);
        let int = match kind {
            TypeKind::Integer { signed, bits } => Some(IntFacts {
                signed,
                bits,
                repr: if bits > 32 {
                    IntRepr::BigInt
                } else {
                    IntRepr::Number
                },
            }),
            _ => None,
        };
        TypeDescriptor {
            id: tid,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                align: Some(size.max(1)),
                sized: true,
                zst: size == 0,
                ..LayoutFacts::default()
            },
            js: JsFacts {
                int,
                is_indirect,
                needs_clone: is_indirect,
                peeled: tid,
                transparent: None,
                crosses_transparent_union: false,
                enum_repr: None,
                is_primitive_enum: false,
                unsized_tail: None,
                byte_element: false,
                pointer_form: None,
                fields: Vec::new(),
            },
            meta: TypeMeta::default(),
        }
    }

    fn lower_func(interns: &Interns, func: &Func, heap: &HeapPlan) -> String {
        let body = Body {
            item: func.item,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: func.sig.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
        };
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: "test_fn".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::default(),
        };
        let js_name = JsName::new("test_fn".to_owned());
        let opts = Opts {
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let items = host::ItemIndex::new();
        let vtables = unsize::VtableIndex::new();
        let input = LowerInput {
            name: &js_name,
            summary: &summary,
            body: &body,
            origins: None,
            interns,
            table: &interns.types,
            opts: &opts,
            items: &items,
            drop_glue: host::EMPTY_DROP_GLUE,
            vtables: &vtables,
            spills: &[],
            abi: None,
            final_lowering: false,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            bigints: None,
            byte_views: ByteViewPlan {
                raw_byte_derefs: false,
                elide_scale_scalar: true,
            },
            heap: Some(heap),
            nested_clone: NestedClone::default(),
        };
        jsast::stmt_to_string(&crate::lower::lower_func(&input, func).expect("lower").decl)
    }

    #[test]
    fn phase1b_lowers_packed_vec_monomorphic_ta_and_fast_vec_resize() {
        let mut interns = Interns::new();
        let sym = interns.intern_str("std_hook");
        let resize_sym = interns.intern_str("alloc$vec$Vec$resize$h1");
        let len_key = interns.intern_str("len");
        let x_key = interns.intern_str("x");
        let none_name = interns.intern_str("None");
        let some_name = interns.intern_str("Some");
        let f0_name = interns.intern_str("_0");

        let unit = TypeId(1);
        let u8_ty = TypeId(2);
        let usize_ty = TypeId(3);
        let obj_ty = TypeId(4);
        interns
            .types
            .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
        interns.types.insert(mk_desc(
            2,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
            false,
        ));
        interns.types.insert(mk_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));
        interns.types.insert(mk_desc(
            4,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 5,
                    local_hash: 1,
                },
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: x_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(x_key),
                        ty: usize_ty,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            8,
            true,
        ));

        let mk_vec = |interns: &mut Interns, vid: u128, rid: u128, elem: TypeId| {
            let mut vd = mk_desc(
                vid,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 2,
                        local_hash: 1,
                    },
                    form: NominalForm::Struct,
                    args: vec![elem],
                    variants: vec![VariantDescriptor {
                        name: len_key,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(len_key),
                            ty: usize_ty,
                            offset: 0,
                        }],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                12,
                true,
            );
            vd.meta.candidate = RepresentationCandidate::Vec;
            vd.meta.allocator = AllocatorIdentity::Global;
            interns.types.insert(vd);
            let mut rd = mk_desc(
                rid,
                TypeKind::Reference {
                    pointee: TypeId(vid),
                    mutable: true,
                },
                4,
                false,
            );
            rd.js.pointer_form = Some(PointerForm::Thin);
            interns.types.insert(rd);
            (TypeId(vid), TypeId(rid))
        };

        let (vec_u8, ref_vec_u8) = mk_vec(&mut interns, 10, 11, u8_ty);
        let (vec_obj, ref_vec_obj) = mk_vec(&mut interns, 12, 13, obj_ty);

        let opt_obj = TypeId(14);
        let mut opt_desc = mk_desc(
            14,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 7,
                },
                form: NominalForm::Enum,
                args: vec![obj_ty],
                variants: vec![
                    VariantDescriptor {
                        name: none_name,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: some_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f0_name),
                            ty: obj_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            8,
            false,
        );
        opt_desc.js.enum_repr = Some(EnumRepr::NullNiche {
            none_variant: 0,
            some_variant: 1,
        });
        interns.types.insert(opt_desc);

        let mut heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: false,
            native_vec_enabled: true,
            raw_alloc_reachable: false,
            non_rc_raw_alloc_reachable: false,
            any_boxed: false,
            boxed_defs: BTreeSet::new(),
            boxed_non_adt_keys: BTreeSet::new(),
            all_non_adt_boxed: false,
            all_adts_boxed: false,
            smi_u32_fields: BTreeSet::new(),
            packed_arr_vec_types: BTreeSet::from([vec_obj]),
        };

        let std_call = |res, op, args, name| Op {
            result: res,
            origin: OriginId(0),
            kind: OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 99,
                        },
                        instantiation: 1,
                    },
                    link_name: name,
                },
                function: None,
                args,
                track_caller: None,
                semantic: CallSemantic::Standard(op),
                host: None,
            },
        };

        // 1. Vec<u8>: with_capacity(n), zeroed(n), push(b), resize(n, b)
        let f_u8 = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ref_vec_u8, usize_ty, u8_ty],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ref_vec_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: vec_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: vec_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
            ],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1), ValueId(2)],
                ops: vec![
                    std_call(
                        Some(ValueId(3)),
                        StandardOperation::NativeVecWithCapacity,
                        vec![ValueId(1)],
                        sym,
                    ),
                    std_call(
                        Some(ValueId(4)),
                        StandardOperation::NativeVecFromElemZeroed,
                        vec![ValueId(1)],
                        sym,
                    ),
                    std_call(
                        None,
                        StandardOperation::NativeVecPush,
                        vec![ValueId(0), ValueId(2)],
                        sym,
                    ),
                    std_call(
                        None,
                        StandardOperation::VecResize,
                        vec![ValueId(0), ValueId(1), ValueId(2)],
                        resize_sym,
                    ),
                ],
                term: RTerm::Return(None),
            }],
        };
        let js_u8 = lower_func(&interns, &f_u8, &heap);
        assert!(js_u8.contains("__rt.nv_with_cap_u8("), "{js_u8}");
        assert!(js_u8.contains("__rt.nv_zeroed_u8("), "{js_u8}");
        assert!(js_u8.contains("__rt.nv_grow_u8("), "{js_u8}");
        assert!(js_u8.contains("__rt.nv_reserve_u8("), "{js_u8}");
        assert!(js_u8.contains(".buf.fill("), "{js_u8}");
        assert!(!js_u8.contains("nv_grow_one_ta"), "{js_u8}");
        assert!(!js_u8.contains("nv_with_cap_ta"), "{js_u8}");
        assert!(!js_u8.contains("nv_zeroed_ta"), "{js_u8}");
        assert!(!js_u8.contains("alloc$vec$Vec$resize$h1"), "{js_u8}");

        // 2. Packed Vec<Obj>: new, with_capacity(16), push, pop, reserve, shrink_to
        let f_obj = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 2,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ref_vec_obj, obj_ty, usize_ty],
                result: opt_obj,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ref_vec_obj,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: obj_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: vec_obj,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: vec_obj,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: opt_obj,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
            ],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1), ValueId(2)],
                ops: vec![
                    Op {
                        result: Some(ValueId(3)),
                        origin: OriginId(0),
                        kind: OpKind::Const(ConstTree::Scalar { bits: 16, bytes: 4 }),
                    },
                    std_call(
                        Some(ValueId(4)),
                        StandardOperation::NativeVecNew,
                        vec![],
                        sym,
                    ),
                    std_call(
                        Some(ValueId(5)),
                        StandardOperation::NativeVecWithCapacity,
                        vec![ValueId(3)],
                        sym,
                    ),
                    std_call(
                        None,
                        StandardOperation::NativeVecReserve,
                        vec![ValueId(0), ValueId(2), ValueId(3)],
                        sym,
                    ),
                    std_call(
                        None,
                        StandardOperation::NativeVecShrinkTo,
                        vec![ValueId(0), ValueId(2)],
                        sym,
                    ),
                    std_call(
                        None,
                        StandardOperation::NativeVecPush,
                        vec![ValueId(0), ValueId(1)],
                        sym,
                    ),
                    std_call(
                        Some(ValueId(6)),
                        StandardOperation::NativeVecPop,
                        vec![ValueId(0)],
                        sym,
                    ),
                ],
                term: RTerm::Return(Some(ValueId(6))),
            }],
        };
        let js_packed = lower_func(&interns, &f_obj, &heap);
        assert!(js_packed.contains("buf: []"), "{js_packed}");
        assert!(js_packed.contains(".push("), "{js_packed}");
        assert!(js_packed.contains(".pop()"), "{js_packed}");
        assert!(!js_packed.contains("new Array("), "{js_packed}");
        assert!(!js_packed.contains("nv_grow_one_arr"), "{js_packed}");
        assert!(!js_packed.contains("nv_reserve_arr"), "{js_packed}");
        assert!(!js_packed.contains("nv_shrink_arr"), "{js_packed}");
        assert!(!js_packed.contains("_nv_ea"), "{js_packed}");

        // 3. Non-TA VecResize (Vec<Obj>) must NOT be intercepted as TypedArray fill
        heap.packed_arr_vec_types.clear();
        let f_obj_resize = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 3,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ref_vec_obj, usize_ty, obj_ty],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ref_vec_obj,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: obj_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
            ],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1), ValueId(2)],
                ops: vec![std_call(
                    None,
                    StandardOperation::VecResize,
                    vec![ValueId(0), ValueId(1), ValueId(2)],
                    resize_sym,
                )],
                term: RTerm::Return(None),
            }],
        };
        let js_obj_resize = lower_func(&interns, &f_obj_resize, &heap);
        assert!(
            js_obj_resize.contains("alloc$vec$Vec$resize$h1("),
            "{js_obj_resize}"
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_func_bstr(
        interns: &Interns,
        func: &Func,
        heap: &HeapPlan,
        byte_views: ByteViewPlan,
        items: &host::ItemIndex,
        ascii: Option<&crate::analysis::ascii::AsciiFacts>,
    ) -> String {
        let body = Body {
            item: func.item,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: func.sig.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
        };
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: "test_fn".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::default(),
        };
        let js_name = JsName::new("test_fn".to_owned());
        let opts = Opts {
            profile: Some(Profile::ReleaseSpeed),
            str_repr: crate::opts::StrRepr::Bytes,
            ..Opts::default()
        };
        let vtables = unsize::VtableIndex::new();
        let input = LowerInput {
            name: &js_name,
            summary: &summary,
            body: &body,
            origins: None,
            interns,
            table: &interns.types,
            opts: &opts,
            items,
            drop_glue: host::EMPTY_DROP_GLUE,
            vtables: &vtables,
            spills: &[],
            abi: None,
            final_lowering: false,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            bigints: None,
            byte_views,
            heap: Some(heap),
            nested_clone: NestedClone::default(),
        };
        let func_ascii = ascii.map(|a| a.for_item(&js_name));
        jsast::stmt_to_string(
            &crate::lower::lower_func_with_ascii(&input, func, func_ascii)
                .expect("lower")
                .decl,
        )
    }

    #[test]
    fn phase2c_known_ascii_str_and_fast_str_lowerings() {
        let mut interns = Interns::new();
        let sym = interns.intern_str("core$str$hook$h0123456789abcdef");
        let start_key = interns.intern_str("start");
        let none_name = interns.intern_str("None");
        let some_name = interns.intern_str("Some");
        let f0_name = interns.intern_str("_0");
        let lit_lt = interns.intern_str("<");
        let lit_gt = interns.intern_str(">");
        let lit_empty = interns.intern_str("");
        let lit_pat = interns.intern_str("ab");

        let unit = TypeId(1);
        let bool_ty = TypeId(2);
        let usize_ty = TypeId(3);
        let char_ty = TypeId(4);
        let str_ty = TypeId(5);
        let ref_str_ty = TypeId(6);
        let range_from_ty = TypeId(7);
        let opt_str_ty = TypeId(8);
        let string_ty = TypeId(9);

        interns
            .types
            .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
        interns.types.insert(mk_desc(2, TypeKind::Bool, 1, false));
        interns.types.insert(mk_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));
        interns.types.insert(mk_desc(4, TypeKind::Char, 4, false));
        let mut str_desc = mk_desc(5, TypeKind::Str, 0, false);
        str_desc.layout.sized = false;
        str_desc.layout.size = None;
        str_desc.layout.zst = false;
        interns.types.insert(str_desc);
        let mut ref_str_desc = mk_desc(
            6,
            TypeKind::Reference {
                pointee: str_ty,
                mutable: false,
            },
            8,
            false,
        );
        ref_str_desc.js.pointer_form = Some(PointerForm::Str);
        interns.types.insert(ref_str_desc);

        interns.types.insert(mk_desc(
            7,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 70,
                },
                form: NominalForm::Struct,
                args: vec![usize_ty],
                variants: vec![VariantDescriptor {
                    name: start_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(start_key),
                        ty: usize_ty,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            4,
            false,
        ));

        let mut opt_desc = mk_desc(
            8,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 80,
                },
                form: NominalForm::Enum,
                args: vec![ref_str_ty],
                variants: vec![
                    VariantDescriptor {
                        name: none_name,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: some_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f0_name),
                            ty: ref_str_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            8,
            false,
        );
        opt_desc.js.enum_repr = Some(EnumRepr::NullNiche {
            none_variant: 0,
            some_variant: 1,
        });
        interns.types.insert(opt_desc);

        let mut string_desc = mk_desc(
            9,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 90,
                },
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: Vec::new(),
                niche: None,
            },
            12,
            true,
        );
        string_desc.meta.candidate = RepresentationCandidate::String;
        interns.types.insert(string_desc);

        let mut ascii = crate::analysis::ascii::AsciiFacts::default();
        ascii.funcs.insert(
            JsName::new("test_fn".to_owned()),
            BTreeSet::from([ValueId(0)]),
        );

        let heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };

        let std_call = |res, op, args| Op {
            result: res,
            origin: OriginId(0),
            kind: OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 1,
                            local_hash: 50,
                        },
                        instantiation: 1,
                    },
                    link_name: sym,
                },
                function: None,
                args,
                track_caller: None,
                semantic: CallSemantic::Standard(op),
                host: None,
            },
        };

        // Params: v0 = known-ASCII &str, v1 = unknown &str, v2 = usize, v3 = RangeFrom<usize>.
        let mut values = vec![
            ValueDef {
                ty: ref_str_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: ref_str_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: usize_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: range_from_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
        ];
        let mut ops = Vec::new();
        let mut push_val = |ty| {
            let id = ValueId(values.len() as u32);
            values.push(ValueDef {
                ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            });
            id
        };
        let c_lt = push_val(ref_str_ty);
        ops.push(Op {
            result: Some(c_lt),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Str(lit_lt)),
        });
        let c_gt = push_val(ref_str_ty);
        ops.push(Op {
            result: Some(c_gt),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Str(lit_gt)),
        });
        let c_empty = push_val(ref_str_ty);
        ops.push(Op {
            result: Some(c_empty),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Str(lit_empty)),
        });
        let c_pat = push_val(ref_str_ty);
        ops.push(Op {
            result: Some(c_pat),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Str(lit_pat)),
        });
        let c_nl = push_val(char_ty);
        ops.push(Op {
            result: Some(c_nl),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Scalar { bits: 10, bytes: 4 }),
        });
        let c_eacute = push_val(char_ty);
        ops.push(Op {
            result: Some(c_eacute),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Scalar {
                bits: 0xe9,
                bytes: 4,
            }),
        });
        let c_zero = push_val(usize_ty);
        ops.push(Op {
            result: Some(c_zero),
            origin: OriginId(0),
            kind: OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        });

        let r_ascii_known = push_val(bool_ty);
        ops.push(std_call(
            Some(r_ascii_known),
            StandardOperation::StrIsAscii,
            vec![ValueId(0)],
        ));
        let r_ascii_unk = push_val(bool_ty);
        ops.push(std_call(
            Some(r_ascii_unk),
            StandardOperation::StrIsAscii,
            vec![ValueId(1)],
        ));
        let r_bound_known = push_val(bool_ty);
        ops.push(std_call(
            Some(r_bound_known),
            StandardOperation::StrIsCharBoundary,
            vec![ValueId(0), ValueId(2)],
        ));
        let r_get_known = push_val(opt_str_ty);
        ops.push(std_call(
            Some(r_get_known),
            StandardOperation::StrGet,
            vec![ValueId(0), ValueId(3)],
        ));
        let r_lower_known = push_val(string_ty);
        ops.push(std_call(
            Some(r_lower_known),
            StandardOperation::StrToLowercase,
            vec![ValueId(0)],
        ));
        let r_sw1 = push_val(bool_ty);
        ops.push(std_call(
            Some(r_sw1),
            StandardOperation::StrStartsWith,
            vec![ValueId(1), c_lt],
        ));
        let r_ew1 = push_val(bool_ty);
        ops.push(std_call(
            Some(r_ew1),
            StandardOperation::BstrEndsWith,
            vec![ValueId(1), c_gt],
        ));
        let r_sw_empty = push_val(bool_ty);
        ops.push(std_call(
            Some(r_sw_empty),
            StandardOperation::BstrStartsWith,
            vec![ValueId(1), c_empty],
        ));
        let r_find_ch = push_val(usize_ty);
        ops.push(std_call(
            Some(r_find_ch),
            StandardOperation::BstrFindChar,
            vec![ValueId(1), c_zero, ValueId(2), c_nl],
        ));
        let r_rfind_ch = push_val(usize_ty);
        ops.push(std_call(
            Some(r_rfind_ch),
            StandardOperation::BstrRfindChar,
            vec![ValueId(1), c_zero, ValueId(2), c_eacute],
        ));
        let r_find_str = push_val(usize_ty);
        ops.push(std_call(
            Some(r_find_str),
            StandardOperation::BstrFindStr,
            vec![ValueId(1), ValueId(2), ValueId(2), c_pat],
        ));

        let func = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 10,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ref_str_ty, ref_str_ty, usize_ty, range_from_ty],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1), ValueId(2), ValueId(3)],
            cells: Vec::new(),
            values,
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1), ValueId(2), ValueId(3)],
                ops,
                term: RTerm::Return(None),
            }],
        };
        let items = host::ItemIndex::new();
        let js = lower_func_bstr(
            &interns,
            &func,
            &heap,
            ByteViewPlan {
                raw_byte_derefs: false,
                elide_scale_scalar: true,
            },
            &items,
            Some(&ascii),
        );
        // KnownAscii elides StrIsAscii, StrIsCharBoundary, StrGet, StrToLowercase on v0.
        assert!(js.contains("__rt._bstr_is_ascii("), "{js}");
        assert!(!js.contains("bstr_boundary"), "{js}");
        assert!(!js.contains("bstr_get_from"), "{js}");
        assert!(!js.contains("bstr_to_lowercase"), "{js}");
        assert!(js.contains(".toLowerCase()"), "{js}");
        // 1-char startsWith/endsWith -> charCodeAt(0) === 60 / charCodeAt(len - 1) === 62.
        assert!(js.contains(".charCodeAt(0) === 60"), "{js}");
        assert!(js.contains("length - 1) === 62"), "{js}");
        // Constant BstrFindChar / BstrRfindChar / BstrFindStr -> inline indexOf / lastIndexOf.
        assert!(!js.contains("bstr_find_char"), "{js}");
        assert!(!js.contains("bstr_rfind_char"), "{js}");
        assert!(!js.contains("bstr_find_str"), "{js}");
        assert!(js.contains(".indexOf(\"\\n\""), "{js}");
        assert!(js.contains(".lastIndexOf(\"Ã©\""), "{js}");
        assert!(js.contains(".indexOf(\"ab\""), "{js}");
    }

    #[test]
    fn phase2c_bstr_slice_search_and_slice_reverse_lowerings() {
        let mut interns = Interns::new();
        let rev_sym = interns.intern_str("core$slice$impl_0$reverse$h0123456789abcdef");
        let spoof_sym = interns.intern_str("user$slice$reverse$h0123456789abcdef");
        let x_key = interns.intern_str("x");

        let unit = TypeId(1);
        let bool_ty = TypeId(2);
        let u8_ty = TypeId(3);
        let u32_ty = TypeId(4);
        let str_ty = TypeId(5);
        let ref_slice_u8_ty = TypeId(8);
        let mut_slice_u32_ty = TypeId(10);
        let struct_ty = TypeId(11);
        let mut_slice_struct_ty = TypeId(13);
        let mut_slice_unit_ty = TypeId(15);

        interns
            .types
            .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
        interns.types.insert(mk_desc(2, TypeKind::Bool, 1, false));
        interns.types.insert(mk_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
            false,
        ));
        interns.types.insert(mk_desc(
            4,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));
        let mut str_desc = mk_desc(5, TypeKind::Str, 0, false);
        str_desc.layout.sized = false;
        str_desc.layout.size = None;
        str_desc.layout.zst = false;
        interns.types.insert(str_desc);
        let mut ref_str_desc = mk_desc(
            6,
            TypeKind::Reference {
                pointee: str_ty,
                mutable: false,
            },
            8,
            false,
        );
        ref_str_desc.js.pointer_form = Some(PointerForm::Str);
        interns.types.insert(ref_str_desc);

        let mk_slice_ref =
            |interns: &mut Interns, sid: u128, rid: u128, elem: TypeId, mut_: bool| {
                let mut sd = mk_desc(sid, TypeKind::Slice(elem), 0, false);
                sd.layout.sized = false;
                sd.layout.size = None;
                sd.layout.zst = false;
                interns.types.insert(sd);
                let mut rd = mk_desc(
                    rid,
                    TypeKind::Reference {
                        pointee: TypeId(sid),
                        mutable: mut_,
                    },
                    8,
                    false,
                );
                rd.js.pointer_form = Some(PointerForm::FatSlice);
                interns.types.insert(rd);
            };
        mk_slice_ref(&mut interns, 7, 8, u8_ty, false);
        mk_slice_ref(&mut interns, 9, 10, u32_ty, true);
        interns.types.insert(mk_desc(
            11,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 5,
                    local_hash: 11,
                },
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: x_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(x_key),
                        ty: u32_ty,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            4,
            true,
        ));
        mk_slice_ref(&mut interns, 12, 13, struct_ty, true);
        mk_slice_ref(&mut interns, 14, 15, unit, true);

        let rev_item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let rev_struct_item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 2,
        };
        let rev_zst_item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 3,
        };
        let spoof_item = ItemId {
            definition: DefinitionId {
                crate_id: 99,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let sw_item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 2,
            },
            instantiation: 1,
        };
        let ew_item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 3,
            },
            instantiation: 1,
        };

        let mut ascii = crate::analysis::ascii::AsciiFacts::default();
        ascii.slice_reverse.insert(rev_item);
        ascii.slice_reverse.insert(rev_struct_item);
        ascii.slice_reverse.insert(rev_zst_item);
        ascii.slice_starts_with.insert(sw_item);
        ascii.slice_ends_with.insert(ew_item);

        let mut heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            raw_alloc_reachable: false,
            non_rc_raw_alloc_reachable: false,
            any_boxed: false,
            ..HeapPlan::default()
        };
        let items = host::ItemIndex::new();
        let bv_clean = ByteViewPlan {
            raw_byte_derefs: false,
            elide_scale_scalar: true,
        };

        // 1. Primitive &mut [u32] reverse -> inline while loop, no call.
        let mk_rev_func = |slice_ty: TypeId, callee_item: ItemId, link_name| Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 20,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![slice_ty],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0)],
            cells: Vec::new(),
            values: vec![ValueDef {
                ty: slice_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            }],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0)],
                ops: vec![Op {
                    result: None,
                    origin: OriginId(0),
                    kind: OpKind::Call {
                        callee: Callee::Direct {
                            item: callee_item,
                            link_name,
                        },
                        function: None,
                        args: vec![ValueId(0)],
                        track_caller: None,
                        semantic: CallSemantic::Ordinary,
                        host: None,
                    },
                }],
                term: RTerm::Return(None),
            }],
        };

        let js_u32 = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_u32_ty, rev_item, rev_sym),
            &heap,
            bv_clean,
            &items,
            Some(&ascii),
        );
        assert!(js_u32.contains("while ("), "{js_u32}");
        assert!(!js_u32.contains("core$slice$impl_0$reverse"), "{js_u32}");

        // 2. Indirect untracked &mut [Struct] reverse -> inline while loop.
        let js_struct = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_struct_ty, rev_struct_item, rev_sym),
            &heap,
            bv_clean,
            &items,
            Some(&ascii),
        );
        assert!(js_struct.contains("while ("), "{js_struct}");
        assert!(
            !js_struct.contains("core$slice$impl_0$reverse"),
            "{js_struct}"
        );

        // 3. Indirect tracked (&mut [Struct] with needs_box_tracking) -> MUST fall back!
        heap.any_boxed = true;
        heap.all_adts_boxed = true;
        let js_struct_tracked = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_struct_ty, rev_struct_item, rev_sym),
            &heap,
            bv_clean,
            &items,
            Some(&ascii),
        );
        assert!(
            js_struct_tracked.contains("core$slice$impl_0$reverse"),
            "{js_struct_tracked}"
        );
        heap.any_boxed = false;
        heap.all_adts_boxed = false;

        // 4. raw_byte_derefs == true -> MUST fall back!
        let js_raw_bv = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_u32_ty, rev_item, rev_sym),
            &heap,
            ByteViewPlan {
                raw_byte_derefs: true,
                elide_scale_scalar: false,
            },
            &items,
            Some(&ascii),
        );
        assert!(
            js_raw_bv.contains("core$slice$impl_0$reverse"),
            "{js_raw_bv}"
        );

        // 5. ZST &mut [()] reverse -> elided completely (no while, no call).
        let js_zst = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_unit_ty, rev_zst_item, rev_sym),
            &heap,
            bv_clean,
            &items,
            Some(&ascii),
        );
        assert!(!js_zst.contains("while ("), "{js_zst}");
        assert!(!js_zst.contains("core$slice$impl_0$reverse"), "{js_zst}");

        // 6. Unauthenticated spoof -> MUST call canonical function.
        let js_spoof = lower_func_bstr(
            &interns,
            &mk_rev_func(mut_slice_u32_ty, spoof_item, spoof_sym),
            &heap,
            bv_clean,
            &items,
            Some(&ascii),
        );
        assert!(js_spoof.contains("user$slice$reverse"), "{js_spoof}");

        // 7. Native ByteStr `core::slice::<impl [u8]>::{starts_with,ends_with}` over ByteViews
        // (FullSlice + VarSlice with const 0/1/multi-byte and dynamic ByteView needles).
        let f_bstr_slice = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 30,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ref_slice_u8_ty, ref_slice_u8_ty],
                result: bool_ty,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ref_slice_u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ref_slice_u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ref_slice_u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ref_slice_u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ref_slice_u8_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
            ],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1)],
                ops: vec![
                    Op {
                        result: Some(ValueId(2)),
                        origin: OriginId(0),
                        kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(
                            vec![
                                ConstTree::Scalar { bits: 96, bytes: 1 },
                                ConstTree::Scalar { bits: 96, bytes: 1 },
                                ConstTree::Scalar { bits: 96, bytes: 1 },
                            ],
                        )))),
                    },
                    Op {
                        result: Some(ValueId(3)),
                        origin: OriginId(0),
                        kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(
                            vec![ConstTree::Scalar { bits: 62, bytes: 1 }],
                        )))),
                    },
                    Op {
                        result: Some(ValueId(4)),
                        origin: OriginId(0),
                        kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(
                            Vec::new(),
                        )))),
                    },
                ],
                term: RTerm::Return(None),
            }],
        };
        let opts = Opts {
            profile: Some(Profile::ReleaseSpeed),
            str_repr: crate::opts::StrRepr::Bytes,
            ..Opts::default()
        };
        let mut plan = crate::lower::plan::Plan::default();
        plan.names.insert(ValueId(0), "h".to_owned());
        plan.names.insert(ValueId(1), "n".to_owned());
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: "test_fn".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::default(),
        };
        let cx = super::super::JsCx::new(
            &f_bstr_slice,
            &plan,
            &interns.types,
            &interns,
            &opts,
            &summary,
            None,
        )
        .with_byte_views(bv_clean)
        .with_heap(Some(&heap))
        .with_ascii(Some(ascii.for_item(&JsName::new("test_fn".to_owned()))));

        let full_bv = super::super::ByteView {
            mode: super::super::ByteViewMode::FullSlice,
            base_str: None,
            base_cell: None,
            base_is_string_obj: false,
            borrow_slice: None,
        };
        let var_bv = super::super::ByteView {
            mode: super::super::ByteViewMode::VarSlice,
            base_str: None,
            base_cell: None,
            base_is_string_obj: false,
            borrow_slice: None,
        };

        let render_lowered = |l: super::Lowered| match l {
            super::Lowered::Value(mut stmts, expr) => {
                stmts.push(jsast::ret(expr));
                jsast::program_to_string(&stmts)
            }
            _ => panic!("expected Value"),
        };

        // FullSlice + multi-byte const `b"```"` -> `h.startsWith("```")`.
        let js_full_sw = render_lowered(
            cx.lower_bstr_slice_search_views(true, ValueId(0), full_bv, ValueId(2), None)
                .unwrap()
                .unwrap(),
        );
        assert!(js_full_sw.contains("h.startsWith(\"```\")"), "{js_full_sw}");

        // FullSlice + 1-byte const `b">"` ends_with -> `h.charCodeAt(h.length - 1) === 62`.
        let js_full_ew1 = render_lowered(
            cx.lower_bstr_slice_search_views(false, ValueId(0), full_bv, ValueId(3), None)
                .unwrap()
                .unwrap(),
        );
        assert!(
            js_full_ew1.contains("h.charCodeAt(h.length - 1) === 62"),
            "{js_full_ew1}"
        );

        // FullSlice + empty const `b""` -> `true`.
        let js_empty = render_lowered(
            cx.lower_bstr_slice_search_views(true, ValueId(0), full_bv, ValueId(4), None)
                .unwrap()
                .unwrap(),
        );
        assert!(js_empty.contains("return true;"), "{js_empty}");

        // VarSlice + multi-byte const `b"```"` starts_with -> `len >= 3 && buf.startsWith("```", off)`.
        let js_var_sw = render_lowered(
            cx.lower_bstr_slice_search_views(true, ValueId(0), var_bv, ValueId(2), None)
                .unwrap()
                .unwrap(),
        );
        assert!(js_var_sw.contains(">= 3 &&"), "{js_var_sw}");
        assert!(js_var_sw.contains(".startsWith(\"```\","), "{js_var_sw}");

        // VarSlice + multi-byte const `b"```"` ends_with -> `len >= 3 && buf.endsWith("```", off + len)`.
        let js_var_ew = render_lowered(
            cx.lower_bstr_slice_search_views(false, ValueId(0), var_bv, ValueId(2), None)
                .unwrap()
                .unwrap(),
        );
        assert!(js_var_ew.contains(">= 3 &&"), "{js_var_ew}");
        assert!(js_var_ew.contains(".endsWith(\"```\","), "{js_var_ew}");

        // VarSlice + dynamic FullSlice needle -> `len >= n.length && buf.startsWith(n, off)`.
        let js_var_dyn = render_lowered(
            cx.lower_bstr_slice_search_views(true, ValueId(0), var_bv, ValueId(1), Some(full_bv))
                .unwrap()
                .unwrap(),
        );
        assert!(js_var_dyn.contains(">= n.length &&"), "{js_var_dyn}");
        assert!(js_var_dyn.contains(".startsWith(n,"), "{js_var_dyn}");
    }
}
