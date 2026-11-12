// Clone depth: copies of nested aggregates of several kinds, written deep; source observed.
#[derive(Clone, Copy)] enum Big { V0, V1(u32), V2(u32), V3(u32), V4(u32), V5(u32), V6(u32), V7(u32), V8(P), V9([u32; 2]) }
#[derive(Clone, Copy)] enum Eight { V0, V1(u32), V2(u32), V3(u32), V4(u32), V5(u32), V6(P), V7([u32; 2]) }
#[derive(Clone, Copy)] enum EN { A(N), B([P; 2]), C((P, u32)), D(Option<P>) }
#[derive(Clone, Copy)] struct Deep { t: (P, [N; 2]), o: Option<[u32; 3]>, e: E, w: W, hw: HW }
#[derive(Clone, Copy)] struct WrapN(N);
#[repr(transparent)] #[derive(Clone, Copy)] struct TN(N);
#[repr(transparent)] #[derive(Clone, Copy)] struct TA([u32; 3]);
#[derive(Clone, Copy)] struct WithMu { m: MaybeUninit<P>, k: u32 }
#[derive(Clone, Copy)] struct WithRef<'a> { r: &'a P, p: P }
fn rs(r: &Result<P, [u32; 2]>) -> u32 { match r { Ok(p) => p.x, Err(a) => a[1] } }
fn big(b: &Big) -> u32 { match b { Big::V8(p) => p.x * 10 + p.y, Big::V9(a) => a[0] * 10 + a[1], Big::V1(v) => *v, _ => 0 } }
fn eight(b: &Eight) -> u32 { match b { Eight::V6(p) => p.x * 10 + p.y, Eight::V7(a) => a[0] * 10 + a[1], Eight::V1(v) => *v, _ => 0 } }
fn en(e: &EN) -> u32 { match e { EN::A(n) => n.a + n.inner.x * 10 + n.inner.y * 100, EN::B(a) => sum(a), EN::C(t) => t.0.x + t.1 * 10, EN::D(o) => o.map_or(1, |p| p.x) } }
fn deep(d: &Deep) -> u32 { d.t.0.x + d.t.1[1].inner.y * 3 + d.o.map_or(0, |a| a[1]) * 5 + obs(&d.e) * 7 + d.w.0 * 11 + d.hw.w.0 * 13 }

