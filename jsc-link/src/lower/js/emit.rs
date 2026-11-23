//! The region tree as JavaScript statements, and the ops of one block as the statements and
//! expressions they become (v1 `backend/src/emit.rs` plus the statement half of `base.rs`).
//!
//! # Ops
//!
//! A block's payload is its ops in order. An op whose result the plan inlined becomes an
//! expression cached on the context and consumed at its use site (`JsCx::expr_of`); every other
//! value-producing op is an assignment to its `let` name (declared once in the prelude, as v1
//! declares every local); an effect is a statement. Every op is preceded by the `Stmt::Loc` marker
//! of its origin under `js-source-map=on`, deduplicated per block exactly as v1 `names.rs::Locs`.
//!
//! # Regions
//!
//! Labels are `L0..`, structurizer temporaries `$s0..`; tests are spelled by [`Emitter::test`]
//! with the sign-bias decode over `TypeTable::int_info`; a switch on a tag enum spells its cases
//! as variant names.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{
    Carrier, CellId, Op, OpKind, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    BinaryOp, CoroutineDesugaring, CoroutineKind, HostPayload, TypeId,
};
use rcgjs_object::jir::types::{EnumRepr, IntRepr, PointerForm, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr, Loc, Stmt};
use rcgjs_object::ptr;
use structurizer::{Label, Region, Scrut, Test, TmpId};

use super::super::plan::{self, BlockPayload, CopyOperand, OpRef, RScrut};
use super::call::Lowered;
use super::place::JsPlace;
use super::value::{self, resign};
use super::{decline, JsCx, Res, CALLER_LOCATION_PARAM};

/// The name of a structurizer label.
pub fn label_name(label: Label) -> String {
    format!("L{}", label.0)
}

/// The name of a structurizer temporary (`emit.rs::tmp_name`).
pub fn tmp_name(tmp: TmpId) -> String {
    format!("$s{}", tmp.0)
}

/// The name spill register `index` would prefer, one-based: `$r1`, `$r2`, ...
///
/// Each register is a module-scope `let` the link step synthesizes as an item of its own
/// (`jsc_link::plan::abi_flat::AbiPlan::spill_items`), so both the reader and the writer name it
/// the same way and the reachability walk follows it like any other reference. Preferred rather
/// than fixed because the program may already call something `$r1`, in which case the plan
/// allocates the register a name around it and hands the printer the list ([`spill_name_of`]).
pub fn spill_name(index: u32) -> String {
    format!("$r{index}")
}

/// The name the plan allocated spill register `index`, falling back to the preferred spelling
/// when there is no plan (a lowering run outside the lane driver).
pub(super) fn spill_name_of(names: &[JsName], index: u32) -> String {
    match names.get(index as usize - 1) {
        Some(name) => name.as_str().to_owned(),
        None => spill_name(index),
    }
}

/// The scrutinee type a test reads its value through.
#[derive(Clone, Copy)]
pub struct ScrutTy {
    pub ty: TypeId,
    pub tag_enum: Option<TypeId>,
}

/// The body of one function, and the names of the temporaries its prelude has to declare.
pub fn emit(cx: &JsCx<'_>, region: Region<BlockPayload, RScrut>) -> Res<(Vec<Stmt>, Vec<String>)> {
    let mut emitter = Emitter {
        cx,
        tmps: BTreeMap::new(),
        next_switch_label: 0,
    };
    let body = emitter.region(region)?;
    let tmps = emitter
        .tmps
        .iter()
        .flat_map(|(tmp, scrut)| {
            let base = tmp_name(TmpId(*tmp));
            if scrut.is_some_and(|s| cx.is_i64_ty(s.ty) && s.tag_enum.is_none()) {
                let (lo, hi) = super::i64_pair::pair_names(&base);
                vec![lo, hi]
            } else {
                vec![base]
            }
        })
        .collect();
    Ok((body, tmps))
}

struct Emitter<'a, 'b> {
    cx: &'a JsCx<'b>,
    /// Every temporary the tree names, with the scrutinee type of the ones a test reads back.
    tmps: BTreeMap<u32, Option<ScrutTy>>,
    next_switch_label: u32,
}

type SwitchArm = (Vec<u128>, Region<BlockPayload, RScrut>);

impl Emitter<'_, '_> {
    fn region(&mut self, region: Region<BlockPayload, RScrut>) -> Res<Vec<Stmt>> {
        Ok(match region {
            Region::Seq(regions) => {
                let mut out = Vec::new();
                for region in regions {
                    out.extend(self.region(region)?);
                }
                out
            }
            Region::Block(payload) => self.cx.payload(&payload)?,
            Region::Labeled { label, body } => {
                let body = self.region(*body)?;
                vec![jsast::labeled(label_name(label), Stmt::Block(body))]
            }
            Region::Loop { label, body } => self.loop_(label, *body)?,
            Region::LetTmp { tmp, value } => {
                let loc = self.scrutinee_loc(&Scrut::Payload(value));
                let ty = self.cx.scrut_ty(&value);
                if self.cx.is_i64_ty(ty.ty) && ty.tag_enum.is_none() {
                    if let RScrut::Value { value: val_id, .. } = value {
                        let (lo, hi) = self.cx.pair_of(val_id)?;
                        self.tmps.insert(tmp.0, Some(ty));
                        let (dst_lo, dst_hi) = super::i64_pair::pair_names(&tmp_name(tmp));
                        return Ok(with_loc(
                            loc,
                            vec![
                                jsast::assign_stmt(jsast::id(dst_lo), lo),
                                jsast::assign_stmt(jsast::id(dst_hi), hi),
                            ],
                        ));
                    }
                }
                let (expr, ty) = self.cx.scrutinee(&value)?;
                self.tmps.insert(tmp.0, Some(ty));
                with_loc(
                    loc,
                    vec![jsast::assign_stmt(jsast::id(tmp_name(tmp)), expr)],
                )
            }
            Region::If {
                scrut,
                test,
                then_,
                else_,
            } => self.if_(scrut, test, *then_, *else_)?,
            Region::Switch {
                scrut,
                arms,
                default,
                label,
            } => self.switch(scrut, arms, *default, label)?,
            Region::Dispatch {
                sel,
                label,
                cases,
                trailing_break,
            } => vec![self.dispatch(sel, label, cases, trailing_break)?],
            Region::SetSel(tmp, value) => {
                self.tmps.entry(tmp.0).or_insert(None);
                vec![jsast::assign_stmt(
                    jsast::id(tmp_name(tmp)),
                    jsast::num(value),
                )]
            }
            Region::Break(label) => vec![Stmt::Break(label.map(label_name))],
            Region::Continue(label) => vec![Stmt::Continue(label.map(label_name))],
            Region::Return => vec![self.cx.return_stmt()?],
            Region::Unreachable => vec![jsast::throw_abort(
                "rustc_codegen_js: entered unreachable code",
            )],
            Region::Throw if self.cx.func.has_invoke() => Vec::new(),
            Region::Throw => vec![jsast::throw_abort(
                "rustc_codegen_js: unwinding is not supported",
            )],
        })
    }

    fn loop_(
        &mut self,
        label: Option<Label>,
        body: Region<BlockPayload, RScrut>,
    ) -> Res<Vec<Stmt>> {
        let mut body = self.region(body)?;
        // A marker-only body belongs to the executable empty loop itself.
        if body.iter().all(|stmt| matches!(stmt, Stmt::Loc(_))) {
            body.push(Stmt::LoopForever(label.map(label_name), Vec::new()));
            return Ok(body);
        }
        Ok(match as_while(&mut body) {
            Some((test, inner)) => {
                // `as_while` leaves the condition's leading source markers in `body`.
                let statement = match label {
                    Some(label) => jsast::labeled(label_name(label), jsast::while_(test, inner)),
                    None => jsast::while_(test, inner),
                };
                body.push(statement);
                body
            }
            None => vec![Stmt::LoopForever(label.map(label_name), body)],
        })
    }

    fn scrutinee_i64_pair(&self, scrut: &Scrut<RScrut>) -> Res<Option<((Expr, Expr), ScrutTy)>> {
        let scrut_ty = match scrut {
            Scrut::Payload(scrutinee) => self.cx.scrut_ty(scrutinee),
            Scrut::Tmp(tmp) => self.tmp_ty(*tmp),
        };
        if !self.cx.is_i64_ty(scrut_ty.ty) || scrut_ty.tag_enum.is_some() {
            return Ok(None);
        }
        let pair = match scrut {
            Scrut::Payload(RScrut::Value { value, .. }) => self.cx.pair_of(*value)?,
            Scrut::Tmp(tmp) => {
                let (lo, hi) = super::i64_pair::pair_names(&tmp_name(*tmp));
                (jsast::id(lo), jsast::id(hi))
            }
            Scrut::Payload(RScrut::Invoke(_)) => return Ok(None),
        };
        Ok(Some((pair, scrut_ty)))
    }

    fn test_i64_pair(
        &self,
        setup: &mut Vec<Stmt>,
        pair: (Expr, Expr),
        scrut: ScrutTy,
        test: Test,
        negated: bool,
    ) -> Expr {
        let signed = super::i64_pair::i64_signedness(self.cx.table, scrut.ty).unwrap_or(false);
        let (al, ah) = super::i64_pair::materialize_pair(self.cx, setup, pair);
        let lit = |encoded: u128| super::i64_pair::literal_pair(self.cx.decode(scrut.ty, encoded));
        match test {
            Test::IsTrue => {
                let nz = jsast::binary(
                    BinOp::StrictNe,
                    jsast::binary(BinOp::BitOr, al, ah),
                    jsast::num(0.0),
                );
                if negated {
                    jsast::not(nz)
                } else {
                    nz
                }
            }
            Test::Eq(v) => {
                let op = if negated { BinaryOp::Ne } else { BinaryOp::Eq };
                super::i64_pair::cmp_pair(self.cx, setup, op, signed, (al, ah), lit(v))
            }
            Test::Lt(v) => {
                let op = if negated { BinaryOp::Le } else { BinaryOp::Gt };
                super::i64_pair::cmp_pair(self.cx, setup, op, signed, (al, ah), lit(v))
            }
            Test::Le(v) => {
                let op = if negated { BinaryOp::Lt } else { BinaryOp::Ge };
                super::i64_pair::cmp_pair(self.cx, setup, op, signed, (al, ah), lit(v))
            }
            Test::InRange { lo, len } => {
                let lo_pair = lit(lo);
                let hi_pair = lit(lo + len - 1);
                if negated {
                    let below = super::i64_pair::cmp_pair(
                        self.cx,
                        setup,
                        BinaryOp::Lt,
                        signed,
                        (al.clone(), ah.clone()),
                        lo_pair,
                    );
                    let above = super::i64_pair::cmp_pair(
                        self.cx,
                        setup,
                        BinaryOp::Gt,
                        signed,
                        (al, ah),
                        hi_pair,
                    );
                    jsast::binary(BinOp::Or, below, above)
                } else {
                    let above = super::i64_pair::cmp_pair(
                        self.cx,
                        setup,
                        BinaryOp::Ge,
                        signed,
                        (al.clone(), ah.clone()),
                        lo_pair,
                    );
                    let below = super::i64_pair::cmp_pair(
                        self.cx,
                        setup,
                        BinaryOp::Le,
                        signed,
                        (al, ah),
                        hi_pair,
                    );
                    jsast::binary(BinOp::And, above, below)
                }
            }
        }
    }

    fn if_(
        &mut self,
        scrut: Scrut<RScrut>,
        test: Test,
        then_: Region<BlockPayload, RScrut>,
        else_: Region<BlockPayload, RScrut>,
    ) -> Res<Vec<Stmt>> {
        let loc = self.scrutinee_loc(&scrut);
        let then_ = self.region(then_)?;
        let else_ = self.region(else_)?;
        let inline = matches!(scrut, Scrut::Payload(_));
        if let Some((pair, scrut_ty)) = self.scrutinee_i64_pair(&scrut)? {
            let mut stmts = Vec::new();
            let branch = match (then_.is_empty(), else_.is_empty()) {
                (false, true) => vec![jsast::if_(
                    self.test_i64_pair(&mut stmts, pair, scrut_ty, test, false),
                    then_,
                )],
                (true, false) => vec![jsast::if_(
                    self.test_i64_pair(&mut stmts, pair, scrut_ty, test, true),
                    else_,
                )],
                (false, false) => vec![jsast::if_else(
                    self.test_i64_pair(&mut stmts, pair, scrut_ty, test, false),
                    then_,
                    else_,
                )],
                (true, true) if inline => {
                    let (lo, hi) = pair;
                    let mut out = Vec::new();
                    if !super::i64_pair::is_trivial(&lo) {
                        out.push(jsast::expr_stmt(lo));
                    }
                    if !super::i64_pair::is_trivial(&hi) {
                        out.push(jsast::expr_stmt(hi));
                    }
                    out
                }
                (true, true) => Vec::new(),
            };
            stmts.extend(branch);
            return Ok(with_loc(loc, stmts));
        }
        let (value, scrut_ty) = self.scrutinee(scrut)?;
        Ok(with_loc(
            loc,
            match (then_.is_empty(), else_.is_empty()) {
                (false, true) => vec![jsast::if_(self.test(value, scrut_ty, test, false), then_)],
                (true, false) => vec![jsast::if_(self.test(value, scrut_ty, test, true), else_)],
                (false, false) => vec![jsast::if_else(
                    self.test(value, scrut_ty, test, false),
                    then_,
                    else_,
                )],
                (true, true) if inline => vec![jsast::expr_stmt(value)],
                (true, true) => Vec::new(),
            },
        ))
    }

