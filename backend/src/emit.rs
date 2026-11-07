//! The region tree as JavaScript statements.
//!
//! `cfg_adapter.rs` hands the structurizer a graph of lowered blocks and it hands back a
//! [`Region`] tree: loops, labeled blocks, ifs and switches, with the labels already resolved —
//! every label in the tree is named by some jump, and every jump that needs one carries it. This
//! module prints that tree and nothing more; it makes no control flow decisions of its own.
//!
//! What it *does* own is the spelling: the names of the labels and the compiler temporaries, and
//! how a [`Test`] reads in JavaScript. See [`Emitter::test`] for the tests, and `cfg_adapter`'s
//! module documentation for why the values a test carries are biased rather than raw.
//!
//! # Names
//!
//! Labels are `L0`, `L1`, ..., numbered by the structurizer. Compiler temporaries — a bound
//! scrutinee, a dispatch selector — are `$s0`, `$s1`, ...; the `$` prefix is the backend's
//! namespace (a Rust identifier cannot contain one) and puts them out of reach of the `_N` locals
//! and of the `$tN` temporaries `place.rs` and `abi.rs` hoist. They are *declared* in the function
//! prelude, never at the use site: the cases of a dispatch's `switch` share one block scope, so a
//! `let` inside one would collide with the same `let` in another.
//!
//! # Dispatch
//!
//! [`Region::Dispatch`] is the spike's `switch (bb)` trampoline, and the whole body becomes one
//! when `-Cllvm-args=js-structure=trampoline` is passed, when the graph is irreducible, or when a
//! group of scopes would nest deeper than the structurizer's limit. Falling off the end of its
//! `switch` would silently return `undefined`, so it keeps the guard the trampoline had: every
//! selector value the backend assigns names a case, and reaching `default` says so.

use std::collections::BTreeMap;

use rustc_middle::mir::RETURN_PLACE;
use rustc_middle::ty::{self, Ty};
use structurizer::{Label, Region, Scrut, Test, TmpId};

use crate::base::FnCx;
use crate::cfg_adapter::{self, ScrutTy, Scrutinee};
use crate::jsast::{self, BinOp, Expr, Stmt};
use crate::value::{self, IntRepr};

/// The body of one function, and the names of the temporaries its prelude has to declare.
pub(crate) fn emit<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    region: Region<Vec<Stmt>, Scrutinee<'tcx>>,
) -> (Vec<Stmt>, Vec<String>) {
    let mut emitter = Emitter { fx, tmps: BTreeMap::new() };
    let body = emitter.region(region);
    let tmps = emitter.tmps.keys().map(|tmp| tmp_name(TmpId(*tmp))).collect();
    (body, tmps)
}

fn label_name(label: Label) -> String {
    format!("L{}", label.0)
}

/// What a structurizer temporary is called. `queue.rs` needs it too: an assignment to one of these
/// is a write like any other, and a queued entry that reads it has to be flushed before it.
pub(crate) fn tmp_name(tmp: TmpId) -> String {
    format!("$s{}", tmp.0)
}

struct Emitter<'a, 'b, 'tcx> {
    fx: &'a FnCx<'b, 'tcx>,
    /// Every temporary the tree names, with the scrutinee type of the ones a test reads back.
    tmps: BTreeMap<u32, Option<ScrutTy<'tcx>>>,
}

impl<'tcx> Emitter<'_, '_, 'tcx> {
    fn region(&mut self, region: Region<Vec<Stmt>, Scrutinee<'tcx>>) -> Vec<Stmt> {
        match region {
            Region::Seq(regions) => regions.into_iter().flat_map(|r| self.region(r)).collect(),
            Region::Block(stmts) => stmts,
            Region::Labeled { label, body } => {
                let body = self.region(*body);
                vec![jsast::labeled(label_name(label), Stmt::Block(body))]
            }
            Region::Loop { label, body } => vec![self.loop_(label, *body)],
            Region::LetTmp { tmp, value } => {
                self.tmps.insert(tmp.0, Some(value.ty));
                vec![jsast::assign_stmt(jsast::id(tmp_name(tmp)), value.value)]
            }
            Region::If { scrut, test, then_, else_ } => self.if_(scrut, test, *then_, *else_),
            Region::Switch { scrut, arms, default, label } => {
                self.switch(scrut, arms, *default, label)
            }
            Region::Dispatch { sel, label, cases, trailing_break } => {
                vec![self.dispatch(sel, label, cases, trailing_break)]
            }
            Region::SetSel(tmp, value) => {
                self.tmps.entry(tmp.0).or_insert(None);
                vec![jsast::assign_stmt(jsast::id(tmp_name(tmp)), jsast::num(value))]
            }
            Region::Break(label) => vec![Stmt::Break(label.map(label_name))],
            Region::Continue(label) => vec![Stmt::Continue(label.map(label_name))],
            // `local_expr`, not the bare name: the return place can be boxed like any other
            // local, in which case what it holds is `_0[0]`.
            Region::Return => vec![jsast::ret(self.fx.local_expr(RETURN_PLACE))],
            Region::Unreachable => {
                vec![jsast::throw_error("rustc_codegen_mires: entered unreachable code")]
            }
            Region::Throw => {
                vec![jsast::throw_error("rustc_codegen_mires: unwinding is not supported")]
            }
        }
    }

