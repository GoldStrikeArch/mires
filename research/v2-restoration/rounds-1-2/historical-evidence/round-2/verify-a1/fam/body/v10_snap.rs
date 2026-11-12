// Snapshots (a copy that is only read must not follow later writes to its source), library copy
// paths, and rule-B control-flow shapes.
#[inline(never)] fn set_x(p: &mut P) { p.x = 99; }
#[inline(always)] fn set_x_a(p: &mut P) { p.x = 99; }
#[inline(never)] fn consume(p: P) -> u32 { p.x * 10 + p.y }
#[repr(transparent)] #[derive(Clone, Copy)] struct WB(bool);
#[repr(transparent)] #[derive(Clone, Copy)] struct WF(f64);
#[repr(transparent)] #[derive(Clone, Copy)] struct W64(u64);
#[repr(transparent)] #[derive(Clone, Copy)] struct WC(char);
#[repr(transparent)] #[derive(Clone, Copy)] struct WI8(i8);
#[repr(transparent)] #[derive(Clone, Copy)] struct WFn(fn(u32) -> u32);
#[repr(transparent)] #[derive(Clone, Copy)] struct WPtr(*const u32);
#[repr(transparent)] #[derive(Clone, Copy)] struct Outer(W);
#[repr(transparent)] #[derive(Clone, Copy)] struct Outer2(Outer);
#[derive(Clone, Copy, PartialEq)] enum Prim { A, B, C }
#[repr(transparent)] #[derive(Clone, Copy)] struct WPrim(Prim);
fn inc(v: u32) -> u32 { v + 1 }
fn dbl(v: u32) -> u32 { v * 2 }

#[inline(never)] fn snap_mut(s: &mut [P]) -> u32 { let c = s[bb(1)]; s[1].x = 99; c.x * 1000 + s[1].x }
#[inline(never)] fn snap_callee(s: &mut [P]) -> u32 { let c = s[bb(1)]; set_x(&mut s[1]); c.x * 1000 + s[1].x }
#[inline(never)] fn snap_callee_a(s: &mut [P]) -> u32 { let c = s[bb(1)]; set_x_a(&mut s[1]); c.x * 1000 + s[1].x }
#[inline(never)] fn snap_cell(cells: &[Cell<P>]) -> u32 { let c = cells[bb(1)].get(); cells[1].set(P { x: 99, y: 98 }); c.x * 1000 + cells[1].get().x }
#[inline(never)] fn snap_raw(p: *mut P) -> u32 { unsafe { let c = *p; (*p).x = 99; c.x * 1000 + (*p).x } }
#[inline(never)] fn snap_field(n: &mut N) -> u32 { let c = n.inner; n.inner.x = 99; let d = n.inner; n.inner.y = 98; c.x * 10000 + d.y * 100 + d.x }
#[inline(never)] fn snap_whole(n: &mut N) -> u32 { let c = *n; n.inner = P { x: 99, y: 98 }; n.a = 97; c.inner.x * 1000 + c.a * 100 + n.a }
#[inline(never)] fn snap_w(w: &mut W) -> u32 { let c = *w; w.0 = 99; c.0 * 1000 + w.0 }
#[inline(never)] fn snap_arr(a: &mut [[u32; 3]]) -> u32 { let c = a[bb(1)]; a[1][1] = 99; c[1] * 1000 + a[1][1] }
#[inline(never)] fn snap_swap(s: &mut [P]) -> u32 { let c = s[bb(0)]; s.swap(0, 2); c.x * 1000 + s[0].x }
#[inline(never)] fn snap_loop(s: &mut [P]) -> u32 { let mut t = 0; for i in 0..s.len() { let c = s[i]; s[i].x += 10; t += c.x; } t * 1000 + sum(s) }
#[inline(never)] fn snap_local(n: u32) -> u32 { let mut a = P { x: n, y: 2 }; let c = a; a.x = 99; c.x * 1000 + a.x }
#[inline(never)] fn snap_local_w(n: u32) -> u32 { let mut a = W(n); let c = a; a.0 = 99; c.0 * 1000 + a.0 }
#[inline(never)] fn snap_ret(s: &mut [P]) -> P { let c = s[bb(1)]; s[1].x = 99; c }
#[inline(never)] fn snap_agg(s: &mut [P]) -> N { let n = N { a: 1, inner: s[bb(1)] }; s[1].x = 99; n }
#[inline(never)] fn snap_arg(s: &mut [P]) -> u32 { let c = s[bb(1)]; s[1].x = 99; consume(c) * 1000 + s[1].x }
#[inline(never)] fn snap_vec(v: &mut Vec<P>) -> u32 { let c = v[bb(1)]; v[1].x = 99; v.push(c); v[2].x * 1000 + v[1].x }
#[inline(never)] fn snap_cellget(c: &Cell<u32>) -> u32 { let q = W(c.get()); c.set(99); q.0 * 1000 + c.get() }
#[inline(never)] fn snap_cell_struct(c: &Cell<u32>, d: &Cell<u32>) -> u32 { let q = P { x: c.get(), y: d.get() }; c.set(99); d.set(98); q.x * 1000 + q.y * 10 + c.get() }
#[inline(never)] fn snap_tuple(s: &mut [P]) -> u32 { let t = (s[bb(0)], s[bb(1)]); s[0].x = 99; s[1].y = 98; (t.0.x + t.1.y) * 1000 + s[0].x + s[1].y }
#[inline(never)] fn snap_arr_of(s: &mut [P]) -> u32 { let t = [s[bb(0)], s[bb(1)]]; s[0].x = 99; s[1].y = 98; sum(&t) * 1000 + s[0].x + s[1].y }
#[inline(never)] fn snap_opt(s: &mut [P]) -> u32 { let o = Some(s[bb(1)]); s[1].x = 99; o.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn snap_closure(s: &mut [P]) -> u32 { let p = s[bb(1)]; let f = move || p.x; s[1].x = 99; f() * 1000 + s[1].x }
#[inline(never)] fn snap_static_mut() -> u32 { static mut SM: [P; 2] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; unsafe { let c = SM[bb(1)]; SM[1].x += 10; let d = SM[1]; SM[1].y += 10; c.x * 10000 + d.x * 100 + d.y } }

