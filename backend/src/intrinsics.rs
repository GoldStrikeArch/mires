//! Native intrinsic lowerings.
//!
//! `abi.rs`'s `InstanceKind::Intrinsic` arm calls [`codegen_intrinsic`] FIRST, and only on `None`
//! falls back to the intrinsic's MIR fallback body (via `Instance::new_raw`) or, when there is no
//! such body, to a zombie.
//!
//! * `None` means "no native lowering — the caller decides between the fallback body and a zombie".
//! * `Some(stmts)` is the complete lowering INCLUDING the write to `destination`, but NOT the block
//!   edge that follows it. The one exception is a *diverging* intrinsic (`abort`, `unreachable`):
//!   there is no destination to write, so the statements end in a throw instead.
//!
//! # What actually reaches this file
//!
//! Rather less than the intrinsic list suggests. Two rustc passes get there first:
//!
//! * `rustc_mir_transform::lower_intrinsics` rewrites a call into an rvalue for `wrapping_*`,
//!   `unchecked_*`, `*_with_overflow`, `three_way_compare`, `offset`, `ptr_metadata`,
//!   `aggregate_raw_ptr`, `slice_get_unchecked`, `discriminant_value`, `read_via_copy`,
//!   `transmute`, `forget`, `copy_nonoverlapping`, `assume` and `unreachable`. Those land in
//!   `rvalue.rs`/`base.rs`, not here.
//! * The nullary type queries (`size_of`, `align_of`, `needs_drop`, `type_name`, `type_id`,
//!   `variant_count`, `offset_of`) are wrapped in `const { .. }` blocks by `core`, so a program
//!   built on the real `core` never calls them at runtime.
//!
//! The arms for both groups are implemented anyway, and they are not dead weight: a `#![no_core]`
//! crate that declares an intrinsic with `#[rustc_intrinsic]` and calls it directly — which is
//! what `examples/mini_core.rs` and `examples/tests/15_intrinsics.rs` do — reaches them, and the
//! MIR passes above are free to stop firing for a shape they do not recognise.
//!
//! # Integer representation
//!
//! Every integer arm dispatches on [`IntRepr`]: up to 32 bits (which on `wasm32-unknown-unknown`
//! includes `isize`/`usize`) a value is a JS number and the bit twiddling is the usual 32 bit
//! folklore; at 64 and 128 bits it is a `BigInt` and the same operations are spelled as small
//! loops. Correctness first — `ctpop` on a `u128` is a loop over 128 bits, and that is fine,
//! because nothing on a hot path calls it.
//!
//! # Floats
//!
//! An `f32` result goes through `Math.fround`, because JavaScript arithmetic is `f64` throughout.
//! The exact operations (`floor`, `ceil`, `trunc`, `round`, `fabs`, `copysign`, `minimum`,
//! `maximum`) do not need it: they map an `f32` to an `f32` exactly even when computed in double
//! precision. `f16` and `f128` have no JavaScript representation at all and are zombies.

use std::cell::RefCell;

use rustc_abi::FieldIdx;
use rustc_middle::mir;
use rustc_middle::ty::layout::ValidityRequirement;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use rustc_span::{Span, Spanned, Symbol};
use rustc_target::spec::PanicStrategy;

use crate::base::FnCx;
use crate::jsast::{self, BinOp as JsBinOp, Expr, Stmt};
use crate::naming::Namer;
use crate::ptr;
use crate::value::{self, IntRepr, int_info, int_repr, is_zst, mask, typing_env};

/// The hidden parameter a `#[track_caller]` function takes its caller's location in.
///
/// `$`-prefixed, so it can never collide with a MIR local (`_0`, `_1`, ...) or with an item name
/// (see the naming invariants in `naming.rs`).
pub(crate) const CALLER_LOCATION_PARAM: &str = "$loc";

/// The vtable slots the layout intrinsics read, per the vtable contract: `[drop, size, align,
/// methods...]`.
const VTABLE_SIZE_SLOT: f64 = 1.0;
const VTABLE_ALIGN_SLOT: f64 = 2.0;

/// The memory ordering words older toolchains spelled into an atomic intrinsic's name. This one
/// takes the ordering as a const generic instead, but stripping the suffixes costs nothing and
/// keeps the whole group handled if the spelling changes back.
const ORDERINGS: [&str; 5] = ["seqcst", "acquire", "release", "acqrel", "relaxed"];

/// The parameters `raw_eq` binds an array's element and index to. `$`-prefixed, so neither can
/// shadow a local of the function the comparison is inlined into.
const RAW_EQ_ELEMENT: &str = "$e";
const RAW_EQ_INDEX: &str = "$i";

/// One lowered intrinsic call.
enum Outcome {
    /// The value to write to the destination.
    Value(Expr),
    /// Statements that are the whole lowering; nothing is written to the destination.
    Done(Vec<Stmt>),
}

/// Lowers a call to an intrinsic natively, or reports that it has no native lowering.
pub(crate) fn codegen_intrinsic<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    instance: Instance<'tcx>,
    args: &[Spanned<mir::Operand<'tcx>>],
    destination: &mir::Place<'tcx>,
) -> Option<Vec<Stmt>> {
    let name = fx.tcx.intrinsic(instance.def_id())?.name;
    let cx = Intrinsic {
        fx,
        instance,
        args,
        destination: *destination,
        prelude: RefCell::new(Vec::new()),
    };

    let outcome = cx.lower(name.as_str())?;
    let mut out = cx.prelude.into_inner();
    match outcome {
        Outcome::Value(value) => out.extend(fx.write_place(*destination, value)),
        Outcome::Done(stmts) => out.extend(stmts),
    }
    Some(out)
}

/// The state of one intrinsic call's lowering: its arguments, and the statements that have to run
/// before the destination write.
struct Intrinsic<'a, 'tcx> {
    fx: &'a FnCx<'a, 'tcx>,
    instance: Instance<'tcx>,
    args: &'a [Spanned<mir::Operand<'tcx>>],
    destination: mir::Place<'tcx>,
    prelude: RefCell<Vec<Stmt>>,
}

