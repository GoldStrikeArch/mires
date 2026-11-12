use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a = bb([1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    let mut b = bb([11u8, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    a[1..10].swap_with_slice(&mut b[0..9]);
    print_u32(a.iter().map(|x| *x as u32).sum::<u32>() * 1000 + b.iter().map(|x| *x as u32).sum::<u32>());
    print_u32(a[1] as u32 * 100 + a[9] as u32);
}
