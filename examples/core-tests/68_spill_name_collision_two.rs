//! Two spill registers, and the program already binds both preferred names.
//!
//! `mix` writes both fields of its `&mut` parameter *and* returns a value of its own, so neither
//! out-field can leave through the return: the plan allocates two registers. The program binds a
//! static called `$r1` and a function called `$r2`, so both registers are named around a name the
//! program owns — one of them a module-scope binding, which is exactly what a register is.

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
fn mix(acc: &mut Pair, k: u32) -> u32 {
    acc.high = acc.high.wrapping_mul(31).wrapping_add(k);
    acc.low = (acc.low ^ k).wrapping_add(acc.high);
    acc.high ^ acc.low
}

/// The program's own `$r1`: a static, which is the same shape as a spill register.
#[unsafe(export_name = "$r1")]
pub static SEED: u32 = 7;

/// The program's own `$r2`. It also calls `mix` once (observably inert), so `mix` has two
/// callers and the Phase-5 inliner's single-caller criterion leaves the loop's call standing —
/// this fixture exists to make the flat-ABI plan allocate spill registers, which needs a call.
#[unsafe(export_name = "$r2")]
pub extern "C" fn other() -> u32 {
    let mut scratch = Pair { high: 1, low: 2 };
    let _ = mix(&mut scratch, 3);
    1000
}

#[no_mangle]
pub fn rust_entry() {
    let mut acc = Pair {
        high: SEED,
        low: SEED ^ 0xffff,
    };
    let mut i = 0_u32;
    let mut out = 0_u32;
    while i < 4 {
        out = out.wrapping_add(mix(&mut acc, i));
        i += 1;
    }
    print_u32(out);
    print_u32(acc.high ^ acc.low);
    print_u32(other());
}
