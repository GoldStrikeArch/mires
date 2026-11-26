//! Per codegen unit state shared by every function lowered into it.
//!
//! A `CguCx` is the context above [`crate::base::FnCx`]: it owns the [`Namer`], and it is the sink
//! for items that are *discovered* during lowering rather than listed by the codegen unit — a
//! vtable interned the first time some `ty as dyn Trait` coercion needs it, say. Those items are
//! collected here and appended to the unit's item list once every mono item has been walked.
//!
//! Everything on it is `&self`: `FnCx` is `&self` throughout (some sixty call sites), so a `&mut`
//! path from a lowering method up to the unit would mean rewriting all of them. Interior
//! mutability is confined to the two sinks below.

use std::cell::{Ref, RefCell};
use std::collections::BTreeSet;

use rcgjs_object::jir::Interns;

use rustc_middle::ty::TyCtxt;

use crate::item::{JsItem, JsName};
use crate::naming::Namer;
use crate::opts::Opts;

pub(crate) struct CguCx<'tcx> {
    pub(crate) tcx: TyCtxt<'tcx>,
    pub(crate) namer: Namer<'tcx>,
    /// Items discovered while lowering, appended to the unit after the mono item walk.
    extra: RefCell<Vec<JsItem>>,
    /// The names already emitted into this unit, so an interned item is built exactly once.
    emitted: RefCell<BTreeSet<JsName>>,
    /// The crate-scope JIR-S tables (strings, bytes, signatures, source facts, the type table)
    /// every body of this unit refers into. See [`CguCx::interns`].
    interns: RefCell<Interns>,
}

impl<'tcx> CguCx<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>, opts: &'static Opts) -> CguCx<'tcx> {
        CguCx {
            tcx,
            namer: Namer::new(tcx, opts.names),
            extra: RefCell::new(Vec::new()),
            emitted: RefCell::new(BTreeSet::new()),
            interns: RefCell::new(Interns::new()),
        }
    }

    /// The JIR-S interns of this unit, accumulated across every item encoded into it.
    ///
    /// Ids are per unit: two units intern the same string under different ids, and the object
    /// writer stores the unit's tables beside its bodies so the linker can re-intern. Within one
    /// unit the ids are deterministic, because items are encoded in
    /// `items_in_deterministic_order`. The encoder takes the `RefCell` mutably while it encodes
    /// one item; nothing else may hold this borrow across an encode.
    pub(crate) fn interns(&self) -> Ref<'_, Interns> {
        self.interns.borrow()
    }

    /// Mutable access for the encoder. Never held across a call back into lowering.
    pub(crate) fn interns_mut(&self) -> std::cell::RefMut<'_, Interns> {
        self.interns.borrow_mut()
    }

    /// Takes the unit's interns, leaving empty tables behind.
    pub(crate) fn take_interns(&self) -> Interns {
        std::mem::take(&mut self.interns.borrow_mut())
    }

    /// Records that `name` has an item in this unit. Returns `false` if it already had one.
    pub(crate) fn claim(&self, name: &JsName) -> bool {
        self.emitted.borrow_mut().insert(name.clone())
    }

    /// Builds the item named `name` if this unit does not have it yet.
    ///
    /// The name is the interning key: two coercions to the same `dyn Trait` from the same type
    /// name the same vtable, so the second one costs nothing.
    pub(crate) fn intern(&self, name: JsName, build: impl FnOnce() -> JsItem) {
        if self.claim(&name) {
            let item = build();
            debug_assert_eq!(
                item.name, name,
                "`intern` built an item under a different name"
            );
            self.extra.borrow_mut().push(item);
        }
    }

    /// Takes the discovered items.
    pub(crate) fn take_extra(&self) -> Vec<JsItem> {
        std::mem::take(&mut self.extra.borrow_mut())
    }
}
