// F3/F10/F11: destructors that write self, unions, views that dereference.
struct D<'a> { p: P, out: &'a Cell<u32> }
impl Drop for D<'_> { fn drop(&mut self) { self.p.x = 99; self.out.set(self.out.get() * 10 + self.p.x + self.p.y); } }
struct DW<'a>(u32, &'a Cell<u32>);
impl Drop for DW<'_> { fn drop(&mut self) { self.0 += 90; self.1.set(self.1.get() * 10 + self.0); } }
static SINK: AtomicU32 = AtomicU32::new(0);
struct DA([u32; 3]);
impl Drop for DA { fn drop(&mut self) { self.0[1] = 99; SINK.store(self.0[0] + self.0[1] + self.0[2], Ordering::Relaxed); } }
#[repr(transparent)]
struct G(u32);
impl Drop for G { fn drop(&mut self) { self.0 += 90; SINK.store(self.0, Ordering::Relaxed); } }
#[repr(transparent)]
struct GP(P);
impl Drop for GP { fn drop(&mut self) { self.0.x += 90; SINK.store(self.0.x, Ordering::Relaxed); } }
#[inline(never)] fn consume(d: D) -> u32 { d.p.y }
#[inline(always)] fn consume_a(d: D) -> u32 { d.p.y }

#[inline(never)] fn d_scope(s: &[P]) -> u32 { let out = Cell::new(0); { let _d = D { p: s[bb(1)], out: &out }; } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_explicit(s: &[P]) -> u32 { let out = Cell::new(0); drop(D { p: s[bb(1)], out: &out }); out.get() * 1000 + s[1].x }
#[inline(never)] fn d_fresh(n: u32, m: u32) -> u32 { let out = Cell::new(0); drop(D { p: P { x: n, y: m }, out: &out }); out.get() * 10000 + n * 100 + m }
#[inline(never)] fn d_param(p: P) -> u32 { let out = Cell::new(0); drop(D { p, out: &out }); out.get() * 1000 + p.x }
#[inline(never)] fn d_static() -> u32 { let out = Cell::new(0); drop(D { p: PS[bb(1)], out: &out }); out.get() * 1000 + PS[1].x }
#[inline(never)] fn d_w(n: u32) -> u32 { let out = Cell::new(0); drop(DW(n, &out)); out.get() * 1000 + n }
#[inline(never)] fn d_w_scope(n: u32) -> u32 { let out = Cell::new(0); { let _d = DW(n, &out); } out.get() * 1000 + n }
#[inline(never)] fn d_w_slice(s: &[u32]) -> u32 { let out = Cell::new(0); { let _d = DW(s[bb(1)], &out); } out.get() * 1000 + s[1] }
#[inline(never)] fn d_arr(a: &[[u32; 3]]) -> u32 { drop(DA(a[bb(1)])); SINK.load(Ordering::Relaxed) * 1000 + a[1][1] }
#[inline(never)] fn d_arr_fresh(n: u32, m: u32) -> u32 { drop(DA([n, m, n])); SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn d_g(n: u32) -> u32 { drop(G(n)); SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn d_g_scope(n: u32) -> u32 { { let _g = G(n); } SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn d_g_slice(s: &[u32]) -> u32 { { let _g = G(s[bb(1)]); } SINK.load(Ordering::Relaxed) * 1000 + s[1] }
#[inline(never)] fn d_g_pair(n: u32, m: u32) -> u32 { { let _g = (G(n), G(m)); } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn d_g_arr(n: u32, m: u32) -> u32 { { let _g = [G(n), G(m)]; } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn d_g_opt(n: u32) -> u32 { { let _g = Some(G(n)); } SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn d_gp(s: &[P]) -> u32 { { let _g = GP(s[bb(1)]); } SINK.load(Ordering::Relaxed) * 1000 + s[1].x }
#[inline(never)] fn d_gp_param(p: P) -> u32 { { let _g = GP(p); } SINK.load(Ordering::Relaxed) * 1000 + p.x }
#[inline(never)] fn d_in_option(s: &[P]) -> u32 { let out = Cell::new(0); { let _o = Some(D { p: s[bb(1)], out: &out }); } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_in_array(s: &[P]) -> u32 { let out = Cell::new(0); { let _a = [D { p: s[bb(1)], out: &out }, D { p: s[bb(2)], out: &out }]; } out.get() * 1000 + s[1].x + s[2].x }
#[inline(never)] fn d_in_tuple(s: &[P]) -> u32 { let out = Cell::new(0); { let _t = (D { p: s[bb(1)], out: &out }, 5u32); } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_moved_into_fn(s: &[P]) -> u32 { let out = Cell::new(0); let r = consume(D { p: s[bb(1)], out: &out }); (out.get() + r) * 1000 + s[1].x }
#[inline(never)] fn d_moved_into_fn_a(s: &[P]) -> u32 { let out = Cell::new(0); let r = consume_a(D { p: s[bb(1)], out: &out }); (out.get() + r) * 1000 + s[1].x }
#[inline(never)] fn d_vec(s: &[P]) -> u32 { let out = Cell::new(0); { let mut v = Vec::new(); v.push(D { p: s[bb(1)], out: &out }); } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_box(s: &[P]) -> u32 { let out = Cell::new(0); { let _b = Box::new(D { p: s[bb(1)], out: &out }); } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_replace(s: &[P]) -> u32 { let out = Cell::new(0); { let mut d = D { p: s[bb(1)], out: &out }; d = D { p: s[bb(2)], out: &out }; d.p.y = 0; } out.get() * 1000 + s[1].x + s[2].x + s[2].y }
#[inline(never)] fn d_manually(s: &[P]) -> u32 { let out = Cell::new(0); let mut m = ManuallyDrop::new(D { p: s[bb(1)], out: &out }); unsafe { ManuallyDrop::drop(&mut m) }; out.get() * 1000 + s[1].x }
#[inline(never)] fn d_cond_drop(s: &[P], c: bool) -> u32 { let out = Cell::new(0); { let d = D { p: s[bb(1)], out: &out }; if c { drop(d); out.set(out.get() + 1000); } } out.get() * 1000 + s[1].x }
#[inline(never)] fn d_in_place(s: &[P]) -> u32 { let out = Cell::new(0); let mut m = MaybeUninit::new(D { p: s[bb(1)], out: &out }); unsafe { core::ptr::drop_in_place(m.as_mut_ptr()) }; out.get() * 1000 + s[1].x }
#[inline(never)] fn d_write_then_drop(s: &[P]) -> u32 { let out = Cell::new(0); { let mut d = D { p: s[bb(1)], out: &out }; d.p.y = 50; } out.get() * 1000 + s[1].x + s[1].y }

