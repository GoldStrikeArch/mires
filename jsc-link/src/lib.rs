//! `jsc-link`: the consumer half of `rustc_codegen_js`.
//!
//! Everything from reading object files to writing the finished program lives here — the graph
//! walk, deduplication, chunking, the source map, the `.d.ts`, the optimization report, and the
//! JIR optimizer. None of it needs a compiler: an object file carries baseline JavaScript, a JIR-S
//! body and a summary, and the schema for all three is
//! [`rcgjs_object`], which is rustc-free by construction.
//!
//! That is the whole point of the split. The linker and the optimizer are a normal cargo crate
//! with a normal `cargo test`, and the dylib is only a producer: MIR in, objects out. The seam is
//! the [`rcgjs_object::manifest`], which the dylib writes before it execs the `jsc-link` binary.

pub mod diag;
pub mod runtime_emit;

/// The `js!{}` block and `#[js_extern]` descriptor formats, shared via `mires-marker-abi`.
pub(crate) use mires_marker_abi::{block, descriptor};

mod analysis;
mod link;
mod link_dedup;
mod lower;
mod opt;
mod plan;
mod profile;
mod report;

/// The shared crate's modules, under the names the moved code already spells them by.
pub(crate) use rcgjs_object::{
    dts, item, jir, jsast, lane_report, manifest, minify, opts, ptr, sourcemap,
};

pub use link::run;
