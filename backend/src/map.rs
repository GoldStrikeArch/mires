//! Host collections: calling the `mires-js` map markers.
//!
//! `mires-js` gives Rust the host's own `Map` as a `HashMap`/`HashSet` (`CONTRACT.md`, "Host
//! collections"). Everything monomorphic about that is an ordinary `extern "C"` declaration and
//! reaches `__rt` with no help from here. What is left is the handful of operations that are
//! generic over the key or the value, which rustc has no foreign declaration for: those are marker
//! functions carrying [`MAP_PREFIX`] in their `link_section`, intercepted in [`crate::abi`]'s
//! `codegen_call` beside the `view-abi` markers and lowered here.
//!
//! # There is nothing to marshal
//!
//! Every marker is one `__rt` call with its arguments passed straight through, for the reason a
//! declared JavaScript interface passes its arguments through: the value model already spells a
//! number as a number, a `&str` as a string and an `Option<V>` as an object. Two things are
//! decided here rather than by the value model, and only two.
//!
//! * **A key arrives as a reference and the host wants the key.** A `&str` already IS the host
//!   string, but a `&u32` is a `{ buf, off }` slot, and handing a slot to `Map.set` would key the
//!   entry on a record nothing can ever produce again. So a slot shaped key is read through, which
//!   is the same question `text_conversion` asks of a `view_abi::text` argument.
//! * **The key's class is checked after monomorphization.** `mires-js` seals its `MapKey` trait,
//!   so an aggregate key is already a type error at the call site and this check is unreachable
//!   through that crate. It is here because the marker is an ordinary generic function that any
//!   crate could call, and because "the map silently never finds this key again" is the failure it
//!   would otherwise have: a host `Map` compares with SameValueZero, which is object identity for
//!   anything that is not a number, a string, a boolean or a BigInt.

use rustc_abi::VariantIdx;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::jsast::{self, BinOp, Expr, Stmt};

/// The `link_section` prefix every `mires-js` collection marker carries.
pub(crate) const MAP_PREFIX: &str = "rcgjs.map.";

/// The marker `def_id` is, by the `link_section` it carries.
pub(crate) fn map_section(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(MAP_PREFIX).map(str::to_owned)
}

/// Expected arity of each `mires-js` collection marker.
fn marker_arity(marker: &str) -> Option<usize> {
    Some(match marker {
        "new" => 0,
        "len" | "clear" | "keys" | "values" => 1,
        "has" | "get" | "getmut" | "try_get" | "try_getmut" | "del" => 2,
        "set" => 3,
        _ => return None,
    })
}

/// Which argument of a marker is the key, for the markers that take one.
fn key_argument(marker: &str) -> Option<usize> {
    match marker {
        "has" | "set" | "get" | "getmut" | "try_get" | "try_getmut" | "del" => Some(1),
        _ => None,
    }
}