    fn switch(
        &mut self,
        scrut: Scrut<RScrut>,
        arms: Vec<SwitchArm>,
        default: Region<BlockPayload, RScrut>,
        label: Option<Label>,
    ) -> Res<Vec<Stmt>> {
        let loc = self.scrutinee_loc(&scrut);
        if let Some((pair, scrut_ty)) = self.scrutinee_i64_pair(&scrut)? {
            let mut setup = Vec::new();
            let pair = super::i64_pair::materialize_pair(self.cx, &mut setup, pair);
            let mut compiled_arms = Vec::with_capacity(arms.len());
            for (values, body) in arms {
                let body = self.region(body)?;
                let mut conds = values.into_iter().map(|v| {
                    let lit = super::i64_pair::literal_pair(self.cx.decode(scrut_ty.ty, v));
                    super::i64_pair::cmp_pair(
                        self.cx,
                        &mut setup,
                        BinaryOp::Eq,
                        false,
                        pair.clone(),
                        lit,
                    )
                });
                let cond = match conds.next() {
                    Some(first) => conds.fold(first, |acc, c| jsast::binary(BinOp::Or, acc, c)),
                    None => jsast::boolean(false),
                };
                compiled_arms.push((cond, body));
            }
            let default = self.region(default)?;
            let mut chain = default;
            for (cond, body) in compiled_arms.into_iter().rev() {
                chain = if chain.is_empty() {
                    vec![jsast::if_(cond, body)]
                } else {
                    vec![jsast::if_else(cond, body, chain)]
                };
            }
            let (sw_label, needs_label) = match label {
                Some(label) => (label_name(label), true),
                None => {
                    let id = self.next_switch_label;
                    self.next_switch_label += 1;
                    (format!("$sw{id}"), false)
                }
            };
            let rewrote = rewrite_unlabeled_breaks(&mut chain, &sw_label);
            if needs_label || rewrote {
                setup.push(jsast::labeled(sw_label, Stmt::Block(chain)));
            } else {
                setup.extend(chain);
            }
            return Ok(with_loc(loc, setup));
        }
        let scrut_ty = match &scrut {
            Scrut::Payload(scrutinee) => self.cx.scrut_ty(scrutinee),
            Scrut::Tmp(tmp) => self.tmp_ty(*tmp),
        };
        let mut cases = Vec::with_capacity(arms.len() + 1);
        for (values, body) in arms {
            let body = self.region(body)?;
            let tests = values
                .into_iter()
                .map(|v| self.case_literal(scrut_ty, v))
                .collect();
            cases.push(jsast::case(tests, body));
        }
        let default = self.region(default)?;
        cases.push(jsast::default_case(default));
        let (value, _) = self.scrutinee(scrut)?;
        let switch = jsast::switch(value, cases);
        Ok(with_loc(
            loc,
            match label {
                Some(label) => vec![jsast::labeled(label_name(label), switch)],
                None => vec![switch],
            },
        ))
    }

    /// A fused condition has no statement at its definition. Attribute the
    /// emitted test (or its saved temporary) to that expression's origin.
    fn scrutinee_loc(&self, scrut: &Scrut<RScrut>) -> Option<Loc> {
        let Scrut::Payload(scrut) = scrut else {
            return None;
        };
        let RScrut::Value { value, .. } = scrut else {
            return None;
        };
        if !self.cx.is_inline(*value) {
            return None;
        }
        self.cx.def_of(*value).and_then(|op| self.cx.op_loc(op))
    }

    fn dispatch(
        &mut self,
        sel: TmpId,
        label: Option<Label>,
        cases: Vec<(u32, Region<BlockPayload, RScrut>)>,
        trailing_break: bool,
    ) -> Res<Stmt> {
        self.tmps.entry(sel.0).or_insert(None);
        let selector = jsast::id(tmp_name(sel));
        let mut out = Vec::with_capacity(cases.len() + 1);
        for (value, body) in cases {
            let body = self.region(body)?;
            out.push(jsast::case(vec![jsast::num(value)], body));
        }
        out.push(jsast::default_case(vec![jsast::throw(jsast::new(
            jsast::id("Error"),
            vec![jsast::binary(
                BinOp::Add,
                jsast::string("rustc_codegen_js: bad block "),
                selector.clone(),
            )],
        ))]));
        let mut body = vec![jsast::switch(selector, out)];
        if trailing_break {
            body.push(Stmt::Break(None));
        }
        Ok(Stmt::LoopForever(label.map(label_name), body))
    }

    /// The expression a branch tests, and the type that says how to read it.
    fn scrutinee(&mut self, scrut: Scrut<RScrut>) -> Res<(Expr, ScrutTy)> {
        match scrut {
            Scrut::Payload(scrutinee) => self.cx.scrutinee(&scrutinee),
            Scrut::Tmp(tmp) => Ok((jsast::id(tmp_name(tmp)), self.tmp_ty(tmp))),
        }
    }

    /// The type recorded for a temporary when it was bound.
    fn tmp_ty(&self, tmp: TmpId) -> ScrutTy {
        self.tmps.get(&tmp.0).copied().flatten().unwrap_or(ScrutTy {
            ty: self.cx.usize_ty(),
            tag_enum: None,
        })
    }

    /// A case value as a JavaScript literal, unbiased back into the scrutinee's own type.
    ///
    /// A tag scrutinee is matched by *name*, so the discriminant the case carries is looked back
    /// up as the variant it belongs to (re-signed through the scrutinee's width, so `255` at `i8`
    /// finds the `-1` variant). A case value naming no variant gets a string no tag can equal.
    fn case_literal(&self, scrut: ScrutTy, encoded: u128) -> Expr {
        let raw = self.cx.decode(scrut.ty, encoded);
        match scrut.tag_enum {
            Some(enum_ty) => {
                let signed = self.cx.case_value(scrut.ty, encoded);
                match value::variant_with_logical_discr(self.cx.table, enum_ty, signed) {
                    Some(variant) => {
                        value::variant_tag(self.cx.table, self.cx.interns, enum_ty, variant)
                            .unwrap_or_else(|| jsast::string(format!("$no-variant-{raw}")))
                    }
                    None => jsast::string(format!("$no-variant-{raw}")),
                }
            }
            None => value::int_literal(self.cx.table, scrut.ty, raw)
                .unwrap_or_else(|| jsast::num(raw as f64)),
        }
    }

    /// The variants of a tag scrutinee's enum whose discriminant `keep` accepts, as an `||`
    /// chain of equalities (or, negated, an `&&` chain of inequalities).
    fn tag_chain(
        &self,
        value: Expr,
        enum_ty: TypeId,
        scrut_ty: TypeId,
        keep: impl Fn(i128) -> bool,
        negated: bool,
    ) -> Expr {
        let (op, join) = if negated {
            (BinOp::StrictNe, BinOp::And)
        } else {
            (BinOp::StrictEq, BinOp::Or)
        };
        let variants = value::enum_variants(self.cx.table, enum_ty);
        let (signed, bits) = self.cx.table.int_info(scrut_ty).unwrap_or((false, 128));
        let mut tests: Vec<Expr> = Vec::new();
        for (index, variant) in variants.iter().enumerate() {
            let discr = resign(signed, bits, variant.logical_discr as u128);
            if !keep(discr) {
                continue;
            }
            let Some(tag) =
                value::variant_tag(self.cx.table, self.cx.interns, enum_ty, index as u32)
            else {
                continue;
            };
            tests.push(jsast::binary(op, value.clone(), tag));
        }
        let mut tests = tests.into_iter();
        let Some(first) = tests.next() else {
            return jsast::boolean(negated);
        };
        tests.fold(first, |acc, test| jsast::binary(join, acc, test))
    }

