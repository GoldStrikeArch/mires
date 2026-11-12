// F7/F8/F9: mem ops on fields of fresh copies, array/tuple element writes, slices of copies.
#[inline(never)] fn m_replace_field(s: &[P]) -> u32 { let mut c = s[bb(1)]; let old = mem::replace(&mut c.x, 99); old * 100000 + c.x * 1000 + s[1].x }
#[inline(never)] fn m_take_field(s: &[P]) -> u32 { let mut c = s[bb(1)]; let old = mem::take(&mut c.x); old * 1000 + c.x * 100 + s[1].x }
#[inline(never)] fn m_swap_fields(s: &[P]) -> u32 { let mut c = s[bb(1)]; mem::swap(&mut c.x, &mut c.y); c.x * 1000 + c.y * 100 + s[1].x * 10 + s[1].y }
#[inline(never)] fn m_swap_copies(s: &[P]) -> u32 { let mut a = s[bb(0)]; let mut b = s[bb(1)]; mem::swap(&mut a, &mut b); a.x = 99; (a.x + b.x + a.y) * 1000 + sum(s) }
#[inline(never)] fn m_swap_copy_field_other(s: &[P]) -> u32 { let mut a = s[bb(0)]; let mut k = bb(77); mem::swap(&mut a.x, &mut k); (a.x + k) * 1000 + s[0].x }
#[inline(never)] fn m_replace_whole(s: &[P]) -> u32 { let mut c = s[bb(1)]; let old = mem::replace(&mut c, P { x: 99, y: 98 }); (old.x + c.x) * 1000 + s[1].x }
#[inline(never)] fn m_replace_whole_write(s: &[P]) -> u32 { let mut c = s[bb(1)]; let mut old = mem::replace(&mut c, s[bb(2)]); old.x = 97; c.y = 96; (old.x + c.y) * 1000 + sum(s) }
#[inline(never)] fn m_replace_inner(ns: &[N]) -> u32 { let mut c = ns[bb(1)]; let old = mem::replace(&mut c.inner, P { x: 99, y: 98 }); c.inner.x = 97; (old.x + c.inner.x) * 1000 + ns[1].inner.x }
#[inline(never)] fn m_replace_inner_from(ns: &[N], s: &[P]) -> u32 { let mut c = ns[bb(1)]; let mut old = mem::replace(&mut c.inner, s[bb(2)]); c.inner.x = 97; old.y = 96; (old.y + c.inner.x) * 1000 + ns[1].inner.x + sum(s) }
#[inline(never)] fn m_take_option(o: &[Option<P>]) -> u32 { let mut c = o[bb(1)]; let t = c.take(); let mut p = t.unwrap(); p.x = 99; (p.x + c.is_none() as u32) * 1000 + o[1].unwrap().x }
#[inline(never)] fn m_replace_option(o: &[Option<P>]) -> u32 { let mut c = o[bb(1)]; let old = c.replace(P { x: 99, y: 98 }); (old.unwrap().x + c.unwrap().x) * 1000 + o[1].unwrap().x }
#[inline(never)] fn m_replace_fresh_agg(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let old = mem::replace(&mut p.x, 99); old * 100000 + n * 1000 + p.x + m }
#[inline(never)] fn m_take_fresh_agg(n: u32, m: u32) -> u32 { let mut t = (n, m); let old = mem::take(&mut t.1); old * 10000 + m * 100 + t.1 + n }
#[inline(never)] fn m_swap_fresh(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; mem::swap(&mut p.x, &mut p.y); p.x * 100000 + p.y * 10000 + n * 100 + m }
#[inline(never)] fn m_swap_fresh_two(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let mut q = P { x: m, y: n }; mem::swap(&mut p.x, &mut q.x); p.x * 100000 + q.x * 10000 + n * 100 + m }
#[inline(never)] fn m_swap_fresh_arr(n: u32, m: u32) -> u32 { let mut a = [n, m]; let (l, r) = a.split_at_mut(1); mem::swap(&mut l[0], &mut r[0]); a[0] * 100000 + a[1] * 10000 + n * 100 + m }
#[inline(never)] fn m_ptr_write_field(s: &[P]) -> u32 { let mut c = s[bb(1)]; unsafe { core::ptr::write(&mut c.x, 99) }; c.x * 1000 + s[1].x }
#[inline(never)] fn m_ptr_write_fresh(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; unsafe { core::ptr::write(&mut c.x, 99) }; c.x * 10000 + n * 100 + m }
#[inline(never)] fn m_ptr_swap(s: &[P]) -> u32 { let mut a = s[bb(0)]; let mut b = s[bb(1)]; unsafe { core::ptr::swap(&mut a, &mut b) }; a.y = 99; (a.x + a.y + b.x) * 1000 + sum(s) }
#[inline(never)] fn m_ptr_copy(s: &[P]) -> u32 { let mut a = s[bb(0)]; unsafe { core::ptr::copy_nonoverlapping(&s[bb(2)], &mut a, 1) }; a.y = 99; (a.x + a.y) * 1000 + sum(s) }
#[inline(never)] fn m_ptr_read(s: &[P]) -> u32 { let mut a = unsafe { core::ptr::read(&s[bb(1)]) }; a.y = 99; (a.x + a.y) * 1000 + sum(s) }
#[inline(never)] fn m_ptr_read_fresh(p: P) -> u32 { let mut a = unsafe { core::ptr::read(&p) }; a.y = 99; (a.x + a.y) * 1000 + p.y }
#[inline(never)] fn m_addr_of_mut(s: &[P]) -> u32 { let mut c = s[bb(1)]; let p = core::ptr::addr_of_mut!(c.y); unsafe { *p = 99 }; c.y * 1000 + s[1].y }
#[inline(never)] fn m_addr_of_mut_fresh(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; let p = core::ptr::addr_of_mut!(c.y); unsafe { *p = 99 }; c.y * 10000 + n * 100 + m }
#[inline(never)] fn m_raw_const_fresh(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let q = &raw const p.x as *mut u32; unsafe { *q = 99 }; p.x * 10000 + n * 100 + m }
#[inline(never)] fn m_raw_whole_fresh(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let q = &raw mut p; unsafe { (*q).x = 99; (*q).y += 1 }; p.x * 100000 + p.y * 10000 + n * 100 + m }
#[inline(never)] fn m_raw_whole_copy(s: &[P]) -> u32 { let mut p = s[bb(1)]; let q = &raw mut p; unsafe { (*q).x = 99; (*q).y += 1 }; (p.x + p.y) * 1000 + sum(s) }
#[inline(never)] fn m_clone_from(s: &[P]) -> u32 { let mut c = s[bb(0)]; c.clone_from(&s[bb(2)]); c.x = 99; (c.x + c.y) * 1000 + sum(s) }
#[inline(never)] fn m_clone(s: &[P]) -> u32 { let mut c = s[bb(0)].clone(); c.x = 99; (c.x + c.y) * 1000 + sum(s) }
#[inline(never)] fn m_clone_arr(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(0)].clone(); c[1] = 99; suma(&c) * 1000 + suma(&a[0]) }
#[inline(never)] fn m_default() -> u32 { let mut a: [u32; 3] = Default::default(); a[1] = 99; let b: [u32; 3] = Default::default(); suma(&a) * 1000 + suma(&b) }

