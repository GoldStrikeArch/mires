//! Native intrinsic lowerings: v1 `backend/src/intrinsics.rs`, over the type table instead of
//! `tcx`.
//!
//! [`lower`] is what `call.rs` reaches for an `OpKind::Intrinsic`. It answers in one of three
//! ways, exactly as v1 does:
//!
//! * a native lowering — the whole call, including any statements it needs first;
//! * the intrinsic's own MIR body, when the mono collector emitted one (`fallback`), called like
//!   any other direct callee;
//! * v1's zombie, when neither exists. A zombie is *not* a decline: v1 emits one too, so the two
//!   lanes abort at the same place with the same message and the item still lowers.
//!
//! A construct v1 answers from a `tcx` query the type table does not carry is a [`decline`]
//! instead — the item keeps its baseline text rather than aborting where v1 worked. Those are
//! `type_name` (a type's printed path), `const_eval_select` (the runtime callee's link name) and
//! the `assert_*_valid` validity queries; see the workstream report.
//!
//! # Integer representation
//!
//! Every integer arm dispatches on [`IntRepr`]: up to 32 bits a value is a JavaScript number and
//! the bit twiddling is the usual 32 bit folklore; at 64 and 128 bits it is a `BigInt` and the
//! same operations are small loops.

use std::cell::RefCell;

use rcgjs_object::evaluation::OperandEvaluation;
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::repr::{Op, OpKind, ValueId};
use rcgjs_object::jir::semantic::{BinaryOp, ConstTree, DirectItemRef};
use rcgjs_object::jir::types::{IntRepr, NominalForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt};
use rcgjs_object::ptr;

use super::call::Lowered;
use super::{decline, value, JsCx, Res};

/// The vtable slots the layout intrinsics read: `[drop, size, align, methods...]`.
const VTABLE_SIZE_SLOT: f64 = 1.0;
const VTABLE_ALIGN_SLOT: f64 = 2.0;

/// The memory ordering words older toolchains spelled into an atomic intrinsic's name.
const ORDERINGS: [&str; 5] = ["seqcst", "acquire", "release", "acqrel", "relaxed"];

/// The parameters `raw_eq` binds an array's element and index to.
const RAW_EQ_ELEMENT: &str = "$e";
const RAW_EQ_INDEX: &str = "$i";

/// One lowered intrinsic call.
enum Outcome {
    /// The value to bind to the result.
    Value(Expr),
    /// The 64-bit integer `(lo, hi)` pair to bind to the result.
    Pair((Expr, Expr)),
    /// Statements that are the whole lowering; there is no value.
    Done(Vec<Stmt>),
}

