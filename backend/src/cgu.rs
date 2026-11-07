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

use std::cell::RefCell;
use std::collections::BTreeSet;

use rustc_middle::ty::TyCtxt;

use crate::item::{ItemKind, JsItem, JsName, Linkage, SourceOrder};
use crate::jsast::{self, Expr};
use crate::naming::Namer;
use crate::opts::Opts;

pub(crate) struct CguCx<'tcx> {
    pub(crate) tcx: TyCtxt<'tcx>,
    pub(crate) namer: Namer<'tcx>,
    /// Read by the lowering milestones that the flags in `opts.rs` are there for.
    #[allow(dead_code)]
    pub(crate) opts: &'static Opts,
    /// Items discovered while lowering, appended to the unit after the mono item walk.
    extra: RefCell<Vec<JsItem>>,
    /// The names already emitted into this unit, so an interned item is built exactly once.
    emitted: RefCell<BTreeSet<JsName>>,
    /// How many keyed loops have been numbered in this unit. See [`CguCx::next_keyed_site`].
    keyed_sites: RefCell<u32>,
}

impl<'tcx> CguCx<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>, opts: &'static Opts) -> CguCx<'tcx> {
        CguCx {
            tcx,
            namer: Namer::new(tcx, opts.names),
            opts,
            extra: RefCell::new(Vec::new()),
            emitted: RefCell::new(BTreeSet::new()),
            keyed_sites: RefCell::new(0),
        }
    }

    /// The next keyed loop's site, which is what separates one keyed loop's row cache from
    /// another's at run time.
    ///
    /// Per `push_keyed` call site, and a keyed `for` contains exactly one of those, so per call
    /// site is per loop. The counter is per codegen unit, so the CRATE NAME is part of the site:
    /// every unit would otherwise start at zero and two loops compiled in different crates would
    /// share one row cache, which is a wrong-rows bug rather than a slow one.
    pub(crate) fn next_keyed_site(&self) -> String {
        let mut sites = self.keyed_sites.borrow_mut();
        let site = *sites;
        *sites += 1;
        format!("{}#{site}", self.tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE))
    }

    /// Records that `name` has an item in this unit. Returns `false` if it already had one.
    pub(crate) fn claim(&self, name: &JsName) -> bool {
        self.emitted.borrow_mut().insert(name.clone())
    }

    /// Moves an already interned item to `order` if that is earlier than where it is now.
    ///
    /// An item whose declaration order matters is interned by the first *codegenned* use, and
    /// codegen order is a symbol-hash order. The reference compiler declares one per template in
    /// the order the module first *writes* them, so every use offers its own position and the
    /// earliest wins. See [`crate::item::SourceOrder`].
    ///
    /// Only uses in this codegen unit can offer one: two units that build the same template each
    /// emit an item, and `link.rs` keeps one of the two. With one unit per crate, which is what the
    /// suites compile with, that is every use in the crate.
    pub(crate) fn order_at_most(&self, name: &JsName, order: SourceOrder) {
        for item in self.extra.borrow_mut().iter_mut() {
            if item.name != *name {
                continue;
            }
            if item.order.as_ref().is_none_or(|current| order < *current) {
                item.order = Some(order);
            }
            return;
        }
    }

    /// Adds a discovered item, unless one of that name is already there.
    #[allow(dead_code)]
    pub(crate) fn push_item(&self, item: JsItem) {
        if self.claim(&item.name) {
            self.extra.borrow_mut().push(item);
        }
    }

    /// Builds the item named `name` if this unit does not have it yet.
    ///
    /// The name is the interning key: two coercions to the same `dyn Trait` from the same type
    /// name the same vtable, so the second one costs nothing.
    #[allow(dead_code)]
    pub(crate) fn intern(&self, name: JsName, build: impl FnOnce() -> JsItem) {
        if self.claim(&name) {
            let item = build();
            debug_assert_eq!(item.name, name, "`intern` built an item under a different name");
            self.extra.borrow_mut().push(item);
        }
    }

    /// Hoists a value the program repeats into a module level `const`, and names it.
    ///
    /// The same value is emitted once per program however many times it is written: `key` is the
    /// interning key, and it goes through [`Namer::synthetic_name`], whose hash is stable across
    /// crates and compilations. Two crates that hoist the same value therefore produce items with
    /// the same name *and* the same text, which is exactly the shape the duplicate check in
    /// `link.rs` drops silently. Nothing has to record the edge from a use to the constant either:
    /// the use is the constant's identifier, so `JsItem::new` picks it up as an outgoing reference
    /// and the reachability pass keeps it alive — and drops it when the last user goes.
    ///
    /// `key` must describe the *value*, never a `def_path_str` (see the note in `naming.rs`), or
    /// two crates would disagree about the name.
    ///
    /// With `-Cllvm-args=js-hoist-consts=off` the value is returned inline instead, which is what
    /// makes a size or behaviour change from this bisectable.
    pub(crate) fn hoist(
        &self,
        prefix: &str,
        key: &str,
        debug_path: String,
        value: impl FnOnce() -> Expr,
    ) -> Expr {
        if !self.opts.hoist_consts {
            return value();
        }
        let name = self.namer.synthetic_name(prefix, key);
        self.intern(name.clone(), || {
            JsItem::new(
                name.clone(),
                ItemKind::Const,
                jsast::const_(name.as_str(), value()),
                Vec::new(),
                Linkage::internal(),
                debug_path,
            )
        });
        jsast::id(name.into_string())
    }

    /// Takes the discovered items.
    pub(crate) fn take_extra(&self) -> Vec<JsItem> {
        std::mem::take(&mut self.extra.borrow_mut())
    }
}