impl<'tcx> Intrinsic<'_, 'tcx> {
    /// The dispatch table. Each group returns `None` for a name it does not own, so the chain falls
    /// through to the next group and finally to the caller's fallback.
    fn lower(&self, name: &str) -> Option<Outcome> {
        // Everything vector shaped, and the two float widths JavaScript cannot hold, are refused up
        // front rather than misread by a later group's suffix matching.
        if name.starts_with("simd_") {
            return Some(self.unsupported(format!(
                "the SIMD intrinsic `{name}` is not supported by rustc_codegen_mires"
            )));
        }
        if name.ends_with("f16") || name.ends_with("f128") {
            return Some(self.unsupported(format!(
                "`{name}` is not supported by rustc_codegen_mires: JavaScript has no `f16` or `f128`"
            )));
        }

        self.layout(name)
            .or_else(|| self.control(name))
            .or_else(|| self.integer(name))
            .or_else(|| self.float(name))
            .or_else(|| self.pointer(name))
            .or_else(|| self.atomic(name))
    }

    // ---------------------------------------------------------------------------------------
    // Types and layout
    // ---------------------------------------------------------------------------------------

    fn layout(&self, name: &str) -> Option<Outcome> {
        let tcx = self.fx.tcx;
        Some(match name {
            "size_of" | "align_of" => {
                let ty = self.type_at(0);
                let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
                    return Some(self.unsupported(format!("`{ty}` has no layout")));
                };
                let bytes =
                    if name == "size_of" { layout.size.bytes() } else { layout.align.bytes() };
                Outcome::Value(self.usize_literal(bytes))
            }
            "size_of_val" | "align_of_val" => self.size_of_val(name == "size_of_val"),
            "vtable_size" | "vtable_align" => {
                let slot = if name == "vtable_size" { VTABLE_SIZE_SLOT } else { VTABLE_ALIGN_SLOT };
                Outcome::Value(jsast::index(self.arg(0), jsast::num(slot)))
            }
            "variant_count" => {
                let count = match self.type_at(0).kind() {
                    ty::Adt(def, _) => def.variants().len(),
                    // Everything else has exactly one variant.
                    _ => 1,
                };
                Outcome::Value(self.usize_literal(count as u64))
            }
            "needs_drop" => {
                Outcome::Value(jsast::boolean(self.type_at(0).needs_drop(tcx, typing_env())))
            }
            // Returns a `&'static str`, which is a JavaScript string.
            "type_name" => {
                let ty = self.type_at(0);
                Outcome::Value(jsast::string(with_no_trimmed_paths!(format!("{ty}"))))
            }
            // A `TypeId` is opaque here: the stable 128 bit type hash, as a hexadecimal string. Two
            // of them compare correctly with `===`, which is all this backend can offer until a
            // `TypeId` is representable field by field — its real shape is an array of pointer
            // sized words carrying provenance.
            "type_id" => {
                let hash = tcx.type_id_hash(self.type_at(0)).as_u128();
                Outcome::Value(jsast::string(format!("{hash:032x}")))
            }
            // Byte offsets do not exist in a model where a struct is a JavaScript object.
            "offset_of" | "field_offset" => self.unsupported(format!(
                "`{name}` is not supported by rustc_codegen_mires: a struct is a JavaScript object, \
                 which has no byte offsets"
            )),
            // Normally rewritten to `Rvalue::Discriminant` by `LowerIntrinsics`.
            //
            // The **number**, never the tag: the result is a `mem::Discriminant<T>` wrapping the
            // raw integer, and a caller may hash it as well as compare it. `value.rs` spells the
            // number for whichever shape the enum has.
            "discriminant_value" => {
                let ty = self.type_at(0);
                match ty.kind() {
                    ty::Adt(def, _) if def.is_enum() => Outcome::Value(
                        value::discriminant_number(tcx, ty, self.arg(0))
                            .unwrap_or_else(|| jsast::num(0)),
                    ),
                    _ => Outcome::Value(jsast::num(0)),
                }
            }
            _ => return None,
        })
    }

    /// `size_of_val` / `align_of_val`, whose answer depends on a fat pointer's metadata.
    fn size_of_val(&self, size: bool) -> Outcome {
        let tcx = self.fx.tcx;
        let ty = self.type_at(0);

        // A sized pointee needs no metadata: the answer is a constant.
        if ty.is_sized(tcx, typing_env()) {
            let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
                return self.unsupported(format!("`{ty}` has no layout"));
            };
            let bytes = if size { layout.size.bytes() } else { layout.align.bytes() };
            return Outcome::Value(self.usize_literal(bytes));
        }

        match ty.kind() {
            ty::Slice(element) => {
                let Ok(layout) = tcx.layout_of(typing_env().as_query_input(*element)) else {
                    return self.unsupported(format!("`{element}` has no layout"));
                };
                if !size {
                    return Outcome::Value(self.usize_literal(layout.align.bytes()));
                }
                // `{ buf, off, len }`: the element count times the element size.
                let len = jsast::member(self.arg(0), "len");
                Outcome::Value(match layout.size.bytes() {
                    1 => len,
                    stride => jsast::binary(JsBinOp::Mul, len, jsast::num(stride as f64)),
                })
            }
            // A `&str` is a JavaScript string, whose size is the number of bytes it encodes to
            // rather than the number of UTF-16 code units it is stored as — the two agree only for
            // ASCII. `__rt.str_len` is the same answer `PtrMetadata` gives (`place.rs`).
            ty::Str => Outcome::Value(if size {
                crate::ptr::str_len(self.arg(0))
            } else {
                self.usize_literal(1)
            }),
            // `{ ptr, meta }`, where `meta` is the vtable array.
            ty::Dynamic(..) => {
                let slot = if size { VTABLE_SIZE_SLOT } else { VTABLE_ALIGN_SLOT };
                Outcome::Value(jsast::index(jsast::member(self.arg(0), "meta"), jsast::num(slot)))
            }
            _ => match crate::value::unsized_tail(tcx, ty) {
                Some(tail) => self.size_of_tail(ty, tail, size),
                None => {
                    self.unsupported(format!("the size of `{ty}` is not known to rustc_codegen_mires"))
                }
            },
        }
    }

    /// `size_of_val` / `align_of_val` of a **struct with an unsized tail**, whose pointer is
    /// `{ ptr, meta }` (`value.rs`).
    ///
    /// The size is the offset of the tail field plus the tail the metadata measures, which is the
    /// layout's own answer with the run of elements filled in. Alignment is a property of the type
    /// alone, so the layout answers it outright.
    fn size_of_tail(&self, ty: Ty<'tcx>, tail: Ty<'tcx>, size: bool) -> Outcome {
        let tcx = self.fx.tcx;
        let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
            return self.unsupported(format!("`{ty}` has no layout"));
        };
        if !size {
            return Outcome::Value(self.usize_literal(layout.align.bytes()));
        }
        let ty::Slice(element) = tail.kind() else {
            return self.unsupported(format!(
                "the size of a struct with a `{tail}` tail is not known to rustc_codegen_mires: only \
                 a slice tail has a run of elements to measure"
            ));
        };
        let Ok(element) = tcx.layout_of(typing_env().as_query_input(*element)) else {
            return self.unsupported(format!("`{tail}` has no layout"));
        };
        // The sized prefix ends where the tail begins, which is the layout's size for an unsized
        // struct: nothing is rounded up past it while the tail's own length is still unknown.
        let prefix = self.usize_literal(layout.size.bytes());
        let meta = jsast::member(self.arg(0), "meta");
        let tail = match element.size.bytes() {
            1 => meta,
            stride => jsast::binary(JsBinOp::Mul, meta, jsast::num(stride as f64)),
        };
        Outcome::Value(jsast::binary(JsBinOp::Add, prefix, tail))
    }

    // ---------------------------------------------------------------------------------------
    // Control and compiler hints
    // ---------------------------------------------------------------------------------------

    fn control(&self, name: &str) -> Option<Outcome> {
        let tcx = self.fx.tcx;
        Some(match name {
            "abort" => Outcome::Done(vec![jsast::expr_stmt(jsast::rt_call(
                "js_abort",
                vec![jsast::string("rustc_codegen_mires: `abort` intrinsic")],
            ))]),
            // Normally rewritten to `TerminatorKind::Unreachable` by `LowerIntrinsics`.
            "unreachable" => {
                Outcome::Done(vec![jsast::throw_error("rustc_codegen_mires: entered unreachable code")])
            }
            // A `debugger` statement would stop every run under attached devtools, which is not
            // what a stray `breakpoint()` inside a library should do.
            "breakpoint" => Outcome::Done(vec![jsast::comment("`breakpoint` intrinsic")]),
            "cold_path" | "assume" | "prefetch_read_data" | "prefetch_write_data"
            | "prefetch_read_instruction" | "prefetch_write_instruction" => {
                Outcome::Done(Vec::new())
            }
            "likely" | "unlikely" | "black_box" | "rustc_peek" => Outcome::Value(self.arg(0)),
            "select_unpredictable" => {
                Outcome::Value(jsast::cond(self.arg(0), self.arg(1), self.arg(2)))
            }
            // Nothing is statically known here, and the runtime checks are all compiled out.
            "is_val_statically_known" | "ub_checks" | "contract_checks" => {
                Outcome::Value(jsast::boolean(false))
            }
            "overflow_checks" => Outcome::Value(jsast::boolean(tcx.sess.overflow_checks())),
            // Normally rewritten to a unit assignment by `LowerIntrinsics`. Dropping a value
            // without running its destructor is a no-op when a garbage collector owns it.
            "forget" => Outcome::Value(jsast::undefined()),
            "assert_inhabited" | "assert_zero_valid" | "assert_mem_uninitialized_valid" => {
                self.assert_valid(name)
            }
            "caller_location" => Outcome::Value(self.caller_location()),
            "const_eval_select" => self.const_eval_select(),
            "catch_unwind" => self.catch_unwind(),
            "return_address" => self.unsupported(
                "`return_address` is not supported by rustc_codegen_mires: a JavaScript call stack \
                 has no addresses"
                    .to_string(),
            ),
            _ => return None,
        })
    }

    /// `assert_inhabited` and its siblings: a compile time check that aborts at runtime when it
    /// does not hold.
    fn assert_valid(&self, name: &str) -> Outcome {
        let tcx = self.fx.tcx;
        let ty = self.type_at(0);
        let Some(requirement) = ValidityRequirement::from_intrinsic(Symbol::intern(name)) else {
            return Outcome::Done(Vec::new());
        };
        let holds = tcx
            .check_validity_requirement((requirement, typing_env().as_query_input(ty)))
            .unwrap_or(true);
        if holds {
            return Outcome::Done(Vec::new());
        }
        let message = with_no_trimmed_paths!(match requirement {
            ValidityRequirement::Inhabited => {
                format!("attempted to instantiate uninhabited type `{ty}`")
            }
            ValidityRequirement::Zero => {
                format!("attempted to zero-initialize type `{ty}`, which is invalid")
            }
            _ => format!("attempted to leave type `{ty}` uninitialized, which is invalid"),
        });
        Outcome::Done(vec![jsast::expr_stmt(jsast::rt_call(
            "js_abort",
            vec![jsast::string(message)],
        ))])
    }

    /// `const_eval_select(args, in_const, at_runtime)`. This is codegen, so it is always the
    /// runtime arm, called with its argument tuple splatted the way the `"rust-call"` ABI does.
    fn const_eval_select(&self) -> Outcome {
        let tcx = self.fx.tcx;
        let runtime_ty = self.arg_ty(2);
        let ty::FnDef(def_id, generic_args) = *runtime_ty.kind() else {
            return self.unsupported(format!(
                "the runtime argument of `const_eval_select` is `{runtime_ty}`, not a function item"
            ));
        };
        let Some(generic_args) = generic_args.no_bound_vars() else {
            return self.unsupported(
                "the runtime argument of `const_eval_select` has bound variables".to_string(),
            );
        };
        let callee =
            Instance::expect_resolve(tcx, typing_env(), def_id, generic_args, self.fx.span());

        let pack_ty = self.arg_ty(0);
        let ty::Tuple(element_tys) = pack_ty.kind() else {
            return self.unsupported(format!(
                "the argument pack of `const_eval_select` is `{pack_ty}`, not a tuple"
            ));
        };
        let pack = self.arg(0);
        let arguments = element_tys
            .iter()
            .enumerate()
            .map(|(i, element_ty)| {
                let element = jsast::index(pack.clone(), jsast::num(i as f64));
                if value::needs_clone(tcx, element_ty) {
                    value::clone_expr(tcx, element_ty, element)
                } else {
                    element
                }
            })
            .collect();

        let name = self.fx.cgu.namer.fn_name(callee).into_string();
        Outcome::Value(jsast::call(jsast::id(name), arguments))
    }

    /// `catch_unwind(try_fn, data, catch_fn)`. Under `-Cpanic=abort` nothing can unwind, so this is
    /// the call plus a zero return code — the same shortcut `rustc_codegen_cranelift` takes.
    fn catch_unwind(&self) -> Outcome {
        if self.fx.tcx.sess.panic_strategy() != PanicStrategy::Abort {
            return self.unsupported(
                "`catch_unwind` is not supported by rustc_codegen_mires: unwinding does not exist in \
                 the JavaScript lowering (build with `-Cpanic=abort`)"
                    .to_string(),
            );
        }
        let call = jsast::call(self.arg(0), vec![self.arg(1)]);
        self.prelude.borrow_mut().push(jsast::expr_stmt(call));
        Outcome::Value(jsast::num(0))
    }

    /// The caller's `Location`, per the `#[track_caller]` mechanism.
    ///
    /// Inside a `#[track_caller]` function it is the hidden parameter the caller passed; anywhere
    /// else `Location::caller()` means "here", so the location of this very call is built inline.
    /// The distinction is the *enclosing* function's, never the intrinsic's own instance.
    fn caller_location(&self) -> Expr {
        caller_location_argument(self.fx, self.fx.span())
    }

    // ---------------------------------------------------------------------------------------
    // Integers
    // ---------------------------------------------------------------------------------------

    fn integer(&self, name: &str) -> Option<Outcome> {
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
            return None;
        }

        // The type the operation happens at: the first operand's. It is not the destination's —
        // `ctpop` returns a `u32` whatever it counted, and `three_way_compare` an `Ordering`.
        let operand_ty = self.arg_ty(0);
        if name == "three_way_compare" {
            return Some(self.three_way_compare(operand_ty));
        }

        // `disjoint_bitor` is defined on `bool` as well as on the integers, and there it is the
        // ordinary `||`: the operands are promised to share no bit, and with one bit each that
        // means at most one of them is `true`. `Option`'s niche helpers reach it.
        if name == "disjoint_bitor" && matches!(operand_ty.kind(), ty::Bool) {
            return Some(Outcome::Value(jsast::binary(JsBinOp::Or, self.arg(0), self.arg(1))));
        }
        let Some((signed, bits)) = int_info(self.fx.tcx, operand_ty) else {
            return Some(self.unsupported(format!(
                "`{name}` on `{operand_ty}` is not supported by rustc_codegen_mires"
            )));
        };
        let int = Int { repr: int_repr(operand_ty), signed, bits };

        Some(if bit_op { self.bit_op(name, int) } else { self.arithmetic(name, int) })
    }

    /// `wrapping_*`, `unchecked_*`, `saturating_*`, `*_with_overflow`, `rotate_*`.
    fn arithmetic(&self, name: &str, int: Int) -> Outcome {
        let lhs = self.arg(0);
        let rhs = self.arg(1);
        let truncate = |value: Expr| mask(int.repr, int.signed, int.bits, value);

        match name {
            "wrapping_add" | "unchecked_add" => {
                Outcome::Value(truncate(jsast::binary(JsBinOp::Add, lhs, rhs)))
            }
            "wrapping_sub" | "unchecked_sub" => {
                Outcome::Value(truncate(jsast::binary(JsBinOp::Sub, lhs, rhs)))
            }
            "wrapping_mul" | "unchecked_mul" => Outcome::Value(truncate(int.multiply(lhs, rhs))),
            // `| 0`, `>>> 0` and `<< n >> n` all truncate toward zero, and a `BigInt` division
            // already does; `exact_div` promises the division came out exact anyway.
            "unchecked_div" | "exact_div" => {
                Outcome::Value(truncate(jsast::binary(JsBinOp::Div, lhs, rhs)))
            }
            "unchecked_rem" => Outcome::Value(truncate(jsast::binary(JsBinOp::Rem, lhs, rhs))),
            // The operands are promised to have no bit in common, so `|` is the sum.
            "disjoint_bitor" => Outcome::Value(truncate(jsast::binary(JsBinOp::BitOr, lhs, rhs))),
            "unchecked_shl" => {
                let amount = self.shift_amount(int, rhs);
                Outcome::Value(truncate(jsast::binary(JsBinOp::Shl, lhs, amount)))
            }
            "unchecked_shr" => {
                let amount = self.shift_amount(int, rhs);
                Outcome::Value(truncate(int.shift_right(lhs, amount)))
            }
            "saturating_add" | "saturating_sub" => self.saturating(name == "saturating_add", int),
            "add_with_overflow" | "sub_with_overflow" | "mul_with_overflow" => {
                self.with_overflow(name, int)
            }
            "rotate_left" | "rotate_right" => self.rotate(name == "rotate_left", int),
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_mires")),
        }
    }

    /// `saturating_add` / `saturating_sub`: the exact result, clamped to the type's range.
    ///
    /// The sum of two values of at most 32 bits is exact in a double, and a `BigInt` sum is exact
    /// by construction, so the clamp only has to compare.
    fn saturating(&self, add: bool, int: Int) -> Outcome {
        let op = if add { JsBinOp::Add } else { JsBinOp::Sub };
        let sum = self.temp(jsast::binary(op, self.arg(0), self.arg(1)));
        let (min, max) = int.range();
        Outcome::Value(jsast::cond(
            jsast::binary(JsBinOp::Gt, sum.clone(), max.clone()),
            max,
            jsast::cond(jsast::binary(JsBinOp::Lt, sum.clone(), min.clone()), min, sum),
        ))
    }

    /// `add_with_overflow` and its siblings: the wrapped result paired with whether it wrapped.
    /// The destination is a `(T, bool)` tuple, which is a two element JS array.
    fn with_overflow(&self, name: &str, int: Int) -> Outcome {
        let lhs = self.arg(0);
        let rhs = self.arg(1);
        let exact = match name {
            "add_with_overflow" => jsast::binary(JsBinOp::Add, lhs, rhs),
            "sub_with_overflow" => jsast::binary(JsBinOp::Sub, lhs, rhs),
            // A 32 bit product overflows a double's 53 bit mantissa, so this is not always the
            // exact product. It does not have to be: it is only compared for *equality* with the
            // wrapped result, and a rounded product that overflowed is never equal to it.
            _ => jsast::binary(JsBinOp::Mul, lhs, rhs),
        };
        let exact = self.temp(exact);
        let wrapped = self.temp(mask(int.repr, int.signed, int.bits, exact.clone()));
        Outcome::Value(jsast::array(vec![
            wrapped.clone(),
            jsast::binary(JsBinOp::StrictNe, wrapped, exact),
        ]))
    }

    /// `rotate_left` / `rotate_right`, as the pair of shifts JavaScript spells directly.
    fn rotate(&self, left: bool, int: Int) -> Outcome {
        let value = self.temp(int.zero_extend(self.arg(0)));
        // The rotate amount is a `u32` and the width is a power of two, so masking is the modulo.
        let amount =
            self.temp(jsast::binary(JsBinOp::BitAnd, self.arg(1), jsast::num((int.bits - 1) as f64)));
        let amount = match int.repr {
            IntRepr::BigInt => jsast::to_bigint(amount),
            IntRepr::Number => amount,
        };
        let width = int.literal(int.bits as i128);
        let (up, down) = if left {
            (amount.clone(), jsast::binary(JsBinOp::Sub, width, amount))
        } else {
            (jsast::binary(JsBinOp::Sub, width, amount.clone()), amount)
        };
        // A rotate by zero comes out right on its own: `x >>> 32` is `x` for a 32 bit number (the
        // shift amount is masked to five bits) and `x >>> bits` is zero for anything narrower,
        // because the value was zero extended first.
        Outcome::Value(mask(
            int.repr,
            int.signed,
            int.bits,
            jsast::binary(
                JsBinOp::BitOr,
                jsast::binary(JsBinOp::Shl, value.clone(), up),
                int.unsigned().shift_right(value, down),
            ),
        ))
    }

    /// `three_way_compare`, whose result is a `core::cmp::Ordering`.
    ///
    /// `Ordering` is a fieldless `#[repr(i8)]` enum, so each of the three is the plain number its
    /// discriminant is. The variants are looked up by name and built through `value.rs`, so that
    /// this spells them exactly the way `rvalue.rs`'s `Rvalue::Aggregate` does.
    fn three_way_compare(&self, operand_ty: Ty<'tcx>) -> Outcome {
        let ordering_ty = self.ret_ty();
        let ty::Adt(def, _) = ordering_ty.kind() else {
            return self.unsupported(format!(
                "`three_way_compare` returned `{ordering_ty}`, which is not an `Ordering`"
            ));
        };
        let variant = |wanted: &str| -> Option<Expr> {
            let index = def
                .variants()
                .iter_enumerated()
                .find_map(|(index, variant)| (variant.name.as_str() == wanted).then_some(index))?;
            value::enum_value(self.fx.tcx, ordering_ty, index, Vec::new())
        };
        let (Some(less), Some(equal), Some(greater)) =
            (variant("Less"), variant("Equal"), variant("Greater"))
        else {
            return self.unsupported(format!(
                "`three_way_compare` returned `{ordering_ty}`, which is not an `Ordering`"
            ));
        };
        if int_info(self.fx.tcx, operand_ty).is_none()
            && !matches!(operand_ty.kind(), ty::Bool | ty::Char)
        {
            return self.unsupported(format!(
                "`three_way_compare` on `{operand_ty}` is not supported by rustc_codegen_mires"
            ));
        }
        Outcome::Value(jsast::cond(
            jsast::binary(JsBinOp::Lt, self.arg(0), self.arg(1)),
            less,
            jsast::cond(
                jsast::binary(JsBinOp::StrictEq, self.arg(0), self.arg(1)),
                equal,
                greater,
            ),
        ))
    }

    fn bit_op(&self, name: &str, int: Int) -> Outcome {
        match int.repr {
            IntRepr::Number => self.bit_op_number(name, int),
            IntRepr::BigInt => self.bit_op_bigint(name, int),
        }
    }

    /// `ctpop`, `ctlz`, `cttz`, `bswap` and `bitreverse` on a number, as the 32 bit folklore.
    fn bit_op_number(&self, name: &str, int: Int) -> Outcome {
        let value = int.zero_extend(self.arg(0));
        let truncate = |value: Expr| mask(int.repr, int.signed, int.bits, value);
        let and = |lhs: Expr, rhs: u32| jsast::binary(JsBinOp::BitAnd, lhs, jsast::num(rhs as f64));
        let ushr = |lhs: Expr, rhs: u32| jsast::binary(JsBinOp::UShr, lhs, jsast::num(rhs as f64));
        let shl = |lhs: Expr, rhs: u32| jsast::binary(JsBinOp::Shl, lhs, jsast::num(rhs as f64));
        let or = |lhs: Expr, rhs: Expr| jsast::binary(JsBinOp::BitOr, lhs, rhs);
        let math = |method: &str, arg: Expr| {
            jsast::call(jsast::member(jsast::id("Math"), method), vec![arg])
        };

        match name {
            // `Math.clz32` counts over 32 bits; a narrower type has that many leading zeros fewer.
            "ctlz" | "ctlz_nonzero" => {
                let leading = math("clz32", value);
                Outcome::Value(if int.bits == 32 {
                    leading
                } else {
                    jsast::binary(JsBinOp::Sub, leading, jsast::num((32 - int.bits) as f64))
                })
            }
            // `x & -x` isolates the lowest set bit, whose index is `31 - clz32`.
            "cttz" | "cttz_nonzero" => {
                let value = self.temp(value);
                let lowest =
                    jsast::binary(JsBinOp::BitAnd, value.clone(), jsast::neg(value.clone()));
                Outcome::Value(jsast::cond(
                    jsast::binary(JsBinOp::StrictEq, value, jsast::num(0)),
                    jsast::num(int.bits as f64),
                    jsast::binary(JsBinOp::Sub, jsast::num(31), math("clz32", lowest)),
                ))
            }
            "ctpop" => {
                let slot = self.temp(value);
                let set = |expr: Expr| jsast::assign_stmt(slot.clone(), expr);
                self.prelude.borrow_mut().extend([
                    set(jsast::binary(
                        JsBinOp::Sub,
                        slot.clone(),
                        and(ushr(slot.clone(), 1), 0x5555_5555),
                    )),
                    set(jsast::binary(
                        JsBinOp::Add,
                        and(slot.clone(), 0x3333_3333),
                        and(ushr(slot.clone(), 2), 0x3333_3333),
                    )),
                    set(and(
                        jsast::binary(JsBinOp::Add, slot.clone(), ushr(slot.clone(), 4)),
                        0x0f0f_0f0f,
                    )),
                ]);
                // The four byte sums, summed into the top byte by one multiply.
                Outcome::Value(ushr(jsast::imul(slot, jsast::num(0x0101_0101u32 as f64)), 24))
            }
            "bswap" => match int.bits {
                8 => Outcome::Value(self.arg(0)),
                16 => {
                    let value = self.temp(value);
                    Outcome::Value(truncate(or(ushr(value.clone(), 8), shl(value, 8))))
                }
                _ => {
                    let value = self.temp(value);
                    Outcome::Value(truncate(or(
                        or(ushr(value.clone(), 24), and(ushr(value.clone(), 8), 0x0000_ff00)),
                        or(and(shl(value.clone(), 8), 0x00ff_0000), shl(value, 24)),
                    )))
                }
            },
            "bitreverse" => {
                let slot = self.temp(value);
                let set = |expr: Expr| jsast::assign_stmt(slot.clone(), expr);
                let swap = |width: u32, pattern: u32| {
                    or(and(ushr(slot.clone(), width), pattern), shl(and(slot.clone(), pattern), width))
                };
                self.prelude.borrow_mut().extend([
                    set(swap(1, 0x5555_5555)),
                    set(swap(2, 0x3333_3333)),
                    set(swap(4, 0x0f0f_0f0f)),
                    set(swap(8, 0x00ff_00ff)),
                    set(or(ushr(slot.clone(), 16), shl(slot.clone(), 16))),
                ]);
                // The reversal happened over 32 bits, so a narrower value ends up in the high bits
                // and has to come back down.
                Outcome::Value(truncate(if int.bits == 32 {
                    slot
                } else {
                    ushr(slot, 32 - int.bits)
                }))
            }
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_mires")),
        }
    }

    /// The same operations on a `BigInt`, as small loops.
    ///
    /// There is no 128 bit bit twiddling in JavaScript to borrow, and none of these is on a hot
    /// path: a loop that is obviously right beats a clever split into 32 bit halves.
    fn bit_op_bigint(&self, name: &str, int: Int) -> Outcome {
        let zero = jsast::bigint(0u32);
        let one = jsast::bigint(1u32);
        let value = self.temp(int.zero_extend(self.arg(0)));
        let shift_down = |by: u32| {
            jsast::assign_stmt(
                value.clone(),
                jsast::binary(JsBinOp::Shr, value.clone(), jsast::bigint(by)),
            )
        };
        let step = |counter: &Expr, by: f64| {
            jsast::assign_stmt(
                counter.clone(),
                jsast::binary(JsBinOp::Add, counter.clone(), jsast::num(by)),
            )
        };

        match name {
            "ctpop" => {
                let count = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::while_(
                    jsast::binary(JsBinOp::StrictNe, value.clone(), zero),
                    vec![
                        jsast::assign_stmt(
                            count.clone(),
                            jsast::binary(
                                JsBinOp::Add,
                                count.clone(),
                                jsast::to_number(jsast::binary(
                                    JsBinOp::BitAnd,
                                    value.clone(),
                                    one,
                                )),
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
                    jsast::binary(JsBinOp::StrictNe, value.clone(), zero),
                    vec![step(&count, -1.0), shift_down(1)],
                ));
                Outcome::Value(count)
            }
            "cttz" | "cttz_nonzero" => {
                let count = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::if_else(
                    jsast::binary(JsBinOp::StrictEq, value.clone(), zero.clone()),
                    vec![jsast::assign_stmt(count.clone(), jsast::num(int.bits as f64))],
                    vec![jsast::while_(
                        jsast::binary(
                            JsBinOp::StrictEq,
                            jsast::binary(JsBinOp::BitAnd, value.clone(), one),
                            zero,
                        ),
                        vec![step(&count, 1.0), shift_down(1)],
                    )],
                ));
                Outcome::Value(count)
            }
            // Both are the same loop over a different chunk width: take the low chunk off one end
            // and push it onto the other.
            "bswap" | "bitreverse" => {
                let (width, chunk, chunks) = if name == "bswap" {
                    (8u32, jsast::bigint(255u32), int.bits / 8)
                } else {
                    (1u32, one, int.bits)
                };
                let out = self.temp(zero);
                let index = self.temp(jsast::num(0));
                self.prelude.borrow_mut().push(jsast::while_(
                    jsast::binary(JsBinOp::Lt, index.clone(), jsast::num(chunks as f64)),
                    vec![
                        jsast::assign_stmt(
                            out.clone(),
                            jsast::binary(
                                JsBinOp::BitOr,
                                jsast::binary(JsBinOp::Shl, out.clone(), jsast::bigint(width)),
                                jsast::binary(JsBinOp::BitAnd, value.clone(), chunk),
                            ),
                        ),
                        shift_down(width),
                        step(&index, 1.0),
                    ],
                ));
                Outcome::Value(mask(int.repr, int.signed, int.bits, out))
            }
            _ => self.unsupported(format!("`{name}` is not supported by rustc_codegen_mires")),
        }
    }

    /// A shift amount, converted into the representation the shifted type uses.
    fn shift_amount(&self, int: Int, amount: Expr) -> Expr {
        match (int.repr, int_repr(self.arg_ty(1))) {
            (IntRepr::BigInt, IntRepr::Number) => jsast::to_bigint(amount),
            // Narrow first: a shift amount only ever uses its low bits, and `Number` of a large
            // `BigInt` rounds.
            (IntRepr::Number, IntRepr::BigInt) => {
                jsast::to_number(jsast::bigint_mask(false, 32, amount))
            }
            _ => amount,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Floats
    // ---------------------------------------------------------------------------------------

    fn float(&self, name: &str) -> Option<Outcome> {
        // The one arm whose result is not a float.
        if name == "float_to_int_unchecked" {
            return Some(self.float_to_int());
        }

        let base = name.strip_suffix("f32").or_else(|| name.strip_suffix("f64")).unwrap_or(name);
        let math = |method: &str, arity: usize| {
            let args = (0..arity).map(|i| self.arg(i)).collect();
            jsast::call(jsast::member(jsast::id("Math"), method), args)
        };
        let binary = |op: JsBinOp| jsast::binary(op, self.arg(0), self.arg(1));

        let value = match base {
            "sqrt" => math("sqrt", 1),
            "sin" => math("sin", 1),
            "cos" => math("cos", 1),
            "exp" => math("exp", 1),
            "log" => math("log", 1),
            "log2" => math("log2", 1),
            "log10" => math("log10", 1),
            "floor" => math("floor", 1),
            "ceil" => math("ceil", 1),
            "trunc" => math("trunc", 1),
            "fabs" => math("abs", 1),
            // `powi` takes an `i32` exponent, which `Math.pow` is happy to take as a number.
            "pow" | "powi" => math("pow", 2),
            "exp2" => jsast::call(
                jsast::member(jsast::id("Math"), "pow"),
                vec![jsast::num(2), self.arg(0)],
            ),
            // JavaScript's `Math.min`/`Math.max` already are IEEE 754 `minimum`/`maximum`: `-0` is
            // ordered below `+0`, and a NaN operand makes the result NaN.
            "minimum" => math("min", 2),
            "maximum" => math("max", 2),
            // `minimumNumber`: the operand that is not NaN. Spelled the way `core`'s own fallback
            // body spells it, so the `-0`/`+0` tie breaks the same way.
            "minimum_number_nsz_" | "maximum_number_nsz_" => {
                let (x, y) = (self.arg(0), self.arg(1));
                let take_y = if base.starts_with("minimum") {
                    jsast::binary(JsBinOp::Le, y.clone(), x.clone())
                } else {
                    jsast::binary(JsBinOp::Ge, y.clone(), x.clone())
                };
                let x_is_nan = jsast::binary(JsBinOp::StrictNe, x.clone(), x.clone());
                jsast::cond(jsast::binary(JsBinOp::Or, x_is_nan, take_y), y, x)
            }
            // Rust rounds a tie away from zero; `Math.round` rounds it toward positive infinity.
            "round" => {
                let x = self.temp(self.arg(0));
                let round =
                    |value: Expr| jsast::call(jsast::member(jsast::id("Math"), "round"), vec![value]);
                jsast::cond(
                    jsast::binary(JsBinOp::Lt, x.clone(), jsast::num(0)),
                    jsast::neg(round(jsast::neg(x.clone()))),
                    round(x),
                )
            }
            "round_ties_even_" => return Some(self.round_ties_even()),
            "copysign" => {
                let (x, y) = (self.arg(0), self.arg(1));
                let magnitude = jsast::call(jsast::member(jsast::id("Math"), "abs"), vec![x]);
                // `Object.is(y, -0)` is the only way to tell `-0` from `+0` in JavaScript.
                let negative = jsast::binary(
                    JsBinOp::Or,
                    jsast::binary(JsBinOp::Lt, y.clone(), jsast::num(0)),
                    jsast::call(
                        jsast::member(jsast::id("Object"), "is"),
                        vec![y, jsast::num(-0.0)],
                    ),
                );
                jsast::cond(negative, jsast::neg(magnitude.clone()), magnitude)
            }
            // A fused multiply-add rounds once; JavaScript has no way to ask for that, so this
            // rounds twice. `fmuladd` explicitly permits that, `fma` does not — the deviation is
            // at most one ulp, and is the price of not shipping a soft-float `fma`.
            "fma" | "fmuladd" => jsast::binary(
                JsBinOp::Add,
                jsast::binary(JsBinOp::Mul, self.arg(0), self.arg(1)),
                self.arg(2),
            ),
            // "fast" and "algebraic" arithmetic: the plain operations, with none of the
            // reassociation a real optimizer would be allowed to do.
            "fadd_fast" | "fadd_algebraic" => binary(JsBinOp::Add),
            "fsub_fast" | "fsub_algebraic" => binary(JsBinOp::Sub),
            "fmul_fast" | "fmul_algebraic" => binary(JsBinOp::Mul),
            "fdiv_fast" | "fdiv_algebraic" => binary(JsBinOp::Div),
            "frem_fast" | "frem_algebraic" => binary(JsBinOp::Rem),
            _ => return None,
        };

        // These map an `f32` to an `f32` exactly even when computed in double precision.
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
        Some(Outcome::Value(if exact { value } else { self.round_to_f32(value) }))
    }

    /// `float_to_int_unchecked`: the truncation toward zero, at the destination's width.
    fn float_to_int(&self) -> Outcome {
        let to_ty = self.ret_ty();
        let Some((signed, bits)) = int_info(self.fx.tcx, to_ty) else {
            return self.unsupported(format!(
                "`float_to_int_unchecked` to `{to_ty}` is not supported by rustc_codegen_mires"
            ));
        };
        let truncated = jsast::call(jsast::member(jsast::id("Math"), "trunc"), vec![self.arg(0)]);
        let repr = int_repr(to_ty);
        let value = match repr {
            IntRepr::BigInt => jsast::to_bigint(truncated),
            IntRepr::Number => truncated,
        };
        Outcome::Value(mask(repr, signed, bits, value))
    }

    /// `round_ties_even`, the rounding a hardware `rint` does in the default mode.
    fn round_ties_even(&self) -> Outcome {
        let x = self.temp(self.arg(0));
        let floor =
            self.temp(jsast::call(jsast::member(jsast::id("Math"), "floor"), vec![x.clone()]));
        let fraction = jsast::binary(JsBinOp::Sub, x, floor.clone());
        let up = jsast::binary(JsBinOp::Add, floor.clone(), jsast::num(1));
        // A tie goes to whichever neighbour is even. `f % 2` is zero exactly when `f` is even, and
        // is NaN for an infinity — which then takes the `f + 1` branch, and `Infinity + 1` is
        // `Infinity`.
        let tie = jsast::cond(
            jsast::binary(
                JsBinOp::StrictEq,
                jsast::binary(JsBinOp::Rem, floor.clone(), jsast::num(2)),
                jsast::num(0),
            ),
            floor.clone(),
            up.clone(),
        );
        let value = jsast::cond(
            jsast::binary(JsBinOp::Lt, fraction.clone(), jsast::num(0.5)),
            floor,
            jsast::cond(jsast::binary(JsBinOp::Gt, fraction, jsast::num(0.5)), up, tie),
        );
        Outcome::Value(self.round_to_f32(value))
    }

    /// Rounds to `f32` precision when that is what the destination holds.
    fn round_to_f32(&self, value: Expr) -> Expr {
        match self.ret_ty().kind() {
            ty::Float(ty::FloatTy::F32) => jsast::fround(value),
            _ => value,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Pointers and slices
    // ---------------------------------------------------------------------------------------

    fn pointer(&self, name: &str) -> Option<Outcome> {
        Some(match name {
            "ptr_metadata" => Outcome::Value(self.fx.ptr_metadata(self.arg_ty(0), self.arg(0))),
            "read_via_copy" | "volatile_load" | "unaligned_volatile_load" => {
                Outcome::Value(self.read_via_copy())
            }
            "write_via_move" | "volatile_store" | "unaligned_volatile_store"
            | "nontemporal_store" => {
                Outcome::Done(ptr::deref_write(self.fx, self.type_at(0), self.arg(0), self.arg(1)))
            }
            "typed_swap_nonoverlapping" => self.typed_swap(),
            "aggregate_raw_ptr" => self.aggregate_raw_ptr(),
            "slice_get_unchecked" => self.slice_get_unchecked(),
            "transmute" | "transmute_unchecked" => Outcome::Value(codegen_transmute(
                self.fx,
                self.arg_ty(0),
                self.ret_ty(),
                self.arg(0),
            )),
            // Pointer arithmetic, in elements: `off` counts pointee units, so an offset is an
            // addition and a difference is a subtraction. `LowerIntrinsics` rewrites `offset` into
            // `BinOp::Offset` before it reaches here, so that arm serves the crate that declares
            // the intrinsic itself (`mini_core`); `arith_offset` is not rewritten and arrives.
            "offset" | "arith_offset" => match self.pointee_of(0) {
                None => self.not_a_pointer(name, 0),
                Some(pointee) => {
                    let (pointer, count) = (self.arg(0), self.arg(1));
                    Outcome::Value(if name == "offset" {
                        ptr::add(self.fx, pointee, pointer, count)
                    } else {
                        ptr::wrapping_add(self.fx, pointee, pointer, count)
                    })
                }
            },
            "ptr_offset_from" | "ptr_offset_from_unsigned" => match self.pointee_of(0) {
                None => self.not_a_pointer(name, 0),
                Some(pointee) => {
                    Outcome::Value(ptr::offset_from(self.fx, pointee, self.arg(0), self.arg(1)))
                }
            },
            // `copy` takes `(src, dst, count)` and its volatile spelling takes `(dst, src, count)`;
            // the helper takes the destination first, the way `memmove` does.
            "copy" | "copy_nonoverlapping" => self.copy(name, name == "copy", false),
            "volatile_copy_memory" | "volatile_copy_nonoverlapping_memory" => {
                self.copy(name, name == "volatile_copy_memory", true)
            }
            "write_bytes" | "volatile_set_memory" => match self.pointee_of(0) {
                None => self.not_a_pointer(name, 0),
                Some(pointee) => Outcome::Done(ptr::write_bytes(
                    self.fx,
                    pointee,
                    self.arg(0),
                    self.arg(1),
                    self.arg(2),
                )),
            },
            "compare_bytes" => {
                Outcome::Value(ptr::compare_bytes(self.arg(0), self.arg(1), self.arg(2)))
            }
            "raw_eq" => self.raw_eq(),
            // The one pointer operation with no answer here: a mask over an address says something
            // about the bits of a pointer, and a slot has no bits.
            "ptr_mask" => self.unsupported(
                "`ptr_mask` is not supported by rustc_codegen_mires: masking the bits of a pointer \
                 has no meaning in a model where a pointer is a buffer and a key rather than an \
                 address"
                    .to_string(),
            ),
            _ => return None,
        })
    }

    /// `read_via_copy(p)`: the value at `*p`, as `ptr::read` means it — a **copy** of the pointee
    /// that shares nothing with it.
    ///
    /// The clone is the whole point for an aggregate. `*p` is the object the place holds, and a
    /// write of a whole aggregate overwrites that object in place so every reference to it sees
    /// the new contents (`ptr::write_indirect`); a `read` that merely aliased the object would
    /// therefore be *changed* by the next write through any pointer to the same place. That is
    /// exactly the shape `core`'s insertion sort is written in — read an element aside, shift the
    /// run over it, write the saved value back — and aliasing it turns the saved element into
    /// whatever landed in its place.
    fn read_via_copy(&self) -> Expr {
        let tcx = self.fx.tcx;
        let pointee = self.type_at(0);
        let read = ptr::deref_read(self.fx, pointee, self.arg(0));
        if value::needs_clone(tcx, pointee) {
            value::clone_expr(tcx, pointee, read)
        } else {
            read
        }
    }

    /// `typed_swap_nonoverlapping(x, y)`: exchange the two pointees. This is `mem::swap`.
    ///
    /// The MIR fallback swaps *bytes*, one machine word at a time, which needs raw pointer
    /// arithmetic and so would zombie the whole of `mem::swap`. Swapping the two values instead is
    /// both expressible and exactly what the intrinsic means.
    ///
    /// The saved value is *cloned* rather than aliased. An indirect pointee is written by
    /// overwriting the object in place — that is how a write through a reference reaches every
    /// alias — so a temporary that merely pointed at `*x` would be overwritten along with it.
    fn typed_swap(&self) -> Outcome {
        let tcx = self.fx.tcx;
        let pointee = self.type_at(0);
        let (x, y) = (self.arg(0), self.arg(1));
        let read = ptr::deref_read(self.fx, pointee, x.clone());
        let copy = if value::needs_clone(tcx, pointee) {
            value::clone_expr(tcx, pointee, read)
        } else {
            read
        };
        let saved = self.temp(copy);
        let mut statements =
            ptr::deref_write(self.fx, pointee, x, ptr::deref_read(self.fx, pointee, y.clone()));
        statements.extend(ptr::deref_write(self.fx, pointee, y, saved));
        Outcome::Done(statements)
    }

    /// `copy` and `copy_nonoverlapping`, and their volatile spellings.
    ///
    /// `overlapping` picks the helper; `volatile` picks the argument order, because the volatile
    /// intrinsics take the destination first (`memmove`'s order) and the plain ones take the
    /// source first.
    fn copy(&self, name: &str, overlapping: bool, volatile: bool) -> Outcome {
        let (source, target) = if volatile { (1, 0) } else { (0, 1) };
        let Some(pointee) = self.pointee_of(source) else {
            return self.not_a_pointer(name, source);
        };
        // The destination is evaluated first, which is the order the helper takes them in. Both
        // are operand reads of a pointer, so the swap is not observable.
        let (dst, src) = (self.arg(target), self.arg(source));
        Outcome::Done(ptr::copy(self.fx, pointee, dst, src, self.arg(2), overlapping))
    }

    /// `raw_eq(a, b)`: whether the two pointees have the same bytes.
    ///
    /// A scalar has one byte pattern per value, so `===` on the two values answers exactly — and
    /// the arguments are references, which for a scalar pointee are slots.
    ///
    /// An array of scalars is the second case, and it is not a luxury: `[T; N] == [T; N]` on a
    /// bytewise comparable element is *specialized* to a single `raw_eq` of the whole array
    /// (`core::array::equality`), so without this arm every fixed size array of numbers would
    /// compare through a zombie. An array of scalars has no padding — its stride is its element
    /// size — so comparing element by element asks exactly the question the bytes would.
    ///
    /// Anything else is a zombie. Two JavaScript objects with equal fields are `!==`, and
    /// comparing them field by field would answer a *different* question from the one `raw_eq`
    /// asks: padding, and the unread bytes of an enum's smaller variant, are part of its answer.
    fn raw_eq(&self) -> Outcome {
        let tcx = self.fx.tcx;
        let pointee = self.type_at(0);
        if is_zst(tcx, pointee) {
            return Outcome::Value(jsast::boolean(true));
        }
        // A reference to an array is the array itself, and `every` names each element once.
        if let ty::Array(element, _) = pointee.kind() {
            if compares_by_value(*element) {
                let rhs = self.temp(self.arg(1));
                let same = jsast::binary(
                    JsBinOp::StrictEq,
                    jsast::id(RAW_EQ_ELEMENT),
                    jsast::index(rhs, jsast::id(RAW_EQ_INDEX)),
                );
                return Outcome::Value(jsast::method_call(
                    self.arg(0),
                    "every",
                    vec![jsast::arrow(
                        vec![RAW_EQ_ELEMENT.to_string(), RAW_EQ_INDEX.to_string()],
                        same,
                    )],
                ));
            }
        }
        if !compares_by_value(pointee) {
            return self.unsupported(format!(
                "`raw_eq` on `{pointee}` is not supported by rustc_codegen_mires: the bytes of a \
                 value are not observable in a model where a value is a JavaScript object"
            ));
        }
        // A reference to a scalar is a slot, and reading one names the pointer twice.
        let (lhs, rhs) = (self.temp(self.arg(0)), self.temp(self.arg(1)));
        Outcome::Value(jsast::binary(
            JsBinOp::StrictEq,
            ptr::slot_element(lhs),
            ptr::slot_element(rhs),
        ))
    }

    /// `aggregate_raw_ptr(data, meta)`: a pointer built from its two halves.
    ///
    /// `LowerIntrinsics` normally rewrites this into an `AggregateKind::RawPtr` before codegen sees
    /// it, so this is the path for the calls that pass survives. Both end at `ptr::build_fat`,
    /// which owns the rule -- including the sized pointee, whose metadata is `()` and whose result
    /// is the data pointer itself.
    fn aggregate_raw_ptr(&self) -> Outcome {
        let pointer_ty = self.ret_ty();
        let Some(pointee) = pointer_ty.builtin_deref(true) else {
            return self.unsupported(format!("`{pointer_ty}` is not a pointer"));
        };
        Outcome::Value(ptr::build_fat(self.fx, pointee, self.arg(0), self.arg(1)))
    }

    /// `slice_get_unchecked(slice, index)`: a pointer to one element of a slice.
    fn slice_get_unchecked(&self) -> Outcome {
        let slice_ty = self.arg_ty(0);
        let Some(pointee) = slice_ty.builtin_deref(true) else {
            return self.unsupported(format!("`{slice_ty}` is not a pointer"));
        };
        let ty::Slice(element_ty) = pointee.kind() else {
            return self.unsupported(format!(
                "`slice_get_unchecked` on `{pointee}` is not supported by rustc_codegen_mires"
            ));
        };
        // `{ buf, off, len }`: the element lives at `buf[off + i]`, so the pointer to it is the
        // slot naming exactly that. The index needs no temporary of its own — a slot holds the
        // key it was built with, rather than closing over the variable it came from.
        //
        // The intrinsic is generic over the pointer type, so the result is a reference as often as
        // it is a raw pointer, and a reference to an aggregate element is that element's object.
        let slice = self.arg(0);
        let index = self.arg(1);
        let element = ptr::slot(
            ptr::slot_buf(slice.clone()),
            jsast::binary(JsBinOp::Add, ptr::slot_off(slice), index),
        );
        let _ = element_ty;
        Outcome::Value(if ptr::is_slot(self.fx.tcx, self.ret_ty()) {
            element
        } else {
            ptr::slot_element(element)
        })
    }

    // ---------------------------------------------------------------------------------------
    // Atomics
    // ---------------------------------------------------------------------------------------

    /// Atomics, lowered as the plain operations they are on a single threaded runtime.
    ///
    /// JavaScript has one thread per realm and no preemption inside a function, so a
    /// read-modify-write sequence *is* atomic and every memory ordering is a no-op.
    fn atomic(&self, name: &str) -> Option<Outcome> {
        let mut name = name.strip_prefix("atomic_")?;
        // `atomic_cxchg_seqcst_seqcst` carries two orderings, so strip until nothing is left.
        while let Some(stripped) = ORDERINGS
            .iter()
            .find_map(|ordering| name.strip_suffix(ordering).and_then(|s| s.strip_suffix('_')))
        {
            name = stripped;
        }

        if matches!(name, "fence" | "singlethreadfence") {
            return Some(Outcome::Done(Vec::new()));
        }
        let read_modify_write = matches!(
            name,
            "xadd" | "xsub" | "and" | "nand" | "or" | "xor" | "max" | "min" | "umax" | "umin"
        );
        if !read_modify_write && !matches!(name, "load" | "store" | "xchg" | "cxchg" | "cxchgweak")
        {
            return None;
        }

        let pointee = self.type_at(0);
        let pointer = self.temp(self.arg(0));
        let read = || ptr::deref_read(self.fx, pointee, pointer.clone());
        let write = |value: Expr| ptr::deref_write(self.fx, pointee, pointer.clone(), value);

        if read_modify_write {
            let old = self.temp(read());
            let updated = match self.atomic_update(name, pointee, old.clone()) {
                Ok(updated) => updated,
                Err(outcome) => return Some(outcome),
            };
            self.prelude.borrow_mut().extend(write(updated));
            return Some(Outcome::Value(old));
        }

        Some(match name {
            "load" => Outcome::Value(read()),
            "store" => Outcome::Done(write(self.arg(1))),
            "xchg" => {
                let old = self.temp(read());
                self.prelude.borrow_mut().extend(write(self.arg(1)));
                Outcome::Value(old)
            }
            // Returns `(old, succeeded)`, which is a two element JS array.
            _ => {
                if !compares_by_value(pointee) {
                    return Some(self.unsupported(format!(
                        "an atomic compare and exchange on `{pointee}` is not supported by \
                         rustc_codegen_mires: values of that type have no `===` equality"
                    )));
                }
                let old = self.temp(read());
                let matched =
                    self.temp(jsast::binary(JsBinOp::StrictEq, old.clone(), self.arg(1)));
                self.prelude.borrow_mut().push(jsast::if_(matched.clone(), write(self.arg(2))));
                Outcome::Value(jsast::array(vec![old, matched]))
            }
        })
    }

    /// The new value a read-modify-write atomic stores, given the old one.
    fn atomic_update(
        &self,
        name: &str,
        pointee: Ty<'tcx>,
        old: Expr,
    ) -> Result<Expr, Outcome> {
        let operand = self.arg(1);
        if matches!(name, "max" | "min" | "umax" | "umin") {
            if !compares_by_value(pointee) {
                return Err(self.unsupported(format!(
                    "an atomic `{name}` on `{pointee}` is not supported by rustc_codegen_mires"
                )));
            }
            // An unsigned value is already non-negative in both representations — a number is
            // masked with `>>> 0`, a `BigInt` with `asUintN` — so `<` is the right comparison for
            // the signed and unsigned forms alike.
            let take_operand = if name.ends_with("max") {
                jsast::binary(JsBinOp::Gt, operand.clone(), old.clone())
            } else {
                jsast::binary(JsBinOp::Lt, operand.clone(), old.clone())
            };
            return Ok(jsast::cond(take_operand, operand, old));
        }

        let Some((signed, bits)) = int_info(self.fx.tcx, pointee) else {
            return Err(self.unsupported(format!(
                "an atomic `{name}` on `{pointee}` is not supported by rustc_codegen_mires"
            )));
        };
        let repr = int_repr(pointee);
        let combined = match name {
            "xadd" => jsast::binary(JsBinOp::Add, old, operand),
            "xsub" => jsast::binary(JsBinOp::Sub, old, operand),
            "and" => jsast::binary(JsBinOp::BitAnd, old, operand),
            "or" => jsast::binary(JsBinOp::BitOr, old, operand),
            "xor" => jsast::binary(JsBinOp::BitXor, old, operand),
            _ => jsast::bit_not(jsast::binary(JsBinOp::BitAnd, old, operand)),
        };
        Ok(mask(repr, signed, bits, combined))
    }

    // ---------------------------------------------------------------------------------------
    // Plumbing
    // ---------------------------------------------------------------------------------------

    /// One argument, reported against its own span rather than the callee's.
    fn arg(&self, index: usize) -> Expr {
        match self.args.get(index) {
            Some(arg) => self.fx.at(arg.span, || self.fx.codegen_operand(&arg.node)),
            None => self.fx.zombie(format!(
                "an intrinsic was called with {} arguments, but needs at least {}",
                self.args.len(),
                index + 1
            )),
        }
    }

    /// The type of one argument, monomorphized.
    fn arg_ty(&self, index: usize) -> Ty<'tcx> {
        match self.args.get(index) {
            Some(arg) => self.fx.operand_ty(&arg.node),
            None => self.fx.tcx.types.unit,
        }
    }

    /// The pointee of an argument that is a pointer.
    ///
    /// Read off the argument rather than the generic arguments, because the pointer intrinsics do
    /// not agree on what their parameters are: `offset` is generic over the *pointer* type
    /// (`offset<Ptr: BuiltinDeref, Delta>`) while `copy` is generic over the pointee.
    fn pointee_of(&self, index: usize) -> Option<Ty<'tcx>> {
        self.arg_ty(index).builtin_deref(true)
    }

    /// The zombie a pointer intrinsic records when the argument it was handed is not a pointer —
    /// which a `#![no_core]` crate that declares the intrinsic itself can arrange.
    fn not_a_pointer(&self, name: &str, index: usize) -> Outcome {
        self.unsupported(format!(
            "`{name}` was called on `{}`, which is not a pointer",
            self.arg_ty(index)
        ))
    }

    /// One of the intrinsic's generic arguments — where the type it operates on lives for
    /// everything that takes no value of that type (`size_of`, `atomic_load`, ...).
    fn type_at(&self, index: usize) -> Ty<'tcx> {
        self.instance.args.type_at(index)
    }

    /// The type the result is written at.
    fn ret_ty(&self) -> Ty<'tcx> {
        self.fx.monomorphize(self.destination.ty(&self.fx.mir.local_decls, self.fx.tcx).ty)
    }

    /// A `usize` literal, in whichever representation `usize` has on this target.
    fn usize_literal(&self, value: u64) -> Expr {
        value::int_literal(self.fx.tcx, self.ret_ty(), value as u128)
            .unwrap_or_else(|| jsast::num(value as f64))
    }

    /// Evaluates `value` into a hoisted temporary, and returns the expression that reads it.
    fn temp(&self, value: Expr) -> Expr {
        let (statement, name) = self.fx.temp(value);
        self.prelude.borrow_mut().push(statement);
        name
    }

    /// Records an intrinsic this backend cannot lower. The zombie stands in for the value, so the
    /// call site is still a complete lowering.
    fn unsupported(&self, message: String) -> Outcome {
        Outcome::Value(self.fx.zombie(message))
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
        Int { signed: false, ..self }
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
        mask(self.repr, false, self.bits, value)
    }

    /// `lhs * rhs`, exactly. A 32 bit product needs `Math.imul`, because a double cannot hold it.
    fn multiply(self, lhs: Expr, rhs: Expr) -> Expr {
        if self.repr == IntRepr::Number && self.bits == 32 {
            jsast::imul(lhs, rhs)
        } else {
            jsast::binary(JsBinOp::Mul, lhs, rhs)
        }
    }

    /// A right shift with this type's signedness. A `BigInt` has no `>>>` and needs none: an
    /// unsigned value is a non-negative `BigInt`, for which the arithmetic shift is the logical
    /// one.
    fn shift_right(self, value: Expr, amount: Expr) -> Expr {
        let op = if self.signed || self.repr == IntRepr::BigInt {
            JsBinOp::Shr
        } else {
            JsBinOp::UShr
        };
        jsast::binary(op, value, amount)
    }

    /// The smallest and largest values of the type.
    fn range(self) -> (Expr, Expr) {
        match (self.repr, self.signed, self.bits) {
            // `u128::MAX` does not fit an `i128`, so the widest unsigned range is spelled apart.
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

/// Whether `===` on two values of this type answers the question Rust asks.
///
/// A reference is an accessor object built fresh at every borrow, so comparing two of them
/// compares identities that mean nothing. Numbers, `BigInt`s, booleans and `char`s compare by
/// value.
fn compares_by_value(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Bool | ty::Char | ty::FnPtr(..)
    )
}

// -------------------------------------------------------------------------------------------
// `#[track_caller]`
// -------------------------------------------------------------------------------------------

/// The caller location argument a call at `span` passes to a `#[track_caller]` callee.
///
/// The mechanism is three pieces, and this is the middle one:
///
/// 1. `base.rs` appends [`CALLER_LOCATION_PARAM`] to the parameter list of a function whose
///    `instance.def.requires_caller_location(tcx)` is true;
/// 2. `abi.rs` (and `base.rs`, for the panic lang items an `Assert` calls) appends this value to
///    the argument list of a call to such a function;
/// 3. the `caller_location` intrinsic reads the parameter back (see
///    [`Intrinsic::caller_location`]).
///
/// A `#[track_caller]` function passes its *own* `$loc` straight through, which is what makes a
/// location propagate down a chain of them — `panic_bounds_check` reports the indexing expression
/// rather than its own body.
pub(crate) fn caller_location_argument<'tcx>(fx: &FnCx<'_, 'tcx>, span: Span) -> Expr {
    if fx.instance.def.requires_caller_location(fx.tcx) {
        return jsast::id(CALLER_LOCATION_PARAM);
    }
    caller_location_value(fx, span)
}

/// The `Location` of `span`, as a JavaScript value.
///
/// The location type is whatever the `panic_location` lang item is, and its first three fields are
/// the file, the line and the column in that order — `rustc_const_eval`'s `alloc_caller_location`
/// writes them by index, so the order is a guarantee rather than a guess.
pub(crate) fn caller_location_value<'tcx>(fx: &FnCx<'_, 'tcx>, span: Span) -> Expr {
    let tcx = fx.tcx;
    if tcx.lang_items().get(rustc_hir::LangItem::PanicLocation).is_none() {
        return fx.zombie(
            "a caller location is needed, but this crate defines no `panic_location` lang item"
                .to_string(),
        );
    }
    let pointer_ty = tcx.caller_location_ty();
    let Some(location_ty) = pointer_ty.builtin_deref(true) else {
        return fx.zombie(format!("the caller location type `{pointer_ty}` is not a reference"));
    };
    let ty::Adt(def, args) = location_ty.kind() else {
        return fx.zombie(format!("the caller location type `{location_ty}` is not a struct"));
    };
    let variant = def.non_enum_variant();
    if variant.fields.len() < 3 {
        return fx.zombie(format!("the caller location type `{location_ty}` has no file field"));
    }

    // The file name field is a *pointer to* `str`, but not always a plain `&'static str`: real
    // `core` stores a `NonNull<str>` so that `Location::file_as_c_str` can read one byte past the
    // end, and `mini_core` stores the reference directly. Both are the same JavaScript string
    // behind zero or more single-field wrappers, so the wrappers are put back the same way a
    // `transmute` into the field's type would put them back.
    let file_ty =
        tcx.normalize_erasing_regions(typing_env(), variant.fields[FieldIdx::ZERO].ty(tcx, args));
    let (inner_file_ty, wrap_file) = target_wrappers(fx, file_ty);
    let points_at_str = value::peel_pattern(inner_file_ty)
        .builtin_deref(true)
        .is_some_and(|pointee| pointee.is_str());
    if !points_at_str {
        return fx.zombie(format!(
            "the caller location type `{location_ty}` stores its file name as `{file_ty}`, which \
             is not supported by rustc_codegen_mires"
        ));
    }

    // The outermost macro expansion, which is the location a user recognises.
    let topmost = span.ctxt().outer_expn().expansion_cause().unwrap_or(span);
    let caller = tcx.sess.source_map().lookup_char_pos(topmost.lo());
    let file = caller.file.name.prefer_remapped_unconditionally().to_string();
    let line = caller.line as u32;
    let column = caller.col_display as u32 + 1;

    // A location is by far the most repeated constant a program has — one per `#[track_caller]`
    // call, most of them the same handful of sites, each carrying a whole file path — so it is
    // hoisted into a shared `const` rather than written out at every call. The key is the location
    // itself: the file spelling comes from `--remap-path-prefix` and so is the same in every crate
    // of a program, which is what lets the location `core` and the user's crate both record for an
    // inlined call collapse to one constant at link time.
    //
    // Only the three fields go in the key. The location *type* is the `panic_location` lang item,
    // of which a program has exactly one, so the field names and the wrappers around the file are
    // fixed for the whole program and cannot make two crates disagree about the value.
    fx.cgu.hoist(
        crate::naming::LOCATION_PREFIX,
        &format!("loc\u{1}{file}\u{1}{line}\u{1}{column}"),
        format!("caller location {file}:{line}:{column}"),
        || {
            jsast::object(vec![
                (Namer::field_name(variant, FieldIdx::ZERO), wrap_file(jsast::string(file.clone()))),
                (Namer::field_name(variant, FieldIdx::from_u32(1)), jsast::num(line as f64)),
                (Namer::field_name(variant, FieldIdx::from_u32(2)), jsast::num(column as f64)),
            ])
        },
    )
}

