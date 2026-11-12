// F5/F6: closures capturing by value and mutating; pattern bindings.
#[inline(never)] fn apply<F: FnMut() -> u32>(mut f: F) -> u32 { f() * 100 + f() }
#[inline(always)] fn apply_a<F: FnMut() -> u32>(mut f: F) -> u32 { f() * 100 + f() }
#[inline(never)] fn once<F: FnOnce() -> u32>(f: F) -> u32 { f() }

#[inline(never)] fn k_move_closure(s: &[P]) -> u32 { let mut p = s[bb(1)]; let mut c = move || { p.x += 1; p.x }; let a = c(); let b = c(); a * 10000 + b * 100 + p.x + s[1].x }
#[inline(never)] fn k_ref_closure(s: &[P]) -> u32 { let mut p = s[bb(1)]; let mut c = || { p.x += 1; }; c(); c(); p.x * 1000 + s[1].x }
#[inline(never)] fn k_move_closure_direct(s: &[P]) -> u32 { let p = s[bb(1)]; let c = move || { let mut q = p; q.x = 99; q.x }; c() * 1000 + p.x + s[1].x }
#[inline(never)] fn k_closure_array(a: &[[u32; 3]]) -> u32 { let mut arr = a[bb(1)]; let mut c = move |i: usize| { arr[i] += 5; arr[i] }; (c(0) + c(0) * 100) * 1000 + arr[0] * 10 + a[1][0] }
#[inline(never)] fn k_closure_copy(s: &[P]) -> u32 { let mut p = s[bb(1)]; let c = move |d: u32| p.x + d; let c2 = c; p.x = 99; c(1) * 10000 + c2(2) * 100 + p.x + s[1].x }
#[inline(never)] fn k_fnmut_passed(s: &[P]) -> u32 { let mut p = s[bb(1)]; let r = apply(move || { p.x += 1; p.x }); r * 1000 + p.x + s[1].x }
#[inline(never)] fn k_fnmut_passed_a(s: &[P]) -> u32 { let mut p = s[bb(1)]; let r = apply_a(move || { p.x += 1; p.x }); r * 1000 + p.x + s[1].x }
#[inline(never)] fn k_fnmut_param(mut p: P, q: P) -> u32 { let r = apply_a(move || { p.x += 1; p.x }); r * 1000 + p.x + q.x }
#[inline(never)] fn k_fnmut_w(w: W) -> u32 { let mut v = w; let r = apply_a(move || { v.0 += 1; v.0 }); r * 1000 + v.0 * 10 + w.0 }
#[inline(never)] fn k_fnmut_w_never(w: W) -> u32 { let mut v = w; let r = apply(move || { v.0 += 1; v.0 }); r * 1000 + v.0 * 10 + w.0 }
#[inline(never)] fn k_fnmut_scalar(n: u32) -> u32 { let mut v = n; let r = apply_a(move || { v += 1; v }); r * 1000 + v * 10 + n }
#[inline(never)] fn k_fnonce(s: &[P]) -> u32 { let p = s[bb(1)]; let r = once(move || { let mut q = p; q.y = 99; q.y + q.x }); r * 1000 + p.y + s[1].y }
#[inline(never)] fn k_fnonce_mut(s: &[P]) -> u32 { let mut p = s[bb(1)]; let r = once(move || { p.y = 99; p.y + p.x }); r * 1000 + p.y + s[1].y }
#[inline(never)] fn k_closure_env_copy(s: &[P]) -> u32 { let p = s[bb(1)]; let mut c = move || { let mut q = p; q.x += 1; q.x }; let mut c2 = c; (c() + c2() * 100) * 1000 + p.x + s[1].x }
#[inline(never)] fn k_closure_env_mut_copy(s: &[P]) -> u32 { let mut p = s[bb(1)]; let mut c = move || { p.x += 1; p.x }; let a = c(); let mut c2 = c; let b = c2(); let d = c(); (a * 10000 + b * 100 + d) * 1000 + p.x + s[1].x }
#[inline(never)] fn k_closure_fresh(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let mut c = move || { p.x += 1; p.x }; (c() + c()) * 10000 + n * 100 + p.x }
#[inline(never)] fn k_closure_ret(s: &[P]) -> u32 {
    #[inline(always)] fn mk(mut p: P) -> impl FnMut() -> u32 { move || { p.x += 1; p.x } }
    let mut f = mk(s[bb(1)]); let mut g = mk(s[bb(1)]); (f() + f() * 10 + g() * 100) * 1000 + s[1].x
}