    /// How a test on a scrutinee reads in JavaScript, or its exact negation (`emit.rs::test`).
    fn test(&self, value: Expr, scrut: ScrutTy, test: Test, negated: bool) -> Expr {
        let ty = scrut.ty;
        if let Some(enum_ty) = scrut.tag_enum {
            let bound = |v: u128| self.cx.case_value(ty, v);
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
                    return self.tag_chain(value, enum_ty, ty, |d| d >= low && d <= high, negated);
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
                let rhs = self.case_literal(scrut, v);
                if let Some(check) = super::ptr::simplify_addr_zero_eq(&value, &rhs) {
                    return if negated { jsast::not(check) } else { check };
                }
                let op = if negated {
                    BinOp::StrictNe
                } else {
                    BinOp::StrictEq
                };
                jsast::binary(op, value, rhs)
            }
            Test::Lt(v) => {
                let op = if negated { BinOp::Le } else { BinOp::Gt };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::Le(v) => {
                let op = if negated { BinOp::Lt } else { BinOp::Ge };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::InRange { lo, len } => match self.cx.table.int_repr(ty) {
                IntRepr::Number => {
                    let offset = if self.cx.case_value(ty, lo) == 0 {
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

fn rewrite_unlabeled_breaks(stmts: &mut [Stmt], label: &str) -> bool {
    let mut found = false;
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::Break(None) => {
                *stmt = Stmt::Break(Some(label.to_owned()));
                found = true;
            }
            Stmt::If(_, then_, els) => {
                found |= rewrite_unlabeled_breaks(then_, label);
                if let Some(els) = els {
                    found |= rewrite_unlabeled_breaks(els, label);
                }
            }
            Stmt::Block(inner) => {
                found |= rewrite_unlabeled_breaks(inner, label);
            }
            Stmt::Labeled(_, inner) => {
                found |= rewrite_unlabeled_breaks(std::slice::from_mut(inner), label);
            }
            Stmt::TryCatch { body, handler, .. } => {
                found |= rewrite_unlabeled_breaks(body, label);
                found |= rewrite_unlabeled_breaks(handler, label);
            }
            Stmt::Switch(_, _)
            | Stmt::LoopForever(_, _)
            | Stmt::While(_, _)
            | Stmt::FunctionDecl { .. } => {}
            _ => {}
        }
    }
    found
}

fn with_loc(loc: Option<Loc>, mut statements: Vec<Stmt>) -> Vec<Stmt> {
    if !statements.is_empty() {
        if let Some(loc) = loc {
            statements.insert(0, Stmt::Loc(loc));
        }
    }
    statements
}

/// The test and body of a `for (;;)` that is really a `while` (`emit.rs::as_while`).
fn as_while(body: &mut Vec<Stmt>) -> Option<(Expr, Vec<Stmt>)> {
    if body
        .iter()
        .filter(|stmt| !matches!(stmt, Stmt::Loc(_)))
        .count()
        != 1
    {
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

// ---------------------------------------------------------------------------------------------
// Sign bias (v1 `cfg_adapter.rs`)
// ---------------------------------------------------------------------------------------------

impl<'a> JsCx<'a> {
    /// The MIR bit pattern an encoded case value stands for (`plan::decode`).
    pub fn decode(&self, ty: TypeId, encoded: u128) -> u128 {
        plan::decode(self.table, ty, encoded)
    }

    /// The Rust value an encoded case value stands for, sign included (`plan::case_value`).
    pub fn case_value(&self, ty: TypeId, encoded: u128) -> i128 {
        plan::case_value(self.table, ty, encoded)
    }

    /// A `usize`-shaped type for a temporary bound with no recorded type: the first 32-bit
    /// unsigned integer in the table, or the scrutinee's own type is used instead.
    fn usize_ty(&self) -> TypeId {
        self.table
            .descriptors()
            .iter()
            .find(|d| {
                matches!(
                    d.kind,
                    TypeKind::Integer {
                        signed: false,
                        bits: 32
                    }
                )
            })
            .map(|d| d.id)
            .unwrap_or(TypeId(0))
    }

    /// The scrutinee type of an `RScrut`.
    pub fn scrut_ty(&self, scrutinee: &RScrut) -> ScrutTy {
        match *scrutinee {
            RScrut::Value { ty, tag_enum, .. } => ScrutTy { ty, tag_enum },
            RScrut::Invoke(_) => ScrutTy {
                ty: self.usize_ty(),
                tag_enum: None,
            },
        }
    }

    /// The expression a scrutinee tests and its type.
    pub fn scrutinee(&self, scrutinee: &RScrut) -> Res<(Expr, ScrutTy)> {
        let expr = match *scrutinee {
            RScrut::Value { value, .. } => self.expr_of(value)?,
            RScrut::Invoke(block) => jsast::id(
                self.plan
                    .invoke_names
                    .get(&block)
                    .ok_or_else(|| decline(OpClass::Terminator, "invoke control name missing"))?,
            ),
        };
        Ok((expr, self.scrut_ty(scrutinee)))
    }
}

// ---------------------------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------------------------

impl<'a> JsCx<'a> {
    /// The statements of one block payload: its ops in order, each behind its source marker,
    /// then the edge copies of the edge it carries.
    pub fn payload(&self, payload: &BlockPayload) -> Res<Vec<Stmt>> {
        let mut out = Vec::new();
        let mut last: Option<Loc> = None;
        for op_ref in &payload.ops {
            let op = self.op(*op_ref)?;
            self.at(op.origin);
            let mut stmts = match op.kind {
                OpKind::CellAlloc(cell)
                    if self
                        .plan
                        .fresh_allocs
                        .contains(&(op_ref.block, op_ref.index)) =>
                {
                    self.fresh_container(cell)?
                }
                _ => self.op_stmts(op)?,
            };
            let block = &self.func.blocks[op_ref.block.0 as usize];
            if matches!(block.term, RTerm::Invoke { .. })
                && op_ref.index as usize + 1 == block.ops.len()
            {
                let success =
                    self.plan.invoke_names.get(&block.id).ok_or_else(|| {
                        decline(OpClass::Terminator, "invoke control name missing")
                    })?;
                let exception =
                    self.plan.exception_name.as_ref().ok_or_else(|| {
                        decline(OpClass::Terminator, "invoke exception name missing")
                    })?;
                let binding = self
                    .plan
                    .catch_name
                    .as_ref()
                    .ok_or_else(|| decline(OpClass::Terminator, "invoke catch name missing"))?;
                let mut body = Vec::new();
                self.mark(&mut body, &mut None, self.op_loc(op));
                body.append(&mut stmts);
                body.push(jsast::assign_stmt(jsast::id(success), jsast::num(1.0)));
                stmts.push(Stmt::TryCatch {
                    body,
                    binding: binding.clone(),
                    handler: vec![
                        jsast::if_(
                            jsast::rt_call("is_abort", vec![jsast::id(binding)]),
                            vec![jsast::throw(jsast::id(binding))],
                        ),
                        jsast::assign_stmt(jsast::id(exception), jsast::id(binding)),
                        jsast::assign_stmt(jsast::id(success), jsast::num(0.0)),
                    ],
                });
            }
            if stmts.is_empty() {
                continue;
            }
            self.mark(&mut out, &mut last, self.op_loc(op));
            out.extend(stmts);
        }
        // What the block's terminator runs before it transfers control: a thrown value, and the
        // returned value parked for the `Region::Return` that follows.
        if let Some(block) = payload.block {
            let stmts = self.term_stmts(block)?;
            if !stmts.is_empty() {
                self.mark(&mut out, &mut last, self.term_loc(block));
                out.extend(stmts);
            }
        }
        if let Some(edge) = payload.edge {
            if let Some(copies) = self.plan.copies.get(&edge) {
                for copy in copies {
                    // An expression fused into an edge assignment has no standalone op marker.
                    // Keep its definition's source, including an imported provider's origin.
                    if let CopyOperand::Value(value) = copy.src {
                        if self.is_inline(value) {
                            let loc = self.def_of(value).and_then(|op| self.op_loc(op));
                            self.mark(&mut out, &mut last, loc);
                        }
                    }
                    if let CopyOperand::Value(value) = copy.dst {
                        if self.plan.physical_slices.contains(&value) {
                            let src = match copy.src {
                                CopyOperand::Value(v) => self.expr_of(v)?,
                                CopyOperand::Temp(_) => {
                                    jsast::id(self.copy_operand_name(copy.src)?)
                                }
                            };
                            out.extend(self.assign_physical_slice(value, src)?);
                            continue;
                        }
                        if self.plan.physical_slots.contains(&value) {
                            let src = if let Some(bv) = self.byte_view(value) {
                                match copy.src {
                                    CopyOperand::Value(v) => jsast::object_of(vec![
                                        ("buf", self.byte_view_base_expr(bv, value)?),
                                        ("off", self.byte_view_off_expr(v)?),
                                    ]),
                                    CopyOperand::Temp(_) => {
                                        jsast::id(self.copy_operand_name(copy.src)?)
                                    }
                                }
                            } else {
                                match copy.src {
                                    CopyOperand::Value(v) => self.expr_of(v)?,
                                    CopyOperand::Temp(_) => {
                                        jsast::id(self.copy_operand_name(copy.src)?)
                                    }
                                }
                            };
                            out.extend(self.assign_physical_slot(value, src)?);
                            continue;
                        }
                    }
                    let dst_is_i64 = match copy.dst {
                        CopyOperand::Value(v) => self.is_i64_value(v),
                        CopyOperand::Temp(_) => false,
                    };
                    let src_is_i64 = match copy.src {
                        CopyOperand::Value(v) => self.is_i64_value(v),
                        CopyOperand::Temp(_) => false,
                    };
                    if dst_is_i64 || src_is_i64 {
                        let dst_base = self.copy_operand_name(copy.dst)?;
                        let (dst_lo, dst_hi) = super::i64_pair::pair_names(&dst_base);
                        let (src_lo, src_hi) = match copy.src {
                            CopyOperand::Value(v) => self.pair_of(v)?,
                            CopyOperand::Temp(_) => {
                                let src_base = self.copy_operand_name(copy.src)?;
                                let (slo, shi) = super::i64_pair::pair_names(&src_base);
                                (jsast::id(slo), jsast::id(shi))
                            }
                        };
                        let mut pair_stmts = Vec::new();
                        super::i64_pair::emit_assign_pair(
                            self,
                            &mut pair_stmts,
                            &dst_lo,
                            &dst_hi,
                            (src_lo, src_hi),
                        );
                        out.extend(pair_stmts);
                        continue;
                    }
                    let dst = self.copy_operand_name(copy.dst)?;
                    let src = match (copy.dst, copy.src) {
                        (CopyOperand::Value(dst_val), CopyOperand::Value(v))
                            if self
                                .byte_view(dst_val)
                                .is_some_and(|b| b.mode == super::ByteViewMode::ThinPtr) =>
                        {
                            self.byte_view_off_expr(v)?
                        }
                        (_, CopyOperand::Value(v)) => self.expr_of(v)?,
                        (_, CopyOperand::Temp(_)) => jsast::id(self.copy_operand_name(copy.src)?),
                    };
                    out.push(jsast::assign_stmt(jsast::id(dst), src));
                }
            }
        }
        while matches!(out.last(), Some(Stmt::Loc(_))) {
            out.pop();
        }
        // An empty self-loop still executes its backedge. Its marker belongs
        // on the recovered loop header rather than an absent body statement.
        if out.is_empty() {
            if let Some(block) = payload.block {
                if matches!(self.func.blocks[block.0 as usize].term, RTerm::Goto(target, _) if target == block)
                {
                    self.mark(&mut out, &mut last, self.term_loc(block));
                }
            }
        }
        Ok(out)
    }

    /// The name a copy operand is assigned through.
    fn copy_operand_name(&self, operand: CopyOperand) -> Res<String> {
        match operand {
            CopyOperand::Value(v) => self.name_of(v).map(str::to_owned).ok_or_else(|| {
                decline(
                    OpClass::Terminator,
                    format!("copy of unnamed value {}", v.0),
                )
            }),
            CopyOperand::Temp(index) => self
                .plan
                .copy_temps
                .get(index as usize)
                .cloned()
                .ok_or_else(|| {
                    decline(
                        OpClass::Terminator,
                        format!("copy temporary {index} is not in the plan"),
                    )
                }),
        }
    }

    /// Appends the marker for `loc`, unless it would repeat the last one (`names.rs::Locs`).
    fn mark(&self, out: &mut Vec<Stmt>, last: &mut Option<Loc>, loc: Option<Loc>) {
        let Some(loc) = loc else { return };
        if last.as_ref() == Some(&loc) {
            return;
        }
        out.push(Stmt::Loc(loc.clone()));
        *last = Some(loc);
    }

    /// The keyed marker owns a source fact for its whole synthesized runtime
    /// call. Prefer that over the enclosing macro expansion's MIR origin so an
    /// exact debugger position lands on the written `for ... key (...)` site.
    fn op_loc(&self, op: &Op) -> Option<Loc> {
        if let Some(loc) = self.keyed_op_loc(op) {
            return Some(loc);
        }
        let args = match &op.kind {
            OpKind::Call { args, .. } | OpKind::Host { args, .. } => args.as_slice(),
            _ => &[],
        };
        let mut seen = BTreeSet::new();
        if let Some(loc) = args
            .iter()
            .find_map(|value| self.inlined_keyed_loc(*value, &mut seen))
        {
            return Some(loc);
        }
        self.loc_of(op.origin)
    }

    /// Finds a keyed expression that this operation prints at its use site.
    fn inlined_keyed_loc(&self, value: ValueId, seen: &mut BTreeSet<ValueId>) -> Option<Loc> {
        if !self.is_inline(value) || !seen.insert(value) {
            return None;
        }
        let op = self.def_of(value)?;
        if let Some(loc) = self.keyed_op_loc(op) {
            return Some(loc);
        }
        let args = match &op.kind {
            OpKind::Call { args, .. } | OpKind::Host { args, .. } => args.as_slice(),
            OpKind::Copy(value) | OpKind::Move(value) => {
                return self.inlined_keyed_loc(*value, seen)
            }
            _ => &[],
        };
        args.iter()
            .find_map(|value| self.inlined_keyed_loc(*value, seen))
    }

    fn keyed_op_loc(&self, op: &Op) -> Option<Loc> {
        let OpKind::Call {
            host: Some(HostPayload::KeyedList(payload)),
            ..
        } = &op.kind
        else {
            return None;
        };
        payload.site.and_then(|site| self.loc_of_fact(site))
    }

    /// The op an `OpRef` names.
    fn op(&self, op_ref: OpRef) -> Res<&'a Op> {
        self.func
            .blocks
            .get(op_ref.block.0 as usize)
            .and_then(|block| block.ops.get(op_ref.index as usize))
            .ok_or_else(|| {
                decline(
                    OpClass::Other,
                    format!(
                        "op {} of block {} is out of range",
                        op_ref.index, op_ref.block.0
                    ),
                )
            })
    }

    /// The `Loc` of a block's terminator, through the origin table's terminator row.
    fn term_loc(&self, block: RBlockId) -> Option<Loc> {
        if !self.opts.source_map {
            return None;
        }
        let origins = self.origins?;
        let origin = origins.terminators.get(block.0 as usize)?;
        self.loc_of(*origin)
    }

    /// What a terminator runs before it transfers control: a `Throw` throws its value; a
    /// `Return` parks its value for the `Region::Return` that follows (the region carries none).
    fn term_stmts(&self, block: RBlockId) -> Res<Vec<Stmt>> {
        let Some(term) = self.func.blocks.get(block.0 as usize).map(|b| &b.term) else {
            return Err(decline(
                OpClass::Terminator,
                format!("block {} is out of range", block.0),
            ));
        };
        match term {
            RTerm::Return(Some(v)) => {
                if self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .is_some_and(|result| result.i64_pair)
                {
                    let pair = self.pair_of(*v)?;
                    self.pending_return_pair.replace(Some(pair));
                    self.pending_return.replace(None);
                    return Ok(Vec::new());
                }
                self.pending_return_pair.replace(None);
                // A `()` (or other zero-sized) result is a bare `return`, as v1's unit function.
                let value = if self.table.is_zst(self.ty_of(*v)?) {
                    None
                } else {
                    Some(self.expr_of(*v)?)
                };
                self.pending_return.replace(value);
                Ok(Vec::new())
            }
            RTerm::Return(None) => {
                self.pending_return_pair.replace(None);
                self.pending_return.replace(None);
                Ok(Vec::new())
            }
            RTerm::Throw(Some(v)) => Ok(vec![jsast::throw(self.expr_of(*v)?)]),
            RTerm::Throw(None) if self.plan.exception_name.is_some() => Ok(vec![jsast::throw(
                jsast::id(self.plan.exception_name.as_ref().unwrap()),
            )]),
            _ => Ok(Vec::new()),
        }
    }

    /// `Region::Return`: `return <value>` with the value the returning block parked, or a bare
    /// `return` for a `()` function.
    fn return_stmt(&self) -> Res<Stmt> {
        if let Some((lo, hi)) = self.pending_return_pair.borrow_mut().take() {
            let spill = jsast::id(spill_name_of(self.spills, 1));
            return Ok(jsast::ret(jsast::seq(vec![jsast::assign(spill, hi), lo])));
        }
        match self.pending_return.take() {
            Some(value) => {
                let tagged = self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.tagged.as_ref());
                let recursive = self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.recursive.as_ref());
                if tagged.is_none() && recursive.is_none() {
                    return Ok(jsast::ret(value));
                }
                let keys = match (tagged, recursive) {
                    (Some(tagged), None) => {
                        value::tagged_product_keys(self.table, self.interns, tagged)
                    }
                    (None, Some(recursive)) => {
                        value::recursive_product_keys(self.table, self.interns, recursive)
                    }
                    _ => None,
                };
                let Some(keys) = keys else {
                    return Err(decline(
                        OpClass::Terminator,
                        "physical return has an unprintable product shape",
                    ));
                };
                let reads = keys.iter().map(String::as_str).collect::<Vec<_>>();
                let (setup, value) = self.reread(value, &reads);
                let lanes = match (tagged, recursive) {
                    (Some(tagged), None) => {
                        value::tagged_product_lanes(self.table, self.interns, tagged, value)
                    }
                    (None, Some(recursive)) => {
                        value::recursive_product_lanes(self.table, self.interns, recursive, value)
                    }
                    _ => None,
                };
                let Some(mut lanes) = lanes else {
                    return Err(decline(
                        OpClass::Terminator,
                        "physical return lanes do not agree with its type",
                    ));
                };
                let tag = lanes.remove(0);
                let mut sequence = Vec::with_capacity(lanes.len() + 2);
                sequence.extend(setup);
                sequence.extend(lanes.into_iter().enumerate().map(|(index, payload)| {
                    jsast::assign(
                        jsast::id(spill_name_of(self.spills, index as u32 + 1)),
                        payload,
                    )
                }));
                sequence.push(tag);
                Ok(jsast::ret(jsast::seq(sequence)))
            }
            None if self.table.is_zst(self.func.sig.result) => Ok(jsast::ret_void()),
            None => Err(decline(
                OpClass::Terminator,
                "a `Region::Return` with no returning block payload before it",
            )),
        }
    }

    /// The statements one op becomes: nothing for an inlined value (its expression is cached), an
    /// assignment for a named one, the effect's statements otherwise.
    pub fn op_stmts(&self, op: &Op) -> Res<Vec<Stmt>> {
        // A tagged result is a virtual canonical object backed by captured scalar locals. Its
        // admission proof permits only field/discriminant reads and Move/Copy aliases leading to
        // those reads or another prepared direct call, so cloning the description would add an
        // `Object.assign` with no observable copy semantics. Propagate the virtual description
        // instead; a true escape was refused by the ABI planner and never reaches this path.
        if let (Some(result), OpKind::Move(source) | OpKind::Copy(source)) = (op.result, &op.kind) {
            if let Some(value) = self.tagged_expr(*source) {
                self.bind_tagged_expr(result, value);
                return Ok(Vec::new());
            }
            let carried = self.carried_i64_overflow.borrow().get(source).cloned();
            if let Some(carried) = carried {
                self.carried_i64_overflow
                    .borrow_mut()
                    .insert(result, carried);
                return Ok(Vec::new());
            }
        }
        if let (
            Some(result),
            OpKind::Field {
                base,
                variant: None,
                field,
            },
        ) = (op.result, &op.kind)
        {
            if let Some((lo, hi, ovf)) = self.carried_i64_overflow.borrow().get(base).cloned() {
                return match field {
                    0 => self.bind_i64_pair_result(result, Vec::new(), (lo, hi)),
                    1 => self.bind_scalar_result(result, Vec::new(), ovf, false),
                    _ => Err(decline(
                        OpClass::Place,
                        format!("field {field} of an i64 overflow pair"),
                    )),
                };
            }
        }
        if let (Some(result), OpKind::Binary { op: bin, l, r, ty }) = (op.result, &op.kind) {
            if self.is_i64_ty(*ty) {
                let mut stmts = Vec::new();
                if self.carried_pairs.contains_key(&result) {
                    let signed = super::i64_pair::i64_signedness(self.table, *ty).unwrap_or(false);
                    let a = self.pair_of(*l)?;
                    let b = self.pair_of(*r)?;
                    let (lo, hi, ovf) =
                        super::i64_pair::with_overflow_pair(self, &mut stmts, *bin, signed, a, b);
                    let lo = snapshot_scratch(self, &mut stmts, lo);
                    let hi = snapshot_scratch(self, &mut stmts, hi);
                    let ovf = snapshot_scratch(self, &mut stmts, ovf);
                    self.carried_i64_overflow
                        .borrow_mut()
                        .insert(result, (lo, hi, ovf));
                    return Ok(stmts);
                }
                let result_ty = self.ty_of(result)?;
                if self.is_i64_ty(result_ty) {
                    let pair = self.i64_binary_pair_stmts(&mut stmts, *bin, *ty, *l, *r)?;
                    return self.bind_i64_pair_result(result, stmts, pair);
                }
                let expr = self.i64_binary_expr_stmts(&mut stmts, *bin, *ty, result_ty, *l, *r)?;
                return self.bind_scalar_result(result, stmts, expr, false);
            }
        }
        if let (Some(result), OpKind::Cast { kind, v, from, to }) = (op.result, &op.kind) {
            let result_ty = self.ty_of(result)?;
            if self.is_i64_ty(result_ty) {
                let mut stmts = Vec::new();
                let pair = self.i64_cast_pair_stmts(&mut stmts, *kind, *from, *to, *v)?;
                return self.bind_i64_pair_result(result, stmts, pair);
            }
            if self.is_i64_ty(*from) {
                let mut stmts = Vec::new();
                let expr = self.i64_cast_expr_stmts(&mut stmts, *kind, *from, *to, *v)?;
                return self.bind_scalar_result(result, stmts, expr, false);
            }
        }
        if let Some(result) = op
            .result
            .filter(|result| self.plan.physical_slices.contains(result))
        {
            let value = match &op.kind {
                // A fat slice is a Copy value. Cloning its temporary JavaScript record would
                // reintroduce precisely the allocation the lanes remove.
                OpKind::Copy(source) | OpKind::Move(source) => self.expr_of(*source)?,
                _ => self.op_expr(op, result)?,
            };
            self.record_allocation_origin(result, &value);
            return self.assign_physical_slice(result, value);
        }
        if let Some(result) = op
            .result
            .filter(|result| self.plan.physical_slots.contains(result))
        {
            if let Some(bv) = self.byte_view(result) {
                let off = match &op.kind {
                    OpKind::Copy(source) | OpKind::Move(source) => {
                        self.byte_view_off_expr(*source)?
                    }
                    _ => self.op_expr(op, result)?,
                };
                let value = jsast::object_of(vec![
                    ("buf", self.byte_view_base_expr(bv, result)?),
                    ("off", off),
                ]);
                self.record_allocation_origin(result, &value);
                return self.assign_physical_slot(result, value);
            }
            let value = match &op.kind {
                OpKind::Copy(source) | OpKind::Move(source) => self.expr_of(*source)?,
                _ => self.op_expr(op, result)?,
            };
            self.record_allocation_origin(result, &value);
            return self.assign_physical_slot(result, value);
        }
        // A call, an intrinsic, a host operation and a drop are lowered as statements plus a
        // value (`call.rs`): whatever the lowering has to run before the result is bound.
        if matches!(
            op.kind,
            OpKind::Call { .. }
                | OpKind::Intrinsic { .. }
                | OpKind::Host { .. }
                | OpKind::Drop { .. }
        ) && !self.is_byte_view_intrinsic(op)
            && !self.is_inlined_vec_bypass_call(op)
        {
            if let Some(stmts) = self.inlined_vec_reserve_stmts(op)? {
                return Ok(stmts);
            }
            if let Some(result) = op.result.filter(|result| self.is_inline(*result)) {
                // Spelled at its one use site (`JsCx::expr_of`) — but the statements its lowering
                // needs have no position there, so they are emitted here, at the definition, and
                // the value is cached for the use to read back. The plan promises the use is in
                // the same block after the definition with only commuting ops in between, so
                // that is evaluating in order.
                let (prelude, value, virtual_tagged) = match self.lower_call_like(op)? {
                    Lowered::Value(prelude, value) => (prelude, value, false),
                    Lowered::Virtual(prelude, value) => (prelude, value, true),
                    Lowered::Pair(prelude, pair) => {
                        return self.bind_i64_pair_result(result, prelude, pair);
                    }
                    Lowered::Done(_) => {
                        return Err(decline(
                            OpClass::Call,
                            "an inlined call whose lowering produces no value",
                        ))
                    }
                };
                if self.is_i64_ty(self.ty_of(result)?) {
                    let mut out = prelude;
                    let value = super::i64_pair::materialize(self, &mut out, value);
                    let pair = super::i64_pair::unpack_bigint_exprs(value);
                    return self.bind_i64_pair_result(result, out, pair);
                }
                if virtual_tagged {
                    self.bind_tagged_expr(result, value);
                    return Ok(prelude);
                }
                let repeated = self.reads_pointer_twice(result)?;
                if prelude.is_empty() && !repeated {
                    return Ok(Vec::new());
                }
                let mut out = prelude;
                let value = if repeated {
                    let (bind, name) = self.temp(value);
                    out.push(bind);
                    name
                } else {
                    value
                };
                self.cache_expr(result, value);
                return Ok(out);
            }
            return self.call_stmts(op);
        }
        match op.result {
            Some(result) => {
                let result_ty = self.ty_of(result)?;
                if self.is_i64_ty(result_ty) {
                    let mut stmts = Vec::new();
                    let pair = match &op.kind {
                        OpKind::Const(tree) => match self.const_i64_pair(tree, result_ty) {
                            Some(pair) => pair,
                            None => {
                                let expr = super::i64_pair::materialize(
                                    self,
                                    &mut stmts,
                                    self.op_expr(op, result)?,
                                );
                                super::i64_pair::unpack_bigint_exprs(expr)
                            }
                        },
                        OpKind::Copy(v) | OpKind::Move(v) => self.pair_of(*v)?,
                        OpKind::Unary { op: un, v, .. } => {
                            self.i64_unary_pair_stmts(&mut stmts, *un, *v)?
                        }
                        OpKind::Aggregate {
                            ty,
                            active_field,
                            fields,
                            ..
                        } if self.table.get(*ty).and_then(|d| d.js.transparent).is_some() => {
                            let descriptor = self.table.get(*ty).expect("checked transparent");
                            let inner = descriptor.js.transparent.expect("checked transparent");
                            let is_union = matches!(
                                descriptor.kind,
                                TypeKind::Adt {
                                    form: rcgjs_object::jir::types::NominalForm::Union,
                                    ..
                                }
                            );
                            if is_union && active_field.is_some_and(|active| active != inner.field)
                            {
                                (jsast::num(0.0), jsast::num(0.0))
                            } else {
                                let idx = if is_union { 0 } else { inner.field as usize };
                                match fields.get(idx) {
                                    Some(field) => self.pair_of(*field)?,
                                    None => (jsast::num(0.0), jsast::num(0.0)),
                                }
                            }
                        }
                        _ => {
                            let expr = super::i64_pair::materialize(
                                self,
                                &mut stmts,
                                self.op_expr(op, result)?,
                            );
                            super::i64_pair::unpack_bigint_exprs(expr)
                        }
                    };
                    return self.bind_i64_pair_result(result, stmts, pair);
                }
                // An inline value is spelled at its use site (`JsCx::expr_of`).
                if self.is_inline(result) {
                    return Ok(Vec::new());
                }
                if let (OpKind::Convert { v: src, .. }, Some(kind)) =
                    (&op.kind, self.dropped_string_kind(result))
                {
                    let skip = match kind {
                        DroppedStringKind::NoOp => self.only_used_by_empty_drop(result),
                        DroppedStringKind::BaselineDealloc { .. } => {
                            self.only_used_by_adjacent_drop(result)
                        }
                    };
                    if skip {
                        let mut stmts = Vec::new();
                        if matches!(kind, DroppedStringKind::NoOp) && self.is_inline(*src) {
                            let expr = self.expr_of(*src)?;
                            if !jsast::is_droppable(&expr) {
                                stmts.push(jsast::expr_stmt(expr));
                            }
                        }
                        return Ok(stmts);
                    }
                }
                let expr = self.op_expr(op, result)?;
                self.record_allocation_origin(result, &expr);
                // A zero-sized value reads as `undefined` everywhere (`expr_of`), so its
                // assignment has nothing to say; a call's effects still run.
                if self.table.is_zst(self.ty_of(result)?)
                    && !matches!(
                        op.kind,
                        OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. }
                    )
                {
                    return Ok(Vec::new());
                }
                let Some(name) = self.name_of(result) else {
                    // A value nobody reads and the plan did not name: evaluated for its effects,
                    // which for the ops of this module there are none of, but a call has.
                    return Ok(match &op.kind {
                        OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. } => {
                            vec![jsast::expr_stmt(expr)]
                        }
                        _ => Vec::new(),
                    });
                };
                Ok(vec![jsast::assign_stmt(jsast::id(name), expr)])
            }
            None => self.effect_stmts(op),
        }
    }

    fn bind_i64_pair_result(
        &self,
        result: ValueId,
        mut stmts: Vec<Stmt>,
        (lo, hi): (Expr, Expr),
    ) -> Res<Vec<Stmt>> {
        if self.is_inline(result) {
            let lo = snapshot_scratch(self, &mut stmts, lo);
            let hi = snapshot_scratch(self, &mut stmts, hi);
            self.bind_pair(result, lo, hi);
            return Ok(stmts);
        }
        let Some(name) = self.name_of(result) else {
            if !super::i64_pair::is_trivial(&lo) {
                stmts.push(jsast::expr_stmt(lo));
            }
            if !super::i64_pair::is_trivial(&hi) {
                stmts.push(jsast::expr_stmt(hi));
            }
            return Ok(stmts);
        };
        let (dst_lo, dst_hi) = super::i64_pair::pair_names(name);
        super::i64_pair::emit_assign_pair(self, &mut stmts, &dst_lo, &dst_hi, (lo, hi));
        Ok(stmts)
    }

    fn bind_scalar_result(
        &self,
        result: ValueId,
        mut stmts: Vec<Stmt>,
        expr: Expr,
        has_effect: bool,
    ) -> Res<Vec<Stmt>> {
        if self.is_inline(result) {
            self.cache_expr(result, expr);
            return Ok(stmts);
        }
        if let Some(name) = self.name_of(result) {
            stmts.push(jsast::assign_stmt(jsast::id(name), expr));
        } else if has_effect || !super::i64_pair::is_trivial(&expr) {
            stmts.push(jsast::expr_stmt(expr));
        }
        Ok(stmts)
    }

    /// Assigns the `buf`/`off`/`len` locals that physically carry one logical fat slice.
    fn assign_physical_slice(&self, result: ValueId, value: Expr) -> Res<Vec<Stmt>> {
        let Some(name) = self.name_of(result) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("physical slice {} has no name", result.0),
            ));
        };
        let (setup, value) = self.reread(value, &[ptr::BUF, ptr::OFF, ptr::LEN]);
        let mut out = Vec::with_capacity(4);
        if let Some(setup) = setup {
            out.push(jsast::expr_stmt(setup));
        }
        for (suffix, lane) in [
            ("$buf", ptr::slot_buf(value.clone())),
            ("$off", ptr::slot_off(value.clone())),
            ("$len", ptr::slot_len(value)),
        ] {
            out.push(jsast::assign_stmt(
                jsast::id(format!("{name}{suffix}")),
                lane,
            ));
        }
        Ok(out)
    }

    /// Assigns the `buf`/`off` locals that physically carry one logical thin pointer.
    fn assign_physical_slot(&self, result: ValueId, value: Expr) -> Res<Vec<Stmt>> {
        let Some(name) = self.name_of(result) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("physical slot {} has no name", result.0),
            ));
        };
        let (setup, value) = self.reread(value, &[ptr::BUF, ptr::OFF]);
        let mut out = Vec::with_capacity(3);
        if let Some(setup) = setup {
            out.push(jsast::expr_stmt(setup));
        }
        for (suffix, lane) in [
            ("$buf", ptr::slot_buf(value.clone())),
            ("$off", ptr::slot_off(value)),
        ] {
            out.push(jsast::assign_stmt(
                jsast::id(format!("{name}{suffix}")),
                lane,
            ));
        }
        Ok(out)
    }

    fn is_byte_view_intrinsic(&self, op: &Op) -> bool {
        if !crate::lower::bstr::enabled() {
            return false;
        }
        let OpKind::Intrinsic { name, args, .. } = &op.kind else {
            return false;
        };
        let Some(result) = op.result else {
            return false;
        };
        let name_s = self.interns.str(*name).unwrap_or("");
        match name_s {
            "ptr_metadata" | "read_via_copy" | "volatile_load" | "unaligned_volatile_load" => {
                args.first().is_some_and(|v| self.byte_view(*v).is_some())
            }
            "ptr_offset_from" | "ptr_offset_from_unsigned" => {
                args.len() == 2
                    && self
                        .byte_view(args[0])
                        .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr)
                    && self
                        .byte_view(args[1])
                        .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr)
            }
            "transmute"
            | "transmute_unchecked"
            | "offset"
            | "arith_offset"
            | "slice_get_unchecked" => self.byte_view(result).is_some(),
            _ => false,
        }
    }

    fn member_or_field(expr: Expr, field: &str) -> Expr {
        if let Expr::Object(entries) = &expr {
            if let Some((_, value)) = entries.iter().find(|(k, _)| k == field) {
                return value.clone();
            }
        }
        jsast::member(expr, field)
    }

    fn unsigned_add(a: Expr, b: Expr) -> Expr {
        if matches!(a, Expr::Num(n) if n == 0.0) {
            return jsast::binary(BinOp::UShr, b, jsast::num(0));
        }
        jsast::binary(BinOp::UShr, jsast::binary(BinOp::Add, a, b), jsast::num(0))
    }

    fn unsigned_sub(a: Expr, b: Expr) -> Expr {
        jsast::binary(BinOp::UShr, jsast::binary(BinOp::Sub, a, b), jsast::num(0))
    }

    fn byte_view_base_expr(&self, bv: super::ByteView, v: ValueId) -> Res<Expr> {
        if let Some(cell) = bv.base_cell {
            let base = self.cell_expr(cell)?;
            return Ok(if bv.base_is_string_obj {
                jsast::member(base, "text")
            } else {
                base
            });
        }
        if let Some(base_id) = bv.base_str {
            let base = self.expr_of(base_id)?;
            if bv.base_is_string_obj {
                return Ok(jsast::member(base, "text"));
            }
            return Ok(base);
        }
        if bv.mode == super::ByteViewMode::FullSlice {
            return self.expr_of(v);
        }
        if let Some(slice_id) = bv.borrow_slice {
            return self.expr_of(slice_id);
        }
        Err(decline(
            OpClass::Place,
            "`js-str=bytes`: byte view lacks a base string".to_owned(),
        ))
    }

    fn byte_view_off_expr(&self, v: ValueId) -> Res<Expr> {
        let Some(bv) = self.byte_view(v) else {
            return Ok(jsast::num(0));
        };
        match bv.mode {
            super::ByteViewMode::FullSlice => Ok(jsast::num(0)),
            super::ByteViewMode::VarSlice => Ok(Self::member_or_field(self.expr_of(v)?, "off")),
            super::ByteViewMode::ThinPtr => {
                if self.plan.physical_slots.contains(&v) {
                    Ok(Self::member_or_field(self.expr_of(v)?, "off"))
                } else {
                    self.expr_of(v)
                }
            }
        }
    }

    fn byte_view_len_expr(&self, v: ValueId) -> Res<Expr> {
        let Some(bv) = self.byte_view(v) else {
            return Ok(jsast::member(self.expr_of(v)?, "length"));
        };
        match bv.mode {
            super::ByteViewMode::FullSlice => {
                if self.plan.physical_slices.contains(&v) {
                    Ok(Self::member_or_field(self.expr_of(v)?, "len"))
                } else {
                    Ok(jsast::member(self.expr_of(v)?, "length"))
                }
            }
            super::ByteViewMode::VarSlice => Ok(Self::member_or_field(self.expr_of(v)?, "len")),
            super::ByteViewMode::ThinPtr => Ok(jsast::num(0)),
        }
    }

    fn byte_view_deref_expr(&self, bv: super::ByteView, ptr: ValueId) -> Res<Expr> {
        let base = if let Some(slice_id) = bv.borrow_slice {
            if self
                .byte_view(slice_id)
                .is_some_and(|s| s.mode == super::ByteViewMode::FullSlice)
            {
                self.expr_of(slice_id)?
            } else {
                self.byte_view_base_expr(bv, ptr)?
            }
        } else {
            self.byte_view_base_expr(bv, ptr)?
        };
        let off = self.byte_view_off_expr(ptr)?;
        Ok(jsast::method_call(base, "charCodeAt", vec![off]))
    }

    fn byte_view_op_expr(&self, op: &Op, result: ValueId) -> Res<Option<Expr>> {
        if !crate::lower::bstr::enabled() {
            return Ok(None);
        }
        use rcgjs_object::jir::repr::Proj;
        use rcgjs_object::jir::semantic::{BinaryOp, UnaryOp};
        match &op.kind {
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } if self.byte_view(*v).is_some() => {
                return Ok(Some(self.byte_view_len_expr(*v)?));
            }
            OpKind::Deref(ptr) if self.byte_view(*ptr).is_some() => {
                let bv = self.byte_view(*ptr).unwrap();
                return Ok(Some(self.byte_view_deref_expr(bv, *ptr)?));
            }
            OpKind::Binary {
                op:
                    bin @ (BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge),
                l,
                r,
                ..
            } if self
                .byte_view(*l)
                .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr)
                && self
                    .byte_view(*r)
                    .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr) =>
            {
                let js_op = match bin {
                    BinaryOp::Eq => BinOp::StrictEq,
                    BinaryOp::Ne => BinOp::StrictNe,
                    BinaryOp::Lt => BinOp::Lt,
                    BinaryOp::Le => BinOp::Le,
                    BinaryOp::Gt => BinOp::Gt,
                    BinaryOp::Ge => BinOp::Ge,
                    _ => unreachable!(),
                };
                return Ok(Some(jsast::binary(
                    js_op,
                    self.byte_view_off_expr(*l)?,
                    self.byte_view_off_expr(*r)?,
                )));
            }
            OpKind::Intrinsic { name, args, .. } => {
                let name_s = self.interns.str(*name).unwrap_or("");
                match name_s {
                    "ptr_metadata"
                        if args.first().is_some_and(|v| self.byte_view(*v).is_some()) =>
                    {
                        return Ok(Some(self.byte_view_len_expr(args[0])?));
                    }
                    "read_via_copy" | "volatile_load" | "unaligned_volatile_load"
                        if args.first().is_some_and(|v| self.byte_view(*v).is_some()) =>
                    {
                        let bv = self.byte_view(args[0]).unwrap();
                        return Ok(Some(self.byte_view_deref_expr(bv, args[0])?));
                    }
                    "ptr_offset_from" | "ptr_offset_from_unsigned"
                        if args.len() == 2
                            && self
                                .byte_view(args[0])
                                .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr)
                            && self
                                .byte_view(args[1])
                                .is_some_and(|v| v.mode == super::ByteViewMode::ThinPtr) =>
                    {
                        let diff = jsast::binary(
                            BinOp::Sub,
                            self.byte_view_off_expr(args[0])?,
                            self.byte_view_off_expr(args[1])?,
                        );
                        let masked = if name_s == "ptr_offset_from" {
                            jsast::binary(BinOp::BitOr, diff, jsast::num(0))
                        } else {
                            jsast::binary(BinOp::UShr, diff, jsast::num(0))
                        };
                        return Ok(Some(masked));
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        let Some(res_bv) = self.byte_view(result) else {
            return Ok(None);
        };

        let expr = match &op.kind {
            OpKind::Cast { v, .. } | OpKind::Copy(v) | OpKind::Move(v) => {
                self.byte_view_cast_or_alias(*v, res_bv, result)?
            }
            OpKind::Intrinsic { name, args, .. }
                if matches!(
                    self.interns.str(*name).unwrap_or(""),
                    "transmute" | "transmute_unchecked"
                ) && !args.is_empty() =>
            {
                self.byte_view_cast_or_alias(args[0], res_bv, result)?
            }
            OpKind::RawView {
                root: Root::Value(v),
                path,
                ..
            }
            | OpKind::BorrowView {
                root: Root::Value(v),
                path,
                ..
            } => match path.as_slice() {
                [Proj::Deref] => self.byte_view_cast_or_alias(*v, res_bv, result)?,
                [Proj::Deref, Proj::Index(idx)] => {
                    let base_off = self.byte_view_off_expr(*v)?;
                    Self::unsigned_add(base_off, self.expr_of(*idx)?)
                }
                [Proj::Deref, Proj::ConstantIndex {
                    offset, from_end, ..
                }] => {
                    let base_off = self.byte_view_off_expr(*v)?;
                    let idx = if *from_end {
                        Self::unsigned_sub(self.byte_view_len_expr(*v)?, jsast::num(*offset as f64))
                    } else {
                        jsast::num(*offset as f64)
                    };
                    if matches!(base_off, Expr::Num(n) if n == 0.0) {
                        idx
                    } else {
                        Self::unsigned_add(base_off, idx)
                    }
                }
                [Proj::Deref, Proj::Subslice { from, to, from_end }] => {
                    let base = self.byte_view_base_expr(res_bv, result)?;
                    let base_off = self.byte_view_off_expr(*v)?;
                    let new_off = if *from == 0 {
                        base_off
                    } else if matches!(base_off, Expr::Num(n) if n == 0.0) {
                        jsast::num(*from as f64)
                    } else {
                        Self::unsigned_add(base_off, jsast::num(*from as f64))
                    };
                    let new_len = if *from_end {
                        let base_len = self.byte_view_len_expr(*v)?;
                        let trimmed = from.saturating_add(*to);
                        if trimmed == 0 {
                            base_len
                        } else {
                            Self::unsigned_sub(base_len, jsast::num(trimmed as f64))
                        }
                    } else {
                        jsast::num(to.saturating_sub(*from) as f64)
                    };
                    jsast::object_of(vec![("buf", base), ("off", new_off), ("len", new_len)])
                }
                _ => return Ok(None),
            },
            OpKind::SliceView { ptr, len } => {
                let base = self.byte_view_base_expr(res_bv, result)?;
                if res_bv.mode == super::ByteViewMode::FullSlice {
                    base
                } else if self.inlined_vec_cluster_of(result).is_some() {
                    let len_expr = jsast::member(base.clone(), "length");
                    jsast::object_of(vec![
                        ("buf", base),
                        ("off", jsast::num(0)),
                        ("len", len_expr),
                    ])
                } else {
                    let off = self.byte_view_off_expr(*ptr)?;
                    let len_expr = self.expr_of(*len)?;
                    jsast::object_of(vec![("buf", base), ("off", off), ("len", len_expr)])
                }
            }
            OpKind::Call {
                semantic:
                    rcgjs_object::jir::semantic::CallSemantic::Standard(
                        rcgjs_object::jir::semantic::StandardOperation::VecAsSlice,
                    ),
                ..
            } if self.inlined_vec_cluster_of(result).is_some() => {
                let base = self.byte_view_base_expr(res_bv, result)?;
                match res_bv.mode {
                    super::ByteViewMode::FullSlice => base,
                    super::ByteViewMode::VarSlice => {
                        let len_expr = jsast::member(base.clone(), "length");
                        jsast::object_of(vec![
                            ("buf", base),
                            ("off", jsast::num(0)),
                            ("len", len_expr),
                        ])
                    }
                    super::ByteViewMode::ThinPtr => return Ok(None),
                }
            }
            OpKind::Binary {
                op: BinaryOp::Offset,
                l,
                r,
                ..
            } => Self::unsigned_add(self.byte_view_off_expr(*l)?, self.expr_of(*r)?),
            OpKind::Intrinsic { name, args, .. }
                if matches!(
                    self.interns.str(*name).unwrap_or(""),
                    "offset" | "arith_offset" | "slice_get_unchecked"
                ) && args.len() >= 2 =>
            {
                Self::unsigned_add(self.byte_view_off_expr(args[0])?, self.expr_of(args[1])?)
            }
            _ => return Ok(None),
        };
        Ok(Some(expr))
    }

    fn byte_view_cast_or_alias(
        &self,
        src: ValueId,
        res_bv: super::ByteView,
        result: ValueId,
    ) -> Res<Expr> {
        match res_bv.mode {
            super::ByteViewMode::FullSlice => self.expr_of(src),
            super::ByteViewMode::VarSlice => {
                if self
                    .byte_view(src)
                    .is_some_and(|sbv| sbv.mode == super::ByteViewMode::VarSlice)
                {
                    self.expr_of(src)
                } else {
                    let base = self.byte_view_base_expr(res_bv, result)?;
                    let len = if self.byte_view(src).is_some() {
                        self.byte_view_len_expr(src)?
                    } else {
                        jsast::member(base.clone(), "length")
                    };
                    Ok(jsast::object_of(vec![
                        ("buf", base),
                        ("off", jsast::num(0)),
                        ("len", len),
                    ]))
                }
            }
            super::ByteViewMode::ThinPtr => self.byte_view_off_expr(src),
        }
    }

    /// Stage 3 (`js-str=bytes`): looks up a closed MIR-inlined `String.vec` cluster if `v`
    /// belongs to one AND the planner committed the `String` root natively (`HostString` or
    /// `StringBuilder`). Always returns `None` when `bstr` is disabled or `root` stayed Baseline.
    fn inlined_vec_cluster_of(
        &self,
        v: ValueId,
    ) -> Option<(
        rcgjs_object::jir::repr::Carrier,
        &crate::lower::semantic_ops::string::InlinedVecCluster,
    )> {
        if !crate::lower::bstr::enabled() {
            return None;
        }
        let analysis = self.inlined_string_vec()?;
        let &cid = analysis.value_cluster.get(&v)?;
        let cluster = analysis.clusters.get(cid)?;
        let carrier = cluster
            .witness_values
            .first()
            .map(|w| self.func.values[w.0 as usize].carrier)
            .filter(|c| {
                matches!(
                    c,
                    rcgjs_object::jir::repr::Carrier::HostString
                        | rcgjs_object::jir::repr::Carrier::StringBuilder
                )
            })?;
        Some((carrier, cluster))
    }

    fn inlined_vec_text_expr(
        &self,
        carrier: rcgjs_object::jir::repr::Carrier,
        root: crate::lower::semantic_ops::string::StringRoot,
    ) -> Res<Expr> {
        let base = match root {
            crate::lower::semantic_ops::string::StringRoot::Cell(c) => {
                let walk = self.walk_place(&rcgjs_object::jir::repr::Root::Cell(c), &[])?;
                self.read_place(&walk.place)
            }
            crate::lower::semantic_ops::string::StringRoot::Value(v) => self.expr_of(v)?,
        };
        Ok(match carrier {
            rcgjs_object::jir::repr::Carrier::StringBuilder => jsast::member(base, "text"),
            _ => base,
        })
    }

    fn is_inlined_vec_bypass_call(&self, op: &Op) -> bool {
        if !crate::lower::bstr::enabled() {
            return false;
        }
        let OpKind::Call {
            semantic: rcgjs_object::jir::semantic::CallSemantic::Standard(std_op),
            ..
        } = &op.kind
        else {
            return false;
        };
        if !matches!(
            std_op,
            rcgjs_object::jir::semantic::StandardOperation::VecLen
                | rcgjs_object::jir::semantic::StandardOperation::VecAsSlice
        ) {
            return false;
        }
        op.result
            .is_some_and(|r| self.inlined_vec_cluster_of(r).is_some())
    }

    fn inlined_vec_reserve_stmts(&self, op: &Op) -> Res<Option<Vec<Stmt>>> {
        if !crate::lower::bstr::enabled() {
            return Ok(None);
        }
        let OpKind::Call {
            callee,
            args,
            semantic,
            host: None,
            ..
        } = &op.kind
        else {
            return Ok(None);
        };
        let (anchor, add, is_raw_vec) = match semantic {
            rcgjs_object::jir::semantic::CallSemantic::Standard(
                rcgjs_object::jir::semantic::StandardOperation::VecReserve,
            ) => {
                if args.len() != 2 {
                    return Ok(None);
                }
                (args[0], args[1], false)
            }
            rcgjs_object::jir::semantic::CallSemantic::Ordinary => {
                let rcgjs_object::jir::semantic::Callee::Direct { item, link_name } = callee else {
                    return Ok(None);
                };
                if args.len() != 4
                    || !crate::lower::semantic_ops::string::is_raw_vec_reserve_by_link_name(
                        self.interns,
                        Some(self.items),
                        *item,
                        *link_name,
                    )
                {
                    return Ok(None);
                }
                (args[0], args[2], true)
            }
            _ => return Ok(None),
        };
        let Some((carrier, cluster)) = self.inlined_vec_cluster_of(anchor) else {
            return Ok(None);
        };
        if carrier != rcgjs_object::jir::repr::Carrier::StringBuilder
            || !cluster.internal_values.contains(&anchor)
        {
            return Ok(None);
        }
        if is_raw_vec && !cluster.len_values.contains(&args[1]) {
            return Ok(None);
        }
        let sb_expr = match cluster.root {
            crate::lower::semantic_ops::string::StringRoot::Cell(c) => {
                let walk = self.walk_place(&rcgjs_object::jir::repr::Root::Cell(c), &[])?;
                self.read_place(&walk.place)
            }
            crate::lower::semantic_ops::string::StringRoot::Value(v) => self.expr_of(v)?,
        };
        let add_expr = self.expr_of(add)?;
        let ptr_bits = self
            .table
            .get(self.ty_of(anchor)?)
            .and_then(|d| d.layout.size)
            .and_then(|b| b.checked_mul(8))
            .and_then(|bits| u32::try_from(bits).ok())
            .unwrap_or(32);
        let max = crate::lower::semantic_ops::pointer_limit(ptr_bits)
            .map_err(|e| decline(OpClass::Call, format!("reserve limit: {e:?}")))?;
        let (sb_init, sb_id) = self.temp(sb_expr);
        let (_, req_id) = self.temp(jsast::undefined());
        let req_target = req_id.clone();
        let mut helpers = std::collections::BTreeSet::new();
        let stmts = crate::lower::semantic_ops::string::reserve_builder_stmts_with(
            sb_init,
            sb_id,
            "",
            |len_expr| {
                jsast::assign_stmt(req_target, jsast::binary(BinOp::Add, len_expr, add_expr))
            },
            req_id,
            max,
            &mut helpers,
        );
        Ok(Some(stmts))
    }

    fn inlined_vec_op_expr(&self, _op: &Op, result: ValueId) -> Res<Option<Expr>> {
        let Some((carrier, cluster)) = self.inlined_vec_cluster_of(result) else {
            return Ok(None);
        };
        if cluster.internal_values.contains(&result) {
            return Ok(Some(jsast::num(0)));
        }
        if cluster.len_values.contains(&result) {
            let text = self.inlined_vec_text_expr(carrier, cluster.root)?;
            return Ok(Some(jsast::member(text, "length")));
        }
        if cluster.slice_values.contains(&result) {
            let text = self.inlined_vec_text_expr(carrier, cluster.root)?;
            return Ok(Some(jsast::rt_call("bstr_bytes", vec![text])));
        }
        Ok(None)
    }

    /// The expression of a value-producing op (v1 `codegen_rvalue`/`codegen_operand`).
    pub(super) fn op_expr(&self, op: &Op, result: ValueId) -> Res<Expr> {
        let table = self.table;
        let result_ty = self.ty_of(result)?;
        // Every zero-sized value is `undefined` (`rvalue.rs:35-37`), whatever op produced it —
        // except a call, whose effects run either way, and a view, which describes a place even
        // when the SSA builder uses its zero-sized pointee type as a placeholder.
        let zst = table.is_zst(result_ty)
            && !matches!(
                op.kind,
                OpKind::Call { .. }
                    | OpKind::Intrinsic { .. }
                    | OpKind::Host { .. }
                    | OpKind::BorrowView { .. }
                    | OpKind::RawView { .. }
                    | OpKind::SliceView { .. }
            );
        if zst {
            // The operands are values, so reading them has no effect to preserve.
            if let OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. } = op.kind {
            } else {
                return Ok(jsast::undefined());
            }
        }
        if let Some(expr) = self.byte_view_op_expr(op, result)? {
            return Ok(expr);
        }
        if let Some(expr) = self.inlined_vec_op_expr(op, result)? {
            return Ok(expr);
        }
        match &op.kind {
            OpKind::Const(tree) => self.const_expr(tree, result_ty),
            OpKind::Copy(v) => {
                let value = self.expr_of(*v)?;
                // A carried overflow pair is a scalar (`rvalue::carried_overflow_pairs`), and
                // nothing mutates one: its copy is the same value.
                if self.carried_pairs.contains_key(&result) {
                    return Ok(value);
                }
                let ty = self.ty_of(*v)?;
                // A committed carrier is cloned at its own shape: the baseline field-wise clone
                // reads a structure the value no longer has (Phase 4).
                match self.func.values[v.0 as usize].carrier {
                    rcgjs_object::jir::repr::Carrier::HostString => Ok(value),
                    rcgjs_object::jir::repr::Carrier::StringBuilder => {
                        Ok(record_clone(value, &["text", "cap", "len"], &[]))
                    }
                    rcgjs_object::jir::repr::Carrier::NativeVec => {
                        Ok(record_clone(value, &["len", "cap"], &["buf"]))
                    }
                    rcgjs_object::jir::repr::Carrier::SliceView => {
                        Ok(record_clone(value, &["buf", "off", "len"], &[]))
                    }
                    _ => Ok(if table.needs_clone(ty) {
                        value::clone_expr_with(table, self.interns, ty, value, self.nested_clone)
                    } else {
                        value
                    }),
                }
            }
            OpKind::Move(v) => self.expr_of(*v),
            OpKind::Aggregate {
                ty,
                variant,
                active_field,
                fields,
            } => {
                let data_ty = fields.first().map(|field| self.ty_of(*field)).transpose()?;
                let values = fields
                    .iter()
                    .map(|field| self.expr_of(*field))
                    .collect::<Res<Vec<_>>>()?;
                self.aggregate(*ty, *variant, *active_field, values, data_ty)
            }
            OpKind::Repeat { v, count } => {
                let element_ty = self.ty_of(*v)?;
                let is_constant = self.is_inline(*v)
                    && self
                        .def_of(*v)
                        .is_some_and(|def| matches!(def.kind, OpKind::Const(_)));
                Ok(self.repeat(element_ty, self.expr_of(*v)?, *count, is_constant))
            }
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                if let Some(&pair) = self.carried_pairs.get(base) {
                    return self.carried_pair_field(pair, *base, *field);
                }
                let base_ty = self.ty_of(*base)?;
                let Some(descriptor) = table.field(base_ty, *variant, *field) else {
                    return Err(decline(
                        OpClass::Place,
                        format!("field {field} of a type with no such field"),
                    ));
                };
                Ok(value::field_expr(
                    self.interns,
                    self.expr_of(*base)?,
                    descriptor.key,
                ))
            }
            OpKind::Discriminant(base) => {
                let base_ty = self.ty_of(*base)?;
                let as_tag = self.plan.tag_values.contains(&result);
                self.discriminant(base_ty, self.expr_of(*base)?, result_ty, as_tag)
            }
            OpKind::CellLoad(cell) => self.cell_expr(*cell),
            OpKind::BorrowView {
                root,
                path,
                mutable,
            } => self.borrow_view(root, path, *mutable),
            OpKind::RawView {
                root,
                path,
                mutable,
            } => self.raw_view(root, path, *mutable),
            OpKind::SliceView { ptr: pointer, len } => {
                let pointer_ty = self.ty_of(*pointer)?;
                let data = self.expr_of(*pointer)?;
                let len = self.expr_of(*len)?;
                let Some(pointee) = value::pointee_of(table, result_ty) else {
                    return Err(decline(OpClass::Rvalue, "SliceView at a non-pointer type"));
                };
                // A reference to an array: the array object is the buffer when the slice's
                // element type matches the array's element type. The test is on the pointee as
                // well as the form, because a thin form says only that the pointer is its
                // pointee's reference form. `from_raw_parts` takes its data half as any thin
                // pointer (`*const ()`, `*mut MaybeUninit<[T; N]>`, etc.); `build_fat` converts
                // it from `pointer_ty` to the slice element pointer, as v1's `aggregate_raw_ptr`
                // does.
                let array_reference = table.pointer_form(pointer_ty) == Some(PointerForm::Thin)
                    && matches!(
                        (
                            value::pointee_of(table, pointer_ty)
                                .and_then(|pointee| value::kind(table, pointee)),
                            value::kind(table, pointee),
                        ),
                        (
                            Some(TypeKind::Array {
                                element: arr_element,
                                ..
                            }),
                            Some(TypeKind::Slice(slice_element)),
                        ) if table.peeled(*arr_element) == table.peeled(*slice_element)
                    );
                if array_reference {
                    Ok(ptr::fat_slice(data, jsast::num(0), len))
                } else {
                    self.build_fat(
                        pointee,
                        value::is_mut_str_ptr(table, result_ty),
                        Some(pointer_ty),
                        data,
                        len,
                    )
                }
            }
            OpKind::Deref(pointer) => {
                let walk = self.pointee_place(*pointer)?;
                Ok(self.read_place(&walk.place))
            }
            OpKind::Binary { op: bin, l, r, ty } => {
                if let Some(pair) = self.carried_pairs.get(&result) {
                    let (lhs, rhs) = (self.expr_of(*l)?, self.expr_of(*r)?);
                    return self.carried_pair_value(pair.carrier, *bin, *ty, lhs, rhs);
                }
                let rhs_ty = self.ty_of(*r)?;
                self.binary(
                    *bin,
                    *ty,
                    rhs_ty,
                    result_ty,
                    self.expr_of(*l)?,
                    self.expr_of(*r)?,
                )
            }
            // `js-str=bytes`: closed byte-string views are handled by `byte_view_op_expr` above.
            OpKind::Unary { op: un, v, ty } => self.unary(*un, *ty, self.expr_of(*v)?),
            OpKind::Cast { kind, v, from, to } => self.cast(*kind, *from, *to, self.expr_of(*v)?),
            OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. } => {
                self.call_expr(op)
            }
            // A spill register: a module-scope `let` the flat direct-call ABI writes on the way
            // out of a callee and the caller reads back (`jsc_link::plan::abi_flat`).
            OpKind::SpillRead(index) => Ok(jsast::id(spill_name_of(self.spills, *index))),
            OpKind::Materialize(_) => Err(decline(
                OpClass::Rvalue,
                "Materialize is a Phase 3 op with no baseline lowering",
            )),
            // A carrier conversion: the registry-owned adapter expression over the value
            // (`adapters.rs`). The value's Rust type is unchanged — `result_ty` is it.
            OpKind::Convert { v, from, to } => super::adapters::convert(
                self.table,
                self.interns,
                self.expr_of(*v)?,
                result_ty,
                *from,
                *to,
            ),
            OpKind::SetField { .. }
            | OpKind::SetDiscriminant { .. }
            | OpKind::CellAlloc(_)
            | OpKind::CellStore { .. }
            | OpKind::StoreThrough { .. }
            | OpKind::Drop { .. }
            | OpKind::Assert { .. }
            | OpKind::Zombie(_)
            | OpKind::SpillWrite { .. } => {
                Err(decline(OpClass::Statement, "an effect op carries a result"))
            }
        }
    }

    /// A `CellAlloc` that creates its cell's container ([`plan::Plan::fresh_allocs`]): the cell
    /// holds an aggregate that MIR assembles field by field, so its storage is a new empty array or
    /// object (v1 `assembly.rs`).
    fn fresh_container(&self, cell: CellId) -> Res<Vec<Stmt>> {
        let Some(container) = value::fresh_container(self.table, self.cell_ty(cell)?) else {
            return Ok(Vec::new());
        };
        Ok(vec![jsast::assign_stmt(self.cell_expr(cell)?, container)])
    }

    /// The statements of an op with no result.
    fn effect_stmts(&self, op: &Op) -> Res<Vec<Stmt>> {
        let table = self.table;
        match &op.kind {
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => {
                let base_ty = self.ty_of(*base)?;
                let Some(descriptor) = table.field(base_ty, *variant, *field) else {
                    return Err(decline(
                        OpClass::Place,
                        format!("field {field} of a type with no such field"),
                    ));
                };
                let value = self.expr_of(*v)?;
                if table.is_zst(descriptor.ty) {
                    // Nothing to store; the value is a JS value with no effect to keep.
                    return Ok(Vec::new());
                }
                let target = value::field_expr(self.interns, self.expr_of(*base)?, descriptor.key);
                self.write_place(JsPlace::Var(target), descriptor.ty, value)
            }
            OpKind::SetDiscriminant { base, variant } => {
                let base_ty = self.ty_of(*base)?;
                let target = self.expr_of(*base)?;
                self.set_discriminant(JsPlace::Var(target), base_ty, *variant)
            }
            // The prelude declares the cell (`let x = []` when boxed); bringing it into existence
            // is nothing at the alloc site, exactly as v1 declared every local once. The allocs
            // that make an aggregate's container are `payload`'s (`fresh_container`).
            OpKind::CellAlloc(_) => Ok(Vec::new()),
            OpKind::CellStore { cell, v } => {
                let value = self.expr_of(*v)?;
                // A boxed parameter cell: v1 builds the box in the prelude, `x = [x];`, so the
                // parameter's own binding becomes the box — only when the parameter's canonical
                // name is actually a formal parameter in the JS signature (not expanded into lanes).
                if self.plan.param_cells.get(cell).is_some_and(|p| p == v) {
                    let cell_name = self.cell_name(*cell)?;
                    if self.is_formal_param_name(cell_name) {
                        if !self.cell_boxed(*cell) {
                            return Ok(Vec::new());
                        }
                        let name = jsast::id(cell_name);
                        return Ok(vec![jsast::assign_stmt(name, jsast::array(vec![value]))]);
                    }
                }
                Ok(vec![jsast::assign_stmt(self.cell_expr(*cell)?, value)])
            }
            OpKind::StoreThrough { ptr: pointer, v } => {
                // The store-back idiom an in-place mutation leaves behind
                // (`ssa.rs::set_discriminant`: `Deref` the view, mutate, store it back). Storing
                // a value back where it was just read from is a no-op — for an object pointee
                // the mutation already happened where the object lives, and for a scalar one the
                // value is unchanged — and `__rt.overwrite` implements a whole-aggregate write by
                // clearing the target first, so spelling it would *erase* the object.
                if self.stores_back(*pointer, *v) {
                    return Ok(Vec::new());
                }
                let walk = self.pointee_place(*pointer)?;
                let value = self.expr_of(*v)?;
                self.write_place(walk.place, walk.ty, value)
            }
            OpKind::Assert {
                cond,
                expected,
                msg,
            } => self.assert_stmts(*cond, *expected, msg),
            OpKind::Zombie(message) => {
                let message = self.interns.str(*message).unwrap_or("").to_owned();
                Ok(vec![jsast::expr_stmt(self.zombie(message))])
            }
            OpKind::SpillWrite { index, v } => Ok(vec![jsast::assign_stmt(
                jsast::id(spill_name_of(self.spills, *index)),
                self.expr_of(*v)?,
            )]),
            // A call, an intrinsic, a host operation and a drop never reach here: `op_stmts`
            // routes them through `call.rs` before it looks at the result.
            _ => Err(decline(OpClass::Statement, "a value op carries no result")),
        }
    }

    /// The expression of a call-shaped op at a use site the plan inlined it into.
    ///
    /// An inlined value has no statement position, so a lowering that needs one is folded into a
    /// comma expression where every statement is an assignment, and declined otherwise.
    fn call_expr(&self, op: &Op) -> Res<Expr> {
        let lowered = self.lower_call_like(op)?;
        lowered.into_expr().ok_or_else(|| {
            decline(
                OpClass::Call,
                "an inlined call whose lowering needs a statement position",
            )
        })
    }

    /// The statements of a call-shaped op: what its lowering has to run, then the binding of its
    /// result (`base.rs`'s `write_place` after a call).
    fn call_stmts(&self, op: &Op) -> Res<Vec<Stmt>> {
        let (mut out, value) = match self.lower_call_like(op)? {
            Lowered::Done(stmts) => return Ok(stmts),
            Lowered::Value(prelude, value) => (prelude, value),
            Lowered::Pair(prelude, pair) => {
                let Some(result) = op.result else {
                    let mut out = prelude;
                    if !super::i64_pair::is_trivial(&pair.0) {
                        out.push(jsast::expr_stmt(pair.0));
                    }
                    return Ok(out);
                };
                return self.bind_i64_pair_result(result, prelude, pair);
            }
            Lowered::Virtual(prelude, value) => {
                let Some(result) = op.result else {
                    return Err(decline(
                        OpClass::Call,
                        "a virtual tagged call result has no JIR value",
                    ));
                };
                self.bind_tagged_expr(result, value);
                return Ok(prelude);
            }
        };
        if let Some(result) = op.result {
            if self.is_i64_ty(self.ty_of(result)?) {
                let value = super::i64_pair::materialize(self, &mut out, value);
                let pair = super::i64_pair::unpack_bigint_exprs(value);
                return self.bind_i64_pair_result(result, out, pair);
            }
        }
        // A `Callee::Nop` call is nothing at all, and a zero-sized result is `undefined`
        // everywhere (`expr_of`), so neither is worth a statement of its own.
        if matches!(value, Expr::Undefined) {
            return Ok(out);
        }
        let mut named = None;
        if let Some(result) = op.result {
            if !self.table.is_zst(self.ty_of(result)?) {
                named = self.name_of(result);
            }
        }
        match named {
            Some(name) => out.push(jsast::assign_stmt(jsast::id(name), value)),
            // Evaluated for its effects, which a call has and nothing else reaching here does.
            None => out.push(jsast::expr_stmt(value)),
        }
        Ok(out)
    }

    /// Whether the one use of an inlined pointer value reads the pointer record more than once.
    ///
    /// A `{ buf, off }` slot is dereferenced as `p.buf[p.off]` and a `{ ptr, meta }` fat pointer
    /// as `p.ptr`/`p.meta`, so a *call* spelled there would run once per field. Such a value is
    /// bound to a temporary at its definition instead, which is what v1's `ptr.rs::reread` does
    /// for every pointer expression it repeats. Everything else — an argument, a field read, an
    /// operand of an operator — spells its operand once and keeps the call inline.
    fn reads_pointer_twice(&self, v: ValueId) -> Res<bool> {
        let repeatable = matches!(
            self.table.pointer_form(self.ty_of(v)?),
            Some(
                PointerForm::Slot
                    | PointerForm::FatSlice
                    | PointerForm::FatDyn
                    | PointerForm::FatTail
            )
        );
        if !repeatable {
            return Ok(false);
        }
        for block in &self.func.blocks {
            for op in &block.ops {
                if !op.kind.uses().contains(&v) {
                    continue;
                }
                return Ok(match &op.kind {
                    OpKind::Deref(pointer) | OpKind::StoreThrough { ptr: pointer, .. } => {
                        *pointer == v
                    }
                    OpKind::BorrowView {
                        root: Root::Value(root),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Value(root),
                        ..
                    }
                    | OpKind::Drop {
                        root: Root::Value(root),
                        ..
                    } => *root == v,
                    OpKind::SliceView { ptr: pointer, .. } => *pointer == v,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l,
                        ..
                    } => *l == v,
                    _ => false,
                });
            }
        }
        Ok(false)
    }

    /// Whether `v` is the value `Deref` of `pointer` produced, carried by renames and by copies
    /// that alias rather than clone.
    ///
    /// A `Move`, and a `Copy` of a type that does not `needs_clone`, are the same JavaScript
    /// value; a `Copy` that clones is a different object and a store of it is a real write.
    fn stores_back(&self, pointer: ValueId, v: ValueId) -> bool {
        let mut value = v;
        for _ in 0..8 {
            let Some(def) = self.def_of(value) else {
                return false;
            };
            match def.kind {
                OpKind::Deref(source) => return source == pointer,
                OpKind::Move(inner) => value = inner,
                OpKind::Copy(inner) => {
                    let Ok(ty) = self.ty_of(inner) else {
                        return false;
                    };
                    if self.table.needs_clone(ty) {
                        return false;
                    }
                    value = inner;
                }
                _ => return false,
            }
        }
        false
    }

    /// The place a pointer value points at: the walked place itself for an inline synthetic view
    /// (`plan.rs`, "Synthetic views": a `BorrowView` consumed by exactly one `Deref`/`StoreThrough`
    /// is spelled as v1's read/write of the place, never as a pointer record), and the `Deref`
    /// of the pointer expression otherwise.
    pub(crate) fn pointee_place(&self, pointer: ValueId) -> Res<super::place::Walk> {
        if self.is_inline(pointer) {
            if let Some(Op {
                kind: OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. },
                ..
            }) = self.def_of(pointer)
            {
                return self.walk_place(root, path);
            }
        }
        self.walk_place(
            &Root::Value(pointer),
            &[rcgjs_object::jir::repr::Proj::Deref],
        )
    }

    /// An `Assert` (`base.rs:707-723`, `857-901`): `if (!cond) { <panic> }`.
    fn assert_stmts(&self, cond: ValueId, expected: bool, msg: &RAssertMessage) -> Res<Vec<Stmt>> {
        // `is_optional_overflow_check`: the arithmetic overflow checks rustc emits regardless of
        // `-Coverflow-checks`, which v1 drops when the session has them off. The link step does
        // not know the session flag; the suite compiles with them off.
        if matches!(
            msg,
            RAssertMessage::Overflow {
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Shl | BinaryOp::Shr,
                ..
            }
        ) {
            return Ok(Vec::new());
        }
        let condition = self.expr_of(cond)?;
        let failed = if expected {
            jsast::not(condition)
        } else {
            condition
        };
        Ok(vec![jsast::if_(failed, self.assert_failure(msg)?)])
    }

    /// The body of the `if` an `Assert` guards (`base.rs::codegen_assert_failure`).
    fn assert_failure(&self, msg: &RAssertMessage) -> Res<Vec<Stmt>> {
        let (lang_fn, args): (&str, Vec<ValueId>) = match msg {
            RAssertMessage::BoundsCheck { length, index } => {
                ("panic_bounds_check", vec![*index, *length])
            }
            RAssertMessage::MisalignedPointer { required, found } => (
                "panic_misaligned_pointer_dereference",
                vec![*required, *found],
            ),
            RAssertMessage::InvalidEnum(source) => {
                ("panic_invalid_enum_construction", vec![*source])
            }
            other => (panic_function(other), Vec::new()),
        };
        // The panic lang item is a `Direct` call the encoder did not record: it is found among
        // the item's baseline references by its path's last segment.
        let candidates: Vec<&String> = self
            .summary
            .deps
            .iter()
            .filter(|dep| names_lang_fn(dep, lang_fn))
            .collect();
        match candidates.as_slice() {
            [name] => {
                let mut arguments = args
                    .iter()
                    .map(|arg| self.expr_of(*arg))
                    .collect::<Res<Vec<_>>>()?;
                // The panic lang items are `#[track_caller]`: the location of the expression that
                // failed, or the enclosing `$loc` in a `#[track_caller]` body.
                if self.func.sig.abi.caller_location {
                    arguments.push(jsast::id(CALLER_LOCATION_PARAM));
                } else {
                    let origin = self.origin.get();
                    let Some(fact) = origin.and_then(|origin| self.fact_of(origin)) else {
                        return Err(decline(
                            OpClass::Terminator,
                            "an assert whose panic takes a caller location has no source fact",
                        ));
                    };
                    arguments.push(self.caller_location_value(fact)?);
                }
                Ok(vec![jsast::expr_stmt(jsast::call(
                    jsast::id((*name).clone()),
                    arguments,
                ))])
            }
            [] => Ok(vec![jsast::expr_stmt(jsast::rt_call(
                "js_abort",
                vec![jsast::string(assert_message(msg))],
            ))]),
            _ => Err(decline(
                OpClass::Terminator,
                format!("the `{lang_fn}` panic function is ambiguous among the item's references"),
            )),
        }
    }
}

