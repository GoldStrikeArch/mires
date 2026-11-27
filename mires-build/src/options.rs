use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{BuildError, Result};

/// Environment variable overriding the Mires toolchain checkout or bundle root.
pub const TOOLCHAIN_ROOT_ENV: &str = "MIRES_ROOT";
/// Legacy alias for [`TOOLCHAIN_ROOT_ENV`].
pub const SPIKE_ROOT_ENV: &str = TOOLCHAIN_ROOT_ENV;
/// Environment variable selecting the build mode: unset (or `unwind`) is the default
/// `-Cpanic=unwind` build, `abort` the `-Cpanic=abort` override.
pub const PANIC_ENV: &str = "JSC_PANIC";
/// Environment variable enabling automatic toolchain download in `cargo-mires`.
pub const AUTO_INSTALL_ENV: &str = "MIRES_AUTO_INSTALL";

#[cfg(feature = "single-file")]
pub use rcgjs_object::opts::{Modules as ModuleFormat, Profile, RuntimeMode};

/// A named bundle of compiler option settings.
#[cfg(not(feature = "single-file"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    /// Fastest compile with optimizer disabled.
    Dev,
    /// Run optimizer analyses and report results without rewriting JavaScript.
    Report,
    /// Compiler-side size policy.
    ReleaseSize,
    /// Weighted bytes and allocation policy (default for Cargo release builds).
    ReleaseBalanced,
    /// Speed-oriented release policy keeping top-level item names readable.
    ReleaseSpeed,
}

#[cfg(not(feature = "single-file"))]
impl Profile {
    /// The CLI spelling of this profile (`"dev"`, `"report"`, `"release-size"`, etc.).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Report => "report",
            Self::ReleaseSize => "release-size",
            Self::ReleaseBalanced => "release-balanced",
            Self::ReleaseSpeed => "release-speed",
        }
    }

    /// The `-Cllvm-args` spelling of this profile (`"js-profile=..."`).
    #[must_use]
    pub const fn backend_arg(self) -> &'static str {
        match self {
            Self::Dev => "js-profile=dev",
            Self::Report => "js-profile=report",
            Self::ReleaseSize => "js-profile=release-size",
            Self::ReleaseBalanced => "js-profile=release-balanced",
            Self::ReleaseSpeed => "js-profile=release-speed",
        }
    }

    /// Parse a profile from its CLI spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dev" => Some(Self::Dev),
            "report" => Some(Self::Report),
            "release-size" => Some(Self::ReleaseSize),
            "release-balanced" => Some(Self::ReleaseBalanced),
            "release-speed" => Some(Self::ReleaseSpeed),
            _ => None,
        }
    }
}

/// What kind of JavaScript file the compiled program is.
#[cfg(not(feature = "single-file"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleFormat {
    /// An ES module: imports the runtime (unless inlined) and exports items with `export`.
    Esm,
    /// A plain script: expects global `__rt` (unless inlined) and exposes global bindings.
    Script,
}

/// How the runtime shim is supplied to the linked JavaScript program (`js-runtime`).
#[cfg(not(feature = "single-file"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeMode {
    /// Import `__rt` from an external shim module in ESM output, or expect global `__rt` in script output.
    #[default]
    External,
    /// Inline the minimal transitive runtime closure as a lexical prelude inside each emitted unit.
    Inline,
    /// Emit neither a runtime import nor an inline prelude; the host supplies `__rt`.
    None,
}

#[cfg(not(feature = "single-file"))]
impl RuntimeMode {
    /// The CLI spelling of this runtime mode (`"external"`, `"inline"`, or `"none"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Inline => "inline",
            Self::None => "none",
        }
    }

    /// Parse a runtime mode from its CLI spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "external" => Some(Self::External),
            "inline" => Some(Self::Inline),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The build mode [`PANIC_ENV`] selects.
///
/// The checkout's scripts read the same variable, and the toolchain passes it to every script it
/// runs, so a client crate, its sysroot and the scripts can never disagree about the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicMode {
    /// `-Cpanic=abort` on `wasm32-unknown-unknown`: explicit opt-in override.
    Abort,
    /// `-Cpanic=unwind` on `targets/mires32-unwind.json`, with `js-cleanup=off` appended by the
    /// scripts: rustc's unwind edges carry host-exception cleanup (the default).
    Unwind,
}

