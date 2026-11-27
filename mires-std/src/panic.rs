//! Panic support in the standard library.

use crate::any::Any;
use crate::boxed::Box;
use crate::fmt;
use crate::string::String;
use crate::thread::Result;

pub use core_crate::panic::{AssertUnwindSafe, Location, RefUnwindSafe, UnwindSafe};

/// Information about a panic passed to a panic hook.
#[derive(Debug)]
pub struct PanicHookInfo<'a> {
    payload: &'a (dyn Any + Send),
    location: &'static Location<'static>,
    can_unwind: bool,
}

/// Deprecated alias for [`PanicHookInfo`].
#[deprecated(since = "1.82.0", note = "use `PanicHookInfo` instead")]
pub type PanicInfo<'a> = PanicHookInfo<'a>;

impl<'a> PanicHookInfo<'a> {
    /// Returns the payload associated with the panic.
    #[must_use]
    #[inline]
    pub fn payload(&self) -> &(dyn Any + Send) {
        self.payload
    }

    /// Returns the payload as a `&str` if it is a `&'static str` or `String`.
    #[must_use]
    #[inline]
    pub fn payload_as_str(&self) -> Option<&str> {
        if let Some(s) = self.payload.downcast_ref::<&str>() {
            Some(s)
        } else if let Some(s) = self.payload.downcast_ref::<String>() {
            Some(s.as_str())
        } else {
            None
        }
    }

    /// Returns the source location where the panic originated.
    #[must_use]
    #[inline]
    pub fn location(&self) -> Option<&'static Location<'static>> {
        Some(self.location)
    }

    /// Returns whether unwinding is permitted from this panic.
    #[must_use]
    #[inline]
    pub fn can_unwind(&self) -> bool {
        self.can_unwind
    }
}

impl fmt::Display for PanicHookInfo<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("panicked at ")?;
        self.location.fmt(f)?;
        if let Some(msg) = self.payload_as_str() {
            f.write_str(":\n")?;
            f.write_str(msg)?;
        }
        Ok(())
    }
}

/// Invokes a closure, capturing the cause of an unwinding panic if one occurs.
pub fn catch_unwind<F: FnOnce() -> R + UnwindSafe, R>(f: F) -> Result<R> {
    crate::panicking::catch_unwind(f)
}

/// Triggers a panic with the given payload without invoking a panic hook.
pub fn resume_unwind(payload: Box<dyn Any + Send>) -> ! {
    crate::panicking::resume_unwind(payload)
}

/// Panics the current thread with an arbitrary `Any + Send` payload.
#[inline]
#[track_caller]
pub fn panic_any<M: 'static + Any + Send>(msg: M) -> ! {
    crate::panicking::begin_panic(msg)
}

impl<T: ?Sized> UnwindSafe for crate::sync::Mutex<T> {}
impl<T: ?Sized> RefUnwindSafe for crate::sync::Mutex<T> {}
impl<T: ?Sized> UnwindSafe for crate::sync::RwLock<T> {}
impl<T: ?Sized> RefUnwindSafe for crate::sync::RwLock<T> {}
impl<K, V, S> UnwindSafe for crate::collections::HashMap<K, V, S>
where
    K: UnwindSafe,
    V: UnwindSafe,
    S: UnwindSafe,
{
}
impl<K, V, S> RefUnwindSafe for crate::collections::HashMap<K, V, S>
where
    K: RefUnwindSafe,
    V: RefUnwindSafe,
    S: RefUnwindSafe,
{
}
