//! The dependency of 298: an `extern "C"` import, in a crate 298's test compiles as
//! `--crate-name compiler_builtins`.

#![no_std]

unsafe extern "C" {
    pub fn host_note(value: f64);
}
