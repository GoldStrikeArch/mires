use std::path::{Path, PathBuf};

/// A proc-macro crate the expansion pass loads, built for the host by whoever
/// owns it.
///
/// The backend cannot run a proc macro, so a client crate that uses one is
/// expanded by plain rustc first and the backend compiles what comes out.
/// `mires-build` knows how to run that pass; it does not know which macros a
/// client uses, so the caller names each one: the crate name rustc resolves it
/// under, the dylib cargo built, and the source directories it was built from.
///
/// The sources matter as much as the dylib. A proc macro that is merely
/// present runs happily and expands the way it did whenever it was built, so
/// an edit to its source would show up as compiled output that quietly
/// disagrees with the macro as written. Naming the sources lets the build
/// refuse a stale dylib and rerun when they change.
///
/// ```rust,no_run
/// use mires_build::{BuildConfig, ProcMacro};
///
/// BuildConfig::new("client/app.rs")
///     .proc_macro(
///         ProcMacro::new("my_macro", "target/release/libmy_macro.dylib")
///             .source_dir("macros/my-macro"),
///     )
///     .render()
///     .unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcMacro {
    crate_name: String,
    dylib: PathBuf,
    sources: Vec<PathBuf>,
}

impl ProcMacro {
    /// The macro crate `crate_name`, built at `dylib`.
    #[must_use]
    pub fn new(crate_name: impl Into<String>, dylib: impl Into<PathBuf>) -> Self {
        Self {
            crate_name: crate_name.into(),
            dylib: dylib.into(),
            sources: Vec::new(),
        }
    }

    /// A directory the macro is built from. Its Rust sources and manifests
    /// decide whether the dylib is current, and are watched for reruns. Calls
    /// accumulate.
    #[must_use]
    pub fn source_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.sources.push(dir.into());
        self
    }

    /// The name rustc resolves the macro crate under.
    #[must_use]
    pub fn crate_name(&self) -> &str {
        &self.crate_name
    }

    /// The host dylib rustc loads.
    #[must_use]
    pub fn dylib(&self) -> &Path {
        &self.dylib
    }

    /// The directories the macro is built from.
    #[must_use]
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }
}
