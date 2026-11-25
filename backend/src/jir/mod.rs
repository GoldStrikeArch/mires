//! JIR-S production: the tcx side of the `rcgjs-object` schema.
//!
//! [`encode`] turns MIR bodies, statics, and vtables into verified [`Body`]s with their
//! [`OriginTable`]s; [`types`] fills the codegen unit's crate-scope type table. The unit's
//! [`rcgjs_object::jir::Interns`] live on [`crate::cgu::CguCx`] and accumulate across items.
//!
//! [`Body`]: rcgjs_object::jir::semantic::Body
//! [`OriginTable`]: rcgjs_object::jir::origin::OriginTable

pub(crate) mod classify;
pub(crate) mod encode;
pub(crate) mod types;

pub(crate) use rcgjs_object::jir::JirItem;