/// Lowers one `OpKind::Intrinsic`.
pub fn lower(
    cx: &JsCx<'_>,
    op: &Op,
    name: StrId,
    types: &[TypeId],
    fallback: Option<DirectItemRef>,
    args: &[ValueId],
    track_caller: Option<SourceFactId>,
) -> Res<Lowered> {
    let Some(name) = cx.interns.str(name) else {
        return Err(decline(
            OpClass::Intrinsic,
            "an intrinsic whose name is not in the interns",
        ));
    };
    let result_ty = op.result.map(|result| cx.ty_of(result)).transpose()?;
    let int32_only = op.result.is_some_and(|result| cx.is_int32_only(result));
    let mut intrinsic = Intrinsic {
        cx,
        types,
        args,
        result_ty,
        int32_only,
        track_caller,
        captured: None,
        prelude: RefCell::new(Vec::new()),
    };
    let has_i64 = result_ty.is_some_and(|ty| cx.is_i64_ty(ty))
        || args.iter().any(|arg| cx.is_i64_value(*arg));
    if OperandEvaluation::intrinsic(name) == OperandEvaluation::Materialize && !has_i64 {
        let mut arguments = args
            .iter()
            .map(|arg| cx.expr_of(*arg))
            .collect::<Res<Vec<_>>>()?;
        let is_deref_intrinsic = matches!(
            name,
            "write_via_move"
                | "volatile_store"
                | "unaligned_volatile_store"
                | "nontemporal_store"
                | "read_via_copy"
                | "volatile_load"
                | "unaligned_volatile_load"
        );
        let unwrapped_slot = if is_deref_intrinsic && !arguments.is_empty() {
            let candidate = super::ptr::unwrap_offset_cond(arguments[0].clone());
            if let Expr::Object(ref fields) = candidate {
                let buf = fields
                    .iter()
                    .find(|(k, _)| k == "buf")
                    .map(|(_, v)| v.clone());
                let off = fields
                    .iter()
                    .find(|(k, _)| k == "off")
                    .map(|(_, v)| v.clone());
                match (buf, off) {
                    (Some(b), Some(o)) if fields.len() == 2 => Some((b, o)),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };
        let unwrapped_copy_slots = if name == "copy_nonoverlapping" && arguments.len() == 3 {
            let s0 = unwrap_safe_slot(&arguments[0]);
            let s1 = unwrap_safe_slot(&arguments[1]);
            if s0.is_some() || s1.is_some() {
                Some((s0, s1))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((buf, off)) = unwrapped_slot {
            let rest = arguments.split_off(1);
            if rest.iter().all(jsast::is_droppable)
                && jsast::is_droppable(&buf)
                && jsast::is_droppable(&off)
            {
                let mut rebuilt = vec![ptr::slot(buf, off)];
                rebuilt.extend(rest);
                intrinsic.captured = Some(rebuilt);
            } else {
                let mut flat = Vec::with_capacity(2 + rest.len());
                flat.push(buf);
                flat.push(off);
                flat.extend(rest);
                let (setup, mut cap) =
                    OperandEvaluation::Materialize.capture(flat, |value| cx.temp(value));
                intrinsic.prelude.borrow_mut().extend(setup);
                let cap_off = cap.remove(1);
                let cap_buf = cap.remove(0);
                let mut rebuilt = vec![ptr::slot(cap_buf, cap_off)];
                rebuilt.extend(cap);
                intrinsic.captured = Some(rebuilt);
            }
        } else if let Some((s0, s1)) = unwrapped_copy_slots {
            let mut flat = Vec::with_capacity(5);
            let s0_unwrapped = if let Some((b, o)) = s0 {
                flat.push(b);
                flat.push(o);
                true
            } else {
                flat.push(arguments[0].clone());
                false
            };
            let s1_unwrapped = if let Some((b, o)) = s1 {
                flat.push(b);
                flat.push(o);
                true
            } else {
                flat.push(arguments[1].clone());
                false
            };
            flat.push(arguments[2].clone());
            let mut cap = if flat.iter().all(jsast::is_droppable) {
                flat
            } else {
                let (setup, captured) =
                    OperandEvaluation::Materialize.capture(flat, |value| cx.temp(value));
                intrinsic.prelude.borrow_mut().extend(setup);
                captured
            };
            let mut idx = 0;
            let a0 = if s0_unwrapped {
                let b = cap[idx].clone();
                let o = cap[idx + 1].clone();
                idx += 2;
                ptr::slot(b, o)
            } else {
                let v = cap[idx].clone();
                idx += 1;
                v
            };
            let a1 = if s1_unwrapped {
                let b = cap[idx].clone();
                let o = cap[idx + 1].clone();
                idx += 2;
                ptr::slot(b, o)
            } else {
                let v = cap[idx].clone();
                idx += 1;
                v
            };
            let a2 = cap.remove(idx);
            intrinsic.captured = Some(vec![a0, a1, a2]);
        } else {
            let (setup, arguments) =
                OperandEvaluation::Materialize.capture(arguments, |value| cx.temp(value));
            intrinsic.prelude.borrow_mut().extend(setup);
            intrinsic.captured = Some(arguments);
        }
    }
    let outcome = match intrinsic.lower(name)? {
        Some(outcome) => outcome,
        // No native lowering: the intrinsic's own MIR body, or v1's zombie.
        None => {
            let prelude = intrinsic.prelude.into_inner();
            return match fallback {
                Some(item) => {
                    let name = cx.interns.str(item.link_name).unwrap_or("").to_owned();
                    let arguments = match intrinsic.captured {
                        Some(arguments) => arguments,
                        None => args
                            .iter()
                            .map(|arg| cx.expr_of(*arg))
                            .collect::<Res<_>>()?,
                    };
                    Ok(Lowered::Value(
                        prelude,
                        jsast::call(jsast::id(name), arguments),
                    ))
                }
                None => Ok(Lowered::Value(
                    prelude,
                    cx.zombie(format!(
                        "intrinsic `{name}` is not supported by rustc_codegen_js"
                    )),
                )),
            };
        }
    };
    let prelude = intrinsic.prelude.into_inner();
    Ok(match outcome {
        Outcome::Value(value) => Lowered::Value(prelude, value),
        Outcome::Pair(pair) => Lowered::Pair(prelude, pair),
        Outcome::Done(stmts) => {
            let mut out = prelude;
            out.extend(stmts);
            Lowered::Done(out)
        }
    })
}

/// The state of one intrinsic call's lowering.
struct Intrinsic<'a, 'b> {
    cx: &'a JsCx<'b>,
    types: &'a [TypeId],
    args: &'a [ValueId],
    result_ty: Option<TypeId>,
    int32_only: bool,
    track_caller: Option<SourceFactId>,
    captured: Option<Vec<Expr>>,
    prelude: RefCell<Vec<Stmt>>,
}

impl Intrinsic<'_, '_> {
    /// The dispatch table (`intrinsics.rs::lower`). `None` means "no native lowering".
    fn lower(&self, name: &str) -> Res<Option<Outcome>> {
        if name.starts_with("simd_") {
            return Ok(Some(self.unsupported(format!(
                "the SIMD intrinsic `{name}` is not supported by rustc_codegen_js"
            ))));
        }
        if name.ends_with("f16") || name.ends_with("f128") {
            return Ok(Some(self.unsupported(format!(
                "`{name}` is not supported by rustc_codegen_js: JavaScript has no `f16` or `f128`"
            ))));
        }
        if let Some(outcome) = self.layout(name)? {
            return Ok(Some(outcome));
        }
        if let Some(outcome) = self.control(name)? {
            return Ok(Some(outcome));
        }
        if let Some(outcome) = self.integer(name)? {
            return Ok(Some(outcome));
        }
        if let Some(outcome) = self.float(name)? {
            return Ok(Some(outcome));
        }
        if let Some(outcome) = self.pointer(name)? {
            return Ok(Some(outcome));
        }
        self.atomic(name)
    }

    // -----------------------------------------------------------------------------------------
    // Types and layout
    // -----------------------------------------------------------------------------------------

    fn layout(&self, name: &str) -> Res<Option<Outcome>> {
        let table = self.cx.table;
        Ok(Some(match name {
            "size_of" | "align_of" => {
                let ty = self.type_at(0)?;
                let bytes = if name == "size_of" {
                    value::size_of(table, ty)
                } else {
                    value::align_of(table, ty)
                };
                match bytes {
                    Some(bytes) => Outcome::Value(self.usize_literal(bytes)?),
                    // v1 read the answer off `tcx.layout_of`; a type the encoder recorded no
                    // layout for is the lowerer's gap, not the program's, so the item falls back.
                    None => {
                        return Err(decline(
                            OpClass::Intrinsic,
                            format!("`{name}` of a type the table records no layout for"),
                        ))
                    }
                }
            }
            "size_of_val" | "align_of_val" => self.size_of_val(name == "size_of_val")?,
            "vtable_size" | "vtable_align" => {
                let slot = if name == "vtable_size" {
                    VTABLE_SIZE_SLOT
                } else {
                    VTABLE_ALIGN_SLOT
                };
                Outcome::Value(jsast::index(self.arg(0)?, jsast::num(slot)))
            }
            "variant_count" => {
                let ty = self.type_at(0)?;
                let count = match value::kind(table, table.peeled(ty)) {
                    Some(TypeKind::Adt {
                        form: NominalForm::Enum,
                        variants,
                        ..
                    }) => variants.len(),
                    _ => 1,
                };
                Outcome::Value(self.usize_literal(count as u64)?)
            }
            "needs_drop" => {
                let ty = self.type_at(0)?;
                let needs = table.get(ty).is_some_and(|d| d.layout.needs_drop);
                Outcome::Value(jsast::boolean(needs))
            }
            // Returns a `&'static str`, which is a JavaScript string. The producer records the
            // printed path on the descriptor (`TypeDescriptor::name`).
            "type_name" => {
                let ty = self.type_at(0)?;
                let name = table.get(ty).and_then(|descriptor| descriptor.name);
                match name.and_then(|name| self.cx.interns.str(name)) {
                    Some(name) => Outcome::Value(self.cx.string_literal(name)),
                    None => {
                        return Err(decline(
                            OpClass::Intrinsic,
                            "`type_name` of a type the table records no printed path for",
                        ))
                    }
                }
            }
            // A `TypeId` is the stable 128 bit type hash, as a hexadecimal string — which is
            // exactly what a `TypeId` in this schema is.
            "type_id" => {
                let ty = self.type_at(0)?;
                Outcome::Value(jsast::string(format!("{:032x}", ty.0)))
            }
            "offset_of" | "field_offset" => self.unsupported(format!(
                "`{name}` is not supported by rustc_codegen_js: a struct is a JavaScript object, \
                 which has no byte offsets"
            )),
            // The discriminant **number**, never the tag: the result wraps the raw integer and a
            // caller may hash it as well as compare it.
            "discriminant_value" => {
                let ty = self.type_at(0)?;
                let peeled = table.peeled(ty);
                let is_enum = matches!(
                    value::kind(table, peeled),
                    Some(TypeKind::Adt {
                        form: NominalForm::Enum,
                        ..
                    })
                );
                if !is_enum {
                    Outcome::Value(jsast::num(0))
                } else {
                    // The value is read through the pointer the intrinsic takes.
                    let read = self.cx.deref_read(ty, self.arg(0)?);
                    let repr = self
                        .result_ty
                        .map_or(IntRepr::Number, |result| table.int_repr(result));
                    match value::discriminant_number(table, self.cx.interns, ty, read, repr) {
                        Some(number) => Outcome::Value(number),
                        None => Outcome::Value(jsast::num(0)),
                    }
                }
            }
            _ => return Ok(None),
        }))
    }

    /// `size_of_val` / `align_of_val`, whose answer depends on a fat pointer's metadata.
    fn size_of_val(&self, size: bool) -> Res<Outcome> {
        let table = self.cx.table;
        let ty = self.type_at(0)?;
        let sized = table.get(ty).is_some_and(|d| d.layout.sized);
        if sized {
            let bytes = if size {
                value::size_of(table, ty)
            } else {
                value::align_of(table, ty)
            };
            let Some(bytes) = bytes else {
                return Err(decline(
                    OpClass::Intrinsic,
                    "`size_of_val` of a sized type the table records no layout for",
                ));
            };
            return Ok(Outcome::Value(self.usize_literal(bytes)?));
        }
        match value::kind(table, table.peeled(ty)) {
            Some(TypeKind::Slice(element)) => {
                let element = *element;
                let Some(align) = value::align_of(table, element) else {
                    return Ok(self.unsupported("a slice element with no layout".to_owned()));
                };
                if !size {
                    return Ok(Outcome::Value(self.usize_literal(align)?));
                }
                let Some(stride) = value::size_of(table, element) else {
                    return Ok(self.unsupported("a slice element with no layout".to_owned()));
                };
                let len = ptr::slot_len(self.arg(0)?);
                Ok(Outcome::Value(match stride {
                    1 => len,
                    stride => jsast::binary(BinOp::Mul, len, jsast::num(stride as f64)),
                }))
            }
            // A `&str` / `*const str` is a JavaScript string whose size is its UTF-8 byte length;
            // a mutable `&mut str` / `*mut str` is a `{ buf, off, len }` byte slice whose size is
            // `.len`.
            Some(TypeKind::Str) => Ok(Outcome::Value(if size {
                if value::is_mut_str_ptr(table, self.arg_ty(0)?) {
                    self.cx.slice_len(self.arg(0)?)
                } else {
                    self.cx.str_len(self.arg(0)?)
                }
            } else {
                self.usize_literal(1)?
            })),
            Some(TypeKind::Dynamic { .. }) => {
                let slot = if size {
                    VTABLE_SIZE_SLOT
                } else {
                    VTABLE_ALIGN_SLOT
                };
                Ok(Outcome::Value(jsast::index(
                    super::ptr::meta(self.arg(0)?),
                    jsast::num(slot),
                )))
            }
            _ => match table.unsized_tail(ty) {
                Some(tail) => self.size_of_tail(ty, tail.ty, size),
                None => Ok(self.unsupported(
                    "the size of this type is not known to rustc_codegen_js".to_owned(),
                )),
            },
        }
    }

    /// `size_of_val` / `align_of_val` of a struct with an unsized tail.
    fn size_of_tail(&self, ty: TypeId, tail: TypeId, size: bool) -> Res<Outcome> {
        let table = self.cx.table;
        let Some(align) = value::align_of(table, ty) else {
            return Err(decline(
                OpClass::Intrinsic,
                "`align_of_val` of a struct with an unsized tail the table records no layout for",
            ));
        };
        if !size {
            return Ok(Outcome::Value(self.usize_literal(align)?));
        }
        let Some(TypeKind::Slice(element)) = value::kind(table, table.peeled(tail)) else {
            return Ok(self.unsupported(
                "the size of a struct whose tail is not a slice is not known to rustc_codegen_js"
                    .to_owned(),
            ));
        };
        // The sized prefix ends where the tail begins, which is the layout's size for an unsized
        // struct (`LayoutFacts::unsized_prefix`).
        let prefix = table
            .get(ty)
            .and_then(|descriptor| descriptor.layout.unsized_prefix);
        let (Some(prefix), Some(stride)) = (prefix, value::size_of(table, *element)) else {
            return Err(decline(
                OpClass::Intrinsic,
                "`size_of_val` of a struct with an unsized tail, whose sized prefix the type \
                 table records no size for",
            ));
        };
        let prefix = self.usize_literal(prefix)?;
        let meta = super::ptr::meta(self.arg(0)?);
        let tail = match stride {
            1 => meta,
            stride => jsast::binary(BinOp::Mul, meta, jsast::num(stride as f64)),
        };
        Ok(Outcome::Value(jsast::binary(BinOp::Add, prefix, tail)))
    }

    // -----------------------------------------------------------------------------------------
    // Control and compiler hints
    // -----------------------------------------------------------------------------------------

    fn control(&self, name: &str) -> Res<Option<Outcome>> {
        Ok(Some(match name {
            "abort" => Outcome::Done(vec![jsast::expr_stmt(jsast::rt_call(
                "js_abort",
                vec![jsast::string("rustc_codegen_js: `abort` intrinsic")],
            ))]),
            "unreachable" => Outcome::Done(vec![jsast::throw_abort(
                "rustc_codegen_js: entered unreachable code",
            )]),
            "breakpoint" => Outcome::Done(vec![jsast::comment("`breakpoint` intrinsic")]),
            "cold_path"
            | "assume"
            | "prefetch_read_data"
            | "prefetch_write_data"
            | "prefetch_read_instruction"
            | "prefetch_write_instruction" => Outcome::Done(Vec::new()),
            "likely" | "unlikely" | "black_box" | "rustc_peek" => {
                if self.result_ty.is_some_and(|ty| self.cx.is_i64_ty(ty)) {
                    let arg = self.args.first().copied().ok_or_else(|| {
                        decline(OpClass::Intrinsic, "intrinsic expected an argument")
                    })?;
                    Outcome::Pair(self.cx.pair_of(arg)?)
                } else {
                    Outcome::Value(self.arg(0)?)
                }
            }
            "select_unpredictable" => {
                if self.result_ty.is_some_and(|ty| self.cx.is_i64_ty(ty)) {
                    let cond_id = *self.args.first().ok_or_else(|| {
                        decline(OpClass::Intrinsic, "intrinsic expected cond argument")
                    })?;
                    let t_id = *self.args.get(1).ok_or_else(|| {
                        decline(OpClass::Intrinsic, "intrinsic expected true argument")
                    })?;
                    let f_id = *self.args.get(2).ok_or_else(|| {
                        decline(OpClass::Intrinsic, "intrinsic expected false argument")
                    })?;
                    let cond = self.temp(self.cx.expr_of(cond_id)?);
                    let (tlo, thi) = self.cx.pair_of(t_id)?;
                    let (flo, fhi) = self.cx.pair_of(f_id)?;
                    Outcome::Pair((
                        jsast::cond(cond.clone(), tlo, flo),
                        jsast::cond(cond, thi, fhi),
                    ))
                } else {
                    Outcome::Value(jsast::cond(self.arg(0)?, self.arg(1)?, self.arg(2)?))
                }
            }
            "is_val_statically_known" | "ub_checks" | "contract_checks" => {
                Outcome::Value(jsast::boolean(false))
            }
            // The session's `-Coverflow-checks` is not a link-time fact; the suite compiles with
            // them off, which is the same assumption `emit.rs`'s optional overflow asserts make.
            "overflow_checks" => Outcome::Value(jsast::boolean(false)),
            // Dropping a value without running its destructor is a no-op under a collector.
            "forget" => Outcome::Value(jsast::undefined()),
            // The validity queries are `tcx`'s; only the one this schema can answer is answered.
            "assert_inhabited" => {
                let ty = self.type_at(0)?;
                if is_uninhabited(self.cx, ty) {
                    Outcome::Done(vec![jsast::expr_stmt(jsast::rt_call(
                        "js_abort",
                        vec![jsast::string("attempted to instantiate uninhabited type")],
                    ))])
                } else {
                    Outcome::Done(Vec::new())
                }
            }
            "assert_zero_valid" | "assert_mem_uninitialized_valid" => {
                return Err(decline(
                    OpClass::Intrinsic,
                    format!("`{name}` needs rustc's validity query"),
                ))
            }
            "caller_location" => Outcome::Value(self.caller_location()?),
            // `const_eval_select(args, in_const, at_runtime)`. This is codegen, so it is always
            // the runtime arm, called with its argument tuple splatted the way `"rust-call"` does.
            "const_eval_select" => self.const_eval_select()?,
            // Under `-Cpanic=abort` nothing can unwind, so this is the call plus a zero code.
            "catch_unwind" => {
                let call = jsast::call(self.arg(0)?, vec![self.arg(1)?]);
                self.prelude.borrow_mut().push(jsast::expr_stmt(call));
                Outcome::Value(jsast::num(0))
            }
            "return_address" => self.unsupported(
                "`return_address` is not supported by rustc_codegen_js: a JavaScript call stack \
                 has no addresses"
                    .to_owned(),
            ),
            _ => return Ok(None),
        }))
    }

    /// `const_eval_select(args, in_const, at_runtime)`, always the runtime arm.
    fn const_eval_select(&self) -> Res<Outcome> {
        let table = self.cx.table;
        let runtime_ty = self.arg_ty(2)?;
        let Some(TypeKind::FnDef {
            instance: Some(instance),
            ..
        }) = value::kind(table, table.peeled(runtime_ty))
        else {
            return Ok(self.unsupported(
                "the runtime argument of `const_eval_select` is not a resolved function item"
                    .to_owned(),
            ));
        };
        let name = self
            .cx
            .interns
            .str(instance.link_name)
            .unwrap_or("")
            .to_owned();
        let pack_ty = self.arg_ty(0)?;
        let Some(TypeKind::Tuple(elements)) = value::kind(table, table.peeled(pack_ty)) else {
            return Ok(self.unsupported(
                "the argument pack of `const_eval_select` is not a tuple".to_owned(),
            ));
        };
        let pack = self.arg(0)?;
        let arguments = elements
            .iter()
            .enumerate()
            .map(|(index, element)| {
                let read = jsast::index(pack.clone(), jsast::num(index as f64));
                if table.needs_clone(*element) {
                    value::clone_expr_with(
                        table,
                        self.cx.interns,
                        *element,
                        read,
                        self.cx.nested_clone,
                    )
                } else {
                    read
                }
            })
            .collect();
        Ok(Outcome::Value(jsast::call(jsast::id(name), arguments)))
    }

    /// The caller's `Location`: a concrete MIR-resolved call site when `track_caller` is `Some`,
    /// this body's own `$loc` inside a `#[track_caller]` function otherwise, or the call's own
    /// origin source fact as fallback.
    fn caller_location(&self) -> Res<Expr> {
        if let Some(fact_id) = self.track_caller {
            let Some(fact) = self.cx.interns.source_fact(fact_id) else {
                return Err(decline(
                    OpClass::Intrinsic,
                    "`caller_location` carries a source fact missing from interns",
                ));
            };
            return self.cx.caller_location_value(fact);
        }
        if self.cx.func.sig.abi.caller_location {
            return Ok(jsast::id(super::CALLER_LOCATION_PARAM));
        }
        let Some(fact) = self.cx.current_fact() else {
            return Err(decline(
                OpClass::Intrinsic,
                "`caller_location` outside a `#[track_caller]` body needs the call's source fact",
            ));
        };
        self.cx.caller_location_value(fact)
    }

    // -----------------------------------------------------------------------------------------
    // Integers
    // -----------------------------------------------------------------------------------------

    fn integer(&self, name: &str) -> Res<Option<Outcome>> {
        let bit_op = matches!(
            name,
            "ctpop" | "ctlz" | "ctlz_nonzero" | "cttz" | "cttz_nonzero" | "bswap" | "bitreverse"
        );
        let arithmetic = matches!(
            name,
            "wrapping_add"
                | "wrapping_sub"
                | "wrapping_mul"
                | "unchecked_add"
                | "unchecked_sub"
                | "unchecked_mul"
                | "unchecked_div"
                | "unchecked_rem"
                | "unchecked_shl"
                | "unchecked_shr"
                | "exact_div"
                | "disjoint_bitor"
                | "saturating_add"
                | "saturating_sub"
                | "add_with_overflow"
                | "sub_with_overflow"
                | "mul_with_overflow"
                | "rotate_left"
                | "rotate_right"
        );
        if !bit_op && !arithmetic && name != "three_way_compare" {
            return Ok(None);
        }
        let table = self.cx.table;
        let operand_ty = self.arg_ty(0)?;
        if super::i64_pair::is_i64(table, operand_ty) {
            return self.integer_i64_pair(name, operand_ty).map(Some);
        }
        if name == "three_way_compare" {
            return self.three_way_compare(operand_ty).map(Some);
        }
        // `disjoint_bitor` is defined on `bool` too, and there it is the ordinary `||`.
        if name == "disjoint_bitor" && value::is_bool(table, operand_ty) {
            return Ok(Some(Outcome::Value(jsast::binary(
                BinOp::Or,
                self.arg(0)?,
                self.arg(1)?,
            ))));
        }
        let Some((signed, bits)) = table.int_info(operand_ty) else {
            return Ok(Some(self.unsupported(format!(
                "`{name}` on a non-integer is not supported by rustc_codegen_js"
            ))));
        };
        let int = Int {
            repr: table.int_repr(operand_ty),
            signed,
            bits,
        };
        Ok(Some(if bit_op {
            self.bit_op(name, int)?
        } else {
            self.arithmetic(name, int)?
        }))
    }

    fn pair_arg(&self, index: usize) -> Res<(Expr, Expr)> {
        let arg = *self.args.get(index).ok_or_else(|| {
            decline(
                OpClass::Intrinsic,
                format!("intrinsic expected argument {index}"),
            )
        })?;
        self.cx.pair_of(arg)
    }

    fn shift_amount_i64_pair(&self, index: usize) -> Res<Expr> {
        let arg = *self.args.get(index).ok_or_else(|| {
            decline(
                OpClass::Intrinsic,
                format!("intrinsic expected argument {index}"),
            )
        })?;
        let ty = self.cx.ty_of(arg)?;
        if let Some(k) = self.cx.const_scalar_i32(arg) {
            return Ok(jsast::num(f64::from(k)));
        }
        if self.cx.is_i64_ty(ty) {
            return Ok(self.cx.pair_of(arg)?.0);
        }
        let expr = self.arg(index)?;
        if self.cx.table.int_repr(ty) == IntRepr::BigInt {
            Ok(jsast::to_number(jsast::bigint_mask(false, 32, expr)))
        } else {
            Ok(expr)
        }
    }

    fn integer_i64_pair(&self, name: &str, operand_ty: TypeId) -> Res<Outcome> {
        let signed = super::i64_pair::i64_signedness(self.cx.table, operand_ty).unwrap_or(false);
        let a = self.pair_arg(0)?;
        let mut stmts = self.prelude.borrow_mut();
        Ok(match name {
            "ctpop" | "ctlz" | "ctlz_nonzero" | "cttz" | "cttz_nonzero" => Outcome::Value(
                super::i64_pair::bit_count_intrinsic(self.cx, &mut stmts, name, a),
            ),
            "bswap" | "bitreverse" => Outcome::Pair(super::i64_pair::bit_permute_pair(
                self.cx, &mut stmts, name, a,
            )),
            "three_way_compare" => {
                let table = self.cx.table;
                let ordering_ty = self.ret_ty()?;
                let variants = value::enum_variants(table, ordering_ty);
                let variant = |wanted: &str| -> Option<Expr> {
                    let index = variants.iter().position(|variant| {
                        value::variant_name(self.cx.interns, variant) == wanted
                    })?;
                    value::enum_value(
                        table,
                        self.cx.interns,
                        ordering_ty,
                        index as u32,
                        Vec::new(),
                    )
                };
                let (Some(less), Some(equal), Some(greater)) =
                    (variant("Less"), variant("Equal"), variant("Greater"))
                else {
                    return Ok(self.unsupported(
                        "`three_way_compare` did not return an `Ordering`".to_owned(),
                    ));
                };
                let b = self.pair_arg(1)?;
                Outcome::Value(super::i64_pair::three_way_ordering(
                    self.cx,
                    &mut stmts,
                    signed,
                    a,
                    b,
                    [less, equal, greater],
                ))
            }
            "wrapping_add" | "unchecked_add" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::add_pair(self.cx, &mut stmts, a, b))
            }
            "wrapping_sub" | "unchecked_sub" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::sub_pair(self.cx, &mut stmts, a, b))
            }
            "wrapping_mul" | "unchecked_mul" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::mul_pair(self.cx, &mut stmts, a, b))
            }
            "unchecked_div" | "exact_div" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::div_rem_pair(
                    self.cx, &mut stmts, signed, false, a, b,
                ))
            }
            "unchecked_rem" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::div_rem_pair(
                    self.cx, &mut stmts, signed, true, a, b,
                ))
            }
            "disjoint_bitor" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::bitwise_pair(BinOp::BitOr, a, b))
            }
            "unchecked_shl" => {
                drop(stmts);
                let amt = self.shift_amount_i64_pair(1)?;
                let mut stmts = self.prelude.borrow_mut();
                Outcome::Pair(super::i64_pair::shl_pair(self.cx, &mut stmts, a, amt))
            }
            "unchecked_shr" => {
                drop(stmts);
                let amt = self.shift_amount_i64_pair(1)?;
                let mut stmts = self.prelude.borrow_mut();
                Outcome::Pair(super::i64_pair::shr_pair(
                    self.cx, &mut stmts, signed, a, amt,
                ))
            }
            "rotate_left" | "rotate_right" => {
                drop(stmts);
                let amt = self.shift_amount_i64_pair(1)?;
                let mut stmts = self.prelude.borrow_mut();
                Outcome::Pair(super::i64_pair::rotate_pair(
                    self.cx,
                    &mut stmts,
                    name == "rotate_left",
                    a,
                    amt,
                ))
            }
            "saturating_add" | "saturating_sub" => {
                let b = self.pair_arg(1)?;
                Outcome::Pair(super::i64_pair::saturating_pair(
                    self.cx,
                    &mut stmts,
                    name == "saturating_add",
                    signed,
                    a,
                    b,
                ))
            }
            "add_with_overflow" | "sub_with_overflow" | "mul_with_overflow" => {
                let b = self.pair_arg(1)?;
                let bin_op = match name {
                    "add_with_overflow" => BinaryOp::AddWithOverflow,
                    "sub_with_overflow" => BinaryOp::SubWithOverflow,
                    _ => BinaryOp::MulWithOverflow,
                };
                let (lo, hi, ovf) =
                    super::i64_pair::with_overflow_pair(self.cx, &mut stmts, bin_op, signed, a, b);
                let lo = super::i64_pair::materialize(self.cx, &mut stmts, lo);
                let hi = super::i64_pair::materialize(self.cx, &mut stmts, hi);
                Outcome::Value(jsast::array(vec![
                    super::i64_pair::pack_bigint(signed, lo, hi),
                    ovf,
                ]))
            }
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_js")),
        })
    }

    fn arithmetic(&self, name: &str, int: Int) -> Res<Outcome> {
        let lhs = self.arg(0)?;
        let rhs = self.arg(1)?;
        let int32_only =
            self.int32_only && int.repr == IntRepr::Number && !int.signed && int.bits == 32;
        let truncate = |value: Expr| {
            if int32_only {
                value::simplify_i32_wrap(value)
            } else {
                value::mask(int.repr, int.signed, int.bits, value)
            }
        };
        let strip32 = |e: Expr| {
            if int.repr == IntRepr::Number && int.bits == 32 {
                value::strip_32bit_wrap(e)
            } else {
                e
            }
        };
        Ok(match name {
            "wrapping_add" | "unchecked_add" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Add, lhs, rhs)))
            }
            "wrapping_sub" | "unchecked_sub" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Sub, lhs, rhs)))
            }
            "wrapping_mul" | "unchecked_mul" => {
                Outcome::Value(truncate(int.multiply(strip32(lhs), strip32(rhs))))
            }
            "unchecked_div" | "exact_div" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Div, lhs, rhs)))
            }
            "unchecked_rem" => Outcome::Value(truncate(jsast::binary(BinOp::Rem, lhs, rhs))),
            "disjoint_bitor" => Outcome::Value(truncate(jsast::binary(
                BinOp::BitOr,
                strip32(lhs),
                strip32(rhs),
            ))),
            "unchecked_shl" => {
                let amount = self.shift_amount(int, rhs)?;
                Outcome::Value(truncate(jsast::binary(
                    BinOp::Shl,
                    strip32(lhs),
                    strip32(amount),
                )))
            }
            "unchecked_shr" => {
                let amount = self.shift_amount(int, rhs)?;
                Outcome::Value(truncate(int.shift_right(strip32(lhs), strip32(amount))))
            }
            "saturating_add" | "saturating_sub" => {
                self.saturating(name == "saturating_add", int)?
            }
            "add_with_overflow" | "sub_with_overflow" | "mul_with_overflow" => {
                self.with_overflow(name, int)?
            }
            "rotate_left" | "rotate_right" => self.rotate(name == "rotate_left", int)?,
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_js")),
        })
    }

    /// `saturating_add` / `saturating_sub`: the exact result, clamped to the type's range.
    fn saturating(&self, add: bool, int: Int) -> Res<Outcome> {
        let op = if add { BinOp::Add } else { BinOp::Sub };
        let sum = self.temp(jsast::binary(op, self.arg(0)?, self.arg(1)?));
        let (min, max) = int.range();
        Ok(Outcome::Value(jsast::cond(
            jsast::binary(BinOp::Gt, sum.clone(), max.clone()),
            max,
            jsast::cond(jsast::binary(BinOp::Lt, sum.clone(), min.clone()), min, sum),
        )))
    }

    /// `add_with_overflow` and its siblings: the wrapped result paired with whether it wrapped.
    ///
    /// A 32 bit product is not exact in a double, so its wrapped half is `Math.imul` over the
    /// (captured) operands; the flag still compares against the rounded product, which lands
    /// outside the type's range exactly when the true product does.
    fn with_overflow(&self, name: &str, int: Int) -> Res<Outcome> {
        let lhs = self.arg(0)?;
        let rhs = self.arg(1)?;
        let product = name == "mul_with_overflow";
        let op = match name {
            "add_with_overflow" => BinOp::Add,
            "sub_with_overflow" => BinOp::Sub,
            _ => BinOp::Mul,
        };
        let exact = self.temp(jsast::binary(op, lhs.clone(), rhs.clone()));
        let wrapping = if product && int.repr == IntRepr::Number && int.bits == 32 {
            int.multiply(lhs, rhs)
        } else {
            exact.clone()
        };
        let wrapped = self.temp(value::mask(int.repr, int.signed, int.bits, wrapping));
        Ok(Outcome::Value(jsast::array(vec![
            wrapped.clone(),
            jsast::binary(BinOp::StrictNe, wrapped, exact),
        ])))
    }

    /// `rotate_left` / `rotate_right`, as the pair of shifts JavaScript spells directly.
    ///
    /// A constant amount of a BigInt rotate is reduced here when the declaration hoists its BigInt
    /// constants: `amount & (bits - 1)` is then a number literal, which needs no temporary and
    /// widens to a BigInt constant (`JsCx::to_bigint`) rather than a `BigInt(..)` call per use.
    fn rotate(&self, left: bool, int: Int) -> Res<Outcome> {
        let int32_only =
            self.int32_only && int.repr == IntRepr::Number && !int.signed && int.bits == 32;
        if int.repr == IntRepr::Number && int.bits == 32 {
            let raw = value::strip_32bit_wrap(self.arg(0)?);
            let value = if matches!(raw, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_)) {
                raw
            } else {
                self.temp(raw)
            };
            let expr = if let Some(shift) = self.constant_u32(1) {
                let k = shift & 31;
                if k == 0 {
                    value
                } else {
                    let (up, down) = if left { (k, 32 - k) } else { (32 - k, k) };
                    jsast::binary(
                        BinOp::BitOr,
                        jsast::binary(BinOp::Shl, value.clone(), jsast::num(up)),
                        jsast::binary(BinOp::UShr, value, jsast::num(down)),
                    )
                }
            } else {
                let amount = self.temp(jsast::binary(
                    BinOp::BitAnd,
                    value::strip_32bit_wrap(self.arg(1)?),
                    jsast::num(31.0),
                ));
                let (up, down) = if left {
                    (
                        amount.clone(),
                        jsast::binary(BinOp::Sub, jsast::num(32.0), amount),
                    )
                } else {
                    (
                        jsast::binary(BinOp::Sub, jsast::num(32.0), amount.clone()),
                        amount,
                    )
                };
                jsast::binary(
                    BinOp::BitOr,
                    jsast::binary(BinOp::Shl, value.clone(), up),
                    jsast::binary(BinOp::UShr, value, down),
                )
            };
            return Ok(Outcome::Value(if int32_only {
                value::simplify_i32_wrap(expr)
            } else {
                value::mask(int.repr, int.signed, int.bits, expr)
            }));
        }
        let value = self.temp(int.zero_extend(self.arg(0)?));
        let constant = match int.repr {
            IntRepr::BigInt if self.cx.hoists_bigints() => self.constant_u32(1),
            _ => None,
        };
        let amount = match constant {
            Some(shift) => jsast::num(shift & (int.bits - 1)),
            None => self.temp(jsast::binary(
                BinOp::BitAnd,
                self.arg(1)?,
                jsast::num((int.bits - 1) as f64),
            )),
        };
        let amount = match int.repr {
            IntRepr::BigInt => self.cx.to_bigint(amount),
            IntRepr::Number => amount,
        };
        let width = int.literal(i128::from(int.bits));
        let (up, down) = if left {
            (amount.clone(), jsast::binary(BinOp::Sub, width, amount))
        } else {
            (jsast::binary(BinOp::Sub, width, amount.clone()), amount)
        };
        Ok(Outcome::Value(value::mask(
            int.repr,
            int.signed,
            int.bits,
            jsast::binary(
                BinOp::BitOr,
                jsast::binary(BinOp::Shl, value.clone(), up),
                int.unsigned().shift_right(value, down),
            ),
        )))
    }

    /// `three_way_compare`, whose result is a `core::cmp::Ordering`.
    fn three_way_compare(&self, operand_ty: TypeId) -> Res<Outcome> {
        let table = self.cx.table;
        let ordering_ty = self.ret_ty()?;
        let variants = value::enum_variants(table, ordering_ty);
        let variant = |wanted: &str| -> Option<Expr> {
            let index = variants
                .iter()
                .position(|variant| value::variant_name(self.cx.interns, variant) == wanted)?;
            value::enum_value(
                table,
                self.cx.interns,
                ordering_ty,
                index as u32,
                Vec::new(),
            )
        };
        let (Some(less), Some(equal), Some(greater)) =
            (variant("Less"), variant("Equal"), variant("Greater"))
        else {
            return Ok(
                self.unsupported("`three_way_compare` did not return an `Ordering`".to_owned())
            );
        };
        if table.int_info(operand_ty).is_none()
            && !value::is_bool(table, operand_ty)
            && !value::is_char(table, operand_ty)
        {
            return Ok(self.unsupported(
                "`three_way_compare` on this type is not supported by rustc_codegen_js".to_owned(),
            ));
        }
        Ok(Outcome::Value(jsast::cond(
            jsast::binary(BinOp::Lt, self.arg(0)?, self.arg(1)?),
            less,
            jsast::cond(
                jsast::binary(BinOp::StrictEq, self.arg(0)?, self.arg(1)?),
                equal,
                greater,
            ),
        )))
    }

    fn bit_op(&self, name: &str, int: Int) -> Res<Outcome> {
        match int.repr {
            IntRepr::Number => self.bit_op_number(name, int),
            IntRepr::BigInt => self.bit_op_bigint(name, int),
        }
    }

    /// `ctpop`, `ctlz`, `cttz`, `bswap` and `bitreverse` on a number, as the 32 bit folklore.
    fn bit_op_number(&self, name: &str, int: Int) -> Res<Outcome> {
        let value = int.zero_extend(self.arg(0)?);
        let truncate = |value: Expr| value::mask(int.repr, int.signed, int.bits, value);
        let and = |lhs: Expr, rhs: u32| jsast::binary(BinOp::BitAnd, lhs, jsast::num(rhs as f64));
        let ushr = |lhs: Expr, rhs: u32| jsast::binary(BinOp::UShr, lhs, jsast::num(rhs as f64));
        let shl = |lhs: Expr, rhs: u32| jsast::binary(BinOp::Shl, lhs, jsast::num(rhs as f64));
        let or = |lhs: Expr, rhs: Expr| jsast::binary(BinOp::BitOr, lhs, rhs);
        let math = |method: &str, arg: Expr| {
            jsast::call(jsast::member(jsast::id("Math"), method), vec![arg])
        };
        Ok(match name {
            "ctlz" | "ctlz_nonzero" => {
                let leading = math("clz32", value);
                Outcome::Value(if int.bits == 32 {
                    leading
                } else {
                    jsast::binary(BinOp::Sub, leading, jsast::num((32 - int.bits) as f64))
                })
            }
            "cttz" | "cttz_nonzero" => {
                let value = self.temp(value);
                let lowest = jsast::binary(BinOp::BitAnd, value.clone(), jsast::neg(value.clone()));
                Outcome::Value(jsast::cond(
                    jsast::binary(BinOp::StrictEq, value, jsast::num(0)),
                    jsast::num(int.bits as f64),
                    jsast::binary(BinOp::Sub, jsast::num(31), math("clz32", lowest)),
                ))
            }
            "ctpop" => {
                let slot = self.temp(value);
                let set = |expr: Expr| jsast::assign_stmt(slot.clone(), expr);
                self.prelude.borrow_mut().extend([
                    set(jsast::binary(
                        BinOp::Sub,
                        slot.clone(),
                        and(ushr(slot.clone(), 1), 0x5555_5555),
                    )),
                    set(jsast::binary(
                        BinOp::Add,
                        and(slot.clone(), 0x3333_3333),
                        and(ushr(slot.clone(), 2), 0x3333_3333),
                    )),
                    set(and(
                        jsast::binary(BinOp::Add, slot.clone(), ushr(slot.clone(), 4)),
                        0x0f0f_0f0f,
                    )),
                ]);
                Outcome::Value(ushr(
                    jsast::imul(slot, jsast::num(f64::from(0x0101_0101u32))),
                    24,
                ))
            }
            "bswap" => match int.bits {
                8 => Outcome::Value(self.arg(0)?),
                16 => {
                    let value = self.temp(value);
                    Outcome::Value(truncate(or(ushr(value.clone(), 8), shl(value, 8))))
                }
                _ => {
                    let value = self.temp(value);
                    Outcome::Value(truncate(or(
                        or(
                            ushr(value.clone(), 24),
                            and(ushr(value.clone(), 8), 0x0000_ff00),
                        ),
                        or(and(shl(value.clone(), 8), 0x00ff_0000), shl(value, 24)),
                    )))
                }
            },
            "bitreverse" => {
                let slot = self.temp(value);
                let set = |expr: Expr| jsast::assign_stmt(slot.clone(), expr);
                let swap = |width: u32, pattern: u32| {
                    or(
                        and(ushr(slot.clone(), width), pattern),
                        shl(and(slot.clone(), pattern), width),
                    )
                };
                self.prelude.borrow_mut().extend([
                    set(swap(1, 0x5555_5555)),
                    set(swap(2, 0x3333_3333)),
                    set(swap(4, 0x0f0f_0f0f)),
                    set(swap(8, 0x00ff_00ff)),
                    set(or(ushr(slot.clone(), 16), shl(slot.clone(), 16))),
                ]);
                Outcome::Value(truncate(if int.bits == 32 {
                    slot
                } else {
                    ushr(slot, 32 - int.bits)
                }))
            }
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_js")),
        })
    }

    /// The same operations on a `BigInt`, as small loops.
    fn bit_op_bigint(&self, name: &str, int: Int) -> Res<Outcome> {
        let zero = jsast::bigint(0u32);
        let one = jsast::bigint(1u32);
        let value = self.temp(int.zero_extend(self.arg(0)?));
        let shift_down = |by: u32| {
            jsast::assign_stmt(
                value.clone(),
                jsast::binary(BinOp::Shr, value.clone(), jsast::bigint(by)),
            )
        };
        let step = |counter: &Expr, by: f64| {
            jsast::assign_stmt(
                counter.clone(),
                jsast::binary(BinOp::Add, counter.clone(), jsast::num(by)),
            )
        };
        Ok(match name {
            "ctpop" => {
                let count = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::while_(
                    jsast::binary(BinOp::StrictNe, value.clone(), zero),
                    vec![
                        jsast::assign_stmt(
                            count.clone(),
                            jsast::binary(
                                BinOp::Add,
                                count.clone(),
                                jsast::to_number(jsast::binary(BinOp::BitAnd, value.clone(), one)),
                            ),
                        ),
                        shift_down(1),
                    ],
                ));
                Outcome::Value(count)
            }
            "ctlz" | "ctlz_nonzero" => {
                let count = self.temp(jsast::num(int.bits as f64));
                self.prelude.borrow_mut().push(jsast::while_(
                    jsast::binary(BinOp::StrictNe, value.clone(), zero),
                    vec![step(&count, -1.0), shift_down(1)],
                ));
                Outcome::Value(count)
            }
            "cttz" | "cttz_nonzero" => {
                let count = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::if_else(
                    jsast::binary(BinOp::StrictEq, value.clone(), zero.clone()),
                    vec![jsast::assign_stmt(
                        count.clone(),
                        jsast::num(int.bits as f64),
                    )],
                    vec![jsast::while_(
                        jsast::binary(
                            BinOp::StrictEq,
                            jsast::binary(BinOp::BitAnd, value.clone(), one),
                            zero,
                        ),
                        vec![step(&count, 1.0), shift_down(1)],
                    )],
                ));
                Outcome::Value(count)
            }
            "bswap" | "bitreverse" => {
                let (width, chunk, chunks) = if name == "bswap" {
                    (8u32, jsast::bigint(255u32), int.bits / 8)
                } else {
                    (1u32, one, int.bits)
                };
                let out = self.temp(zero);
                let index = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::while_(
                    jsast::binary(BinOp::Lt, index.clone(), jsast::num(chunks as f64)),
                    vec![
                        jsast::assign_stmt(
                            out.clone(),
                            jsast::binary(
                                BinOp::BitOr,
                                jsast::binary(BinOp::Shl, out.clone(), jsast::bigint(width)),
                                jsast::binary(BinOp::BitAnd, value.clone(), chunk),
                            ),
                        ),
                        shift_down(width),
                        step(&index, 1.0),
                    ],
                ));
                Outcome::Value(value::mask(int.repr, int.signed, int.bits, out))
            }
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_js")),
        })
    }

    /// A shift amount, converted into the representation the shifted type uses.
    fn shift_amount(&self, int: Int, amount: Expr) -> Res<Expr> {
        let rhs_repr = self.cx.table.int_repr(self.arg_ty(1)?);
        Ok(match (int.repr, rhs_repr) {
            (IntRepr::BigInt, IntRepr::Number) => self.cx.to_bigint(amount),
            (IntRepr::Number, IntRepr::BigInt) => {
                jsast::to_number(jsast::bigint_mask(false, 32, amount))
            }
            _ => amount,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Floats
    // -----------------------------------------------------------------------------------------

    fn float(&self, name: &str) -> Res<Option<Outcome>> {
        if name == "float_to_int_unchecked" {
            return self.float_to_int().map(Some);
        }
        let base = name
            .strip_suffix("f32")
            .or_else(|| name.strip_suffix("f64"))
            .unwrap_or(name);
        let math = |method: &str, arity: usize| -> Res<Expr> {
            let args = (0..arity).map(|i| self.arg(i)).collect::<Res<Vec<_>>>()?;
            Ok(jsast::call(jsast::member(jsast::id("Math"), method), args))
        };
        let binary = |op: BinOp| -> Res<Expr> { Ok(jsast::binary(op, self.arg(0)?, self.arg(1)?)) };
        let value = match base {
            "sqrt" => math("sqrt", 1)?,
            "sin" => math("sin", 1)?,
            "cos" => math("cos", 1)?,
            "exp" => math("exp", 1)?,
            "log" => math("log", 1)?,
            "log2" => math("log2", 1)?,
            "log10" => math("log10", 1)?,
            "floor" => math("floor", 1)?,
            "ceil" => math("ceil", 1)?,
            "trunc" => math("trunc", 1)?,
            "fabs" => math("abs", 1)?,
            "pow" | "powi" => math("pow", 2)?,
            "exp2" => jsast::call(
                jsast::member(jsast::id("Math"), "pow"),
                vec![jsast::num(2), self.arg(0)?],
            ),
            "minimum" => math("min", 2)?,
            "maximum" => math("max", 2)?,
            "minimum_number_nsz_" | "maximum_number_nsz_" => {
                let (x, y) = (self.arg(0)?, self.arg(1)?);
                let take_y = if base.starts_with("minimum") {
                    jsast::binary(BinOp::Le, y.clone(), x.clone())
                } else {
                    jsast::binary(BinOp::Ge, y.clone(), x.clone())
                };
                let x_is_nan = jsast::binary(BinOp::StrictNe, x.clone(), x.clone());
                jsast::cond(jsast::binary(BinOp::Or, x_is_nan, take_y), y, x)
            }
            // Rust rounds a tie away from zero; `Math.round` rounds it toward positive infinity.
            "round" => {
                let x = self.temp(self.arg(0)?);
                let round = |value: Expr| {
                    jsast::call(jsast::member(jsast::id("Math"), "round"), vec![value])
                };
                jsast::cond(
                    jsast::binary(BinOp::Lt, x.clone(), jsast::num(0)),
                    jsast::neg(round(jsast::neg(x.clone()))),
                    round(x),
                )
            }
            "round_ties_even_" => return self.round_ties_even().map(Some),
            "copysign" => {
                let (x, y) = (self.arg(0)?, self.arg(1)?);
                let magnitude = jsast::call(jsast::member(jsast::id("Math"), "abs"), vec![x]);
                let negative = jsast::binary(
                    BinOp::Or,
                    jsast::binary(BinOp::Lt, y.clone(), jsast::num(0)),
                    jsast::call(
                        jsast::member(jsast::id("Object"), "is"),
                        vec![y, jsast::num(-0.0)],
                    ),
                );
                jsast::cond(negative, jsast::neg(magnitude.clone()), magnitude)
            }
            "fma" | "fmuladd" => jsast::binary(
                BinOp::Add,
                jsast::binary(BinOp::Mul, self.arg(0)?, self.arg(1)?),
                self.arg(2)?,
            ),
            "fadd_fast" | "fadd_algebraic" => binary(BinOp::Add)?,
            "fsub_fast" | "fsub_algebraic" => binary(BinOp::Sub)?,
            "fmul_fast" | "fmul_algebraic" => binary(BinOp::Mul)?,
            "fdiv_fast" | "fdiv_algebraic" => binary(BinOp::Div)?,
            "frem_fast" | "frem_algebraic" => binary(BinOp::Rem)?,
            _ => return Ok(None),
        };
        let exact = matches!(
            base,
            "floor"
                | "ceil"
                | "trunc"
                | "round"
                | "fabs"
                | "copysign"
                | "minimum"
                | "maximum"
                | "minimum_number_nsz_"
                | "maximum_number_nsz_"
        );
        Ok(Some(Outcome::Value(if exact {
            value
        } else {
            self.round_to_f32(value)?
        })))
    }

    /// `float_to_int_unchecked`: the truncation toward zero, at the destination's width.
    fn float_to_int(&self) -> Res<Outcome> {
        let table = self.cx.table;
        let to_ty = self.ret_ty()?;
        let Some((signed, bits)) = table.int_info(to_ty) else {
            return Ok(self.unsupported(
                "`float_to_int_unchecked` to a non-integer is not supported by rustc_codegen_js"
                    .to_owned(),
            ));
        };
        let truncated = jsast::call(
            jsast::member(jsast::id("Math"), "trunc"),
            vec![self.arg(0)?],
        );
        if let Some(signed) = super::i64_pair::i64_signedness(table, to_ty) {
            let mut stmts = self.prelude.borrow_mut();
            return Ok(Outcome::Pair(super::i64_pair::float_to_i64_pair(
                self.cx, &mut stmts, signed, truncated,
            )));
        }
        let repr = table.int_repr(to_ty);
        let value = match repr {
            IntRepr::BigInt => jsast::to_bigint(truncated),
            IntRepr::Number => truncated,
        };
        Ok(Outcome::Value(value::mask(repr, signed, bits, value)))
    }

    /// `round_ties_even`, the rounding a hardware `rint` does in the default mode.
    fn round_ties_even(&self) -> Res<Outcome> {
        let x = self.temp(self.arg(0)?);
        let floor = self.temp(jsast::call(
            jsast::member(jsast::id("Math"), "floor"),
            vec![x.clone()],
        ));
        let fraction = jsast::binary(BinOp::Sub, x, floor.clone());
        let up = jsast::binary(BinOp::Add, floor.clone(), jsast::num(1));
        let tie = jsast::cond(
            jsast::binary(
                BinOp::StrictEq,
                jsast::binary(BinOp::Rem, floor.clone(), jsast::num(2)),
                jsast::num(0),
            ),
            floor.clone(),
            up.clone(),
        );
        let value = jsast::cond(
            jsast::binary(BinOp::Lt, fraction.clone(), jsast::num(0.5)),
            floor,
            jsast::cond(jsast::binary(BinOp::Gt, fraction, jsast::num(0.5)), up, tie),
        );
        Ok(Outcome::Value(self.round_to_f32(value)?))
    }

    /// Rounds to `f32` precision when that is what the result holds.
    fn round_to_f32(&self, value: Expr) -> Res<Expr> {
        let Some(result) = self.result_ty else {
            return Ok(value);
        };
        Ok(match value::float_bits(self.cx.table, result) {
            Some(32) => jsast::fround(value),
            _ => value,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Pointers and slices
    // -----------------------------------------------------------------------------------------

    fn pointer(&self, name: &str) -> Res<Option<Outcome>> {
        let cx = self.cx;
        Ok(Some(match name {
            "ptr_metadata" => Outcome::Value(cx.ptr_metadata(self.arg_ty(0)?, self.arg(0)?)?),
            "read_via_copy" | "volatile_load" | "unaligned_volatile_load" => {
                Outcome::Value(self.read_via_copy()?)
            }
            "write_via_move"
            | "volatile_store"
            | "unaligned_volatile_store"
            | "nontemporal_store" => {
                Outcome::Done(cx.deref_write(self.type_at(0)?, self.arg(0)?, self.arg(1)?)?)
            }
            "typed_swap_nonoverlapping" => self.typed_swap()?,
            "aggregate_raw_ptr" => self.aggregate_raw_ptr()?,
            "slice_get_unchecked" => self.slice_get_unchecked()?,
            "transmute" | "transmute_unchecked" => {
                Outcome::Value(cx.transmute(self.arg_ty(0)?, self.ret_ty()?, self.arg(0)?)?)
            }
            "offset" | "arith_offset" => match self.pointee_of(0)? {
                None => self.not_a_pointer(name),
                Some(pointee) => {
                    let (pointer, count) = (self.arg(0)?, self.arg(1)?);
                    Outcome::Value(if name == "offset" {
                        cx.ptr_add(pointee, pointer, count)
                    } else {
                        cx.ptr_wrapping_add(pointee, pointer, count)
                    })
                }
            },
            "ptr_offset_from" | "ptr_offset_from_unsigned" => match self.pointee_of(0)? {
                None => self.not_a_pointer(name),
                Some(pointee) => {
                    Outcome::Value(cx.ptr_offset_from(pointee, self.arg(0)?, self.arg(1)?))
                }
            },
            "copy" | "copy_nonoverlapping" => self.copy(name, name == "copy", false)?,
            "volatile_copy_memory" | "volatile_copy_nonoverlapping_memory" => {
                self.copy(name, name == "volatile_copy_memory", true)?
            }
            "write_bytes" | "volatile_set_memory" => match self.pointee_of(0)? {
                None => self.not_a_pointer(name),
                Some(pointee) => Outcome::Done(self.write_bytes(
                    pointee,
                    self.arg(0)?,
                    self.arg(1)?,
                    self.arg(2)?,
                )),
            },
            "compare_bytes" => Outcome::Value(jsast::rt_call(
                "compare_bytes",
                vec![self.arg(0)?, self.arg(1)?, self.arg(2)?],
            )),
            "raw_eq" => self.raw_eq()?,
            "ptr_mask" => self.unsupported(
                "`ptr_mask` is not supported by rustc_codegen_js: masking the bits of a pointer \
                 has no meaning in a model where a pointer is a buffer and a key rather than an \
                 address"
                    .to_owned(),
            ),
            _ => return Ok(None),
        }))
    }

    /// `read_via_copy(p)`: a **copy** of the pointee that shares nothing with it.
    fn read_via_copy(&self) -> Res<Expr> {
        let cx = self.cx;
        let pointee = self.type_at(0)?;
        let read = if self
            .args
            .first()
            .is_some_and(|v| cx.is_direct_ref_copy_cast(*v))
        {
            self.arg(0)?
        } else {
            cx.deref_read(pointee, self.arg(0)?)
        };
        Ok(if cx.table.needs_clone(pointee) {
            value::clone_expr_with(cx.table, cx.interns, pointee, read, cx.nested_clone)
        } else {
            read
        })
    }

    /// `typed_swap_nonoverlapping(x, y)`: exchange the two pointees.
    fn typed_swap(&self) -> Res<Outcome> {
        let cx = self.cx;
        let pointee = self.type_at(0)?;
        let x_direct = self
            .args
            .first()
            .is_some_and(|v| cx.is_direct_ref_copy_cast(*v));
        let y_direct = self
            .args
            .get(1)
            .is_some_and(|v| cx.is_direct_ref_copy_cast(*v));
        let (mut x, mut y) = (self.arg(0)?, self.arg(1)?);
        if x_direct && !matches!(x, Expr::Ident(_)) {
            x = self.temp(x);
        }
        if y_direct && !matches!(y, Expr::Ident(_)) {
            y = self.temp(y);
        }
        let read = if x_direct {
            x.clone()
        } else {
            cx.deref_read(pointee, x.clone())
        };
        let copy = if cx.table.needs_clone(pointee) {
            value::clone_expr_with(cx.table, cx.interns, pointee, read, cx.nested_clone)
        } else {
            read
        };
        let saved = self.temp(copy);
        let y_val = if y_direct {
            y.clone()
        } else {
            cx.deref_read(pointee, y.clone())
        };
        let mut statements = if x_direct {
            cx.write_indirect(pointee, x, y_val)?
        } else {
            cx.deref_write(pointee, x, y_val)?
        };
        statements.extend(if y_direct {
            cx.write_indirect(pointee, y, saved)?
        } else {
            cx.deref_write(pointee, y, saved)?
        });
        Ok(Outcome::Done(statements))
    }

    /// `copy` and `copy_nonoverlapping`, and their volatile spellings.
    fn copy(&self, name: &str, overlapping: bool, volatile: bool) -> Res<Outcome> {
        let (source, target) = if volatile { (1, 0) } else { (0, 1) };
        let Some(pointee) = self.pointee_of(source)? else {
            return Ok(self.not_a_pointer(name));
        };
        if !volatile
            && crate::lower::bstr::enabled()
            && value::is_u8(self.cx.table, pointee)
            && self
                .args
                .get(source)
                .is_some_and(|v| self.cx.byte_view(*v).is_some())
        {
            let src_val = self.args[source];
            let bv = self.cx.byte_view(src_val).unwrap();
            let base = self.cx.byte_view_base_expr(bv, src_val)?;
            let off = self.cx.byte_view_off_expr(src_val)?;
            let dst = self.arg(target)?;
            let count = self.arg(2)?;
            return Ok(Outcome::Done(vec![jsast::expr_stmt(jsast::rt_call(
                "bstr_copy_to",
                vec![base, off, dst, count],
            ))]));
        }
        let count_is_one = self.args.get(2).is_some_and(|v| {
            self.cx.def_of(*v).is_some_and(|op| {
                matches!(
                    op.kind,
                    OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar { bits: 1, .. })
                )
            })
        });
        if !volatile
            && count_is_one
            && self
                .args
                .get(source)
                .is_some_and(|v| self.cx.is_direct_ref_copy_cast(*v))
        {
            let src_ref = self.arg(source)?;
            let dst = self.arg(target)?;
            return Ok(Outcome::Done(
                self.cx.ptr_copy_one_from_ref(pointee, dst, src_ref)?,
            ));
        }
        let (dst, src) = (self.arg(target)?, self.arg(source)?);
        if !volatile
            && !overlapping
            && !count_is_one
            && self.can_inline_ta_copy_nonoverlapping(source, target, pointee)
        {
            return self.inline_ta_copy_nonoverlapping(dst, src);
        }
        Ok(Outcome::Done(self.cx.ptr_copy(
            pointee,
            dst,
            src,
            self.arg(2)?,
            overlapping,
            count_is_one,
        )))
    }

    /// Whether a non-overlapping, non-volatile `copy_nonoverlapping` on numeric scalar `pointee`
    /// can emit an inline Tier-A same-type `TypedArray` fast path (avoiding `{ buf, off }` slot
    /// allocations and `$rt_copy_nonoverlapping` dispatch on small copies).
    fn can_inline_ta_copy_nonoverlapping(
        &self,
        source: usize,
        target: usize,
        pointee: TypeId,
    ) -> bool {
        let cx = self.cx;
        let table = cx.table;
        let heap = cx.heap();
        if !(heap.typed_heap_enabled
            && !heap.non_rc_raw_alloc_reachable
            && !cx.byte_views.raw_byte_derefs
            && cx.byte_views.elide_scale_scalar)
        {
            return false;
        }
        if table.is_zst(pointee)
            || table.needs_clone(pointee)
            || !value::is_slot_pointee(table, pointee)
        {
            return false;
        }
        if table
            .get(pointee)
            .and_then(crate::lower::semantic_ops::vec::typed_array_kind)
            .is_none()
        {
            return false;
        }
        let (Ok(src_ty), Ok(dst_ty)) = (self.arg_ty(source), self.arg_ty(target)) else {
            return false;
        };
        table.pointer_form(src_ty) == Some(rcgjs_object::jir::types::PointerForm::Slot)
            && table.pointer_form(dst_ty) == Some(rcgjs_object::jir::types::PointerForm::Slot)
    }

    fn inline_ta_copy_nonoverlapping(&self, dst: Expr, src: Expr) -> Res<Outcome> {
        let const_count = self.constant_u32(2);
        if const_count == Some(0) {
            return Ok(Outcome::Done(Vec::new()));
        }
        let count = self.arg(2)?;
        let cx = self.cx;
        let is_zero = |e: &Expr| {
            matches!(e, Expr::Num(n) if *n == 0.0) || matches!(e, Expr::RawNum(s) if s == "0")
        };
        let is_atom = |e: &Expr| {
            matches!(
                e,
                Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_)
            )
        };
        let mut body = Vec::new();
        let extract_slot = |expr: Expr, body: &mut Vec<Stmt>| -> (Expr, Expr, Expr) {
            if let Some((b_raw, o_raw)) = unwrap_safe_slot(&expr) {
                let b = if matches!(b_raw, Expr::Ident(_)) {
                    b_raw
                } else {
                    let (bind, id) = cx.temp(b_raw);
                    body.push(bind);
                    id
                };
                let o = if is_atom(&o_raw) {
                    o_raw
                } else {
                    let (bind, id) = cx.temp(o_raw);
                    body.push(bind);
                    id
                };
                let fallback = ptr::slot(b.clone(), o.clone());
                (b, o, fallback)
            } else {
                let slot = if matches!(expr, Expr::Ident(_)) {
                    expr
                } else {
                    let (bind, id) = cx.temp(expr);
                    body.push(bind);
                    id
                };
                let (b_bind, b) = cx.temp(jsast::member(slot.clone(), "buf"));
                let (o_bind, o) = cx.temp(jsast::member(slot.clone(), "off"));
                body.push(b_bind);
                body.push(o_bind);
                (b, o, slot)
            }
        };

        let (db, doff, dst_fallback) = extract_slot(dst, &mut body);
        let (sb, soff, src_fallback) = extract_slot(src, &mut body);
        let n = if is_atom(&count) {
            count
        } else {
            self.temp(count)
        };

        let build_loop = || -> Vec<Stmt> {
            let (bind_i, idx) = cx.temp(jsast::num(0));
            let d_idx = if is_zero(&doff) {
                idx.clone()
            } else {
                jsast::binary(BinOp::Add, doff.clone(), idx.clone())
            };
            let s_idx = if is_zero(&soff) {
                idx.clone()
            } else {
                jsast::binary(BinOp::Add, soff.clone(), idx.clone())
            };
            vec![
                bind_i,
                jsast::while_(
                    jsast::binary(BinOp::Lt, idx.clone(), n.clone()),
                    vec![
                        jsast::assign_stmt(
                            jsast::index(db.clone(), d_idx),
                            jsast::index(sb.clone(), s_idx),
                        ),
                        jsast::expr_stmt(jsast::compound_assign(BinOp::Add, idx, jsast::num(1))),
                    ],
                ),
            ]
        };

        let build_bulk_set = || -> Stmt {
            let end = if is_zero(&soff) {
                n.clone()
            } else {
                jsast::binary(BinOp::Add, soff.clone(), n.clone())
            };
            let sub = jsast::method_call(sb.clone(), "subarray", vec![soff.clone(), end]);
            let src_view = if is_zero(&soff) {
                jsast::cond(
                    jsast::binary(
                        BinOp::StrictEq,
                        n.clone(),
                        jsast::member(sb.clone(), "length"),
                    ),
                    sb.clone(),
                    sub,
                )
            } else {
                sub
            };
            jsast::expr_stmt(jsast::method_call(
                db.clone(),
                "set",
                vec![src_view, doff.clone()],
            ))
        };

        let is_view = |buf: Expr| jsast::method_call(jsast::id("ArrayBuffer"), "isView", vec![buf]);
        let same_ta = jsast::binary(
            BinOp::And,
            jsast::binary(BinOp::And, is_view(db.clone()), is_view(sb.clone())),
            jsast::binary(
                BinOp::StrictEq,
                jsast::member(db.clone(), "constructor"),
                jsast::member(sb.clone(), "constructor"),
            ),
        );

        let fast_branch = match const_count {
            Some(c) if c <= 32 => build_loop(),
            Some(_) => vec![build_bulk_set()],
            None => vec![jsast::if_else(
                jsast::binary(BinOp::Le, n.clone(), jsast::num(32)),
                build_loop(),
                vec![build_bulk_set()],
            )],
        };
        let fallback_branch = vec![jsast::expr_stmt(jsast::rt_call(
            "copy_nonoverlapping",
            vec![dst_fallback, src_fallback, n.clone()],
        ))];
        body.push(jsast::if_else(same_ta, fast_branch, fallback_branch));

        if const_count.is_some() {
            Ok(Outcome::Done(body))
        } else {
            Ok(Outcome::Done(vec![jsast::if_(
                jsast::binary(BinOp::Gt, n, jsast::num(0)),
                body,
            )]))
        }
    }

    /// `write_bytes` (`ptr.rs::write_bytes`).
    fn write_bytes(&self, pointee: TypeId, dst: Expr, byte: Expr, count: Expr) -> Vec<Stmt> {
        let table = self.cx.table;
        if table.is_zst(pointee) {
            return Vec::new();
        }
        let peeled = table.peeled(pointee);
        let one_byte = value::size_of(table, peeled) == Some(1)
            && (table.int_info(peeled).is_some() || value::is_bool(table, peeled));
        let args = if one_byte {
            let value = if value::is_bool(table, peeled) {
                jsast::not(jsast::not(byte))
            } else {
                match table.int_info(peeled) {
                    Some((true, bits)) => value::mask(table.int_repr(peeled), true, bits, byte),
                    _ => byte,
                }
            };
            vec![dst, value, count]
        } else {
            let Some(zero) =
                value::zero_value_for_heap_plan(table, self.cx.interns, peeled, self.cx.heap())
            else {
                return vec![jsast::expr_stmt(self.cx.zombie(
                    "`write_bytes` is not supported by rustc_codegen_js for this pointee: a byte \
                     pattern names a value only for a one byte pointee, and zero for a scalar one"
                        .to_owned(),
                ))];
            };
            vec![dst, byte, count, zero]
        };
        vec![jsast::expr_stmt(jsast::rt_call("write_bytes", args))]
    }

    /// `raw_eq(a, b)`: whether the two pointees have the same bytes.
    fn raw_eq(&self) -> Res<Outcome> {
        let table = self.cx.table;
        let pointee = self.type_at(0)?;
        if table.is_zst(pointee) {
            return Ok(Outcome::Value(jsast::boolean(true)));
        }
        // A reference to an array is the array itself, and `every` names each element once.
        if let Some(TypeKind::Array { element, .. }) = value::kind(table, table.peeled(pointee)) {
            if value::compares_by_value(table, *element) {
                let rhs = self.temp(self.arg(1)?);
                let same = jsast::binary(
                    BinOp::StrictEq,
                    jsast::id(RAW_EQ_ELEMENT),
                    jsast::index(rhs, jsast::id(RAW_EQ_INDEX)),
                );
                return Ok(Outcome::Value(jsast::method_call(
                    self.arg(0)?,
                    "every",
                    vec![jsast::arrow(
                        vec![RAW_EQ_ELEMENT.to_owned(), RAW_EQ_INDEX.to_owned()],
                        same,
                    )],
                )));
            }
        }
        if !value::compares_by_value(table, pointee) {
            return Ok(self.unsupported(
                "`raw_eq` is not supported by rustc_codegen_js for this pointee: the bytes of a \
                 value are not observable in a model where a value is a JavaScript object"
                    .to_owned(),
            ));
        }
        let (lhs, rhs) = (self.temp(self.arg(0)?), self.temp(self.arg(1)?));
        Ok(Outcome::Value(jsast::binary(
            BinOp::StrictEq,
            ptr::slot_element(lhs),
            ptr::slot_element(rhs),
        )))
    }

    /// `aggregate_raw_ptr(data, meta)`: a pointer built from its two halves.
    fn aggregate_raw_ptr(&self) -> Res<Outcome> {
        let pointer_ty = self.ret_ty()?;
        let Some(pointee) = value::pointee_of(self.cx.table, pointer_ty) else {
            return Ok(self.unsupported("`aggregate_raw_ptr` did not return a pointer".to_owned()));
        };
        Ok(Outcome::Value(self.cx.build_fat(
            pointee,
            value::is_mut_str_ptr(self.cx.table, pointer_ty),
            Some(self.arg_ty(0)?),
            self.arg(0)?,
            self.arg(1)?,
        )?))
    }

    /// `slice_get_unchecked(slice, index)`: a pointer to one element of a slice.
    fn slice_get_unchecked(&self) -> Res<Outcome> {
        let table = self.cx.table;
        let slice_ty = self.arg_ty(0)?;
        let Some(pointee) = value::pointee_of(table, slice_ty) else {
            return Ok(self.unsupported("`slice_get_unchecked` on a non-pointer".to_owned()));
        };
        if !matches!(
            value::kind(table, table.peeled(pointee)),
            Some(TypeKind::Slice(_))
        ) {
            return Ok(self.unsupported(
                "`slice_get_unchecked` on a non-slice is not supported by rustc_codegen_js"
                    .to_owned(),
            ));
        }
        let slice = self.arg(0)?;
        let index = self.arg(1)?;
        let element = ptr::slot(
            ptr::slot_buf(slice.clone()),
            jsast::binary(BinOp::Add, ptr::slot_off(slice), index),
        );
        let ret_ty = self.ret_ty()?;
        Ok(Outcome::Value(if value::is_slot(table, ret_ty) {
            element
        } else {
            ptr::slot_element(element)
        }))
    }

    // -----------------------------------------------------------------------------------------
    // Atomics
    // -----------------------------------------------------------------------------------------

    /// Atomics, lowered as the plain operations they are on a single threaded runtime.
    fn atomic(&self, name: &str) -> Res<Option<Outcome>> {
        let Some(mut name) = name.strip_prefix("atomic_") else {
            return Ok(None);
        };
        while let Some(stripped) = ORDERINGS.iter().find_map(|ordering| {
            name.strip_suffix(ordering)
                .and_then(|rest| rest.strip_suffix('_'))
        }) {
            name = stripped;
        }
        if matches!(name, "fence" | "singlethreadfence") {
            return Ok(Some(Outcome::Done(Vec::new())));
        }
        let read_modify_write = matches!(
            name,
            "xadd" | "xsub" | "and" | "nand" | "or" | "xor" | "max" | "min" | "umax" | "umin"
        );
        if !read_modify_write && !matches!(name, "load" | "store" | "xchg" | "cxchg" | "cxchgweak")
        {
            return Ok(None);
        }
        let cx = self.cx;
        let pointee = self.type_at(0)?;
        let pointer = self.temp(self.arg(0)?);

        if read_modify_write {
            let old = self.temp(cx.deref_read(pointee, pointer.clone()));
            let updated = match self.atomic_update(name, pointee, old.clone())? {
                Ok(updated) => updated,
                Err(outcome) => return Ok(Some(outcome)),
            };
            self.prelude
                .borrow_mut()
                .extend(cx.deref_write(pointee, pointer, updated)?);
            return Ok(Some(Outcome::Value(old)));
        }

        Ok(Some(match name {
            "load" => Outcome::Value(cx.deref_read(pointee, pointer)),
            "store" => Outcome::Done(cx.deref_write(pointee, pointer, self.arg(1)?)?),
            "xchg" => {
                let old = self.temp(cx.deref_read(pointee, pointer.clone()));
                self.prelude
                    .borrow_mut()
                    .extend(cx.deref_write(pointee, pointer, self.arg(1)?)?);
                Outcome::Value(old)
            }
            // Returns `(old, succeeded)`, which is a two element JS array.
            _ => {
                let old = self.temp(cx.deref_read(pointee, pointer.clone()));
                let matched = if value::is_raw(cx.table, pointee) {
                    cx.ptr_eq(pointee, old.clone(), self.arg(1)?)
                } else if value::compares_by_value(cx.table, pointee) {
                    jsast::binary(BinOp::StrictEq, old.clone(), self.arg(1)?)
                } else {
                    return Ok(Some(
                        self.unsupported(
                            "an atomic compare and exchange on this type is not supported by \
                         rustc_codegen_js: values of that type have no atomic equality"
                                .to_owned(),
                        ),
                    ));
                };
                let matched = self.temp(matched);
                let write = cx.deref_write(pointee, pointer, self.arg(2)?)?;
                self.prelude
                    .borrow_mut()
                    .push(jsast::if_(matched.clone(), write));
                Outcome::Value(jsast::array(vec![old, matched]))
            }
        }))
    }

    /// The new value a read-modify-write atomic stores, given the old one.
    fn atomic_update(&self, name: &str, pointee: TypeId, old: Expr) -> Res<Result<Expr, Outcome>> {
        let table = self.cx.table;
        let operand = self.arg(1)?;
        if matches!(name, "max" | "min" | "umax" | "umin") {
            if !value::compares_by_value(table, pointee) {
                return Ok(Err(self.unsupported(format!(
                    "an atomic `{name}` on this type is not supported by rustc_codegen_js"
                ))));
            }
            let take_operand = if name.ends_with("max") {
                jsast::binary(BinOp::Gt, operand.clone(), old.clone())
            } else {
                jsast::binary(BinOp::Lt, operand.clone(), old.clone())
            };
            return Ok(Ok(jsast::cond(take_operand, operand, old)));
        }
        let Some((signed, bits)) = table.int_info(pointee) else {
            return Ok(Err(self.unsupported(format!(
                "an atomic `{name}` on this type is not supported by rustc_codegen_js"
            ))));
        };
        let repr = table.int_repr(pointee);
        let combined = match name {
            "xadd" => jsast::binary(BinOp::Add, old, operand),
            "xsub" => jsast::binary(BinOp::Sub, old, operand),
            "and" => jsast::binary(BinOp::BitAnd, old, operand),
            "or" => jsast::binary(BinOp::BitOr, old, operand),
            "xor" => jsast::binary(BinOp::BitXor, old, operand),
            _ => jsast::bit_not(jsast::binary(BinOp::BitAnd, old, operand)),
        };
        Ok(Ok(value::mask(repr, signed, bits, combined)))
    }

    // -----------------------------------------------------------------------------------------
    // Plumbing
    // -----------------------------------------------------------------------------------------

    /// One argument.
    fn arg(&self, index: usize) -> Res<Expr> {
        if let Some(value) = self.captured.as_ref().and_then(|args| args.get(index)) {
            return Ok(value.clone());
        }
        match self.args.get(index) {
            Some(arg) => self.cx.expr_of(*arg),
            None => Ok(self.cx.zombie(format!(
                "an intrinsic was called with {} arguments, but needs at least {}",
                self.args.len(),
                index + 1
            ))),
        }
    }

    /// The `u32` an argument holds when it is a constant. The plan names every operand of a
    /// materializing intrinsic, a constant included (`fuse.rs`), so the value is read off the
    /// operand's own definition rather than off its spelling.
    fn constant_u32(&self, index: usize) -> Option<u32> {
        let mut cur = *self.args.get(index)?;
        for _ in 0..8 {
            let op = self.cx.def_of(cur)?;
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }) => {
                    return u32::try_from(*bits).ok();
                }
                OpKind::Copy(src) | OpKind::Move(src) => cur = *src,
                _ => return None,
            }
        }
        None
    }

    /// The type of one argument.
    fn arg_ty(&self, index: usize) -> Res<TypeId> {
        match self.args.get(index) {
            Some(arg) => self.cx.ty_of(*arg),
            None => Err(decline(
                OpClass::Intrinsic,
                "an intrinsic read the type of an argument it was not given",
            )),
        }
    }

    /// One of the intrinsic's generic type arguments.
    fn type_at(&self, index: usize) -> Res<TypeId> {
        self.types.get(index).copied().ok_or_else(|| {
            decline(
                OpClass::Intrinsic,
                "an intrinsic read a type argument it was not instantiated with",
            )
        })
    }

    /// The type the result is bound at.
    fn ret_ty(&self) -> Res<TypeId> {
        self.result_ty.ok_or_else(|| {
            decline(
                OpClass::Intrinsic,
                "an intrinsic whose lowering needs its result type has no result",
            )
        })
    }

    /// The pointee of an argument that is a pointer.
    fn pointee_of(&self, index: usize) -> Res<Option<TypeId>> {
        Ok(value::pointee_of(self.cx.table, self.arg_ty(index)?))
    }

    fn not_a_pointer(&self, name: &str) -> Outcome {
        self.unsupported(format!(
            "`{name}` was called on a value that is not a pointer"
        ))
    }

    /// Evaluates `value` into a hoisted temporary, and returns the expression that reads it.
    fn temp(&self, value: Expr) -> Expr {
        let (statement, name) = self.cx.temp(value);
        self.prelude.borrow_mut().push(statement);
        name
    }

    /// A `usize` literal, in whichever representation `usize` has on this target.
    fn usize_literal(&self, value: u64) -> Res<Expr> {
        let ty = self.ret_ty()?;
        Ok(value::int_literal(self.cx.table, ty, u128::from(value))
            .unwrap_or_else(|| jsast::num(value as f64)))
    }

    /// Records an intrinsic this backend cannot lower, exactly as v1 does: the zombie stands in
    /// for the value, so the call site is still a complete lowering.
    fn unsupported(&self, message: String) -> Outcome {
        Outcome::Value(self.cx.zombie(message))
    }
}

