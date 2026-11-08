//! The program. On a host throw, `metadata_host_throw` must drop the guard held by the middle crate's
//! only copy of its instance of `guarded`, and `shared_host_throw` the guards held by whichever copy
//! of `guarded::<fn() -> u32>` the link kept. See `cleanup_metadata_provider.rs`.

#![no_std]
#![no_main]

extern crate alloc;

unsafe extern "C" {
    fn host_throw();
}

fn throws() -> u32 {
    unsafe { host_throw() };
    5
}

fn returns() -> u32 {
    5
}

#[unsafe(no_mangle)]
pub fn metadata_host_throw(throwing: bool) -> u32 {
    cleanup_metadata_middle::run(throwing)
}

/// Guard 9 through the middle crate's call, guard 11 through this crate's own.
#[unsafe(no_mangle)]
pub fn shared_host_throw(throwing: bool, own: bool) -> u32 {
    let f: fn() -> u32 = if throwing { throws } else { returns };
    if own {
        cleanup_metadata_provider::guarded(11, f)
    } else {
        cleanup_metadata_middle::shared(f)
    }
}

/// `String::from_utf16le` collects through `core::iter::adapters::try_process`, an instance of a
/// `core` generic that only the sysroot's `alloc` holds, and cargo compiled `alloc` against
/// `core`'s `.rmeta` alone. The test reads that instance's row in the lane report; it does not call
/// this, because the runtime cannot read a byte buffer through `as_chunks::<2>`.
#[unsafe(no_mangle)]
pub fn utf16le_probe(bytes: &[u8]) -> u32 {
    alloc::string::String::from_utf16le(bytes).map_or(0, |text| text.len() as u32)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
