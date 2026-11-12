use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a = bb([1u32, 2, 3, 4]);
    let mut b = bb([5u32, 6, 7, 8]);
    a[1..3].swap_with_slice(&mut b[2..4]);
    print_u32(a[1] * 1000 + a[2] * 100 + b[2] * 10 + b[3]);
}