// -------------------------------------------------------------------------------------------
// `transmute`
// -------------------------------------------------------------------------------------------

/// `transmute::<From, To>(value)`, for the shapes a value based model can honour.
///
/// A transmute reinterprets bytes, and this backend has no bytes. What it does have is a small set
/// of cases where the *value* is unchanged, or is related to the target by a computation:
///
/// * the same representation class on both sides — two integers of the same width, `char` and
///   `u32`, `bool` and `u8`, two pointers of the same fatness — where the value is carried across
///   and re-masked;
/// * a wrapper with exactly one non-zero-sized field, unwrapped on the way in and rebuilt on the
///   way out, which is what makes `MaybeUninit<T>`, `ManuallyDrop<T>`, `NonNull<T>` and newtypes
///   work in both directions;
/// * a float and its bit pattern, through a pair of typed array views.
///
/// Anything else is a zombie: reinterpreting one struct as another has no meaning here, and
/// guessing would be a silent miscompilation.
///
/// The `transmute` intrinsic arm above comes here — but rustc's `LowerIntrinsics` pass rewrites
/// every `transmute` into a `CastKind::Transmute` cast long before codegen, so in practice the
/// live entry point is `rvalue.rs`'s cast arm, which still zombies. Pointing it here is one line:
///
/// ```ignore
/// CastKind::Transmute => crate::intrinsics::codegen_transmute(self, from_ty, to_ty, value),
/// ```
#[allow(dead_code)]
pub(crate) fn codegen_transmute<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
    value: Expr,
) -> Expr {
    if from_ty == to_ty {
        return value;
    }
    // Peel the single field wrappers off the source, then rebuild the ones the destination has.
    // Doing it in that order keeps the two sides independent, so `MaybeUninit<u32>` to a newtype
    // over `u32` needs no case of its own.
    let (inner_from, inner_value) = unwrap_wrappers(fx, from_ty, value);
    let (inner_to, rebuild) = target_wrappers(fx, to_ty);
    rebuild(transmute_scalar(fx, inner_from, inner_to, inner_value, from_ty, to_ty))
}

