use std::{io, path::PathBuf, process::ExitStatus};

use crate::{Step, Unmet};

pub type Result<T = ()> = std::result::Result<T, BuildError>;

/// Errors that can occur while compiling a client crate to JavaScript.
///
/// The variants that have a fix end their message with the command that
/// applies it, on its own line, so a build script that unwraps prints
/// something a reader can copy.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("`OUT_DIR` is not set; `BuildConfig::render` must be called from a build script")]
    NoOutDir,
    #[error("unsupported platform: {os}; the codegen backend is only built for unix")]
    UnsupportedPlatform { os: &'static str },
    #[error(
        "JSC_PANIC={value:?} names no build mode; leave it unset (unwind) or set it to `abort`"
    )]
    PanicMode { value: String },
    #[error("no client crate root at {}", path.display())]
    ClientMissing { path: PathBuf },
    /// A setup step the checkout is missing, boxed because it carries the
    /// command that performs it and every other variant is a word or two.
    #[error("{0}")]
    Unmet(Box<Unmet>),
    #[error("the {} step failed with {status}", step.label())]
    Fix { step: Step, status: ExitStatus },
    #[error(
        "rustc exited with {status} compiling the client crate{}{}",
        if stderr.trim().is_empty() { "" } else { ":\n" },
        stderr.trim()
    )]
    Compile { status: ExitStatus, stderr: String },
    #[error("minimal runtime extraction exited with {status}:\n{stderr}")]
    MinimalShim { status: ExitStatus, stderr: String },
    #[error("the compiler produced no {}", path.display())]
    MissingOutput { path: PathBuf },
    #[error(
        "{name:?} cannot name a chunk; a name is made of ASCII letters, digits, `_`, `-`, and `$`"
    )]
    ChunkName { name: String },
    #[error("duplicate chunk name {name:?}")]
    DuplicateChunkName { name: String },
    #[error("chunk {name:?} collides with the shared chunk name")]
    SharedChunkNameCollision { name: String },
    #[error("{dir:?} is not a valid relative chunk directory")]
    InvalidChunkDir { dir: String },
    #[error(
        "optimization reports do not yet describe split chunk artifacts; disable chunking or \
         the report"
    )]
    ChunkedOptimizationReport,
    #[error("invalid artifact manifest at {}: {reason}", path.display())]
    ManifestInvalid { path: PathBuf, reason: String },
    #[error(
        "one runtime publication at {} cannot supply different entry providers",
        path.display()
    )]
    RuntimeProviderMismatch { path: PathBuf },
    #[error("runtime provider {name:?} collides with a compiler artifact")]
    RuntimeProviderCollision { name: String },
    #[error(
        "generated runtime for {specifier:?} needs a local `./` module specifier; use `shim(false)` \
         for an external provider"
    )]
    ExternalRuntimeRequiresNoShim { specifier: String },
    #[error("failed to derive sysroot cache key via {}: {detail}", script.display())]
    SysrootKey { script: PathBuf, detail: String },
    #[error(
        "the module supplying `__rt` ({}, imported as {specifier:?}) does not export {}:\n  {}\n\
         Add them to that module, or stop the compiled code reaching for them.",
        module.display(),
        if missing.len() == 1 { "this member" } else { "these members" },
        missing.join("\n  ")
    )]
    ShimMember {
        module: PathBuf,
        specifier: String,
        missing: Vec<String>,
    },
    #[error(
        "mires-build cannot read this export of {}, so it cannot check `__rt` members against it:\n  \
         {line}",
        module.display()
    )]
    ShimExportForm { module: PathBuf, line: String },
    #[error("io error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
