//! 293 across crates: the `extern "C" fn` is a dependency's (`deps/299_dep.rs`, passed as
//! `--extern unwind_dep`), and its body is compiled in that crate, not here.
//!
//! Under `-Cpanic=unwind` (`JSC_PANIC=unwind`) the call to `unwind_dep::middle` loses its cleanup
//! edge as 293's does, and here the callee's body is not even available to show that no host
//! exception leaves it, so the backend refuses the call and names `extern "C-unwind"`. Under the
//! default `-Cpanic=abort` the program compiles and `js-cleanup=on` keeps the drop on a host throw.

#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(value: f64);
}

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(self.0) };
    }
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_dependency(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    unwind_dep::middle(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
