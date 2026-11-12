#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
#[derive(Clone, Copy)]
enum Parsed { Never((), Infallible), Char(char) }
const HIT: Option<Parsed> = Some(Parsed::Char('w'));
#[inline(never)]
fn code(p: Option<Parsed>) -> u32 { match p { Some(Parsed::Char(c)) => c as u32, Some(Parsed::Never(..)) => 99, None => 0 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(code(HIT));
    print_bool(HIT.is_some());
    print_u32(match HIT { Some(Parsed::Char(c)) => c as u32, _ => 0 });
    print_u32(code(black_box(HIT)));
}
