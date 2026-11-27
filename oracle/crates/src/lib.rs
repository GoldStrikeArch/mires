//! Shared crate root for the Mires crates.io differential oracle.
//!
//! Each real-world crate is exercised in its own integration test target under
//! `tests/<crate>.rs` so it compiles, executes, and diffs independently under
//! `cargo mires test --diff`.
