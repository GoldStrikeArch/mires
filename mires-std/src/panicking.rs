//! Internal panic machinery: panic depth counter, payload slot, and unwind catching.

use crate::any::Any;
use crate::boxed::Box;
use crate::cell::{Cell, UnsafeCell};
use crate::string::String;

/// Stored payload for an in-flight unwinding panic.
///
/// Keeping static strings and formatted `String`s unboxed in dedicated variants
/// avoids instantiating `dyn Any` vtables in programs that never call
/// `catch_unwind` or `panic_any`, while preserving the exact payload type
/// (`&'static str`, `String`, or arbitrary `Box<dyn Any + Send>`) when caught.
enum PanicPayload {
    StaticStr(&'static str),
    OwnedString(String),
    Any(Box<dyn Any + Send>),
}

impl PanicPayload {
    fn into_any(self) -> Box<dyn Any + Send> {
        match self {
            Self::StaticStr(s) => Box::new(s),
            Self::OwnedString(s) => Box::new(s),
            Self::Any(b) => b,
        }
    }

    fn into_message(self) -> String {
        match self {
            Self::StaticStr(s) => String::from(s),
            Self::OwnedString(s) => s,
            Self::Any(b) => payload_message(&*b),
        }
    }
}

fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        String::from(*s)
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::from("Box<dyn Any>")
    }
}

/// Single-threaded global panic state.
///
/// Sound on single-threaded JavaScript targets (`singlethread = true`).
struct PanicState {
    depth: Cell<usize>,
    payload: UnsafeCell<Option<PanicPayload>>,
}

// SAFETY: `mires` targets single-threaded JavaScript execution (`singlethread = true`),
// so no concurrent thread can access `PanicState`.
unsafe impl Sync for PanicState {}

static PANIC_STATE: PanicState = PanicState {
    depth: Cell::new(0),
    payload: UnsafeCell::new(None),
};

/// Returns `true` if the current execution is unwinding due to a panic.
#[inline]
pub(crate) fn panicking() -> bool {
    PANIC_STATE.depth.get() > 0
}

#[inline]
fn set_payload(payload: PanicPayload) {
    // SAFETY: Single-threaded execution; no overlapping borrows of `PANIC_STATE.payload`.
    unsafe {
        *PANIC_STATE.payload.get() = Some(payload);
    }
}

#[inline]
fn take_payload() -> Option<PanicPayload> {
    // SAFETY: Single-threaded execution; no overlapping borrows of `PANIC_STATE.payload`.
    unsafe { (*PANIC_STATE.payload.get()).take() }
}

/// Raises a panic with a static message `&'static str`.
///
/// Allocates nothing and avoids `core::fmt` and `dyn Any` so ordinary programs
/// stay small and free of `TypeId` dependencies.
#[inline(never)]
#[cold]
pub(crate) fn panic_with_static_str(msg: &'static str) -> ! {
    if cfg!(panic = "unwind") {
        let depth = PANIC_STATE.depth.get();
        PANIC_STATE.depth.set(depth.saturating_add(1));
        set_payload(PanicPayload::StaticStr(msg));
        crate::sys::panic_error(msg)
    } else {
        crate::sys::abort(msg)
    }
}

/// Raises a panic with a formatted message `fmt::Arguments`.
///
/// Uses the static-string fast path when no runtime formatting is needed.
#[inline(never)]
#[cold]
pub(crate) fn panic_with_fmt(args: crate::fmt::Arguments<'_>) -> ! {
    if let Some(s) = args.as_str() {
        panic_with_static_str(s)
    } else {
        let text = alloc_crate::fmt::format(args);
        if cfg!(panic = "unwind") {
            let depth = PANIC_STATE.depth.get();
            PANIC_STATE.depth.set(depth.saturating_add(1));
            let msg_copy = text.clone();
            set_payload(PanicPayload::OwnedString(msg_copy));
            crate::sys::panic_error(&text)
        } else {
            crate::sys::abort(&text)
        }
    }
}

/// Entry point for panicking with an arbitrary payload (`panic_any`).
#[inline(never)]
#[cold]
pub fn begin_panic<M: Any + Send>(msg: M) -> ! {
    let boxed: Box<dyn Any + Send> = Box::new(msg);
    resume_unwind(boxed)
}

/// Resumes unwinding with a boxed panic payload.
#[inline(never)]
#[cold]
pub(crate) fn resume_unwind(payload: Box<dyn Any + Send>) -> ! {
    let text = payload_message(&*payload);
    if cfg!(panic = "unwind") {
        let depth = PANIC_STATE.depth.get();
        PANIC_STATE.depth.set(depth.saturating_add(1));
        set_payload(PanicPayload::Any(payload));
        crate::sys::panic_error(&text)
    } else {
        crate::sys::abort(&text)
    }
}

fn catch_unwind_payload<F, R>(f: F) -> Result<R, Option<PanicPayload>>
where
    F: FnOnce() -> R,
{
    struct CatchData<F, R> {
        f: Option<F>,
        out: Option<R>,
    }

    extern "C-unwind" fn trampoline<F: FnOnce() -> R, R>(raw: *mut u8) -> i32 {
        // SAFETY: `raw` is a valid pointer to `CatchData<F, R>` passed by
        // `catch_unwind_payload` below, alive for the synchronous JS `try` call.
        let data = unsafe { &mut *(raw as *mut CatchData<F, R>) };
        if let Some(func) = data.f.take() {
            data.out = Some(func());
        }
        0
    }

    let prev_depth = PANIC_STATE.depth.get();
    let mut data = CatchData {
        f: Some(f),
        out: None,
    };
    let raw = (&mut data as *mut CatchData<F, R>).cast::<u8>();
    let res = crate::sys::catch_unwind_raw(trampoline::<F, R>, raw);
    if res.ok {
        match data.out {
            Some(val) => Ok(val),
            None => crate::sys::abort("catch_unwind trampoline returned without output"),
        }
    } else {
        PANIC_STATE.depth.set(prev_depth);
        Err(take_payload())
    }
}

/// Invokes `f`, catching a Rust panic when compiled in unwind mode.
pub(crate) fn catch_unwind<F, R>(f: F) -> Result<R, Box<dyn Any + Send>>
where
    F: FnOnce() -> R,
{
    match catch_unwind_payload(f) {
        Ok(val) => Ok(val),
        Err(Some(payload)) => Err(payload.into_any()),
        Err(None) => Err(Box::new("panicked")),
    }
}

/// Invokes `f`, catching a Rust panic and returning its message without requiring `dyn Any`.
#[doc(hidden)]
pub fn catch_unwind_message<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R,
{
    match catch_unwind_payload(f) {
        Ok(val) => Ok(val),
        Err(Some(payload)) => Err(payload.into_message()),
        Err(None) => Err(String::from("panicked")),
    }
}
