//! Single-threaded `Mutex` and `MutexGuard` with poison tracking.

use crate::cell::{Cell, UnsafeCell};
use crate::fmt;
use crate::ops::{Deref, DerefMut};
use crate::sync::{LockResult, PoisonError, TryLockError, TryLockResult};

/// A mutual exclusion primitive useful for protecting shared data.
pub struct Mutex<T: ?Sized> {
    locked: Cell<bool>,
    poisoned: Cell<bool>,
    data: UnsafeCell<T>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`), so
// cross-thread data races cannot occur. Dynamic re-entrancy is guarded by `locked`.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
// SAFETY: Same single-threaded guarantee; `locked` prevents aliasing `&mut T`.
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

/// An RAII implementation of a "scoped lock" of a mutex.
#[must_use = "if unused the Mutex will immediately unlock"]
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a Mutex<T>,
}

// SAFETY: `MutexGuard` is `Sync` when `T: Sync` (matching upstream `std`).
unsafe impl<T: ?Sized + Sync> Sync for MutexGuard<'_, T> {}

impl<T> Mutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    #[inline]
    pub const fn new(t: T) -> Mutex<T> {
        Mutex {
            locked: Cell::new(false),
            poisoned: Cell::new(false),
            data: UnsafeCell::new(t),
        }
    }

    /// Consumes this mutex, returning the underlying data.
    #[inline]
    pub fn into_inner(self) -> LockResult<T> {
        let poisoned = self.poisoned.get();
        let data = self.data.into_inner();
        if poisoned {
            Err(PoisonError::new(data))
        } else {
            Ok(data)
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquires a mutex, panicking on single-threaded recursive re-entry.
    #[inline]
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        if self.locked.get() {
            panic!("cannot recursively lock a Mutex on a single-threaded target");
        }
        self.locked.set(true);
        let guard = MutexGuard { lock: self };
        if self.poisoned.get() {
            Err(PoisonError::new(guard))
        } else {
            Ok(guard)
        }
    }

    /// Attempts to acquire this lock without blocking.
    #[inline]
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        if self.locked.get() {
            return Err(TryLockError::WouldBlock);
        }
        self.locked.set(true);
        let guard = MutexGuard { lock: self };
        if self.poisoned.get() {
            Err(TryLockError::Poisoned(PoisonError::new(guard)))
        } else {
            Ok(guard)
        }
    }

    /// Determines whether the mutex is poisoned.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.get()
    }

    /// Clears the poisoned state from a mutex.
    #[inline]
    pub fn clear_poison(&self) {
        self.poisoned.set(false);
    }

    /// Returns a mutable reference to the underlying data.
    #[inline]
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        let poisoned = self.poisoned.get();
        let data = self.data.get_mut();
        if poisoned {
            Err(PoisonError::new(data))
        } else {
            Ok(data)
        }
    }
}

impl<T> From<T> for Mutex<T> {
    #[inline]
    fn from(t: T) -> Self {
        Mutex::new(t)
    }
}

impl<T: ?Sized + Default> Default for Mutex<T> {
    #[inline]
    fn default() -> Mutex<T> {
        Mutex::new(Default::default())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Mutex");
        match self.try_lock() {
            Ok(guard) => {
                d.field("data", &&*guard);
            }
            Err(TryLockError::Poisoned(err)) => {
                d.field("data", &&**err.get_ref());
            }
            Err(TryLockError::WouldBlock) => {
                d.field("data", &format_args!("<locked>"));
            }
        }
        d.field("poisoned", &self.is_poisoned());
        d.finish_non_exhaustive()
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: Holding `MutexGuard` guarantees exclusive access to `self.lock.data`.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: Holding `MutexGuard` guarantees exclusive mutable access to `self.lock.data`.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        if crate::panicking::panicking() {
            self.lock.poisoned.set(true);
        }
        self.lock.locked.set(false);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}
