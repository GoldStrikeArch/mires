// Single-use shapes: the fresh value's only use is the storage use; results read only the
// operands/sources, or the value through the one view taken of it.
struct C { a: Cell<u32>, b: u32 }
struct CP { p: Cell<P>, k: u32 }
#[repr(transparent)] struct G(u32);
static SINK: AtomicU32 = AtomicU32::new(0);
impl Drop for G { fn drop(&mut self) { self.0 += 90; SINK.store(self.0, Ordering::Relaxed); } }
struct GH { g: G, k: u32 }
#[repr(transparent)] struct GP(P);
impl Drop for GP { fn drop(&mut self) { self.0.x += 90; SINK.store(self.0.x, Ordering::Relaxed); } }
struct DP { p: P, k: u32 }
impl Drop for DP { fn drop(&mut self) { self.p.x += 90; self.k += 1; SINK.store(self.p.x + self.k, Ordering::Relaxed); } }
struct DS { x: u32, y: u32 }
impl Drop for DS { fn drop(&mut self) { self.x += 90; SINK.store(self.x + self.y, Ordering::Relaxed); } }
#[inline(never)] fn wr_never(p: &mut P) -> u32 { p.x = 99; p.x + p.y }
#[inline(always)] fn wr_always(p: &mut P) -> u32 { p.x = 99; p.x + p.y }
#[inline(never)] fn wru_never(p: &mut u32) -> u32 { *p = 99; *p }
#[inline(always)] fn wru_always(p: &mut u32) -> u32 { *p = 99; *p }
#[inline(never)] fn wra_never(p: &mut [u32]) -> u32 { p[1] = 99; p[0] + p[1] }
#[inline(always)] fn wra_always(p: &mut [u32]) -> u32 { p[1] = 99; p[0] + p[1] }
#[inline(never)] fn wrw_never(p: &mut W) -> u32 { p.0 = 99; p.0 }
#[inline(always)] fn wrw_always(p: &mut W) -> u32 { p.0 = 99; p.0 }
#[inline(never)] fn raw_never(p: *mut P) -> u32 { unsafe { (*p).x = 99; (*p).x + (*p).y } }
#[inline(always)] fn raw_always(p: *mut P) -> u32 { unsafe { (*p).x = 99; (*p).x + (*p).y } }

