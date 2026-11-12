// Values MOVED out of fresh aggregates (no clone: the read of the field is the value), then written
// or viewed. The fresh aggregate is built from scalars read out of a slice / static / parameters.
struct PC { x: u32, y: u32 }
struct NC { a: u32, inner: PC }
struct NN { n: NC, k: u32 }
struct HW2 { w: W, pad: u32 }
struct AC { a: [u32; 3], k: u32 }
enum EC { A(u32), C(PC), D(NC) }
#[inline(never)] fn wr_pc(p: &mut PC) -> u32 { p.x = 99; p.x + p.y }
#[inline(always)] fn wr_pc_a(p: &mut PC) -> u32 { p.x = 99; p.x + p.y }
#[inline(never)] fn by_val(mut p: PC) -> u32 { p.x = 99; p.x + p.y }
#[inline(always)] fn by_val_a(mut p: PC) -> u32 { p.x = 99; p.x + p.y }
#[inline(never)] fn nc_inner(n: NC) -> u32 { let mut inner = n.inner; inner.x = 99; let r = &mut inner.y; *r = 98; inner.x + inner.y }
#[inline(always)] fn nc_inner_a(n: NC) -> u32 { let mut inner = n.inner; inner.x = 99; let r = &mut inner.y; *r = 98; inner.x + inner.y }
#[inline(never)] fn first<T>(t: (T, u32)) -> T { t.0 }
#[inline(always)] fn first_a<T>(t: (T, u32)) -> T { t.0 }
fn pc(s: &[P], i: usize) -> PC { PC { x: s[i].x, y: s[i].y } }

