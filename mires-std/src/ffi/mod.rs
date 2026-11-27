//! Foreign function interface utilities and OS strings (`OsStr`, `OsString`, `CStr`, `CString`).

mod os_str;

pub use alloc_crate::ffi::{CString, FromVecWithNulError, IntoStringError, NulError};
pub use core_crate::ffi::*;
pub use os_str::{Display, OsStr, OsString};
