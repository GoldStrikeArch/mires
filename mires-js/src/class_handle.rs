//! Safe raw-pointer lifecycle helpers for `#[js_export]` stateful JavaScript classes.
//!
//! Macro-generated class wrappers call these helpers so user crates compiled with
//! `#![forbid(unsafe_code)]` can export stateful `impl` blocks without emitting any `unsafe`
//! blocks in user code.

use alloc::boxed::Box;

/// Panic message emitted when a JavaScript wrapper invokes a method on a freed class handle.
const FREED_HANDLE_MSG: &str = "mires class handle was already freed";

/// Moves `value` onto the heap and returns an opaque handle pointer for JavaScript class state.
#[must_use]
pub fn into_raw<T>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

/// Borrows the heap-allocated class instance behind `ptr` immutably and runs `f`.
///
/// # Panics
///
/// Panics with `"mires class handle was already freed"` if `ptr` is null.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "macro-generated #[js_export] class wrappers call this in #![forbid(unsafe_code)] user crates with handles created by into_raw"
)]
pub fn with_ref<T, R>(ptr: *mut T, f: impl FnOnce(&T) -> R) -> R {
    assert!(!ptr.is_null(), "{FREED_HANDLE_MSG}");
    #[allow(
        unsafe_code,
        reason = "dereferences boxed class handle produced by into_raw"
    )]
    // SAFETY: `ptr` is non-null and was created by `into_raw` for a live `Box<T>`. The single-
    // threaded JS class wrapper nulls the handle on drop/consumption, so `&*ptr` is valid for `f`.
    let value: &T = unsafe { &*ptr };
    f(value)
}

/// Borrows the heap-allocated class instance behind `ptr` mutably and runs `f`.
///
/// # Panics
///
/// Panics with `"mires class handle was already freed"` if `ptr` is null.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "macro-generated #[js_export] class wrappers call this in #![forbid(unsafe_code)] user crates with handles created by into_raw"
)]
pub fn with_mut<T, R>(ptr: *mut T, f: impl FnOnce(&mut T) -> R) -> R {
    assert!(!ptr.is_null(), "{FREED_HANDLE_MSG}");
    #[allow(
        unsafe_code,
        reason = "dereferences boxed class handle produced by into_raw"
    )]
    // SAFETY: `ptr` is non-null and was created by `into_raw` for a live `Box<T>`. The JS class
    // wrapper owns the handle exclusively on the single-threaded JS runtime.
    let value: &mut T = unsafe { &mut *ptr };
    f(value)
}

/// Reclaims ownership of the heap-allocated class instance behind `ptr`, freeing its box.
///
/// # Panics
///
/// Panics with `"mires class handle was already freed"` if `ptr` is null.
#[must_use]
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "macro-generated #[js_export] class wrappers call this in #![forbid(unsafe_code)] user crates with handles created by into_raw"
)]
pub fn into_owned<T>(ptr: *mut T) -> T {
    assert!(!ptr.is_null(), "{FREED_HANDLE_MSG}");
    #[allow(
        unsafe_code,
        reason = "reclaims boxed class handle produced by into_raw"
    )]
    // SAFETY: `ptr` is non-null and was allocated by `Box::into_raw` in `into_raw`. Ownership is
    // transferred back once and the JS wrapper clears its pointer slot immediately.
    *unsafe { Box::from_raw(ptr) }
}

/// Drops the heap-allocated class instance behind `ptr`, ignoring null pointers.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "macro-generated #[js_export] class wrappers call this in #![forbid(unsafe_code)] user crates with handles created by into_raw"
)]
pub fn drop_raw<T>(ptr: *mut T) {
    if !ptr.is_null() {
        #[allow(
            unsafe_code,
            reason = "reclaims and drops boxed class handle produced by into_raw"
        )]
        // SAFETY: `ptr` is non-null and was allocated by `Box::into_raw` in `into_raw`. The JS
        // wrapper clears the handle slot so `drop_raw` runs at most once per allocation.
        let boxed = unsafe { Box::from_raw(ptr) };
        drop(boxed);
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::string::String;
    use core::cell::Cell;

    use super::*;

    struct Counter {
        value: i32,
        label: String,
    }

    struct DropTracker {
        drops: Rc<Cell<usize>>,
    }

    impl Drop for DropTracker {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    #[test]
    fn lifecycle_into_raw_ref_mut_and_into_owned() {
        let ptr = into_raw(Counter {
            value: 10,
            label: String::from("clicks"),
        });
        assert!(!ptr.is_null());

        let initial = with_ref(ptr, |c| (c.value, c.label.len()));
        assert_eq!(initial, (10, 6));

        let updated = with_mut(ptr, |c| {
            c.value += 5;
            c.label.push('!');
            c.value
        });
        assert_eq!(updated, 15);

        let owned = into_owned(ptr);
        assert_eq!(owned.value, 15);
        assert_eq!(owned.label, "clicks!");
    }

    #[test]
    fn drop_raw_runs_drop_and_ignores_null() {
        let drops = Rc::new(Cell::new(0usize));
        let ptr = into_raw(DropTracker {
            drops: Rc::clone(&drops),
        });
        assert_eq!(drops.get(), 0);

        drop_raw(ptr);
        assert_eq!(drops.get(), 1);

        drop_raw(core::ptr::null_mut::<DropTracker>());
        assert_eq!(drops.get(), 1);
    }

    #[test]
    #[should_panic(expected = "mires class handle was already freed")]
    fn with_ref_panics_on_null() {
        let _ = with_ref(core::ptr::null_mut::<i32>(), |v| *v);
    }

    #[test]
    #[should_panic(expected = "mires class handle was already freed")]
    fn with_mut_panics_on_null() {
        with_mut(core::ptr::null_mut::<i32>(), |v| *v += 1);
    }

    #[test]
    #[should_panic(expected = "mires class handle was already freed")]
    fn into_owned_panics_on_null() {
        let _ = into_owned(core::ptr::null_mut::<i32>());
    }
}