#[inline(never)] fn mv_field_view(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; let r = &mut inner.x; *r = 99; *r * 1000 + s[1].x }
#[inline(never)] fn mv_field_view_dead(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; let r = &mut inner.x; *r = 99; s[1].x }
#[inline(never)] fn mv_field_set(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; inner.x = 99; inner.x * 1000 + s[1].x }
#[inline(never)] fn mv_field_dead(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; inner.x = 99; s[1].x }
#[inline(never)] fn mv_field_callee(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; wr_pc(&mut inner) * 1000 + s[1].x }
#[inline(never)] fn mv_field_callee_a(s: &[P], k: u32) -> u32 { let mut inner = NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner; wr_pc_a(&mut inner) * 1000 + s[1].x }
#[inline(never)] fn mv_field_tmp_callee(s: &[P], k: u32) -> u32 { wr_pc_a(&mut NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner) * 1000 + s[1].x }
#[inline(never)] fn mv_field_tmp_x(s: &[P], k: u32) -> u32 { let r = &mut NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner.x; *r = 99; *r * 1000 + s[1].x }
#[inline(never)] fn mv_field_byval(s: &[P], k: u32) -> u32 { by_val(NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner) * 1000 + s[1].x }
#[inline(never)] fn mv_field_byval_a(s: &[P], k: u32) -> u32 { by_val_a(NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }.inner) * 1000 + s[1].x }
#[inline(never)] fn mv_field_params(n: u32, m: u32) -> u32 { let mut inner = NC { a: 1, inner: PC { x: n, y: m } }.inner; let r = &mut inner.x; *r = 99; *r * 10000 + n * 100 + m }
#[inline(never)] fn mv_field_params_dead(n: u32, m: u32) -> u32 { let mut inner = NC { a: 1, inner: PC { x: n, y: m } }.inner; inner.x = 99; let r = &mut inner.y; *r = 98; n * 100 + m }
#[inline(never)] fn mv_field_static() -> u32 { let mut inner = NC { a: 1, inner: PC { x: PS[bb(1)].x, y: PS[1].y } }.inner; let r = &mut inner.x; *r = 99; *r * 1000 + PS[1].x }
#[inline(never)] fn mv_field2(s: &[P], k: u32) -> u32 { let mut x = NN { n: NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }, k }.n.inner; x.y = 98; let r = &mut x.x; *r = 99; (x.x + x.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_field2_n(s: &[P], k: u32) -> u32 { let mut n = NN { n: NC { a: k, inner: PC { x: s[bb(1)].x, y: s[1].y } }, k }.n; n.inner.y = 98; let r = &mut n.inner.x; *r = 99; n.a = 97; (n.inner.x + n.inner.y + n.a) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_tuple(s: &[P]) -> u32 { let (mut a, b) = (pc(s, 0), pc(s, 1)); a.x = 99; let r = &mut a.y; *r = 98; (a.x + a.y + b.x) * 1000 + s[0].x * 10 + s[0].y }
#[inline(never)] fn mv_tuple_lit(s: &[P]) -> u32 { let (mut a, b) = (PC { x: s[bb(0)].x, y: s[0].y }, PC { x: s[1].x, y: s[1].y }); a.x = 99; let r = &mut a.y; *r = 98; (a.x + a.y + b.x) * 1000 + s[0].x * 10 + s[0].y }
#[inline(never)] fn mv_tuple_field(s: &[P]) -> u32 { let mut a = (PC { x: s[bb(0)].x, y: s[0].y }, 5u32).0; let r = &mut a.y; *r = 98; a.x = 99; (a.x + a.y) * 1000 + s[0].x * 10 + s[0].y }
#[inline(never)] fn mv_option(s: &[P]) -> u32 { let o = Some(PC { x: s[bb(1)].x, y: s[1].y }); if let Some(mut p) = o { p.x = 99; let r = &mut p.y; *r = 98; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y } else { 0 } }
#[inline(never)] fn mv_option_unwrap(s: &[P]) -> u32 { let mut p = Some(PC { x: s[bb(1)].x, y: s[1].y }).unwrap(); let r = &mut p.y; *r = 98; p.x = 99; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_enum(s: &[P]) -> u32 { let e = EC::C(PC { x: s[bb(1)].x, y: s[1].y }); match e { EC::C(mut p) => { p.x = 99; let r = &mut p.y; *r = 98; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y } _ => 0 } }
#[inline(never)] fn mv_enum_nested(s: &[P]) -> u32 { let e = EC::D(NC { a: 1, inner: PC { x: s[bb(1)].x, y: s[1].y } }); match e { EC::D(n) => { let mut p = n.inner; let r = &mut p.x; *r = 99; *r * 1000 + s[1].x } _ => 0 } }
#[inline(never)] fn mv_array(s: &[P]) -> u32 { let [mut a, b] = [PC { x: s[bb(0)].x, y: s[0].y }, PC { x: s[1].x, y: s[1].y }]; a.x = 99; let r = &mut a.y; *r = 98; (a.x + a.y + b.x) * 1000 + s[0].x * 10 + s[0].y }
#[inline(never)] fn mv_box(s: &[P]) -> u32 { let b = Box::new(NC { a: 1, inner: PC { x: s[bb(1)].x, y: s[1].y } }); let mut inner = b.inner; inner.x = 99; let r = &mut inner.y; *r = 98; (inner.x + inner.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_param(s: &[P]) -> u32 { nc_inner(NC { a: 1, inner: PC { x: s[bb(1)].x, y: s[1].y } }) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_param_a(s: &[P]) -> u32 { nc_inner_a(NC { a: 1, inner: PC { x: s[bb(1)].x, y: s[1].y } }) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_param_own(n: NC, s: &[P]) -> u32 { let mut inner = n.inner; let r = &mut inner.x; *r = 99; *r * 1000 + n.a + s[1].x }
#[inline(never)] fn mv_closure(s: &[P]) -> u32 { let p = PC { x: s[bb(1)].x, y: s[1].y }; let f = move || { let mut q = p; q.x = 99; let r = &mut q.y; *r = 98; q.x + q.y }; f() * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_generic(s: &[P]) -> u32 { let mut p = first((PC { x: s[bb(1)].x, y: s[1].y }, 1)); p.x = 99; let r = &mut p.y; *r = 98; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_generic_a(s: &[P]) -> u32 { let mut p = first_a((PC { x: s[bb(1)].x, y: s[1].y }, 1)); p.x = 99; let r = &mut p.y; *r = 98; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_generic_copy(s: &[P]) -> u32 { let mut p = first_a((s[bb(1)], 1)); p.x = 99; let r = &mut p.y; *r = 98; (p.x + p.y) * 1000 + s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_generic_copy_dead(s: &[P]) -> u32 { let mut p = first_a((s[bb(1)], 1)); let r = &mut p.y; *r = 98; s[1].x * 10 + s[1].y }
#[inline(never)] fn mv_w(ws: &[W]) -> u32 { let mut w = HW2 { w: ws[bb(1)], pad: 1 }.w; w.0 = 99; w.0 * 1000 + ws[1].0 }
#[inline(never)] fn mv_w_dead(ws: &[W]) -> u32 { let mut w = HW2 { w: ws[bb(1)], pad: 1 }.w; w.0 = 99; ws[1].0 }
#[inline(never)] fn mv_w_ref(ws: &[W]) -> u32 { let h = HW2 { w: ws[bb(1)], pad: 1 }; let mut w = h.w; let r = &mut w; r.0 = 99; r.0 * 1000 + ws[1].0 }
#[inline(never)] fn mv_w_ref_field(ws: &[W]) -> u32 { let mut h = HW2 { w: ws[bb(1)], pad: 1 }; let r = &mut h.w; r.0 = 99; r.0 * 1000 + ws[1].0 }
#[inline(never)] fn mv_w_ref_field0(ws: &[W]) -> u32 { let mut h = HW2 { w: ws[bb(1)], pad: 1 }; let r = &mut h.w.0; *r = 99; *r * 1000 + ws[1].0 }
#[inline(never)] fn mv_w_param(w: W) -> u32 { let mut h = HW2 { w, pad: 1 }; let r = &mut h.w.0; *r = 99; *r * 1000 + w.0 }
#[inline(never)] fn mv_w_param_moved(w: W) -> u32 { let mut v = HW2 { w, pad: 1 }.w; let r = &mut v.0; *r = 99; *r * 1000 + w.0 }
#[inline(never)] fn mv_arr(a: &[[u32; 3]]) -> u32 { let mut c = AC { a: a[bb(1)], k: 1 }.a; c[bb(1)] = 99; let r = &mut c[2]; *r = 98; suma(&c) * 1000 + suma(&a[1]) }
#[inline(never)] fn mv_arr_dead(a: &[[u32; 3]]) -> u32 { let mut c = AC { a: a[bb(1)], k: 1 }.a; let r = &mut c[2]; *r = 98; suma(&a[1]) }
#[inline(never)] fn mv_arr_lit(n: u32, m: u32) -> u32 { let mut c = AC { a: [n, m, n], k: 1 }.a; let r = &mut c[bb(1)]; *r = 98; c[0] = 99; suma(&c) * 10000 + n * 100 + m }
#[inline(never)] fn mv_arr_lit_dead(n: u32, m: u32) -> u32 { let mut c = AC { a: [n, m, n], k: 1 }.a; let r = &mut c[1]; *r = 98; n * 100 + m }
#[inline(never)] fn mv_ret(s: &[P]) -> u32 {
    #[inline(always)] fn mk(s: &[P]) -> NC { NC { a: 1, inner: PC { x: s[bb(1)].x, y: s[1].y } } }
    let mut inner = mk(s).inner; let r = &mut inner.x; *r = 99; *r * 1000 + s[1].x
}
#[inline(never)] fn mv_cell(s: &[u32]) -> u32 { struct CC { c: Cell<u32>, k: u32 } let c = CC { c: Cell::new(s[bb(1)]), k: 1 }.c; c.set(99); c.get() * 1000 + s[1] }
#[inline(never)] fn mv_cell_dead(s: &[u32]) -> u32 { struct CC { c: Cell<u32>, k: u32 } let c = CC { c: Cell::new(s[bb(1)]), k: 1 }.c; c.set(99); s[1] }
#[inline(never)] fn mv_cell_p(s: &[P]) -> u32 { struct CC { c: Cell<P>, k: u32 } let c = CC { c: Cell::new(s[bb(1)]), k: 1 }.c; unsafe { (*c.as_ptr()).x = 99 }; s[1].x }
#[inline(never)] fn mv_cell_pc(s: &[P]) -> u32 { struct CC { c: Cell<PC>, k: u32 } let c = CC { c: Cell::new(PC { x: s[bb(1)].x, y: s[1].y }), k: 1 }.c; unsafe { (*c.as_ptr()).x = 99 }; s[1].x }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let us = bb([13u32, 14, 15]);
        t!("mv_field_view", mv_field_view(s, bb(2)));
        t!("mv_field_view_dead", mv_field_view_dead(s, bb(2)));
        t!("mv_field_set", mv_field_set(s, bb(2)));
        t!("mv_field_dead", mv_field_dead(s, bb(2)));
        t!("mv_field_callee", mv_field_callee(s, bb(2)));
        t!("mv_field_callee_a", mv_field_callee_a(s, bb(2)));
        t!("mv_field_tmp_callee", mv_field_tmp_callee(s, bb(2)));
        t!("mv_field_tmp_x", mv_field_tmp_x(s, bb(2)));
        t!("mv_field_byval", mv_field_byval(s, bb(2)));
        t!("mv_field_byval_a", mv_field_byval_a(s, bb(2)));
        t!("mv_field_params", mv_field_params(bb(3), bb(4)));
        t!("mv_field_params_dead", mv_field_params_dead(bb(3), bb(4)));
        t!("mv_field_static", mv_field_static());
        t!("mv_field2", mv_field2(s, bb(2)));
        t!("mv_field2_n", mv_field2_n(s, bb(2)));
        t!("mv_tuple", mv_tuple(s));
        t!("mv_tuple_lit", mv_tuple_lit(s));
        t!("mv_tuple_field", mv_tuple_field(s));
        t!("mv_option", mv_option(s));
        t!("mv_option_unwrap", mv_option_unwrap(s));
        t!("mv_enum", mv_enum(s));
        t!("mv_enum_nested", mv_enum_nested(s));
        t!("mv_array", mv_array(s));
        t!("mv_box", mv_box(s));
        t!("mv_param", mv_param(s));
        t!("mv_param_a", mv_param_a(s));
        t!("mv_param_own", mv_param_own(NC { a: 1, inner: PC { x: ps[1].x, y: ps[1].y } }, s));
        t!("mv_closure", mv_closure(s));
        t!("mv_generic", mv_generic(s));
        t!("mv_generic_a", mv_generic_a(s));
        t!("mv_generic_copy", mv_generic_copy(s));
        t!("mv_generic_copy_dead", mv_generic_copy_dead(s));
        t!("mv_w", mv_w(&ws));
        t!("mv_w_dead", mv_w_dead(&ws));
        t!("mv_w_ref", mv_w_ref(&ws));
        t!("mv_w_ref_field", mv_w_ref_field(&ws));
        t!("mv_w_ref_field0", mv_w_ref_field0(&ws));
        t!("mv_w_param", mv_w_param(bb(W(5))));
        t!("mv_w_param_moved", mv_w_param_moved(bb(W(5))));
        t!("mv_arr", mv_arr(&aa));
        t!("mv_arr_dead", mv_arr_dead(&aa));
        t!("mv_arr_lit", mv_arr_lit(bb(3), bb(4)));
        t!("mv_arr_lit_dead", mv_arr_lit_dead(bb(3), bb(4)));
        t!("mv_ret", mv_ret(s));
        t!("mv_cell", mv_cell(&us));
        t!("mv_cell_dead", mv_cell_dead(&us));
        t!("mv_cell_p", mv_cell_p(s));
        t!("mv_cell_pc", mv_cell_pc(s));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + suma(&us));
    }
}
