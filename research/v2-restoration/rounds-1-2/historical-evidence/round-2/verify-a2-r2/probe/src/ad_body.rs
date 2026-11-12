// Observable address relations between rows and their flattened elements (done-when 4).
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
fn b(x: bool) { print_bool(x) }
#[inline(never)]
fn rel(rows: &mut [[u32; 3]]) {
    let n = rows.len();
    let base = rows.as_ptr() as usize;
    let fbase = rows.as_flattened().as_ptr() as usize;
    b(base == fbase);
    b(rows.as_flattened().as_ptr_range().end as usize - fbase == n * 12);
    // element addresses through the flattened view are contiguous
    let f = rows.as_flattened();
    let mut ok = true;
    for i in 0..f.len() { ok &= (&f[i] as *const u32 as usize) == fbase + i * 4; }
    b(ok);
    // row pointers taken as places agree with the flattened elements
    let mut ok = true;
    for r in 0..n { ok &= unsafe { rows.as_ptr().add(r) } as usize == fbase + r * 12; }
    b(ok);
    let mut ok = true;
    for (r, row) in rows.iter().enumerate() { ok &= row.as_ptr() as usize == fbase + r * 12; }
    b(ok);
    // ptr::eq / ordering between a flattened element pointer and a row element pointer
    let p = unsafe { rows.as_ptr().add(n - 1) } as *const u32;
    let q = &rows.as_flattened()[(n - 1) * 3] as *const u32;
    b(core::ptr::eq(p, q));
    b(p <= q && q <= p);
    let first = rows.as_flattened().as_ptr();
    b(first < q || n == 1);
    b(unsafe { q.offset_from(first) } == ((n - 1) * 3) as isize);
    // subslice of the flattened view vs the rows
    let sub = &rows.as_flattened()[3..];
    b(n < 2 || sub.as_ptr() as usize == base + 12);
    // as_mut_ptr and a write through it lands in the row
    let m = rows.as_flattened_mut().as_mut_ptr();
    unsafe { *m.add(n * 3 - 1) = 77; *m = 78; }
    b(rows[n - 1][2] == 77 && rows[0][0] == 78);
    // as_ptr_range of rows and of flattened agree on the end
    b(rows.as_ptr_range().end as usize == rows.as_flattened().as_ptr_range().end as usize);
    // pointer to the row as a whole, cast to its first element, compared with the row's own as_ptr
    let rp: *const [u32; 3] = &raw const rows[n - 1];
    b(rp as *const u32 as usize == rows[n - 1].as_ptr() as usize);
    b(rp as usize == base + (n - 1) * 12);
    // alignment questions a program can ask
    b(base % 4 == 0 && fbase % 4 == 0);
    b((rows.as_flattened().as_ptr() as usize + 4) % 4 == 0);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([[1u32, 2, 3], [4, 5, 6], [7, 8, 9]]);
    rel(&mut g);
    print_u32(1);
    rel(&mut g[1..]);
    print_u32(2);
    let mut bx = Box::new(bb([[1u32, 2, 3], [4, 5, 6]]));
    rel(&mut bx[..]);
    print_u32(3);
    let mut v = bb(vec![[1u32, 2, 3], [4, 5, 6], [7, 8, 9], [1, 1, 1]]);
    rel(&mut v);
    print_u32(4);
    let mut p: Vec<[u32; 3]> = Vec::new();
    for i in 0..5 { p.push([i, i + 1, i + 2]); }
    rel(&mut p);
    let before = p.as_ptr() as usize;
    p.reserve(100);
    rel(&mut p);
    print_u32(5);
    let mut one = bb(vec![[1u32, 2, 3]]);
    rel(&mut one);
    print_u32(6);
    let mut b1: Box<[u32; 3]> = Box::new(bb([1, 2, 3]));
    let a0 = &*b1 as *const [u32; 3] as usize;
    let a1 = b1.as_ptr() as usize;
    let a2 = &b1[2] as *const u32 as usize;
    b(a0 == a1);
    b(a2 == a1 + 8);
    rel(core::slice::from_mut(&mut *b1));
    print_u32(7);
    // two different blocks never overlap
    let x = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let y = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let (xa, ya) = (x.as_flattened().as_ptr() as usize, y.as_flattened().as_ptr() as usize);
    b(xa + 24 <= ya || ya + 24 <= xa);
    b(!core::ptr::eq(x.as_flattened().as_ptr(), y.as_flattened().as_ptr()));
    // into_flattened keeps the address (vec!-built)
    let z = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let za = z.as_ptr() as usize;
    let zf = z.into_flattened();
    b(zf.as_ptr() as usize == za);
    b(&zf[4] as *const u32 as usize == za + 16);
}
