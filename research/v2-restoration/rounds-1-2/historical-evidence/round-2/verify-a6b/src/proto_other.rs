#![no_std]
#![no_main]
#![allow(dead_code, unused, non_snake_case, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// Things that spell `__proto__` in a cycle of statics without being a field of that name: a
// string value, a static NAMED `__proto__`, a field whose name merely contains it. None may be
// refused, and all must read right.
struct N { id: u32, name: &'static str, next: &'static N, __proto: u32, proto__: u32, x__proto__: u32 }
#[unsafe(no_mangle)]
pub static __proto__: N = N { id: 1, name: "__proto__", next: &OTHER, __proto: 3, proto__: 4, x__proto__: 5 };
static OTHER: N = N { id: 2, name: "{__proto__:1}", next: &__proto__, __proto: 6, proto__: 7, x__proto__: 8 };
struct Arr { all: [&'static N; 2], me: &'static Arr }
static ARR: Arr = Arr { all: [&__proto__, &OTHER], me: &ARR };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(__proto__.next.id * 10 + OTHER.next.id);
    print_str(__proto__.name);
    print_str(OTHER.next.next.name);
    print_u32(__proto__.__proto + OTHER.proto__ * 10 + OTHER.next.x__proto__ * 100);
    print_bool(core::ptr::eq(ARR.me.all[0], &__proto__));
    print_u32(ARR.all[1].next.id);
}
