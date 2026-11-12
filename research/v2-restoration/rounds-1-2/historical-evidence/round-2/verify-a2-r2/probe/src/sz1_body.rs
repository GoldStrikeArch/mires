use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<u32> = bb(vec![7u32]);
    print_u32(v[0]);
}
