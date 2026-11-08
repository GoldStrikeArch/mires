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

/// The `js!{}` block section format, shared verbatim with the `js-macro` crate that writes it.
#[path = "../../js-macro/src/block.rs"]
mod block;
/// The `#[js_extern]` descriptor format, shared verbatim with the macro crate that writes it.
#[path = "../../js-extern-macro/src/descriptor.rs"]
mod descriptor;

mod analysis;
mod link;
mod link_dedup;
mod lower;
mod opt;
mod plan;
mod report;

/// The shared crate's modules, under the names the moved code already spells them by.
pub(crate) use rcgjs_object::{
    baseline_shadow, dts, item, jir, jsast, lane_report, late_ir, manifest, minify, opts, ptr,
    sourcemap,
};

pub use link::run;