// temporaries borrowed once, then used only through the borrow
#[inline(never)] fn t_tmp_slice_never(s: &[P]) -> u32 { wr_never(&mut { s[bb(1)] }) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_slice_always(s: &[P]) -> u32 { wr_always(&mut { s[bb(1)] }) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_static_never() -> u32 { wr_never(&mut { PS[bb(1)] }) * 1000 + PS[1].x }
#[inline(never)] fn t_tmp_static_always() -> u32 { wr_always(&mut { PS[bb(1)] }) * 1000 + PS[1].x }
#[inline(never)] fn t_tmp_static_const_idx() -> u32 { wr_always(&mut { PS[2] }) * 1000 + PS[2].x }
#[inline(never)] fn t_tmp_const() -> u32 { wr_always(&mut { KP }) * 1000 + KP.x }
#[inline(never)] fn t_tmp_const_never() -> u32 { wr_never(&mut { KP }) * 1000 + KP.x }
#[inline(never)] fn t_tmp_const_lit() -> u32 { wr_always(&mut P { x: 1, y: 2 }) + wr_never(&mut P { x: 1, y: 2 }) }
#[inline(never)] fn t_tmp_param_never(p: P) -> u32 { wr_never(&mut { p }) * 1000 + p.x }
#[inline(never)] fn t_tmp_param_always(p: P) -> u32 { wr_always(&mut { p }) * 1000 + p.x }
#[inline(never)] fn t_tmp_ref_never(p: &P) -> u32 { wr_never(&mut { *p }) * 1000 + p.x }
#[inline(never)] fn t_tmp_ref_always(p: &P) -> u32 { wr_always(&mut { *p }) * 1000 + p.x }
#[inline(never)] fn t_tmp_field_never(n: &N) -> u32 { wr_never(&mut { n.inner }) * 1000 + n.inner.x }
#[inline(never)] fn t_tmp_field_always(n: &N) -> u32 { wr_always(&mut { n.inner }) * 1000 + n.inner.x }
#[inline(never)] fn t_tmp_fresh_never(n: u32, m: u32) -> u32 { wr_never(&mut P { x: n, y: m }) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_fresh_always(n: u32, m: u32) -> u32 { wr_always(&mut P { x: n, y: m }) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_fresh_slice(s: &[u32]) -> u32 { wr_always(&mut P { x: s[bb(0)], y: s[bb(1)] }) * 1000 + s[0] + s[1] }
#[inline(never)] fn t_tmp_fresh_static() -> u32 { wr_always(&mut P { x: AA[bb(0)][0], y: AA[1][bb(1)] }) * 1000 + AA[0][0] + AA[1][1] }
#[inline(never)] fn t_tmp_scalar_never(n: u32) -> u32 { wru_never(&mut { n }) * 1000 + n }
#[inline(never)] fn t_tmp_scalar_always(n: u32) -> u32 { wru_always(&mut { n }) * 1000 + n }
#[inline(never)] fn t_tmp_scalar_slice(s: &[u32]) -> u32 { wru_always(&mut { s[bb(1)] }) * 1000 + s[1] }
#[inline(never)] fn t_tmp_scalar_field(p: &P) -> u32 { wru_always(&mut { p.x }) * 1000 + p.x }
#[inline(never)] fn t_tmp_scalar_field_val(p: P) -> u32 { wru_always(&mut { p.x }) * 1000 + p.x }
#[inline(never)] fn t_tmp_field_of_tmp(s: &[P]) -> u32 { wru_always(&mut { s[bb(1)] }.x) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_field_of_tmp_never(s: &[P]) -> u32 { wru_never(&mut { s[bb(1)] }.x) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_field_of_fresh(n: u32, m: u32) -> u32 { wru_always(&mut P { x: n, y: m }.x) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_field_of_fresh_never(n: u32, m: u32) -> u32 { wru_never(&mut P { x: n, y: m }.y) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_elem_of_fresh(n: u32, m: u32) -> u32 { wru_always(&mut [n, m][bb(1)]) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_elem_of_fresh_never(n: u32, m: u32) -> u32 { wru_never(&mut [n, m][1]) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_tuple_of_fresh(n: u32, m: u32) -> u32 { wru_always(&mut (n, m).0) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_inner_of_fresh(p: P, k: u32) -> u32 { wr_always(&mut N { a: k, inner: p }.inner) * 1000 + p.x + k }
#[inline(never)] fn t_tmp_inner_of_fresh_never(p: P, k: u32) -> u32 { wr_never(&mut N { a: k, inner: p }.inner) * 1000 + p.x + k }
#[inline(never)] fn t_tmp_inner_of_copy(ns: &[N]) -> u32 { wr_always(&mut { ns[bb(1)] }.inner) * 1000 + ns[1].inner.x }
#[inline(never)] fn t_tmp_inner_x_of_copy(ns: &[N]) -> u32 { wru_always(&mut { ns[bb(1)] }.inner.x) * 1000 + ns[1].inner.x }
#[inline(never)] fn t_tmp_arr_never(a: &[[u32; 3]]) -> u32 { wra_never(&mut { a[bb(1)] }) * 1000 + a[1][1] }
#[inline(never)] fn t_tmp_arr_always(a: &[[u32; 3]]) -> u32 { wra_always(&mut { a[bb(1)] }) * 1000 + a[1][1] }
#[inline(never)] fn t_tmp_arr_sub(a: &[[u32; 3]]) -> u32 { wra_always(&mut { a[bb(1)] }[1..]) * 1000 + a[1][2] }
#[inline(never)] fn t_tmp_arr_fresh(n: u32, m: u32) -> u32 { wra_always(&mut [n, m, n]) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_arr_fresh_never(n: u32, m: u32) -> u32 { wra_never(&mut [n, m, n]) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_arr_fresh_sub(n: u32, m: u32) -> u32 { wra_always(&mut [n, m, n][1..]) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_arr_static() -> u32 { wra_always(&mut { AA[bb(1)] }) * 1000 + AA[1][1] }
#[inline(never)] fn t_tmp_arr_const() -> u32 { wra_always(&mut { KA }) * 1000 + KA[1] + wra_never(&mut [1, 2, 3]) }
#[inline(never)] fn t_tmp_w_never(w: W) -> u32 { wrw_never(&mut { w }) * 1000 + w.0 }
#[inline(never)] fn t_tmp_w_always(w: W) -> u32 { wrw_always(&mut { w }) * 1000 + w.0 }
#[inline(never)] fn t_tmp_w_slice(ws: &[W]) -> u32 { wrw_always(&mut { ws[bb(1)] }) * 1000 + ws[1].0 }
#[inline(never)] fn t_tmp_w_fresh(n: u32) -> u32 { wrw_always(&mut W(n)) * 1000 + n }
#[inline(never)] fn t_tmp_w_fresh_never(n: u32) -> u32 { wrw_never(&mut W(n)) * 1000 + n }
#[inline(never)] fn t_tmp_w_inner(n: u32) -> u32 { wru_always(&mut W(n).0) * 1000 + n }
#[inline(never)] fn t_tmp_w_in_hw(w: W) -> u32 { wrw_always(&mut HW { w, pad: 1 }.w) * 1000 + w.0 }
#[inline(never)] fn t_tmp_w_in_hw0(w: W) -> u32 { wru_always(&mut HW { w, pad: 1 }.w.0) * 1000 + w.0 }
#[inline(never)] fn t_tmp_raw_never(s: &[P]) -> u32 { raw_never(&mut { s[bb(1)] }) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_raw_always(s: &[P]) -> u32 { raw_always(&mut { s[bb(1)] }) * 1000 + s[1].x }
#[inline(never)] fn t_tmp_raw_fresh(n: u32, m: u32) -> u32 { raw_always(&mut P { x: n, y: m }) * 10000 + n * 100 + m }
#[inline(never)] fn t_tmp_raw_addr(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &raw mut q; unsafe { (*r).x = 99; (*r).x * 10000 + n * 100 + m } }
#[inline(never)] fn t_tmp_raw_addr_field(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &raw mut q.x; unsafe { *r = 99; *r * 10000 + n * 100 + m } }
#[inline(never)] fn t_tmp_raw_const_addr(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &raw const q.x; unsafe { *(r as *mut u32) = 99; *r * 10000 + n * 100 + m } }
#[inline(never)] fn t_tmp_raw_const_whole(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &raw const q; unsafe { (*(r as *mut P)).y = 99; (*r).y * 10000 + n * 100 + m } }
#[inline(never)] fn t_tmp_raw_copy(s: &[P]) -> u32 { let mut q = s[bb(1)]; let r = &raw mut q; unsafe { (*r).x = 99; (*r).x * 1000 + s[1].x } }
#[inline(never)] fn t_tmp_raw_copy_field(s: &[P]) -> u32 { let mut q = s[bb(1)]; let r = &raw mut q.y; unsafe { *r = 99; *r * 1000 + s[1].y } }
#[inline(never)] fn t_ref_local_only(s: &[P]) -> u32 { let mut q = s[bb(1)]; let r = &mut q; r.x = 99; r.y += 1; (r.x + r.y) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn t_ref_local_field(s: &[P]) -> u32 { let mut q = s[bb(1)]; let r = &mut q.x; *r = 99; *r += 1; *r * 1000 + s[1].x }
#[inline(never)] fn t_ref_local_fresh(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &mut q; r.x = 99; r.y += 1; (r.x + r.y) * 10000 + n * 100 + m }
#[inline(never)] fn t_ref_local_fresh_field(n: u32, m: u32) -> u32 { let mut q = P { x: n, y: m }; let r = &mut q.y; *r += 90; *r * 10000 + n * 100 + m }
#[inline(never)] fn t_ref_local_arr(a: &[[u32; 3]]) -> u32 { let mut q = a[bb(1)]; let r = &mut q; r[bb(1)] = 99; r[0] += 1; suma(r) * 1000 + suma(&a[1]) }
#[inline(never)] fn t_ref_local_arr_elem(a: &[[u32; 3]]) -> u32 { let mut q = a[bb(1)]; let r = &mut q[bb(2)]; *r = 99; *r * 1000 + a[1][2] }
#[inline(never)] fn t_ref_local_arr_fresh(n: u32, m: u32) -> u32 { let mut q = [n, m]; let r = &mut q[bb(1)]; *r = 99; *r * 10000 + n * 100 + m }
#[inline(never)] fn t_ref_local_arr_fresh0(n: u32, m: u32) -> u32 { let mut q = [n, m]; let r = &mut q[0]; *r = 99; *r * 10000 + n * 100 + m }
#[inline(never)] fn t_ref_local_tuple_fresh(n: u32, m: u32) -> u32 { let mut q = (n, m); let r = &mut q.1; *r = 99; *r * 10000 + n * 100 + m }
#[inline(never)] fn t_ref_local_enum(n: u32, m: u32) -> u32 { let mut e = E::B(n, m); if let E::B(a, b) = &mut e { *a = 99; *b += 1; (*a + *b) * 10000 + n * 100 + m } else { 0 } }
#[inline(never)] fn t_ref_local_enum_copy(es: &[E]) -> u32 { let mut e = es[bb(1)]; if let E::B(a, b) = &mut e { *a = 99; *b += 1; (*a + *b) * 1000 + obs(&es[1]) } else { 0 } }
#[inline(never)] fn t_ref_local_opt(n: u32) -> u32 { let mut o = Some(n); if let Some(v) = &mut o { *v = 99; *v * 1000 + n } else { 0 } }
#[inline(never)] fn t_ref_local_opt_copy(o: &[Option<P>]) -> u32 { let mut c = o[bb(1)]; if let Some(v) = &mut c { v.x = 99; v.x * 1000 + o[1].unwrap().x } else { 0 } }
#[inline(never)] fn t_ref_local_opt_asmut(o: &[Option<P>]) -> u32 { let r = { o[bb(1)] }.as_mut().map(|v| { v.x = 99; v.x }).unwrap_or(0); r * 1000 + o[1].unwrap().x }