#[inline(never)] fn q_big_p(s: &[Big]) -> u32 { let mut c = s[bb(0)]; if let Big::V8(p) = &mut c { p.x = 99; } big(&c) * 1000 + big(&s[0]) }
#[inline(never)] fn q_big_a(s: &[Big]) -> u32 { let mut c = s[bb(1)]; if let Big::V9(a) = &mut c { a[bb(1)] = 99; } big(&c) * 1000 + big(&s[1]) }
#[inline(never)] fn q_big_dead(s: &[Big]) -> u32 { let mut c = s[bb(0)]; if let Big::V8(p) = &mut c { p.x = 99; } big(&s[0]) }
#[inline(never)] fn q_big_param(b: Big) -> u32 { let mut c = b; if let Big::V8(p) = &mut c { p.x = 99; } big(&c) * 1000 + big(&b) }
#[inline(never)] fn q_big_scalar(s: &[Big]) -> u32 { let mut c = s[bb(2)]; if let Big::V1(v) = &mut c { *v = 99; } big(&c) * 1000 + big(&s[2]) }
#[inline(never)] fn q_big_in_struct(s: &[(Big, u32)]) -> u32 { let mut c = s[bb(0)]; if let Big::V8(p) = &mut c.0 { p.y = 99; } c.1 = 98; big(&c.0) * 1000 + big(&s[0].0) + s[0].1 }
#[inline(never)] fn q_big_arr(s: &[Big; 3]) -> u32 { let mut c = *s; if let Big::V8(p) = &mut c[bb(0)] { p.y = 99; } big(&c[0]) * 1000 + big(&s[0]) }
#[inline(never)] fn q_eight_p(s: &[Eight]) -> u32 { let mut c = s[bb(0)]; if let Eight::V6(p) = &mut c { p.x = 99; } eight(&c) * 1000 + eight(&s[0]) }
#[inline(never)] fn q_eight_a(s: &[Eight]) -> u32 { let mut c = s[bb(1)]; if let Eight::V7(a) = &mut c { a[bb(1)] = 99; } eight(&c) * 1000 + eight(&s[1]) }
#[inline(never)] fn q_en_a(s: &[EN]) -> u32 { let mut c = s[bb(0)]; if let EN::A(n) = &mut c { n.inner.x = 99; } en(&c) * 1000 + en(&s[0]) }
#[inline(never)] fn q_en_b(s: &[EN]) -> u32 { let mut c = s[bb(1)]; if let EN::B(a) = &mut c { a[bb(1)].x = 99; } en(&c) * 1000 + en(&s[1]) }
#[inline(never)] fn q_en_c(s: &[EN]) -> u32 { let mut c = s[bb(2)]; if let EN::C(t) = &mut c { t.0.x = 99; t.1 = 9; } en(&c) * 1000 + en(&s[2]) }
#[inline(never)] fn q_en_d(s: &[EN]) -> u32 { let mut c = s[bb(3)]; if let EN::D(Some(p)) = &mut c { p.x = 99; } en(&c) * 1000 + en(&s[3]) }
#[inline(never)] fn q_en_dead(s: &[EN]) -> u32 { let mut c = s[bb(1)]; if let EN::B(a) = &mut c { a[bb(1)].x = 99; } let mut d = s[bb(3)]; if let EN::D(Some(p)) = &mut d { p.x = 99; } en(&s[1]) * 1000 + en(&s[3]) }
#[inline(never)] fn q_deep(s: &[Deep]) -> u32 { let mut c = s[bb(0)]; c.t.0.x = 99; c.t.1[bb(1)].inner.y = 98; if let Some(a) = &mut c.o { a[1] = 97; } if let E::B(a, _) = &mut c.e { *a = 96; } c.w.0 = 95; c.hw.w.0 = 94; deep(&c) * 10000 + deep(&s[0]) }
#[inline(never)] fn q_deep_dead(s: &[Deep]) -> u32 { let mut c = s[bb(0)]; c.t.0.x = 99; c.t.1[bb(1)].inner.y = 98; if let Some(a) = &mut c.o { a[1] = 97; } if let E::B(a, _) = &mut c.e { *a = 96; } c.w.0 = 95; c.hw.w.0 = 94; deep(&s[0]) }
#[inline(never)] fn q_deep_param(d: Deep) -> u32 { let mut c = d; c.t.1[bb(0)].inner.x = 98; c.hw.w = W(1); let mut e = c; e.t.1[0].inner.x = 97; deep(&c) * 10000 + deep(&d) + c.t.1[0].inner.x + e.t.1[0].inner.x + d.t.1[0].inner.x }
#[inline(never)] fn q_deep_sub(s: &[Deep]) -> u32 { let mut t = s[bb(0)].t; t.1[1].inner.y = 99; let mut a = s[bb(0)].t.1; a[0].a = 98; let mut n = s[bb(0)].t.1[bb(1)]; n.inner.x = 97; let mut o = s[0].o; if let Some(v) = &mut o { v[0] = 96; } (t.1[1].inner.y + a[0].a + n.inner.x + o.unwrap()[0]) * 10000 + deep(&s[0]) + s[0].t.1[0].a + s[0].t.1[1].inner.x + s[0].o.unwrap()[0] }
#[inline(never)] fn q_wrapn(s: &[WrapN]) -> u32 { let mut c = s[bb(0)]; c.0.inner.x = 99; c.0.inner.x * 1000 + s[0].0.inner.x }
#[inline(never)] fn q_tn(s: &[TN]) -> u32 { let mut c = s[bb(0)]; c.0.inner.x = 99; let mut d = c; d.0.a = 98; (c.0.inner.x + d.0.a + c.0.a) * 1000 + s[0].0.inner.x + s[0].0.a }
#[inline(never)] fn q_tn_param(t: TN) -> u32 { let mut c = t; c.0.inner.x = 99; let mut d = c; d.0.a = 98; (c.0.inner.x + d.0.a + c.0.a) * 1000 + t.0.inner.x + t.0.a }
#[inline(never)] fn q_ta(s: &[TA]) -> u32 { let mut c = s[bb(0)]; c.0[bb(1)] = 99; let mut d = c; d.0[0] = 98; (suma(&c.0) + suma(&d.0)) * 1000 + suma(&s[0].0) }
#[inline(never)] fn q_ta_param(t: TA) -> u32 { let mut c = t; c.0[bb(1)] = 99; let mut d = c; d.0[0] = 98; (suma(&c.0) + suma(&d.0)) * 1000 + suma(&t.0) }
#[inline(never)] fn q_with_mu(s: &[WithMu]) -> u32 { let mut c = s[bb(0)]; unsafe { c.m.assume_init_mut().x = 99; } c.k = 98; unsafe { (c.m.assume_init().x + c.k) * 1000 + s[0].m.assume_init().x + s[0].k } }
#[inline(never)] fn q_with_mu_uninit() -> u32 { let a = WithMu { m: MaybeUninit::uninit(), k: 5 }; let mut c = a; c.m.write(P { x: 9, y: 8 }); c.k = 7; unsafe { c.m.assume_init().x * 100 + c.k * 10 + a.k } }
#[inline(never)] fn q_with_ref(s: &[P]) -> u32 { let a = WithRef { r: &s[bb(0)], p: s[bb(1)] }; let mut c = a; c.p.x = 99; c.r = &s[bb(2)]; (c.p.x + c.r.x) * 1000 + a.p.x * 10 + a.r.x }
#[inline(never)] fn q_opt_arr(s: &[Option<[u32; 3]>]) -> u32 { let mut c = s[bb(0)]; if let Some(a) = &mut c { a[bb(1)] = 99; } c.unwrap()[1] * 1000 + s[0].unwrap()[1] }
#[inline(never)] fn q_opt_opt(s: &[Option<Option<P>>]) -> u32 { let mut c = s[bb(0)]; if let Some(Some(p)) = &mut c { p.x = 99; } c.unwrap().unwrap().x * 1000 + s[0].unwrap().unwrap().x }
#[inline(never)] fn q_result(s: &[Result<P, [u32; 2]>]) -> u32 { let mut c = s[bb(0)]; let mut d = s[bb(1)]; if let Ok(p) = &mut c { p.x = 99; } if let Err(a) = &mut d { a[1] = 98; } (rs(&c) + rs(&d)) * 1000 + rs(&s[0]) + rs(&s[1]) }
#[inline(never)] fn q_tuple_nest(s: &[((P, u32), [P; 2])]) -> u32 { let mut c = s[bb(0)]; c.0.0.x = 99; c.1[bb(1)].y = 98; c.0.1 = 97; (c.0.0.x + c.1[1].y + c.0.1) * 1000 + s[0].0.0.x + s[0].1[1].y + s[0].0.1 }
#[inline(never)] fn q_arr3(s: &[[[u32; 2]; 2]; 2]) -> u32 { let mut c = *s; c[bb(1)][bb(0)][1] = 99; let mut d = c[1]; d[0][0] = 98; let mut e = d[0]; e[1] = 97; (c[1][0][1] + d[0][0] + e[1] + d[0][1] + c[1][0][0]) * 1000 + s[1][0][1] + s[1][0][0] }
#[inline(never)] fn q_const_loop(n: u32) -> u32 { let mut t = 0; for i in 0..n { let mut c = KP; c.x += i; t = t * 100 + c.x; let mut a = KA; a[bb(1)] += i; t += a[1]; let mut k = KN; k.inner.y += i; t += k.inner.y; } t }
#[inline(never)] fn q_lit_loop(n: u32) -> u32 { let mut t = 0; for i in 0..n { let mut c = P { x: 1, y: 2 }; c.x += i; t = t * 100 + c.x; let mut a = [5u32, 6, 7]; a[bb(1)] += i; t += a[1]; let mut o = Some(P { x: 3, y: 4 }); if let Some(p) = &mut o { p.y += i; t += p.y; } } t }
#[inline(never)] fn q_static_loop(n: u32) -> u32 { let mut t = 0; for i in 0..n { let mut c = PS[1]; c.x += i; t = t * 100 + c.x; let mut a = AA[1]; a[bb(1)] += i; t += a[1]; let mut w = WS[1]; w.0 += i; t += w.0; } t * 10 + PS[1].x + AA[1][1] + WS[1].0 }
#[inline(never)] fn q_closure_env(s: &[P]) -> u32 { let p = s[bb(1)]; let n = NS[bb(1)]; let mut f = move || { let mut q = p; q.x += 1; let mut m = n; m.inner.y += 1; q.x + m.inner.y }; let mut g = f; (f() + g()) * 1000 + p.x + n.inner.y + s[1].x }
#[inline(never)] fn q_closure_env_mut(s: &[P]) -> u32 { let mut p = s[bb(1)]; let mut arr = AA[bb(1)]; let mut f = move || { p.x += 1; arr[1] += 1; p.x + arr[1] }; let a = f(); let mut g = f; let b = g(); let c = f(); (a * 10000 + b * 100 + c) * 100 + p.x + arr[1] }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let bigs = bb([Big::V8(P { x: 1, y: 2 }), Big::V9([3, 4]), Big::V1(5)]);
        let bigt = bb([(Big::V8(P { x: 1, y: 2 }), 6u32)]);
        let eights = bb([Eight::V6(P { x: 1, y: 2 }), Eight::V7([3, 4]), Eight::V1(5)]);
        let n0 = N { a: 1, inner: P { x: 2, y: 3 } };
        let ens = bb([EN::A(n0), EN::B([P { x: 1, y: 2 }, P { x: 3, y: 4 }]), EN::C((P { x: 5, y: 6 }, 7)), EN::D(Some(P { x: 8, y: 9 }))]);
        let deeps = bb([Deep { t: (P { x: 1, y: 2 }, [n0, n0]), o: Some([4, 5, 6]), e: E::B(7, 8), w: W(9), hw: HW { w: W(10), pad: 11 } }]);
        let wn = bb([WrapN(n0)]);
        let tn = bb([TN(n0)]);
        let ta = bb([TA([1, 2, 3])]);
        let wm = bb([WithMu { m: MaybeUninit::new(P { x: 1, y: 2 }), k: 3 }]);
        let oa = bb([Some([1u32, 2, 3])]);
        let oo = bb([Some(Some(P { x: 1, y: 2 }))]);
        let rs: [Result<P, [u32; 2]>; 2] = bb([Ok(P { x: 1, y: 2 }), Err([3, 4])]);
        let tns = bb([((P { x: 1, y: 2 }, 3u32), [P { x: 4, y: 5 }, P { x: 6, y: 7 }])]);
        let a3 = bb([[[1u32, 2], [3, 4]], [[5, 6], [7, 8]]]);
        t!("q_big_p", q_big_p(&bigs));
        t!("q_big_a", q_big_a(&bigs));
        t!("q_big_dead", q_big_dead(&bigs));
        t!("q_big_param", q_big_param(bigs[0]));
        t!("q_big_scalar", q_big_scalar(&bigs));
        t!("q_big_in_struct", q_big_in_struct(&bigt));
        t!("q_big_arr", q_big_arr(&bigs));
        t!("q_eight_p", q_eight_p(&eights));
        t!("q_eight_a", q_eight_a(&eights));
        t!("q_en_a", q_en_a(&ens));
        t!("q_en_b", q_en_b(&ens));
        t!("q_en_c", q_en_c(&ens));
        t!("q_en_d", q_en_d(&ens));
        t!("q_en_dead", q_en_dead(&ens));
        t!("q_deep", q_deep(&deeps));
        t!("q_deep_dead", q_deep_dead(&deeps));
        t!("q_deep_param", q_deep_param(deeps[0]));
        t!("q_deep_sub", q_deep_sub(&deeps));
        t!("q_wrapn", q_wrapn(&wn));
        t!("q_tn", q_tn(&tn));
        t!("q_tn_param", q_tn_param(tn[0]));
        t!("q_ta", q_ta(&ta));
        t!("q_ta_param", q_ta_param(ta[0]));
        t!("q_with_mu", q_with_mu(&wm));
        t!("q_with_mu_uninit", q_with_mu_uninit());
        t!("q_with_ref", q_with_ref(s));
        t!("q_opt_arr", q_opt_arr(&oa));
        t!("q_opt_opt", q_opt_opt(&oo));
        t!("q_result", q_result(&rs));
        t!("q_tuple_nest", q_tuple_nest(&tns));
        t!("q_arr3", q_arr3(&a3));
        t!("q_const_loop", q_const_loop(bb(3)));
        t!("q_lit_loop", q_lit_loop(bb(3)));
        t!("q_static_loop", q_static_loop(bb(3)));
        t!("q_closure_env", q_closure_env(s));
        t!("q_closure_env_mut", q_closure_env_mut(s));
        t!("after", sum(s) + big(&bigs[0]) + big(&bigs[1]) + eight(&eights[0]) + en(&ens[1]) + deep(&deeps[0]) + tn[0].0.inner.x + suma(&ta[0].0) + a3[1][0][1]);
    }
}
