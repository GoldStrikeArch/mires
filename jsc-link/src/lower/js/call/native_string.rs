//! Phase 5 universal native `{ s: byteString, cap: number }` `String` hooks and
//! `str::to_{lower,upper}case` lowering.

use rcgjs_object::jir::repr::ValueId;
use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jsast;

use super::super::{JsCx, Res};
use super::Lowered;

impl JsCx<'_> {
    /// Lowers Phase 5 `alloc::string::rcgjs_str_*` hooks and `str::to_{lower,upper}case` directly
    /// against the universal `{ s: byteString, cap: number }` `String` representation.
    pub(super) fn lower_native_string_call(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeStrNew
            | StandardOperation::NativeStrWithCapacity
            | StandardOperation::NativeStrFromStr
            | StandardOperation::NativeStrAsStr
            | StandardOperation::NativeStrLen
            | StandardOperation::NativeStrCapacity => {
                self.lower_native_str_construct_and_query(operation, args)
            }
            StandardOperation::NativeStrPushStr
            | StandardOperation::NativeStrPush
            | StandardOperation::NativeStrClear
            | StandardOperation::NativeStrTruncate
            | StandardOperation::NativeStrReplaceSlice
            | StandardOperation::NativeStrExtendWithin
            | StandardOperation::NativeStrCloneFrom => {
                self.lower_native_str_mutate(operation, args)
            }
            StandardOperation::NativeStrReserve
            | StandardOperation::NativeStrShrinkTo
            | StandardOperation::NativeStrFromVec
            | StandardOperation::NativeStrIntoBytes => {
                self.lower_native_str_capacity_and_convert(operation, args)
            }
            StandardOperation::NativeStrToAsciiLowercase
            | StandardOperation::NativeStrToAsciiUppercase
            | StandardOperation::NativeStrRepeat
            | StandardOperation::StrToLowercase
            | StandardOperation::StrToUppercase => self.lower_native_str_transform(operation, args),
            _ => Ok(None),
        }
    }

    fn lower_native_str_construct_and_query(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeStrNew => Ok(Some(Lowered::value(jsast::object_of(vec![
                ("s", jsast::string("")),
                ("cap", jsast::num(0)),
            ])))),
            StandardOperation::NativeStrWithCapacity => {
                let [cap_v] = args else {
                    return Ok(None);
                };
                if let Some(c) = self.const_scalar_u128(*cap_v) {
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
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
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
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
                let cap = jsast::member(s.clone(), "cap");
                let len = jsast::member(jsast::member(s, "s"), "length");
                let expr = jsast::cond(
                    jsast::binary(jsast::BinOp::Gt, cap.clone(), len.clone()),
                    cap,
                    len,
                );
                Ok(Some(Lowered::Value(setup, expr)))
            }
            _ => unreachable!("matched in lower_native_string_call"),
        }
    }

    fn lower_native_str_mutate(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeStrPushStr => {
                let [s_v, rhs_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                let rhs = self.expr_of(*rhs_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(
                    jsast::compound_assign(jsast::CompoundOp::Add, jsast::member(s, "s"), rhs),
                )])))
            }
            StandardOperation::NativeStrPush => {
                let [s_v, ch_v] = args else {
                    return Ok(None);
                };
                let s = self.expr_of(*s_v)?;
                if let Some(c) = self.const_scalar_u128(*ch_v) {
                    if let Some(ch) = u32::try_from(c).ok().and_then(char::from_u32) {
                        let mut buf = [0u8; 4];
                        let piece = crate::lower::bstr::literal_for(
                            ch.encode_utf8(&mut buf),
                            self.bstr_enabled() || self.heap().native_string_enabled,
                        );
                        return Ok(Some(Lowered::Done(vec![jsast::expr_stmt(
                            jsast::compound_assign(
                                jsast::CompoundOp::Add,
                                jsast::member(s, "s"),
                                piece,
                            ),
                        )])));
                    }
                }
                let mut setup = Vec::new();
                let s = self.bind_atom(s, &mut setup);
                let ch = self.bind_atom(self.expr_of(*ch_v)?, &mut setup);
                let piece = jsast::cond(
                    jsast::binary(jsast::BinOp::Lt, ch.clone(), jsast::num(128)),
                    jsast::method_call(jsast::id("String"), "fromCharCode", vec![ch.clone()]),
                    jsast::rt_call("bstr_char", vec![ch]),
                );
                setup.push(jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
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
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
                let new_len = self.bind_atom(self.expr_of(*len_v)?, &mut setup);
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
            StandardOperation::NativeStrReplaceSlice => {
                let [s_v, start_v, end_v, rep_v] = args else {
                    return Ok(None);
                };
                let mut setup = Vec::new();
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
                let start = self.bind_atom(self.expr_of(*start_v)?, &mut setup);
                let end = self.bind_atom(self.expr_of(*end_v)?, &mut setup);
                let rep = self.bind_atom(self.expr_of(*rep_v)?, &mut setup);
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
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
                let start = self.bind_atom(self.expr_of(*start_v)?, &mut setup);
                let end = self.bind_atom(self.expr_of(*end_v)?, &mut setup);
                let field = jsast::member(s, "s");
                let sub = jsast::method_call(field.clone(), "slice", vec![start, end]);
                setup.push(jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
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
                let dst = self.bind_atom(self.expr_of(*dst_v)?, &mut setup);
                let src = self.bind_atom(self.expr_of(*src_v)?, &mut setup);
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
            _ => unreachable!("matched in lower_native_string_call"),
        }
    }

    fn lower_native_str_capacity_and_convert(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
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
                let s = self.bind_atom(self.expr_of(*s_v)?, &mut setup);
                let min_cap = self.bind_atom(self.expr_of(*min_v)?, &mut setup);
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
                let v = self.bind_atom(self.expr_of(*vec_v)?, &mut setup);
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
                            jsast::member(ptr.clone(), rcgjs_object::ptr::BUF),
                            jsast::member(ptr, rcgjs_object::ptr::OFF),
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
            _ => unreachable!("matched in lower_native_string_call"),
        }
    }

    fn lower_native_str_transform(
        &self,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
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
            _ => unreachable!("matched in lower_native_string_call"),
        }
    }
}