// library copy paths, then a write into the copy
#[inline(never)] fn l_to_vec(s: &[P]) -> u32 { let mut v = s.to_vec(); v[1].x = 99; sum(&v) * 1000 + sum(s) }
#[inline(never)] fn l_extend(s: &[P]) -> u32 { let mut v = Vec::new(); v.extend_from_slice(s); v[1].x = 99; sum(&v) * 1000 + sum(s) }
#[inline(never)] fn l_vec_clone(s: &[P]) -> u32 { let v = s.to_vec(); let mut w = v.clone(); w[1].x = 99; (sum(&w) + sum(&v)) * 1000 + sum(s) }
#[inline(never)] fn l_first_copied(s: &[P]) -> u32 { let mut c = s.first().copied().unwrap(); c.x = 99; c.x * 1000 + s[0].x }
#[inline(never)] fn l_last_deref(s: &[P]) -> u32 { let mut c = *s.last().unwrap(); c.x = 99; c.x * 1000 + s[2].x }
#[inline(never)] fn l_get_cloned(s: &[P]) -> u32 { let mut c = s.get(bb(1)).cloned().unwrap(); c.y = 99; c.y * 1000 + s[1].y }
#[inline(never)] fn l_iter_copied(s: &[P]) -> u32 { let mut t = 0; for mut p in s.iter().copied() { p.x += 50; t += p.x; } let mut it = s.iter().copied(); let mut f = it.next().unwrap(); f.y = 99; (t + f.y) * 1000 + sum(s) }
#[inline(never)] fn l_iter_map(s: &[P]) -> u32 { let mut t = 0; for mut p in s.iter().map(|p| *p) { p.x += 50; t += p.x; } t * 1000 + sum(s) }
#[inline(never)] fn l_iter_rev(s: &[P]) -> u32 { let mut t = 0; for mut p in s.iter().rev().cloned() { p.x += 50; t = t * 2 + p.x; } t * 1000 + sum(s) }
#[inline(never)] fn l_from_fn(s: &[P]) -> u32 { let mut a: [P; 3] = core::array::from_fn(|i| s[i]); a[bb(1)].x = 99; sum(&a) * 1000 + sum(s) }
#[inline(never)] fn l_arr_map(s: &[P; 3]) -> u32 { let mut a = s.map(|p| p); a[bb(1)].x = 99; let mut b = s.map(|mut p| { p.y += 1; p }); b[0].x = 98; (sum(&a) + sum(&b)) * 1000 + sum(s) }
#[inline(never)] fn l_arr_into_iter(s: &[P; 3]) -> u32 { let mut t = 0; for mut p in s.into_iter().copied() { p.x += 50; t += p.x; } for mut p in *s { p.y += 50; t += p.y; } t * 1000 + sum(s) }
#[inline(never)] fn l_zip(s: &[P], a: &[[u32; 3]]) -> u32 { let mut t = 0; for (mut p, mut r) in s.iter().copied().zip(a.iter().copied()) { p.x += r[0]; r[1] += p.y; t += p.x + r[1]; } t * 1000 + sum(s) + suma(&a[1]) }
#[inline(never)] fn l_try_into(s: &[P]) -> u32 { let mut a: [P; 2] = s[..2].try_into().unwrap(); a[bb(1)].x = 99; sum(&a) * 1000 + sum(s) }
#[inline(never)] fn l_try_into_ref(s: &[P]) -> u32 { let r: &[P; 2] = s[1..].try_into().unwrap(); let mut a = *r; a[bb(1)].x = 99; sum(&a) * 1000 + sum(s) }
#[inline(never)] fn l_split_first(s: &[P]) -> u32 { let (f, rest) = s.split_first().unwrap(); let mut c = *f; c.x = 99; let mut d = rest[bb(1)]; d.y = 98; (c.x + d.y) * 1000 + sum(s) }
#[inline(never)] fn l_chunks(a: &[u32]) -> u32 { let mut t = 0; for ch in a.chunks(2) { let mut c: [u32; 2] = [ch[0], *ch.get(1).unwrap_or(&0)]; c[0] += 10; t += c[0] + c[1]; } t * 1000 + suma(a) }
#[inline(never)] fn l_option_copied(s: &[P]) -> u32 { let o: Option<&P> = s.get(bb(1)); let mut c = o.copied(); if let Some(p) = &mut c { p.x = 99; } c.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn l_option_map(s: &[P]) -> u32 { let c = s.get(bb(1)).map(|p| { let mut q = *p; q.x = 99; q }); c.unwrap().x * 1000 + s[1].x }
#[inline(never)] fn l_unwrap_or(s: &[P]) -> u32 { let mut c = s.get(bb(7)).copied().unwrap_or(s[bb(1)]); c.x = 99; c.x * 1000 + s[1].x }
#[inline(never)] fn l_box_clone(s: &[P]) -> u32 { let b = Box::new(s[bb(1)]); let mut c = b.clone(); c.x = 99; let mut d = *b.clone(); d.y = 98; (c.x + d.y + b.x + b.y) * 1000 + s[1].x }
#[inline(never)] fn l_vec_pop(s: &[P]) -> u32 { let mut v = s.to_vec(); let mut c = v.pop().unwrap(); c.x = 99; let mut d = v[bb(0)]; d.y = 98; (c.x + d.y) * 1000 + sum(&v) + sum(s) }
#[inline(never)] fn l_vec_insert(s: &[P]) -> u32 { let mut v = Vec::new(); v.push(s[bb(0)]); v.insert(0, s[bb(1)]); v[0].x = 99; let mut c = v.remove(1); c.y = 98; (sum(&v) + c.y) * 1000 + sum(s) }
#[inline(never)] fn l_vec_resize(p: P) -> u32 { let mut v = Vec::new(); v.resize(3, p); v[1].x = 99; let w = alloc::vec![p; 2]; let mut w2 = w; w2[0].y = 98; (sum(&v) + sum(&w2)) * 1000 + p.x + p.y }
#[inline(never)] fn l_mem_copy(s: &[P]) -> u32 { let mut c = mem::replace(&mut { s[bb(1)] }, P { x: 0, y: 0 }); c.x = 99; let mut d = mem::take(&mut { AA[bb(1)] }); d[1] = 98; (c.x + d[1]) * 1000 + s[1].x + AA[1][1] }