/// Whether an emitted item name is `<path>$<lang_fn>$h<hash>`.
fn names_lang_fn(name: &str, lang_fn: &str) -> bool {
    let Some((stem, hash)) = name.rsplit_once("$h") else {
        return false;
    };
    if hash.len() != 16 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    stem == lang_fn || stem.ends_with(&format!("${lang_fn}"))
}

/// The lang item function name of an `AssertKind` (`AssertKind::panic_function`).
fn panic_function(msg: &RAssertMessage) -> &'static str {
    match msg {
        RAssertMessage::Overflow { op, .. } => match op {
            BinaryOp::Add => "panic_const_add_overflow",
            BinaryOp::Sub => "panic_const_sub_overflow",
            BinaryOp::Mul => "panic_const_mul_overflow",
            BinaryOp::Div => "panic_const_div_overflow",
            BinaryOp::Rem => "panic_const_rem_overflow",
            BinaryOp::Shr => "panic_const_shr_overflow",
            BinaryOp::Shl => "panic_const_shl_overflow",
            _ => "panic_const_generic_overflow",
        },
        RAssertMessage::OverflowNeg(_) => "panic_const_neg_overflow",
        RAssertMessage::DivisionByZero(_) => "panic_const_div_by_zero",
        RAssertMessage::RemainderByZero(_) => "panic_const_rem_by_zero",
        RAssertMessage::ResumedAfterReturn(kind) => match kind {
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Async,
                ..
            } => "panic_const_async_fn_resumed",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::AsyncGen,
                ..
            } => "panic_const_async_gen_fn_resumed",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Gen,
                ..
            } => "panic_const_gen_fn_none",
            CoroutineKind::Coroutine { .. } => "panic_const_coroutine_resumed",
        },
        RAssertMessage::ResumedAfterPanic(kind) => match kind {
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Async,
                ..
            } => "panic_const_async_fn_resumed_panic",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::AsyncGen,
                ..
            } => "panic_const_async_gen_fn_resumed_panic",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Gen,
                ..
            } => "panic_const_gen_fn_none_panic",
            CoroutineKind::Coroutine { .. } => "panic_const_coroutine_resumed_panic",
        },
        RAssertMessage::ResumedAfterDrop(kind) => match kind {
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Async,
                ..
            } => "panic_const_async_fn_resumed_drop",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::AsyncGen,
                ..
            } => "panic_const_async_gen_fn_resumed_drop",
            CoroutineKind::Desugared {
                kind: CoroutineDesugaring::Gen,
                ..
            } => "panic_const_gen_fn_none_drop",
            CoroutineKind::Coroutine { .. } => "panic_const_coroutine_resumed_drop",
        },
        RAssertMessage::NullPointer => "panic_null_pointer_dereference",
        RAssertMessage::NullReference => "panic_null_reference_construction",
        RAssertMessage::BoundsCheck { .. } => "panic_bounds_check",
        RAssertMessage::MisalignedPointer { .. } => "panic_misaligned_pointer_dereference",
        RAssertMessage::InvalidEnum(_) => "panic_invalid_enum_construction",
    }
}

