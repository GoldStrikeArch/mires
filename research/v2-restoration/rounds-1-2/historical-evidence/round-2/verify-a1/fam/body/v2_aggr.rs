// F2: a copy moved into an aggregate that is then written through; fresh aggregates of operands.
#[inline(never)] fn a_struct(s: &[P]) -> u32 { let mut n = N { a: 1, inner: s[bb(1)] }; n.inner.x = 99; n.inner.x * 1000 + s[1].x }
#[inline(never)] fn a_struct_ref(s: &[P]) -> u32 { let mut n = N { a: 1, inner: s[bb(1)] }; let r = &mut n.inner; r.x = 99; n.inner.x * 1000 + s[1].x }
#[inline(never)] fn a_struct_static() -> u32 { let mut n = N { a: 1, inner: PS[bb(1)] }; n.inner.y = 99; n.inner.y * 1000 + PS[1].y }
#[inline(never)] fn a_tuple(s: &[P]) -> u32 { let mut t = (s[bb(1)], 5u32); t.0.y = 99; t.0.y * 1000 + s[1].y }
#[inline(never)] fn a_array(s: &[P]) -> u32 { let mut a = [s[bb(0)], s[bb(1)]]; a[bb(1)].x = 99; a[0].x = 98; sum(&a) * 1000 + sum(s) }
#[inline(never)] fn a_option(s: &[P]) -> u32 { let mut o = Some(s[bb(1)]); if let Some(ref mut p) = o { p.x = 99; } o.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn a_option_asmut(s: &[P]) -> u32 { let mut o = Some(s[bb(1)]); o.as_mut().unwrap().x = 99; o.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn a_option_insert(s: &[P]) -> u32 { let mut o: Option<P> = None; let r = o.insert(s[bb(1)]); r.x = 99; o.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn a_option_get_or(s: &[P]) -> u32 { let mut o: Option<P> = None; o.get_or_insert(s[bb(1)]).y = 99; o.unwrap().y * 1000 + s[1].y }
#[inline(never)] fn a_enum(s: &[P]) -> u32 { let mut e = E::C(s[bb(1)]); if let E::C(p) = &mut e { p.y = 99 } obs(&e) * 1000 + s[1].y }
#[inline(never)] fn a_enum_param(p: P) -> u32 { let mut e = E::C(p); if let E::C(q) = &mut e { q.y = 99 } obs(&e) * 1000 + p.y }
#[inline(never)] fn a_nested(s: &[N]) -> u32 { let mut t = (s[bb(0)], [s[bb(1)]; 2]); t.1[1].inner.x = 99; t.0.inner.y = 98; (t.1[1].inner.x + t.0.inner.y + t.1[0].inner.x) * 1000 + s[0].inner.y + s[1].inner.x }
#[inline(never)] fn a_local_then_agg() -> u32 { let p = bb(P { x: 1, y: 2 }); let mut n = N { a: 3, inner: p }; n.inner.x = 99; n.inner.x * 1000 + p.x }
#[inline(never)] fn a_param_agg(p: P) -> u32 { let mut n = N { a: 3, inner: p }; n.inner.x = 99; n.inner.x * 1000 + p.x }
#[inline(never)] fn a_param_agg_ref(p: &P) -> u32 { let mut n = N { a: 3, inner: *p }; n.inner.x = 99; n.inner.x * 1000 + p.x }
#[inline(never)] fn a_scalar_agg(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; p.x = 99; p.x * 10000 + n * 100 + m }
#[inline(never)] fn a_scalar_agg2(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; p.x += 1; p.y += p.x; p.x * 10000 + p.y * 100 + n + m }
#[inline(never)] fn a_scalar_tuple(n: u32, m: u32) -> u32 { let mut t = (n, m); t.0 = 99; t.0 * 10000 + n * 100 + m }
#[inline(never)] fn a_scalar_arr(n: u32, m: u32) -> u32 { let mut a = [n, m]; a[bb(0)] = 99; a[0] * 10000 + n * 100 + m }
#[inline(never)] fn a_scalar_arr_const(n: u32, m: u32) -> u32 { let mut a = [n, m]; a[1] = 99; a[1] * 10000 + n * 100 + m }
#[inline(never)] fn a_scalar_enum(n: u32, m: u32) -> u32 { let mut e = E::B(n, m); if let E::B(a, _) = &mut e { *a = 99; } obs(&e) * 1000 + n }
#[inline(never)] fn a_scalar_option(n: u32) -> u32 { let mut o = Some(n); if let Some(v) = &mut o { *v = 99; } o.unwrap() * 1000 + n }
#[inline(never)] fn a_scalar_option2(n: u32) -> u32 { let mut o = Some(n); *o.as_mut().unwrap() += 1; o.unwrap() * 1000 + n }
#[inline(never)] fn a_slice_elem_agg(s: &[u32]) -> u32 { let mut p = P { x: s[bb(0)], y: s[bb(1)] }; p.x = 99; p.y += 1; (p.x + p.y) * 1000 + s[0] + s[1] }
#[inline(never)] fn a_slice_elem_arr(s: &[u32]) -> u32 { let mut a = [s[bb(0)], s[bb(1)]]; a[0] = 99; a[bb(1)] += 1; (a[0] + a[1]) * 1000 + s[0] + s[1] }
#[inline(never)] fn a_static_elem_agg() -> u32 { let mut p = P { x: AA[bb(0)][0], y: AA[1][bb(1)] }; p.x = 99; p.y += 1; (p.x + p.y) * 1000 + AA[0][0] + AA[1][1] }
#[inline(never)] fn a_field_agg(q: &P) -> u32 { let mut p = P { x: q.x, y: q.y }; p.x = 99; p.x * 1000 + q.x }
#[inline(never)] fn a_field_agg_mut(q: &mut P) -> u32 { let mut p = P { x: q.x, y: q.y }; p.x = 99; q.y = 5; p.x * 1000 + q.x + p.y }
#[inline(never)] fn a_w_agg(n: u32) -> u32 { let mut w = W(n); w.0 = 99; w.0 * 1000 + n }
#[inline(never)] fn a_w_in_struct(w: W) -> u32 { let mut h = HW { w, pad: 1 }; h.w.0 = 99; h.w.0 * 1000 + w.0 }
#[inline(never)] fn a_w_in_struct2(w: W) -> u32 { let mut h = HW { w, pad: 1 }; h.w = W(98); h.w.0 += 1; h.w.0 * 1000 + w.0 }
#[inline(never)] fn a_w_in_tuple(w: W, k: u32) -> u32 { let mut t = (w, k); t.0.0 = 99; t.1 = 98; (t.0.0 + t.1) * 10000 + w.0 * 100 + k }
#[inline(never)] fn a_w_in_array(ws: &[W]) -> u32 { let mut a = [ws[0], ws[1]]; a[bb(1)].0 = 99; a[0].0 += 1; (a[0].0 + a[1].0) * 1000 + ws[0].0 + ws[1].0 }
#[inline(never)] fn a_w_in_option(w: W) -> u32 { let mut o = Some(w); if let Some(v) = &mut o { v.0 = 99; } o.unwrap().0 * 1000 + w.0 }
#[inline(never)] fn a_repeat(s: &[P]) -> u32 { let mut a = [s[bb(1)]; 3]; a[bb(2)].x = 99; (a[0].x + a[2].x) * 1000 + s[1].x }
#[inline(never)] fn a_repeat_scalar(n: u32) -> u32 { let mut a = [n; 4]; a[bb(2)] = 99; (a[1] + a[2]) * 1000 + n }
#[inline(never)] fn a_repeat_param(p: P) -> u32 { let mut a = [p; 2]; a[1].x = 99; a[0].y = 98; (a[0].x + a[1].y + a[1].x + a[0].y) * 1000 + p.x + p.y }
#[inline(never)] fn a_repeat_w(w: W) -> u32 { let mut a = [w; 3]; a[bb(1)].0 = 99; (a[0].0 + a[1].0) * 1000 + w.0 }
#[inline(never)] fn a_struct_update(s: &[P]) -> u32 { let mut q = P { x: 9, ..s[bb(1)] }; q.y = 99; let mut n = N { a: 1, ..NS[bb(1)] }; n.inner.x = 98; (q.y + n.inner.x) * 1000 + s[1].y + NS[1].inner.x }
#[inline(never)] fn a_box(s: &[P]) -> u32 { let mut b = Box::new(s[bb(1)]); b.x = 99; b.x * 1000 + s[1].x }
#[inline(never)] fn a_box_n(s: &[N]) -> u32 { let mut b = Box::new(s[bb(1)]); b.inner.x = 99; let c = *b; c.inner.x * 1000 + s[1].inner.x }
#[inline(never)] fn a_box_copy_out(s: &[N]) -> u32 { let b = Box::new(s[bb(1)]); let mut c = *b; c.inner.x = 99; let mut d = b.inner; d.y = 98; (c.inner.x + d.y) * 1000 + b.inner.x + b.inner.y + s[1].inner.x }
#[inline(never)] fn a_box_fresh(n: u32, m: u32) -> u32 { let mut b = Box::new(P { x: n, y: m }); b.x = 99; b.x * 10000 + n * 100 + m }
#[inline(never)] fn a_box_scalar(n: u32) -> u32 { let mut b = Box::new(n); *b = 99; *b * 1000 + n }
#[inline(never)] fn a_box_w(w: W) -> u32 { let mut b = Box::new(w); b.0 = 99; b.0 * 1000 + w.0 }
#[inline(never)] fn a_vec(s: &[P]) -> u32 { let mut v = Vec::new(); v.push(s[bb(1)]); v[0].x = 99; v[0].x * 1000 + s[1].x }
#[inline(never)] fn a_vec_from(s: &[P]) -> u32 { let mut v = s.to_vec(); v[1].x = 99; sum(&v) * 1000 + sum(s) }
#[inline(never)] fn a_vec_scalar(n: u32, m: u32) -> u32 { let mut v = alloc::vec![n, m]; v[0] = 99; v[0] * 10000 + n * 100 + m }
#[inline(never)] fn a_vec_param(p: P) -> u32 { let mut v = alloc::vec![p, p]; v[0].x = 99; v[1].y = 98; (v[0].x + v[1].y + v[1].x) * 1000 + p.x + p.y }
#[inline(never)] fn a_cell(s: &[P]) -> u32 { let c = Cell::new(s[bb(1)]); let mut v = c.get(); v.x = 99; let before = c.get().x; c.set(v); (before * 100 + c.get().x) * 1000 + s[1].x }
#[inline(never)] fn a_arr_of_arr(a: &[[u32; 3]]) -> u32 { let mut b = [a[bb(0)], a[bb(1)]]; b[1][2] = 99; b[1][2] * 1000 + a[1][2] }
#[inline(never)] fn a_t3(ts: &[T3]) -> u32 { let mut t = ts[bb(1)]; t.0.x = 99; t.1[bb(1)] = 98; t.2 = 97; (t.0.x + t.1[1] + t.2) * 1000 + ts[1].0.x + ts[1].1[1] + ts[1].2 }
#[inline(never)] fn a_t3_fresh(p: P, a: [u32; 2], k: u32) -> u32 { let mut t = T3(p, a, k); t.0.x = 99; t.1[bb(1)] = 98; t.2 = 97; (t.0.x + t.1[1] + t.2) * 1000 + p.x + a[1] + k }
#[inline(never)] fn a_chain(s: &[P]) -> u32 { let a = s[bb(1)]; let b = a; let mut c = b; c.x = 99; let d = c; let mut e = d; e.y = 98; (a.x + b.x + c.x + d.y + e.y) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn a_chain_w(w: W) -> u32 { let a = w; let b = a; let mut c = b; c.0 = 99; let d = c; let mut e = d; e.0 = 98; a.0 + b.0 * 10 + c.0 * 100 + d.0 * 1000 + e.0 * 10000 + w.0 }
#[inline(never)] fn a_ret_agg(s: &[P]) -> u32 {
    #[inline(always)] fn mk(p: P, k: u32) -> N { N { a: k, inner: p } }
    let mut n = mk(s[bb(1)], 3); n.inner.x = 99; let mut m = mk(s[bb(1)], 4); m.inner.y = 98; (n.inner.x + m.inner.y + n.inner.y + m.inner.x) * 1000 + s[1].x + s[1].y
}
#[inline(never)] fn a_ret_agg_scalar(n: u32, m: u32) -> u32 {
    #[inline(always)] fn mk(x: u32, y: u32) -> P { P { x, y } }
    let mut p = mk(n, m); p.x = 99; let q = mk(n, m); (p.x + q.x) * 10000 + n * 100 + m
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let mut ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let ns = bb([N { a: 1, inner: P { x: 2, y: 3 } }, N { a: 4, inner: P { x: 5, y: 6 } }]);
        let ts = bb([T3(P { x: 1, y: 2 }, [3, 4], 5), T3(P { x: 6, y: 7 }, [8, 9], 10)]);
        let us = bb([13u32, 14, 15]);
        t!("a_struct", a_struct(s));
        t!("a_struct_ref", a_struct_ref(s));
        t!("a_struct_static", a_struct_static());
        t!("a_tuple", a_tuple(s));
        t!("a_array", a_array(s));
        t!("a_option", a_option(s));
        t!("a_option_asmut", a_option_asmut(s));
        t!("a_option_insert", a_option_insert(s));
        t!("a_option_get_or", a_option_get_or(s));
        t!("a_enum", a_enum(s));
        t!("a_enum_param", a_enum_param(bb(P { x: 7, y: 8 })));
        t!("a_nested", a_nested(&ns));
        t!("a_local_then_agg", a_local_then_agg());
        t!("a_param_agg", a_param_agg(bb(P { x: 7, y: 8 })));
        t!("a_param_agg_ref", a_param_agg_ref(&s[2]));
        t!("a_scalar_agg", a_scalar_agg(bb(3), bb(4)));
        t!("a_scalar_agg2", a_scalar_agg2(bb(3), bb(4)));
        t!("a_scalar_tuple", a_scalar_tuple(bb(3), bb(4)));
        t!("a_scalar_arr", a_scalar_arr(bb(3), bb(4)));
        t!("a_scalar_arr_const", a_scalar_arr_const(bb(3), bb(4)));
        t!("a_scalar_enum", a_scalar_enum(bb(3), bb(4)));
        t!("a_scalar_option", a_scalar_option(bb(3)));
        t!("a_scalar_option2", a_scalar_option2(bb(3)));
        t!("a_slice_elem_agg", a_slice_elem_agg(&us));
        t!("a_slice_elem_arr", a_slice_elem_arr(&us));
        t!("a_static_elem_agg", a_static_elem_agg());
        t!("a_field_agg", a_field_agg(&s[1]));
        t!("a_field_agg_mut", a_field_agg_mut(&mut bb(P { x: 7, y: 8 })));
        t!("a_w_agg", a_w_agg(bb(3)));
        t!("a_w_in_struct", a_w_in_struct(bb(W(5))));
        t!("a_w_in_struct2", a_w_in_struct2(bb(W(5))));
        t!("a_w_in_tuple", a_w_in_tuple(bb(W(5)), bb(6)));
        t!("a_w_in_array", a_w_in_array(&ws));
        t!("a_w_in_option", a_w_in_option(bb(W(5))));
        t!("a_repeat", a_repeat(s));
        t!("a_repeat_scalar", a_repeat_scalar(bb(3)));
        t!("a_repeat_param", a_repeat_param(bb(P { x: 7, y: 8 })));
        t!("a_repeat_w", a_repeat_w(bb(W(5))));
        t!("a_struct_update", a_struct_update(s));
        t!("a_box", a_box(s));
        t!("a_box_n", a_box_n(&ns));
        t!("a_box_copy_out", a_box_copy_out(&ns));
        t!("a_box_fresh", a_box_fresh(bb(3), bb(4)));
        t!("a_box_scalar", a_box_scalar(bb(3)));
        t!("a_box_w", a_box_w(bb(W(5))));
        t!("a_vec", a_vec(s));
        t!("a_vec_from", a_vec_from(s));
        t!("a_vec_scalar", a_vec_scalar(bb(3), bb(4)));
        t!("a_vec_param", a_vec_param(bb(P { x: 7, y: 8 })));
        t!("a_cell", a_cell(s));
        t!("a_arr_of_arr", a_arr_of_arr(&aa));
        t!("a_t3", a_t3(&ts));
        t!("a_t3_fresh", a_t3_fresh(bb(P { x: 7, y: 8 }), bb([1, 2]), bb(3)));
        t!("a_chain", a_chain(s));
        t!("a_chain_w", a_chain_w(bb(W(5))));
        t!("a_ret_agg", a_ret_agg(s));
        t!("a_ret_agg_scalar", a_ret_agg_scalar(bb(3), bb(4)));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + ns[1].inner.x + suma(&us) + ts[1].0.x + ts[1].1[1]);
    }
}