#[inline(never)] fn e_arr_static() -> u32 { let mut a = AA[bb(1)]; a[bb(2)] = 99; a[2] * 1000 + AA[1][2] }
#[inline(never)] fn e_arr_const_idx() -> u32 { let mut a = AA[1]; a[0] = 99; a[0] * 1000 + AA[1][0] }
#[inline(never)] fn e_arr_whole_static() -> u32 { let mut a = AA; a[bb(1)][bb(1)] = 99; a[0][0] = 98; (a[1][1] + a[0][0]) * 1000 + AA[1][1] + AA[0][0] }
#[inline(never)] fn e_arr_const() -> u32 { let mut a = KA; a[bb(1)] = 99; let b = KA; (a[1] + b[1]) * 1000 + KA[1] }
#[inline(never)] fn e_const_p() -> u32 { let mut a = KP; a.x = 99; let b = KP; (a.x + b.x) * 1000 + KP.x }
#[inline(never)] fn e_const_n() -> u32 { let mut a = KN; a.inner.x = 99; a.a = 98; let b = KN; (a.inner.x + b.inner.x + a.a) * 1000 + KN.inner.x + KN.a }
#[inline(never)] fn e_const_inline() -> u32 { let mut a = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; a[bb(1)].x = 99; a[0].y = 98; sum(&a) }
#[inline(never)] fn e_const_inline_arr() -> u32 { let mut a = [[1u32, 2], [3, 4]]; a[bb(1)][bb(0)] = 99; a[0][1] += 5; suma(&a[0]) * 1000 + suma(&a[1]) }
#[inline(never)] fn e_const_tuple() -> u32 { let mut t = (1u32, [2u32, 3], P { x: 4, y: 5 }); t.0 += 1; t.1[bb(1)] += 1; t.2.x += 1; t.0 * 10000 + t.1[1] * 100 + t.2.x }
#[inline(never)] fn e_const_promoted() -> u32 { let r: &'static [P; 2] = &[P { x: 1, y: 2 }, P { x: 3, y: 4 }]; let mut a = *r; a[bb(1)].x = 99; let mut b = r[bb(0)]; b.y = 98; (sum(&a) + b.y) * 1000 + sum(r) }
#[inline(never)] fn e_const_enum() -> u32 { let mut e = E::C(P { x: 1, y: 2 }); if let E::C(p) = &mut e { p.x += 5; } obs(&e) }
#[inline(never)] fn e_const_option() -> u32 { let mut o = Some(P { x: 1, y: 2 }); if let Some(p) = &mut o { p.x += 5; } o.unwrap().x }
#[inline(never)] fn e_arr_param_copy(a: [u32; 3]) -> u32 { let mut b = a; b[1] = 99; b[1] * 1000 + a[1] }
#[inline(never)] fn e_arr2d(g: &[[[u32; 2]; 2]]) -> u32 { let mut c = g[bb(0)]; c[1][0] = 99; c[1][0] * 1000 + g[0][1][0] }
#[inline(never)] fn e_arr2d_row(g: &[[[u32; 2]; 2]]) -> u32 { let mut row = g[bb(0)][bb(1)]; row[1] = 99; row[1] * 1000 + g[0][1][1] }
#[inline(never)] fn e_tuple_static() -> u32 { let mut t = TS[bb(1)]; t.0.x = 99; t.1[1] = 98; t.2 = 97; (t.0.x + t.1[1] + t.2) * 1000 + TS[1].0.x + TS[1].1[1] + TS[1].2 }
#[inline(never)] fn e_tuple_local_copy() -> u32 { let t = bb((P { x: 1, y: 2 }, 3u32)); let mut u = t; u.0.x = 99; u.1 = 98; (u.0.x + u.1) * 1000 + t.0.x + t.1 }
#[inline(never)] fn e_arr_loop(a: &[[u32; 3]]) -> u32 { let mut t = 0; for i in 0..a.len() { let mut r = a[i]; for j in 0..3 { r[j] += 1; t += r[j]; } } t * 1000 + a[0][0] + a[1][2] }
#[inline(never)] fn e_arr_whole_assign_then_elem(a: &[[u32; 3]]) -> u32 { let mut r = a[bb(0)]; r[2] = 5; r = a[bb(1)]; r[0] = 99; r[0] * 1000 + a[1][0] + a[0][0] + a[0][2] }
#[inline(never)] fn e_cond_copy(s: &[P], c: bool) -> u32 { let mut p = if c { s[0] } else { s[2] }; p.x = 99; p.x * 1000 + sum(s) }
#[inline(never)] fn e_cond_w(ws: &[W], c: bool) -> u32 { let mut w = if c { ws[0] } else { ws[2] }; w.0 = 99; w.0 * 1000 + ws[0].0 + ws[2].0 }
#[inline(never)] fn e_cond_w_param(a: W, b: W, c: bool) -> u32 { let mut w = if c { a } else { b }; w.0 = 99; w.0 * 10000 + a.0 * 100 + b.0 }
#[inline(never)] fn e_cond_write_w(a: W, c: bool) -> u32 { let mut w = a; if c { w.0 = 99; } w.0 * 1000 + a.0 }
#[inline(never)] fn e_cond_write_p(s: &[P], c: bool) -> u32 { let mut p = s[bb(1)]; if c { p.x = 99; } p.x * 1000 + s[1].x }
#[inline(never)] fn e_loop_w(a: W, n: u32) -> u32 { let mut w = a; let mut t = 0; for i in 0..n { w.0 += i; t += w.0; w = a; } t * 1000 + a.0 }
#[inline(never)] fn e_loop_w2(a: W, n: u32) -> u32 { let mut w = a; let mut t = 0; for i in 0..n { let old = w; w.0 += i + 1; t += w.0 * 10 + old.0; } t * 1000 + a.0 }
#[inline(never)] fn e_loop_p(s: &[P], n: u32) -> u32 { let mut p = s[bb(1)]; let mut t = 0; for i in 0..n { let old = p; p.x += i + 1; t += p.x * 10 + old.x; } t * 1000 + s[1].x }
#[inline(never)] fn e_w_swap(a: W, b: W) -> u32 { let mut x = a; let mut y = b; mem::swap(&mut x, &mut y); x.0 = 99; x.0 * 1000000 + y.0 * 10000 + a.0 * 100 + b.0 }
#[inline(never)] fn e_w_self_assign(a: W) -> u32 { let mut b = a; b.0 = b.0 + a.0; let c = b; b.0 = 1; c.0 * 1000 + a.0 * 10 + b.0 }
#[inline(never)] fn e_w_two_copies(a: W) -> u32 { let mut b = a; let mut c = a; b.0 = 98; c.0 = 99; b.0 * 10000 + c.0 * 100 + a.0 }
#[inline(never)] fn e_w_copy_back(a: W) -> u32 { let mut b = a; b.0 = 98; let mut c = b; c.0 = 97; b = c; b.0 += 1; let d = b; b.0 = 0; a.0 * 1000000 + c.0 * 10000 + d.0 * 100 + b.0 }

