// F1: by-value arguments to a callee that writes its parameter.
#[inline(never)] fn w_never(mut p: P) -> u32 { p.x = 99; p.x + p.y }
#[inline(always)] fn w_always(mut p: P) -> u32 { p.x = 99; p.x + p.y }
#[inline(never)] fn w_ret(mut p: P) -> P { p.y = 77; p }
#[inline(always)] fn w_ret_a(mut p: P) -> P { p.y = 77; p }
#[inline(never)] fn w_arr(mut a: [u32; 3]) -> u32 { a[1] = 99; a[0] + a[1] }
#[inline(always)] fn w_arr_a(mut a: [u32; 3]) -> u32 { a[bb(1)] = 99; a[0] + a[1] }
#[inline(never)] fn w_enum(mut e: E) -> u32 { if let E::B(ref mut a, _) = e { *a = 99; } obs(&e) }
#[inline(always)] fn w_enum_a(mut e: E) -> u32 { if let E::B(ref mut a, _) = e { *a = 99; } obs(&e) }
#[inline(never)] fn w_w(mut w: W) -> u32 { w.0 = 99; w.0 }
#[inline(always)] fn w_w_a(mut w: W) -> u32 { w.0 = 99; w.0 }
#[inline(never)] fn w_n(mut n: N) -> u32 { n.inner.x = 99; n.a + n.inner.x }
#[inline(always)] fn w_n_a(mut n: N) -> u32 { n.inner.x = 99; n.a + n.inner.x }
#[inline(never)] fn w_tuple(mut t: (P, u32)) -> u32 { t.0.x = 99; t.1 = 98; t.0.y + t.1 }
#[inline(always)] fn w_tuple_a(mut t: (P, u32)) -> u32 { t.0.x = 99; t.1 = 98; t.0.y + t.1 }
#[inline(never)] fn w_pat((mut p, k): (P, u32)) -> u32 { p.x = 99; p.x + k }
#[inline(never)] fn w_two(mut a: P, mut b: P) -> u32 { a.x = 99; b.y = 98; a.x + b.y + a.y + b.x }
#[inline(always)] fn w_two_a(mut a: P, mut b: P) -> u32 { a.x = 99; b.y = 98; a.x + b.y + a.y + b.x }
impl P {
    #[inline(never)] fn bump(mut self) -> P { self.x += 100; self }
    #[inline(always)] fn bump_a(mut self) -> P { self.x += 100; self }
}
trait Tr { fn go(self) -> u32; }
impl Tr for P { #[inline(never)] fn go(mut self) -> u32 { self.x = 99; self.x + self.y } }
#[inline(never)] fn gen<T: Tr + Copy>(v: T) -> u32 { v.go() + v.go() }

#[inline(never)] fn c_slice_never(s: &[P]) -> u32 { let r = w_never(s[bb(1)]); r * 1000 + s[1].x }
#[inline(never)] fn c_slice_always(s: &[P]) -> u32 { let r = w_always(s[bb(1)]); r * 1000 + s[1].x }
#[inline(never)] fn c_static_never() -> u32 { let r = w_never(PS[bb(1)]); r * 1000 + PS[1].x }
#[inline(never)] fn c_static_always() -> u32 { let r = w_always(PS[bb(1)]); r * 1000 + PS[1].x }
#[inline(never)] fn c_static_const_idx() -> u32 { let r = w_always(PS[2]); r * 1000 + PS[2].x }
#[inline(never)] fn c_const() -> u32 { w_never(KP) * 1000 + KP.x }
#[inline(never)] fn c_const_always() -> u32 { w_always(KP) * 1000 + KP.x }
#[inline(never)] fn c_const_n() -> u32 { w_n_a(KN) * 1000 + KN.inner.x }
#[inline(never)] fn c_local() -> u32 { let a = bb(P { x: 41, y: 42 }); let r = w_never(a); r * 1000 + a.x }
#[inline(never)] fn c_local_always() -> u32 { let a = bb(P { x: 41, y: 42 }); let r = w_always(a); r * 1000 + a.x }
#[inline(never)] fn c_param_fwd(p: P) -> u32 { let r = w_never(p); r * 1000 + p.x }
#[inline(never)] fn c_param_fwd_always(p: P) -> u32 { let r = w_always(p); r * 1000 + p.x }
#[inline(never)] fn c_param_fwd_twice(p: P) -> u32 { let r = w_always(p); let q = w_never(p); r * 1000 + q + p.x }
#[inline(never)] fn c_field(n: &N) -> u32 { w_never(n.inner) * 1000 + n.inner.x }
#[inline(never)] fn c_field_always(n: &N) -> u32 { w_always(n.inner) * 1000 + n.inner.x }
#[inline(never)] fn c_field_local() -> u32 { let n = bb(N { a: 1, inner: P { x: 2, y: 3 } }); w_always(n.inner) * 1000 + n.inner.x }
#[inline(never)] fn c_field_param(n: N) -> u32 { w_always(n.inner) * 1000 + n.inner.x + w_never(n.inner) }
#[inline(never)] fn c_ret(s: &[P]) -> u32 { let q = w_ret(s[bb(0)]); q.y * 1000 + s[0].y }
#[inline(never)] fn c_ret_always(s: &[P]) -> u32 { let q = w_ret_a(s[bb(0)]); q.y * 1000 + s[0].y }
#[inline(never)] fn c_ret_mut(s: &[P]) -> u32 { let mut q = w_ret(s[bb(0)]); q.x = 55; q.x * 1000 + s[0].x }
#[inline(never)] fn c_ret_mut_always(s: &[P]) -> u32 { let mut q = w_ret_a(s[bb(0)]); q.x = 55; q.x * 1000 + s[0].x + s[0].y }
#[inline(never)] fn c_arr(a: &[[u32; 3]]) -> u32 { w_arr(a[bb(1)]) * 1000 + a[1][1] }
#[inline(never)] fn c_arr_always(a: &[[u32; 3]]) -> u32 { w_arr_a(a[bb(1)]) * 1000 + a[1][1] }
#[inline(never)] fn c_arr_static() -> u32 { w_arr(AA[bb(0)]) * 1000 + AA[0][1] }
#[inline(never)] fn c_arr_static_always() -> u32 { w_arr_a(AA[bb(0)]) * 1000 + AA[0][1] }
#[inline(never)] fn c_arr_const() -> u32 { w_arr_a(KA) * 1000 + KA[1] }
#[inline(never)] fn c_arr_fresh(n: u32, m: u32) -> u32 { w_arr_a([n, m, n]) * 1000 + m }
#[inline(never)] fn c_enum(s: &[E]) -> u32 { w_enum(s[bb(1)]) * 1000 + obs(&s[1]) }
#[inline(never)] fn c_enum_always(s: &[E]) -> u32 { w_enum_a(s[bb(1)]) * 1000 + obs(&s[1]) }
#[inline(never)] fn c_enum_fresh(n: u32, m: u32) -> u32 { w_enum_a(E::B(n, m)) * 1000 + n }
#[inline(never)] fn c_w(s: &[W]) -> u32 { w_w(s[bb(1)]) * 1000 + s[1].0 }
#[inline(never)] fn c_w_always(s: &[W]) -> u32 { w_w_a(s[bb(1)]) * 1000 + s[1].0 }
#[inline(never)] fn c_w_param(w: W) -> u32 { w_w(w) * 1000 + w.0 }
#[inline(never)] fn c_w_param_always(w: W) -> u32 { w_w_a(w) * 1000 + w.0 }
#[inline(never)] fn c_w_fresh(n: u32) -> u32 { w_w_a(W(n)) * 1000 + n }
#[inline(never)] fn c_n(s: &[N]) -> u32 { w_n(s[bb(1)]) * 1000 + s[1].inner.x }
#[inline(never)] fn c_n_always(s: &[N]) -> u32 { w_n_a(s[bb(1)]) * 1000 + s[1].inner.x }
#[inline(never)] fn c_n_fresh(p: P, k: u32) -> u32 { w_n_a(N { a: k, inner: p }) * 1000 + p.x }
#[inline(never)] fn c_n_fresh_slice(s: &[P], k: u32) -> u32 { w_n_a(N { a: k, inner: s[bb(2)] }) * 1000 + s[2].x }
#[inline(never)] fn c_tuple(s: &[P]) -> u32 { let k = bb(5); w_tuple((s[bb(2)], k)) * 1000 + s[2].x + k }
#[inline(never)] fn c_tuple_always(s: &[P]) -> u32 { let k = bb(5); w_tuple_a((s[bb(2)], k)) * 1000 + s[2].x + k }
#[inline(never)] fn c_tuple_param(p: P, k: u32) -> u32 { w_tuple_a((p, k)) * 1000 + p.x + k }
#[inline(never)] fn c_pat(s: &[P]) -> u32 { w_pat((s[bb(2)], 5)) * 1000 + s[2].x }
#[inline(never)] fn c_two(s: &[P]) -> u32 { w_two(s[bb(1)], s[bb(1)]) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn c_two_always(s: &[P]) -> u32 { w_two_a(s[bb(1)], s[bb(1)]) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn c_two_param(p: P) -> u32 { w_two_a(p, p) * 1000 + p.x + p.y }
#[inline(never)] fn c_fnptr(s: &[P]) -> u32 { let f: fn(P) -> u32 = bb(w_never); f(s[bb(1)]) * 1000 + s[1].x }
#[inline(never)] fn c_closure_arg(s: &[P]) -> u32 { let f = |mut p: P| { p.x = 99; p.x + p.y }; f(s[bb(1)]) * 1000 + s[1].x }
#[inline(never)] fn c_closure_arg_param(p: P) -> u32 { let f = |mut q: P| { q.x = 99; q.x + q.y }; f(p) * 1000 + p.x }
#[inline(never)] fn c_dyn(s: &[P]) -> u32 { let f: &dyn Fn(P) -> u32 = &|mut p: P| { p.x = 99; p.x + p.y }; f(s[bb(1)]) * 1000 + s[1].x }
#[inline(never)] fn c_twice(s: &[P]) -> u32 { let a = w_never(s[bb(1)]); let b = w_always(s[bb(1)]); a * 1000 + b + s[1].x }
#[inline(never)] fn c_method(s: &[P]) -> u32 { s[bb(1)].bump().bump().x * 1000 + s[1].x }
#[inline(never)] fn c_method_always(s: &[P]) -> u32 { s[bb(1)].bump_a().bump_a().x * 1000 + s[1].x }
#[inline(never)] fn c_method_param(p: P) -> u32 { p.bump_a().bump().x * 1000 + p.x }
#[inline(never)] fn c_trait(s: &[P]) -> u32 { s[bb(1)].go() * 1000 + s[1].x }
#[inline(never)] fn c_generic(s: &[P]) -> u32 { gen(s[bb(1)]) * 1000 + s[1].x }
#[inline(never)] fn c_loop(s: &[P]) -> u32 { let mut t = 0; for i in 0..s.len() { t += w_always(s[i]); t += w_never(s[i]); } t * 1000 + sum(s) }
#[inline(never)] fn c_loop_param(p: P, n: u32) -> u32 { let mut t = 0; for _ in 0..n { t += w_always(p); } t * 1000 + p.x }
#[inline(never)] fn c_cond(s: &[P], c: bool) -> u32 { let r = if c { w_always(s[0]) } else { w_never(s[2]) }; r * 1000 + sum(s) }
#[inline(never)] fn c_option_arg(o: &[Option<P>]) -> u32 {
    #[inline(always)] fn w(mut o: Option<P>) -> u32 { if let Some(p) = &mut o { p.x = 99; } o.map_or(0, |p| p.x) }
    w(o[bb(1)]) * 1000 + o[1].map_or(0, |p| p.x)
}
#[inline(never)] fn c_box_arg(s: &[P]) -> u32 {
    #[inline(always)] fn w(mut b: Box<P>) -> u32 { b.x = 99; b.x + b.y }
    w(Box::new(s[bb(1)])) * 1000 + s[1].x
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let mut ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let es = bb([E::A(1), E::B(2, 3), E::C(P { x: 4, y: 5 })]);
        let ws = bb([W(10), W(11), W(12)]);
        let ns = bb([N { a: 1, inner: P { x: 2, y: 3 } }, N { a: 4, inner: P { x: 5, y: 6 } }]);
        let op = bb([None, Some(P { x: 31, y: 32 })]);
        t!("c_slice_never", c_slice_never(s));
        t!("c_slice_always", c_slice_always(s));
        t!("c_static_never", c_static_never());
        t!("c_static_always", c_static_always());
        t!("c_static_const_idx", c_static_const_idx());
        t!("c_const", c_const());
        t!("c_const_always", c_const_always());
        t!("c_const_n", c_const_n());
        t!("c_local", c_local());
        t!("c_local_always", c_local_always());
        t!("c_param_fwd", c_param_fwd(bb(P { x: 7, y: 8 })));
        t!("c_param_fwd_always", c_param_fwd_always(bb(P { x: 7, y: 8 })));
        t!("c_param_fwd_twice", c_param_fwd_twice(bb(P { x: 7, y: 8 })));
        t!("c_field", c_field(&ns[1]));
        t!("c_field_always", c_field_always(&ns[1]));
        t!("c_field_local", c_field_local());
        t!("c_field_param", c_field_param(ns[1]));
        t!("c_ret", c_ret(s));
        t!("c_ret_always", c_ret_always(s));
        t!("c_ret_mut", c_ret_mut(s));
        t!("c_ret_mut_always", c_ret_mut_always(s));
        t!("c_arr", c_arr(&aa));
        t!("c_arr_always", c_arr_always(&aa));
        t!("c_arr_static", c_arr_static());
        t!("c_arr_static_always", c_arr_static_always());
        t!("c_arr_const", c_arr_const());
        t!("c_arr_fresh", c_arr_fresh(bb(3), bb(4)));
        t!("c_enum", c_enum(&es));
        t!("c_enum_always", c_enum_always(&es));
        t!("c_enum_fresh", c_enum_fresh(bb(3), bb(4)));
        t!("c_w", c_w(&ws));
        t!("c_w_always", c_w_always(&ws));
        t!("c_w_param", c_w_param(bb(W(5))));
        t!("c_w_param_always", c_w_param_always(bb(W(5))));
        t!("c_w_fresh", c_w_fresh(bb(6)));
        t!("c_n", c_n(&ns));
        t!("c_n_always", c_n_always(&ns));
        t!("c_n_fresh", c_n_fresh(bb(P { x: 7, y: 8 }), bb(2)));
        t!("c_n_fresh_slice", c_n_fresh_slice(s, bb(2)));
        t!("c_tuple", c_tuple(s));
        t!("c_tuple_always", c_tuple_always(s));
        t!("c_tuple_param", c_tuple_param(bb(P { x: 7, y: 8 }), bb(2)));
        t!("c_pat", c_pat(s));
        t!("c_two", c_two(s));
        t!("c_two_always", c_two_always(s));
        t!("c_two_param", c_two_param(bb(P { x: 7, y: 8 })));
        t!("c_fnptr", c_fnptr(s));
        t!("c_closure_arg", c_closure_arg(s));
        t!("c_closure_arg_param", c_closure_arg_param(bb(P { x: 7, y: 8 })));
        t!("c_dyn", c_dyn(s));
        t!("c_twice", c_twice(s));
        t!("c_method", c_method(s));
        t!("c_method_always", c_method_always(s));
        t!("c_method_param", c_method_param(bb(P { x: 7, y: 8 })));
        t!("c_trait", c_trait(s));
        t!("c_generic", c_generic(s));
        t!("c_loop", c_loop(s));
        t!("c_loop_param", c_loop_param(bb(P { x: 7, y: 8 }), bb(3)));
        t!("c_cond", c_cond(s, bb(true)));
        t!("c_cond2", c_cond(s, bb(false)));
        t!("c_option_arg", c_option_arg(&op));
        t!("c_box_arg", c_box_arg(s));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + obs(&es[1]) + ws[1].0 + ns[1].inner.x);
    }
}
