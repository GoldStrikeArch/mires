//! The dependency of 299: a non-generic `extern "C" fn` that calls a throwing host import.

#![no_std]

unsafe extern "C-unwind" {
    fn host_note(value: f64);
}

#[inline(never)]
pub extern "C" fn middle(throwing: bool) {
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
}
