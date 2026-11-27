//! Process termination, exit codes, and aborting.

use crate::convert::Infallible;
use crate::fmt;

/// The status code returned by `main` or `process::exit`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExitCode(u8);

impl ExitCode {
    /// Successful program termination (`0`).
    pub const SUCCESS: ExitCode = ExitCode(0);

    /// Unsuccessful program termination (`1`).
    pub const FAILURE: ExitCode = ExitCode(1);

    #[inline]
    pub(crate) const fn to_isize(self) -> isize {
        self.0 as isize
    }
}

impl From<u8> for ExitCode {
    #[inline]
    fn from(code: u8) -> Self {
        ExitCode(code)
    }
}

/// Trait for types that can be returned from `fn main()`.
#[lang = "termination"]
pub trait Termination {
    /// Converts this value into an [`ExitCode`].
    fn report(self) -> ExitCode;
}

impl Termination for () {
    #[inline]
    fn report(self) -> ExitCode {
        ExitCode::SUCCESS
    }
}

impl Termination for ! {
    #[inline]
    fn report(self) -> ExitCode {
        self
    }
}

impl Termination for Infallible {
    #[inline]
    fn report(self) -> ExitCode {
        match self {}
    }
}

impl Termination for ExitCode {
    #[inline]
    fn report(self) -> ExitCode {
        self
    }
}

impl<T: Termination, E: fmt::Debug> Termination for Result<T, E> {
    fn report(self) -> ExitCode {
        match self {
            Ok(val) => val.report(),
            Err(err) => {
                crate::eprintln!("Error: {err:?}");
                ExitCode::FAILURE
            }
        }
    }
}

/// Aborts the current program immediately without running destructors.
pub fn abort() -> ! {
    crate::sys::abort("process::abort")
}

/// Terminates the current program with the given exit code.
///
/// On JavaScript hosts without a process exit primitive, flushes standard
/// streams and aborts the execution.
pub fn exit(code: i32) -> ! {
    crate::io::stdio::flush_stdio();
    if code == 0 {
        crate::sys::abort("process::exit(0)")
    } else {
        crate::sys::abort("process::exit(non-zero)")
    }
}

/// Returns a synthetic process identifier (`1` in single-process JS hosts).
#[inline]
pub fn id() -> u32 {
    1
}