/// The scalar half of a transmute, once both sides have had their wrappers removed. `from_ty` and
/// `to_ty` are the original types, for the diagnostic.
fn transmute_scalar<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    inner_from: Ty<'tcx>,
    inner_to: Ty<'tcx>,
    value: Expr,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
) -> Expr {
    let tcx = fx.tcx;
    if inner_from == inner_to {
        return value;
    }
    // Two arrays of the same length whose elements have the same JavaScript representation *are*
    // the same array. `[T; N]` to `[MaybeUninit<T>; N]` is the shape `array::IntoIter` is written
    // as, and with the transparent wrappers peeled (`value.rs`) both are an array of the same
    // values; the wrapper peeling above cannot see it, because an array of more than one element
    // is not a wrapper around anything.
    if let (ty::Array(from_element, from_len), ty::Array(to_element, to_len)) =
        (inner_from.kind(), inner_to.kind())
    {
        let peel = |ty| value::peel_pattern(value::peel_transparent(tcx, ty));
        if from_len == to_len && peel(*from_element) == peel(*to_element) {
            return value;
        }
    }
    // A zero sized value carries no bits, and is `undefined`.
    if is_zst(tcx, inner_to) {
        return jsast::undefined();
    }
    // `str::as_bytes` and `str::from_utf8_unchecked` are both written as a `transmute` between two
    // pointers, and neither is one here: a `str` is a JavaScript string, so its byte view is
    // encoded and decoded rather than reinterpreted. `ptr::cast_str_ptr` answers `None` unless one
    // of the two pointees is a `str`, which leaves every other pointer pair to the arms below.
    if let Some(converted) = crate::ptr::cast_str_ptr(fx, inner_from, inner_to, value.clone()) {
        return converted;
    }

    match (ReprClass::of(tcx, inner_from), ReprClass::of(tcx, inner_to)) {
        // Two integers of the same width: the same bits, read with the destination's signedness.
        (ReprClass::Int(from), ReprClass::Int(to)) if from.bits == to.bits => {
            mask(to.repr, to.signed, to.bits, value)
        }
        (ReprClass::Bool, ReprClass::Int(to)) => {
            mask(to.repr, to.signed, to.bits, jsast::cond(value, jsast::num(1), jsast::num(0)))
        }
        (ReprClass::Int(_), ReprClass::Bool) => {
            jsast::binary(JsBinOp::StrictNe, value, jsast::num(0))
        }
        // A `char` is its scalar value, which is exactly what a `u32` holds.
        (ReprClass::Char, ReprClass::Int(to)) => mask(to.repr, to.signed, to.bits, value),
        (ReprClass::Int(_), ReprClass::Char) => value,
        (ReprClass::Float(from_bits), ReprClass::Int(to)) if from_bits == to.bits => {
            bit_cast(float_array(from_bits), int_array(to), value)
        }
        (ReprClass::Int(from), ReprClass::Float(to_bits)) if from.bits == to_bits => {
            bit_cast(int_array(from), float_array(to_bits), value)
        }
        // Two pointers: the same question `CastKind::PtrToPtr` asks, and the same answer, so it
        // is the same function — `ptr::cast_pointer` owns the whole matrix. A fn pointer has no
        // pointee to ask about and is a JavaScript function whichever pointer type it is spelled
        // as, so it is carried across as it is.
        (ReprClass::Pointer(_), ReprClass::Pointer(_)) => {
            match (
                value::peel_pattern(inner_from).builtin_deref(true),
                value::peel_pattern(inner_to).builtin_deref(true),
            ) {
                (Some(from_pointee), Some(to_pointee)) => {
                    crate::ptr::cast_pointer(fx, from_pointee, to_pointee, value)
                }
                _ => value,
            }
        }
        // A thin pointer and a pointer sized integer. Rust itself calls the integer direction
        // "exposing an address" and the pointer direction a pointer *without provenance*: the
        // result is a pointer that must never be dereferenced, only compared or turned back into
        // an integer. An integer-born pointer (`without_provenance`, `dangling`) is a number and
        // `__rt.addr` passes it through, so that round trip stays exact — which is what
        // tagged-pointer code such as `fmt::Arguments` relies on. A real pointer gets a synthetic
        // base per buffer (see `ptr::addr`'s guarantees), so `is_null()` is false for it and
        // alignment predicates answer correctly.
        (ReprClass::Pointer(true), ReprClass::Int(to)) if to.bits == pointer_bits(tcx) => {
            let address = match inner_from.builtin_deref(true) {
                Some(pointee) => crate::ptr::addr(fx, pointee, value),
                None => value,
            };
            mask(to.repr, to.signed, to.bits, address)
        }
        (ReprClass::Int(from), ReprClass::Pointer(true)) if from.bits == pointer_bits(tcx) => value,
        // `[u8; N]` and `uN`, little endian: `u32::from_le_bytes` and `u16::to_le_bytes` are
        // written as exactly this transmute, and `core::fmt`'s integer formatting reads its digit
        // buffer through `cast_array().read()` on the way to one. Nothing else in this backend can
        // do it: a byte buffer has no place holding the integer its bytes spell, which is why the
        // pointer matrix in `ptr.rs` sends the question here instead of answering it.
        (ReprClass::Other, ReprClass::Int(to))
            if matches!(to.bits, 16 | 32 | 64)
                && byte_array_len(tcx, inner_from) == Some(u64::from(to.bits / 8)) =>
        {
            bytes_to_int(to, value)
        }
        (ReprClass::Int(from), ReprClass::Other)
            if matches!(from.bits, 16 | 32 | 64)
                && byte_array_len(tcx, inner_to) == Some(u64::from(from.bits / 8)) =>
        {
            int_to_bytes(from, value)
        }
        // A **fieldless enum with an integer `repr`** and the integer of its own width. Such a
        // value *is* its discriminant, in this model as on a real machine (`value::EnumRepr`), so
        // moving between the two moves nothing at all. `core` transmutes both ways: `u8` to
        // `AsciiChar` on the ASCII escaping path, and `Alignment` to `usize` under
        // `NonNull::dangling`.
        //
        // The widths and the signedness have to agree exactly. Where they do, the number is already
        // in range for the destination and needs no mask; where they do not, this says nothing and
        // the rows below report. A fieldless enum *without* an integer `repr` is a variant name
        // rather than a number, and no row here claims it: the discriminant such a transmute would
        // be observing is not part of the value, so the zombie at the end reports instead.
        (ReprClass::Other, ReprClass::Int(to))
            if value::direct_tag(tcx, inner_from) == Some((u64::from(to.bits), to.signed)) =>
        {
            value
        }
        (ReprClass::Int(from), ReprClass::Other)
            if value::direct_tag(tcx, inner_to) == Some((u64::from(from.bits), from.signed)) =>
        {
            value
        }
        // The same value seen as a *pointer* rather than as an integer. `NonNull::dangling()` is
        // `without_provenance(align)` and is written as a transmute of an `Alignment` straight to a
        // pointer, with no `usize` in between. An address is a JavaScript number here and such an
        // enum is that number, so the two meet: the result is an address, never dereferenceable,
        // which is exactly what a dangling pointer is. Every allocation goes through one, because
        // an empty `Vec` *is* one.
        (ReprClass::Other, ReprClass::Pointer(true))
            if value::direct_tag(tcx, inner_from)
                == Some((u64::from(pointer_bits(tcx)), false)) =>
        {
            value
        }
        _ => match niche_from_scalar(fx, inner_from, inner_to, value) {
            Some(decoded) => decoded,
            None => fx.zombie(format!(
                "`transmute` from `{from_ty}` to `{to_ty}` is not supported by rustc_codegen_mires"
            )),
        },
    }
}

