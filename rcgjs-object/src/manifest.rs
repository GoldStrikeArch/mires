//! What the producer hands the consumer.
//!
//! The dylib knows four things `jsc-link` cannot work out for itself: the options this compilation
//! parsed out of `-Cllvm-args`, which object files this crate just wrote, which rlibs rustc
//! resolved for it and in what order, and where the output goes. It writes them into one file in
//! the temp directory and execs `jsc-link --manifest <path>`; nothing else crosses the seam.
//!
//! The encoding is postcard, the same codec the object format already uses, because both sides are
//! always built from one tree and a typed round trip beats a hand-rolled parser. `jsc-link
//! --print-manifest <path>` dumps one when a build has to be inspected.
//!
//! The type lives in this crate rather than in `jsc-link` so that the dylib never depends on the
//! consumer. That is what keeps the sysroot cache key — a digest of the dylib — indifferent to a
//! `jsc-link` edit, which is the whole reason a linker change is not a sysroot rebuild.

use std::path::PathBuf;

use crate::opts::Opts;
use serde::{Deserialize, Serialize};

/// The current manifest layout. Bumped whenever a field is added, removed, or reordered: postcard
/// is positional, so a mismatched pair would misread rather than fail.
pub const MANIFEST_VERSION: u16 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u16,
    /// The options, exactly as the backend parsed them. Parsed once, in the compiler, so that a
    /// bad option is reported against the command line that carried it.
    pub opts: Opts,
    /// The crate being linked, for diagnostics.
    pub crate_name: String,
    /// The crate type that made this a program, spelled as rustc spells it. Empty in prune mode.
    pub crate_type: String,
    pub mode: Mode,
    /// This crate's own objects, in the order they were written: the codegen unit modules, then
    /// the allocator module.
    pub objects: Vec<PathBuf>,
    /// Every linked rlib, in `each_linked_rlib` order. Their items are demoted to internal linkage
    /// as they are read, which is what keeps a dependency's exports from rooting the program.
    pub rlibs: Vec<PathBuf>,
    /// The text of every source file this compilation still holds, keyed by the name rustc prints
    /// in a diagnostic. Only filled when `js-source-map=on`; the map embeds it as
    /// `sourcesContent`.
    pub sources: Vec<(String, String)>,
}

/// Which of the two link entry points ran.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Mode {
    /// `--crate-type bin`/`cdylib`/`dylib`: a finished program, written to `output` or to stdout.
    Program { output: Option<PathBuf> },
    /// The single-crate `--emit=obj` path: the objects that were just written are resolved in
    /// place. The program replaces `first`; every other candidate is emptied.
    Prune { first: PathBuf, rest: Vec<PathBuf> },
}
