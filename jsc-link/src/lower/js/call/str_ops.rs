//! Byte-string (`js-str=bytes`) `str` operations, fast `core::str` comparison/search hooks,
//! and `[u8]` `starts_with`/`ends_with` byte-view search lowerings.

use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::SourceFactId;
use rcgjs_object::jir::repr::{Op, OpKind, Root, ValueId};
use rcgjs_object::jir::semantic::{Callee, CastKind, ConstTree, ItemId, StandardOperation};
use rcgjs_object::jir::types::{TypeId, TypeKind};
use rcgjs_object::jsast::{self, Expr, Stmt};

use super::super::{value, ByteView, ByteViewMode, JsCx, Res};
use super::Lowered;

enum StrRange {
    Full,
    From { start: Expr, range: Expr },
    To { end: Expr, range: Expr },
    Bounds { start: Expr, end: Expr, range: Expr },
}

impl JsCx<'_> {
    fn is_str_val(&self, value: ValueId) -> Res<bool> {
        Ok(crate::lower::bstr::is_str_pointer(
            self.table,
            self.ty_of(value)?,
        ))
    }

    fn is_char_val(&self, value: ValueId) -> Res<bool> {
        Ok(matches!(
            value::kind(self.table, self.ty_of(value)?),
            Some(TypeKind::Char)
        ))
    }

    /// Holds an expression in a temp if it is not already an atomic literal/ident.
    fn hold_str_atom(&self, expr: Expr, prelude: &mut Vec<Stmt>) -> Expr {
        if matches!(expr, Expr::Ident(_) | Expr::Str(_) | Expr::Num(_)) {
            expr
        } else {
            let (bind, held) = self.temp(expr);
            prelude.push(bind);
            held
        }
    }

    fn str_needle_expr(&self, value: ValueId, prelude: &mut Vec<Stmt>) -> Res<Option<Expr>> {
        if self.is_str_val(value)? {
            return Ok(Some(self.hold_str_atom(self.expr_of(value)?, prelude)));
        }
        if self.is_char_val(value)? {
            return Ok(Some(match self.expr_of(value)? {
                Expr::Num(point)
                    if point.fract() == 0.0 && (0.0..=0x10_FFFF as f64).contains(&point) =>
                {
                    if let Some(ch) = char::from_u32(point as u32) {
                        let mut buf = [0u8; 4];
                        jsast::string(crate::lower::bstr::byte_string(ch.encode_utf8(&mut buf)))
                    } else {
                        self.hold_str_atom(
                            crate::lower::bstr::char_text_for(
                                Expr::Num(point),
                                self.bstr_enabled(),
                            ),
                            prelude,
                        )
                    }
                }
                point => self.hold_str_atom(
                    crate::lower::bstr::char_text_for(point, self.bstr_enabled()),
                    prelude,
                ),
            }));
        }
        Ok(None)
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
    pub(super) fn lower_str_call(
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
        let mut prelude = Vec::new();
        match operation {
            StandardOperation::StrLen
            | StandardOperation::StrByteAt
            | StandardOperation::StrSliceUnchecked
            | StandardOperation::StrEq
            | StandardOperation::StrTrim
            | StandardOperation::StrTrimStart
            | StandardOperation::StrTrimEnd => {
                self.lower_str_basic_call(operation, args, &mut prelude)
            }
            StandardOperation::StrIsAscii
            | StandardOperation::SliceIsAscii
            | StandardOperation::StrIsCharBoundary => {
                self.lower_str_ascii_call(operation, args, &mut prelude)
            }
            StandardOperation::StrStartsWith
            | StandardOperation::StrEndsWith
            | StandardOperation::StrContains
            | StandardOperation::StrFind
            | StandardOperation::StrRFind => {
                self.lower_str_search_call(operation, args, result, &mut prelude)
            }
            StandardOperation::StrCmp | StandardOperation::StrPartialCmp => {
                self.lower_str_cmp_call(operation, args, result, &mut prelude)
            }
            StandardOperation::StrStripPrefix | StandardOperation::StrStripSuffix => {
                self.lower_str_strip_call(operation, args, result, &mut prelude)
            }
            StandardOperation::StrGet => self.lower_str_get_call(args, result, &mut prelude),
            StandardOperation::StrIndex => self.lower_str_index_call(
                callee,
                *item,
                *link_name,
                args,
                track_caller,
                &mut prelude,
            ),
            _ => Ok(None),
        }
    }

    fn lower_str_basic_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let value = match (operation, args) {
            (StandardOperation::StrLen, [s]) if self.is_str_val(*s)? => {
                jsast::member(self.hold_str_atom(self.expr_of(*s)?, prelude), "length")
            }
            (StandardOperation::StrByteAt, [s, index]) if self.is_str_val(*s)? => {
                let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
                let index = self.hold_str_atom(self.expr_of(*index)?, prelude);
                jsast::method_call(s, "charCodeAt", vec![index])
            }
            (StandardOperation::StrSliceUnchecked, [s, start, end]) if self.is_str_val(*s)? => {
                let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
                let start = self.hold_str_atom(self.expr_of(*start)?, prelude);
                let end = self.hold_str_atom(self.expr_of(*end)?, prelude);
                jsast::method_call(s, "slice", vec![start, end])
            }
            (StandardOperation::StrEq, [a, b])
                if self.is_str_val(*a)? && self.is_str_val(*b)? =>
            {
                let a = self.hold_str_atom(self.expr_of(*a)?, prelude);
                let b = self.hold_str_atom(self.expr_of(*b)?, prelude);
                jsast::binary(jsast::BinOp::StrictEq, a, b)
            }
            (
                StandardOperation::StrTrim
                | StandardOperation::StrTrimStart
                | StandardOperation::StrTrimEnd,
                [s],
            ) if self.is_str_val(*s)? => {
                let helper = match operation {
                    StandardOperation::StrTrim => "bstr_trim",
                    StandardOperation::StrTrimStart => "bstr_trim_start",
                    _ => "bstr_trim_end",
                };
                jsast::rt_call(helper, vec![self.hold_str_atom(self.expr_of(*s)?, prelude)])
            }
            _ => return Ok(None),
        };
        Ok(Some(Lowered::Value(std::mem::take(prelude), value)))
    }

    fn lower_str_ascii_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let table = self.table;
        let value = match (operation, args) {
            // A byte string is ASCII when every code unit is; `core`'s word-at-a-time scan would
            // first build the string's byte array and then read it through scalar views.
            (StandardOperation::StrIsAscii, [s]) if self.is_str_val(*s)? => {
                let expr = self.expr_of(*s)?;
                if self.is_known_ascii(*s) {
                    if !jsast::is_droppable(&expr) {
                        prelude.push(jsast::expr_stmt(expr));
                    }
                    Expr::Bool(true)
                } else {
                    jsast::rt_call("_bstr_is_ascii", vec![self.hold_str_atom(expr, prelude)])
                }
            }
            // The same answer one byte at a time over a `{ buf, off, len }` slice (or directly
            // over a `js-str=bytes` byte-string view).
            (StandardOperation::SliceIsAscii, [bytes])
                if matches!(
                    table.get(table.peeled(self.ty_of(*bytes)?)).map(|d| &d.kind),
                    Some(TypeKind::Reference {
                        pointee,
                        mutable: false,
                    }) if matches!(
                        table.get(table.peeled(*pointee)).map(|d| &d.kind),
                        Some(TypeKind::Slice(elem)) if table.byte_element(*elem)
                    )
                ) =>
            {
                if let Some(bv) = self.byte_view(*bytes) {
                    if matches!(bv.mode, ByteViewMode::FullSlice | ByteViewMode::VarSlice)
                        && bv.base_str.is_some_and(|b| self.is_known_ascii(b))
                    {
                        let expr = self.expr_of(*bytes)?;
                        if !jsast::is_droppable(&expr) {
                            prelude.push(jsast::expr_stmt(expr));
                        }
                        Expr::Bool(true)
                    } else {
                        match bv.mode {
                            ByteViewMode::FullSlice => {
                                jsast::rt_call("_bstr_is_ascii", vec![self.expr_of(*bytes)?])
                            }
                            ByteViewMode::VarSlice => {
                                let (base, off, len) =
                                    self.extract_byte_view_var_slice(bv, *bytes, prelude)?;
                                jsast::rt_call("_bstr_is_ascii_range", vec![base, off, len])
                            }
                            ByteViewMode::ThinPtr => {
                                jsast::rt_call("bytes_is_ascii", vec![self.expr_of(*bytes)?])
                            }
                        }
                    }
                } else {
                    jsast::rt_call("bytes_is_ascii", vec![self.expr_of(*bytes)?])
                }
            }
            (StandardOperation::StrIsCharBoundary, [s, index]) if self.is_str_val(*s)? => {
                let s_expr = self.hold_str_atom(self.expr_of(*s)?, prelude);
                let index_expr = self.hold_str_atom(self.expr_of(*index)?, prelude);
                if self.is_known_ascii(*s) {
                    let is_u32 = matches!(
                        value::kind(table, table.peeled(self.ty_of(*index)?)),
                        Some(TypeKind::Integer {
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
            _ => return Ok(None),
        };
        Ok(Some(Lowered::Value(std::mem::take(prelude), value)))
    }

    fn lower_str_search_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
        result: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let [s, pattern] = args else {
            return Ok(None);
        };
        if !self.is_str_val(*s)? || !(self.is_str_val(*pattern)? || self.is_char_val(*pattern)?) {
            return Ok(None);
        }
        let find = matches!(
            operation,
            StandardOperation::StrFind | StandardOperation::StrRFind
        );
        let variants = if find {
            match self.option_variants(result)? {
                Some(variants) => Some(variants),
                None => return Ok(None),
            }
        } else {
            None
        };
        let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
        let Some(pattern) = self.str_needle_expr(*pattern, prelude)? else {
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
                    return Ok(Some(Lowered::Value(
                        std::mem::take(prelude),
                        Expr::Bool(true),
                    )));
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
                        std::mem::take(prelude),
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
        let value = match variants {
            Some((none, some)) => {
                let (bind, at) = self.temp(found);
                prelude.push(bind);
                let ty = self.ty_of(result)?;
                let (Some(none), Some(some)) = (
                    value::enum_value(self.table, self.interns, ty, none, Vec::new()),
                    value::enum_value(
                        self.table,
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
            None => found,
        };
        Ok(Some(Lowered::Value(std::mem::take(prelude), value)))
    }

    fn lower_str_cmp_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
        result: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let [a, b] = args else {
            return Ok(None);
        };
        if !self.is_str_val(*a)? || !self.is_str_val(*b)? {
            return Ok(None);
        }
        let value = match operation {
            StandardOperation::StrCmp => {
                let ordering_ty = self.ty_of(result)?;
                let a = self.hold_str_atom(self.expr_of(*a)?, prelude);
                let b = self.hold_str_atom(self.expr_of(*b)?, prelude);
                let Some(cmp) = self.str_cmp_expr(ordering_ty, a, b) else {
                    return Ok(None);
                };
                cmp
            }
            StandardOperation::StrPartialCmp => {
                let Some((_, some)) = self.option_variants(result)? else {
                    return Ok(None);
                };
                let result_ty = self.ty_of(result)?;
                let Some(ordering_field) = self.table.field(result_ty, Some(some.0), 0) else {
                    return Ok(None);
                };
                let a = self.hold_str_atom(self.expr_of(*a)?, prelude);
                let b = self.hold_str_atom(self.expr_of(*b)?, prelude);
                let Some(cmp) = self.str_cmp_expr(ordering_field.ty, a, b) else {
                    return Ok(None);
                };
                let Some(wrapped) = value::enum_value(
                    self.table,
                    self.interns,
                    result_ty,
                    some.0,
                    vec![(some.1, cmp)],
                ) else {
                    return Ok(None);
                };
                wrapped
            }
            _ => unreachable!("matched in lower_str_call"),
        };
        Ok(Some(Lowered::Value(std::mem::take(prelude), value)))
    }

    fn lower_str_strip_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
        result: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let [s, pattern] = args else {
            return Ok(None);
        };
        if !self.is_str_val(*s)? || !(self.is_str_val(*pattern)? || self.is_char_val(*pattern)?) {
            return Ok(None);
        }
        let Some((none, some)) = self.option_variants(result)? else {
            return Ok(None);
        };
        let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
        let Some(pattern) = self.str_needle_expr(*pattern, prelude)? else {
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
            value::enum_value(self.table, self.interns, ty, none, Vec::new()),
            value::enum_value(self.table, self.interns, ty, some.0, vec![(some.1, rest)]),
        ) else {
            return Ok(None);
        };
        Ok(Some(Lowered::Value(
            std::mem::take(prelude),
            jsast::cond(jsast::method_call(s, method, vec![pattern]), some, none),
        )))
    }

    fn lower_str_get_call(
        &self,
        args: &[ValueId],
        result: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let [s, range] = args else {
            return Ok(None);
        };
        if !self.is_str_val(*s)? {
            return Ok(None);
        }
        let Some((none, some)) = self.option_variants(result)? else {
            return Ok(None);
        };
        let table = self.table;
        let ty = self.ty_of(result)?;
        let s_known_ascii = self.is_known_ascii(*s);
        let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
        let Some(shape) = self.decompose_str_range(*range, prelude)? else {
            return Ok(None);
        };
        let Some(none_val) = value::enum_value(table, self.interns, ty, none, Vec::new()) else {
            return Ok(None);
        };
        if let StrRange::Full = shape {
            let Some(some_val) =
                value::enum_value(table, self.interns, ty, some.0, vec![(some.1, s)])
            else {
                return Ok(None);
            };
            return Ok(Some(Lowered::Value(std::mem::take(prelude), some_val)));
        }
        if s_known_ascii {
            let (cond, sub) = match shape {
                StrRange::Full => unreachable!("handled above"),
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
                std::mem::take(prelude),
                jsast::cond(cond, some_val, none_val),
            )));
        }
        let got = match shape {
            StrRange::Full => unreachable!("handled above"),
            StrRange::From { start, .. } => jsast::rt_call("bstr_get_from", vec![s, start]),
            StrRange::To { end, .. } => jsast::rt_call("bstr_get_to", vec![s, end]),
            StrRange::Bounds { start, end, .. } => jsast::rt_call("bstr_get", vec![s, start, end]),
        };
        let (bind, at) = self.temp(got);
        prelude.push(bind);
        let Some(some_val) =
            value::enum_value(table, self.interns, ty, some.0, vec![(some.1, at.clone())])
        else {
            return Ok(None);
        };
        Ok(Some(Lowered::Value(
            std::mem::take(prelude),
            jsast::cond(
                jsast::binary(jsast::BinOp::StrictEq, at, jsast::undefined()),
                none_val,
                some_val,
            ),
        )))
    }

    fn lower_str_index_call(
        &self,
        callee: &Callee,
        item: ItemId,
        link_name: StrId,
        args: &[ValueId],
        track_caller: Option<SourceFactId>,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Option<Lowered>> {
        let [s, range] = args else {
            return Ok(None);
        };
        if !self.is_str_val(*s)? {
            return Ok(None);
        }
        let s = self.hold_str_atom(self.expr_of(*s)?, prelude);
        let Some(shape) = self.decompose_str_range(*range, prelude)? else {
            return Ok(None);
        };
        let (helper, mut sub_args, range_expr) = match shape {
            StrRange::Full => return Ok(Some(Lowered::Value(std::mem::take(prelude), s))),
            StrRange::Bounds { start, end, range } => {
                ("bstr_sub", vec![s.clone(), start, end], range)
            }
            StrRange::From { start, range } => ("bstr_sub_from", vec![s.clone(), start], range),
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
                    .is_some_and(|abi| abi.call_signature(item).is_some())
                {
                    return Ok(None);
                }
                let name = match self.call_retargets.get(&item) {
                    Some(name) => name.as_str().to_owned(),
                    None => self.link_name(link_name)?,
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
        Ok(Some(Lowered::Value(
            std::mem::take(prelude),
            jsast::rt_call(helper, sub_args),
        )))
    }

    /// Lowers Phase 5 `core::str` fast comparison/search hooks (`patches/0021-bstr-str-ops.patch`).
    pub(super) fn lower_fast_str_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let is_bstr = self.bstr_enabled();
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
            StandardOperation::BstrStartsWith
            | StandardOperation::BstrEndsWith
            | StandardOperation::BstrContains => self.lower_fast_str_pred(operation, args),
            StandardOperation::BstrFindChar
            | StandardOperation::BstrRfindChar
            | StandardOperation::BstrFindStr
            | StandardOperation::BstrRfindStr
                if is_bstr =>
            {
                self.lower_fast_str_find(operation, args)
            }
            _ => Ok(None),
        }
    }

    fn lower_fast_str_pred(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
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
            if matches!(
                operation,
                StandardOperation::BstrStartsWith | StandardOperation::BstrEndsWith
            ) && lit.encode_utf16().count() == 1
            {
                let code = f64::from(lit.encode_utf16().next().unwrap());
                if operation == StandardOperation::BstrStartsWith {
                    return Ok(Some(Lowered::value(jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::method_call(h, "charCodeAt", vec![jsast::num(0)]),
                        jsast::num(code),
                    ))));
                }
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
        let method = match operation {
            StandardOperation::BstrStartsWith => "startsWith",
            StandardOperation::BstrEndsWith => "endsWith",
            StandardOperation::BstrContains => "includes",
            _ => unreachable!("matched in lower_fast_str_call"),
        };
        Ok(Some(Lowered::value(jsast::method_call(h, method, vec![n]))))
    }

    fn lower_fast_str_find(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [h_v, s_v, e_v, n_v] = args else {
            return Ok(None);
        };
        let h = self.expr_of(*h_v)?;
        let s = self.expr_of(*s_v)?;
        let e = self.expr_of(*e_v)?;
        let n = self.expr_of(*n_v)?;
        let const_pat: Option<(Expr, usize)> = match operation {
            StandardOperation::BstrFindChar | StandardOperation::BstrRfindChar => match &n {
                Expr::Num(point)
                    if point.fract() == 0.0 && (0.0..=0x10_FFFF as f64).contains(point) =>
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
            },
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
            let h = self.hold_str_atom(h, &mut prelude);
            let s = self.hold_str_atom(s, &mut prelude);
            let e = self.hold_str_atom(e, &mut prelude);
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
                let end_pos = jsast::binary(jsast::BinOp::Add, i.clone(), jsast::num(plen as f64));
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

    /// Extracts `(base, off, len)` for a `VarSlice` byte-view while evaluating non-atomic
    /// expressions at most once.
    fn extract_byte_view_var_slice(
        &self,
        bv: ByteView,
        v: ValueId,
        prelude: &mut Vec<Stmt>,
    ) -> Res<(Expr, Expr, Expr)> {
        if bv.base_str.is_none() {
            let expr = self.expr_of(v)?;
            if let Expr::Object(ref entries) = expr {
                let field = |k: &str| {
                    entries
                        .iter()
                        .find(|(ek, _)| ek == k)
                        .map(|(_, ev)| ev.clone())
                };
                if let (Some(b), Some(o), Some(l)) = (field("buf"), field("off"), field("len")) {
                    let b = self.bind_atom(b, prelude);
                    let o = self.bind_atom(o, prelude);
                    let l = self.bind_atom(l, prelude);
                    return Ok((b, o, l));
                }
            }
            let held = self.bind_atom(expr, prelude);
            let b = self.bind_atom(jsast::member(held.clone(), "buf"), prelude);
            let o = self.bind_atom(jsast::member(held.clone(), "off"), prelude);
            let l = self.bind_atom(jsast::member(held, "len"), prelude);
            return Ok((b, o, l));
        }
        let base = self.bind_atom(self.byte_view_base_expr(bv, v)?, prelude);
        let off = self.bind_atom(self.byte_view_off_expr(v)?, prelude);
        let len = self.bind_atom(self.byte_view_len_expr(v)?, prelude);
        Ok((base, off, len))
    }

    /// Extracts constant raw bytes when `v` is defined purely by a constant `&[u8]` / `&[u8; N]`
    /// / `&str` literal without side effects.
    fn const_byte_slice(&self, mut v: ValueId) -> Option<Vec<u8>> {
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
    pub(super) fn lower_bstr_slice_search(
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
            Some(TypeKind::Bool)
        ) {
            return Ok(None);
        }
        let Some(hbv) = self.byte_view(*h_v) else {
            return Ok(None);
        };
        let nbv = self.byte_view(*n_v);
        self.lower_bstr_slice_search_views(is_starts, *h_v, hbv, *n_v, nbv)
    }

    pub(super) fn lower_bstr_slice_search_views(
        &self,
        is_starts: bool,
        h_v: ValueId,
        hbv: ByteView,
        n_v: ValueId,
        nbv: Option<ByteView>,
    ) -> Res<Option<Lowered>> {
        if !matches!(hbv.mode, ByteViewMode::FullSlice | ByteViewMode::VarSlice) {
            return Ok(None);
        }
        let mut prelude = Vec::new();
        if let Some(bytes) = self.const_byte_slice(n_v) {
            let expr =
                self.lower_bstr_slice_search_const(is_starts, h_v, hbv, &bytes, &mut prelude)?;
            return Ok(Some(Lowered::Value(prelude, expr)));
        }
        let Some(nbv) = nbv else {
            return Ok(None);
        };
        if !matches!(nbv.mode, ByteViewMode::FullSlice | ByteViewMode::VarSlice) {
            return Ok(None);
        }
        let expr =
            self.lower_bstr_slice_search_dynamic(is_starts, h_v, hbv, n_v, nbv, &mut prelude)?;
        Ok(Some(Lowered::Value(prelude, expr)))
    }

    fn lower_bstr_slice_search_const(
        &self,
        is_starts: bool,
        h_v: ValueId,
        hbv: ByteView,
        bytes: &[u8],
        prelude: &mut Vec<Stmt>,
    ) -> Res<Expr> {
        let nlen = bytes.len();
        if nlen == 0 {
            let h_expr = self.expr_of(h_v)?;
            if !jsast::is_droppable(&h_expr) {
                prelude.push(jsast::expr_stmt(h_expr));
            }
            return Ok(Expr::Bool(true));
        }
        let pat_str: String = bytes.iter().copied().map(char::from).collect();
        let pat = jsast::string(pat_str);
        Ok(match hbv.mode {
            ByteViewMode::FullSlice => {
                let h = self.hold_str_atom(self.expr_of(h_v)?, prelude);
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
            ByteViewMode::VarSlice => {
                let (base, off, len) = self.extract_byte_view_var_slice(hbv, h_v, prelude)?;
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
            ByteViewMode::ThinPtr => unreachable!(),
        })
    }

    fn lower_bstr_slice_search_dynamic(
        &self,
        is_starts: bool,
        h_v: ValueId,
        hbv: ByteView,
        n_v: ValueId,
        nbv: ByteView,
        prelude: &mut Vec<Stmt>,
    ) -> Res<Expr> {
        Ok(match (hbv.mode, nbv.mode) {
            (ByteViewMode::FullSlice, ByteViewMode::FullSlice) => {
                let h = self.hold_str_atom(self.expr_of(h_v)?, prelude);
                let n = self.hold_str_atom(self.expr_of(n_v)?, prelude);
                let method = if is_starts { "startsWith" } else { "endsWith" };
                jsast::method_call(h, method, vec![n])
            }
            (ByteViewMode::FullSlice, ByteViewMode::VarSlice) => {
                let h = self.hold_str_atom(self.expr_of(h_v)?, prelude);
                let (nb, no, nl) = self.extract_byte_view_var_slice(nbv, n_v, prelude)?;
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
            (ByteViewMode::VarSlice, ByteViewMode::FullSlice) => {
                let (hb, ho, hl) = self.extract_byte_view_var_slice(hbv, h_v, prelude)?;
                let n = self.hold_str_atom(self.expr_of(n_v)?, prelude);
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
            (ByteViewMode::VarSlice, ByteViewMode::VarSlice) => {
                let (hb, ho, hl) = self.extract_byte_view_var_slice(hbv, h_v, prelude)?;
                let (nb, no, nl) = self.extract_byte_view_var_slice(nbv, n_v, prelude)?;
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
        })
    }

    /// Three-way `Ordering` result expression `a < b ? Less : a === b ? Equal : Greater`.
    fn str_cmp_expr(&self, ordering_ty: TypeId, a: Expr, b: Expr) -> Option<Expr> {
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
                    .map(|(key, value)| (key, self.hold_str_atom(value, prelude)))
                    .collect();
                (held.clone(), Expr::Object(held))
            }
            other => {
                let held = self.hold_str_atom(other, prelude);
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
}