#[derive(Clone, Copy)] union U { a: u32, b: i32 }
#[derive(Clone, Copy)] union U1 { a: u32, z: () }
#[derive(Clone, Copy)] union UP1 { p: P, z: () }
#[inline(never)] fn u_field(us: &[U]) -> u32 { let mut u = us[bb(1)]; u.a = 99; unsafe { u.a * 1000 + us[1].a } }
#[inline(never)] fn u_param(u: U) -> u32 { let mut v = u; v.a = 99; unsafe { v.a * 1000 + u.a } }
#[inline(never)] fn u_fresh(n: u32) -> u32 { let mut v = U { a: n }; v.a = 99; unsafe { v.a * 1000 + n } }
#[inline(never)] fn u1_param(u: U1) -> u32 { let mut v = u; v.a = 99; unsafe { v.a * 1000 + u.a } }
#[inline(never)] fn u1_slice(us: &[U1]) -> u32 { let mut v = us[bb(1)]; v.a = 99; unsafe { v.a * 1000 + us[1].a } }
#[inline(never)] fn u1_fresh(n: u32) -> u32 { let mut v = U1 { a: n }; v.a = 99; unsafe { v.a * 1000 + n } }
#[inline(never)] fn u1_chain(u: U1) -> u32 { let mut v = u; v.a = 98; let w = v; v.a = 97; unsafe { v.a * 1000000 + w.a * 1000 + u.a } }
#[inline(never)] fn up1_param(u: UP1) -> u32 { let mut v = u; unsafe { v.p.x = 99; v.p.x * 1000 + u.p.x } }
#[inline(never)] fn up1_slice(us: &[UP1]) -> u32 { let mut v = us[bb(0)]; unsafe { v.p.x = 99; v.p.x * 1000 + us[0].p.x } }
#[inline(never)] fn mu_scalar(n: u32) -> u32 { let mut m = MaybeUninit::new(n); let m2 = m; m.write(99); let r = unsafe { m.assume_init() * 10000 + m2.assume_init() * 100 }; r + n }
#[inline(never)] fn mu_copy_then_write(n: u32) -> u32 { let m = MaybeUninit::new(n); let mut m2 = m; m2.write(99); unsafe { m2.assume_init() * 1000 + m.assume_init() } }
#[inline(never)] fn mu_param(m: MaybeUninit<u32>) -> u32 { let mut m2 = m; m2.write(99); unsafe { m2.assume_init() * 1000 + m.assume_init() } }
#[inline(never)] fn mu_as_mut_ptr(m: MaybeUninit<u32>) -> u32 { let mut m2 = m; unsafe { *m2.as_mut_ptr() = 99; m2.assume_init() * 1000 + m.assume_init() } }
#[inline(never)] fn mu_assume_mut(m: MaybeUninit<u32>) -> u32 { let mut m2 = m; unsafe { *m2.assume_init_mut() += 1; m2.assume_init() * 1000 + m.assume_init() } }
#[inline(never)] fn mu_p(s: &[P]) -> u32 { let mut m = MaybeUninit::new(s[bb(1)]); unsafe { (*m.as_mut_ptr()).x = 99; m.assume_init_mut().y = 98; let v = m.assume_init(); (v.x + v.y) * 1000 + s[1].x + s[1].y } }
#[inline(never)] fn mu_p_copy(s: &[P]) -> u32 { let m = MaybeUninit::new(s[bb(1)]); let mut m2 = m; unsafe { m2.assume_init_mut().y = 98; (m2.assume_init().y + m.assume_init().y) * 1000 + s[1].y } }
#[inline(never)] fn mu_arr(n: u32) -> u32 { let mut a = [MaybeUninit::<u32>::uninit(); 3]; let b = a; a[0].write(n); a[bb(1)].write(n + 1); a[2] = MaybeUninit::new(7); let mut c = a; c[1].write(99); unsafe { (a[0].assume_init() + a[1].assume_init()) * 1000 + c[1].assume_init() + n } }
#[inline(never)] fn md(s: &[P]) -> u32 { let mut m = ManuallyDrop::new(s[bb(1)]); m.x = 99; m.x * 1000 + s[1].x }
#[inline(never)] fn md_copy(s: &[P]) -> u32 { let m = ManuallyDrop::new(s[bb(1)]); let mut m2 = m; m2.x = 99; (m2.x + m.x) * 1000 + s[1].x }
#[inline(never)] fn md_scalar(n: u32) -> u32 { let m = ManuallyDrop::new(n); let mut m2 = m; *m2 = 99; *m2 * 10000 + *m * 100 + n }
#[inline(never)] fn wrapping(n: u32) -> u32 { use core::num::Wrapping; let a = Wrapping(n); let mut b = a; b.0 = 99; let mut c = a; c += Wrapping(1); c.0 += 1; b.0 * 10000 + c.0 * 100 + a.0 }
#[inline(never)] fn wrapping_slice(s: &[core::num::Wrapping<u32>]) -> u32 { let mut b = s[bb(1)]; b.0 = 99; let mut c = s[bb(1)]; c += s[0]; b.0 * 10000 + c.0 * 100 + s[1].0 }

