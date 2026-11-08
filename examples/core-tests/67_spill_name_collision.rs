//! A program symbol spelled like a spill register.
//!
//! The flat direct-call ABI copies a `&mut` parameter's extra out-fields back through module
//! scope `let $r1; let $r2; ...` bindings the link step synthesizes. Nothing stops a *program*
//! from binding those names first — `#[unsafe(export_name = "$r1")]` is all it takes — and the
//! synthesized item must not overwrite it. The link step allocates each register a name the item
//! table does not already use, so the export below keeps `$r1` and the register moves aside.
//!
//! `step` is the flat callee: two fields copied back, one through the return value and one
//! through a register, so a register is really allocated. Its numbers are `65_flat_abi_spill`'s.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    high: u32,
    low: u32,
}

#[inline(never)]
fn step(acc: &mut Pair, k: Pair) {
    acc.high = acc.high.wrapping_mul(5).wrapping_add(k.high);
    acc.low = (acc.low ^ k.low).wrapping_add(acc.high);
}

/// The program's own `$r1`: an exported function, so the link step may neither rename it nor
/// drop it.
#[unsafe(export_name = "$r1")]
pub extern "C" fn collision() -> u32 {
    1000
}

#[no_mangle]
pub fn rust_entry() {
    let mut acc = Pair {
        high: 1,
        low: 1 ^ 0xffff,
    };
    let mut i = 0_u32;
    while i < 4 {
        step(
            &mut acc,
            Pair {
                high: i,
                low: i.wrapping_mul(7),
            },
        );
        i += 1;
    }
    print_u32(acc.high ^ acc.low);
    print_u32(collision());
}
