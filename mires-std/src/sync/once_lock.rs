//! One-time initialization primitives: `Once`, `OnceState`, `OnceLock`, and `LazyLock`.

use crate::cell::{Cell, UnsafeCell};
use crate::fmt;
use crate::ops::Deref;

/// State yielded to [`Once::call_once_force`].
#[derive(Debug)]
pub struct OnceState {
    poisoned: bool,
}

impl OnceState {
    /// Returns `true` if the associated [`Once`] was poisoned prior to this invocation.
    #[must_use]
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
}

const ONCE_INCOMPLETE: u8 = 0;
const ONCE_RUNNING: u8 = 1;
const ONCE_COMPLETE: u8 = 2;
const ONCE_POISONED: u8 = 3;

/// A synchronization primitive for running a one-time global initialization.
pub struct Once {
    state: Cell<u8>,
}

// SAFETY: `mires` targets single-threaded JS execution (`singlethread = true`).
unsafe impl Sync for Once {}
// SAFETY: `mires` targets single-threaded JS execution (`singlethread = true`).
unsafe impl Send for Once {}

impl Once {
    /// Creates a new `Once` value.
    #[must_use]
    #[inline]
    pub const fn new() -> Once {
        Once {
            state: Cell::new(ONCE_INCOMPLETE),
        }
    }

    /// Performs an initialization routine once and only once.
    #[inline]
    pub fn call_once<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        if self.state.get() == ONCE_COMPLETE {
            return;
        }
        self.call_once_slow(false, |_| f());
    }

    /// Performs the same function as [`call_once`](Once::call_once) except ignores poisoning.
    #[inline]
    pub fn call_once_force<F>(&self, f: F)
    where
        F: FnOnce(&OnceState),
    {
        if self.state.get() == ONCE_COMPLETE {
            return;
        }
        self.call_once_slow(true, f);
    }

    #[cold]
    fn call_once_slow<F>(&self, ignore_poisoning: bool, f: F)
    where
        F: FnOnce(&OnceState),
    {
        let current = self.state.get();
        if current == ONCE_RUNNING {
            panic!("Once instance cannot be re-entered recursively");
        }
        let poisoned = current == ONCE_POISONED;
        if poisoned && !ignore_poisoning {
            panic!("Once instance has previously been poisoned");
        }
        struct FinishGuard<'a> {
            state: &'a Cell<u8>,
            done: bool,
        }
        impl Drop for FinishGuard<'_> {
            fn drop(&mut self) {
                if !self.done {
                    self.state.set(ONCE_POISONED);
                }
            }
        }
        self.state.set(ONCE_RUNNING);
        let mut guard = FinishGuard {
            state: &self.state,
            done: false,
        };
        let once_state = OnceState { poisoned };
        f(&once_state);
        guard.done = true;
        self.state.set(ONCE_COMPLETE);
    }

    /// Returns `true` if some `call_once` call has completed successfully.
    #[must_use]
    #[inline]
    pub fn is_completed(&self) -> bool {
        self.state.get() == ONCE_COMPLETE
    }
}

impl Default for Once {
    #[inline]
    fn default() -> Once {
        Once::new()
    }
}

impl fmt::Debug for Once {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Once").finish_non_exhaustive()
    }
}