// rule B: control flow around writes into no-clone locals, other transparent payloads
#[inline(never)] fn b_match(a: W, k: u32) -> u32 { let mut w = a; match k { 0 => w.0 = 1, 1 => w.0 += 5, _ => {} } w.0 * 1000 + a.0 }
#[inline(never)] fn b_nested_loop(a: W, n: u32) -> u32 { let mut w = a; let mut t = 0; for i in 0..n { let before = w; for j in 0..i { w.0 += j; if w.0 > 20 { break; } } t += before.0 * 10 + w.0; if i == 1 { continue; } w.0 += 1; } t * 1000 + a.0 }
#[inline(never)] fn b_early(a: W, c: bool) -> u32 { let mut w = a; w.0 += 1; if c { return w.0 * 1000 + a.0; } w.0 += 1; let x = w; w.0 += 1; x.0 * 10000 + w.0 * 100 + a.0 }
#[inline(never)] fn b_while(a: W) -> u32 { let mut w = a; let mut prev = w; while w.0 < 20 { prev = w; w.0 += 4; } prev.0 * 10000 + w.0 * 100 + a.0 }
#[inline(never)] fn b_closure(a: W) -> u32 { let mut w = a; let mut f = |d: u32| { w.0 += d; w.0 }; let r = f(1) + f(2); r * 10000 + w.0 * 100 + a.0 }
#[inline(never)] fn b_bool(a: WB) -> u32 { let mut b = a; b.0 = !b.0; (b.0 as u32) * 10 + a.0 as u32 }
#[inline(never)] fn b_f64(a: WF) -> u32 { let mut b = a; b.0 += 1.5; let c = b; b.0 *= 2.0; (b.0 as u32) * 10000 + (c.0 as u32) * 100 + a.0 as u32 }
#[inline(never)] fn b_u64(a: W64) -> u32 { let mut b = a; b.0 += 1 << 40; let c = b; b.0 >>= 39; (b.0 as u32) * 10000 + ((c.0 >> 40) as u32) * 100 + a.0 as u32 }
#[inline(never)] fn b_char(a: WC) -> u32 { let mut b = a; b.0 = 'z'; (b.0 as u32) * 1000 + a.0 as u32 }
#[inline(never)] fn b_i8(a: WI8) -> u32 { let mut b = a; b.0 = b.0.wrapping_add(100); let c = b; b.0 = -1; ((b.0 as i32 + 2) as u32) * 10000 + ((c.0 as i32 + 200) as u32) * 100 + a.0 as u32 }
#[inline(never)] fn b_fn(a: WFn) -> u32 { let mut b = a; b.0 = dbl; b.0(5) * 100 + a.0(5) }
#[inline(never)] fn b_ptr(a: WPtr, o: &u32) -> u32 { let mut b = a; b.0 = o; unsafe { *b.0 * 100 + *a.0 } }
#[inline(never)] fn b_outer(a: Outer) -> u32 { let mut b = a; b.0.0 = 99; let mut c = a; c.0 = W(98); b.0.0 * 10000 + c.0.0 * 100 + a.0.0 }
#[inline(never)] fn b_outer2(a: Outer2) -> u32 { let mut b = a; b.0.0.0 = 99; let mut c = b; c.0.0 = W(98); let mut d = c; d.0 = Outer(W(97)); b.0.0.0 * 1000000 + c.0.0.0 * 10000 + d.0.0.0 * 100 + a.0.0.0 }
#[inline(never)] fn b_prim(a: WPrim) -> u32 { let mut b = a; b.0 = Prim::C; let c = b; b.0 = Prim::A; (b.0 as u32) * 100 + (c.0 as u32) * 10 + a.0 as u32 }
#[inline(never)] fn b_prim_plain(a: Prim) -> u32 { let mut b = a; b = Prim::C; let c = b; b = Prim::A; (b as u32) * 100 + (c as u32) * 10 + a as u32 }
#[inline(never)] fn b_in_struct(h: HW) -> u32 { let mut g = h; g.w.0 = 99; let mut k = g; k.w.0 += 1; k.pad = 3; g.w.0 * 10000 + k.w.0 * 100 + h.w.0 + k.pad + g.pad }
#[inline(never)] fn b_ret(a: W) -> W { let mut b = a; b.0 += 1; b }
#[inline(never)] fn b_ret_use(a: W) -> u32 { let b = b_ret(a); let c = b_ret(b); c.0 * 10000 + b.0 * 100 + a.0 }
#[inline(never)] fn b_arg_mut(mut a: W, k: u32) -> u32 { let o = a; a.0 += k; let p = a; a.0 += k; o.0 * 10000 + p.0 * 100 + a.0 }
#[inline(never)] fn b_two_params(mut a: W, mut b: W) -> u32 { let x = a; a.0 = b.0; b.0 = x.0; a.0 * 10000 + b.0 * 100 + x.0 }
#[inline(never)] fn b_swap_manual(s: &[W]) -> u32 { let mut a = s[bb(0)]; let mut b = s[bb(1)]; let t = a; a.0 = b.0; b.0 = t.0; a.0 * 10000 + b.0 * 100 + s[0].0 }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let mut ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let mut aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let mut n1 = bb(N { a: 4, inner: P { x: 5, y: 6 } });
        let cps = [Cell::new(P { x: 1, y: 2 }), Cell::new(P { x: 3, y: 4 })];
        let us = bb([13u32, 14, 15, 16, 17]);
        let mut p78 = bb(P { x: 7, y: 8 });
        let k5 = bb(5u32);
        let k6 = bb(6u32);
        t!("snap_mut", snap_mut(&mut ps.clone()));
        t!("snap_callee", snap_callee(&mut ps.clone()));
        t!("snap_callee_a", snap_callee_a(&mut ps.clone()));
        t!("snap_cell", snap_cell(&cps));
        t!("snap_raw", snap_raw(&mut p78));
        t!("snap_field", snap_field(&mut n1));
        t!("snap_whole", snap_whole(&mut n1));
        t!("snap_w", snap_w(&mut bb(W(5))));
        t!("snap_arr", snap_arr(&mut aa.clone()));
        t!("snap_swap", snap_swap(&mut ps.clone()));
        t!("snap_loop", snap_loop(&mut ps.clone()));
        t!("snap_local", snap_local(bb(3)));
        t!("snap_local_w", snap_local_w(bb(3)));
        t!("snap_ret", { let mut q = ps; let r = snap_ret(&mut q); r.x * 1000 + q[1].x });
        t!("snap_agg", { let mut q = ps; let r = snap_agg(&mut q); r.inner.x * 1000 + q[1].x });
        t!("snap_arg", snap_arg(&mut ps.clone()));
        t!("snap_vec", snap_vec(&mut ps.to_vec()));
        t!("snap_cellget", snap_cellget(&Cell::new(bb(5))));
        t!("snap_cell_struct", snap_cell_struct(&Cell::new(bb(5)), &Cell::new(bb(6))));
        t!("snap_tuple", snap_tuple(&mut ps.clone()));
        t!("snap_arr_of", snap_arr_of(&mut ps.clone()));
        t!("snap_opt", snap_opt(&mut ps.clone()));
        t!("snap_closure", snap_closure(&mut ps.clone()));
        t!("snap_static_mut", snap_static_mut());
        let s = &ps[..];
        t!("l_to_vec", l_to_vec(s));
        t!("l_extend", l_extend(s));
        t!("l_vec_clone", l_vec_clone(s));
        t!("l_first_copied", l_first_copied(s));
        t!("l_last_deref", l_last_deref(s));
        t!("l_get_cloned", l_get_cloned(s));
        t!("l_iter_copied", l_iter_copied(s));
        t!("l_iter_map", l_iter_map(s));
        t!("l_iter_rev", l_iter_rev(s));
        t!("l_from_fn", l_from_fn(s));
        t!("l_arr_map", l_arr_map(&ps));
        t!("l_arr_into_iter", l_arr_into_iter(&ps));
        t!("l_zip", l_zip(s, &aa));
        t!("l_try_into", l_try_into(s));
        t!("l_try_into_ref", l_try_into_ref(s));
        t!("l_split_first", l_split_first(s));
        t!("l_chunks", l_chunks(&us));
        t!("l_option_copied", l_option_copied(s));
        t!("l_option_map", l_option_map(s));
        t!("l_unwrap_or", l_unwrap_or(s));
        t!("l_box_clone", l_box_clone(s));
        t!("l_vec_pop", l_vec_pop(s));
        t!("l_vec_insert", l_vec_insert(s));
        t!("l_vec_resize", l_vec_resize(ps[1]));
        t!("l_mem_copy", l_mem_copy(s));
        t!("b_match0", b_match(bb(W(5)), bb(0)));
        t!("b_match1", b_match(bb(W(5)), bb(1)));
        t!("b_match2", b_match(bb(W(5)), bb(2)));
        t!("b_nested_loop", b_nested_loop(bb(W(5)), bb(4)));
        t!("b_early", b_early(bb(W(5)), bb(true)));
        t!("b_early2", b_early(bb(W(5)), bb(false)));
        t!("b_while", b_while(bb(W(5))));
        t!("b_closure", b_closure(bb(W(5))));
        t!("b_bool", b_bool(bb(WB(true))));
        t!("b_f64", b_f64(bb(WF(3.0))));
        t!("b_u64", b_u64(bb(W64(7))));
        t!("b_char", b_char(bb(WC('a'))));
        t!("b_i8", b_i8(bb(WI8(7))));
        t!("b_fn", b_fn(bb(WFn(inc))));
        t!("b_ptr", b_ptr(WPtr(&k5), &k6));
        t!("b_outer", b_outer(bb(Outer(W(5)))));
        t!("b_outer2", b_outer2(bb(Outer2(Outer(W(5))))));
        t!("b_prim", b_prim(bb(WPrim(Prim::B))));
        t!("b_prim_plain", b_prim_plain(bb(Prim::B)));
        t!("b_in_struct", b_in_struct(bb(HW { w: W(5), pad: 1 })));
        t!("b_ret_use", b_ret_use(bb(W(5))));
        t!("b_arg_mut", b_arg_mut(bb(W(5)), bb(2)));
        t!("b_two_params", b_two_params(bb(W(5)), bb(W(6))));
        t!("b_swap_manual", b_swap_manual(&ws));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + n1.inner.x + cps[1].get().x + suma(&us) + p78.x + k5 + k6);
    }
}
