include!("fam_common.rs");
#[inline(never)]
fn sweep<T: El + Copy + PartialOrd, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    let len = rows.len() * N;
    rows.as_flattened_mut().rotate_left(1);
    out(t + 1, hr(rows));
    rows.as_flattened_mut().rotate_right(3);
    out(t + 2, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); core::mem::swap(&mut a[0], &mut b[0]); }
    out(t + 3, hr(rows));
    { let f = rows.as_flattened_mut(); f.select_nth_unstable_by(len / 2, |a, b| a.partial_cmp(b).unwrap()); }
    out(t + 4, hr(rows));
    { let f = rows.as_flattened_mut(); let mut i = 0u32; f.fill_with(|| { i += 1; T::mk(i) }); }
    out(t + 5, hr(rows));
    { let f = rows.as_flattened_mut(); unsafe { core::ptr::swap(f.as_mut_ptr(), f.as_mut_ptr().add(len - 1)); let v = core::ptr::replace(f.as_mut_ptr().add(1), T::mk(70)); f.as_mut_ptr().add(2).write(v); } }
    out(t + 6, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); let n = a.len().min(b.len()); unsafe { core::ptr::swap_nonoverlapping(a.as_mut_ptr(), b.as_mut_ptr(), n.min(2)); } }
    out(t + 7, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); let n = a.len().min(b.len()); a[..n].swap_with_slice(&mut b[..n]); }
    out(t + 8, hr(rows));
    out(t + 9, hs(rows.as_flattened()));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[f64; 3]> = (0..6u32).map(|i| mkrow(i * 2)).collect();
    sweep(0, &mut v);
    out(50, hr(&v));
    v.push(mkrow(1));
    out(51, hr(&v) ^ hs(v.as_flattened()));
    let mut b: Box<[[f64; 3]; 4]> = Box::new(mkgrid(3));
    sweep(100, &mut b[..]);
    out(150, hr(&b[..]));
    let mut g: [[f64; 3]; 4] = bb(mkgrid(3));
    sweep(200, &mut g);
    out(250, hr(&g));
}