/// Extracts `(buf, off)` from a plain `{ buf, off }` slot literal (or a zero-offset guard
/// `c === 0 ? { buf, off: num } : { buf, off: ... }` whose `then` arm is already a numeric slot
/// literal, safe to evaluate even when `count === 0`).
fn unwrap_safe_slot(expr: &Expr) -> Option<(Expr, Expr)> {
    let fields = match expr {
        Expr::Object(fields) => fields,
        Expr::Cond(cond, then_expr, else_expr)
            if matches!(
                cond.as_ref(),
                Expr::Binary(BinOp::StrictEq, _, zero) if **zero == jsast::num(0)
            ) =>
        {
            let Expr::Object(then_fields) = then_expr.as_ref() else {
                return None;
            };
            if then_fields.len() != 2
                || !then_fields
                    .iter()
                    .any(|(k, v)| k == "off" && matches!(v, Expr::Num(_) | Expr::RawNum(_)))
            {
                return None;
            }
            match else_expr.as_ref() {
                Expr::Object(else_fields) => else_fields,
                _ => return None,
            }
        }
        _ => return None,
    };
    if fields.len() != 2 {
        return None;
    }
    let buf = fields
        .iter()
        .find(|(k, _)| k == "buf")
        .map(|(_, v)| v.clone())?;
    let off = fields
        .iter()
        .find(|(k, _)| k == "off")
        .map(|(_, v)| v.clone())?;
    Some((buf, off))
}

