//! Ergonomic error type for `mires` Rust-to-JavaScript functions.

use alloc::string::{String, ToString};
use core::fmt;

/// A lightweight JavaScript-facing error carrying a UTF-8 message.
///
/// Works identically in `#![no_std]` + `alloc` crates, `mires-std` crates, and native host
/// `cargo test` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    message: String,
}

impl JsError {
    /// Creates a new `JsError` from any string-like message.
    #[must_use]
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Throws this error as a synchronous JavaScript exception on the `mires` JS target.
    ///
    /// On native host targets (such as `cargo test`), panics with the error message.
    pub fn throw(self) -> ! {
        #[cfg(mires)]
        {
            mires_js::throw_value(mires_js::JsValue::erase(self.message.as_str()))
        }
        #[cfg(not(mires))]
        {
            panic!("{}", self.message)
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for JsError {}

impl From<String> for JsError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for JsError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<fmt::Error> for JsError {
    fn from(error: fmt::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<JsError> for String {
    fn from(error: JsError) -> Self {
        error.message
    }
}
