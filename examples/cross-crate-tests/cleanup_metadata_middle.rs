//! Instantiates `cleanup_metadata_provider::guarded` here, with a closure whose host call throws on
//! request. See `cleanup_metadata_provider.rs`.

#![no_std]

unsafe extern "C-unwind" {
    fn host_throw();
}

#[inline(never)]
fn call_host(throwing: bool) -> u32 {
    if throwing {
        unsafe { host_throw() };
    }
    5
}

pub fn run(throwing: bool) -> u32 {
    cleanup_metadata_provider::guarded(7, move || call_host(throwing))
}

/// The middle crate's copy of `guarded::<fn() -> u32>`; the client has one too.
pub fn shared(f: fn() -> u32) -> u32 {
    cleanup_metadata_provider::guarded(9, f)
}
