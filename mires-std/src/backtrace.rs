//! Support for capturing a stack backtrace of an OS thread.
//!
//! On the JavaScript target native OS stack unwinding metadata is unavailable,
//! so [`Backtrace::capture`] and [`Backtrace::force_capture`] return an
//! unsupported backtrace (matching `wasm32-unknown-unknown` in upstream `std`).

use crate::fmt;

/// The current status of a backtrace, indicating whether it was captured or
/// whether it is empty for some other reason.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub enum BacktraceStatus {
    /// Capturing a backtrace is not supported on the current platform.
    Unsupported,
    /// Capturing a backtrace has been disabled.
    Disabled,
    /// A backtrace has been captured.
    Captured,
}

#[derive(Clone, Copy)]
enum Inner {
    Unsupported,
    Disabled,
}

/// A captured OS thread stack backtrace.
#[must_use]
pub struct Backtrace {
    inner: Inner,
}

impl Backtrace {
    /// Capture a stack backtrace of the current thread.
    ///
    /// Always returns an unsupported backtrace on the JavaScript target.
    #[inline]
    pub fn capture() -> Backtrace {
        Backtrace {
            inner: Inner::Unsupported,
        }
    }

    /// Forcibly capture a full stack backtrace regardless of environment configuration.
    ///
    /// Always returns an unsupported backtrace on the JavaScript target.
    #[inline]
    pub fn force_capture() -> Backtrace {
        Backtrace {
            inner: Inner::Unsupported,
        }
    }

    /// Forcibly captures a disabled backtrace, regardless of environment
    /// variable configuration.
    #[inline]
    pub const fn disabled() -> Backtrace {
        Backtrace {
            inner: Inner::Disabled,
        }
    }

    /// Returns the status of this backtrace.
    #[must_use]
    #[inline]
    pub fn status(&self) -> BacktraceStatus {
        match self.inner {
            Inner::Unsupported => BacktraceStatus::Unsupported,
            Inner::Disabled => BacktraceStatus::Disabled,
        }
    }
}

impl fmt::Debug for Backtrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner {
            Inner::Unsupported => f.write_str("<unsupported>"),
            Inner::Disabled => f.write_str("<disabled>"),
        }
    }
}

impl fmt::Display for Backtrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner {
            Inner::Unsupported => f.write_str("unsupported backtrace"),
            Inner::Disabled => f.write_str("disabled backtrace"),
        }
    }
}