impl PanicMode {
    /// The mode a [`PANIC_ENV`] value names, or `None` for a value that names none.
    #[must_use]
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("" | "unwind") => Some(Self::Unwind),
            Some("abort") => Some(Self::Abort),
            Some(_) => None,
        }
    }

    /// The mode in this process's environment.
    ///
    /// # Errors
    ///
    /// Returns `Err` when [`PANIC_ENV`] is set to anything but `abort` or `unwind`.
    pub fn from_env() -> Result<Self> {
        let value = env::var(PANIC_ENV).ok();
        Self::parse(value.as_deref()).ok_or_else(|| BuildError::PanicMode {
            value: value.unwrap_or_default(),
        })
    }

    /// The target's name: what the sysroot's `lib/rustlib/<name>/lib` is called.
    #[must_use]
    pub fn target_name(self) -> &'static str {
        match self {
            Self::Abort => "wasm32-unknown-unknown",
            Self::Unwind => "mires32-unwind",
        }
    }

    /// The CLI spelling of this panic mode (`"unwind"` or `"abort"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unwind => "unwind",
            Self::Abort => "abort",
        }
    }

    /// What [`PANIC_ENV`] is set to for a child process; `None` removes an inherited value.
    #[cfg(feature = "single-file")]
    pub(crate) fn env_value(self) -> Option<&'static str> {
        match self {
            Self::Unwind => None,
            Self::Abort => Some("abort"),
        }
    }
}

pub(crate) fn module_format_str(format: ModuleFormat) -> &'static str {
    match format {
        ModuleFormat::Esm => "esm",
        ModuleFormat::Script => "script",
    }
}

/// The parent directory of this package's manifest, which is the Mires checkout
/// root when built directly from a local repository checkout.
pub(crate) fn checkout_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf()
}

/// The codegen backend dylib's file name on supported Unix platforms.
pub(crate) fn dylib_filename() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        None
    } else if cfg!(target_os = "macos") {
        Some("librustc_codegen_mires.dylib")
    } else {
        Some("librustc_codegen_mires.so")
    }
}

/// Drop the environment Cargo set for this build script.
///
/// Nested compiler invocations ask for the pinned nightly explicitly, which an
/// inherited `RUSTUP_TOOLCHAIN` would silently override. The remaining Cargo and
/// rustc variables are cleared so they never shift nested fingerprints or
/// leak host settings into the client build.
pub(crate) fn scrub(command: &mut Command) {
    for (key, _) in env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("CARGO")
            || name == "RUSTC"
            || name == "RUSTC_WRAPPER"
            || name == "RUSTC_WORKSPACE_WRAPPER"
            || name == "RUSTUP_TOOLCHAIN"
            || name == "RUSTFLAGS"
        {
            command.env_remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_strings_and_parsing_match_cli_contract() {
        let cases = [
            (Profile::Dev, "dev", "js-profile=dev"),
            (Profile::Report, "report", "js-profile=report"),
            (
                Profile::ReleaseSize,
                "release-size",
                "js-profile=release-size",
            ),
            (
                Profile::ReleaseBalanced,
                "release-balanced",
                "js-profile=release-balanced",
            ),
            (
                Profile::ReleaseSpeed,
                "release-speed",
                "js-profile=release-speed",
            ),
        ];
        for (profile, name, arg) in cases {
            assert_eq!(profile.as_str(), name);
            assert_eq!(profile.backend_arg(), arg);
            assert_eq!(Profile::parse(name), Some(profile));
        }
        assert_eq!(Profile::parse("unknown"), None);
    }

    #[test]
    fn runtime_mode_and_module_format_match_cli_contract() {
        assert_eq!(RuntimeMode::default(), RuntimeMode::External);
        for (mode, name) in [
            (RuntimeMode::External, "external"),
            (RuntimeMode::Inline, "inline"),
            (RuntimeMode::None, "none"),
        ] {
            assert_eq!(mode.as_str(), name);
            assert_eq!(RuntimeMode::parse(name), Some(mode));
        }
        assert_eq!(RuntimeMode::parse("invalid"), None);
        assert_eq!(module_format_str(ModuleFormat::Esm), "esm");
        assert_eq!(module_format_str(ModuleFormat::Script), "script");
    }

    #[test]
    fn panic_mode_names_the_two_build_modes() {
        assert_eq!(PanicMode::parse(None), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("")), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("unwind")), Some(PanicMode::Unwind));
        assert_eq!(PanicMode::parse(Some("abort")), Some(PanicMode::Abort));
        assert_eq!(PanicMode::parse(Some("Unwind")), None);
        assert_eq!(PanicMode::Unwind.as_str(), "unwind");
        assert_eq!(PanicMode::Abort.as_str(), "abort");
        assert_eq!(PanicMode::Abort.target_name(), "wasm32-unknown-unknown");
        assert_eq!(PanicMode::Unwind.target_name(), "mires32-unwind");
    }
}