/// A synchronization primitive which can nominally be written to only once.
pub struct OnceLock<T> {
    initializing: Cell<bool>,
    value: UnsafeCell<Option<T>>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`); `OnceLock`
// only hands out shared `&T` once initialized and never mutates afterward.
unsafe impl<T: Send> Send for OnceLock<T> {}
// SAFETY: Same single-threaded guarantee.
unsafe impl<T: Sync + Send> Sync for OnceLock<T> {}

impl<T> OnceLock<T> {
    /// Creates a new empty cell.
    #[must_use]
    #[inline]
    pub const fn new() -> OnceLock<T> {
        OnceLock {
            initializing: Cell::new(false),
            value: UnsafeCell::new(None),
        }
    }

    /// Gets a reference to the underlying value, if initialized.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        // SAFETY: Single-threaded; no mutable reference exists while `initializing` is false,
        // and when `initializing` is true `value` is `None`.
        unsafe { (*self.value.get()).as_ref() }
    }

    /// Gets a mutable reference to the underlying value, if initialized.
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.value.get_mut().as_mut()
    }

    /// Sets the contents of this cell to `value`.
    #[inline]
    pub fn set(&self, value: T) -> Result<(), T> {
        let mut value = Some(value);
        self.get_or_init(|| value.take().unwrap());
        match value {
            None => Ok(()),
            Some(v) => Err(v),
        }
    }

    /// Gets the contents of the cell, initializing it with `f` if empty.
    #[inline]
    pub fn get_or_init<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        if let Some(val) = self.get() {
            return val;
        }
        self.init_slow(f)
    }

    #[cold]
    fn init_slow<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        if self.initializing.get() {
            panic!("re-entrant initialization of OnceLock");
        }
        struct ResetGuard<'a>(&'a Cell<bool>);
        impl Drop for ResetGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        self.initializing.set(true);
        let _guard = ResetGuard(&self.initializing);
        let val = f();
        // SAFETY: Single-threaded and guarded against re-entry; no reference into `value` exists yet.
        unsafe {
            *self.value.get() = Some(val);
            (*self.value.get()).as_ref().unwrap()
        }
    }

    /// Takes the value out of this `OnceLock`, moving it back to an uninitialized state.
    #[inline]
    pub fn take(&mut self) -> Option<T> {
        self.value.get_mut().take()
    }

    /// Consumes the `OnceLock`, returning the wrapped value.
    #[inline]
    pub fn into_inner(self) -> Option<T> {
        self.value.into_inner()
    }
}

impl<T> Default for OnceLock<T> {
    #[inline]
    fn default() -> OnceLock<T> {
        OnceLock::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for OnceLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("OnceLock");
        match self.get() {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}

impl<T: Clone> Clone for OnceLock<T> {
    #[inline]
    fn clone(&self) -> OnceLock<T> {
        let cell = OnceLock::new();
        if let Some(value) = self.get() {
            let _ = cell.set(value.clone());
        }
        cell
    }
}

impl<T> From<T> for OnceLock<T> {
    #[inline]
    fn from(value: T) -> Self {
        OnceLock {
            initializing: Cell::new(false),
            value: UnsafeCell::new(Some(value)),
        }
    }
}

impl<T: PartialEq> PartialEq for OnceLock<T> {
    #[inline]
    fn eq(&self, other: &OnceLock<T>) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq> Eq for OnceLock<T> {}

enum LazyState<T, F> {
    Uninit(F),
    Poisoned,
    Init(T),
}

/// A value which is initialized on the first access.
pub struct LazyLock<T, F = fn() -> T> {
    state: UnsafeCell<LazyState<T, F>>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`); initialization
// transitions through `Poisoned` so re-entrancy panics instead of aliasing.
unsafe impl<T: Send + Sync, F: Send> Sync for LazyLock<T, F> {}
// SAFETY: Matches upstream `std` `Send` bound.
unsafe impl<T: Send, F: Send> Send for LazyLock<T, F> {}

impl<T, F> LazyLock<T, F> {
    /// Creates a new lazy value with the given initializing function.
    #[inline]
    pub const fn new(f: F) -> LazyLock<T, F> {
        LazyLock {
            state: UnsafeCell::new(LazyState::Uninit(f)),
        }
    }
}

impl<T, F: FnOnce() -> T> LazyLock<T, F> {
    /// Forces the evaluation of this lazy value and returns a reference to the result.
    #[inline]
    pub fn force(this: &LazyLock<T, F>) -> &T {
        // SAFETY: Single-threaded execution; we check the tag and only mutate
        // when transitioning from `Uninit` to `Poisoned` to `Init`.
        unsafe {
            match &*this.state.get() {
                LazyState::Init(val) => val,
                LazyState::Poisoned => {
                    panic!("LazyLock instance has previously been poisoned")
                }
                LazyState::Uninit(_) => Self::force_slow(this),
            }
        }
    }

    #[cold]
    fn force_slow(this: &LazyLock<T, F>) -> &T {
        // SAFETY: Single-threaded; no live references into `state` exist when `Uninit`.
        let f = unsafe {
            match crate::mem::replace(&mut *this.state.get(), LazyState::Poisoned) {
                LazyState::Uninit(f) => f,
                LazyState::Poisoned => panic!("LazyLock instance has previously been poisoned"),
                LazyState::Init(_) => unreachable!(),
            }
        };
        let value = f();
        // SAFETY: Single-threaded; `state` is `Poisoned` with no references into it.
        unsafe {
            *this.state.get() = LazyState::Init(value);
            match &*this.state.get() {
                LazyState::Init(val) => val,
                _ => unreachable!(),
            }
        }
    }

    /// Forces the evaluation of this lazy value and returns a mutable reference.
    #[inline]
    pub fn force_mut(this: &mut LazyLock<T, F>) -> &mut T {
        let _ = Self::force(this);
        match this.state.get_mut() {
            LazyState::Init(val) => val,
            _ => unreachable!(),
        }
    }
}

impl<T, F: FnOnce() -> T> Deref for LazyLock<T, F> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        LazyLock::force(self)
    }
}

impl<T: Default> Default for LazyLock<T> {
    #[inline]
    fn default() -> LazyLock<T> {
        LazyLock::new(T::default)
    }
}

impl<T: fmt::Debug, F> fmt::Debug for LazyLock<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("LazyLock");
        // SAFETY: Single-threaded read of the state tag.
        let val = unsafe {
            match &*self.state.get() {
                LazyState::Init(v) => Some(v),
                _ => None,
            }
        };
        match val {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}
