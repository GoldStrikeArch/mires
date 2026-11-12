use alloc::boxed::Box;
use alloc::string::String;
use core::hint::black_box;
use core::panic::Location;
#[track_caller]
fn with(f: &dyn Fn(u32) -> u32) -> u32 {
    let s = String::from("abcd");
    f(Location::caller().line()) + s.len() as u32
}
#[track_caller]
fn with_box(f: Box<dyn Fn(u32) -> u32>) -> u32 {
    f(Location::caller().line())
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let k = black_box(3);
    print_u32(with(&|x| x + k));
    print_u32(with_box(Box::new(move |x| x * k)));
}