/// Whether a type has no values at all, as far as this schema can tell.
fn is_uninhabited(cx: &JsCx<'_>, ty: TypeId) -> bool {
    match value::kind(cx.table, cx.table.peeled(ty)) {
        Some(TypeKind::Never) => true,
        Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        }) => variants.is_empty(),
        _ => false,
    }
}

/// An integer type, reduced to what the arms above need to know about it.
#[derive(Clone, Copy)]
struct Int {
    repr: IntRepr,
    signed: bool,
    bits: u32,
}

impl Int {
    /// The same width, read as unsigned.
    fn unsigned(self) -> Int {
        Int {
            signed: false,
            ..self
        }
    }

    /// A literal in this representation.
    fn literal(self, value: i128) -> Expr {
        match self.repr {
            IntRepr::BigInt => jsast::bigint(value),
            IntRepr::Number => jsast::num(value as f64),
        }
    }

    /// The value's bit pattern, as a non-negative number.
    fn zero_extend(self, value: Expr) -> Expr {
        if !self.signed {
            return value;
        }
        value::mask(self.repr, false, self.bits, value)
    }

    /// `lhs * rhs`, exactly. A 32 bit product needs `Math.imul`.
    fn multiply(self, lhs: Expr, rhs: Expr) -> Expr {
        if self.repr == IntRepr::Number && self.bits == 32 {
            jsast::imul(lhs, rhs)
        } else {
            jsast::binary(BinOp::Mul, lhs, rhs)
        }
    }