/// Why `key_ty` cannot be a key, or `None` when it can.
///
/// The type asked about is the marker's own argument, so it is a reference to the key's class; the
/// class is what a host `Map` ends up comparing.
fn key_class_error<'tcx>(key_ty: Ty<'tcx>) -> Option<String> {
    let ty::Ref(_, class, _) = key_ty.kind() else {
        return Some(format!(
            "a `{MAP_PREFIX}` marker takes its key by reference, and `{key_ty}` is not one"
        ));
    };
    match class.kind() {
        // Every one of these is a JavaScript value a `Map` compares by value: a number, a BigInt
        // for the widths above 53 bits, a boolean, a string.
        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Str => None,
        ty::Float(_) => Some(format!(
            "`{class}` cannot be a map key: `core` gives the floats no `Eq` for the reason that \
             `NaN != NaN`, and a host `Map` answers a third way again"
        )),
        _ => Some(format!(
            "`{class}` cannot be a map key: a host `Map` compares keys with SameValueZero, which \
             is identity for anything that is not an integer, a `bool`, a `char` or a `str`, so \
             an equal key built later would never find this entry"
        )),
    }
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers a call to one of the `mires-js` collection markers.
    pub(crate) fn codegen_map_marker(
        &self,
        marker: &str,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let Some(arity) = marker_arity(marker) else {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "`{MAP_PREFIX}{marker}` is not a `mires-js` collection marker this backend knows"
            )))];
        };
        if args.len() != arity {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "`{MAP_PREFIX}{marker}` expects {arity} arguments, got {}",
                args.len()
            )))];
        }
        let key = key_argument(marker);

        let mut lowered: Vec<Expr> = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let value = self.at(arg.span, || self.codegen_operand(&arg.node));
            if Some(index) != key {
                lowered.push(value);
                continue;
            }
            let ty = self.operand_ty(&arg.node);
            if let Some(why) = key_class_error(ty) {
                return vec![jsast::expr_stmt(self.at(arg.span, || self.zombie(why)))];
            }
            // A reference to a place the value model keeps in a slot is the slot record, and the
            // host wants what is in it. The operand is a local or a constant, so reading it twice
            // is free and has no effect to repeat.
            lowered.push(match crate::ptr::is_slot(self.tcx, ty) {
                true => crate::ptr::slot_element(value),
                false => value,
            });
        }

        let (_, ty) = self.codegen_place(destination);
        let mut iter = lowered.into_iter();
        let mut stmts = Vec::new();
        let expr = match marker {
            "new" => jsast::new(jsast::id("Map"), Vec::new()),
            "len" => jsast::member(iter.next().unwrap(), "size"),
            "clear" => jsast::call(jsast::member(iter.next().unwrap(), "clear"), Vec::new()),
            "has" => {
                let map = iter.next().unwrap();
                let k = iter.next().unwrap();
                jsast::call(jsast::member(map, "has"), vec![k])
            }
            "get" | "getmut" => {
                let map = iter.next().unwrap();
                let k = iter.next().unwrap();
                jsast::call(jsast::member(map, "get"), vec![k])
            }
            "set" => {
                let map = iter.next().unwrap();
                let k = iter.next().unwrap();
                let v = iter.next().unwrap();
                jsast::call(jsast::member(map, "set"), vec![k, v])
            }
            "del" => {
                let map = iter.next().unwrap();
                let k = iter.next().unwrap();
                jsast::call(jsast::member(map, "delete"), vec![k])
            }
            "keys" => jsast::rt_call("map_keys", vec![iter.next().unwrap()]),
            "values" => jsast::rt_call("map_values", vec![iter.next().unwrap()]),
            "try_get" | "try_getmut" => {
                let map = iter.next().unwrap();
                let k = iter.next().unwrap();
                let get_call = jsast::call(jsast::member(map, "get"), vec![k]);
                let peeled =
                    crate::value::peel_pattern(crate::value::peel_transparent(self.tcx, ty));
                let (none_idx, some_idx, some_field) = match peeled.kind() {
                    ty::Adt(def, _) if def.is_enum() && def.variants().len() == 2 => {
                        let v0 = VariantIdx::from_u32(0);
                        let v1 = VariantIdx::from_u32(1);
                        if def.variant(v0).fields.is_empty() && def.variant(v1).fields.len() == 1 {
                            (
                                v0,
                                v1,
                                crate::naming::Namer::field_name(def.variant(v1), 0u32.into()),
                            )
                        } else if def.variant(v1).fields.is_empty()
                            && def.variant(v0).fields.len() == 1
                        {
                            (
                                v1,
                                v0,
                                crate::naming::Namer::field_name(def.variant(v0), 0u32.into()),
                            )
                        } else {
                            return vec![jsast::expr_stmt(self.zombie(format!(
                                "`{MAP_PREFIX}{marker}` must return an `Option`-shaped enum, got `{ty}`"
                            )))];
                        }
                    }
                    _ => {
                        return vec![jsast::expr_stmt(self.zombie(format!(
                            "`{MAP_PREFIX}{marker}` must return an `Option`-shaped enum, got `{ty}`"
                        )))];
                    }
                };
                let (assign, tmp) = self.temp(get_call);
                stmts.push(assign);
                let Some(none_expr) = crate::value::enum_value(self.tcx, ty, none_idx, Vec::new())
                else {
                    return vec![jsast::expr_stmt(self.zombie(format!(
                        "`{MAP_PREFIX}{marker}` could not build `None` for `{ty}`"
                    )))];
                };
                let Some(some_expr) = crate::value::enum_value(
                    self.tcx,
                    ty,
                    some_idx,
                    vec![(some_field, tmp.clone())],
                ) else {
                    return vec![jsast::expr_stmt(self.zombie(format!(
                        "`{MAP_PREFIX}{marker}` could not build `Some` for `{ty}`"
                    )))];
                };
                jsast::cond(
                    jsast::binary(BinOp::StrictEq, tmp, Expr::Undefined),
                    none_expr,
                    some_expr,
                )
            }
            _ => unreachable!(),
        };
        // A marker that answers nothing is a statement. Writing it to the destination would be a
        // store to a local of unit type, which is a value the model does not have.
        match crate::value::is_zst(self.tcx, ty) {
            true => stmts.push(jsast::expr_stmt(expr)),
            false => stmts.extend(self.write_place(destination, expr)),
        }
        stmts
    }
}