// dead writes: value written, never read
#[inline(never)] fn z_dead_field(s: &[P]) -> u32 { let mut c = s[bb(1)]; c.x = 99; s[1].x }
#[inline(never)] fn z_dead_field_fresh(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; c.x = 99; c.y = 98; n * 100 + m }
#[inline(never)] fn z_dead_field_fresh_slice(s: &[u32]) -> u32 { let mut c = P { x: s[bb(0)], y: s[bb(1)] }; c.x = 99; c.y = 98; s[0] * 100 + s[1] }
#[inline(never)] fn z_dead_elem(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c[bb(1)] = 99; c[0] = 98; suma(&a[1]) }
#[inline(never)] fn z_dead_elem_fresh(n: u32, m: u32, i: usize) -> u32 { let mut c = [n, m]; c[i] = 99; n * 100 + m }
#[inline(never)] fn z_dead_elem_fresh_const(n: u32, m: u32) -> u32 { let mut c = [n, m]; c[0] = 99; c[1] = 98; n * 100 + m }
#[inline(never)] fn z_dead_tuple(n: u32, m: u32) -> u32 { let mut c = (n, m); c.0 = 99; c.1 += 1; n * 100 + m }
#[inline(never)] fn z_dead_w(w: W) -> u32 { let mut c = w; c.0 = 99; w.0 }
#[inline(never)] fn z_dead_w_fresh(n: u32) -> u32 { let mut c = W(n); c.0 = 99; n }
#[inline(never)] fn z_dead_w_slice(ws: &[W]) -> u32 { let mut c = ws[bb(1)]; c.0 = 99; ws[1].0 }
#[inline(never)] fn z_dead_nested(ns: &[N]) -> u32 { let mut c = ns[bb(1)]; c.inner.x = 99; c.a = 98; ns[1].inner.x * 100 + ns[1].a }
#[inline(never)] fn z_dead_nested_fresh(p: P, k: u32) -> u32 { let mut c = N { a: k, inner: p }; c.inner.x = 99; c.a = 98; p.x * 100 + k }
#[inline(never)] fn z_dead_nested_fresh2(s: &[P], k: u32) -> u32 { let mut c = N { a: k, inner: s[bb(1)] }; c.inner.x = 99; s[1].x * 100 + k }
#[inline(never)] fn z_dead_hw(w: W) -> u32 { let mut c = HW { w, pad: 3 }; c.w.0 = 99; c.pad = 98; w.0 }
#[inline(never)] fn z_dead_hw2(w: W) -> u32 { let mut c = HW { w, pad: 3 }; c.w = W(99); w.0 }
#[inline(never)] fn z_dead_enum(n: u32, m: u32) -> u32 { let mut e = E::B(n, m); if let E::B(a, _) = &mut e { *a = 99; } n * 100 + m }
#[inline(never)] fn z_dead_enum_whole(es: &[E]) -> u32 { let mut e = es[bb(1)]; e = E::A(7); if let E::A(v) = &mut e { *v = 99; } obs(&es[1]) }
#[inline(never)] fn z_dead_opt(n: u32) -> u32 { let mut o = Some(n); if let Some(v) = &mut o { *v = 99; } n }
#[inline(never)] fn z_dead_opt_take(o: &[Option<P>]) -> u32 { let mut c = o[bb(1)]; c.take(); o[1].is_some() as u32 }
#[inline(never)] fn z_dead_opt_take_fresh(p: P) -> u32 { let mut c = Some(p); let t = c.take(); t.unwrap().x * 100 + p.x }
#[inline(never)] fn z_dead_replace(s: &[P]) -> u32 { let mut c = s[bb(1)]; let _ = mem::replace(&mut c.x, 99); s[1].x }
#[inline(never)] fn z_dead_replace_fresh(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; let old = mem::replace(&mut c.x, 99); old * 10000 + n * 100 + m }
#[inline(never)] fn z_dead_take_fresh(n: u32, m: u32) -> u32 { let mut c = (n, m); let old = mem::take(&mut c.1); old * 10000 + n * 100 + m }
#[inline(never)] fn z_dead_swap_fresh(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; mem::swap(&mut c.x, &mut c.y); n * 100 + m }
#[inline(never)] fn z_dead_swap_two(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; let mut d = P { x: m, y: n }; mem::swap(&mut c, &mut d); n * 100 + m }
#[inline(never)] fn z_dead_swap_two_fields(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; let mut d = P { x: m, y: n }; mem::swap(&mut c.x, &mut d.x); n * 100 + m }
#[inline(never)] fn z_dead_swap_copies(s: &[P]) -> u32 { let mut c = s[bb(0)]; let mut d = s[bb(1)]; mem::swap(&mut c, &mut d); sum(s) }
#[inline(never)] fn z_dead_swap_scalars(n: u32, m: u32) -> u32 { let mut c = n; let mut d = m; mem::swap(&mut c, &mut d); n * 100 + m }
#[inline(never)] fn z_dead_swap_slice_elems(s: &[u32]) -> u32 { let mut c = s[bb(0)]; let mut d = s[bb(1)]; mem::swap(&mut c, &mut d); s[0] * 100 + s[1] }
#[inline(never)] fn z_dead_fill(n: u32, m: u32) -> u32 { let mut c = [n, m, n]; c.fill(99); n * 100 + m }
#[inline(never)] fn z_dead_fill_sub(n: u32, m: u32) -> u32 { let mut c = [n, m, n]; c[1..].fill(99); n * 100 + m }
#[inline(never)] fn z_dead_reverse(n: u32, m: u32) -> u32 { let mut c = [n, m]; c.reverse(); n * 100 + m }
#[inline(never)] fn z_dead_swap_idx(n: u32, m: u32) -> u32 { let mut c = [n, m]; c.swap(0, 1); n * 100 + m }
#[inline(never)] fn z_dead_iter_mut(n: u32, m: u32) -> u32 { let mut c = [n, m]; for v in c.iter_mut() { *v += 1; } n * 100 + m }
#[inline(never)] fn z_dead_iter_mut_copy(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; for v in c.iter_mut() { *v += 1; } suma(&a[1]) }
#[inline(never)] fn z_dead_copy_from(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; c.copy_from_slice(&a[bb(0)]); suma(&a[1]) * 1000 + suma(&a[0]) }
#[inline(never)] fn z_dead_ptr_write(n: u32, m: u32) -> u32 { let mut c = P { x: n, y: m }; unsafe { core::ptr::write(&mut c.x, 99); core::ptr::write(&mut c, P { x: 1, y: 2 }); } n * 100 + m }
#[inline(never)] fn z_dead_ptr_write_copy(s: &[P]) -> u32 { let mut c = s[bb(1)]; unsafe { core::ptr::write(&mut c.x, 99); } s[1].x }

