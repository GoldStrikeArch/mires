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

use std::collections::BTreeMap;

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::repr::{CellId, Op, OpKind, RAssertMessage, RBlockId, RTerm, Root, ValueId};
use rcgjs_object::jir::semantic::{BinaryOp, CoroutineDesugaring, CoroutineKind, TypeId};
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
    pub strict_i32: bool,
    pub sentinel_result: Option<crate::plan::abi_flat::SentinelResultShape>,
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
        if let Some((enum_ty, none_variant, some_variant, none_expr)) =
            self.two_variant_niche_info(scrut_ty)
        {
            let (value, _) = self.scrutinee(scrut)?;
            let mut setup = Vec::new();
            let variants = value::enum_variants(self.cx.table, enum_ty);
            let (signed, bits) = self.cx.table.int_info(scrut_ty.ty).unwrap_or((false, 128));
            let none_d = variants
                .get(none_variant as usize)
                .map(|v| resign(signed, bits, v.logical_discr as u128));
            let some_d = variants
                .get(some_variant as usize)
                .map(|v| resign(signed, bits, v.logical_discr as u128));
            let mut seen_none = false;
            let mut seen_some = false;
            let mut used_value = false;
            let mut compiled_arms = Vec::with_capacity(arms.len());
            let mut exhaustive_tail: Option<Vec<Stmt>> = None;
            for (values, body) in arms {
                let mut has_none = false;
                let mut has_some = false;
                for v in values {
                    let d = self.cx.case_value(scrut_ty.ty, v);
                    if Some(d) == none_d {
                        has_none = true;
                    }
                    if Some(d) == some_d {
                        has_some = true;
                    }
                }
                let eff_none = has_none && !seen_none;
                let eff_some = has_some && !seen_some;
                if !eff_none && !eff_some {
                    continue;
                }
                let body = self.region(body)?;
                seen_none |= eff_none;
                seen_some |= eff_some;
                // `value === none_expr` and `value !== none_expr` partition every JS value, so
                // once both variants are covered this arm is unconditionally reached as the `else`
                // tail (eliminating redundant `else if (x !== null) ... else throw $ur()`).
                if seen_none && seen_some {
                    exhaustive_tail = Some(body);
                    break;
                }
                let op = if eff_none {
                    BinOp::StrictEq
                } else {
                    BinOp::StrictNe
                };
                used_value = true;
                compiled_arms.push((jsast::binary(op, value.clone(), none_expr.clone()), body));
            }
            if !used_value && !jsast::is_droppable(&value) {
                setup.push(jsast::expr_stmt(value));
            }
            let mut chain = match exhaustive_tail {
                Some(tail) => tail,
                None => self.region(default)?,
            };
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
            strict_i32: false,
            sentinel_result: None,
        })
    }

    /// Returns `(enum_ty, sentinel_variant, payload_variant, sentinel_expr)` when `scrut_ty`
    /// tests a 2-variant `NullNiche`, `SentinelNiche`, or virtual `SentinelResult` scrutinee.
    fn two_variant_niche_info(&self, scrut_ty: ScrutTy) -> Option<(TypeId, u32, u32, Expr)> {
        if let Some(shape) = scrut_ty.sentinel_result {
            let se = self.cx.sentinel_err_expr()?;
            return Some((shape.ty, shape.err_variant, shape.ok_variant, se));
        }
        let enum_ty = scrut_ty.tag_enum?;
        let repr = self.cx.table.enum_repr(self.cx.table.peeled(enum_ty))?;
        let (none_variant, some_variant, _) = repr.niche_variants()?;
        let none_expr = value::niche_none_expr(self.cx.table, repr)?;
        Some((enum_ty, none_variant, some_variant, none_expr))
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
            None if scrut.strict_i32 => jsast::num(f64::from((raw as u32) as i32)),
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
        if let Some((enum_ty, none_variant, some_variant, none_expr)) =
            self.two_variant_niche_info(scrut)
        {
            if !matches!(test, Test::IsTrue) {
                let variants = value::enum_variants(self.cx.table, enum_ty);
                let (signed, bits) = self.cx.table.int_info(ty).unwrap_or((false, 128));
                let none_d = variants
                    .get(none_variant as usize)
                    .map(|v| resign(signed, bits, v.logical_discr as u128));
                let some_d = variants
                    .get(some_variant as usize)
                    .map(|v| resign(signed, bits, v.logical_discr as u128));
                let bound = |v: u128| self.cx.case_value(ty, v);
                let matches_d = |d: Option<i128>| -> bool {
                    let Some(d) = d else {
                        return false;
                    };
                    match test {
                        Test::Eq(v) => d == bound(v),
                        Test::Lt(v) => d > bound(v),
                        Test::Le(v) => d >= bound(v),
                        Test::InRange { lo, len } => d >= bound(lo) && d <= bound(lo + len - 1),
                        Test::IsTrue => false,
                    }
                };
                let m_none = matches_d(none_d) ^ negated;
                let m_some = matches_d(some_d) ^ negated;
                return match (m_none, m_some) {
                    (true, true) => {
                        if jsast::is_droppable(&value) {
                            jsast::boolean(true)
                        } else {
                            jsast::seq(vec![value, jsast::boolean(true)])
                        }
                    }
                    (false, false) => {
                        if jsast::is_droppable(&value) {
                            jsast::boolean(false)
                        } else {
                            jsast::seq(vec![value, jsast::boolean(false)])
                        }
                    }
                    (true, false) => jsast::binary(BinOp::StrictEq, value, none_expr),
                    (false, true) => jsast::binary(BinOp::StrictNe, value, none_expr),
                };
            }
        }
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
                let (value, scrut) = if scrut.strict_i32 {
                    (
                        jsast::u32_wrap(value),
                        ScrutTy {
                            strict_i32: false,
                            ..scrut
                        },
                    )
                } else {
                    (value, scrut)
                };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::Le(v) => {
                let op = if negated { BinOp::Lt } else { BinOp::Ge };
                let (value, scrut) = if scrut.strict_i32 {
                    (
                        jsast::u32_wrap(value),
                        ScrutTy {
                            strict_i32: false,
                            ..scrut
                        },
                    )
                } else {
                    (value, scrut)
                };
                jsast::binary(op, value, self.case_literal(scrut, v))
            }
            Test::InRange { lo, len } => match self.cx.table.int_repr(ty) {
                IntRepr::Number => {
                    let (value, scrut) = if scrut.strict_i32 {
                        (
                            jsast::u32_wrap(value),
                            ScrutTy {
                                strict_i32: false,
                                ..scrut
                            },
                        )
                    } else {
                        (value, scrut)
                    };
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
            RScrut::Value {
                value,
                ty,
                tag_enum,
            } => {
                let sentinel_result = if tag_enum.is_some() && self.plan.tag_values.contains(&value)
                {
                    self.def_of(value).and_then(|op| match &op.kind {
                        OpKind::Discriminant(base) => {
                            self.virtual_sentinel(*base).map(|vs| vs.shape)
                        }
                        _ => None,
                    })
                } else {
                    None
                };
                ScrutTy {
                    ty,
                    tag_enum,
                    strict_i32: tag_enum.is_none() && self.is_strict_i32_u32(value),
                    sentinel_result,
                }
            }
            RScrut::Invoke(_) => ScrutTy {
                ty: self.usize_ty(),
                tag_enum: None,
                strict_i32: false,
                sentinel_result: None,
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
                    match (copy.dst, copy.src) {
                        (CopyOperand::Value(dst_v), CopyOperand::Value(src_v))
                            if self.virtual_sentinel(dst_v).is_some() =>
                        {
                            let super::VirtualSentinelRep::Slot { val_var, err_var } =
                                &self.virtual_sentinel(dst_v).expect("checked").rep
                            else {
                                return Err(decline(
                                    OpClass::Terminator,
                                    "virtual sentinel edge copy target has no slot",
                                ));
                            };
                            out.extend(self.emit_assign_virtual_sentinel(
                                val_var,
                                err_var.as_deref(),
                                src_v,
                            )?);
                            continue;
                        }
                        (CopyOperand::Temp(tmp_idx), CopyOperand::Value(src_v))
                            if self.virtual_sentinel(src_v).is_some() =>
                        {
                            let vs = self.virtual_sentinel(src_v).expect("checked");
                            let dst_val = self.copy_operand_name(copy.dst)?;
                            let dst_err =
                                vs.shape.err_field.map(|_| self.copy_temp_err_var(tmp_idx));
                            out.extend(self.emit_assign_virtual_sentinel(
                                &dst_val,
                                dst_err.as_deref(),
                                src_v,
                            )?);
                            continue;
                        }
                        (CopyOperand::Value(dst_v), CopyOperand::Temp(tmp_idx))
                            if self.virtual_sentinel(dst_v).is_some() =>
                        {
                            let super::VirtualSentinelRep::Slot { val_var, err_var } =
                                &self.virtual_sentinel(dst_v).expect("checked").rep
                            else {
                                return Err(decline(
                                    OpClass::Terminator,
                                    "virtual sentinel edge copy target has no slot",
                                ));
                            };
                            let src_val = jsast::id(self.copy_operand_name(copy.src)?);
                            out.push(jsast::assign_stmt(jsast::id(val_var.clone()), src_val));
                            if let Some(de) = err_var {
                                let src_err = jsast::id(self.copy_temp_err_var(tmp_idx));
                                out.push(jsast::assign_stmt(jsast::id(de.clone()), src_err));
                            }
                            continue;
                        }
                        _ => {}
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

    fn op_loc(&self, op: &Op) -> Option<Loc> {
        self.loc_of(op.origin)
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
                if let Some(shape) = self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.sentinel())
                {
                    let pending = self.prepare_sentinel_return(*v, shape)?;
                    self.pending_return
                        .replace(Some(super::PendingReturn::Sentinel(pending)));
                    return Ok(Vec::new());
                }
                if self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .is_some_and(|result| result.is_i64_pair())
                {
                    let (lo, hi) = self.pair_of(*v)?;
                    self.pending_return
                        .replace(Some(super::PendingReturn::I64Pair(lo, hi)));
                    return Ok(Vec::new());
                }
                // A `()` (or other zero-sized) result is a bare `return`, as v1's unit function.
                let pending = if self.table.is_zst(self.ty_of(*v)?) {
                    None
                } else {
                    Some(super::PendingReturn::Value(self.expr_of(*v)?))
                };
                self.pending_return.replace(pending);
                Ok(Vec::new())
            }
            RTerm::Return(None) => {
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
        match self.pending_return.borrow_mut().take() {
            Some(super::PendingReturn::Sentinel(pending)) => self.emit_sentinel_return(pending),
            Some(super::PendingReturn::I64Pair(lo, hi)) => {
                let spill = jsast::id(spill_name_of(self.spills, 1));
                Ok(jsast::ret(jsast::seq(vec![jsast::assign(spill, hi), lo])))
            }
            Some(super::PendingReturn::Value(value)) => {
                let tagged = self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.tagged());
                let recursive = self
                    .target_abi
                    .and_then(|sig| sig.result.as_ref())
                    .and_then(|result| result.recursive());
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
        if let Some(stmts) = self.try_sentinel_or_virtual_op_stmts(op)? {
            return Ok(stmts);
        }
        if let Some(stmts) = self.try_i64_op_stmts(op)? {
            return Ok(stmts);
        }
        if let Some(stmts) = self.try_physical_lane_op_stmts(op)? {
            return Ok(stmts);
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
        {
            return self.emit_call_like_op_stmts(op);
        }
        match op.result {
            Some(result) => self.emit_value_op_stmts(op, result),
            None => self.effect_stmts(op),
        }
    }

    fn try_sentinel_or_virtual_op_stmts(&self, op: &Op) -> Res<Option<Vec<Stmt>>> {
        if let OpKind::Drop {
            root: Root::Value(v),
            path,
            glue: rcgjs_object::jir::semantic::DropGlue::None,
        } = &op.kind
        {
            if path.is_empty() && self.virtual_sentinel(*v).is_some() {
                return Ok(Some(Vec::new()));
            }
        }
        if let Some(shape) = self.sentinel_call_shape(op) {
            return self.sentinel_call_stmts(op, shape).map(Some);
        }
        if let Some(result) = op.result {
            if let Some(vs) = self.virtual_sentinel(result) {
                match &vs.rep {
                    super::VirtualSentinelRep::InlineAlias(_)
                    | super::VirtualSentinelRep::InlineConstruct => return Ok(Some(Vec::new())),
                    super::VirtualSentinelRep::Slot { val_var, err_var } => match &op.kind {
                        OpKind::Move(src) | OpKind::Copy(src) => {
                            return self
                                .emit_assign_virtual_sentinel(val_var, err_var.as_deref(), *src)
                                .map(Some);
                        }
                        OpKind::Aggregate { .. }
                        | OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Variant {
                            ..
                        }) => {
                            return self
                                .emit_construct_virtual_sentinel(
                                    val_var,
                                    err_var.as_deref(),
                                    vs.shape,
                                    op,
                                )
                                .map(Some);
                        }
                        _ => {}
                    },
                }
            }
        }
        // A tagged result is a virtual canonical object backed by captured scalar locals. Its
        // admission proof permits only field/discriminant reads and Move/Copy aliases leading to
        // those reads or another prepared direct call, so cloning the description would add an
        // `Object.assign` with no observable copy semantics. Propagate the virtual description
        // instead; a true escape was refused by the ABI planner and never reaches this path.
        if let (Some(result), OpKind::Move(source) | OpKind::Copy(source)) = (op.result, &op.kind) {
            if let Some(value) = self.tagged_expr(*source) {
                self.bind_tagged_expr(result, value);
                return Ok(Some(Vec::new()));
            }
            let carried = self.carried_i64_overflow.borrow().get(source).cloned();
            if let Some(carried) = carried {
                self.carried_i64_overflow
                    .borrow_mut()
                    .insert(result, carried);
                return Ok(Some(Vec::new()));
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
                    0 => self
                        .bind_i64_pair_result(result, Vec::new(), (lo, hi))
                        .map(Some),
                    1 => self
                        .bind_scalar_result(result, Vec::new(), ovf, false)
                        .map(Some),
                    _ => Err(decline(
                        OpClass::Place,
                        format!("field {field} of an i64 overflow pair"),
                    )),
                };
            }
        }
        Ok(None)
    }

    fn try_i64_op_stmts(&self, op: &Op) -> Res<Option<Vec<Stmt>>> {
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
                    return Ok(Some(stmts));
                }
                let result_ty = self.ty_of(result)?;
                if self.is_i64_ty(result_ty) {
                    let pair = self.i64_binary_pair_stmts(&mut stmts, *bin, *ty, *l, *r)?;
                    return self.bind_i64_pair_result(result, stmts, pair).map(Some);
                }
                let expr = self.i64_binary_expr_stmts(&mut stmts, *bin, *ty, result_ty, *l, *r)?;
                return self
                    .bind_scalar_result(result, stmts, expr, false)
                    .map(Some);
            }
        }
        if let (Some(result), OpKind::Cast { kind, v, from, to }) = (op.result, &op.kind) {
            let result_ty = self.ty_of(result)?;
            if self.is_i64_ty(result_ty) {
                let mut stmts = Vec::new();
                let pair = self.i64_cast_pair_stmts(&mut stmts, *kind, *from, *to, *v)?;
                return self.bind_i64_pair_result(result, stmts, pair).map(Some);
            }
            if self.is_i64_ty(*from) {
                let mut stmts = Vec::new();
                let expr = self.i64_cast_expr_stmts(&mut stmts, *kind, *from, *to, *v)?;
                return self
                    .bind_scalar_result(result, stmts, expr, false)
                    .map(Some);
            }
        }
        Ok(None)
    }

    fn try_physical_lane_op_stmts(&self, op: &Op) -> Res<Option<Vec<Stmt>>> {
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
            return self.assign_physical_slice(result, value).map(Some);
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
                return self.assign_physical_slot(result, value).map(Some);
            }
            let value = match &op.kind {
                OpKind::Copy(source) | OpKind::Move(source) => self.expr_of(*source)?,
                _ => self.op_expr(op, result)?,
            };
            self.record_allocation_origin(result, &value);
            return self.assign_physical_slot(result, value).map(Some);
        }
        Ok(None)
    }

    fn emit_call_like_op_stmts(&self, op: &Op) -> Res<Vec<Stmt>> {
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
                    ));
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
        self.call_stmts(op)
    }

    fn emit_value_op_stmts(&self, op: &Op, result: ValueId) -> Res<Vec<Stmt>> {
        let result_ty = self.ty_of(result)?;
        if self.is_i64_ty(result_ty) {
            return self.emit_i64_value_op_stmts(op, result, result_ty);
        }
        // An inline value is spelled at its use site (`JsCx::expr_of`).
        if self.is_inline(result) {
            return Ok(Vec::new());
        }
        if let Some(stmts) = self.try_scalarize_soa_view(op, result)? {
            return Ok(stmts);
        }
        let expr = self.op_expr(op, result)?;
        self.record_allocation_origin(result, &expr);
        // A zero-sized value reads as `undefined` everywhere (`expr_of`), so its assignment has
        // nothing to say; a call's effects still run.
        if self.table.is_zst(self.ty_of(result)?)
            && !matches!(
                op.kind,
                OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. }
            )
        {
            return Ok(Vec::new());
        }
        let Some(name) = self.name_of(result) else {
            // A value nobody reads and the plan did not name: evaluated for its effects, which for
            // the ops of this module there are none of, but a call has.
            return Ok(match &op.kind {
                OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. } => {
                    vec![jsast::expr_stmt(expr)]
                }
                _ => Vec::new(),
            });
        };
        Ok(vec![jsast::assign_stmt(jsast::id(name), expr)])
    }

    fn emit_i64_value_op_stmts(
        &self,
        op: &Op,
        result: ValueId,
        result_ty: TypeId,
    ) -> Res<Vec<Stmt>> {
        let mut stmts = Vec::new();
        let pair = match &op.kind {
            OpKind::Const(tree) => match self.const_i64_pair(tree, result_ty) {
                Some(pair) => pair,
                None => {
                    let expr =
                        super::i64_pair::materialize(self, &mut stmts, self.op_expr(op, result)?);
                    super::i64_pair::unpack_bigint_exprs(expr)
                }
            },
            OpKind::Copy(v) | OpKind::Move(v) => self.pair_of(*v)?,
            OpKind::Unary { op: un, v, .. } => self.i64_unary_pair_stmts(&mut stmts, *un, *v)?,
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
                if is_union && active_field.is_some_and(|active| active != inner.field) {
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
                let expr =
                    super::i64_pair::materialize(self, &mut stmts, self.op_expr(op, result)?);
                super::i64_pair::unpack_bigint_exprs(expr)
            }
        };
        self.bind_i64_pair_result(result, stmts, pair)
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
        if !self.bstr_enabled() {
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
            "compare_bytes" => {
                args.len() == 3
                    && (self.byte_view(args[0]).is_some() || self.byte_view(args[1]).is_some())
            }
            "raw_eq" => {
                args.len() == 2
                    && (self.byte_view(args[0]).is_some() || self.byte_view(args[1]).is_some())
            }
            "transmute" | "transmute_unchecked" => {
                self.byte_view(result).is_some()
                    || (args.len() == 1
                        && self.byte_view(args[0]).is_some()
                        && self.ty_of(result).ok().is_some_and(|ty| {
                            super::byte_view_analysis::is_immutable_str_ref_pub(self.table, ty)
                        }))
            }
            "offset" | "arith_offset" | "slice_get_unchecked" => self.byte_view(result).is_some(),
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

    fn is_unsigned_u32_val(&self, v: ValueId) -> bool {
        self.ty_of(v).ok().is_some_and(|ty| {
            matches!(
                value::kind(self.table, self.table.peeled(ty)),
                Some(TypeKind::Integer {
                    signed: false,
                    bits: ..=32
                })
            )
        })
    }

    fn is_already_u32_expr(e: &Expr) -> bool {
        match e {
            Expr::Num(n) => n.is_finite() && *n >= 0.0 && *n <= 4294967295.0 && n.fract() == 0.0,
            Expr::Binary(BinOp::UShr, _, _) => true,
            _ => false,
        }
    }

    fn unsigned_add(a: Expr, b: Expr) -> Expr {
        if matches!(a, Expr::Num(n) if n == 0.0) {
            if Self::is_already_u32_expr(&b) {
                return b;
            }
            return jsast::binary(BinOp::UShr, b, jsast::num(0));
        }
        if matches!(b, Expr::Num(n) if n == 0.0) && Self::is_already_u32_expr(&a) {
            return a;
        }
        jsast::binary(BinOp::UShr, jsast::binary(BinOp::Add, a, b), jsast::num(0))
    }

    fn unsigned_add_val(&self, a: Expr, b_val: ValueId) -> Res<Expr> {
        let b = self.expr_of(b_val)?;
        if matches!(a, Expr::Num(n) if n == 0.0)
            && (self.is_unsigned_u32_val(b_val) || Self::is_already_u32_expr(&b))
        {
            return Ok(b);
        }
        Ok(Self::unsigned_add(a, b))
    }

    fn unsigned_sub(a: Expr, b: Expr) -> Expr {
        jsast::binary(BinOp::UShr, jsast::binary(BinOp::Sub, a, b), jsast::num(0))
    }

    pub(super) fn byte_view_base_expr(&self, bv: super::ByteView, v: ValueId) -> Res<Expr> {
        if let Some(base_id) = bv.base_str {
            let base = self.expr_of(base_id)?;
            if bv.base_is_string_obj {
                return Ok(jsast::member(base, "text"));
            }
            if self
                .byte_view(base_id)
                .is_some_and(|base_bv| base_bv.mode == super::ByteViewMode::VarSlice)
            {
                return Ok(Self::member_or_field(base, "buf"));
            }
            return Ok(base);
        }
        if bv.mode == super::ByteViewMode::FullSlice {
            return self.expr_of(v);
        }
        if bv.mode == super::ByteViewMode::VarSlice {
            return Ok(Self::member_or_field(self.expr_of(v)?, "buf"));
        }
        if let Some(slice_id) = bv.borrow_slice {
            if let Some(sbv) = self.byte_view(slice_id) {
                return self.byte_view_base_expr(sbv, slice_id);
            }
            return self.expr_of(slice_id);
        }
        Err(decline(
            OpClass::Place,
            "`js-str=bytes`: byte view lacks a base string".to_owned(),
        ))
    }

    pub(super) fn byte_view_off_expr(&self, v: ValueId) -> Res<Expr> {
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

    pub(super) fn byte_view_len_expr(&self, v: ValueId) -> Res<Expr> {
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

    pub(super) fn byte_view_to_str_expr(&self, src: ValueId) -> Res<Expr> {
        let Some(bv) = self.byte_view(src) else {
            return self.expr_of(src);
        };
        match bv.mode {
            super::ByteViewMode::FullSlice => self.expr_of(src),
            super::ByteViewMode::VarSlice => {
                let base = self.byte_view_base_expr(bv, src)?;
                let off = self.byte_view_off_expr(src)?;
                let len = self.byte_view_len_expr(src)?;
                let end = if matches!(off, Expr::Num(n) if n == 0.0) {
                    len
                } else {
                    Self::unsigned_add(off.clone(), len)
                };
                Ok(jsast::method_call(base, "slice", vec![off, end]))
            }
            super::ByteViewMode::ThinPtr => Err(decline(
                OpClass::Place,
                "`js-str=bytes`: thin pointer cannot convert directly to &str".to_owned(),
            )),
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
        if !self.bstr_enabled() {
            return Ok(None);
        }
        if let Some(expr) = self.byte_view_consumer_expr(op, result)? {
            return Ok(Some(expr));
        }
        let Some(res_bv) = self.byte_view(result) else {
            return Ok(None);
        };
        self.byte_view_producer_expr(op, res_bv, result)
    }

    fn byte_view_consumer_expr(&self, op: &Op, result: ValueId) -> Res<Option<Expr>> {
        use rcgjs_object::jir::repr::Proj;
        use rcgjs_object::jir::semantic::{BinaryOp, UnaryOp};
        match &op.kind {
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } if self.byte_view(*v).is_some() => Ok(Some(self.byte_view_len_expr(*v)?)),
            OpKind::Deref(ptr) if self.byte_view(*ptr).is_some() => {
                let bv = self.byte_view(*ptr).unwrap();
                Ok(Some(self.byte_view_deref_expr(bv, *ptr)?))
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
                Ok(Some(jsast::binary(
                    js_op,
                    self.byte_view_off_expr(*l)?,
                    self.byte_view_off_expr(*r)?,
                )))
            }
            OpKind::Intrinsic {
                name, types, args, ..
            } => self.byte_view_intrinsic_consumer_expr(result, *name, types, args),
            OpKind::Cast {
                kind:
                    rcgjs_object::jir::semantic::CastKind::Transmute
                    | rcgjs_object::jir::semantic::CastKind::PtrToPtr
                    | rcgjs_object::jir::semantic::CastKind::PointerCoercion,
                v: src,
                to,
                ..
            } if self.byte_view(*src).is_some()
                && super::byte_view_analysis::is_immutable_str_ref_pub(self.table, *to) =>
            {
                Ok(Some(self.byte_view_to_str_expr(*src)?))
            }
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                mutable: false,
            } if path.as_slice() == [Proj::Deref]
                && self.byte_view(*src).is_some()
                && self.ty_of(result).ok().is_some_and(|ty| {
                    super::byte_view_analysis::is_immutable_str_ref_pub(self.table, ty)
                }) =>
            {
                Ok(Some(self.byte_view_to_str_expr(*src)?))
            }
            _ => Ok(None),
        }
    }

    fn byte_view_intrinsic_consumer_expr(
        &self,
        result: ValueId,
        name: rcgjs_object::jir::interns::StrId,
        types: &[TypeId],
        args: &[ValueId],
    ) -> Res<Option<Expr>> {
        let name_s = self.interns.str(name).unwrap_or("");
        match name_s {
            "ptr_metadata" if args.first().is_some_and(|v| self.byte_view(*v).is_some()) => {
                Ok(Some(self.byte_view_len_expr(args[0])?))
            }
            "read_via_copy" | "volatile_load" | "unaligned_volatile_load"
                if args.first().is_some_and(|v| self.byte_view(*v).is_some()) =>
            {
                let bv = self.byte_view(args[0]).unwrap();
                Ok(Some(self.byte_view_deref_expr(bv, args[0])?))
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
                Ok(Some(masked))
            }
            "compare_bytes"
                if args.len() == 3
                    && (self.byte_view(args[0]).is_some() || self.byte_view(args[1]).is_some()) =>
            {
                let count = self.expr_of(args[2])?;
                Ok(Some(
                    match (self.byte_view(args[0]), self.byte_view(args[1])) {
                        (Some(bv0), Some(bv1)) => {
                            let b0 = self.byte_view_base_expr(bv0, args[0])?;
                            let o0 = self.byte_view_off_expr(args[0])?;
                            let b1 = self.byte_view_base_expr(bv1, args[1])?;
                            let o1 = self.byte_view_off_expr(args[1])?;
                            jsast::rt_call("bstr_cmp_range", vec![b0, o0, b1, o1, count])
                        }
                        (Some(bv0), None) => {
                            let b0 = self.byte_view_base_expr(bv0, args[0])?;
                            let o0 = self.byte_view_off_expr(args[0])?;
                            let p1 = self.expr_of(args[1])?;
                            jsast::rt_call("bstr_cmp_ptr", vec![b0, o0, p1, count])
                        }
                        (None, Some(bv1)) => {
                            let p0 = self.expr_of(args[0])?;
                            let b1 = self.byte_view_base_expr(bv1, args[1])?;
                            let o1 = self.byte_view_off_expr(args[1])?;
                            jsast::rt_call("ptr_cmp_bstr", vec![p0, b1, o1, count])
                        }
                        (None, None) => unreachable!(),
                    },
                ))
            }
            "raw_eq"
                if args.len() == 2
                    && (self.byte_view(args[0]).is_some() || self.byte_view(args[1]).is_some()) =>
            {
                let arr_len = types
                    .first()
                    .and_then(|t| value::kind(self.table, self.table.peeled(*t)))
                    .and_then(|k| match k {
                        TypeKind::Array { length, .. } => Some(*length),
                        _ => None,
                    });
                let Some(len) = arr_len else {
                    return Ok(None);
                };
                let n = jsast::num(len as f64);
                Ok(Some(
                    match (self.byte_view(args[0]), self.byte_view(args[1])) {
                        (Some(bv0), Some(bv1)) => {
                            let b0 = self.byte_view_base_expr(bv0, args[0])?;
                            let o0 = self.byte_view_off_expr(args[0])?;
                            let b1 = self.byte_view_base_expr(bv1, args[1])?;
                            let o1 = self.byte_view_off_expr(args[1])?;
                            jsast::binary(
                                BinOp::StrictEq,
                                jsast::rt_call("bstr_cmp_range", vec![b0, o0, b1, o1, n]),
                                jsast::num(0),
                            )
                        }
                        (Some(bv0), None) => {
                            let b0 = self.byte_view_base_expr(bv0, args[0])?;
                            let o0 = self.byte_view_off_expr(args[0])?;
                            let arr = self.expr_of(args[1])?;
                            jsast::rt_call("bstr_eq_arr", vec![b0, o0, arr, n])
                        }
                        (None, Some(bv1)) => {
                            let arr = self.expr_of(args[0])?;
                            let b1 = self.byte_view_base_expr(bv1, args[1])?;
                            let o1 = self.byte_view_off_expr(args[1])?;
                            jsast::rt_call("bstr_eq_arr", vec![b1, o1, arr, n])
                        }
                        (None, None) => unreachable!(),
                    },
                ))
            }
            "transmute" | "transmute_unchecked"
                if args.len() == 1
                    && self.byte_view(args[0]).is_some()
                    && self.ty_of(result).ok().is_some_and(|ty| {
                        super::byte_view_analysis::is_immutable_str_ref_pub(self.table, ty)
                    }) =>
            {
                Ok(Some(self.byte_view_to_str_expr(args[0])?))
            }
            _ => Ok(None),
        }
    }

    fn byte_view_const_expr(
        tree: &rcgjs_object::jir::semantic::ConstTree,
        mode: super::ByteViewMode,
    ) -> Option<Expr> {
        let s = super::byte_view_analysis::const_u8_slice_str(tree)?;
        let len = s.chars().count();
        Some(match mode {
            super::ByteViewMode::FullSlice => jsast::string(s),
            super::ByteViewMode::VarSlice => jsast::object_of(vec![
                ("buf", jsast::string(s)),
                ("off", jsast::num(0)),
                ("len", jsast::num(len as f64)),
            ]),
            super::ByteViewMode::ThinPtr => jsast::num(0),
        })
    }

    fn byte_view_producer_expr(
        &self,
        op: &Op,
        res_bv: super::ByteView,
        result: ValueId,
    ) -> Res<Option<Expr>> {
        let expr = match &op.kind {
            OpKind::Const(tree)
                if super::byte_view_analysis::const_u8_slice_str(tree).is_some() =>
            {
                Self::byte_view_const_expr(tree, res_bv.mode).unwrap()
            }
            OpKind::Cast {
                kind: rcgjs_object::jir::semantic::CastKind::PointerCoercion,
                v: src,
                ..
            } if self.byte_view(*src).is_none()
                && self.def_of(*src).is_some_and(|def| {
                    matches!(
                        &def.kind,
                        OpKind::Const(tree)
                            if super::byte_view_analysis::const_u8_slice_str(tree).is_some()
                    )
                }) =>
            {
                let OpKind::Const(tree) = &self.def_of(*src).unwrap().kind else {
                    unreachable!()
                };
                Self::byte_view_const_expr(tree, res_bv.mode).unwrap()
            }
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
            } => return self.byte_view_index_or_subslice_expr(*v, path, res_bv, result),
            OpKind::SliceView { ptr, len } => {
                self.byte_view_slice_view_expr(*ptr, *len, res_bv, result)?
            }
            OpKind::Binary {
                op: BinaryOp::Offset,
                l,
                r,
                ..
            } => self.unsigned_add_val(self.byte_view_off_expr(*l)?, *r)?,
            OpKind::Intrinsic { name, args, .. }
                if matches!(
                    self.interns.str(*name).unwrap_or(""),
                    "offset" | "arith_offset" | "slice_get_unchecked"
                ) && args.len() >= 2 =>
            {
                self.unsigned_add_val(self.byte_view_off_expr(args[0])?, args[1])?
            }
            _ => return self.byte_view_adt_or_field_expr(op, res_bv, result),
        };
        Ok(Some(expr))
    }

    fn byte_view_index_or_subslice_expr(
        &self,
        v: ValueId,
        path: &[rcgjs_object::jir::repr::Proj],
        res_bv: super::ByteView,
        result: ValueId,
    ) -> Res<Option<Expr>> {
        use rcgjs_object::jir::repr::Proj;
        let expr = match path {
            [Proj::Deref] => self.byte_view_cast_or_alias(v, res_bv, result)?,
            [Proj::Deref, Proj::Index(idx)] => {
                let base_off = self.byte_view_off_expr(v)?;
                self.unsigned_add_val(base_off, *idx)?
            }
            [Proj::Deref, Proj::ConstantIndex {
                offset, from_end, ..
            }] => {
                let base_off = self.byte_view_off_expr(v)?;
                let idx = if *from_end {
                    if let Some(arr_len) = self.ty_of(v).ok().and_then(|ty| {
                        super::byte_view_analysis::is_immutable_byte_array_ptr(self.table, ty)
                    }) {
                        jsast::num(arr_len.saturating_sub(*offset) as f64)
                    } else {
                        Self::unsigned_sub(self.byte_view_len_expr(v)?, jsast::num(*offset as f64))
                    }
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
                let base = if let Some(sbv) = self.byte_view(v) {
                    self.byte_view_base_expr(sbv, v)?
                } else {
                    self.byte_view_base_expr(res_bv, result)?
                };
                let base_off = self.byte_view_off_expr(v)?;
                let new_off = if *from == 0 {
                    base_off
                } else if matches!(base_off, Expr::Num(n) if n == 0.0) {
                    jsast::num(*from as f64)
                } else {
                    Self::unsigned_add(base_off, jsast::num(*from as f64))
                };
                let new_len = if *from_end {
                    let base_len = self.byte_view_len_expr(v)?;
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
        };
        Ok(Some(expr))
    }

    fn byte_view_slice_view_expr(
        &self,
        ptr: ValueId,
        len: ValueId,
        res_bv: super::ByteView,
        result: ValueId,
    ) -> Res<Expr> {
        let base = if let Some(pbv) = self.byte_view(ptr) {
            self.byte_view_base_expr(pbv, ptr)?
        } else {
            self.byte_view_base_expr(res_bv, result)?
        };
        if res_bv.mode == super::ByteViewMode::FullSlice {
            Ok(base)
        } else {
            let off = self.byte_view_off_expr(ptr)?;
            let len_expr = self.expr_of(len)?;
            Ok(jsast::object_of(vec![
                ("buf", base),
                ("off", off),
                ("len", len_expr),
            ]))
        }
    }

    fn byte_view_adt_or_field_expr(
        &self,
        op: &Op,
        res_bv: super::ByteView,
        result: ValueId,
    ) -> Res<Option<Expr>> {
        let expr = match &op.kind {
            OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Variant { variant, .. })
                if self.ty_of(result).ok().is_some_and(|ty| {
                    super::byte_view_analysis::null_niche_byte_payload(self.table, ty)
                        .is_some_and(|(none_v, _, _, _, _)| *variant == none_v)
                }) =>
            {
                jsast::null()
            }
            OpKind::Aggregate {
                ty,
                variant,
                fields,
                ..
            } if super::byte_view_analysis::null_niche_byte_payload(self.table, *ty).is_some() => {
                let (none_v, some_v, fidx, _, _) =
                    super::byte_view_analysis::null_niche_byte_payload(self.table, *ty).unwrap();
                if *variant == none_v {
                    jsast::null()
                } else if *variant == some_v {
                    let Some(&src) = fields.get(fidx as usize) else {
                        return Ok(None);
                    };
                    self.byte_view_cast_or_alias(src, res_bv, result)?
                } else {
                    return Ok(None);
                }
            }
            OpKind::Field {
                base,
                variant,
                field,
            } if self.ty_of(*base).ok().is_some_and(|bty| {
                super::byte_view_analysis::null_niche_byte_payload(self.table, bty).is_some_and(
                    |(_, some_v, fidx, _, _)| *variant == Some(some_v) && *field == fidx,
                )
            }) =>
            {
                self.byte_view_cast_or_alias(*base, res_bv, result)?
            }
            OpKind::Aggregate { ty, fields, .. }
                if fields.len() == 2
                    && matches!(
                        self.table.get(*ty).map(|d| &d.kind),
                        Some(TypeKind::RawPointer { .. })
                    ) =>
            {
                self.byte_view_slice_view_expr(fields[0], fields[1], res_bv, result)?
            }
            OpKind::Aggregate {
                ty,
                active_field,
                fields,
                ..
            } if self.table.get(*ty).and_then(|d| d.js.transparent).is_some() => {
                let d = self.table.get(*ty).unwrap();
                let inner = d.js.transparent.unwrap();
                let is_union = matches!(
                    d.kind,
                    TypeKind::Adt {
                        form: rcgjs_object::jir::types::NominalForm::Union,
                        ..
                    }
                );
                if is_union && active_field.is_some_and(|a| a != inner.field) {
                    return Ok(None);
                }
                let idx = if is_union { 0 } else { inner.field as usize };
                let Some(&src) = fields.get(idx) else {
                    return Ok(None);
                };
                self.byte_view_cast_or_alias(src, res_bv, result)?
            }
            OpKind::Field { base, field, .. }
                if self
                    .ty_of(*base)
                    .ok()
                    .and_then(|bty| self.table.get(bty))
                    .and_then(|d| d.js.transparent)
                    .is_some_and(|inner| inner.field == *field) =>
            {
                self.byte_view_cast_or_alias(*base, res_bv, result)?
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
                    let base = if let Some(sbv) = self.byte_view(src) {
                        self.byte_view_base_expr(sbv, src)?
                    } else if res_bv.base_str.is_some() {
                        self.byte_view_base_expr(res_bv, result)?
                    } else {
                        self.expr_of(src)?
                    };
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
            return Ok(jsast::undefined());
        }
        if let Some(expr) = self.byte_view_op_expr(op, result)? {
            return Ok(expr);
        }
        match &op.kind {
            OpKind::Const(tree) => {
                if let Some(expr) = self.soa_const_expr(tree, result_ty)? {
                    return Ok(expr);
                }
                self.const_expr(tree, result_ty)
            }
            OpKind::Copy(v) => self.emit_copy_op(*v, result),
            OpKind::Move(v) => self.expr_of(*v),
            OpKind::Aggregate {
                ty,
                variant,
                active_field,
                fields,
            } => self.emit_aggregate_op(*ty, *variant, *active_field, fields),
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
            } => self.emit_field_op(*base, *variant, *field, result, result_ty),
            OpKind::Discriminant(base) => self.emit_discriminant_op(*base, result, result_ty),
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
            } => {
                if self.is_direct_ref_copy_cast(result) {
                    if let (Root::Value(v), [rcgjs_object::jir::repr::Proj::Deref]) =
                        (root, path.as_slice())
                    {
                        return self.expr_of(*v);
                    }
                }
                self.raw_view(root, path, *mutable)
            }
            OpKind::SliceView { ptr: pointer, len } => {
                self.emit_slice_view_op(*pointer, *len, result_ty)
            }
            OpKind::Deref(pointer) => self.emit_deref_op(*pointer, result),
            OpKind::Binary { op: bin, l, r, ty } => {
                self.emit_binary_op(*bin, *l, *r, *ty, result, result_ty)
            }
            // `js-str=bytes`: closed byte-string views are handled by `byte_view_op_expr` above.
            OpKind::Unary { op: un, v, ty } => {
                self.unary(*un, *ty, self.expr_of(*v)?, self.is_int32_only(result))
            }
            OpKind::Cast { kind, v, from, to } => {
                if self.is_direct_ref_copy_cast(result) {
                    self.expr_of(*v)
                } else {
                    self.cast_with_int32_only(
                        *kind,
                        *from,
                        *to,
                        self.expr_of(*v)?,
                        self.is_int32_only(result),
                    )
                }
            }
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
            OpKind::Convert { v, from, to } => {
                if from == to {
                    self.expr_of(*v)
                } else {
                    Err(decline(
                        OpClass::Rvalue,
                        format!("unsupported carrier conversion {from:?} -> {to:?}"),
                    ))
                }
            }
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

    fn emit_copy_op(&self, v: ValueId, result: ValueId) -> Res<Expr> {
        let value = self.expr_of(v)?;
        // A carried overflow pair is a scalar (`rvalue::carried_overflow_pairs`), and nothing
        // mutates one: its copy is the same value.
        if self.carried_pairs.contains_key(&result) {
            return Ok(value);
        }
        let ty = self.ty_of(v)?;
        // A committed carrier is cloned at its own shape: the baseline field-wise clone reads a
        // structure the value no longer has (Phase 4).
        match self.func.values[v.0 as usize].carrier {
            rcgjs_object::jir::repr::Carrier::HostString
            | rcgjs_object::jir::repr::Carrier::NativeMap => Ok(value),
            _ => Ok(if self.table.needs_clone(ty) {
                value::clone_expr_with(self.table, self.interns, ty, value, self.nested_clone)
            } else {
                value
            }),
        }
    }

    fn emit_aggregate_op(
        &self,
        ty: TypeId,
        variant: u32,
        active_field: Option<u32>,
        fields: &[ValueId],
    ) -> Res<Expr> {
        let data_ty = fields.first().map(|field| self.ty_of(*field)).transpose()?;
        let values = fields
            .iter()
            .enumerate()
            .map(|(fi, field)| {
                let expr = self.expr_of(*field)?;
                if variant == 0
                    && active_field.is_none()
                    && self.is_smi_u32_field(ty, fi as u32)
                    && !self.is_strict_i32_u32(*field)
                {
                    Ok(JsCx::smi_i32_store(expr))
                } else {
                    Ok(expr)
                }
            })
            .collect::<Res<Vec<_>>>()?;
        self.aggregate(ty, variant, active_field, values, data_ty)
    }

    fn emit_field_op(
        &self,
        base: ValueId,
        variant: Option<u32>,
        field: u32,
        result: ValueId,
        result_ty: TypeId,
    ) -> Res<Expr> {
        let table = self.table;
        if let Some(&pair) = self.carried_pairs.get(&base) {
            return self.carried_pair_field(pair, base, field);
        }
        if let Some(vs) = self.virtual_sentinel(base) {
            if table.is_zst(result_ty) {
                return Ok(jsast::undefined());
            }
            let resolved = self.resolve_virtual_sentinel(base);
            let super::VirtualSentinelRep::Slot { val_var, err_var } =
                &self.virtual_sentinel(resolved).expect("checked").rep
            else {
                return Err(decline(
                    OpClass::Place,
                    "virtual sentinel field read has no backing slot",
                ));
            };
            let vi = variant.unwrap_or(0);
            if vi == vs.shape.ok_variant && vs.shape.ok_field.is_some_and(|(f, _)| f == field) {
                return Ok(jsast::id(val_var.clone()));
            }
            if vi == vs.shape.err_variant && vs.shape.err_field.is_some_and(|(f, _)| f == field) {
                let Some(ev) = err_var else {
                    return Err(decline(
                        OpClass::Place,
                        "virtual sentinel err payload slot missing",
                    ));
                };
                return Ok(jsast::id(ev.clone()));
            }
            return Err(decline(
                OpClass::Place,
                format!("field {field} on variant {vi} of virtual sentinel result"),
            ));
        }
        let base_ty = self.ty_of(base)?;
        if table.is_zst(result_ty)
            && matches!(
                table.enum_repr(table.peeled(base_ty)),
                Some(
                    EnumRepr::IntTag { fieldless: true }
                        | EnumRepr::NullNiche { .. }
                        | EnumRepr::SentinelNiche { .. }
                )
            )
        {
            return Ok(jsast::undefined());
        }
        let Some(descriptor) = table.field(base_ty, variant, field) else {
            return Err(decline(
                OpClass::Place,
                format!("field {field} of a type with no such field"),
            ));
        };
        let expr = value::field_expr(self.interns, self.expr_of(base)?, descriptor.key);
        if variant.is_none()
            && self.is_smi_u32_field(base_ty, field)
            && !self.is_strict_i32_u32(result)
        {
            Ok(jsast::u32_wrap(expr))
        } else {
            Ok(expr)
        }
    }

    fn emit_discriminant_op(&self, base: ValueId, result: ValueId, result_ty: TypeId) -> Res<Expr> {
        let table = self.table;
        if let Some(vs) = self.virtual_sentinel(base) {
            let resolved = self.resolve_virtual_sentinel(base);
            let super::VirtualSentinelRep::Slot { val_var, .. } =
                &self.virtual_sentinel(resolved).expect("checked").rep
            else {
                return Err(decline(
                    OpClass::Rvalue,
                    "virtual sentinel discriminant read has no backing slot",
                ));
            };
            if self.plan.tag_values.contains(&result) {
                return Ok(jsast::id(val_var.clone()));
            }
            let Some(se) = self.sentinel_err_expr() else {
                return Err(decline(
                    OpClass::Rvalue,
                    "virtual sentinel discriminant missing sentinel binding",
                ));
            };
            let discr_repr = table.int_repr(result_ty);
            return Ok(jsast::cond(
                jsast::binary(BinOp::StrictEq, jsast::id(val_var.clone()), se),
                value::discriminant_literal(table, vs.shape.ty, vs.shape.err_variant, discr_repr),
                value::discriminant_literal(table, vs.shape.ty, vs.shape.ok_variant, discr_repr),
            ));
        }
        let base_ty = self.ty_of(base)?;
        let as_tag = self.plan.tag_values.contains(&result);
        self.discriminant(base_ty, self.expr_of(base)?, result_ty, as_tag)
    }

    fn emit_slice_view_op(&self, pointer: ValueId, len: ValueId, result_ty: TypeId) -> Res<Expr> {
        let table = self.table;
        let pointer_ty = self.ty_of(pointer)?;
        let data = self.expr_of(pointer)?;
        let len = self.expr_of(len)?;
        let Some(pointee) = value::pointee_of(table, result_ty) else {
            return Err(decline(OpClass::Rvalue, "SliceView at a non-pointer type"));
        };
        // A reference to an array: the array object is the buffer when the slice's element type
        // matches the array's element type. The test is on the pointee as well as the form,
        // because a thin form says only that the pointer is its pointee's reference form.
        // `from_raw_parts` takes its data half as any thin pointer (`*const ()`,
        // `*mut MaybeUninit<[T; N]>`, etc.); `build_fat` converts it from `pointer_ty` to the
        // slice element pointer, as v1's `aggregate_raw_ptr` does.
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

    fn emit_deref_op(&self, pointer: ValueId, result: ValueId) -> Res<Expr> {
        if self.unboxed_map_key_cell(pointer).is_some() {
            return self.expr_of(pointer);
        }
        let walk = self.pointee_place(pointer)?;
        let expr = self.read_place(&walk.place);
        if (walk.smi_u32 || self.is_smi_u32_ptr(pointer)) && !self.is_strict_i32_u32(result) {
            Ok(jsast::u32_wrap(expr))
        } else {
            Ok(expr)
        }
    }

    fn emit_binary_op(
        &self,
        bin: BinaryOp,
        l: ValueId,
        r: ValueId,
        ty: TypeId,
        result: ValueId,
        result_ty: TypeId,
    ) -> Res<Expr> {
        if let Some(pair) = self.carried_pairs.get(&result) {
            let (lhs, rhs) = (self.expr_of(l)?, self.expr_of(r)?);
            return self.carried_pair_value(pair.carrier, bin, ty, lhs, rhs);
        }
        let rhs_ty = self.ty_of(r)?;
        let (lhs, rhs) = if matches!(bin, BinaryOp::Eq | BinaryOp::Ne)
            && self.table.int_repr(ty) == rcgjs_object::jir::types::IntRepr::Number
            && self.table.int_info(ty) == Some((false, 32))
        {
            let l_strict = self.is_strict_i32_u32(l);
            let r_strict = self.is_strict_i32_u32(r);
            let lhs = if r_strict && !l_strict {
                if let Some(bits) = self.const_u32_bits(l) {
                    jsast::num(f64::from((bits as u32) as i32))
                } else {
                    self.expr_of(l)?
                }
            } else {
                self.expr_of(l)?
            };
            let rhs = if l_strict && !r_strict {
                if let Some(bits) = self.const_u32_bits(r) {
                    jsast::num(f64::from((bits as u32) as i32))
                } else {
                    self.expr_of(r)?
                }
            } else {
                self.expr_of(r)?
            };
            (lhs, rhs)
        } else {
            (self.expr_of(l)?, self.expr_of(r)?)
        };
        self.binary(super::rvalue::BinaryArgs {
            op: bin,
            ty,
            rhs_ty,
            result_ty,
            lhs,
            rhs,
            int32_only: self.is_int32_only(result),
        })
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
                if table.is_zst(descriptor.ty)
                    && matches!(
                        table.enum_repr(table.peeled(base_ty)),
                        Some(
                            EnumRepr::IntTag { fieldless: true }
                                | EnumRepr::NullNiche { .. }
                                | EnumRepr::SentinelNiche { .. }
                        )
                    )
                {
                    let val = self.expr_of(*v)?;
                    return Ok(if jsast::is_droppable(&val) {
                        Vec::new()
                    } else {
                        vec![jsast::expr_stmt(val)]
                    });
                }
                let value = self.expr_of(*v)?;
                if table.is_zst(descriptor.ty) {
                    // Nothing to store; the value is a JS value with no effect to keep.
                    return Ok(Vec::new());
                }
                let value = if variant.is_none()
                    && self.is_smi_u32_field(base_ty, *field)
                    && !self.is_strict_i32_u32(*v)
                {
                    JsCx::smi_i32_store(value)
                } else {
                    value
                };
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
                if self.stores_back(*pointer, *v)
                    && (!self.is_soa_ptr(*pointer) || !self.value_has_set_field(*v))
                {
                    return Ok(Vec::new());
                }
                let walk = self.pointee_place(*pointer)?;
                let value = self.expr_of(*v)?;
                let value = if (walk.smi_u32 || self.is_smi_u32_ptr(*pointer))
                    && !self.is_strict_i32_u32(*v)
                {
                    JsCx::smi_i32_store(value)
                } else {
                    value
                };
                self.write_place(walk.place, walk.ty, value)
            }
            OpKind::Assert {
                cond,
                expected,
                msg,
                track_caller,
            } => self.assert_stmts(*cond, *expected, msg, *track_caller),
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

    /// Lowers a direct call to a `SentinelResult` callee: captures into a virtual `(val, err)`
    /// pair on the fast unboxed path, materializes canonical `{ TAG, ... }` at escaping call
    /// sites, or clears `$r1` when the result is discarded.
    fn sentinel_call_stmts(
        &self,
        op: &Op,
        shape: crate::plan::abi_flat::SentinelResultShape,
    ) -> Res<Vec<Stmt>> {
        let (mut out, call_expr) = match self.lower_call_like(op)? {
            Lowered::Done(stmts) => return Ok(stmts),
            Lowered::Value(prelude, value) => (prelude, value),
            _ => {
                return Err(decline(
                    OpClass::Call,
                    "sentinel result call did not produce a JS call expression",
                ));
            }
        };
        let Some(se) = self.sentinel_err_expr() else {
            return Err(decline(
                OpClass::Call,
                "sentinel result call missing sentinel error binding",
            ));
        };
        if let Some(result) = op.result {
            if let Some(vs) = self.virtual_sentinel(result) {
                let super::VirtualSentinelRep::Slot { val_var, err_var } = &vs.rep else {
                    return Err(decline(
                        OpClass::Call,
                        "virtual sentinel call result has no backing slot",
                    ));
                };
                out.push(jsast::assign_stmt(jsast::id(val_var.clone()), call_expr));
                if shape.err_field.is_some() {
                    let spill = jsast::id(spill_name_of(self.spills, 1));
                    let mut err_stmts = Vec::with_capacity(2);
                    if let Some(ev) = err_var {
                        err_stmts.push(jsast::assign_stmt(jsast::id(ev.clone()), spill.clone()));
                    }
                    err_stmts.push(jsast::assign_stmt(spill, jsast::undefined()));
                    out.push(jsast::if_(
                        jsast::binary(BinOp::StrictEq, jsast::id(val_var.clone()), se),
                        err_stmts,
                    ));
                }
                return Ok(out);
            }
            let dst = if self.is_inline(result) {
                let tmp_out = self.alloc_temp();
                self.cache_expr(result, jsast::id(tmp_out.clone()));
                Some(jsast::id(tmp_out))
            } else {
                self.name_of(result).map(jsast::id)
            };
            if let Some(dst) = dst {
                let (bind, raw_id) = self.temp(call_expr);
                out.push(bind);
                let ok_obj = self.build_canonical_sentinel_variant(
                    shape,
                    shape.ok_variant,
                    shape.ok_field.map(|_| raw_id.clone()),
                )?;
                let spill = jsast::id(spill_name_of(self.spills, 1));
                let err_obj = self.build_canonical_sentinel_variant(
                    shape,
                    shape.err_variant,
                    shape.err_field.map(|_| spill.clone()),
                )?;
                let mut then_stmts = vec![jsast::assign_stmt(dst.clone(), err_obj)];
                if shape.err_field.is_some() {
                    then_stmts.push(jsast::assign_stmt(spill, jsast::undefined()));
                }
                let else_stmts = vec![jsast::assign_stmt(dst, ok_obj)];
                out.push(jsast::if_else(
                    jsast::binary(BinOp::StrictEq, raw_id, se),
                    then_stmts,
                    else_stmts,
                ));
                return Ok(out);
            }
        }
        // Discarded result: still clear `$r1` on `Err` so GC does not retain the error value.
        if shape.err_field.is_some() {
            let spill = jsast::id(spill_name_of(self.spills, 1));
            out.push(jsast::if_(
                jsast::binary(BinOp::StrictEq, call_expr, se),
                vec![jsast::assign_stmt(spill, jsast::undefined())],
            ));
        } else {
            out.push(jsast::expr_stmt(call_expr));
        }
        Ok(out)
    }

    /// Rebuilds the canonical `{ TAG, ... }` enum object for one variant of `SentinelResultShape`
    /// at an escaping call site.
    fn build_canonical_sentinel_variant(
        &self,
        shape: crate::plan::abi_flat::SentinelResultShape,
        variant: u32,
        payload: Option<Expr>,
    ) -> Res<Expr> {
        let variants = value::enum_variants(self.table, shape.ty);
        let Some(vdesc) = variants.get(variant as usize) else {
            return Err(decline(
                OpClass::Call,
                "sentinel result variant out of range",
            ));
        };
        let active_field = if variant == shape.ok_variant {
            shape.ok_field.map(|(f, _)| f)
        } else {
            shape.err_field.map(|(f, _)| f)
        };
        let mut entries = Vec::with_capacity(vdesc.fields.len());
        let mut payload = payload;
        for (fi, field) in vdesc.fields.iter().enumerate() {
            if matches!(field.key, rcgjs_object::jir::types::FieldKey::Transparent) {
                continue;
            }
            let key = match field.key {
                rcgjs_object::jir::types::FieldKey::Name(id) => {
                    self.interns.str(id).unwrap_or("").to_owned()
                }
                rcgjs_object::jir::types::FieldKey::Index(i) => i.to_string(),
                rcgjs_object::jir::types::FieldKey::Transparent => unreachable!(),
            };
            let expr = if active_field == Some(fi as u32) {
                payload.take().unwrap_or_else(jsast::undefined)
            } else {
                jsast::undefined()
            };
            entries.push((key, expr));
        }
        value::enum_value(self.table, self.interns, shape.ty, variant, entries).ok_or_else(|| {
            decline(
                OpClass::Call,
                "sentinel result canonical materialization failed",
            )
        })
    }

    /// Extracts `(is_ok, payload_expr)` from a virtual `Aggregate` or `Const(Variant)` op.
    fn extract_sentinel_construct(
        &self,
        shape: crate::plan::abi_flat::SentinelResultShape,
        op: &Op,
    ) -> Res<(bool, Option<Expr>)> {
        match &op.kind {
            OpKind::Aggregate {
                variant, fields, ..
            } => {
                if *variant == shape.ok_variant {
                    let expr = match shape.ok_field {
                        Some((fi, _)) => Some(self.expr_of(fields[fi as usize])?),
                        None => None,
                    };
                    Ok((true, expr))
                } else if *variant == shape.err_variant {
                    let expr = match shape.err_field {
                        Some((fi, _)) => Some(self.expr_of(fields[fi as usize])?),
                        None => None,
                    };
                    Ok((false, expr))
                } else {
                    Err(decline(
                        OpClass::Rvalue,
                        "sentinel result aggregate variant out of range",
                    ))
                }
            }
            OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Variant { variant, fields }) => {
                if *variant == shape.ok_variant {
                    let expr = match shape.ok_field {
                        Some((fi, fty)) => Some(self.const_expr(&fields[fi as usize], fty)?),
                        None => None,
                    };
                    Ok((true, expr))
                } else if *variant == shape.err_variant {
                    let expr = match shape.err_field {
                        Some((fi, fty)) => Some(self.const_expr(&fields[fi as usize], fty)?),
                        None => None,
                    };
                    Ok((false, expr))
                } else {
                    Err(decline(
                        OpClass::Constant,
                        "sentinel result constant variant out of range",
                    ))
                }
            }
            _ => Err(decline(
                OpClass::Rvalue,
                "expected Aggregate or Const(Variant) for virtual sentinel construct",
            )),
        }
    }

    /// Writes a constructed `Ok(v)` / `Err(e)` directly into virtual sentinel slot `(dst_val, dst_err)`.
    fn emit_construct_virtual_sentinel(
        &self,
        dst_val: &str,
        dst_err: Option<&str>,
        shape: crate::plan::abi_flat::SentinelResultShape,
        op: &Op,
    ) -> Res<Vec<Stmt>> {
        let (is_ok, payload) = self.extract_sentinel_construct(shape, op)?;
        if is_ok {
            Ok(vec![jsast::assign_stmt(
                jsast::id(dst_val),
                payload.unwrap_or_else(jsast::undefined),
            )])
        } else {
            let Some(se) = self.sentinel_err_expr() else {
                return Err(decline(
                    OpClass::Rvalue,
                    "virtual sentinel construct missing sentinel error binding",
                ));
            };
            let mut stmts = vec![jsast::assign_stmt(jsast::id(dst_val), se)];
            if let (Some(de), Some(err_expr)) = (dst_err, payload) {
                stmts.push(jsast::assign_stmt(jsast::id(de), err_expr));
            }
            Ok(stmts)
        }
    }

    /// Copies a virtual sentinel value `src` into `(dst_val, dst_err)`.
    fn emit_assign_virtual_sentinel(
        &self,
        dst_val: &str,
        dst_err: Option<&str>,
        src: ValueId,
    ) -> Res<Vec<Stmt>> {
        let resolved = self.resolve_virtual_sentinel(src);
        let Some(vs) = self.virtual_sentinel(resolved) else {
            return Err(decline(
                OpClass::Terminator,
                "virtual sentinel assignment source is not virtual",
            ));
        };
        match &vs.rep {
            super::VirtualSentinelRep::InlineConstruct => {
                let Some(def) = self.def_of(resolved) else {
                    return Err(decline(
                        OpClass::Rvalue,
                        "inline virtual sentinel construct has no defining op",
                    ));
                };
                self.emit_construct_virtual_sentinel(dst_val, dst_err, vs.shape, def)
            }
            super::VirtualSentinelRep::Slot { val_var, err_var } => {
                let mut stmts = Vec::with_capacity(2);
                if dst_val != val_var {
                    stmts.push(jsast::assign_stmt(
                        jsast::id(dst_val),
                        jsast::id(val_var.clone()),
                    ));
                }
                if let (Some(de), Some(se)) = (dst_err, err_var.as_deref()) {
                    if de != se {
                        stmts.push(jsast::assign_stmt(jsast::id(de), jsast::id(se)));
                    }
                }
                Ok(stmts)
            }
            super::VirtualSentinelRep::InlineAlias(_) => unreachable!("resolved above"),
        }
    }

    /// Parks the pending return payload for a callee returning `SentinelResultShape`.
    fn prepare_sentinel_return(
        &self,
        v: ValueId,
        shape: crate::plan::abi_flat::SentinelResultShape,
    ) -> Res<super::PendingSentinelReturn> {
        if let Some(vs) = self.virtual_sentinel(v) {
            if vs.shape == shape {
                let resolved = self.resolve_virtual_sentinel(v);
                let vs = self.virtual_sentinel(resolved).expect("checked");
                return match &vs.rep {
                    super::VirtualSentinelRep::InlineConstruct => {
                        let Some(def) = self.def_of(resolved) else {
                            return Err(decline(
                                OpClass::Terminator,
                                "inline sentinel return construct has no defining op",
                            ));
                        };
                        let (is_ok, payload) = self.extract_sentinel_construct(shape, def)?;
                        if is_ok {
                            Ok(super::PendingSentinelReturn::Ok(
                                payload.unwrap_or_else(jsast::undefined),
                            ))
                        } else {
                            Ok(super::PendingSentinelReturn::Err(payload))
                        }
                    }
                    super::VirtualSentinelRep::Slot { val_var, err_var } => {
                        Ok(super::PendingSentinelReturn::Virtual {
                            val: jsast::id(val_var.clone()),
                            err: err_var.as_ref().map(|e| jsast::id(e.clone())),
                        })
                    }
                    super::VirtualSentinelRep::InlineAlias(_) => unreachable!("resolved above"),
                };
            }
        }
        Ok(super::PendingSentinelReturn::Canonical {
            value: self.expr_of(v)?,
            shape,
        })
    }

    /// Emits `return` for a callee returning `SentinelResultShape`.
    fn emit_sentinel_return(&self, pending: super::PendingSentinelReturn) -> Res<Stmt> {
        let Some(se) = self.sentinel_err_expr() else {
            return Err(decline(
                OpClass::Terminator,
                "sentinel result return missing sentinel error binding",
            ));
        };
        match pending {
            super::PendingSentinelReturn::Ok(expr) => {
                if matches!(expr, Expr::Undefined) {
                    Ok(jsast::ret_void())
                } else {
                    Ok(jsast::ret(expr))
                }
            }
            super::PendingSentinelReturn::Err(Some(err_expr)) => {
                let spill = jsast::id(spill_name_of(self.spills, 1));
                Ok(jsast::ret(jsast::seq(vec![
                    jsast::assign(spill, err_expr),
                    se,
                ])))
            }
            super::PendingSentinelReturn::Err(None) => Ok(jsast::ret(se)),
            super::PendingSentinelReturn::Virtual { val, err: None } => Ok(jsast::ret(val)),
            super::PendingSentinelReturn::Virtual {
                val,
                err: Some(err_expr),
            } => {
                let spill = jsast::id(spill_name_of(self.spills, 1));
                Ok(jsast::ret(jsast::cond(
                    jsast::binary(BinOp::StrictEq, val.clone(), se.clone()),
                    jsast::seq(vec![jsast::assign(spill, err_expr), se]),
                    val,
                )))
            }
            super::PendingSentinelReturn::Canonical { value, shape } => {
                let peeled = self.table.peeled(shape.ty);
                let Some(repr) = self.table.enum_repr(peeled) else {
                    return Err(decline(
                        OpClass::Terminator,
                        "canonical sentinel return at non-enum type",
                    ));
                };
                let Some(err_tag) =
                    value::variant_tag(self.table, self.interns, shape.ty, shape.err_variant)
                else {
                    return Err(decline(
                        OpClass::Terminator,
                        "canonical sentinel return err variant tag missing",
                    ));
                };
                let ok_key = shape
                    .ok_field
                    .and_then(|(f, _)| self.table.field(shape.ty, Some(shape.ok_variant), f))
                    .map(|fd| fd.key);
                let err_key = shape
                    .err_field
                    .and_then(|(f, _)| self.table.field(shape.ty, Some(shape.err_variant), f))
                    .map(|fd| fd.key);
                let mut read_names = vec![value::TAG.to_owned()];
                for key in ok_key.into_iter().chain(err_key) {
                    if let rcgjs_object::jir::types::FieldKey::Name(id) = key {
                        if let Some(s) = self.interns.str(id) {
                            read_names.push(s.to_owned());
                        }
                    }
                }
                let reads: Vec<&str> = read_names.iter().map(String::as_str).collect();
                let (setup, v) = self.reread(value, &reads);
                let is_err =
                    jsast::binary(BinOp::StrictEq, value::tag_expr(repr, v.clone()), err_tag);
                let err_branch = match err_key {
                    Some(key) => {
                        let spill = jsast::id(spill_name_of(self.spills, 1));
                        jsast::seq(vec![
                            jsast::assign(spill, value::field_expr(self.interns, v.clone(), key)),
                            se.clone(),
                        ])
                    }
                    None => se,
                };
                let ok_branch = match ok_key {
                    Some(key) => value::field_expr(self.interns, v, key),
                    None => jsast::undefined(),
                };
                let body = jsast::cond(is_err, err_branch, ok_branch);
                Ok(jsast::ret(match setup {
                    Some(setup) => jsast::seq(vec![setup, body]),
                    None => body,
                }))
            }
        }
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
        ) || self.is_soa_ptr(v);
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

    fn value_has_set_field(&self, v: ValueId) -> bool {
        let mut target = v;
        for _ in 0..8 {
            if self.func.blocks.iter().any(|b| {
                b.ops
                    .iter()
                    .any(|op| matches!(op.kind, OpKind::SetField { base, .. } if base == target))
            }) {
                return true;
            }
            match self.def_of(target).map(|op| &op.kind) {
                Some(OpKind::Move(inner) | OpKind::Copy(inner)) => target = *inner,
                _ => break,
            }
        }
        false
    }

    /// The place a pointer value points at: the walked place itself for an inline synthetic view
    /// (`plan.rs`, "Synthetic views": a `BorrowView` consumed by exactly one `Deref`/`StoreThrough`
    /// is spelled as v1's read/write of the place, never as a pointer record), and the `Deref`
    /// of the pointer expression otherwise.
    pub(crate) fn pointee_place(&self, pointer: ValueId) -> Res<super::place::Walk> {
        if let Some((buf, col, off)) = self.soa_bool_refs.borrow().get(&pointer).cloned() {
            let ty = value::pointee_of(self.table, self.ty_of(pointer)?)
                .ok_or_else(|| decline(OpClass::Place, "SoA bool view without pointee"))?;
            return Ok(super::place::Walk {
                place: super::place::JsPlace::SoaBool { buf, col, off },
                ty,
                identity: None,
                object: false,
                str_kind: super::place::StrPlaceKind::None,
                smi_u32: false,
            });
        }
        if let Some((buf, off)) = self.soa_refs.borrow().get(&pointer).cloned() {
            let elem_ty = value::pointee_of(self.table, self.ty_of(pointer)?)
                .ok_or_else(|| decline(OpClass::Place, "SoA element view without pointee"))?;
            return Ok(super::place::Walk {
                place: super::place::JsPlace::SoaElem { buf, off, elem_ty },
                ty: elem_ty,
                identity: None,
                object: false,
                str_kind: super::place::StrPlaceKind::None,
                smi_u32: false,
            });
        }
        if self.is_inline(pointer) {
            if let Some(Op {
                kind: OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. },
                ..
            }) = self.def_of(pointer)
            {
                return self.walk_place(root, path);
            }
        }
        if self.is_direct_ref_copy_cast(pointer) && !self.is_soa_ptr(pointer) {
            let ty = self.ty_of(pointer)?;
            if let Some(pointee) = value::pointee_of(self.table, ty) {
                let expr = self.expr_of(pointer)?;
                return Ok(super::place::Walk {
                    place: super::place::JsPlace::Deref(expr),
                    ty: pointee,
                    identity: None,
                    object: super::place::is_object(self.table, pointee),
                    str_kind: super::place::StrPlaceKind::None,
                    smi_u32: false,
                });
            }
        }
        self.walk_place(
            &Root::Value(pointer),
            &[rcgjs_object::jir::repr::Proj::Deref],
        )
    }

    /// An `Assert` (`base.rs:707-723`, `857-901`): `if (!cond) { <panic> }`.
    fn assert_stmts(
        &self,
        cond: ValueId,
        expected: bool,
        msg: &RAssertMessage,
        track_caller: Option<SourceFactId>,
    ) -> Res<Vec<Stmt>> {
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
        Ok(vec![jsast::if_(
            failed,
            self.assert_failure(msg, track_caller)?,
        )])
    }

    /// The body of the `if` an `Assert` guards (`base.rs::codegen_assert_failure`).
    fn assert_failure(
        &self,
        msg: &RAssertMessage,
        track_caller: Option<SourceFactId>,
    ) -> Res<Vec<Stmt>> {
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
                // The panic lang items are `#[track_caller]`: prefer the MIR-resolved
                // `track_caller` fact (which handles MIR-inlined `#[track_caller]` frames as
                // well as non-`#[track_caller]` helpers inlined into a `#[track_caller]` body),
                // then `$loc` inside a `#[track_caller]` body, and finally the assert's origin.
                if let Some(fact_id) = track_caller {
                    let Some(fact) = self.interns.source_fact(fact_id) else {
                        return Err(decline(
                            OpClass::Terminator,
                            "an assert whose track_caller source fact is missing",
                        ));
                    };
                    arguments.push(self.caller_location_value(fact)?);
                } else if self.func.sig.abi.caller_location {
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
                let call = jsast::call(jsast::id((*name).clone()), arguments);
                let stmt = if self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed) {
                    jsast::throw(call)
                } else {
                    jsast::expr_stmt(call)
                };
                Ok(vec![stmt])
            }
            [] => {
                let call = jsast::rt_call("js_abort", vec![jsast::string(assert_message(msg))]);
                let stmt = if self.opts.profile == Some(crate::opts::Profile::ReleaseSpeed) {
                    jsast::throw(call)
                } else {
                    jsast::expr_stmt(call)
                };
                Ok(vec![stmt])
            }
            _ => Err(decline(
                OpClass::Terminator,
                format!("the `{lang_fn}` panic function is ambiguous among the item's references"),
            )),
        }
    }
}

/// Whether an emitted item name is `<path>$<lang_fn>$h<hash>`.
pub(crate) fn names_lang_fn(name: &str, lang_fn: &str) -> bool {
    let Some((stem, hash)) = name.rsplit_once("$h") else {
        return false;
    };
    if hash.len() != 16 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    stem == lang_fn || stem.ends_with(&format!("${lang_fn}"))
}

/// The lang item function name of an `AssertKind` (`AssertKind::panic_function`).
pub(crate) fn panic_function(msg: &RAssertMessage) -> &'static str {
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
                || name.as_str() == super::i64_pair::HELPER_RL
                || name.as_str() == super::i64_pair::HELPER_RH
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

#[cfg(test)]
mod sentinel_tests {
    use rcgjs_object::item::JsName;
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::origin::OriginId;
    use rcgjs_object::jir::repr::{Func, FuncBuilder, OpKind, RTerm};
    use rcgjs_object::jir::semantic::{
        AbiSignature, CallSemantic, Callee, ConstTree, DefinitionId, ItemId, Signature, TypeId,
    };
    use rcgjs_object::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
    use rcgjs_object::jir::types::{
        EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
        NominalDiscriminant, NominalForm, PinnedBoundary, TypeDescriptor, TypeKind, TypeMeta,
        VariantDescriptor,
    };
    use rcgjs_object::jsast;
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};
    use rcgjs_object::opts::{OptimizerMode, Opts, Profile};

    use crate::lower::fuse;
    use crate::lower::js::JsCx;
    use crate::plan::abi_flat::{AbiPlan, FlatResult, FlatSig, SentinelResultShape};

    fn render_func(
        name: &str,
        func: &Func,
        interns: &Interns,
        abi: Option<&AbiPlan>,
        target_abi: Option<&FlatSig>,
    ) -> String {
        let opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let plan = fuse::plan(func, None, &interns.types, interns, &opts);
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
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
        let spills = vec![JsName::new("$r1".to_owned())];
        let cx = JsCx::with_plans(
            func,
            &plan,
            &interns.types,
            interns,
            &opts,
            &summary,
            crate::lower::js::ProgramPlans {
                spills: &spills,
                abi,
                target_abi,
                ..crate::lower::js::ProgramPlans::default()
            },
        );
        let structured = crate::lower::structure::structure(func, &plan, &interns.types, false)
            .expect("structure");
        let (body, _tmps) = super::emit(&cx, structured.region).expect("emit");
        let params = func
            .params
            .iter()
            .map(|v| plan.names.get(v).cloned().unwrap_or_default())
            .collect();
        let decl = jsast::function(name.to_owned(), params, body);
        jsast::stmt_to_string(&decl)
    }

    fn base_facts(id: TypeId) -> JsFacts {
        JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: id,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        }
    }

    #[test]
    fn sentinel_niche_lowers_switch_const_and_aggregate_without_tag_objects() {
        let mut interns = Interns::new();
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let s_f0 = interns.intern_str("_0");

        let t_u32 = TypeId(1);
        let t_js = TypeId(2);
        let t_opt_js = TypeId(3);

        let mut u32_d = TypeDescriptor {
            id: t_u32,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_u32),
        };
        u32_d.js.int = Some(IntFacts {
            signed: false,
            bits: 32,
            repr: IntRepr::Number,
        });
        interns.types.insert(u32_d);
        interns.types.insert(TypeDescriptor {
            id: t_js,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 1,
                },
                args: Vec::new(),
                form: NominalForm::Opaque,
                variants: Vec::new(),
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta {
                boundary: PinnedBoundary::Host,
                ..Default::default()
            },
            js: base_facts(t_js),
        });
        let mut opt_js = TypeDescriptor {
            id: t_opt_js,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 2,
                },
                args: vec![t_js],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_none,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_some,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_f0),
                            ty: t_js,
                            offset: 4,
                        }],
                        logical_discr: 1,
                        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(8),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_opt_js),
        };
        opt_js.js.enum_repr = Some(EnumRepr::SentinelNiche {
            none_variant: 0,
            some_variant: 1,
            some_field: 0,
        });
        interns.types.insert(opt_js);
        interns.types.set_sentinel_none(Some("$sn".to_owned()));

        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 10,
                },
                instantiation: 0,
            },
            Signature {
                arguments: vec![t_opt_js],
                result: t_opt_js,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            },
        );
        let p = b.param(t_opt_js, OriginId(0));
        let b_none = b.block();
        let b_some = b.block();
        let discr = b.op(
            FuncBuilder::ENTRY,
            OpKind::Discriminant(p),
            t_u32,
            OriginId(0),
        );
        b.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: t_u32,
                cases: vec![(0, b_none, Vec::new())],
                default: (b_some, Vec::new()),
            },
        );
        let none_v = b.op(
            b_none,
            OpKind::Const(ConstTree::Variant {
                variant: 0,
                fields: Vec::new(),
            }),
            t_opt_js,
            OriginId(0),
        );
        b.terminate(b_none, RTerm::Return(Some(none_v)));
        let inner = b.op(
            b_some,
            OpKind::Field {
                base: p,
                variant: Some(1),
                field: 0,
            },
            t_js,
            OriginId(0),
        );
        let some_v = b.op(
            b_some,
            OpKind::Aggregate {
                ty: t_opt_js,
                variant: 1,
                active_field: None,
                fields: vec![inner],
            },
            t_opt_js,
            OriginId(0),
        );
        b.terminate(b_some, RTerm::Return(Some(some_v)));
        let func = b.finish();

        let js = render_func("id_opt_js", &func, &interns, None, None);
        assert!(js.contains("=== $sn"), "expected `=== $sn` in:\n{js}");
        assert!(
            js.contains("return $sn;"),
            "expected `return $sn;` in:\n{js}"
        );
        assert!(
            !js.contains("TAG"),
            "SentinelNiche must not emit TAG objects:\n{js}"
        );
    }

    #[test]
    fn sentinel_result_lowers_recursive_invoke_zero_alloc_and_materializes_escaping_caller() {
        let mut interns = Interns::new();
        let s_ok = interns.intern_str("Ok");
        let s_err = interns.intern_str("Err");
        let s_f0 = interns.intern_str("_0");
        let s_eval = interns.intern_str("eval");

        let t_u32 = TypeId(1);
        let t_res = TypeId(2);
        let mut u32_d = TypeDescriptor {
            id: t_u32,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_u32),
        };
        u32_d.js.int = Some(IntFacts {
            signed: false,
            bits: 32,
            repr: IntRepr::Number,
        });
        interns.types.insert(u32_d);
        let mut res_d = TypeDescriptor {
            id: t_res,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 20,
                },
                args: vec![t_u32, t_u32],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_ok,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_f0),
                            ty: t_u32,
                            offset: 4,
                        }],
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: true,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_err,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_f0),
                            ty: t_u32,
                            offset: 4,
                        }],
                        logical_discr: 1,
                        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(8),
                align: Some(4),
                sized: true,
                needs_drop: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_res),
        };
        res_d.js.is_indirect = true;
        res_d.js.needs_clone = true;
        res_d.js.enum_repr = Some(EnumRepr::IntTag { fieldless: false });
        interns.types.insert(res_d);

        let eval_id = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 30,
            },
            instantiation: 0,
        };
        let shape = SentinelResultShape {
            ty: t_res,
            ok_variant: 0,
            ok_field: Some((0, t_u32)),
            err_variant: 1,
            err_field: Some((0, t_u32)),
        };
        let eval_sig = FlatSig {
            params: vec![None],
            result: Some(FlatResult::sentinel_result(t_res, shape)),
            spill: 1,
            returns_out_field: false,
        };
        let abi =
            AbiPlan::for_target_test(eval_id, JsName::new("eval".to_owned()), eval_sig.clone())
                .with_sentinel_err_test(JsName::new("$se".to_owned()));

        // Recursive `eval` with `RTerm::Invoke`, `?`, and block-param join before return.
        let mut eb = FuncBuilder::new(
            eval_id,
            Signature {
                arguments: vec![t_u32],
                result: t_res,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            },
        );
        let x = eb.param(t_u32, OriginId(0));
        let b_rec = eb.block();
        let b_norm = eb.block();
        let b_unwind = eb.block();
        let b_ok = eb.block();
        let b_err = eb.block();
        let b_ret = eb.block();
        let p_ret = eb.block_param(b_ret, t_res, OriginId(0));
        eb.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: x,
                scrut_ty: t_u32,
                cases: vec![(0, b_rec, Vec::new())],
                default: (b_ret, Vec::new()),
            },
        );
        // Fix up default arm cleanly via a separate base block.
        let b_base = eb.block();
        let base_ok = eb.op(
            b_base,
            OpKind::Aggregate {
                ty: t_res,
                variant: 0,
                active_field: None,
                fields: vec![x],
            },
            t_res,
            OriginId(0),
        );
        eb.terminate(b_base, RTerm::Goto(b_ret, vec![base_ok]));
        eb.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: x,
                scrut_ty: t_u32,
                cases: vec![(0, b_base, Vec::new())],
                default: (b_rec, Vec::new()),
            },
        );

        let sub = eb.op(
            b_rec,
            OpKind::Call {
                callee: Callee::Direct {
                    item: eval_id,
                    link_name: s_eval,
                },
                function: None,
                args: vec![x],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            t_res,
            OriginId(0),
        );
        eb.terminate(
            b_rec,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unwind, Vec::new()),
            },
        );
        eb.terminate(b_unwind, RTerm::Throw(None));
        let discr = eb.op(b_norm, OpKind::Discriminant(sub), t_u32, OriginId(0));
        eb.terminate(
            b_norm,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: t_u32,
                cases: vec![(0, b_ok, Vec::new())],
                default: (b_err, Vec::new()),
            },
        );
        let ok_v = eb.op(
            b_ok,
            OpKind::Field {
                base: sub,
                variant: Some(0),
                field: 0,
            },
            t_u32,
            OriginId(0),
        );
        let ret_ok = eb.op(
            b_ok,
            OpKind::Aggregate {
                ty: t_res,
                variant: 0,
                active_field: None,
                fields: vec![ok_v],
            },
            t_res,
            OriginId(0),
        );
        eb.terminate(b_ok, RTerm::Goto(b_ret, vec![ret_ok]));
        let err_v = eb.op(
            b_err,
            OpKind::Field {
                base: sub,
                variant: Some(1),
                field: 0,
            },
            t_u32,
            OriginId(0),
        );
        let ret_err = eb.op(
            b_err,
            OpKind::Aggregate {
                ty: t_res,
                variant: 1,
                active_field: None,
                fields: vec![err_v],
            },
            t_res,
            OriginId(0),
        );
        eb.terminate(b_err, RTerm::Goto(b_ret, vec![ret_err]));
        eb.terminate(b_ret, RTerm::Return(Some(p_ret)));
        let eval_func = eb.finish();

        let eval_js = render_func("eval", &eval_func, &interns, Some(&abi), Some(&eval_sig));
        assert!(
            eval_js.contains("try {"),
            "Invoke must stay guarded:\n{eval_js}"
        );
        assert!(
            eval_js.contains("=== $se"),
            "must test against $se:\n{eval_js}"
        );
        assert!(
            eval_js.contains("$r1 = void 0") || eval_js.contains("$r1 = undefined"),
            "must clear $r1 after capturing err:\n{eval_js}"
        );
        assert!(
            !eval_js.contains("TAG"),
            "recursive SentinelResult callee must emit ZERO {{ TAG }} allocations:\n{eval_js}"
        );

        // Escaping canonical wrapper calling `eval` (no `target_abi`).
        let mut wb = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 31,
                },
                instantiation: 0,
            },
            Signature {
                arguments: vec![t_u32],
                result: t_res,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            },
        );
        let wx = wb.param(t_u32, OriginId(0));
        let wres = wb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: eval_id,
                    link_name: s_eval,
                },
                function: None,
                args: vec![wx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            t_res,
            OriginId(0),
        );
        wb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(wres)));
        let wrapper_func = wb.finish();

        let wrapper_js = render_func("wrapper", &wrapper_func, &interns, Some(&abi), None);
        assert!(
            wrapper_js.contains("=== $se"),
            "escaping caller must branch on $se:\n{wrapper_js}"
        );
        assert!(
            wrapper_js.contains("TAG: 0") && wrapper_js.contains("TAG: 1"),
            "escaping caller must materialize canonical Ok/Err objects:\n{wrapper_js}"
        );
        assert!(
            wrapper_js.contains("$r1 = void 0") || wrapper_js.contains("$r1 = undefined"),
            "escaping caller must clear $r1 on Err:\n{wrapper_js}"
        );
    }
}
