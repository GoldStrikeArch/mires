use alloc::string::String;
use core::hint::black_box;
use core::panic::Location;
#[track_caller]
fn leaf(x: u32) -> u32 {
    x + Location::caller().line() * 100 + Location::caller().column()
}
#[track_caller]
fn with_gen<F: Fn(u32) -> u32>(f: F) -> u32 {
    let s = String::from("ab");
    f(Location::caller().line()) + s.len() as u32
}
#[track_caller]
fn with_ptr(f: fn(u32) -> u32) -> u32 {
    let s = String::from("ab");
    f(Location::caller().line()) + s.len() as u32
}
#[track_caller]
fn with_once<F: FnOnce(u32) -> u32>(f: F) -> u32 {
    f(Location::caller().column())
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let k = black_box(3);
    print_u32(with_gen(|x| x + k));
    print_u32(with_gen(leaf));
    print_u32(with_ptr(leaf));
    print_u32(with_ptr(|x| x + 1));
    print_u32(with_once(leaf));
    print_u32(with_once(move |x| x + k));
    let p: fn(u32) -> u32 = leaf;
    print_u32(with_gen(p));
    print_u32(with_once(p));
}