    fn loop_(&mut self, label: Option<Label>, body: Region<Vec<Stmt>, Scrutinee<'tcx>>) -> Stmt {
        let mut body = self.region(body);
        // `for (;;) { if (c) { .. } else break; }` is the `while` the source had. Both forms run
        // the body while `c` holds, and both capture the same `break`s and `continue`s, so the
        // rewrite is local to this statement.
        match as_while(&mut body) {
            Some((test, inner)) => match label {
                Some(label) => jsast::labeled(label_name(label), jsast::while_(test, inner)),
                None => jsast::while_(test, inner),
            },
            None => Stmt::LoopForever(label.map(label_name), body),
        }
    }

    fn if_(
        &mut self,
        scrut: Scrut<Scrutinee<'tcx>>,
        test: Test,
        then_: Region<Vec<Stmt>, Scrutinee<'tcx>>,
        else_: Region<Vec<Stmt>, Scrutinee<'tcx>>,
    ) -> Vec<Stmt> {
        let then_ = self.region(then_);
        let else_ = self.region(else_);
        let inline = matches!(scrut, Scrut::Payload(_));
        let (value, scrut_ty) = self.scrutinee(scrut);
        match (then_.is_empty(), else_.is_empty()) {
            (false, true) => vec![jsast::if_(self.test(value, scrut_ty, test, false), then_)],
            // An empty consequent reads better as the negated test than as `if (c) {}`, and every
            // test has an exact negation: the scrutinee of a `SwitchInt` is an integer, a `char`
            // or a boolean, never a float, so nothing here can be NaN.
            (true, false) => vec![jsast::if_(self.test(value, scrut_ty, test, true), else_)],
            (false, false) => {
                vec![jsast::if_else(self.test(value, scrut_ty, test, false), then_, else_)]
            }
            // Nothing left to choose between. An inline scrutinee still has to be evaluated for
            // whatever effects it has; one already bound to a temporary has been.
            (true, true) if inline => vec![jsast::expr_stmt(value)],
            (true, true) => Vec::new(),
        }
    }