// drops of fresh values whose destructor writes self
#[inline(never)] fn g_param(n: u32) -> u32 { { let _g = G(n); } SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn g_slice(s: &[u32]) -> u32 { { let _g = G(s[bb(1)]); } SINK.load(Ordering::Relaxed) * 1000 + s[1] }
#[inline(never)] fn g_static() -> u32 { { let _g = G(AA[bb(1)][bb(1)]); } SINK.load(Ordering::Relaxed) * 1000 + AA[1][1] }
#[inline(never)] fn g_field(p: &P) -> u32 { { let _g = G(p.x); } SINK.load(Ordering::Relaxed) * 1000 + p.x }
#[inline(never)] fn g_field_val(p: P) -> u32 { { let _g = G(p.x); } SINK.load(Ordering::Relaxed) * 1000 + p.x }
#[inline(never)] fn g_drop_fn(n: u32) -> u32 { drop(G(n)); SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn g_drop_fn_slice(s: &[u32]) -> u32 { drop(G(s[bb(1)])); SINK.load(Ordering::Relaxed) * 1000 + s[1] }
#[inline(never)] fn g_held(n: u32, k: u32) -> u32 { { let _h = GH { g: G(n), k }; } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + k }
#[inline(never)] fn g_held_slice(s: &[u32]) -> u32 { { let _h = GH { g: G(s[bb(1)]), k: s[bb(2)] }; } SINK.load(Ordering::Relaxed) * 1000 + s[1] + s[2] }
#[inline(never)] fn g_tuple(n: u32, m: u32) -> u32 { { let _t = (G(n), m); } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_arr(n: u32, m: u32) -> u32 { { let _t = [G(n), G(m)]; } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_opt(n: u32) -> u32 { { let _t = Some(G(n)); } SINK.load(Ordering::Relaxed) * 1000 + n }
#[inline(never)] fn g_assign(n: u32, m: u32) -> u32 { { let mut g = G(n); g = G(m); } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_gp(s: &[P]) -> u32 { { let _g = GP(s[bb(1)]); } SINK.load(Ordering::Relaxed) * 1000 + s[1].x }
#[inline(never)] fn g_gp_param(p: P) -> u32 { { let _g = GP(p); } SINK.load(Ordering::Relaxed) * 1000 + p.x }
#[inline(never)] fn g_gp_ref(p: &P) -> u32 { { let _g = GP(*p); } SINK.load(Ordering::Relaxed) * 1000 + p.x }
#[inline(never)] fn g_gp_fresh(n: u32, m: u32) -> u32 { { let _g = GP(P { x: n, y: m }); } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_dp(s: &[P], k: u32) -> u32 { { let _g = DP { p: s[bb(1)], k }; } SINK.load(Ordering::Relaxed) * 1000 + s[1].x + k }
#[inline(never)] fn g_dp_param(p: P, k: u32) -> u32 { { let _g = DP { p, k }; } SINK.load(Ordering::Relaxed) * 1000 + p.x + k }
#[inline(never)] fn g_dp_drop(p: P, k: u32) -> u32 { drop(DP { p, k }); SINK.load(Ordering::Relaxed) * 1000 + p.x + k }
#[inline(never)] fn g_ds(n: u32, m: u32) -> u32 { { let _g = DS { x: n, y: m }; } SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_ds_slice(s: &[u32]) -> u32 { { let _g = DS { x: s[bb(1)], y: s[bb(2)] }; } SINK.load(Ordering::Relaxed) * 1000 + s[1] + s[2] }
#[inline(never)] fn g_ds_drop(n: u32, m: u32) -> u32 { drop(DS { x: n, y: m }); SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_ds_in_place(n: u32, m: u32) -> u32 { let mut d = ManuallyDrop::new(DS { x: n, y: m }); unsafe { ManuallyDrop::drop(&mut d) }; SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }
#[inline(never)] fn g_ds_box(n: u32, m: u32) -> u32 { drop(Box::new(DS { x: n, y: m })); SINK.load(Ordering::Relaxed) * 10000 + n * 100 + m }

