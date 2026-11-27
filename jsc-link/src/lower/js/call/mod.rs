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

mod native_string;
mod native_vec;
mod slice_ops;
mod str_ops;
mod typed_heap;

#[cfg(test)]
mod tests;

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::repr::{Op, OpKind, Proj, Root, ValueId};
use rcgjs_object::jir::semantic::{CallSemantic, Callee, DropGlue, ItemId, StandardOperation};
use rcgjs_object::jsast::{self, Expr, Stmt};
use rcgjs_object::ptr;

use super::ptr::sequenced;
use super::{decline, emit, host, intrinsics, value, JsCx, Res, CALLER_LOCATION_PARAM};

pub(super) use native_vec::resolve_vec_call_target;

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

    /// Reads a constant scalar integer value if `v` is defined by an `OpKind::Const(Scalar)`.
    fn const_scalar_u128(&self, v: ValueId) -> Option<u128> {
        match self.def_of(v).map(|def| &def.kind) {
            Some(OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar {
                bits, ..
            })) => Some(*bits),
            _ => None,
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
        let _ = host;

        if let Some(lowered) =
            self.try_lower_special_call(op, callee, args, *track_caller, semantic)?
        {
            return Ok(lowered);
        }

        let physical = match (callee, self.abi) {
            (Callee::Direct { item, .. }, Some(abi)) => abi.call_signature(*item),
            _ => None,
        };
        let (arguments, owned_virtual_receiver) =
            self.lower_call_arguments(callee, args, *track_caller, physical)?;

        match callee {
            Callee::Direct { item, link_name } => {
                self.emit_direct_call(op, *item, *link_name, arguments, physical)
            }
            Callee::Foreign { symbol, .. } => Ok(Lowered::value(jsast::call(
                jsast::id(self.link_name(*symbol)?),
                arguments,
            ))),
            Callee::Runtime { member, .. } => self.emit_runtime_call(op, *member, args, arguments),
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
            Callee::Virtual { slot, .. } => {
                self.emit_virtual_call(*slot, arguments, owned_virtual_receiver)
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

    /// Tries all call-site lowering interceptors (host markers, native Vec/String/SoA/typed-heap,
    /// `js-str=bytes` str ops, format kernel, `Callee::Nop`, and slice search/reverse).
    fn try_lower_special_call(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
        track_caller: Option<SourceFactId>,
        semantic: &CallSemantic,
    ) -> Res<Option<Lowered>> {
        // Everything host-shaped is W9's, and is recognized before anything asks what the callee
        // is: a marker resolves to an ordinary Rust body whose only statement is `unreachable!`,
        // so calling it would compile and abort at run time (`abi.rs`, "view-abi markers"). The
        // exception `host::intercepts` makes is v1's: an async marker with a safe Rust body is an
        // ordinary call unless the future-poller pass applies.
        if host::intercepts(self, op) {
            return host::lower_call(self, op).map(Lowered::value).map(Some);
        }

        if let Some(lowered) =
            self.try_lower_standard_semantic_call(op, callee, args, track_caller, semantic)?
        {
            return Ok(Some(lowered));
        }

        // A format call the representation plan committed (Phase 4, W4g): the constructor's text
        // or the consumer's concatenation, inlined at the site. Inert unless the plan marked the
        // site's carriers, so `off`/`report` and every uncommitted call stay plain calls.
        if let Some(lowered) = crate::lower::semantic_ops::fmt_kernel::intercept(self, op)? {
            return Ok(Some(lowered));
        }

        if matches!(callee, Callee::Nop) {
            // `drop_in_place::<T>` for a `T` with empty glue: the arguments are place reads with
            // no effect, so the whole call is nothing.
            return Ok(Some(Lowered::Done(Vec::new())));
        }

        // Phase 2C: Native ByteStr `core::slice::<impl [u8]>::{starts_with,ends_with}` and inline
        // `core::slice::<impl [T]>::{reverse,revswap}` under `ReleaseSpeed`.
        if self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed) && track_caller.is_none() {
            if self.bstr_enabled() {
                if let Some(lowered) = self.lower_bstr_slice_search(op, callee, args)? {
                    return Ok(Some(lowered));
                }
            }
            if let Some(lowered) = self.lower_slice_reverse(op, callee, args)? {
                return Ok(Some(lowered));
            }
        }

        Ok(None)
    }

    fn try_lower_standard_semantic_call(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
        track_caller: Option<SourceFactId>,
        semantic: &CallSemantic,
    ) -> Res<Option<Lowered>> {
        let CallSemantic::Standard(operation) = semantic else {
            return Ok(None);
        };

        // Phase 12D's opt-in exact numeric storage is value-local, so it is intercepted at the
        // prepared caller rather than changing the shared `Vec::with_capacity`/`Vec::push` bodies
        // for every vector of the same element type.
        if *operation == StandardOperation::VecAsSlice && self.heap().native_vec_enabled {
            if let Some(lowered) =
                self.lower_soa_vec_call(op, StandardOperation::NativeVecAsSlice, args)?
            {
                return Ok(Some(lowered));
            }
            if let Some(lowered) =
                self.lower_native_vec_call(op, StandardOperation::NativeVecAsSlice, args)?
            {
                return Ok(Some(lowered));
            }
        }

        // A compiler-tagged `String::deref` at a committed carrier is a boundary operation, not a
        // reason to enter (or replace) the shared `alloc` body.
        if *operation == StandardOperation::StringAsStr {
            let [receiver] = args else {
                return Err(decline(OpClass::Call, "String::deref with the wrong arity"));
            };
            if self.heap().native_string_enabled {
                let receiver = self.expr_of(*receiver)?;
                return Ok(Some(Lowered::value(jsast::member(receiver, "s"))));
            }
        }

        // Phase 5 fast `core::str` comparison/search hooks (`ReleaseSpeed`, plus exact JS string
        // predicates `BstrEq`/`BstrStartsWith`/`BstrEndsWith`/`BstrContains` whenever `js-opt=on`,
        // since UTF-8 and UTF-16 equality/prefix/suffix/substring predicates agree identically).
        let release_speed = self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed);
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
                return Ok(Some(lowered));
            }
        }

        // Phase 5 universal native `{ s, cap }` `String` hooks (`ReleaseSpeed` + `js-str=bytes`).
        if self.heap().native_string_enabled
            && (operation.is_native_string()
                || matches!(
                    operation,
                    StandardOperation::StrToLowercase | StandardOperation::StrToUppercase
                ))
        {
            if let Some(lowered) = self.lower_native_string_call(*operation, args)? {
                return Ok(Some(lowered));
            }
        }

        // Mires v4 Phase 3 universal flat `Vec<T, Global>` hooks (`ReleaseSpeed` + `Default`
        // allocator + `!byte_views` + `!non_rc_raw_alloc_reachable`).
        if self.heap().native_vec_enabled {
            if operation.is_native_vec() {
                if let Some(lowered) = self.lower_soa_vec_call(op, *operation, args)? {
                    return Ok(Some(lowered));
                }
                if let Some(lowered) = self.lower_native_vec_call(op, *operation, args)? {
                    return Ok(Some(lowered));
                }
            } else if *operation == StandardOperation::VecResize {
                if let Some(lowered) = self.lower_soa_vec_resize(args)? {
                    return Ok(Some(lowered));
                }
                if let Some(lowered) = self.lower_native_vec_resize(args)? {
                    return Ok(Some(lowered));
                }
            }
        }

        // Tier A typed managed heap (`ReleaseSpeed && Default allocator && !byte_views`): lower
        // `Global` `RawVec`/`Box`/`BTreeMap`/`Rc` allocation hooks directly to zero-WeakMap
        // TypedArrays / plain Arrays and no-op deallocations.
        if operation.is_typed_heap() && self.heap().typed_heap_enabled {
            if let Some(lowered) = self.lower_typed_heap_call(op, *operation, args)? {
                return Ok(Some(lowered));
            }
        }

        // `js-str=bytes`: a `str` method with a producer-tagged identity is the JavaScript string
        // operation it is on a byte string (`lower/bstr.rs`).
        if operation.is_str() && self.bstr_enabled() {
            if let Some(lowered) =
                self.lower_str_call(op, *operation, callee, args, track_caller)?
            {
                return Ok(Some(lowered));
            }
        }

        Ok(None)
    }

    fn lower_call_arguments(
        &self,
        callee: &Callee,
        args: &[ValueId],
        track_caller: Option<SourceFactId>,
        physical: Option<&crate::plan::abi_flat::FlatSig>,
    ) -> Res<(Vec<Expr>, bool)> {
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
            if slice_positions.contains(&position) {
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
        if let Some(location) = self.caller_location_argument(callee, args.len(), track_caller)? {
            arguments.push(location);
        }
        Ok((arguments, owned_virtual_receiver))
    }

    fn emit_direct_call(
        &self,
        op: &Op,
        item: ItemId,
        link_name: StrId,
        arguments: Vec<Expr>,
        physical: Option<&crate::plan::abi_flat::FlatSig>,
    ) -> Res<Lowered> {
        let name = match self.call_retargets.get(&item) {
            Some(name) => name.as_str().to_owned(),
            None => self.link_name(link_name)?,
        };
        let call = jsast::call(jsast::id(name), arguments);
        let tagged = physical
            .and_then(|sig| sig.result.as_ref())
            .and_then(|result| result.tagged());
        let recursive = physical
            .and_then(|sig| sig.result.as_ref())
            .and_then(|result| result.recursive());
        let i64_pair = physical
            .and_then(|sig| sig.result.as_ref())
            .is_some_and(|result| result.is_i64_pair());
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
        // The call runs before any spill read, then every lane is copied into a local: another
        // flattened call may reuse the module-scope registers before this value is consumed. The
        // ordinary object below is only a virtual semantic description; `emit` caches it, and
        // member folding erases it on the field-only/forwarded uses whose proof admitted this
        // result ABI.
        let (bind, tag) = self.temp(call);
        let mut prelude = vec![bind];
        let fields = tagged.map_or_else(
            || recursive.expect("one physical result shape").fields(),
            |tagged| tagged.fields(),
        );
        let payloads: Vec<Expr> = (1..fields)
            .map(|index| {
                let (bind, payload) = self.temp(jsast::id(emit::spill_name_of(self.spills, index)));
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

    fn emit_runtime_call(
        &self,
        op: &Op,
        member: StrId,
        args: &[ValueId],
        mut arguments: Vec<Expr>,
    ) -> Res<Lowered> {
        // An `extern "C"` item is the host's (`__rt.<symbol>`): under `js-str=bytes` every `str`
        // an argument reaches is decoded to the host text it encodes and every `str` the result
        // reaches is encoded into the byte string Rust holds (`lower/bstr.rs`).
        if self.bstr_enabled() {
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
        let call = jsast::rt_call(self.link_name(member)?, arguments);
        match op.result {
            Some(result) if self.bstr_enabled() => Ok(Lowered::value(host::crossing(
                self,
                self.ty_of(result)?,
                call,
                crate::lower::bstr::Crossing::In,
            )?)),
            _ => Ok(Lowered::value(call)),
        }
    }

    /// `slot` is absolute: `first_method_vtable_slot` counts the three header entries, so it
    /// indexes the array `vtable.rs` builds directly.
    fn emit_virtual_call(
        &self,
        slot: u32,
        arguments: Vec<Expr>,
        owned_virtual_receiver: bool,
    ) -> Res<Lowered> {
        let Some((receiver, rest)) = arguments.split_first() else {
            return Err(decline(
                OpClass::Call,
                "a virtual call takes at least a receiver",
            ));
        };
        let (setup, receiver) = self.reread(receiver.clone(), &[ptr::META, ptr::PTR]);
        let data = jsast::member(receiver.clone(), ptr::PTR);
        let data = if owned_virtual_receiver {
            jsast::rt_call("unref", vec![data])
        } else {
            data
        };
        let mut call_args = vec![data];
        call_args.extend(rest.iter().cloned());
        let call = jsast::call(vtable_slot(receiver, f64::from(slot)), call_args);
        let prelude = setup.map(jsast::expr_stmt).into_iter().collect();
        Ok(Lowered::Value(prelude, call))
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
                let (setup, receiver) = self.reread(value, &[ptr::META, ptr::PTR]);
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
