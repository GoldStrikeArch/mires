//! Vtables: the `meta` half of a `&dyn Trait` fat pointer.
//!
//! A vtable is a module level JavaScript **array**, one element per entry of
//! `tcx.vtable_entries`, in exactly that order:
//!
//! ```js
//! const vtable$Square$Shape$h... = [null, 4, 4, Square$area$h..., Shape$sides$h..., ...];
//! //                                 ^drop ^size ^align ^ methods, in vtable order
//! ```
//!
//! Keeping rustc's own slot numbering is what makes a virtual call a single index:
//! `InstanceKind::Virtual(_, idx)` carries an **absolute** slot index — `first_method_vtable_slot`
//! (rustc_trait_selection/src/traits/vtable.rs:312-364) starts its running offset at
//! `TyCtxt::COMMON_VTABLE_ENTRIES.len()`, so `idx` already counts the three header slots, exactly
//! like `rustc_codegen_cranelift`'s `get_ptr_and_method_ref` loading `vtable[idx * ptr_size]`. The
//! same holds for the upcast slot `tcx.supertrait_vtable_slot` returns, which `unsize.rs` uses.
//!
//! The three header slots are the ones `rustc_middle::ty::vtable` writes: slot 0 the drop glue (or
//! `null` when the type needs none), slot 1 the size in bytes, slot 2 the alignment.
//!
//! Vtables are interned per `(ty, trait_ref)` through [`CguCx::intern`], so two coercions of the
//! same type to the same trait share one array. Because the array literal mentions the method
//! functions as plain identifiers, `JsItem::new` picks them up as outgoing references
//! automatically, and the reachability pass in `link.rs` keeps them alive without anything having
//! to record that edge by hand.

use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, ExistentialTraitRef, Instance, Ty, TyCtxt, VtblEntry};
use rustc_span::{DUMMY_SP, Span};

use crate::cgu::CguCx;
use crate::item::{ItemKind, JsItem, JsName, Linkage, ZombieKind, ZombieLog};
use crate::jsast::{self, Expr};
use crate::value::typing_env;

/// The vtable for `ty as dyn Trait`, as the expression that names it.
///
/// Interns the array on first use; later calls only produce the identifier.
pub(crate) fn vtable_expr<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    trait_ref: Option<ExistentialTraitRef<'tcx>>,
) -> Expr {
    let name = cgu.namer.vtable_name(ty, trait_ref);
    cgu.intern(name.clone(), || build(cgu, ty, trait_ref, name.clone()));
    jsast::id(name.into_string())
}

/// Builds the vtable item itself.
fn build<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    trait_ref: Option<ExistentialTraitRef<'tcx>>,
    name: JsName,
) -> JsItem {
    let tcx = cgu.tcx;
    let span = trait_ref.map_or(DUMMY_SP, |trait_ref| tcx.def_span(trait_ref.def_id));
    let zombies = ZombieLog::default();

    let entries: &[VtblEntry<'tcx>] = match trait_ref {
        Some(trait_ref) => {
            let trait_ref = trait_ref.with_self_ty(tcx, ty);
            tcx.vtable_entries(tcx.erase_and_anonymize_regions(trait_ref))
        }
        // A `dyn Trait` with no principal (`dyn Send`) still carries the header.
        None => TyCtxt::COMMON_VTABLE_ENTRIES,
    };

    let (size, align) = size_and_align(cgu, &zombies, span, ty);
    let slots = entries
        .iter()
        .map(|entry| match *entry {
            VtblEntry::MetadataDropInPlace => drop_slot(cgu, ty),
            VtblEntry::MetadataSize => jsast::num(size as f64),
            VtblEntry::MetadataAlign => jsast::num(align as f64),
            // A method the trait object cannot dispatch. Calling it is impossible, so the slot
            // only has to exist and keep the numbering right.
            VtblEntry::Vacant => jsast::null(),
            // `vtable_entries` hands back an already resolved instance: a default method reached
            // through `dyn` is the trait's own body instantiated with `Self = dyn Trait`, and
            // re-resolving it here would lose that and call the trait body on a concrete type.
            VtblEntry::Method(instance) => jsast::id(cgu.namer.fn_name(instance).into_string()),
            // A supertrait vtable, for upcasting. Interned like any other.
            VtblEntry::TraitVPtr(super_trait_ref) => {
                let principal = ty::ExistentialTraitRef::erase_self_ty(tcx, super_trait_ref);
                vtable_expr(cgu, ty, Some(principal))
            }
        })
        .collect();

    let debug_path = with_no_trimmed_paths!(match trait_ref {
        Some(trait_ref) => format!("<{ty} as {trait_ref}> vtable"),
        None => format!("<{ty}> vtable"),
    });

    JsItem::new(
        name.clone(),
        ItemKind::Vtable,
        jsast::const_(name.into_string(), jsast::array(slots)),
        zombies.take(),
        Linkage::internal(),
        debug_path,
    )
}

/// Slot 0: the drop glue of the concrete type, or `null` when it has none.
///
/// `abi.rs` null checks the slot before calling it, which is what makes dropping a `dyn Trait`
/// whose concrete type is trivially droppable a no-op rather than a call through `null`.
fn drop_slot<'tcx>(cgu: &CguCx<'tcx>, ty: Ty<'tcx>) -> Expr {
    let tcx = cgu.tcx;
    if !ty.needs_drop(tcx, typing_env()) {
        return jsast::null();
    }
    let instance = Instance::resolve_drop_glue(tcx, ty);
    match instance.def {
        // Empty glue is not emitted as an item, so naming it would dangle.
        ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => jsast::null(),
        _ => jsast::id(cgu.namer.fn_name(instance).into_string()),
    }
}

/// Slots 1 and 2: the size and alignment of the concrete type, in bytes.
fn size_and_align<'tcx>(
    cgu: &CguCx<'tcx>,
    zombies: &ZombieLog,
    span: Span,
    ty: Ty<'tcx>,
) -> (u64, u64) {
    match cgu.tcx.layout_of(typing_env().as_query_input(ty)) {
        Ok(layout) => (layout.size.bytes(), layout.align.abi.bytes()),
        Err(err) => {
            zombies.record(
                cgu.tcx,
                span,
                ZombieKind::Internal,
                format!("cannot lay out `{ty}` for its vtable ({err})"),
            );
            (0, 1)
        }
    }
}
