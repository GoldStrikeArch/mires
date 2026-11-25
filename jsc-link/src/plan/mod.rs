//! Whole-program plans the lane driver makes once, before it optimizes or prints anything.
//!
//! A pass rewrites one function from what that function says. A plan decides something no single
//! function can decide on its own — here, the calling convention of an item, which is a fact about
//! every reference to it in the program — and then rewrites both ends of it. The lane driver runs
//! the plans between phase A (build every `Func`) and phase C (optimize and print each one); see
//! `link/lane.rs`.

pub mod abi_flat;
pub mod async_graph;
pub(crate) mod async_ready;
pub mod bstr_abi;
pub mod cost;
pub mod enum_repr;
pub mod graph;
pub(crate) mod lowering_reuse;
pub mod mincut;
pub mod physical;
pub mod physical_cost;
pub mod representation;
pub mod soa_vec;
pub mod versions;