    /// A right shift with this type's signedness.
    fn shift_right(self, value: Expr, amount: Expr) -> Expr {
        let op = if self.signed || self.repr == IntRepr::BigInt {
            BinOp::Shr
        } else {
            BinOp::UShr
        };
        jsast::binary(op, value, amount)
    }

    /// The smallest and largest values of the type.
    fn range(self) -> (Expr, Expr) {
        match (self.repr, self.signed, self.bits) {
            (IntRepr::BigInt, false, 128) => (jsast::bigint(0u32), jsast::bigint(u128::MAX)),
            (IntRepr::BigInt, true, 128) => (jsast::bigint(i128::MIN), jsast::bigint(i128::MAX)),
            (repr, signed, bits) => {
                let (min, max) = if signed {
                    let magnitude = 1i128 << (bits - 1);
                    (-magnitude, magnitude - 1)
                } else {
                    (0, ((1u128 << bits) - 1) as i128)
                };
                match repr {
                    IntRepr::BigInt => (jsast::bigint(min), jsast::bigint(max)),
                    IntRepr::Number => (jsast::num(min as f64), jsast::num(max as f64)),
                }
            }
        }
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
        AbiSignature, Body, BodyKind, BodySemantic, ConstTree, DefinitionId, ItemId, Signature,
    };
    use rcgjs_object::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeId, TypeKind,
        TypeMeta,
    };
    use rcgjs_object::jir::ItemClass;
    use rcgjs_object::jsast;
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};

    use crate::analysis::byte_views::ByteViewPlan;
    use crate::analysis::heap::HeapPlan;
    use crate::lower::js::{host, unsize};
    use crate::lower::{LowerInput, NestedClone};
    use crate::opts::{Opts, Profile};

    fn mk_desc(id: u128, kind: TypeKind, size: u64) -> TypeDescriptor {
        let tid = TypeId(id);
        let int = match kind {
            TypeKind::Integer { signed, bits } => Some(IntFacts {
                signed,
                bits,
                repr: IntRepr::Number,
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
                is_indirect: false,
                needs_clone: false,
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

    #[test]
    fn tier_a_inline_copy_nonoverlapping_handles_dynamic_and_const_counts() {
        let mut interns = Interns::new();
        let copy_name = interns.intern_str("copy_nonoverlapping");
        let unit = TypeId(1);
        let u8_ty = TypeId(2);
        let usize_ty = TypeId(3);
        let ptr_const_u8 = TypeId(4);
        let ptr_mut_u8 = TypeId(5);
        interns
            .types
            .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0));
        interns.types.insert(mk_desc(
            2,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
        ));
        interns.types.insert(mk_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
        ));
        let mut pc = mk_desc(
            4,
            TypeKind::RawPointer {
                pointee: u8_ty,
                mutable: false,
            },
            4,
        );
        pc.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(pc);
        let mut pm = mk_desc(
            5,
            TypeKind::RawPointer {
                pointee: u8_ty,
                mutable: true,
            },
            4,
        );
        pm.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(pm);

        let heap = HeapPlan {
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
            packed_arr_vec_types: BTreeSet::new(),
        };

        let lower = |func: &Func| -> String {
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
                    link_name: "copy_test".to_owned(),
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
            let js_name = JsName::new("copy_test".to_owned());
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
                interns: &interns,
                table: &interns.types,
                opts: &opts,
                items: &items,
                drop_glue: host::EMPTY_DROP_GLUE,
                vtables: &vtables,
                spills: &[],
                abi: None,
                physical: None,
                call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
                bigints: None,
                byte_views: ByteViewPlan {
                    raw_byte_derefs: false,
                    elide_scale_scalar: true,
                },
                heap: Some(&heap),
                nested_clone: NestedClone::default(),
            };
            jsast::stmt_to_string(&crate::lower::lower_func(&input, func).expect("lower").decl)
        };

        // 1. Dynamic count: guarded by `> 0`, inline `<= 32` loop, `.set(.subarray())` for `> 32`,
        //    fallback to `__rt.copy_nonoverlapping` for non-TypedArray.
        let f_dyn = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 10,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ptr_const_u8, ptr_mut_u8, usize_ty],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ptr_const_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ptr_mut_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
            ],
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: vec![ValueId(0), ValueId(1), ValueId(2)],
                ops: vec![Op {
                    result: None,
                    origin: OriginId(0),
                    kind: OpKind::Intrinsic {
                        name: copy_name,
                        types: vec![u8_ty],
                        fallback: None,
                        args: vec![ValueId(0), ValueId(1), ValueId(2)],
                        track_caller: None,
                    },
                }],
                term: RTerm::Return(None),
            }],
        };
        let js_dyn = lower(&f_dyn);
        assert!(js_dyn.contains("ArrayBuffer.isView("), "{js_dyn}");
        assert!(js_dyn.contains("<= 32"), "{js_dyn}");
        assert!(js_dyn.contains(".subarray("), "{js_dyn}");
        assert!(js_dyn.contains("__rt.copy_nonoverlapping("), "{js_dyn}");

        // 2. Const count 16: inline loop only, no `.subarray(`.
        let f_16 = Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 9,
                    local_hash: 11,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: vec![ptr_const_u8, ptr_mut_u8],
                result: unit,
                abi: AbiSignature::default(),
            },
            params: vec![ValueId(0), ValueId(1)],
            cells: Vec::new(),
            values: vec![
                ValueDef {
                    ty: ptr_const_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: ptr_mut_u8,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                },
                ValueDef {
                    ty: usize_ty,
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
                        kind: OpKind::Const(ConstTree::Scalar { bits: 16, bytes: 4 }),
                    },
                    Op {
                        result: None,
                        origin: OriginId(0),
                        kind: OpKind::Intrinsic {
                            name: copy_name,
                            types: vec![u8_ty],
                            fallback: None,
                            args: vec![ValueId(0), ValueId(1), ValueId(2)],
                            track_caller: None,
                        },
                    },
                ],
                term: RTerm::Return(None),
            }],
        };
        let js_16 = lower(&f_16);
        assert!(js_16.contains("ArrayBuffer.isView("), "{js_16}");
        assert!(js_16.contains("while ("), "{js_16}");
        assert!(!js_16.contains(".subarray("), "{js_16}");

        // 3. Const count 0: complete no-op.
        let mut f_0 = f_16;
        f_0.blocks[0].ops[0].kind = OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 });
        let js_0 = lower(&f_0);
        assert!(!js_0.contains("copy_nonoverlapping"), "{js_0}");
        assert!(!js_0.contains("ArrayBuffer.isView"), "{js_0}");
    }
}
