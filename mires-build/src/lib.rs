//! Compiles Rust client crates to JavaScript from a Cargo build script.
//!
//! Two builders cover the two ways a host Cargo crate embeds Rust-compiled
//! JavaScript:
//!
//! * [`Crate`] compiles a full Cargo package (with its own `Cargo.toml`, `std`,
//!   `alloc`, and crates.io dependencies) via `cargo-mires`, defaulting to
//!   self-contained ES modules with inline runtime, `.d.ts` declarations, source
//!   maps, and an isolated `--target-dir` under `$OUT_DIR`:
//!
//!   ```rust,no_run
//!   let client = mires_build::Crate::new("../client")
//!       .build()
//!       .unwrap();
//!   ```
//!
//! * [`BuildConfig`] drives `rustc` directly on a single `.rs` root file
//!   (useful for lightweight scripts or multi-island chunked builds), publishing
//!   the compiled program, source map, optional `.d.ts`, and runtime shim into
//!   `$OUT_DIR`:
//!
//!   ```rust,no_run
//!   use mires_build::{BuildConfig, ModuleFormat, Profile, RuntimeMode};
//!
//!   BuildConfig::new("client/app.rs")
//!       .profile(Profile::ReleaseBalanced)
//!       .format(ModuleFormat::Esm)
//!       .runtime(RuntimeMode::Inline)
//!       .dts(true)
//!       .render()
//!       .unwrap();
//!   ```
//!
//! The compiled program can then be embedded or served like any other generated
//! file, for example `include_str!(concat!(env!("OUT_DIR"), "/app.js"))`.
//!
//! # Setup
//!
//! The pinned nightly, the backend built with it, and the sysroot (`core`,
//! `alloc`, and Mires's `std`) are set up once per Mires checkout:
//!
//! ```sh
//! rustup toolchain install "$(...)"           # the nightly the backend loads into
//! scripts/build.sh                            # the codegen backend + jsc-link
//! JS_EXTRA_ARGS='' scripts/build_sysroot.sh   # `core`, `alloc`, and `std`
//! ```
//!
//! [`Crate::build`] invokes `cargo-mires`, which automatically ensures the
//! backend and matching cached sysroot exist. [`BuildConfig::render`] reports
//! the first missing precondition with its fix-it command, or builds them
//! automatically when [`BuildConfig::bootstrap`] is enabled.
//!
//! [`Toolchain::preconditions`] returns the unmet steps, and
//! [`Toolchain::apply`] performs one, so a tool outside a build script can set
//! the checkout up without a second opinion about when it is ready.
//!
//! The `JS_EXTRA_ARGS` the sysroot was built with are recorded next to it. Any
//! backend option that changes how items are spelled has to be the same on
//! both sides, so [`BuildConfig::emit_arg`] is checked against that record and
//! refuses a sysroot that disagrees. [`BuildConfig::crate_arg`] and link-only
//! typed options ([`BuildConfig::format`], [`BuildConfig::runtime`],
//! [`BuildConfig::dts`], [`BuildConfig::chunks`]) only affect linking and need
//! no sysroot rebuild.
//!
//! # Single-file client crates (`BuildConfig`)
//!
//! When using [`BuildConfig`] directly rather than [`Crate`], `rustc` is handed
//! a single `.rs` root file (edition 2021):
//!
//! * The Mires sysroot provides `core`, `alloc` (`Box`, `Vec`, `String`,
//!   `BTreeMap`, ...), and Mires's `std` crate.
//! * Extra modules are reached with `#[path = "..."] mod name;` (or switch to
//!   [`Crate`] for full Cargo package layout and external crate dependencies).
//! * `#[no_mangle]` / `#[mires::export]` functions are the exports and roots
//!   for whole-program dead code elimination, keeping their names unchanged in
//!   the emitted JavaScript.
//! * An `extern "C"` function lowers to `__rt.<name>(...)`. With
//!   [`RuntimeMode::Inline`] the minimal runtime closure is bundled directly
//!   into the output; with [`RuntimeMode::External`] `jsc-link --emit-runtime`
//!   emits a canonical runtime module in pure Rust without Node.js.
//!
//! # Chunks
//!
//! A page with several islands does not have to load all of them out of one
//! file. [`BuildConfig::chunks`] asks for one file per island, plus a shared
//! file holding what two or more of them reach:
//!
//! ```rust,no_run
//! use mires_build::{BuildConfig, Chunking, ModuleFormat};
//!
//! let artifacts = BuildConfig::new("client-island/island.rs")
//!     .output_name("islands.js")
//!     .format(ModuleFormat::Esm)
//!     .chunks(
//!         Chunking::new()
//!             .island("counter")
//!             .island("search")
//!             .url_base("/demo/chunks/"),
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
//! # Rebuilds
//!
//! Both [`Crate::build`] and [`BuildConfig::render`] emit
//! `cargo:rerun-if-changed` and `cargo:rerun-if-env-changed` directives for the
//! client crate's sources and toolchain inputs. Emitting any such directive
//! replaces Cargo's default rule of rerunning on any change in the host
//! package, so editing server code no longer recompiles client JavaScript.

mod chunk;
mod config;
mod crate_build;
mod error;
mod pinned;
mod preconditions;
mod rerun;
mod shimcheck;
mod toolchain;

pub use chunk::*;
pub use config::*;
pub use crate_build::*;
pub use error::*;
pub use pinned::*;
pub use preconditions::*;
pub use rcgjs_object::artifact::EntryRole;
#[doc(hidden)]
pub use rcgjs_object::opts::OptimizerPass;
pub use rcgjs_object::opts::{Lane, Modules as ModuleFormat, OptimizerMode, Profile, RuntimeMode};
pub use rerun::*;
pub use shimcheck::*;
pub use toolchain::*;

#[cfg(test)]
mod tests;
