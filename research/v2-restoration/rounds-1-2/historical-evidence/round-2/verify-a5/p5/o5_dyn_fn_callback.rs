use alloc::boxed::Box;
use alloc::string::String;
use core::hint::black_box;
use core::panic::Location;
#[track_caller]
fn with(f: &dyn Fn(u32) -> u32) -> u32 {
    f(Location::caller().line())
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let k = black_box(3);
    print_u32(with(&|x| x + k));
    print_u32(with(&move |x| x * k));
}
