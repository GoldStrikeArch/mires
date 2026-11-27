//! Single-threaded thread-local storage (`LocalKey<T>`).

use crate::cell::{Cell, RefCell, UnsafeCell};
use crate::error::Error;
use crate::fmt;

/// An error returned by [`LocalKey::try_with`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AccessError;

impl fmt::Debug for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessError").finish()
    }
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("already destroyed")
    }
}

impl Error for AccessError {}

/// Internal `const`-initialized storage backing `thread_local!` on single-threaded JS.
#[doc(hidden)]
pub struct __ConstLocalStorage<T> {
    value: UnsafeCell<T>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`), so only
// one thread ever accesses thread-local storage.
unsafe impl<T> Sync for __ConstLocalStorage<T> {}

impl<T> __ConstLocalStorage<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            value: UnsafeCell::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> &T {
        // SAFETY: Single-threaded execution; value is initialized for `'static`.
        unsafe { &*self.value.get() }
    }
}

/// Internal lazily-initialized storage backing `thread_local!` on single-threaded JS.
#[doc(hidden)]
pub struct __LazyLocalStorage<T> {
    initializing: Cell<bool>,
    value: UnsafeCell<Option<T>>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`).
unsafe impl<T> Sync for __LazyLocalStorage<T> {}

impl<T> __LazyLocalStorage<T> {
    #[inline]
    pub const fn new() -> Self {
        Self {
            initializing: Cell::new(false),
            value: UnsafeCell::new(None),
        }
    }

    #[inline]
    pub fn get_or_init(&self, init: fn() -> T) -> &T {
        // SAFETY: Single-threaded; if already initialized, `value` is immutable thereafter.
        if let Some(val) = unsafe { (*self.value.get()).as_ref() } {
            return val;
        }
        self.init_slow(init)
    }

    #[cold]
    fn init_slow(&self, init: fn() -> T) -> &T {
        if self.initializing.get() {
            panic!("cannot recursively initialize a thread_local! key");
        }
        struct ResetGuard<'a>(&'a Cell<bool>);
        impl Drop for ResetGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        self.initializing.set(true);
        let _guard = ResetGuard(&self.initializing);
        let val = init();
        // SAFETY: Single-threaded and guarded against re-entrancy; no live reference exists yet.
        unsafe {
            *self.value.get() = Some(val);
            (*self.value.get()).as_ref().unwrap()
        }
    }
}

/// A thread-local storage key which owns its contents.
pub struct LocalKey<T: 'static> {
    inner: fn() -> &'static T,
}

impl<T: 'static> LocalKey<T> {
    #[doc(hidden)]
    #[inline]
    pub const fn __new(inner: fn() -> &'static T) -> LocalKey<T> {
        LocalKey { inner }
    }

    /// Acquires a reference to the value in this TLS key.
    #[inline]
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        f((self.inner)())
    }

    /// Acquires a reference to the value in this TLS key, returning an error if destroyed.
    #[inline]
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        Ok(f((self.inner)()))
    }
}

impl<T: 'static> LocalKey<Cell<T>> {
    /// Sets or initializes the contained value.
    #[inline]
    pub fn set(&'static self, value: T) {
        self.with(|c| c.set(value));
    }

    /// Returns a copy of the contained value.
    #[inline]
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(Cell::get)
    }

    /// Takes the contained value, leaving `Default::default()` in its place.
    #[inline]
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(Cell::take)
    }

    /// Replaces the contained value, returning the old value.
    #[inline]
    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}

impl<T: 'static> LocalKey<RefCell<T>> {
    /// Acquires a shared borrow of the contained value and passes it to `f`.
    #[inline]
    pub fn with_borrow<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.with(|cell| f(&cell.borrow()))
    }

    /// Acquires a mutable borrow of the contained value and passes it to `f`.
    #[inline]
    pub fn with_borrow_mut<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.with(|cell| f(&mut cell.borrow_mut()))
    }

    /// Sets the contained value.
    #[inline]
    pub fn set(&'static self, value: T) {
        self.with(|cell| *cell.borrow_mut() = value);
    }

    /// Takes the contained value, leaving `Default::default()` in its place.
    #[inline]
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(RefCell::take)
    }

    /// Replaces the contained value, returning the old value.
    #[inline]
    pub fn replace(&'static self, value: T) -> T {
        self.with(|cell| cell.replace(value))
    }
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
}