struct H<'a> { r: &'a mut P, k: u32 }
#[derive(Clone, Copy)] struct HS<'a> { r: &'a P, k: u32 }
struct HB { b: Box<P>, k: u32 }
struct HBS { b: Box<u32>, k: u32 }
#[derive(Clone, Copy)] struct RCell<'a> { c: &'a Cell<u32>, k: u32 }
#[derive(Clone, Copy)] struct HR { p: *mut P, k: u32 }
#[derive(Clone, Copy)] struct HU { p: *mut u32, k: u32 }
#[repr(transparent)] struct RW<'a>(&'a mut P);
#[repr(transparent)] #[derive(Clone, Copy)] struct RS<'a>(&'a P);
#[inline(never)] fn v_holder_copy_src(s: &[P]) -> u32 { let mut c = s[bb(1)]; let h = H { r: &mut c, k: 1 }; h.r.x = 99; c.x * 1000 + s[1].x }
#[inline(never)] fn v_holder_moved(s: &[P]) -> u32 { let mut c = s[bb(1)]; { let h = H { r: &mut c, k: 1 }; let h2 = h; h2.r.x = 99; } c.x * 1000 + s[1].x }
#[inline(never)] fn v_hs(s: &[P]) -> u32 { let h = HS { r: &s[bb(1)], k: 1 }; let mut h2 = h; h2.k = 99; let mut c = *h2.r; c.x = 98; (h.k + h2.k + c.x) * 1000 + s[1].x }
#[inline(never)] fn v_hs_reseat(s: &[P]) -> u32 { let h = HS { r: &s[bb(1)], k: 1 }; let mut h2 = h; h2.r = &s[bb(2)]; (h.r.x * 10 + h2.r.x) * 1000 + s[1].x }
#[inline(never)] fn v_rs_reseat(s: &[P]) -> u32 { let a = RS(&s[bb(1)]); let mut b = a; b.0 = &s[bb(2)]; (a.0.x * 10 + b.0.x) * 1000 + s[1].x }
#[inline(never)] fn v_rs_param(a: RS, o: &P) -> u32 { let mut b = a; b.0 = o; a.0.x * 100 + b.0.x }
#[inline(never)] fn v_box_field(s: &[P]) -> u32 { let mut h = HB { b: Box::new(s[bb(1)]), k: 1 }; h.b.x = 99; h.k = 5; (h.b.x + h.k) * 1000 + s[1].x }
#[inline(never)] fn v_box_field_fresh(n: u32, m: u32) -> u32 { let h = HB { b: Box::new(P { x: n, y: m }), k: m }; let mut h = h; h.b.y = 99; h.b.y * 10000 + n * 100 + m }
#[inline(never)] fn v_box_scalar_field(n: u32, m: u32) -> u32 { let mut h = HBS { b: Box::new(n), k: m }; *h.b = 99; h.k += 1; (*h.b + h.k) * 10000 + n * 100 + m }
#[inline(never)] fn v_box_deref_move(s: &[P]) -> u32 { let h = HB { b: Box::new(s[bb(1)]), k: 1 }; let mut p = *h.b; p.x = 99; p.x * 1000 + s[1].x }
#[inline(never)] fn v_refmut_in_tuple(s: &[P]) -> u32 { let mut a = s[bb(0)]; let t = (&mut a, 5u32); t.0.x = 99; a.x * 1000 + s[0].x }
#[inline(never)] fn v_ref_cell_field(cells: &[Cell<u32>], n: u32) -> u32 { let h = RCell { c: &cells[bb(1)], k: n }; let h2 = h; h2.c.set(99); cells[1].get() * 1000 + n + h.k }
#[inline(never)] fn v_option_ref_mut(s: &[P]) -> u32 { let mut c = s[bb(1)]; let o = Some(&mut c); if let Some(r) = o { r.x = 99; } c.x * 1000 + s[1].x }
#[inline(never)] fn v_raw_in_struct(s: &[P]) -> u32 { let mut c = s[bb(1)]; let h = HR { p: &mut c, k: 1 }; let h2 = h; unsafe { (*h2.p).x = 99 }; c.x * 1000 + s[1].x }
#[inline(never)] fn v_slot_ptr_in_struct(n: u32) -> u32 { let mut n2 = n; let h = HU { p: &mut n2, k: 0 }; let h2 = h; unsafe { *h2.p = 99 }; n2 * 1000 + n }
#[inline(never)] fn v_ref_to_w(ws: &[W]) -> u32 { let mut w = ws[bb(1)]; let h = (&mut w, 1); h.0.0 = 99; w.0 * 1000 + ws[1].0 }
#[inline(never)] fn v_nested_ref(s: &[P]) -> u32 { let mut c = s[bb(1)]; let mut r = &mut c; let rr = &mut r; rr.x = 99; c.x * 1000 + s[1].x }
#[inline(never)] fn v_rw(s: &[P]) -> u32 { let mut c = s[bb(1)]; { let rw = RW(&mut c); rw.0.x = 99; } c.x * 1000 + s[1].x }
#[inline(never)] fn v_rw_reseat(s: &[P]) -> u32 { let mut a = s[bb(0)]; let mut b = s[bb(1)]; { let mut rw = RW(&mut a); rw.0.x = 97; rw.0 = &mut b; rw.0.x = 99; } (a.x * 100 + b.x) * 1000 + s[0].x + s[1].x }
#[inline(never)] fn v_mut_ref_param(p: &mut P, s: &[P]) -> u32 { let mut c = *p; c.x = 99; p.y = 98; (c.x + c.y + p.x + p.y) * 1000 + sum(s) }
#[inline(never)] fn v_mut_ref_reborrow(p: &mut P) -> u32 { let q = &mut *p; let mut c = *q; c.x = 99; q.y = 98; let d = *p; (c.x + c.y) * 1000 + d.x + d.y }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let us = bb([13u32, 14, 15]);
        let uu = bb([U { a: 5 }, U { a: 6 }]);
        let u1 = bb([U1 { a: 5 }, U1 { a: 6 }]);
        let up1 = bb([UP1 { p: P { x: 1, y: 2 } }, UP1 { p: P { x: 3, y: 4 } }]);
        let wr = bb([core::num::Wrapping(3u32), core::num::Wrapping(4u32)]);
        let cells = [Cell::new(1u32), Cell::new(2), Cell::new(3)];
        t!("d_scope", d_scope(s));
        t!("d_explicit", d_explicit(s));
        t!("d_fresh", d_fresh(bb(3), bb(4)));
        t!("d_param", d_param(bb(P { x: 7, y: 8 })));
        t!("d_static", d_static());
        t!("d_w", d_w(bb(3)));
        t!("d_w_scope", d_w_scope(bb(3)));
        t!("d_w_slice", d_w_slice(&us));
        t!("d_arr", d_arr(&aa));
        t!("d_arr_fresh", d_arr_fresh(bb(3), bb(4)));
        t!("d_g", d_g(bb(3)));
        t!("d_g_scope", d_g_scope(bb(3)));
        t!("d_g_slice", d_g_slice(&us));
        t!("d_g_pair", d_g_pair(bb(3), bb(4)));
        t!("d_g_arr", d_g_arr(bb(3), bb(4)));
        t!("d_g_opt", d_g_opt(bb(3)));
        t!("d_gp", d_gp(s));
        t!("d_gp_param", d_gp_param(bb(P { x: 7, y: 8 })));
        t!("d_in_option", d_in_option(s));
        t!("d_in_array", d_in_array(s));
        t!("d_in_tuple", d_in_tuple(s));
        t!("d_moved_into_fn", d_moved_into_fn(s));
        t!("d_moved_into_fn_a", d_moved_into_fn_a(s));
        t!("d_vec", d_vec(s));
        t!("d_box", d_box(s));
        t!("d_replace", d_replace(s));
        t!("d_manually", d_manually(s));
        t!("d_cond_drop", d_cond_drop(s, bb(true)));
        t!("d_cond_drop2", d_cond_drop(s, bb(false)));
        t!("d_in_place", d_in_place(s));
        t!("d_write_then_drop", d_write_then_drop(s));
        t!("u_field", u_field(&uu));
        t!("u_param", u_param(bb(U { a: 7 })));
        t!("u_fresh", u_fresh(bb(7)));
        t!("u1_param", u1_param(bb(U1 { a: 7 })));
        t!("u1_slice", u1_slice(&u1));
        t!("u1_fresh", u1_fresh(bb(7)));
        t!("u1_chain", u1_chain(bb(U1 { a: 7 })));
        t!("up1_param", up1_param(bb(UP1 { p: P { x: 7, y: 8 } })));
        t!("up1_slice", up1_slice(&up1));
        t!("mu_scalar", mu_scalar(bb(7)));
        t!("mu_copy_then_write", mu_copy_then_write(bb(7)));
        t!("mu_param", mu_param(bb(MaybeUninit::new(7))));
        t!("mu_as_mut_ptr", mu_as_mut_ptr(bb(MaybeUninit::new(7))));
        t!("mu_assume_mut", mu_assume_mut(bb(MaybeUninit::new(7))));
        t!("mu_p", mu_p(s));
        t!("mu_p_copy", mu_p_copy(s));
        t!("mu_arr", mu_arr(bb(7)));
        t!("md", md(s));
        t!("md_copy", md_copy(s));
        t!("md_scalar", md_scalar(bb(7)));
        t!("wrapping", wrapping(bb(7)));
        t!("wrapping_slice", wrapping_slice(&wr));
        t!("v_holder_copy_src", v_holder_copy_src(s));
        t!("v_holder_moved", v_holder_moved(s));
        t!("v_hs", v_hs(s));
        t!("v_hs_reseat", v_hs_reseat(s));
        t!("v_rs_reseat", v_rs_reseat(s));
        t!("v_rs_param", v_rs_param(RS(&ps[1]), &ps[2]));
        t!("v_box_field", v_box_field(s));
        t!("v_box_field_fresh", v_box_field_fresh(bb(3), bb(4)));
        t!("v_box_scalar_field", v_box_scalar_field(bb(3), bb(4)));
        t!("v_box_deref_move", v_box_deref_move(s));
        t!("v_refmut_in_tuple", v_refmut_in_tuple(s));
        t!("v_ref_cell_field", v_ref_cell_field(&cells, bb(4)));
        t!("v_option_ref_mut", v_option_ref_mut(s));
        t!("v_raw_in_struct", v_raw_in_struct(s));
        t!("v_slot_ptr_in_struct", v_slot_ptr_in_struct(bb(4)));
        t!("v_ref_to_w", v_ref_to_w(&ws));
        t!("v_nested_ref", v_nested_ref(s));
        t!("v_rw", v_rw(s));
        t!("v_rw_reseat", v_rw_reseat(s));
        t!("v_mut_ref_param", v_mut_ref_param(&mut bb(P { x: 7, y: 8 }), s));
        t!("v_mut_ref_reborrow", v_mut_ref_reborrow(&mut bb(P { x: 7, y: 8 })));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + suma(&us) + unsafe { uu[1].a + u1[1].a + up1[0].p.x } + wr[1].0 + cells[1].get());
    }
}
