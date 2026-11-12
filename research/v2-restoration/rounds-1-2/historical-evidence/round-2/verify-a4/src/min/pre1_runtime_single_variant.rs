#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
#[inline(never)]
fn code(r: Result<(Infallible, ()), char>) -> u32 { match r { Ok(_) => 99, Err(c) => c as u32 } }
#[derive(Clone, Copy)]
struct Wrap<T>(T);
#[derive(Clone, Copy)]
enum Second { Dead(Wrap<Infallible>), Live(u32) }
#[inline(never)]
fn code2(s: Second) -> u32 { match s { Second::Dead(_) => 99, Second::Live(n) => n } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(code(black_box(Err('z'))));
    print_u32(code2(Second::Live(black_box(5))));
}
