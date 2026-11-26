//! The compiler-facing half of `item.rs`.
//!
//! Everything an item *is* — [`JsItem`], [`LinkItem`], the object reader and writer, the name and
//! linkage types — lives in [`rcgjs_object::item`], because `jsc-link` reads and writes the very
//! same objects without rustc. What is left here is the seam to the compiler: recording a zombie
//! from a `Span`, and reading this session's object identity off the `Session`.

use rustc_middle::ty::TyCtxt;
use rustc_session::Session;
use rustc_span::Span;

pub(crate) use rcgjs_object::item::*;

use crate::jsast::{self, Expr};

/// A place to record zombies from code that has no [`crate::base::FnCx`] — constants, statics.
#[derive(Default)]
pub(crate) struct ZombieLog {
    list: std::cell::RefCell<Vec<Zombie>>,
}

impl ZombieLog {
    /// Records a zombie and returns the poison expression that stands in for the value.
    pub(crate) fn record(
        &self,
        tcx: TyCtxt<'_>,
        span: Span,
        kind: ZombieKind,
        message: String,
    ) -> Expr {
        let loc = tcx.sess.source_map().span_to_diagnostic_string(span);
        self.list.borrow_mut().push(Zombie {
            loc,
            message: message.clone(),
            span: Some(span_key(span)),
            kind,
        });
        jsast::rt_call("js_abort", vec![jsast::string(message)])
    }

    pub(crate) fn take(&self) -> Vec<Zombie> {
        std::mem::take(&mut self.list.borrow_mut())
    }
}

/// A span, projected onto the plain numbers [`SpanKey`] carries.
fn span_key(span: Span) -> SpanKey {
    let data = span.data();
    (
        data.lo.0,
        data.hi.0,
        data.parent.map(|parent| parent.local_def_index.as_u32()),
    )
}

/// The identity of the objects this session writes.
pub(crate) fn object_identity(sess: &Session) -> ObjectIdentity {
    ObjectIdentity {
        rustc_version_hash: Hash128::of(sess.cfg_version.as_bytes()).fold(),
        data_layout: sess.target.data_layout.to_string(),
        emit_args: normalized_emit_args(&sess.opts.cg.llvm_args),
    }
}