#[inline(never)] fn p_ref_mut_temp(s: &[P]) -> u32 { let ref mut r = { s[bb(1)] }; r.x = 99; r.x * 1000 + s[1].x }
#[inline(never)] fn p_match_ref_mut(s: &[P]) -> u32 { match { s[bb(1)] } { ref mut p => { p.x = 99; p.x * 1000 + s[1].x } } }
#[inline(never)] fn p_destructure_ref_mut(s: &[P]) -> u32 { let mut c = s[bb(1)]; let P { x: ref mut xr, y } = c; *xr = 99; c.x * 1000 + s[1].x + y }
#[inline(never)] fn p_destructure_ref_mut_temp(s: &[P]) -> u32 { let P { x: ref mut xr, y } = { s[bb(1)] }; *xr = 99; *xr * 1000 + s[1].x + y }
#[inline(never)] fn p_destructure_mut(s: &[P]) -> u32 { let P { mut x, mut y } = s[bb(1)]; x = 99; y += x; (x + y) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn p_at_binding(s: &[P]) -> u32 { match s[bb(1)] { mut p @ P { x: 3, .. } => { p.y = 99; p.y * 1000 + s[1].y } , _ => 0 } }
#[inline(never)] fn p_at_option(o: &[Option<P>]) -> u32 { match o[bb(1)] { mut w @ Some(_) => { if let Some(ref mut p) = w { p.x = 99; } w.unwrap().x * 1000 + o[1].unwrap().x }, None => 0 } }
#[inline(never)] fn p_at_ref(s: &[P]) -> u32 { match s[bb(1)] { ref p @ P { x, .. } => { let mut q = *p; q.x = 99; q.x * 1000 + p.x + x + s[1].x } } }
#[inline(never)] fn p_if_let_some(s: &[P]) -> u32 { if let Some(ref mut p) = Some(s[bb(1)]) { p.x = 99; p.x * 1000 + s[1].x } else { 0 } }
#[inline(never)] fn p_if_let_some_mut(s: &[P]) -> u32 { if let Some(mut p) = Some(s[bb(1)]) { p.x = 99; p.x * 1000 + s[1].x } else { 0 } }
#[inline(never)] fn p_arr_pat(a: &[[u32; 3]]) -> u32 { let [ref mut f, ..] = { a[bb(1)] }; *f = 99; *f * 1000 + a[1][0] }
#[inline(never)] fn p_arr_pat2(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let [_, ref mut m, _] = c; *m = 99; c[1] * 1000 + a[1][1] }
#[inline(never)] fn p_arr_pat3(a: &[[u32; 3]]) -> u32 { let [mut f, mut g, h] = a[bb(1)]; f = 99; g += f; (f + g + h) * 1000 + a[1][0] + a[1][1] }
#[inline(never)] fn p_arr_pat_rest(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let [first, ref mut rest @ ..] = c; rest[1] = 99; (c[2] + first) * 1000 + a[1][2] }
#[inline(never)] fn p_arr_pat_structs(s: &[P; 3]) -> u32 { let [mut a, _, ref c] = *s; a.x = 99; (a.x + c.y) * 1000 + s[0].x }
#[inline(never)] fn p_tuple_pat(s: &[P]) -> u32 { let (mut p, mut q) = (s[bb(0)], s[bb(1)]); p.x = 99; q.y = 98; (p.x + q.y) * 1000 + s[0].x + s[1].y }
#[inline(never)] fn p_tuple_pat_ref(s: &[P]) -> u32 { let mut t = (s[bb(0)], s[bb(1)]); let (ref mut p, ref mut q) = t; p.x = 99; q.y = 98; (t.0.x + t.1.y) * 1000 + s[0].x + s[1].y }
#[inline(never)] fn p_for_loop_copy(s: &[P]) -> u32 { let mut t = 0; for mut p in s.iter().copied() { p.x += 100; t += p.x; } t * 1000 + sum(s) }
#[inline(never)] fn p_for_deref(s: &[P]) -> u32 { let mut t = 0; for &(mut p) in s { p.y = 99; t += p.y + p.x; } t * 1000 + sum(s) }
#[inline(never)] fn p_for_array(s: &[P; 3]) -> u32 { let mut t = 0; for mut p in *s { p.y = 99; t += p.y + p.x; } t * 1000 + sum(s) }
#[inline(never)] fn p_for_w(ws: &[W]) -> u32 { let mut t = 0; for mut w in ws.iter().copied() { w.0 += 100; t += w.0; } t * 1000 + ws[0].0 + ws[2].0 }
#[inline(never)] fn p_for_w_arr(ws: &[W; 3]) -> u32 { let mut t = 0; for mut w in *ws { w.0 += 100; t += w.0; } t * 1000 + ws[0].0 + ws[2].0 }
#[inline(never)] fn p_while_let(s: &[P]) -> u32 { let mut o = Some(s[bb(1)]); let mut t = 0; while let Some(mut p) = o { p.x += 50; t += p.x; if p.x > 150 { o = None } else { o = Some(p) } } t * 1000 + s[1].x }
#[inline(never)] fn p_enum_payload_bind(es: &[E]) -> u32 { match es[bb(2)] { E::C(mut p) => { p.x = 99; p.x * 1000 + obs(&es[2]) } _ => 0 } }
#[inline(never)] fn p_enum_payload_scalar(es: &[E]) -> u32 { match es[bb(1)] { E::B(mut a, b) => { a = 99; (a + b) * 1000 + obs(&es[1]) } _ => 0 } }
#[inline(never)] fn p_enum_ref_mut_temp(es: &[E]) -> u32 { match { es[bb(2)] } { E::C(ref mut p) => { p.x = 99; p.x * 1000 + obs(&es[2]) } _ => 0 } }
#[inline(never)] fn p_enum_ref_mut_local(es: &[E]) -> u32 { let mut e = es[bb(1)]; match e { E::B(ref mut a, _) => { *a = 99; } _ => {} } obs(&e) * 1000 + obs(&es[1]) }
#[inline(never)] fn p_slice_pat(s: &[P]) -> u32 { if let [first, .., last] = s { let mut f = *first; f.x = 99; let mut l = *last; l.y = 98; (f.x + l.y) * 1000 + sum(s) } else { 0 } }
#[inline(never)] fn p_ref_pat_deref(s: &[P]) -> u32 { let &P { mut x, y } = &s[bb(1)]; x = 99; x * 1000 + s[1].x + y }
#[inline(never)] fn p_nested_pat(ns: &[N]) -> u32 { let N { a, inner: mut q } = ns[bb(1)]; q.x = 99; (q.x + a) * 1000 + ns[1].inner.x }
#[inline(never)] fn p_nested_pat_ref(ns: &[N]) -> u32 { let mut n = ns[bb(1)]; let N { a, inner: P { ref mut x, .. } } = n; *x = 99; (n.inner.x + a) * 1000 + ns[1].inner.x }
#[inline(never)] fn p_w_pat(ws: &[W]) -> u32 { let W(mut v) = ws[bb(1)]; v = 99; v * 1000 + ws[1].0 }
#[inline(never)] fn p_w_ref_mut(ws: &[W]) -> u32 { let mut w = ws[bb(1)]; let W(ref mut v) = w; *v = 99; w.0 * 1000 + ws[1].0 }
#[inline(never)] fn p_w_ref_mut_param(a: W) -> u32 { let mut w = a; let W(ref mut v) = w; *v = 99; w.0 * 1000 + a.0 }
#[inline(never)] fn p_w_ref_mut_temp(a: W) -> u32 { let W(ref mut v) = { a }; *v = 99; *v * 1000 + a.0 }
#[inline(never)] fn p_param_mut(mut p: P, q: P) -> u32 { p.x = 99; p.x * 1000 + q.x }
#[inline(never)] fn p_param_copy(p: P) -> u32 { let mut q = p; q.x = 99; let mut r = p; r.y = 98; (q.x + r.y) * 1000 + p.x + p.y + q.y + r.x }
#[inline(never)] fn p_shadow(s: &[P]) -> u32 { let p = s[bb(1)]; let mut p2 = p; p2.x = 99; let p = p2; let mut p3 = p; p3.y = 98; (p.x + p3.y + p2.y) * 1000 + s[1].x + s[1].y }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let es = bb([E::A(1), E::B(2, 3), E::C(P { x: 4, y: 5 })]);
        let ws = bb([W(10), W(11), W(12)]);
        let ns = bb([N { a: 1, inner: P { x: 2, y: 3 } }, N { a: 4, inner: P { x: 5, y: 6 } }]);
        let op = bb([None, Some(P { x: 31, y: 32 })]);
        t!("k_move_closure", k_move_closure(s));
        t!("k_ref_closure", k_ref_closure(s));
        t!("k_move_closure_direct", k_move_closure_direct(s));
        t!("k_closure_array", k_closure_array(&aa));
        t!("k_closure_copy", k_closure_copy(s));
        t!("k_fnmut_passed", k_fnmut_passed(s));
        t!("k_fnmut_passed_a", k_fnmut_passed_a(s));
        t!("k_fnmut_param", k_fnmut_param(bb(P { x: 7, y: 8 }), bb(P { x: 7, y: 8 })));
        t!("k_fnmut_w", k_fnmut_w(bb(W(5))));
        t!("k_fnmut_w_never", k_fnmut_w_never(bb(W(5))));
        t!("k_fnmut_scalar", k_fnmut_scalar(bb(5)));
        t!("k_fnonce", k_fnonce(s));
        t!("k_fnonce_mut", k_fnonce_mut(s));
        t!("k_closure_env_copy", k_closure_env_copy(s));
        t!("k_closure_env_mut_copy", k_closure_env_mut_copy(s));
        t!("k_closure_fresh", k_closure_fresh(bb(3), bb(4)));
        t!("k_closure_ret", k_closure_ret(s));
        t!("p_ref_mut_temp", p_ref_mut_temp(s));
        t!("p_match_ref_mut", p_match_ref_mut(s));
        t!("p_destructure_ref_mut", p_destructure_ref_mut(s));
        t!("p_destructure_ref_mut_temp", p_destructure_ref_mut_temp(s));
        t!("p_destructure_mut", p_destructure_mut(s));
        t!("p_at_binding", p_at_binding(s));
        t!("p_at_option", p_at_option(&op));
        t!("p_at_ref", p_at_ref(s));
        t!("p_if_let_some", p_if_let_some(s));
        t!("p_if_let_some_mut", p_if_let_some_mut(s));
        t!("p_arr_pat", p_arr_pat(&aa));
        t!("p_arr_pat2", p_arr_pat2(&aa));
        t!("p_arr_pat3", p_arr_pat3(&aa));
        t!("p_arr_pat_rest", p_arr_pat_rest(&aa));
        t!("p_arr_pat_structs", p_arr_pat_structs(&ps));
        t!("p_tuple_pat", p_tuple_pat(s));
        t!("p_tuple_pat_ref", p_tuple_pat_ref(s));
        t!("p_for_loop_copy", p_for_loop_copy(s));
        t!("p_for_deref", p_for_deref(s));
        t!("p_for_array", p_for_array(&ps));
        t!("p_for_w", p_for_w(&ws));
        t!("p_for_w_arr", p_for_w_arr(&ws));
        t!("p_while_let", p_while_let(s));
        t!("p_enum_payload_bind", p_enum_payload_bind(&es));
        t!("p_enum_payload_scalar", p_enum_payload_scalar(&es));
        t!("p_enum_ref_mut_temp", p_enum_ref_mut_temp(&es));
        t!("p_enum_ref_mut_local", p_enum_ref_mut_local(&es));
        t!("p_slice_pat", p_slice_pat(s));
        t!("p_ref_pat_deref", p_ref_pat_deref(s));
        t!("p_nested_pat", p_nested_pat(&ns));
        t!("p_nested_pat_ref", p_nested_pat_ref(&ns));
        t!("p_w_pat", p_w_pat(&ws));
        t!("p_w_ref_mut", p_w_ref_mut(&ws));
        t!("p_w_ref_mut_param", p_w_ref_mut_param(bb(W(5))));
        t!("p_w_ref_mut_temp", p_w_ref_mut_temp(bb(W(5))));
        t!("p_param_mut", p_param_mut(ps[1], ps[1]));
        t!("p_param_copy", p_param_copy(ps[1]));
        t!("p_shadow", p_shadow(s));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + obs(&es[1]) + obs(&es[2]) + ws[1].0 + ns[1].inner.x);
    }
}
