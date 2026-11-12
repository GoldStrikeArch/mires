// Two family failures isolated: concat of rows of tuples, and clone of a boxed grid of references.
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box as bb;
static R: [u32; 6] = [10, 20, 30, 40, 50, 60];
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([[(1u8, 100u32), (2, 200)], [(3, 300), (4, 400)]]);
    let c: Vec<(u8, u32)> = g.concat();
    print_u32(c.iter().map(|t| t.0 as u32 + t.1).sum());
    let f = g.as_flattened();
    print_u32(f.iter().map(|t| t.0 as u32 + t.1).sum());
    let b: Box<[[&'static u32; 3]; 2]> = Box::new(bb([[&R[0], &R[1], &R[2]], [&R[3], &R[4], &R[5]]]));
    let d = b.clone();
    print_u32(d.as_flattened().iter().map(|r| **r).sum());
    print_u32(*d[1][2] + *b[0][0]);
    let b2: Box<[[u32; 3]; 2]> = Box::new(bb([[1, 2, 3], [4, 5, 6]]));
    let d2 = b2.clone();
    print_u32(d2.as_flattened().iter().sum());
    let b3: Box<[&'static u32; 3]> = Box::new(bb([&R[0], &R[1], &R[2]]));
    let d3 = b3.clone();
    print_u32(d3.iter().map(|r| **r).sum());
}
