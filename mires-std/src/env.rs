//! Inspection and manipulation of the process's environment (stubs matching
//! upstream `wasm32-unknown-unknown`).

use crate::error::Error;
use crate::ffi::{OsStr, OsString};
use crate::fmt;
use crate::io;
use crate::path::{Path, PathBuf};
use crate::string::String;

/// Constants describing the target platform.
pub mod consts {
    /// A string describing the architecture (`"wasm32"`).
    pub const ARCH: &str = "wasm32";
    /// A string describing the specific operating system (`"unknown"`).
    pub const OS: &str = "unknown";
    /// A string describing the family of the operating system (`"wasm"`).
    pub const FAMILY: &str = "wasm";
    /// A string describing the ABI environment (`""`).
    pub const ENV: &str = "";
    /// Specifies the filename prefix used for shared libraries.
    pub const DLL_PREFIX: &str = "";
    /// Specifies the filename suffix used for shared libraries.
    pub const DLL_SUFFIX: &str = ".wasm";
    /// Specifies the file extension used for shared libraries.
    pub const DLL_EXTENSION: &str = "wasm";
    /// Specifies the filename suffix used for executable binaries.
    pub const EXE_SUFFIX: &str = ".wasm";
    /// Specifies the file extension used for executable binaries.
    pub const EXE_EXTENSION: &str = "wasm";
}

/// The error type for operations interacting with environment variables.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VarError {
    /// The specified environment variable was not present.
    NotPresent,
    /// The specified environment variable was found, but was not valid UTF-8.
    NotUnicode(OsString),
}

impl fmt::Display for VarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarError::NotPresent => f.write_str("environment variable not found"),
            VarError::NotUnicode(s) => {
                write!(f, "environment variable was not valid unicode: {s:?}")
            }
        }
    }
}

impl Error for VarError {}

/// An iterator over the arguments of a process (empty on JS targets).
#[derive(Debug)]
pub struct Args {
    _private: (),
}

impl Iterator for Args {
    type Item = String;

    #[inline]
    fn next(&mut self) -> Option<String> {
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}

impl ExactSizeIterator for Args {
    #[inline]
    fn len(&self) -> usize {
        0
    }
}

impl DoubleEndedIterator for Args {
    #[inline]
    fn next_back(&mut self) -> Option<String> {
        None
    }
}

/// An iterator over the OS-string arguments of a process (empty on JS targets).
#[derive(Debug)]
pub struct ArgsOs {
    _private: (),
}

impl Iterator for ArgsOs {
    type Item = OsString;

    #[inline]
    fn next(&mut self) -> Option<OsString> {
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}

impl ExactSizeIterator for ArgsOs {
    #[inline]
    fn len(&self) -> usize {
        0
    }
}

impl DoubleEndedIterator for ArgsOs {
    #[inline]
    fn next_back(&mut self) -> Option<OsString> {
        None
    }
}

/// An iterator over a snapshot of the environment variables (empty on JS targets).
#[derive(Debug)]
pub struct Vars {
    _private: (),
}

impl Iterator for Vars {
    type Item = (String, String);

    #[inline]
    fn next(&mut self) -> Option<(String, String)> {
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}

/// An iterator over a snapshot of OS-string environment variables (empty on JS targets).
#[derive(Debug)]
pub struct VarsOs {
    _private: (),
}

impl Iterator for VarsOs {
    type Item = (OsString, OsString);

    #[inline]
    fn next(&mut self) -> Option<(OsString, OsString)> {
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}

/// Returns the arguments that this program was started with (empty on JS targets).
#[must_use]
#[inline]
pub fn args() -> Args {
    Args { _private: () }
}

/// Returns the OS-string arguments that this program was started with (empty on JS targets).
#[must_use]
#[inline]
pub fn args_os() -> ArgsOs {
    ArgsOs { _private: () }
}

/// Returns an iterator of `(variable, value)` pairs of strings (empty on JS targets).
#[must_use]
#[inline]
pub fn vars() -> Vars {
    Vars { _private: () }
}

/// Returns an iterator of `(variable, value)` pairs of `OsString`s (empty on JS targets).
#[must_use]
#[inline]
pub fn vars_os() -> VarsOs {
    VarsOs { _private: () }
}

/// Fetches the environment variable `key` from the current process (always `NotPresent`).
#[inline]
pub fn var<K: AsRef<OsStr>>(_key: K) -> Result<String, VarError> {
    Err(VarError::NotPresent)
}

/// Fetches the environment variable `key` as an `OsString` (always `None`).
#[must_use]
#[inline]
pub fn var_os<K: AsRef<OsStr>>(_key: K) -> Option<OsString> {
    None
}

/// Returns the current working directory (`Unsupported` on JS targets).
pub fn current_dir() -> io::Result<PathBuf> {
    Err(io::Error::unsupported(
        "current_dir is not supported on JavaScript targets",
    ))
}

/// Changes the current working directory (`Unsupported` on JS targets).
pub fn set_current_dir<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    Err(io::Error::unsupported(
        "set_current_dir is not supported on JavaScript targets",
    ))
}

/// Returns the full filesystem path of the current running executable (`Unsupported`).
pub fn current_exe() -> io::Result<PathBuf> {
    Err(io::Error::unsupported(
        "current_exe is not supported on JavaScript targets",
    ))
}

/// Returns the path of a temporary directory (`"/tmp"`).
#[must_use]
#[inline]
pub fn temp_dir() -> PathBuf {
    PathBuf::from("/tmp")
}
