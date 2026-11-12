#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
trait HasOut { type Out; }
impl<T> HasOut for fn() -> T { type Out = T; }
type Never = <fn() -> ! as HasOut>::Out;
const HIT: Option<Result<Never, char>> = Some(Err('x'));
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(HIT.is_some());
    print_u32(match HIT { Some(Err(c)) => c as u32, _ => 0 });
    print_bool(black_box(HIT).is_some());
}
