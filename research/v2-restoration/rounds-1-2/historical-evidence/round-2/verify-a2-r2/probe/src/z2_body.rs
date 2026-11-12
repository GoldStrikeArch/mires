// concat of zero rows (no flattening), to compare with main.
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let none: [[u32; 2]; 0] = bb([]);
    print_usize(none.concat().len());
    print_usize(none.iter().flatten().count());
}
