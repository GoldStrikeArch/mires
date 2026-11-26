//! Shared wire formats for `js!{}` blocks and `#[js_extern]` descriptors.
//!
//! Zero external dependencies so proc-macro crates (`js-macro`, `js-extern-macro`),
//! the rustc backend, and `jsc-link` share one canonical encoder/decoder without
//! pulling heavy crates into host proc-macro builds.

pub mod block;
pub mod descriptor;