// shared views with interior mutability, value dead afterwards or read only through the view
#[inline(never)] fn c_set_dead(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; q.a.set(99); n * 100 + m }
#[inline(never)] fn c_set_view(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; let r = &q.a; r.set(99); r.get() * 10000 + n * 100 + m }
#[inline(never)] fn c_set_view_whole(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; let r = &q; r.a.set(99); (r.a.get() + r.b) * 10000 + n * 100 + m }
#[inline(never)] fn c_set_slice(s: &[u32]) -> u32 { let q = C { a: Cell::new(s[bb(1)]), b: 2 }; let r = &q.a; r.set(99); r.get() * 1000 + s[1] }
#[inline(never)] fn c_only(n: u32) -> u32 { let q = Cell::new(n); let r = &q; r.set(99); r.get() * 1000 + n }
#[inline(never)] fn c_only_dead(n: u32) -> u32 { let q = Cell::new(n); q.set(99); n }
#[inline(never)] fn c_only_slice(s: &[u32]) -> u32 { let q = Cell::new(s[bb(1)]); q.set(99); s[1] }
#[inline(never)] fn c_only_slice_view(s: &[u32]) -> u32 { let q = Cell::new(s[bb(1)]); let r = &q; r.set(99); r.get() * 1000 + s[1] }
#[inline(never)] fn c_arr_dead(n: u32, m: u32) -> u32 { let q = [Cell::new(n), Cell::new(m)]; q[bb(1)].set(99); q[0].set(98); n * 100 + m }
#[inline(never)] fn c_arr_view(n: u32, m: u32) -> u32 { let q = [Cell::new(n), Cell::new(m)]; let r = &q[bb(1)]; r.set(99); r.get() * 10000 + n * 100 + m }
#[inline(never)] fn c_tuple_dead(n: u32, m: u32) -> u32 { let q = (Cell::new(n), m); q.0.set(99); n * 100 + m }
#[inline(never)] fn c_opt_dead(n: u32) -> u32 { let q = Some(Cell::new(n)); if let Some(c) = &q { c.set(99); } n }
#[inline(never)] fn c_p_dead(s: &[P]) -> u32 { let q = Cell::new(s[bb(1)]); q.set(P { x: 99, y: 98 }); s[1].x }
#[inline(never)] fn c_p_as_ptr(s: &[P]) -> u32 { let q = Cell::new(s[bb(1)]); unsafe { (*q.as_ptr()).x = 99 }; s[1].x }
#[inline(never)] fn c_p_as_ptr_view(s: &[P]) -> u32 { let q = Cell::new(s[bb(1)]); let p = q.as_ptr(); unsafe { (*p).x = 99; (*p).x * 1000 + s[1].x } }
#[inline(never)] fn c_p_as_ptr_param(p: P) -> u32 { let q = Cell::new(p); unsafe { (*q.as_ptr()).x = 99 }; p.x }
#[inline(never)] fn c_cp_dead(p: P, k: u32) -> u32 { let q = CP { p: Cell::new(p), k }; unsafe { (*q.p.as_ptr()).x = 99 }; p.x * 100 + k }
#[inline(never)] fn c_swap_dead(n: u32, m: u32) -> u32 { let a = Cell::new(n); let b = Cell::new(m); a.swap(&b); n * 100 + m }
#[inline(never)] fn c_replace_dead(n: u32) -> u32 { let a = Cell::new(n); let old = a.replace(99); old * 1000 + n }
#[inline(never)] fn c_take_dead(n: u32) -> u32 { let a = Cell::new(n); let old = a.take(); old * 1000 + n }
#[inline(never)] fn c_w_dead(w: W) -> u32 { let a = Cell::new(w); a.set(W(99)); w.0 }
#[inline(never)] fn c_w_view(w: W) -> u32 { let a = Cell::new(w); let r = &a; r.set(W(99)); r.get().0 * 1000 + w.0 }
#[inline(never)] fn c_atomic_dead(n: u32) -> u32 { let a = AtomicU32::new(n); a.store(99, Ordering::Relaxed); n }
#[inline(never)] fn c_atomic_add(n: u32) -> u32 { let a = AtomicU32::new(n); let old = a.fetch_add(5, Ordering::Relaxed); old * 1000 + n }
#[inline(never)] fn c_atomic_slice(s: &[u32]) -> u32 { let a = AtomicU32::new(s[bb(1)]); a.fetch_add(5, Ordering::Relaxed); s[1] }
#[inline(never)] fn c_unsafecell_dead(n: u32) -> u32 { let a = UnsafeCell::new(n); unsafe { *a.get() = 99 }; n }
#[inline(never)] fn c_unsafecell_view(n: u32) -> u32 { let a = UnsafeCell::new(n); let p = a.get(); unsafe { *p = 99; *p * 1000 + n } }
#[inline(never)] fn c_unsafecell_p(p: P) -> u32 { let a = UnsafeCell::new(p); unsafe { (*a.get()).x = 99 }; p.x }
#[inline(never)] fn c_unsafecell_p_slice(s: &[P]) -> u32 { let a = UnsafeCell::new(s[bb(1)]); let q = a.get(); unsafe { (*q).x = 99; (*q).x * 1000 + s[1].x } }
#[inline(never)] fn c_refcell_dead(n: u32) -> u32 { let a = RefCell::new(n); *a.borrow_mut() = 99; n }
#[inline(never)] fn c_refcell_p(s: &[P]) -> u32 { let a = RefCell::new(s[bb(1)]); a.borrow_mut().x = 99; s[1].x }
#[inline(never)] fn c_refcell_p_param(p: P) -> u32 { let a = RefCell::new(p); a.borrow_mut().x = 99; p.x }

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
        let us = bb([13u32, 14, 15]);
        let p78 = bb(P { x: 7, y: 8 });
        t!("t_tmp_slice_never", t_tmp_slice_never(s));
        t!("t_tmp_slice_always", t_tmp_slice_always(s));
        t!("t_tmp_static_never", t_tmp_static_never());
        t!("t_tmp_static_always", t_tmp_static_always());
        t!("t_tmp_static_const_idx", t_tmp_static_const_idx());
        t!("t_tmp_const", t_tmp_const());
        t!("t_tmp_const_never", t_tmp_const_never());
        t!("t_tmp_const_lit", t_tmp_const_lit());
        t!("t_tmp_param_never", t_tmp_param_never(p78));
        t!("t_tmp_param_always", t_tmp_param_always(p78));
        t!("t_tmp_ref_never", t_tmp_ref_never(&s[1]));
        t!("t_tmp_ref_always", t_tmp_ref_always(&s[1]));
        t!("t_tmp_field_never", t_tmp_field_never(&ns[1]));
        t!("t_tmp_field_always", t_tmp_field_always(&ns[1]));
        t!("t_tmp_fresh_never", t_tmp_fresh_never(bb(3), bb(4)));
        t!("t_tmp_fresh_always", t_tmp_fresh_always(bb(3), bb(4)));
        t!("t_tmp_fresh_slice", t_tmp_fresh_slice(&us));
        t!("t_tmp_fresh_static", t_tmp_fresh_static());
        t!("t_tmp_scalar_never", t_tmp_scalar_never(bb(3)));
        t!("t_tmp_scalar_always", t_tmp_scalar_always(bb(3)));
        t!("t_tmp_scalar_slice", t_tmp_scalar_slice(&us));
        t!("t_tmp_scalar_field", t_tmp_scalar_field(&s[1]));
        t!("t_tmp_scalar_field_val", t_tmp_scalar_field_val(p78));
        t!("t_tmp_field_of_tmp", t_tmp_field_of_tmp(s));
        t!("t_tmp_field_of_tmp_never", t_tmp_field_of_tmp_never(s));
        t!("t_tmp_field_of_fresh", t_tmp_field_of_fresh(bb(3), bb(4)));
        t!("t_tmp_field_of_fresh_never", t_tmp_field_of_fresh_never(bb(3), bb(4)));
        t!("t_tmp_elem_of_fresh", t_tmp_elem_of_fresh(bb(3), bb(4)));
        t!("t_tmp_elem_of_fresh_never", t_tmp_elem_of_fresh_never(bb(3), bb(4)));
        t!("t_tmp_tuple_of_fresh", t_tmp_tuple_of_fresh(bb(3), bb(4)));
        t!("t_tmp_inner_of_fresh", t_tmp_inner_of_fresh(p78, bb(2)));
        t!("t_tmp_inner_of_fresh_never", t_tmp_inner_of_fresh_never(p78, bb(2)));
        t!("t_tmp_inner_of_copy", t_tmp_inner_of_copy(&ns));
        t!("t_tmp_inner_x_of_copy", t_tmp_inner_x_of_copy(&ns));
        t!("t_tmp_arr_never", t_tmp_arr_never(&aa));
        t!("t_tmp_arr_always", t_tmp_arr_always(&aa));
        t!("t_tmp_arr_sub", t_tmp_arr_sub(&aa));
        t!("t_tmp_arr_fresh", t_tmp_arr_fresh(bb(3), bb(4)));
        t!("t_tmp_arr_fresh_never", t_tmp_arr_fresh_never(bb(3), bb(4)));
        t!("t_tmp_arr_fresh_sub", t_tmp_arr_fresh_sub(bb(3), bb(4)));
        t!("t_tmp_arr_static", t_tmp_arr_static());
        t!("t_tmp_arr_const", t_tmp_arr_const());
        t!("t_tmp_w_never", t_tmp_w_never(bb(W(5))));
        t!("t_tmp_w_always", t_tmp_w_always(bb(W(5))));
        t!("t_tmp_w_slice", t_tmp_w_slice(&ws));
        t!("t_tmp_w_fresh", t_tmp_w_fresh(bb(5)));
        t!("t_tmp_w_fresh_never", t_tmp_w_fresh_never(bb(5)));
        t!("t_tmp_w_inner", t_tmp_w_inner(bb(5)));
        t!("t_tmp_w_in_hw", t_tmp_w_in_hw(bb(W(5))));
        t!("t_tmp_w_in_hw0", t_tmp_w_in_hw0(bb(W(5))));
        t!("t_tmp_raw_never", t_tmp_raw_never(s));
        t!("t_tmp_raw_always", t_tmp_raw_always(s));
        t!("t_tmp_raw_fresh", t_tmp_raw_fresh(bb(3), bb(4)));
        t!("t_tmp_raw_addr", t_tmp_raw_addr(bb(3), bb(4)));
        t!("t_tmp_raw_addr_field", t_tmp_raw_addr_field(bb(3), bb(4)));
        t!("t_tmp_raw_const_addr", t_tmp_raw_const_addr(bb(3), bb(4)));
        t!("t_tmp_raw_const_whole", t_tmp_raw_const_whole(bb(3), bb(4)));
        t!("t_tmp_raw_copy", t_tmp_raw_copy(s));
        t!("t_tmp_raw_copy_field", t_tmp_raw_copy_field(s));
        t!("t_ref_local_only", t_ref_local_only(s));
        t!("t_ref_local_field", t_ref_local_field(s));
        t!("t_ref_local_fresh", t_ref_local_fresh(bb(3), bb(4)));
        t!("t_ref_local_fresh_field", t_ref_local_fresh_field(bb(3), bb(4)));
        t!("t_ref_local_arr", t_ref_local_arr(&aa));
        t!("t_ref_local_arr_elem", t_ref_local_arr_elem(&aa));
        t!("t_ref_local_arr_fresh", t_ref_local_arr_fresh(bb(3), bb(4)));
        t!("t_ref_local_arr_fresh0", t_ref_local_arr_fresh0(bb(3), bb(4)));
        t!("t_ref_local_tuple_fresh", t_ref_local_tuple_fresh(bb(3), bb(4)));
        t!("t_ref_local_enum", t_ref_local_enum(bb(3), bb(4)));
        t!("t_ref_local_enum_copy", t_ref_local_enum_copy(&es));
        t!("t_ref_local_opt", t_ref_local_opt(bb(3)));
        t!("t_ref_local_opt_copy", t_ref_local_opt_copy(&op));
        t!("t_ref_local_opt_asmut", t_ref_local_opt_asmut(&op));
        t!("z_dead_field", z_dead_field(s));
        t!("z_dead_field_fresh", z_dead_field_fresh(bb(3), bb(4)));
        t!("z_dead_field_fresh_slice", z_dead_field_fresh_slice(&us));
        t!("z_dead_elem", z_dead_elem(&aa));
        t!("z_dead_elem_fresh", z_dead_elem_fresh(bb(3), bb(4), bb(1)));
        t!("z_dead_elem_fresh_const", z_dead_elem_fresh_const(bb(3), bb(4)));
        t!("z_dead_tuple", z_dead_tuple(bb(3), bb(4)));
        t!("z_dead_w", z_dead_w(bb(W(5))));
        t!("z_dead_w_fresh", z_dead_w_fresh(bb(5)));
        t!("z_dead_w_slice", z_dead_w_slice(&ws));
        t!("z_dead_nested", z_dead_nested(&ns));
        t!("z_dead_nested_fresh", z_dead_nested_fresh(p78, bb(2)));
        t!("z_dead_nested_fresh2", z_dead_nested_fresh2(s, bb(2)));
        t!("z_dead_hw", z_dead_hw(bb(W(5))));
        t!("z_dead_hw2", z_dead_hw2(bb(W(5))));
        t!("z_dead_enum", z_dead_enum(bb(3), bb(4)));
        t!("z_dead_enum_whole", z_dead_enum_whole(&es));
        t!("z_dead_opt", z_dead_opt(bb(3)));
        t!("z_dead_opt_take", z_dead_opt_take(&op));
        t!("z_dead_opt_take_fresh", z_dead_opt_take_fresh(p78));
        t!("z_dead_replace", z_dead_replace(s));
        t!("z_dead_replace_fresh", z_dead_replace_fresh(bb(3), bb(4)));
        t!("z_dead_take_fresh", z_dead_take_fresh(bb(3), bb(4)));
        t!("z_dead_swap_fresh", z_dead_swap_fresh(bb(3), bb(4)));
        t!("z_dead_swap_two", z_dead_swap_two(bb(3), bb(4)));
        t!("z_dead_swap_two_fields", z_dead_swap_two_fields(bb(3), bb(4)));
        t!("z_dead_swap_copies", z_dead_swap_copies(s));
        t!("z_dead_swap_scalars", z_dead_swap_scalars(bb(3), bb(4)));
        t!("z_dead_swap_slice_elems", z_dead_swap_slice_elems(&us));
        t!("z_dead_fill", z_dead_fill(bb(3), bb(4)));
        t!("z_dead_fill_sub", z_dead_fill_sub(bb(3), bb(4)));
        t!("z_dead_reverse", z_dead_reverse(bb(3), bb(4)));
        t!("z_dead_swap_idx", z_dead_swap_idx(bb(3), bb(4)));
        t!("z_dead_iter_mut", z_dead_iter_mut(bb(3), bb(4)));
        t!("z_dead_iter_mut_copy", z_dead_iter_mut_copy(&aa));
        t!("z_dead_copy_from", z_dead_copy_from(&aa));
        t!("z_dead_ptr_write", z_dead_ptr_write(bb(3), bb(4)));
        t!("z_dead_ptr_write_copy", z_dead_ptr_write_copy(s));
        t!("g_param", g_param(bb(3)));
        t!("g_slice", g_slice(&us));
        t!("g_static", g_static());
        t!("g_field", g_field(&s[1]));
        t!("g_field_val", g_field_val(p78));
        t!("g_drop_fn", g_drop_fn(bb(3)));
        t!("g_drop_fn_slice", g_drop_fn_slice(&us));
        t!("g_held", g_held(bb(3), bb(4)));
        t!("g_held_slice", g_held_slice(&us));
        t!("g_tuple", g_tuple(bb(3), bb(4)));
        t!("g_arr", g_arr(bb(3), bb(4)));
        t!("g_opt", g_opt(bb(3)));
        t!("g_assign", g_assign(bb(3), bb(4)));
        t!("g_gp", g_gp(s));
        t!("g_gp_param", g_gp_param(p78));
        t!("g_gp_ref", g_gp_ref(&s[1]));
        t!("g_gp_fresh", g_gp_fresh(bb(3), bb(4)));
        t!("g_dp", g_dp(s, bb(2)));
        t!("g_dp_param", g_dp_param(p78, bb(2)));
        t!("g_dp_drop", g_dp_drop(p78, bb(2)));
        t!("g_ds", g_ds(bb(3), bb(4)));
        t!("g_ds_slice", g_ds_slice(&us));
        t!("g_ds_drop", g_ds_drop(bb(3), bb(4)));
        t!("g_ds_in_place", g_ds_in_place(bb(3), bb(4)));
        t!("g_ds_box", g_ds_box(bb(3), bb(4)));
        t!("c_set_dead", c_set_dead(bb(3), bb(4)));
        t!("c_set_view", c_set_view(bb(3), bb(4)));
        t!("c_set_view_whole", c_set_view_whole(bb(3), bb(4)));
        t!("c_set_slice", c_set_slice(&us));
        t!("c_only", c_only(bb(3)));
        t!("c_only_dead", c_only_dead(bb(3)));
        t!("c_only_slice", c_only_slice(&us));
        t!("c_only_slice_view", c_only_slice_view(&us));
        t!("c_arr_dead", c_arr_dead(bb(3), bb(4)));
        t!("c_arr_view", c_arr_view(bb(3), bb(4)));
        t!("c_tuple_dead", c_tuple_dead(bb(3), bb(4)));
        t!("c_opt_dead", c_opt_dead(bb(3)));
        t!("c_p_dead", c_p_dead(s));
        t!("c_p_as_ptr", c_p_as_ptr(s));
        t!("c_p_as_ptr_view", c_p_as_ptr_view(s));
        t!("c_p_as_ptr_param", c_p_as_ptr_param(p78));
        t!("c_cp_dead", c_cp_dead(p78, bb(2)));
        t!("c_swap_dead", c_swap_dead(bb(3), bb(4)));
        t!("c_replace_dead", c_replace_dead(bb(3)));
        t!("c_take_dead", c_take_dead(bb(3)));
        t!("c_w_dead", c_w_dead(bb(W(5))));
        t!("c_w_view", c_w_view(bb(W(5))));
        t!("c_atomic_dead", c_atomic_dead(bb(3)));
        t!("c_atomic_add", c_atomic_add(bb(3)));
        t!("c_atomic_slice", c_atomic_slice(&us));
        t!("c_unsafecell_dead", c_unsafecell_dead(bb(3)));
        t!("c_unsafecell_view", c_unsafecell_view(bb(3)));
        t!("c_unsafecell_p", c_unsafecell_p(p78));
        t!("c_unsafecell_p_slice", c_unsafecell_p_slice(s));
        t!("c_refcell_dead", c_refcell_dead(bb(3)));
        t!("c_refcell_p", c_refcell_p(s));
        t!("c_refcell_p_param", c_refcell_p_param(p78));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + obs(&es[1]) + ws[1].0 + ns[1].inner.x + op[1].unwrap().x + suma(&us) + p78.x);
    }
}