/// The parameter the byte conversions bind their operand to, so that it is evaluated once.
const BYTES_PARAM: &str = "$b";

/// The length of `ty` if it is an array of bytes, and `None` otherwise.
///
/// A transparent union element counts: `[MaybeUninit<u8>; N]` holds the same JavaScript numbers
/// `[u8; N]` does (`value.rs`).
fn byte_array_len<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<u64> {
    let ty::Array(element, count) = value::peel_pattern(ty).kind() else { return None };
    let element = value::peel_pattern(value::peel_transparent(tcx, *element));
    matches!(element.kind(), ty::Uint(ty::UintTy::U8)).then(|| count.try_to_target_usize(tcx))?
}

/// `[u8; N]` to the integer those bytes spell, little endian: `b[0] | b[1] << 8 | ...`.
///
/// The shifted bytes are combined with `|`, which is an int32 operation, and the destination's
/// mask then reads the result at its own width and signedness — so `u32` comes out `>>> 0` and
/// `i32` comes out `| 0`.
///
/// A **64 bit** destination is a `BigInt` (`value.rs`), and the int32 operators would silently
/// throw the top four bytes away, so each byte is widened first and the shifts are `BigInt` ones.
/// That is what `str::parse::<f64>()` reaches: `dec2flt` reads its digits eight at a time through
/// `ByteSlice::read_u64`, which is exactly this transmute.
fn bytes_to_int(int: Int, value: Expr) -> Expr {
    let bytes = jsast::id(BYTES_PARAM);
    let big = int.repr == crate::value::IntRepr::BigInt;
    let byte = |index: u32| {
        let read = jsast::index(bytes.clone(), jsast::num(index));
        if big { jsast::call(jsast::id("BigInt"), vec![read]) } else { read }
    };
    let shift = |amount: u32| if big { jsast::bigint(amount) } else { jsast::num(amount) };
    let mut combined = byte(0);
    for index in 1..int.bits / 8 {
        let shifted = jsast::binary(JsBinOp::Shl, byte(index), shift(index * 8));
        combined = jsast::binary(JsBinOp::BitOr, combined, shifted);
    }
    let combined = mask(int.repr, int.signed, int.bits, combined);
    jsast::call(jsast::arrow(vec![BYTES_PARAM.to_string()], combined), vec![value])
}

