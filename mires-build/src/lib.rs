//! Compiles a client crate to JavaScript from a Cargo build script.
//!
//! This is a wrapper around the `rustc_codegen_mires` backend and the
//! `scripts/compile_core.sh` script that drives it. A build script points it
//! at one `.rs` file and gets a program, a source map, and the runtime shim in
//! `$OUT_DIR`:
//!
//! ```rust,no_run
//! mires_build::BuildConfig::new("client/app.rs")
//!     .render()
//!     .unwrap();
//! ```
//!
//! The compiled program can then be served like any other generated file, for
//! example `asset!(concat!(env!("OUT_DIR"), "/app.js"))`.
//!
//! # Setup
//!
//! The pinned nightly, the backend built with it, and the `core` it compiles
//! against are set up once per checkout, and take minutes:
//!
//! ```sh
//! rustup toolchain install "$(...)"           # the nightly the backend loads into
//! scripts/build.sh                            # the codegen backend
//! JS_EXTRA_ARGS='' scripts/build_sysroot.sh   # `core`, compiled by it
//! ```
//!
//! Until they are all there, [`BuildConfig::render`] reports the first one that
//! is missing and repeats the command that builds it. An unattended machine can
//! run them itself with [`BuildConfig::bootstrap`].
//!
//! [`Toolchain::preconditions`] is that list of steps, and
//! [`Toolchain::apply`] performs one, so a tool outside a build script can set
//! the checkout up without a second opinion about when it is ready.
//!
//! The `JS_EXTRA_ARGS` the sysroot was built with are recorded next to it. Any
//! backend option that changes how items are spelled has to be the same on
//! both sides, so [`BuildConfig::emit_arg`] is checked against that record and
//! refuses a sysroot that disagrees. [`BuildConfig::crate_arg`] carries the
//! options that only change this crate's own output, which need no rebuild.
//!
//! # The client crate
//!
//! The client crate is not a Cargo target: rustc is handed a single root file,
//! at the edition the script fixes (2021). It has these constraints.
//!
//! * Extra modules are reached with `#[path = "..."] mod name;`, since there is no Cargo target
//!   directory layout to infer them from.
//! * The crate is `#![no_std]` and `#![no_main]` and defines its own `#[panic_handler]`.
//! * Only `core` is available. There is no `alloc`, so no `Box`, `Vec`, or `String`.
//! * `#[no_mangle]` functions are the exports. They are also the roots for dead code elimination,
//!   which drops everything they do not reach, and their names appear in the JavaScript exactly as
//!   written. Four characters or more keeps them clear of the short names minification generates.
//! * An `extern "C"` function has no body to compile, so a call to one lowers to
//!   `__rt.<name>(...)`. The page has to define `__rt`, which is what the published `shim.js` does
//!   for the names it knows.
//!
//! # Macros
//!
//! The backend cannot run a proc macro, so a crate that uses one is compiled in
//! two passes: plain rustc expands the macros and resolves the `--cfg` flags,
//! and the backend compiles the expansion. [`BuildConfig::js_extern`] asks for
//! the `#[js_extern]` macro this checkout owns, and [`BuildConfig::proc_macro`]
//! names any other macro, built for the host by whoever owns it. A `no_std`
//! crate the client links against, such as the template ABI, is compiled to an
//! rlib by the same backend first with [`BuildConfig::rlib`] (or
//! [`BuildConfig::view_abi`] for that one), so the expansion and the program
//! agree about it.
//!
//! # Chunks
//!
//! A page with several islands does not have to load all of them out of one
//! file. [`BuildConfig::chunks`] asks for one file per island, plus a shared
//! file holding what two or more of them reach:
//!
//! ```rust,no_run
//! use mires_build::{BuildConfig, Chunking};
//!
//! let artifacts = BuildConfig::new("client/islands.rs")
//!     .output_name("islands.js")
//!     .js_extern(true)
//!     .view_abi(true)
//!     .crate_arg("js-modules=esm")
//!     .chunks(
//!         Chunking::new()
//!             .island("counter")
//!             .island("search")
//!             .url_base("/demo/chunks/")
//!             .import("mires-dom", "/demo/mires-dom.js"),
//!     )
//!     .render()
//!     .unwrap();
//! ```
//!
//! [`Artifacts::chunks`] names each emitted file, and they all land in one
//! directory inside `OUT_DIR`, so a single route serving that directory serves
//! the whole program. Splitting is a link step decision, so it costs no sysroot
//! rebuild.
//!
//! A chunk is imported by a bare specifier, its prefix and its name, which the
//! page's import map resolves to the URL the chunk is served from.
//! [`Chunking::url_base`] says where that is and turns the map on;
//! [`render`](BuildConfig::render) writes it beside the program as
//! `<name>.importmap.json`, ready to be the body of a
//! `<script type="importmap">`:
//!
//! ```text
//! const IMPORT_MAP: &str = include_str!(concat!(env!("OUT_DIR"), "/islands.importmap.json"));
//! ```
//!
//! Whatever else the page resolves goes in the same map through
//! [`Chunking::import`], which is how the runtime the islands import gets its
//! entry.
//!
//! Whether the program is really split is up to the backend. One that cannot
//! answers with a single file, and every requested chunk then names it: the
//! directory, the URLs, and the import map are the ones a split program has, so
//! the page and the loader do not change when the backend learns to split. What
//! the backend is asked for, and what it writes when it obliges, is on
//! [`Chunking`].
//!
//! # Rebuilds
//!
//! [`BuildConfig::render`] emits `cargo:rerun-if-changed` for the client
//! crate's sources and for the toolchain files that decide what they compile
//! to. A module the crate reaches with `#[path]` out of its own directory
//! counts as a source, which is how an island shared with the server half gets
//! watched. Emitting any such directive replaces cargo's default rule of
//! rerunning on any change in the package, which is deliberate: editing the
//! server half of an app no longer recompiles the client half. Output is
//! published only when its bytes change, so a recompile that lands on the same
//! JavaScript does not look like an edit to anything watching `$OUT_DIR`.

mod chunk;
mod config;
mod error;
mod pinned;
mod preconditions;
mod proc_macro;
mod rerun;
mod shimcheck;
mod toolchain;

pub use chunk::*;
pub use config::*;
pub use error::*;
pub use pinned::*;
pub use preconditions::*;
pub use proc_macro::*;
pub use rerun::*;
pub use shimcheck::*;
pub use toolchain::*;

#[cfg(test)]
mod tests;
