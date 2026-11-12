use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a = bb([1u32, 2, 3, 4, 5]);
    a.rotate_left(2);
    print_u32(a.iter().fold(0, |x, y| x * 10 + y));
    let mut g = bb([[1u32, 2], [3, 4], [5, 6]]);
    g.rotate_left(1);
    g.as_flattened_mut().rotate_right(1);
    print_u32(g.as_flattened().iter().fold(0, |x, y| x * 10 + y));
}
