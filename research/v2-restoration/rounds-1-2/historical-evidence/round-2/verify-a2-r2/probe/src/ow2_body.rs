include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // destructors: the old value dropped once, the new one owned once
    {
        let mut a: [D; 2] = [D::mk(1), D::mk(2)];
        a = [D::mk(3), D::mk(4)];
        out(1, drops() as u64); out(2, hs(&a));
        let b = a.clone();
        a = b;
        out(3, drops() as u64); out(4, live() as i64 as u64);
        let mut g: [[D; 2]; 2] = [[D::mk(5), D::mk(6)], [D::mk(7), D::mk(8)]];
        g[0] = a;
        out(5, drops() as u64);
        g[1] = g[0].clone();
        out(6, drops() as u64); out(7, hr(&g));
        { let r = &mut g; *r = [[D::mk(9), D::mk(10)], [D::mk(11), D::mk(12)]]; }
        out(8, drops() as u64); out(9, live() as i64 as u64);
        let mut v: Vec<[D; 2]> = vec![[D::mk(1), D::mk(2)], [D::mk(3), D::mk(4)]];
        v[0] = [D::mk(20), D::mk(21)];
        out(10, drops() as u64);
        let last = v.pop().unwrap();
        v[0] = last;
        out(11, drops() as u64); out(12, hr(&v));
        v.as_flattened_mut()[1] = D::mk(30);
        out(13, drops() as u64);
        let mut t: (D, [D; 1]) = (D::mk(40), [D::mk(41)]);
        t = (D::mk(42), [D::mk(43)]);
        t.1 = [D::mk(44)];
        out(14, drops() as u64); out(15, t.0.h() + hs(&t.1));
        let mut ob: Option<[D; 2]> = Some([D::mk(50), D::mk(51)]);
        ob = None;
        out(16, drops() as u64);
        ob = Some([D::mk(52), D::mk(53)]);
        if let Some(arr) = &mut ob { *arr = [D::mk(54), D::mk(55)]; core::mem::swap(&mut arr[0], &mut g[0][0]); }
        out(17, drops() as u64); out(18, live() as i64 as u64);
        let mut bx: Box<[D; 2]> = Box::new([D::mk(60), D::mk(61)]);
        *bx = [D::mk(62), D::mk(63)];
        let moved = *bx;
        out(19, drops() as u64); out(20, hs(&moved));
        let mut bg: Box<[[D; 1]; 2]> = Box::new([[D::mk(70)], [D::mk(71)]]);
        bg[1] = [D::mk(72)];
        *bg = [[D::mk(73)], [D::mk(74)]];
        out(21, drops() as u64); out(22, hr(&bg[..]));
    }
    out(23, drops() as u64); out(24, live() as i64 as u64);
}
