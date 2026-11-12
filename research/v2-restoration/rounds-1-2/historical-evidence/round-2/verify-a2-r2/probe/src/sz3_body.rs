use alloc::vec::Vec;
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<u32> = Vec::new();
    v.push(bb(7u32));
    v.push(8);
    print_u32(v[1]);
}
