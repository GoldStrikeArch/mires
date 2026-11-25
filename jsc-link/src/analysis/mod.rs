//! Whole-function analyses the lane's plans and passes read, over JIR-R.
//!
//! Nothing here rewrites: an analysis answers a question about a [`crate::jir::repr::Func`] and
//! the caller decides what to do with the answer. They live outside `opt/` because a *plan*
//! (`crate::plan`) reads them too, and a plan runs before any pass does.

pub mod alloc_sites;
pub mod arena;
pub mod ascii;
pub mod byte_views;
pub mod callgraph;
pub mod effects;
pub mod heap;
pub mod host_contract;
pub mod js_alloc;
pub mod legality;
pub mod param_use;
pub mod refs;
pub mod summary_check;
pub mod valueflow;
