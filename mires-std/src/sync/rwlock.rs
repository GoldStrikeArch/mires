//! Single-threaded `RwLock`, `RwLockReadGuard`, and `RwLockWriteGuard`.

use crate::cell::{Cell, UnsafeCell};
use crate::fmt;
use crate::ops::{Deref, DerefMut};
use crate::sync::{LockResult, PoisonError, TryLockError, TryLockResult};

/// A reader-writer lock allowing multiple readers or a single writer.
pub struct RwLock<T: ?Sized> {
    /// `0` = unlocked, `> 0` = active reader count, `-1` = writer locked.
    state: Cell<isize>,
    poisoned: Cell<bool>,
    data: UnsafeCell<T>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`); dynamic
// aliasing is checked by `state`.
unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
// SAFETY: Same single-threaded guarantee; `state` prevents aliasing `&mut T`.
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

/// RAII structure used to release the shared read access of a lock when dropped.
#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockReadGuard<'a, T: ?Sized + 'a> {
    lock: &'a RwLock<T>,
}

/// RAII structure used to release the exclusive write access of a lock when dropped.
#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockWriteGuard<'a, T: ?Sized + 'a> {
    lock: &'a RwLock<T>,
}

// SAFETY: Matches upstream `std` trait bounds.
unsafe impl<T: ?Sized + Sync> Sync for RwLockReadGuard<'_, T> {}
// SAFETY: Matches upstream `std` trait bounds.
unsafe impl<T: ?Sized + Sync> Sync for RwLockWriteGuard<'_, T> {}

impl<T> RwLock<T> {
    /// Creates a new instance of an `RwLock<T>` which is unlocked.
    #[inline]
    pub const fn new(t: T) -> RwLock<T> {
        RwLock {
            state: Cell::new(0),
            poisoned: Cell::new(false),
            data: UnsafeCell::new(t),
        }
    }

    /// Consumes this `RwLock`, returning the underlying data.
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

impl<T: ?Sized> RwLock<T> {
    /// Locks this `RwLock` with shared read access.
    #[inline]
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        let s = self.state.get();
        if s < 0 {
            panic!("cannot acquire RwLock read lock while write-locked on a single-threaded target");
        }
        self.state.set(s.checked_add(1).expect("RwLock reader count overflow"));
        let guard = RwLockReadGuard { lock: self };
        if self.poisoned.get() {
            Err(PoisonError::new(guard))
        } else {
            Ok(guard)
        }
    }

    /// Attempts to acquire this `RwLock` with shared read access.
    #[inline]
    pub fn try_read(&self) -> TryLockResult<RwLockReadGuard<'_, T>> {
        let s = self.state.get();
        if s < 0 {
            return Err(TryLockError::WouldBlock);
        }
        self.state.set(s.checked_add(1).expect("RwLock reader count overflow"));
        let guard = RwLockReadGuard { lock: self };
        if self.poisoned.get() {
            Err(TryLockError::Poisoned(PoisonError::new(guard)))
        } else {
            Ok(guard)
        }
    }

    /// Locks this `RwLock` with exclusive write access.
    #[inline]
    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, T>> {
        if self.state.get() != 0 {
            panic!("cannot acquire RwLock write lock while already locked on a single-threaded target");
        }
        self.state.set(-1);
        let guard = RwLockWriteGuard { lock: self };
        if self.poisoned.get() {
            Err(PoisonError::new(guard))
        } else {
            Ok(guard)
        }
    }

    /// Attempts to lock this `RwLock` with exclusive write access.
    #[inline]
    pub fn try_write(&self) -> TryLockResult<RwLockWriteGuard<'_, T>> {
        if self.state.get() != 0 {
            return Err(TryLockError::WouldBlock);
        }
        self.state.set(-1);
        let guard = RwLockWriteGuard { lock: self };
        if self.poisoned.get() {
            Err(TryLockError::Poisoned(PoisonError::new(guard)))
        } else {
            Ok(guard)
        }
    }

    /// Determines whether the lock is poisoned.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.get()
    }

    /// Clears the poisoned state from a lock.
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

impl<T> From<T> for RwLock<T> {
    #[inline]
    fn from(t: T) -> Self {
        RwLock::new(t)
    }
}

impl<T: ?Sized + Default> Default for RwLock<T> {
    #[inline]
    fn default() -> RwLock<T> {
        RwLock::new(Default::default())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("RwLock");
        match self.try_read() {
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

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: Active read guard ensures no writer holds `self.lock.data`.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        let s = self.lock.state.get();
        self.lock.state.set(s - 1);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: Active write guard guarantees exclusive access to `self.lock.data`.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: Active write guard guarantees exclusive mutable access to `self.lock.data`.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        if crate::panicking::panicking() {
            self.lock.poisoned.set(true);
        }
        self.lock.state.set(0);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}
