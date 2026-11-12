use core::hint::black_box as bb;
#[inline(never)] fn set(a: &mut [u8; 4]) { *a = [9, 8, 7, 6]; }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut buf = bb([0u8; 8]);
    let a: &mut [u8; 4] = (&mut buf[2..6]).try_into().unwrap();
    set(a);
    print_u32(buf.iter().fold(0u32, |x, y| x * 10 + *y as u32));
}
