#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// `#[no_mangle]` statics whose fixed JavaScript names are also spelled as object keys of the
// pointer and enum records (`buf`, `off`, `len`, `TAG`, `_0`) and as their own field names:
// self-reference detection must not confuse a key with a name, either way.
struct Me { id: u32, me: &'static Me }
#[unsafe(no_mangle)]
pub static me: Me = Me { id: 1, me: &me };            // names itself; `me:` is also a key
struct Id { id: u32 }
#[unsafe(no_mangle)]
pub static id: Id = Id { id: 2 };                     // does not name itself; `id:` is a key
static VAL: u32 = 40;
#[unsafe(no_mangle)]
pub static off: &u32 = &VAL;                          // boxed slot record has an `off:` key
#[unsafe(no_mangle)]
pub static buf: &u32 = &VAL;                          // and a `buf:` key
#[unsafe(no_mangle)]
pub static len: &[u32] = &[1, 2, 3];                  // fat slice record has a `len:` key
struct Bx { id: u32, b: &'static &'static Bx }
#[unsafe(no_mangle)]
pub static next: &Bx = &BX;                           // boxed, in a cycle with BX
static BX: Bx = Bx { id: 3, b: &next };
struct Pair { id: u32, a: &'static Pair, b: &'static Pair }
#[unsafe(no_mangle)]
pub static a: Pair = Pair { id: 4, a: &b, b: &a };
#[unsafe(no_mangle)]
pub static b: Pair = Pair { id: 5, a: &a, b: &b };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(me.me.me.id);
    print_bool(core::ptr::eq(me.me, &me));
    print_u32(id.id);
    print_u32(**(&off) + *buf + len.len() as u32 * 100 + len[2] * 1000);
    print_u32(next.id + next.b.b.id * 10 + BX.b.id * 100);
    print_bool(core::ptr::eq(BX.b, &next));
    print_u32(a.a.id * 1000 + a.b.id * 100 + b.a.b.id * 10 + b.b.a.id);
    print_bool(core::ptr::eq(a.a.a, &a) && core::ptr::eq(b.b, &b));
}
