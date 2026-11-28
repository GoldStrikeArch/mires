//! Compiles Rust client crates to JavaScript from a Cargo build script.
//!
//! Two builders cover the two ways a host Cargo crate embeds Rust-compiled
//! JavaScript:
//!
//! * [`Crate`] (always available, zero heavy dependencies) compiles a full
//!   Cargo package (with its own `Cargo.toml`, `std`, `alloc`, and crates.io
//!   dependencies) via `cargo-mires`, defaulting to self-contained ES modules
//!   with inline runtime, `.d.ts` declarations, source maps, and an isolated
//!   `--target-dir` under `$OUT_DIR`:
//!
//!   ```rust,no_run
//!   if mires_build::is_target_build() {
//!       return;
//!   }
//!   let client = mires_build::Crate::new("../client")
//!       .release()
//!       .emit_rust_module("client_assets.rs")
//!       .build()
//!       .unwrap();
//!   ```
//!
//! * `BuildConfig` (requires the optional `single-file` Cargo feature) drives
//!   `rustc` directly on a single `.rs` root file (useful for lightweight
//!   scripts or multi-island chunked builds), publishing the compiled program,
//!   source map, optional `.d.ts`, and runtime shim into `$OUT_DIR`:
//!
//!   ```rust,no_run
//!   # #[cfg(feature = "single-file")]
//!   # fn main() {
//!   use mires_build::{BuildConfig, ModuleFormat, Profile, RuntimeMode};
//!
//!   BuildConfig::new("client/app.rs")
//!       .profile(Profile::ReleaseBalanced)
//!       .format(ModuleFormat::Esm)
//!       .runtime(RuntimeMode::Inline)
//!       .dts(true)
//!       .render()
//!       .unwrap();
//!   # }
//!   # #[cfg(not(feature = "single-file"))]
//!   # fn main() {}
//!   ```
//!
//! The compiled program can then be embedded or served like any other generated
//! file, for example `include_str!(concat!(env!("OUT_DIR"), "/app.js"))`.
//!
//! # Setup
//!
//! For installed users, install `cargo-mires` (e.g. via `cargo binstall cargo-mires`
//! or `cargo install cargo-mires`). [`Crate::build`] invokes `cargo-mires`, which
//! resolves its managed toolchain cache automatically (or downloads a prebuilt
//! toolchain bundle when [`Crate::auto_install`] is enabled).
//!
//! From a local Mires checkout, the pinned nightly, the backend built with it,
//! and the sysroot (`core`, `alloc`, and Mires's `std`) are set up once:
//!
//! ```sh
//! rustup toolchain install "$(...)"           # the nightly the backend loads into
//! scripts/build.sh                            # the codegen backend + jsc-link
//! JS_EXTRA_ARGS='' scripts/build_sysroot.sh   # `core`, `alloc`, and `std`
//! ```
//!
//! With the `single-file` feature enabled, `BuildConfig::render` reports the
//! first missing precondition with its fix-it command (or builds them
//! automatically when `BuildConfig::bootstrap` is enabled), and
//! `Toolchain::preconditions` / `Toolchain::apply` expose those checks directly.
//!
//! # Single-file client crates (`feature = "single-file"`)
//!
//! When using `BuildConfig` directly rather than [`Crate`], `rustc` is handed
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
//! # Chunks (`feature = "single-file"`)
//!
//! A page with several islands does not have to load all of them out of one
//! file. `BuildConfig::chunks` asks for one file per island, plus a shared
//! file holding what two or more of them reach:
//!
//! ```rust,no_run
//! # #[cfg(feature = "single-file")]
//! # fn main() {
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
//! # }
//! # #[cfg(not(feature = "single-file"))]
//! # fn main() {}
//! ```
//!
//! # Rebuilds
//!
//! Both [`Crate::build`] and `BuildConfig::render` emit
//! `cargo:rerun-if-changed` and `cargo:rerun-if-env-changed` directives for the
//! client crate's sources and toolchain inputs. Emitting any such directive
//! replaces Cargo's default rule of rerunning on any change in the host
//! package, so editing server code no longer recompiles client JavaScript.

#[cfg(feature = "single-file")]
mod chunk;
#[cfg(feature = "single-file")]
mod config;
mod crate_build;
mod error;
mod options;
#[cfg(feature = "single-file")]
mod pinned;
#[cfg(feature = "single-file")]
mod preconditions;
#[cfg(feature = "single-file")]
mod rerun;
#[cfg(feature = "single-file")]
mod shimcheck;
#[cfg(feature = "single-file")]
mod toolchain;

#[cfg(feature = "single-file")]
pub use chunk::*;
#[cfg(feature = "single-file")]
pub use config::*;
pub use crate_build::*;
pub use error::*;
pub use options::*;
#[cfg(feature = "single-file")]
pub use pinned::*;
#[cfg(feature = "single-file")]
pub use preconditions::*;
#[cfg(feature = "single-file")]
pub use rcgjs_object::artifact::EntryRole;
#[cfg(feature = "single-file")]
#[doc(hidden)]
pub use rcgjs_object::opts::OptimizerPass;
#[cfg(feature = "single-file")]
pub use rcgjs_object::opts::{Lane, OptimizerMode};
#[cfg(feature = "single-file")]
pub use rerun::*;
#[cfg(feature = "single-file")]
pub use shimcheck::*;
#[cfg(feature = "single-file")]
pub use toolchain::*;

#[cfg(test)]
mod tests;
