//! Internal panic machinery: panic depth counter, payload slot, and unwind catching.

use crate::any::Any;
use crate::borrow::Cow;
use crate::boxed::Box;
use crate::cell::{Cell, UnsafeCell};
use crate::string::String;

/// Single-threaded global panic state.
///
/// Sound on single-threaded JavaScript targets (`singlethread = true`).
struct PanicState {
    depth: Cell<usize>,
    message: UnsafeCell<Option<Cow<'static, str>>>,
}

// SAFETY: `mires` targets single-threaded JavaScript execution (`singlethread = true`),
// so no concurrent thread can access `PanicState`.
unsafe impl Sync for PanicState {}

static PANIC_STATE: PanicState = PanicState {
    depth: Cell::new(0),
    message: UnsafeCell::new(None),
};

/// Returns `true` if the current execution is unwinding due to a panic.
#[inline]
pub(crate) fn panicking() -> bool {
    PANIC_STATE.depth.get() > 0
}

#[inline]
fn set_message(msg: Cow<'static, str>) {
    // SAFETY: Single-threaded execution; no overlapping borrows of `PANIC_STATE.message`.
    unsafe {
        *PANIC_STATE.message.get() = Some(msg);
    }
}

#[inline]
fn take_message() -> Option<Cow<'static, str>> {
    // SAFETY: Single-threaded execution; no overlapping borrows of `PANIC_STATE.message`.
    unsafe { (*PANIC_STATE.message.get()).take() }
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
        set_message(Cow::Borrowed(msg));
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
            set_message(Cow::Owned(msg_copy));
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
    let text = if let Some(s) = payload.downcast_ref::<&'static str>() {
        String::from(*s)
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::from("Box<dyn Any>")
    };
    if cfg!(panic = "unwind") {
        let depth = PANIC_STATE.depth.get();
        PANIC_STATE.depth.set(depth.saturating_add(1));
        set_message(Cow::Owned(text.clone()));
        crate::sys::panic_error(&text)
    } else {
        crate::sys::abort(&text)
    }
}

/// Invokes `f`, catching a Rust panic when compiled in unwind mode.
pub(crate) fn catch_unwind<F, R>(f: F) -> Result<R, Box<dyn Any + Send>>
where
    F: FnOnce() -> R,
{
    match catch_unwind_message(f) {
        Ok(val) => Ok(val),
        Err(msg) => Err(Box::new(msg)),
    }
}

/// Invokes `f`, catching a Rust panic and returning its message without requiring `dyn Any`.
#[doc(hidden)]
pub fn catch_unwind_message<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R,
{
    struct CatchData<F, R> {
        f: Option<F>,
        out: Option<R>,
    }

    extern "C-unwind" fn trampoline<F: FnOnce() -> R, R>(raw: *mut u8) -> i32 {
        // SAFETY: `raw` is a valid pointer to `CatchData<F, R>` passed by
        // `catch_unwind_message` below, alive for the synchronous JS `try` call.
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
        let msg = take_message().map_or_else(|| String::from("panicked"), Cow::into_owned);
        Err(msg)
    }
}
