include!("fam_common.rs");
use core::mem::MaybeUninit;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // *a = a.clone(), x = [x[1], x[0]], swaps through a temporary
    let mut a = bb([1u32, 2, 3]);
    { let r = &mut a; *r = r.clone(); }
    out(1, hs(&a));
    let mut x = bb([10u32, 20]);
    x = [x[1], x[0]];
    out(2, hs(&x));
    let mut y = bb([5u32, 6]);
    let t = x; x = y; y = t;
    out(3, hs(&x) ^ hs(&y).rotate_left(3));
    // nested arrays assigned whole; rows stay independent
    let mut g = bb([[1u32, 2], [3, 4]]);
    let h = bb([[5u32, 6], [7, 8]]);
    g = h;
    g[0][0] = 99;
    out(4, hr(&g) ^ hr(&h).rotate_left(3));
    { let r = &mut g; *r = [r[1], r[0]]; }
    g[1][1] = 42;
    out(5, hr(&g));
    g[0] = g[1];
    g[1][0] = 43;
    out(6, hr(&g));
    // tuples and structs holding arrays
    let mut tu = bb(([1u8, 2], 3u32, [4u64, 5]));
    let keep = tu;
    tu = ([9, 9], 8, [7, 6]);
    tu.0 = keep.0;
    tu.2[1] = keep.2[0];
    out(7, hs(&tu.0) ^ tu.1 as u64 ^ hs(&tu.2));
    struct S { a: [u32; 2], b: [[u8; 2]; 2], c: Option<[u32; 2]> }
    let mut s = S { a: [1, 2], b: [[3, 4], [5, 6]], c: None };
    let s2 = S { a: [7, 8], b: [[9, 10], [11, 12]], c: Some([13, 14]) };
    { let r = &mut s; *r = s2; }
    s.b[1] = s.b[0];
    s.c = s.c.map(|[p, q]| [q, p]);
    out(8, hs(&s.a) ^ hr(&s.b) ^ hs(&s.c.unwrap()));
    // an array of references and of boxes, assigned over
    let (p, q, r) = (bb(1u32), bb(2u32), bb(3u32));
    let mut refs = bb([&p, &q]);
    refs = [refs[1], &r];
    out(9, (*refs[0] * 10 + *refs[1]) as u64);
    let mut bxs: [Box<u32>; 2] = [Box::new(1), Box::new(2)];
    bxs = [Box::new(*bxs[1] + 10), Box::new(*bxs[0] + 20)];
    out(10, (*bxs[0] * 100 + *bxs[1]) as u64);
    // through raw pointers
    let mut z = bb([1u32, 2, 3, 4]);
    unsafe {
        let pz: *mut [u32; 4] = &mut z;
        *pz = *pz;
        pz.write(pz.read());
        let w = pz as *mut [u32; 2];
        *w.add(1) = *w;
        w.write([8, 9]);
    }
    out(11, hs(&z));
    // element views: a row inside a longer flat buffer, assigned over itself and over its neighbour
    let mut f = bb([1u32, 2, 3, 4, 5, 6]);
    {
        let (ch, _) = f.as_chunks_mut::<2>();
        ch[0] = ch[1];
        ch[2] = ch[0];
        let c1 = ch[1];
        ch[1] = [c1[1], c1[0]];
    }
    out(12, hs(&f));
    { let a3: &mut [u32; 3] = (&mut f[1..4]).try_into().unwrap(); *a3 = [a3[2], a3[1], a3[0]]; let c = a3.clone(); *a3 = c; }
    out(13, hs(&f));
    { let whole: &mut [u32; 6] = (&mut f[..]).try_into().unwrap(); *whole = [6, 5, 4, 3, 2, 1]; }
    out(14, hs(&f));
    // uninitialised arrays assigned over initialised ones
    let mut mu = bb([MaybeUninit::new(1u32), MaybeUninit::new(2)]);
    mu = [MaybeUninit::uninit(); 2];
    mu[0].write(7);
    out(15, unsafe { mu[0].assume_init() } as u64);
    let mut e: [u32; 0] = [];
    e = [];
    out(16, e.len() as u64);
}
