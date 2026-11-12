//! Initialisation order: one-element slices named before/after their statics under names that sort
//! either way, chains of them, and uses from other modules.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::ptr;
use core::slice::from_ref;

pub struct N { id: u32 }

static AAA_SL: &[N] = from_ref(&ZZZ);
static ZZZ: N = N { id: 1 };
static ZZZ_SL: &[N] = from_ref(&AAA);
static AAA: N = N { id: 2 };
mod a {
    use super::*;
    pub static SL: &[N] = from_ref(&super::z::TARGET);
    pub static TARGET: N = N { id: 3 };
}
mod z {
    use super::*;
    pub static SL: &[N] = from_ref(&super::a::TARGET);
    pub static TARGET: N = N { id: 4 };
}
// a chain: L3 -> L2 -> L1 -> BASE, declared in reverse
struct L { next: &'static [N], id: u32 }
struct LL { next: &'static [L], id: u32 }
static L3: &[LL] = from_ref(&L2);
static L2: LL = LL { next: from_ref(&L1), id: 30 };
static L1: L = L { next: from_ref(&BASE), id: 20 };
static BASE: N = N { id: 10 };
// many slices over one static, and a table of slices over many statics
static M0: N = N { id: 100 };
static M1: N = N { id: 101 };
static M2: N = N { id: 102 };
static TABLE: [&[N]; 6] = [from_ref(&M2), from_ref(&M0), from_ref(&M1), from_ref(&M0), from_ref(&ZZZ), from_ref(&BASE)];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(AAA_SL[0].id); print_bool(ptr::eq(AAA_SL.as_ptr(), &ZZZ));
    print_u32(ZZZ_SL[0].id); print_bool(ptr::eq(ZZZ_SL.as_ptr(), &AAA));
    print_u32(a::SL[0].id); print_bool(ptr::eq(a::SL.as_ptr(), &z::TARGET));
    print_u32(z::SL[0].id); print_bool(ptr::eq(z::SL.as_ptr(), &a::TARGET));
    print_u32(L3[0].id + L3[0].next[0].id + L3[0].next[0].next[0].id);
    print_bool(ptr::eq(L3.as_ptr(), &L2)); print_bool(ptr::eq(L3[0].next.as_ptr(), &L1)); print_bool(ptr::eq(L3[0].next[0].next.as_ptr(), &BASE));
    let mut s = 0;
    for t in TABLE.iter() { s += t[0].id; }
    print_u32(s);
    print_bool(ptr::eq(TABLE[1].as_ptr(), TABLE[3].as_ptr())); print_bool(ptr::eq(TABLE[1].as_ptr(), &M0));
    print_bool(ptr::eq(TABLE[4].as_ptr(), AAA_SL.as_ptr())); print_bool(ptr::eq(TABLE[5].as_ptr(), L1.next.as_ptr()));
}
