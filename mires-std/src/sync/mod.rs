//! Synchronization primitives for single-threaded JavaScript targets.
//!
//! Because `mires` targets single-threaded JS execution (`singlethread = true`),
//! `Mutex`, `RwLock`, `OnceLock`, `LazyLock`, and `Once` are backed by
//! single-threaded interior mutability cells wrapped with `Sync`, while still
//! enforcing Rust's dynamic borrow and poison invariants.

mod mutex;
mod once_lock;
mod rwlock;

use crate::error::Error;
use crate::fmt;

pub use alloc_crate::sync::{Arc, Weak};
pub use core_crate::sync::atomic;

pub use mutex::{Mutex, MutexGuard};
pub use once_lock::{LazyLock, Once, OnceLock, OnceState};
pub use rwlock::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A type of error which can be returned whenever a lock is acquired.
pub struct PoisonError<T> {
    guard: T,
}

impl<T> PoisonError<T> {
    /// Creates a `PoisonError` wrapping the given lock guard.
    #[inline]
    pub fn new(guard: T) -> PoisonError<T> {
        PoisonError { guard }
    }

    /// Consumes this error, returning the underlying guard to allow access
    /// to the poisoned data.
    #[inline]
    pub fn into_inner(self) -> T {
        self.guard
    }

    /// Reaches into this error, returning a reference to the underlying guard.
    #[inline]
    pub fn get_ref(&self) -> &T {
        &self.guard
    }

    /// Reaches into this error, returning a mutable reference to the underlying guard.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> fmt::Debug for PoisonError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoisonError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for PoisonError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("poisoned lock: another task failed inside")
    }
}

impl<T> Error for PoisonError<T> {}

/// An enumeration of possible errors associated with a [`TryLockResult`].
pub enum TryLockError<T> {
    /// The lock could not be acquired because another user panicked while holding it.
    Poisoned(PoisonError<T>),
    /// The lock could not be acquired at this time because it is already held.
    WouldBlock,
}

impl<T> From<PoisonError<T>> for TryLockError<T> {
    #[inline]
    fn from(err: PoisonError<T>) -> TryLockError<T> {
        TryLockError::Poisoned(err)
    }
}

impl<T> fmt::Debug for TryLockError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TryLockError::Poisoned(_) => f.write_str("Poisoned(..)"),
            TryLockError::WouldBlock => f.write_str("WouldBlock"),
        }
    }
}

impl<T> fmt::Display for TryLockError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TryLockError::Poisoned(_) => f.write_str("poisoned lock: another task failed inside"),
            TryLockError::WouldBlock => f.write_str("try_lock failed because the operation would block"),
        }
    }
}

impl<T> Error for TryLockError<T> {}

/// A type alias for the result of a lock method which can be poisoned.
pub type LockResult<Guard> = Result<Guard, PoisonError<Guard>>;

/// A type alias for the result of a nonblocking locking method.
pub type TryLockResult<Guard> = Result<Guard, TryLockError<Guard>>;

/// Result returned by [`Condvar::wait_timeout`].
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct WaitTimeoutResult(bool);

impl WaitTimeoutResult {
    /// Returns `true` if the wait is known to have timed out.
    #[must_use]
    #[inline]
    pub fn timed_out(&self) -> bool {
        self.0
    }
}

/// A Condition Variable (single-threaded stub).
#[derive(Debug, Default)]
pub struct Condvar;

impl Condvar {
    /// Creates a new condition variable.
    #[must_use]
    #[inline]
    pub const fn new() -> Condvar {
        Condvar
    }

    /// Blocks the current thread (unsupported on single-threaded JS).
    pub fn wait<'a, T>(&self, _guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        panic!("Condvar::wait would deadlock on single-threaded JavaScript target")
    }

    /// Waits while `condition` returns `true` (unsupported on single-threaded JS).
    pub fn wait_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> LockResult<MutexGuard<'a, T>>
    where
        F: FnMut(&mut T) -> bool,
    {
        if condition(&mut *guard) {
            panic!("Condvar::wait_while would deadlock on single-threaded JavaScript target");
        }
        Ok(guard)
    }

    /// Wakes up one blocked thread (no-op on single-threaded JS).
    #[inline]
    pub fn notify_one(&self) {}

    /// Wakes up all blocked threads (no-op on single-threaded JS).
    #[inline]
    pub fn notify_all(&self) {}
}

/// Result returned by [`Barrier::wait`].
#[derive(Debug, Clone)]
pub struct BarrierWaitResult(bool);

impl BarrierWaitResult {
    /// Returns `true` if this thread is the barrier leader.
    #[must_use]
    #[inline]
    pub fn is_leader(&self) -> bool {
        self.0
    }
}

/// A barrier enabling multiple threads to synchronize (single-threaded stub).
#[derive(Debug)]
pub struct Barrier {
    num_threads: usize,
}

impl Barrier {
    /// Creates a new barrier that can block `n` threads.
    #[must_use]
    #[inline]
    pub const fn new(n: usize) -> Barrier {
        Barrier { num_threads: n }
    }

    /// Blocks until all threads reach this point.
    pub fn wait(&self) -> BarrierWaitResult {
        if self.num_threads <= 1 {
            BarrierWaitResult(true)
        } else {
            panic!("Barrier::wait with n > 1 would deadlock on single-threaded JavaScript target")
        }
    }
}
