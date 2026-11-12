use alloc::boxed::Box;
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Box<u32> = Box::new(bb(7u32));
    print_u32(*v);
}
