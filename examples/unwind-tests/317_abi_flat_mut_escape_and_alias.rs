//! Negative control 3 for Slice R3-A:
//! 1. Escaping pointer-identity casts (`state as *const State`, `acc as *const i32`) decline
//!    (`ParamEscapes`) and preserve exact JS reference identity.
//! 2. Shared `&i32` alongside `&mut i32` in a writing callee is NOT flattened (`purity > Read`),
//!    preserving visibility of mutations through an aliased pointer.

#![no_std]
#![no_main]

#[allow(dead_code)]
enum State {
    Idle,
    Loading { progress: i32, retries: i32 },
    Ready { value: i32 },
}

#[inline(never)]
fn check_ptr_identity(
    state: &mut State,
    acc: &mut i32,
    expected_s: *const State,
    expected_a: *const i32,
) -> bool {
    *acc = 5;
    *state = State::Ready { value: 7 };
    core::ptr::eq(state as *const State, expected_s)
        && core::ptr::eq(acc as *const i32, expected_a)
}

#[inline(never)]
fn shared_alias_helper(r: &i32, w: &mut i32) -> i32 {
    *w = 42;
    *r
}

#[unsafe(no_mangle)]
pub fn test_ptr_identity() -> i32 {
    let mut state = State::Loading {
        progress: 1,
        retries: 2,
    };
    let mut acc = 1;
    let s_ptr: *const State = &raw const state;
    let a_ptr: *const i32 = &raw const acc;
    let same = check_ptr_identity(&mut state, &mut acc, s_ptr, a_ptr);
    if same && acc == 5 {
        match state {
            State::Ready { value } => value,
            _ => -1,
        }
    } else {
        -2
    }
}

#[unsafe(no_mangle)]
pub fn test_shared_alias() -> i32 {
    let mut v = 10;
    let p: *mut i32 = &raw mut v;
    unsafe { shared_alias_helper(&*p, &mut *p) }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
