//! FAIL (round 3): a one-element slice over a `static` whose JavaScript value is not an object
//! (`()`, a single-variant fieldless enum: both are emitted as `undefined`) throws while the module
//! loads: `__rt.box(undefined)` -> "TypeError: Invalid value used as weak map key".
//! Native (-O0, -O3): 1 1 7. Round-2 head 8586d19: 1 1 7. e2245df, all four modes: TypeError at
//! load, nothing printed. Main (ea5604f): TypeError/abort at run time on first use of the slice.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

enum Only {
    One,
}
static Z: () = ();
static UNITS: &[()] = core::slice::from_ref(&Z);
static O: Only = Only::One;
static ONLY: &[Only] = core::slice::from_ref(&O);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(UNITS.len());
    print_usize(ONLY.iter().filter(|o| matches!(o, Only::One)).count());
    print_u32(7);
}