/// What a failed assertion says when the program defines no panic lang item (`base.rs:1022`).
fn assert_message(msg: &RAssertMessage) -> &'static str {
    match msg {
        RAssertMessage::BoundsCheck { .. } => "index out of bounds",
        RAssertMessage::DivisionByZero(..) => "attempt to divide by zero",
        RAssertMessage::RemainderByZero(..) => {
            "attempt to calculate the remainder with a divisor of zero"
        }
        RAssertMessage::OverflowNeg(..) => "attempt to negate with overflow",
        RAssertMessage::Overflow { .. } => "attempt to compute with overflow",
        _ => "assertion failed",
    }
}

fn snapshot_scratch(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, expr: Expr) -> Expr {
    let is_scratch = matches!(
        &expr,
        Expr::Ident(name)
            if name.as_str() == super::i64_pair::HELPER_HI
                || name.as_str() == super::i64_pair::HELPER_OVF
    );
    if is_scratch {
        let (bind, ident) = cx.temp(expr);
        stmts.push(bind);
        ident
    } else {
        expr
    }
}

/// Keeps `EnumRepr` in scope for the documentation of the tag rules above.
#[allow(dead_code)]
fn _repr(_: EnumRepr) {}

/// A fresh record over a carrier value: `fields` copied as they are, `sliced` copied with
/// `.slice()` (the one owned buffer a native Vec clone must not share). Evaluates `value` once.
fn record_clone(value: Expr, fields: &[&str], sliced: &[&str]) -> Expr {
    let (value, wrap): (Expr, Box<dyn FnOnce(Expr) -> Expr>) = if matches!(&value, Expr::Ident(_)) {
        (value, Box::new(|body| body))
    } else {
        (
            jsast::id("$cp"),
            Box::new(move |body| {
                jsast::call(jsast::arrow(vec!["$cp".to_owned()], body), vec![value])
            }),
        )
    };
    let mut entries = Vec::new();
    for field in sliced {
        entries.push((
            (*field).to_owned(),
            jsast::method_call(jsast::member(value.clone(), *field), "slice", Vec::new()),
        ));
    }
    for field in fields {
        entries.push(((*field).to_owned(), jsast::member(value.clone(), *field)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    wrap(jsast::object(entries))
}

/// How a `String` drop whose carrier differs from (or bypasses) its drop glue should lower.
pub(super) enum DroppedStringKind {
    NoOp,
    BaselineDealloc {
        src: ValueId,
        layout: super::adapters::StringLayout,
    },
}

impl<'a> JsCx<'a> {
    pub(super) fn only_used_by_empty_drop(&self, v: ValueId) -> bool {
        let mut drops = 0usize;
        for block in &self.func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Drop {
                        root: Root::Value(r),
                        path,
                        ..
                    } if path.is_empty() && *r == v => drops += 1,
                    kind if kind.uses().contains(&v) => return false,
                    _ => {}
                }
            }
            if block.term.uses().contains(&v) {
                return false;
            }
        }
        drops >= 1
    }

    pub(super) fn only_used_by_adjacent_drop(&self, v: ValueId) -> bool {
        let mut drops = 0usize;
        let mut adjacent = false;
        for block in &self.func.blocks {
            for (idx, op) in block.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::Drop {
                        root: Root::Value(r),
                        path,
                        ..
                    } if path.is_empty() && *r == v => {
                        drops += 1;
                        adjacent = idx > 0 && block.ops[idx - 1].result == Some(v);
                    }
                    kind if kind.uses().contains(&v) => return false,
                    _ => {}
                }
            }
            if block.term.uses().contains(&v) {
                return false;
            }
        }
        drops == 1 && adjacent
    }

    pub(super) fn dropped_string_kind(&self, v: ValueId) -> Option<DroppedStringKind> {
        if let Some(op) = self.def_of(v) {
            if let OpKind::Convert { v: src, from, to } = &op.kind {
                return match (*from, *to) {
                    (Carrier::HostString, Carrier::Baseline | Carrier::StringBuilder)
                    | (Carrier::StringBuilder, Carrier::Baseline) => Some(DroppedStringKind::NoOp),
                    (Carrier::Baseline, Carrier::StringBuilder) => {
                        let ty = self.ty_of(*src).ok()?;
                        let layout =
                            super::adapters::StringLayout::of(self.table, self.interns, ty).ok()?;
                        Some(DroppedStringKind::BaselineDealloc { src: *src, layout })
                    }
                    _ => None,
                };
            }
        }
        if self.func.values.get(v.0 as usize).map(|d| d.carrier) == Some(Carrier::HostString) {
            return Some(DroppedStringKind::NoOp);
        }
        None
    }
}
