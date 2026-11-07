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

use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::jsast::{self, Expr, Stmt};

/// The `link_section` prefix every `mires-js` collection marker carries.
pub(crate) const MAP_PREFIX: &str = "rcgjs.map.";

/// The marker `def_id` is, by the `link_section` it carries.
pub(crate) fn map_section(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(MAP_PREFIX).map(str::to_owned)
}

/// The `__rt` member a marker becomes.
///
/// `get` and `getmut` are one member: the two differ in the Rust type they hand back and not in
/// what the host does, and a `Map` has one read.
fn rt_member(marker: &str) -> Option<&'static str> {
    Some(match marker {
        "has" => "map_has",
        "set" => "map_set",
        "get" | "getmut" => "map_get",
        "del" => "map_del",
        "keys" => "map_keys",
        _ => return None,
    })
}

/// Which argument of a marker is the key, for the markers that take one.
fn key_argument(marker: &str) -> Option<usize> {
    match marker {
        "has" | "set" | "get" | "getmut" | "del" => Some(1),
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
        let Some(member) = rt_member(marker) else {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "`{MAP_PREFIX}{marker}` is not a `mires-js` collection marker this backend knows"
            )))];
        };
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

        let call = jsast::rt_call(member, lowered);
        // A marker that answers nothing is a statement. Writing it to the destination would be a
        // store to a local of unit type, which is a value the model does not have.
        let (_, ty) = self.codegen_place(destination);
        match crate::value::is_zst(self.tcx, ty) {
            true => vec![jsast::expr_stmt(call)],
            false => self.write_place(destination, call),
        }
    }
}
