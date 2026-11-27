//! Thread support for single-threaded JavaScript targets.
//!
//! Provides working [`LocalKey`], [`panicking`], [`current`], [`Thread`],
//! [`ThreadId`], [`yield_now`], and [`available_parallelism`], while
//! [`spawn`] and [`Builder::spawn`] honestly report unsupported errors just
//! like upstream `wasm32-unknown-unknown`.

mod local;

use crate::any::Any;
use crate::boxed::Box;
use crate::io;
use crate::marker::PhantomData;
use crate::num::NonZeroUsize;
use crate::string::String;
use crate::time::Duration;

pub use local::{AccessError, LocalKey, __ConstLocalStorage, __LazyLocalStorage};

/// A specialized [`core::result::Result`] type for threads and [`crate::panic::catch_unwind`].
pub type Result<T> = core_crate::result::Result<T, Box<dyn Any + Send + 'static>>;

/// A unique identifier for a running thread.
#[derive(Eq, PartialEq, Clone, Copy, Hash, Debug)]
pub struct ThreadId(u64);

/// A handle to a thread.
#[derive(Clone, Debug)]
pub struct Thread {
    id: ThreadId,
}

impl Thread {
    /// Atomically makes the handle's token available (no-op on single-threaded JS).
    #[inline]
    pub fn unpark(&self) {}

    /// Gets the thread's unique identifier.
    #[must_use]
    #[inline]
    pub fn id(&self) -> ThreadId {
        self.id
    }

    /// Gets the thread's name (`Some("main")` on the single JS thread).
    #[must_use]
    #[inline]
    pub fn name(&self) -> Option<&str> {
        Some("main")
    }
}

/// An owned permission to join on a thread (cannot be created on single-threaded JS).
#[derive(Debug)]
pub struct JoinHandle<T> {
    _marker: PhantomData<T>,
}

impl<T> JoinHandle<T> {
    /// Extracts a handle to the underlying thread.
    #[must_use]
    pub fn thread(&self) -> &Thread {
        unreachable!("JoinHandle cannot be constructed on single-threaded JavaScript")
    }

    /// Waits for the associated thread to finish.
    pub fn join(self) -> Result<T> {
        unreachable!("JoinHandle cannot be constructed on single-threaded JavaScript")
    }

    /// Checks if the associated thread has finished running.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        unreachable!("JoinHandle cannot be constructed on single-threaded JavaScript")
    }
}

/// Thread factory, which can be used to configure the properties of a new thread.
#[derive(Debug, Default)]
pub struct Builder {
    name: Option<String>,
    stack_size: Option<usize>,
}

impl Builder {
    /// Generates the base configuration for spawning a thread.
    #[must_use]
    #[inline]
    pub fn new() -> Builder {
        Builder {
            name: None,
            stack_size: None,
        }
    }

    /// Names the thread-to-be.
    #[must_use]
    #[inline]
    pub fn name(mut self, name: String) -> Builder {
        self.name = Some(name);
        self
    }

    /// Sets the size of the stack (in bytes) for the new thread.
    #[must_use]
    #[inline]
    pub fn stack_size(mut self, size: usize) -> Builder {
        self.stack_size = Some(size);
        self
    }

    /// Spawns a new thread — always returns [`io::ErrorKind::Unsupported`] on JS.
    pub fn spawn<F, T>(self, _f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        Err(io::Error::unsupported(
            "spawning threads is not supported on JavaScript targets",
        ))
    }
}

/// Spawns a new thread — panics with unsupported error matching `wasm32-unknown-unknown`.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new()
        .spawn(f)
        .expect("failed to spawn thread")
}

/// Gets a handle to the thread that invokes it.
#[must_use]
#[inline]
pub fn current() -> Thread {
    Thread { id: ThreadId(1) }
}

/// Cooperatively gives up a timeslice (no-op on single-threaded JS).
#[inline]
pub fn yield_now() {}

/// Determines whether the current thread is unwinding because of panic.
#[must_use]
#[inline]
pub fn panicking() -> bool {
    crate::panicking::panicking()
}

/// Puts the current thread to sleep (unsupported blocking sleep on JS).
pub fn sleep(_dur: Duration) {
    panic!("thread::sleep is not supported on single-threaded JavaScript targets")
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    panic!("thread::park would deadlock on single-threaded JavaScript targets")
}

/// Blocks unless or until the current thread's token is made available or `dur` elapses.
pub fn park_timeout(_dur: Duration) {
    panic!("thread::park_timeout is not supported on single-threaded JavaScript targets")
}

/// Returns an estimate of the default amount of parallelism a program should use (`1`).
#[inline]
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    // SAFETY: `1` is non-zero.
    Ok(unsafe { NonZeroUsize::new_unchecked(1) })
}
