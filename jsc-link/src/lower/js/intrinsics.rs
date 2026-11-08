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
use rcgjs_object::jir::repr::{Op, ValueId};
use rcgjs_object::jir::semantic::DirectItemRef;
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
) -> Res<Lowered> {
    let Some(name) = cx.interns.str(name) else {
        return Err(decline(
            OpClass::Intrinsic,
            "an intrinsic whose name is not in the interns",
        ));
    };
    let result_ty = op.result.map(|result| cx.ty_of(result)).transpose()?;
    let mut intrinsic = Intrinsic {
        cx,
        types,
        args,
        result_ty,
        captured: None,
        prelude: RefCell::new(Vec::new()),
    };
    if OperandEvaluation::intrinsic(name) == OperandEvaluation::Materialize {
        let arguments = args
            .iter()
            .map(|arg| cx.expr_of(*arg))
            .collect::<Res<_>>()?;
        let (setup, arguments) =
            OperandEvaluation::Materialize.capture(arguments, |value| cx.temp(value));
        intrinsic.prelude.borrow_mut().extend(setup);
        intrinsic.captured = Some(arguments);
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
            // A `&str` is a JavaScript string, whose size is the number of bytes it encodes to.
            Some(TypeKind::Str) => Ok(Outcome::Value(if size {
                self.cx.str_len(self.arg(0)?)
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
            "likely" | "unlikely" | "black_box" | "rustc_peek" => Outcome::Value(self.arg(0)?),
            "select_unpredictable" => {
                Outcome::Value(jsast::cond(self.arg(0)?, self.arg(1)?, self.arg(2)?))
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
                    value::clone_expr(table, self.cx.interns, *element, read)
                } else {
                    read
                }
            })
            .collect();
        Ok(Outcome::Value(jsast::call(jsast::id(name), arguments)))
    }

    /// The caller's `Location`: this body's own `$loc` inside a `#[track_caller]` function, the
    /// location of this very call anywhere else.
    fn caller_location(&self) -> Res<Expr> {
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

    fn arithmetic(&self, name: &str, int: Int) -> Res<Outcome> {
        let lhs = self.arg(0)?;
        let rhs = self.arg(1)?;
        let truncate = |value: Expr| value::mask(int.repr, int.signed, int.bits, value);
        Ok(match name {
            "wrapping_add" | "unchecked_add" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Add, lhs, rhs)))
            }
            "wrapping_sub" | "unchecked_sub" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Sub, lhs, rhs)))
            }
            "wrapping_mul" | "unchecked_mul" => Outcome::Value(truncate(int.multiply(lhs, rhs))),
            "unchecked_div" | "exact_div" => {
                Outcome::Value(truncate(jsast::binary(BinOp::Div, lhs, rhs)))
            }
            "unchecked_rem" => Outcome::Value(truncate(jsast::binary(BinOp::Rem, lhs, rhs))),
            "disjoint_bitor" => Outcome::Value(truncate(jsast::binary(BinOp::BitOr, lhs, rhs))),
            "unchecked_shl" => {
                let amount = self.shift_amount(int, rhs)?;
                Outcome::Value(truncate(jsast::binary(BinOp::Shl, lhs, amount)))
            }
            "unchecked_shr" => {
                let amount = self.shift_amount(int, rhs)?;
                Outcome::Value(truncate(int.shift_right(lhs, amount)))
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
    fn with_overflow(&self, name: &str, int: Int) -> Res<Outcome> {
        let lhs = self.arg(0)?;
        let rhs = self.arg(1)?;
        let exact = match name {
            "add_with_overflow" => jsast::binary(BinOp::Add, lhs, rhs),
            "sub_with_overflow" => jsast::binary(BinOp::Sub, lhs, rhs),
            _ => jsast::binary(BinOp::Mul, lhs, rhs),
        };
        let exact = self.temp(exact);
        let wrapped = self.temp(value::mask(int.repr, int.signed, int.bits, exact.clone()));
        Ok(Outcome::Value(jsast::array(vec![
            wrapped.clone(),
            jsast::binary(BinOp::StrictNe, wrapped, exact),
        ])))
    }

    /// `rotate_left` / `rotate_right`, as the pair of shifts JavaScript spells directly.
    fn rotate(&self, left: bool, int: Int) -> Res<Outcome> {
        let value = self.temp(int.zero_extend(self.arg(0)?));
        let amount = self.temp(jsast::binary(
            BinOp::BitAnd,
            self.arg(1)?,
            jsast::num((int.bits - 1) as f64),
        ));
        let amount = match int.repr {
            IntRepr::BigInt => jsast::to_bigint(amount),
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
            (IntRepr::BigInt, IntRepr::Number) => jsast::to_bigint(amount),
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
        let read = cx.deref_read(pointee, self.arg(0)?);
        Ok(if cx.table.needs_clone(pointee) {
            value::clone_expr(cx.table, cx.interns, pointee, read)
        } else {
            read
        })
    }

    /// `typed_swap_nonoverlapping(x, y)`: exchange the two pointees.
    fn typed_swap(&self) -> Res<Outcome> {
        let cx = self.cx;
        let pointee = self.type_at(0)?;
        let (x, y) = (self.arg(0)?, self.arg(1)?);
        let read = cx.deref_read(pointee, x.clone());
        let copy = if cx.table.needs_clone(pointee) {
            value::clone_expr(cx.table, cx.interns, pointee, read)
        } else {
            read
        };
        let saved = self.temp(copy);
        let mut statements = cx.deref_write(pointee, x, cx.deref_read(pointee, y.clone()))?;
        statements.extend(cx.deref_write(pointee, y, saved)?);
        Ok(Outcome::Done(statements))
    }

    /// `copy` and `copy_nonoverlapping`, and their volatile spellings.
    fn copy(&self, name: &str, overlapping: bool, volatile: bool) -> Res<Outcome> {
        let (source, target) = if volatile { (1, 0) } else { (0, 1) };
        let Some(pointee) = self.pointee_of(source)? else {
            return Ok(self.not_a_pointer(name));
        };
        let (dst, src) = (self.arg(target)?, self.arg(source)?);
        Ok(Outcome::Done(self.cx.ptr_copy(
            pointee,
            dst,
            src,
            self.arg(2)?,
            overlapping,
        )))
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
            let Some(zero) = value::zero_value(table, self.cx.interns, peeled) else {
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