/// The other direction: the `N` bytes of an integer, least significant first.
///
/// `>>>` reads its operand as a `u32` first, so a negative source needs no separate arm — the
/// shift already sees the bit pattern, and `& 255` takes the byte out of it.
///
/// A `BigInt` source shifts with `>>`, which is the only shift `BigInt` has and is arithmetic; that
/// is the same answer, because a `BigInt` behaves as an infinitely sign extended two's complement
/// number and `& 255n` reads the byte out of it either way. Each byte then comes back through
/// `Number`, because the elements of a `[u8; N]` are JavaScript numbers.
fn int_to_bytes(int: Int, value: Expr) -> Expr {
    let bound = jsast::id(BYTES_PARAM);
    let big = int.repr == crate::value::IntRepr::BigInt;
    let bytes = (0..int.bits / 8)
        .map(|index| {
            let shifted = if index == 0 {
                bound.clone()
            } else if big {
                jsast::binary(JsBinOp::Shr, bound.clone(), jsast::bigint(index * 8))
            } else {
                jsast::binary(JsBinOp::UShr, bound.clone(), jsast::num(index * 8))
            };
            let mask = if big { jsast::bigint(255u32) } else { jsast::num(255) };
            let byte = jsast::binary(JsBinOp::BitAnd, shifted, mask);
            if big { jsast::call(jsast::id("Number"), vec![byte]) } else { byte }
        })
        .collect();
    jsast::call(jsast::arrow(vec![BYTES_PARAM.to_string()], jsast::array(bytes)), vec![value])
}

