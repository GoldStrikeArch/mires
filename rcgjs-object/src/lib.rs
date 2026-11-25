//! `rcgjs-object`: the rustc-free schema shared by the `rustc_codegen_js` producer and the
//! `jsc-link` consumer.
//!
//! # JIR-S
//!
//! JIR-S is the **semantic** intermediate representation of `rustc_codegen_js` v2: an owned,
//! versioned, MIR-shaped, representation-neutral body per monomorphic item
//! ([`jir::semantic::Body`]), together with the crate-scope tables it refers into — the
//! [`jir::types::TypeTable`] of type descriptors, the [`jir::interns::Interns`] of strings, byte
//! blobs, signatures and source facts — and, beside it, the per-body [`jir::origin::OriginTable`].
//! The producer (the codegen dylib) encodes it from MIR; the consumer lowers it to JavaScript at
//! link time without rustc.
//!
//! # Semantic versus diagnostic
//!
//! Everything in [`jir::semantic`] and [`jir::types`] is semantic: it decides what the program does
//! and takes part in body equality (`Body: Eq`) and, downstream, in the body hash the linker
//! deduplicates on. It carries stable identities only — definition hashes, type id hashes,
//! interned ids — never paths, spans, or printed text, so two crates that describe one monomorphic
//! item through different paths produce equal bodies.
//!
//! Everything in [`jir::origin`] is diagnostic: source facts, local names, inline stacks, item
//! paths. It feeds source maps, warnings, reports and the debugger, is stored separately, and never
//! influences equality, hashing, or lowering.
//!
//! [`jir::decline`] is the one vocabulary of reasons a body, type, operation, or optimization is
//! not taken further; its `Display` reproduces v1's report strings.
//!
//! # What else lives here
//!
//! Beyond the schema, this crate holds every module of the compiler that never needed rustc: the
//! JavaScript AST and its printer ([`jsast`]), the minifier ([`minify`]), the item-local AST passes
//! ([`optimizer`], [`sroa`]), the object file's item model and reader/writer ([`item`]), the source
//! map builder ([`sourcemap`]), the `.d.ts` writer ([`dts`]), the frozen v1 late IR schema
//! ([`late_ir`]) and the parsed `-Cllvm-args` options ([`opts`]). That is what makes `jsc-link` —
//! the consumer that turns a set of objects into a finished program — a plain cargo crate with a
//! plain `cargo test`, and it is why this crate, not the backend, is the shared contract.
//!
//! # Append-only rule
//!
//! Bodies are serialized with postcard, which writes an enum variant as its index. Every `enum` in
//! this crate is therefore **append-only**: a variant may be added at the end; none may be
//! reordered, removed, or have its payload changed. A change that cannot be expressed that way
//! bumps [`jir::JIR_SCHEMA`], and objects written under another schema are rejected, never
//! reinterpreted. Struct fields are positional too: add new fields at the end of a struct only with
//! a schema bump (postcard has no field names to skip by).

pub mod artifact;
pub mod dts;
pub mod evaluation;
pub mod host;
pub mod item;
pub mod jir;
pub mod jsast;
pub mod lane_report;
pub mod manifest;
pub mod minify;
pub mod object;
pub mod optimizer;
pub mod opts;
pub mod ptr;
pub mod sourcemap;
pub mod sroa;