    fn switch(
        &mut self,
        scrut: Scrut<Scrutinee<'tcx>>,
        arms: Vec<(Vec<u128>, Region<Vec<Stmt>, Scrutinee<'tcx>>)>,
        default: Region<Vec<Stmt>, Scrutinee<'tcx>>,
        label: Option<Label>,
    ) -> Vec<Stmt> {
        let scrut_ty = match &scrut {
            Scrut::Payload(scrutinee) => scrutinee.ty,
            Scrut::Tmp(tmp) => self.tmp_ty(*tmp),
        };
        // The arms run in order and fall through into each other exactly as in JavaScript: an arm
        // that can reach its end already carries the `break` that stops it.
        let mut cases: Vec<_> = arms
            .into_iter()
            .map(|(values, body)| {
                let body = self.region(body);
                let tests = values.into_iter().map(|v| self.case_literal(scrut_ty, v)).collect();
                jsast::case(tests, body)
            })
            .collect();
        let default = self.region(default);
        cases.push(jsast::default_case(default));

        let (value, _) = self.scrutinee(scrut);
        let switch = jsast::switch(value, cases);
        match label {
            Some(label) => vec![jsast::labeled(label_name(label), switch)],
            None => vec![switch],
        }
    }

    fn dispatch(
        &mut self,
        sel: TmpId,
        label: Option<Label>,
        cases: Vec<(u32, Region<Vec<Stmt>, Scrutinee<'tcx>>)>,
        trailing_break: bool,
    ) -> Stmt {
        self.tmps.entry(sel.0).or_insert(None);
        let selector = jsast::id(tmp_name(sel));
        let mut cases: Vec<_> = cases
            .into_iter()
            .map(|(value, body)| {
                let body = self.region(body);
                jsast::case(vec![jsast::num(value)], body)
            })
            .collect();
        // Every jump the backend emits assigns a selector value that names a case, so reaching
        // this is a backend bug, and it says so rather than returning `undefined`.
        cases.push(jsast::default_case(vec![jsast::throw(jsast::new(
            jsast::id("Error"),
            vec![jsast::binary(
                BinOp::Add,
                jsast::string("rustc_codegen_mires: bad block "),
                selector.clone(),
            )],
        ))]));

        let mut body = vec![jsast::switch(selector, cases)];
        if trailing_break {
            body.push(Stmt::Break(None));
        }
        Stmt::LoopForever(label.map(label_name), body)
    }

    /// The expression a branch tests, and the type that says how to read it.
    fn scrutinee(&mut self, scrut: Scrut<Scrutinee<'tcx>>) -> (Expr, ScrutTy<'tcx>) {
        match scrut {
            Scrut::Payload(scrutinee) => (scrutinee.value, scrutinee.ty),
            Scrut::Tmp(tmp) => (jsast::id(tmp_name(tmp)), self.tmp_ty(tmp)),
        }
    }

    /// The type recorded for a temporary when it was bound.
    ///
    /// A `Region::LetTmp` always precedes the tests that read its temporary, so the type is there.
    fn tmp_ty(&self, tmp: TmpId) -> ScrutTy<'tcx> {
        self.tmps
            .get(&tmp.0)
            .copied()
            .flatten()
            .unwrap_or(ScrutTy { ty: self.fx.tcx.types.usize, tag_enum: None })
    }

    /// A case value as a JavaScript literal, unbiased back into the scrutinee's own type.
    ///
    /// A tag scrutinee is matched by *name*, so the discriminant the case carries is looked back up
    /// as the variant it belongs to. A case value naming no variant cannot arise from a `match` --
    /// every arm names one -- and gets a string no tag can equal rather than a number that would
    /// silently match the wrong thing.
    fn case_literal(&self, scrut: ScrutTy<'tcx>, encoded: u128) -> Expr {
        let raw = cfg_adapter::decode(self.fx.tcx, scrut.ty, encoded);
        match scrut.tag_enum {
            Some(enum_ty) => {
                match value::variant_with_discriminant(self.fx.tcx, enum_ty, raw) {
                    Some(variant) => value::tag_literal(variant),
                    None => jsast::string(format!("$no-variant-{raw}")),
                }
            }
            None => self.fx.switch_case_value(scrut.ty, raw),
        }
    }

    /// The variants of a tag scrutinee's enum whose discriminant `keep` accepts, as an `||` chain of
    /// equalities (or, negated, an `&&` chain of inequalities).
    ///
    /// The structurizer reports an ordering test -- "below 3", "one of 2 values from 0" -- whenever
    /// the values it is partitioning happen to be contiguous. Names have no order, so the test is
    /// spelled by listing the variants it accepts; the list is bounded by the enum's variant count,
    /// and an or-pattern over two variants is what usually produces one.
    fn tag_chain(
        &self,
        value: Expr,
        enum_ty: Ty<'tcx>,
        scrut_ty: Ty<'tcx>,
        keep: impl Fn(i128) -> bool,
        negated: bool,
    ) -> Expr {
        let tcx = self.fx.tcx;
        let (op, join) = if negated {
            (BinOp::StrictNe, BinOp::And)
        } else {
            (BinOp::StrictEq, BinOp::Or)
        };
        let peeled = value::peel_pattern(value::peel_transparent(tcx, enum_ty));
        let ty::Adt(def, _) = peeled.kind() else { return jsast::boolean(negated) };
        let (signed, bits) = value::int_info(tcx, scrut_ty).unwrap_or((false, 128));
        let mut tests: Vec<Expr> = Vec::new();
        for (index, variant) in def.variants().iter_enumerated() {
            let Some(discr) = peeled.discriminant_for_variant(tcx, index) else { continue };
            if !keep(value::resign(signed, bits, discr.val)) {
                continue;
            }
            tests.push(jsast::binary(op, value.clone(), value::tag_literal(variant)));
        }
        let mut tests = tests.into_iter();
        let Some(first) = tests.next() else { return jsast::boolean(negated) };
        tests.fold(first, |acc, test| jsast::binary(join, acc, test))
    }

    /// How a test on a scrutinee of type `ty` reads in JavaScript, or its exact negation.
    ///
    /// The structurizer picks which values go which way; this picks the operators. `Lt(v)` and
    /// `Le(v)` compare `v` *against* the scrutinee — `v < x` and `v <= x` — so they come out as
    /// `x > v` and `x >= v`.
    ///
    /// `InRange { lo, len }` is `len` consecutive values from `lo`. A number-represented integer
    /// is at most 32 bits wide, so `(x - lo) >>> 0 < len` decides it in one comparison: the
    /// subtraction is exact in doubles, and a value below `lo` wraps to at least `2^32 - len`,
    /// which is never below `len`. A `BigInt` has no such trick and gets the two comparisons.
    fn test(&self, value: Expr, scrut: ScrutTy<'tcx>, test: Test, negated: bool) -> Expr {
        let ty = scrut.ty;
        // A tag has no order, so every test but `Eq` is spelled by listing the variants it accepts.
        if let Some(enum_ty) = scrut.tag_enum {
            let bound = |v: u128| cfg_adapter::case_value(self.fx.tcx, ty, v);
            match test {
                Test::Eq(_) | Test::IsTrue => {}
                Test::Lt(v) => {
                    let bound = bound(v);
                    return self.tag_chain(value, enum_ty, ty, |d| d > bound, negated);
                }
                Test::Le(v) => {
                    let bound = bound(v);
                    return self.tag_chain(value, enum_ty, ty, |d| d >= bound, negated);
                }
                Test::InRange { lo, len } => {
                    let (low, high) = (bound(lo), bound(lo + len - 1));
                    return self
                        .tag_chain(value, enum_ty, ty, |d| d >= low && d <= high, negated);
                }
            }
        }
        match test {
            Test::IsTrue => {
                if negated {
                    jsast::not(value)
                } else {
                    value
                }
            }
            Test::Eq(v) => {
                let op = if negated { BinOp::StrictNe } else { BinOp::StrictEq };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::Lt(v) => {
                let op = if negated { BinOp::Le } else { BinOp::Gt };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::Le(v) => {
                let op = if negated { BinOp::Lt } else { BinOp::Ge };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::InRange { lo, len } => match value::int_repr(ty) {
                IntRepr::Number => {
                    let offset = if cfg_adapter::case_value(self.fx.tcx, ty, lo) == 0 {
                        value
                    } else {
                        jsast::binary(BinOp::Sub, value, self.case_literal(scrut, lo))
                    };
                    let op = if negated { BinOp::Ge } else { BinOp::Lt };
                    jsast::binary(op, jsast::u32_wrap(offset), jsast::num(len as f64))
                }
                IntRepr::BigInt => {
                    let (low, high, join) = if negated {
                        (BinOp::Lt, BinOp::Gt, BinOp::Or)
                    } else {
                        (BinOp::Ge, BinOp::Le, BinOp::And)
                    };
                    let above = jsast::binary(low, value.clone(), self.case_literal(scrut, lo));
                    let below = jsast::binary(high, value, self.case_literal(scrut, lo + len - 1));
                    jsast::binary(join, above, below)
                }
            },
        }
    }
}

/// The test and body of a `for (;;)` that is really a `while`.
///
/// Takes the body apart only when the loop is exactly one `if` whose `else` is an unlabeled
/// `break`: that `break` leaves the loop in both forms, and so does every other jump inside, since
/// neither form captures anything the other does not.
fn as_while(body: &mut Vec<Stmt>) -> Option<(Expr, Vec<Stmt>)> {
    // Source map markers print nothing, so a loop that is one `if` behind a marker is still one
    // `if`. The markers go with the test they precede, which is where the `while` puts it anyway.
    if body.iter().filter(|stmt| !matches!(stmt, Stmt::Loc(_))).count() != 1 {
        return None;
    }
    match body.pop()? {
        Stmt::If(test, then, Some(els))
            if matches!(els.as_slice(), [Stmt::Break(None)]) && !then.is_empty() =>
        {
            Some((test, then))
        }
        other => {
            body.push(other);
            None
        }
    }
}
