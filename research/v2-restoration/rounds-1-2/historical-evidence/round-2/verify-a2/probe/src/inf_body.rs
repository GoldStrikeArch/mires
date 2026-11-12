use alloc::vec::Vec;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a: Vec<[u8; 4]> = Vec::new();
    a.push([1, 2, 3, 4]);
    a.push([5, 6, 7, 8]);
    let mut f: Vec<u8> = a.into_flattened();
    print_u32(f[5] as u32 + f.len() as u32 * 100);
    f.push(9);
    print_u32(f.iter().map(|x| *x as u32).sum());
    let mut b: Vec<[u32; 2]> = Vec::with_capacity(3);
    b.push([1, 2]);
    b.push([3, 4]);
    let mut g: Vec<u32> = b.into_flattened();
    g.push(5);
    print_u32(g.iter().fold(0, |x, y| x * 10 + y));
    let mut c: Vec<[[u8; 2]; 2]> = Vec::new();
    c.push([[1, 2], [3, 4]]);
    c.push([[5, 6], [7, 8]]);
    let h: Vec<[u8; 2]> = c.into_flattened();
    print_u32(h[2][1] as u32 + h.len() as u32);
}
