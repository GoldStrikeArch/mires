//! 290's `extern "C"` import declared in a dependency that is merely named like a sysroot crate.
//!
//! `scripts/unwind-mode-test.sh` compiles `deps/298_dep.rs` with `--crate-name compiler_builtins`
//! and passes it as `--extern sysroot_named`. The imports of the sysroot's own crates are the
//! runtime's and are accepted, but which crates those are is decided by where rustc loaded the
//! crate from, not by its name: this one is the program's, so under `-Cpanic=unwind`
//! (`JSC_PANIC=unwind`) the call is refused like 290's, naming `extern "C-unwind"`. Under the
//! default `-Cpanic=abort` the program compiles and `js-cleanup=on` keeps the drop on a host throw.

#![no_std]
#![no_main]

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { sysroot_named::host_note(self.0) };
    }
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_import(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    unsafe { sysroot_named::host_note(if throwing { -1.0 } else { 2.0 }) };
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