/// The parameter the niche decode binds the incoming scalar to.
const NICHE_PARAM: &str = "$n";

/// A `transmute` from an integer into a niche encoded two variant enum, decoded at run time.
///
/// This is how `NonZero::<T>::new` is written — `transmute::<u32, Option<NonZero<u32>>>(n)` — and
/// through it how `i32::pow`, `next_power_of_two` and a good deal of `core::num` are written. The
/// bit pattern is the payload for every value except one, the *niche*, which stands for the empty
/// variant; the two are told apart by a comparison and rebuilt into the ordinary shape `value.rs`
/// gives that enum.
///
/// Deliberately narrow. It applies only when
///
/// * the destination is an enum whose tag is niche encoded and as wide as the source integer;
/// * exactly **one** variant is niche encoded, so the comparison has a single answer (with several,
///   the discriminant would be a function of the value rather than a constant, and this would have
///   to build an arithmetic expression instead);
/// * that variant carries no data, and the other carries exactly one non-zero-sized field, which
///   the payload is transmuted into recursively.
///
/// `Option<NonZero<T>>`, `Option<&T>` and `Option<char>` all satisfy that; anything else stays a
/// zombie rather than a guess.
fn niche_from_scalar<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
    value: Expr,
) -> Option<Expr> {
    use rustc_abi::{TagEncoding, Variants};

    let tcx = fx.tcx;
    let (signed, bits) = int_info(tcx, from_ty)?;
    let int = Int { repr: int_repr(from_ty), signed, bits };

    let ty::Adt(def, args) = to_ty.kind() else { return None };
    if !def.is_enum() {
        return None;
    }
    let layout = tcx.layout_of(typing_env().as_query_input(to_ty)).ok()?;
    let Variants::Multiple {
        tag,
        tag_encoding: TagEncoding::Niche { untagged_variant, niche_variants, niche_start },
        ..
    } = &layout.variants
    else {
        return None;
    };
    if tag.size(&tcx).bits() as u32 != bits || niche_variants.start != niche_variants.last {
        return None;
    }

    let empty_index = niche_variants.start;
    let empty = def.variant(empty_index);
    let normalize = |ty| tcx.normalize_erasing_regions(typing_env(), ty);
    let live = |ty| !is_zst(tcx, normalize(ty));
    if empty.fields.iter().any(|field| live(field.ty(tcx, args))) {
        return None;
    }

    let payload = def.variant(*untagged_variant);
    let mut fields =
        payload.fields.iter_enumerated().filter(|(_, field)| live(field.ty(tcx, args)));
    let (field_index, field) = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    let field_ty = normalize(field.ty(tcx, args));

    // The niche is the one value that is not a payload. With a single niche variant the tag *is*
    // `niche_start`, so the test is an equality rather than the range check the general encoding
    // would need.
    let bound = jsast::id(NICHE_PARAM);
    let niche = int.literal(i128::try_from(*niche_start).ok()?);
    let empty_value = value::enum_value(tcx, to_ty, empty_index, Vec::new())?;
    let filled = value::enum_value(
        tcx,
        to_ty,
        *untagged_variant,
        vec![(
            Namer::field_name(payload, field_index),
            codegen_transmute(fx, from_ty, field_ty, bound.clone()),
        )],
    )?;
    let decoded =
        jsast::cond(jsast::binary(JsBinOp::StrictEq, bound, niche), empty_value, filled);
    Some(jsast::call(jsast::arrow(vec![NICHE_PARAM.to_string()], decoded), vec![value]))
}