#[inline(never)] fn s_subslice(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let sl = &mut c[1..]; sl[0] = 99; c[1] * 1000 + a[1][1] }
#[inline(never)] fn s_fill(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c[1..].fill(9); suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_reverse(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.reverse(); c[0] * 1000 + a[1][0] }
#[inline(never)] fn s_swap(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.swap(0, 2); c[0] * 1000 + a[1][0] }
#[inline(never)] fn s_copy_within(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.copy_within(0..2, 1); suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_iter_mut(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.iter_mut().for_each(|v| *v += 1); suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_split_at_mut(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let (l, r) = c.split_at_mut(1); l[0] = 99; r[1] = 98; suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_chunks_mut(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; for ch in c.chunks_mut(2) { ch[0] += 10; } suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_last_mut(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; *c.last_mut().unwrap() = 99; *c.first_mut().unwrap() = 98; if let Some(v) = c.get_mut(1) { *v = 97; } suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_as_mut_ptr(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; unsafe { *c.as_mut_ptr().add(1) = 99 }; c[1] * 1000 + a[1][1] }
#[inline(never)] fn s_sort(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.sort_unstable_by(|x, y| y.cmp(x)); suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn s_ps_slice(s: &[P]) -> u32 { let mut c = [s[0], s[1], s[2]]; for p in c[1..].iter_mut() { p.x += 100; } sum(&c) * 1000 + sum(s) }
#[inline(never)] fn s_ps_whole(s: &[P; 3]) -> u32 { let mut c = *s; for p in c.iter_mut() { p.x += 100; } c[bb(0)].y = 0; sum(&c) * 1000 + sum(s) }
#[inline(never)] fn s_ps_swap(s: &[P; 3]) -> u32 { let mut c = *s; c.swap(0, 2); c[0].x = 99; sum(&c) * 1000 + sum(s) }
#[inline(never)] fn s_copy_from_slice(s: &[P; 3]) -> u32 { let mut c = [P { x: 0, y: 0 }; 3]; c.copy_from_slice(s); c[1].x = 99; sum(&c) * 1000 + sum(s) }
#[inline(never)] fn s_clone_from_slice(s: &[P; 3]) -> u32 { let mut c = [P { x: 0, y: 0 }; 3]; c.clone_from_slice(s); c[1].x = 99; sum(&c) * 1000 + sum(s) }
#[inline(never)] fn s_fresh_arr_slice(n: u32, m: u32) -> u32 { let mut a = [n, m, n]; a[1..].fill(99); a[2] * 10000 + n * 100 + m }
#[inline(never)] fn s_fresh_arr_swap(n: u32, m: u32) -> u32 { let mut a = [n, m]; a.swap(0, 1); a[0] * 10000 + n * 100 + m }
#[inline(never)] fn s_fresh_arr_elem(n: u32, m: u32, i: usize) -> u32 { let mut a = [n, m]; a[i] = 99; (a[0] + a[1]) * 10000 + n * 100 + m }
#[inline(never)] fn s_fresh_arr_iter(n: u32, m: u32) -> u32 { let mut a = [n, m]; for v in a.iter_mut() { *v += 1; } (a[0] + a[1]) * 10000 + n * 100 + m }
#[inline(never)] fn s_fresh_arr_ps(p: P, q: P) -> u32 { let mut a = [p, q]; for v in a.iter_mut() { v.x += 1; } a.swap(0, 1); a[0].y = 0; sum(&a) * 1000 + p.x + q.x + q.y }
#[inline(never)] fn s_fresh_tuple(n: u32, m: u32) -> u32 { let mut t = (n, m); let r = &mut t.0; *r = 99; t.0 * 10000 + n * 100 + m }
#[inline(never)] fn s_to_slice_fn(a: &[[u32; 3]]) -> u32 {
    #[inline(never)] fn wr(s: &mut [u32]) { s[0] = 99; }
    #[inline(always)] fn wr_a(s: &mut [u32]) { s[1] = 98; }
    let mut c = a[bb(1)]; wr(&mut c); wr_a(&mut c); wr_a(&mut c[1..]); suma(&c) * 1000 + suma(&a[1])
}
#[inline(never)] fn s_to_arr_fn(a: &[[u32; 3]]) -> u32 {
    #[inline(never)] fn wr(s: &mut [u32; 3]) { s[0] = 99; }
    #[inline(always)] fn wr_a(s: &mut [u32; 3]) { s[1] = 98; }
    let mut c = a[bb(1)]; wr(&mut c); wr_a(&mut c); suma(&c) * 1000 + suma(&a[1])
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let ns = bb([N { a: 1, inner: P { x: 2, y: 3 } }, N { a: 4, inner: P { x: 5, y: 6 } }]);
        let op = bb([None, Some(P { x: 31, y: 32 })]);
        let g = bb([[[1u32, 2], [3, 4]], [[5, 6], [7, 8]]]);
        t!("m_replace_field", m_replace_field(s));
        t!("m_take_field", m_take_field(s));
        t!("m_swap_fields", m_swap_fields(s));
        t!("m_swap_copies", m_swap_copies(s));
        t!("m_swap_copy_field_other", m_swap_copy_field_other(s));
        t!("m_replace_whole", m_replace_whole(s));
        t!("m_replace_whole_write", m_replace_whole_write(s));
        t!("m_replace_inner", m_replace_inner(&ns));
        t!("m_replace_inner_from", m_replace_inner_from(&ns, s));
        t!("m_take_option", m_take_option(&op));
        t!("m_replace_option", m_replace_option(&op));
        t!("m_replace_fresh_agg", m_replace_fresh_agg(bb(3), bb(4)));
        t!("m_take_fresh_agg", m_take_fresh_agg(bb(3), bb(4)));
        t!("m_swap_fresh", m_swap_fresh(bb(3), bb(4)));
        t!("m_swap_fresh_two", m_swap_fresh_two(bb(3), bb(4)));
        t!("m_swap_fresh_arr", m_swap_fresh_arr(bb(3), bb(4)));
        t!("m_ptr_write_field", m_ptr_write_field(s));
        t!("m_ptr_write_fresh", m_ptr_write_fresh(bb(3), bb(4)));
        t!("m_ptr_swap", m_ptr_swap(s));
        t!("m_ptr_copy", m_ptr_copy(s));
        t!("m_ptr_read", m_ptr_read(s));
        t!("m_ptr_read_fresh", m_ptr_read_fresh(bb(P { x: 7, y: 8 })));
        t!("m_addr_of_mut", m_addr_of_mut(s));
        t!("m_addr_of_mut_fresh", m_addr_of_mut_fresh(bb(3), bb(4)));
        t!("m_raw_const_fresh", m_raw_const_fresh(bb(3), bb(4)));
        t!("m_raw_whole_fresh", m_raw_whole_fresh(bb(3), bb(4)));
        t!("m_raw_whole_copy", m_raw_whole_copy(s));
        t!("m_clone_from", m_clone_from(s));
        t!("m_clone", m_clone(s));
        t!("m_clone_arr", m_clone_arr(&aa));
        t!("m_default", m_default());
        t!("e_arr_static", e_arr_static());
        t!("e_arr_const_idx", e_arr_const_idx());
        t!("e_arr_whole_static", e_arr_whole_static());
        t!("e_arr_const", e_arr_const());
        t!("e_const_p", e_const_p());
        t!("e_const_n", e_const_n());
        t!("e_const_inline", e_const_inline());
        t!("e_const_inline_arr", e_const_inline_arr());
        t!("e_const_tuple", e_const_tuple());
        t!("e_const_promoted", e_const_promoted());
        t!("e_const_enum", e_const_enum());
        t!("e_const_option", e_const_option());
        t!("e_arr_param_copy", e_arr_param_copy(aa[1]));
        t!("e_arr2d", e_arr2d(&g));
        t!("e_arr2d_row", e_arr2d_row(&g));
        t!("e_tuple_static", e_tuple_static());
        t!("e_tuple_local_copy", e_tuple_local_copy());
        t!("e_arr_loop", e_arr_loop(&aa));
        t!("e_arr_whole_assign_then_elem", e_arr_whole_assign_then_elem(&aa));
        t!("e_cond_copy", e_cond_copy(s, bb(true)));
        t!("e_cond_copy2", e_cond_copy(s, bb(false)));
        t!("e_cond_w", e_cond_w(&ws, bb(true)));
        t!("e_cond_w2", e_cond_w(&ws, bb(false)));
        t!("e_cond_w_param", e_cond_w_param(bb(W(5)), bb(W(6)), bb(true)));
        t!("e_cond_w_param2", e_cond_w_param(bb(W(5)), bb(W(6)), bb(false)));
        t!("e_cond_write_w", e_cond_write_w(bb(W(5)), bb(true)));
        t!("e_cond_write_w2", e_cond_write_w(bb(W(5)), bb(false)));
        t!("e_cond_write_p", e_cond_write_p(s, bb(true)));
        t!("e_cond_write_p2", e_cond_write_p(s, bb(false)));
        t!("e_loop_w", e_loop_w(bb(W(5)), bb(3)));
        t!("e_loop_w2", e_loop_w2(bb(W(5)), bb(3)));
        t!("e_loop_p", e_loop_p(s, bb(3)));
        t!("e_w_swap", e_w_swap(bb(W(5)), bb(W(6))));
        t!("e_w_self_assign", e_w_self_assign(bb(W(5))));
        t!("e_w_two_copies", e_w_two_copies(bb(W(5))));
        t!("e_w_copy_back", e_w_copy_back(bb(W(5))));
        t!("s_subslice", s_subslice(&aa));
        t!("s_fill", s_fill(&aa));
        t!("s_reverse", s_reverse(&aa));
        t!("s_swap", s_swap(&aa));
        t!("s_copy_within", s_copy_within(&aa));
        t!("s_iter_mut", s_iter_mut(&aa));
        t!("s_split_at_mut", s_split_at_mut(&aa));
        t!("s_chunks_mut", s_chunks_mut(&aa));
        t!("s_last_mut", s_last_mut(&aa));
        t!("s_as_mut_ptr", s_as_mut_ptr(&aa));
        t!("s_sort", s_sort(&aa));
        t!("s_ps_slice", s_ps_slice(s));
        t!("s_ps_whole", s_ps_whole(&ps));
        t!("s_ps_swap", s_ps_swap(&ps));
        t!("s_copy_from_slice", s_copy_from_slice(&ps));
        t!("s_clone_from_slice", s_clone_from_slice(&ps));
        t!("s_fresh_arr_slice", s_fresh_arr_slice(bb(3), bb(4)));
        t!("s_fresh_arr_swap", s_fresh_arr_swap(bb(3), bb(4)));
        t!("s_fresh_arr_elem", s_fresh_arr_elem(bb(3), bb(4), bb(1)));
        t!("s_fresh_arr_iter", s_fresh_arr_iter(bb(3), bb(4)));
        t!("s_fresh_arr_ps", s_fresh_arr_ps(ps[0], ps[1]));
        t!("s_fresh_tuple", s_fresh_tuple(bb(3), bb(4)));
        t!("s_to_slice_fn", s_to_slice_fn(&aa));
        t!("s_to_arr_fn", s_to_arr_fn(&aa));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + ns[1].inner.x + op[1].unwrap().x + g[0][1][0] + g[0][1][1]);
    }
}