/// How a value is represented, at the granularity a transmute cares about.
#[derive(Clone, Copy)]
enum ReprClass {
    Int(Int),
    Float(u32),
    Bool,
    Char,
    /// A pointer, with whether it is thin.
    Pointer(bool),
    Other,
}

/// The target's pointer width in bits, which `wasm32-unknown-unknown` makes 32.
fn pointer_bits(tcx: TyCtxt<'_>) -> u32 {
    tcx.data_layout.pointer_size().bits() as u32
}

impl ReprClass {
    fn of<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> ReprClass {
        // A pattern type is represented exactly like its base: `NonNull<T>` wraps
        // `pattern_type!(*const T is !null)`, and that is a pointer.
        let ty = value::peel_pattern(ty);
        match ty.kind() {
            ty::Bool => ReprClass::Bool,
            ty::Char => ReprClass::Char,
            ty::Float(ty::FloatTy::F32) => ReprClass::Float(32),
            ty::Float(ty::FloatTy::F64) => ReprClass::Float(64),
            ty::Int(_) | ty::Uint(_) => match int_info(tcx, ty) {
                Some((signed, bits)) => ReprClass::Int(Int { repr: int_repr(ty), signed, bits }),
                None => ReprClass::Other,
            },
            ty::FnPtr(..) => ReprClass::Pointer(true),
            ty::Ref(..) | ty::RawPtr(..) => match ty.builtin_deref(true) {
                Some(pointee) => ReprClass::Pointer(pointee.is_sized(tcx, typing_env())),
                None => ReprClass::Other,
            },
            _ => ReprClass::Other,
        }
    }
}

/// The typed array constructor holding a float of this width.
fn float_array(bits: u32) -> &'static str {
    if bits == 32 { "Float32Array" } else { "Float64Array" }
}

/// The typed array constructor holding an integer of this width and signedness.
fn int_array(int: Int) -> &'static str {
    match (int.signed, int.bits) {
        (true, 32) => "Int32Array",
        (false, 32) => "Uint32Array",
        (true, _) => "BigInt64Array",
        (false, _) => "BigUint64Array",
    }
}

/// `new To(new From([value]).buffer)[0]`: the bits of `value`, read back as another type.
///
/// One scratch buffer per call is wasteful, and deliberately so for now — a shared buffer would
/// have to live in `runtime/shim.js`, which several workstreams append to at once. The obvious
/// follow up is a pair of `__rt.f32_bits`/`__rt.bits_f32` helpers over one module level
/// `DataView`.
fn bit_cast(from: &str, to: &str, value: Expr) -> Expr {
    let source = jsast::new(jsast::id(from), vec![jsast::array(vec![value])]);
    let view = jsast::new(jsast::id(to), vec![jsast::member(source, "buffer")]);
    jsast::index(view, jsast::num(0))
}

/// Strips every single-non-ZST-field wrapper off a value.
fn unwrap_wrappers<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    mut ty: Ty<'tcx>,
    mut value: Expr,
) -> (Ty<'tcx>, Expr) {
    // A wrapper chain is as deep as the type nests, which is finite; the bound is belt and braces
    // against a shape this loop has not been taught about.
    for _ in 0..16 {
        let Some((inner_ty, key)) = sole_field(fx, ty) else { break };
        value = value::field_expr(value, key);
        ty = inner_ty;
    }
    (ty, value)
}

/// The destination's wrappers: the innermost type, and the closure that puts the wrappers back
/// around a value of it.
fn target_wrappers<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    mut ty: Ty<'tcx>,
) -> (Ty<'tcx>, impl Fn(Expr) -> Expr) {
    let mut wrappers = Vec::new();
    for _ in 0..16 {
        let Some((inner_ty, key)) = sole_field(fx, ty) else { break };
        wrappers.push(key);
        ty = inner_ty;
    }
    let rebuild = move |mut value: Expr| {
        for key in wrappers.iter().rev() {
            value = match key {
                // A struct is an object keyed by field name; the zero sized fields that were
                // skipped are `undefined`, which is what reading a missing key gives anyway.
                value::FieldKey::Name(name) => jsast::object(vec![(name.clone(), value)]),
                value::FieldKey::Index(_) => jsast::array(vec![value]),
                // A transparent union is its field: there is no wrapper to put back.
                value::FieldKey::Transparent => value,
            };
        }
        value
    };
    (ty, rebuild)
}

/// The one field of a wrapper type that carries its bits, if there is exactly one.
///
/// A struct or union with a single non-zero-sized field, a one element tuple or a one element
/// array: `MaybeUninit<T>`, `ManuallyDrop<T>`, `NonNull<T>`, a newtype.
///
/// `ptr.rs`'s cast matrix walks the same chain to decide whether a pointer to a wrapper can be
/// reprojected into a pointer to what it wraps.
pub(crate) fn sole_field<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    ty: Ty<'tcx>,
) -> Option<(Ty<'tcx>, value::FieldKey)> {
    let tcx = fx.tcx;
    if is_zst(tcx, ty) {
        return None;
    }
    // A transparent union is represented as its field, so the field is reached by doing nothing:
    // peeling it off a value is the identity, and putting it back around one is too.
    if let Some(field_ty) = value::transparent_field(tcx, ty) {
        return Some((field_ty, value::FieldKey::Transparent));
    }
    match ty.kind() {
        ty::Adt(def, args) if def.is_struct() || def.is_union() => {
            let variant = def.non_enum_variant();
            let mut live = variant.fields.iter_enumerated().filter(|(_, field)| {
                !is_zst(tcx, tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args)))
            });
            let (index, field) = live.next()?;
            if live.next().is_some() {
                return None;
            }
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
            Some((field_ty, value::FieldKey::Name(Namer::field_name(variant, index))))
        }
        ty::Tuple(element_tys) => {
            let mut live =
                element_tys.iter().enumerate().filter(|(_, element)| !is_zst(tcx, *element));
            let (index, element_ty) = live.next()?;
            if live.next().is_some() {
                return None;
            }
            Some((element_ty, value::FieldKey::Index(index)))
        }
        ty::Array(element_ty, count) => (count.try_to_target_usize(tcx) == Some(1))
            .then(|| (*element_ty, value::FieldKey::Index(0))),
        _ => None,
    }
}
