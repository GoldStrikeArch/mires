// Shared views of fresh values with interior mutability.
struct C { a: Cell<u32>, b: u32 }
struct CP { p: Cell<P>, k: u32 }
struct CA { a: AtomicU32, k: u32 }
struct CU { a: UnsafeCell<u32>, b: u32 }
struct CC { inner: C, k: u32 }
enum EC { A(Cell<u32>), B(u32, Cell<u32>) }
#[inline(never)] fn set_never(c: &C) { c.a.set(99) }
#[inline(always)] fn set_always(c: &C) { c.a.set(99) }
#[inline(never)] fn setc_never(c: &Cell<u32>) { c.set(99) }
#[inline(always)] fn setc_always(c: &Cell<u32>) { c.set(c.get() + 90) }

#[inline(never)] fn i_cell_set(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; q.a.set(99); q.a.get() * 10000 + n * 100 + q.b }
#[inline(never)] fn i_cell_set_slice(s: &[u32]) -> u32 { let q = C { a: Cell::new(s[bb(1)]), b: s[bb(2)] }; q.a.set(99); q.a.get() * 1000 + s[1] + q.b }
#[inline(never)] fn i_cell_set_static() -> u32 { let q = C { a: Cell::new(AA[bb(1)][bb(1)]), b: 1 }; q.a.set(99); q.a.get() * 1000 + AA[1][1] }
#[inline(never)] fn i_cell_arr(n: u32, m: u32) -> u32 { let a = [Cell::new(n), Cell::new(m)]; a[bb(1)].set(99); a[1].get() * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_arr_const(n: u32, m: u32) -> u32 { let a = [Cell::new(n), Cell::new(m)]; a[1].set(99); a[0].set(98); (a[1].get() + a[0].get()) * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_tuple(n: u32, m: u32) -> u32 { let t = (Cell::new(n), m); t.0.set(99); t.0.get() * 10000 + n * 100 + t.1 }
#[inline(never)] fn i_cell_only(n: u32) -> u32 { let c = Cell::new(n); c.set(99); c.get() * 1000 + n }
#[inline(never)] fn i_cell_only_slice(s: &[u32]) -> u32 { let c = Cell::new(s[bb(1)]); c.set(99); c.get() * 1000 + s[1] }
#[inline(never)] fn i_cell_replace(n: u32) -> u32 { let c = Cell::new(n); let old = c.replace(99); (c.get() * 100 + old) * 1000 + n }
#[inline(never)] fn i_cell_swap(n: u32, m: u32) -> u32 { let a = Cell::new(n); let b = Cell::new(m); a.swap(&b); (a.get() * 10 + b.get()) * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_take(n: u32) -> u32 { let c = Cell::new(n); let old = c.take(); (c.get() * 100 + old) * 1000 + n }
#[inline(never)] fn i_cell_update(n: u32) -> u32 { let c = Cell::new(n); c.update(|v| v + 90); c.get() * 1000 + n }
#[inline(never)] fn i_cell_p_from_slice(s: &[P]) -> u32 { let c = Cell::new(s[bb(1)]); c.set(P { x: 99, ..c.get() }); c.get().x * 1000 + s[1].x }
#[inline(never)] fn i_cell_p_struct(s: &[P]) -> u32 { let q = CP { p: Cell::new(s[bb(1)]), k: 1 }; let mut v = q.p.get(); v.x = 99; q.p.set(v); (q.p.get().x + q.k) * 1000 + s[1].x }
#[inline(never)] fn i_cell_p_get_mut(s: &[P]) -> u32 { let mut q = CP { p: Cell::new(s[bb(1)]), k: 1 }; q.p.get_mut().x = 99; q.p.get().x * 1000 + s[1].x }
#[inline(never)] fn i_cell_p_as_ptr(s: &[P]) -> u32 { let q = CP { p: Cell::new(s[bb(1)]), k: 1 }; unsafe { (*q.p.as_ptr()).x = 99 }; q.p.get().x * 1000 + s[1].x }
#[inline(never)] fn i_cell_p_param(p: P) -> u32 { let q = CP { p: Cell::new(p), k: 1 }; unsafe { (*q.p.as_ptr()).x = 99 }; q.p.get().x * 1000 + p.x }
#[inline(never)] fn i_cell_in_option(n: u32) -> u32 { let o = Some(Cell::new(n)); if let Some(c) = &o { c.set(99) } o.unwrap().get() * 1000 + n }
#[inline(never)] fn i_cell_in_enum(n: u32, m: u32) -> u32 { let e = EC::B(n, Cell::new(m)); if let EC::B(_, c) = &e { c.set(99) } (match e { EC::B(a, c) => a + c.get(), EC::A(c) => c.get() }) * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_nested(n: u32, m: u32) -> u32 { let q = CC { inner: C { a: Cell::new(n), b: m }, k: n }; q.inner.a.set(99); q.inner.a.get() * 10000 + n * 100 + q.k }
#[inline(never)] fn i_cell_nested_ref(n: u32, m: u32) -> u32 { let q = CC { inner: C { a: Cell::new(n), b: m }, k: n }; let r = &q.inner; let c = &r.a; c.set(99); q.inner.a.get() * 10000 + n * 100 + q.k }
#[inline(never)] fn i_as_slice_of_cells(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let cs = Cell::from_mut(&mut c[..]).as_slice_of_cells(); cs[1].set(99); c[1] * 1000 + a[1][1] }
#[inline(never)] fn i_as_array_of_cells(a: &[[u32; 3]]) -> u32 { let c = Cell::new(a[bb(1)]); let cs: &[Cell<u32>; 3] = c.as_array_of_cells(); cs[2].set(99); c.get()[2] * 1000 + a[1][2] }
#[inline(never)] fn i_from_mut_scalar(n: u32) -> u32 { let mut v = n; let c = Cell::from_mut(&mut v); c.set(99); v * 1000 + n }
#[inline(never)] fn i_from_mut_field(n: u32, m: u32) -> u32 { let mut p = P { x: n, y: m }; let c = Cell::from_mut(&mut p.x); c.set(99); p.x * 10000 + n * 100 + m }
#[inline(never)] fn i_refcell(s: &[P]) -> u32 { let r = RefCell::new(s[bb(1)]); r.borrow_mut().x = 99; let v = r.borrow().x; v * 1000 + s[1].x }
#[inline(never)] fn i_refcell_fresh(n: u32, m: u32) -> u32 { let r = RefCell::new(P { x: n, y: m }); { let mut g = r.borrow_mut(); g.x = 99; } let v = r.borrow().x; v * 10000 + n * 100 + m }
#[inline(never)] fn i_refcell_scalar(n: u32) -> u32 { let r = RefCell::new(n); *r.borrow_mut() = 99; let v = *r.borrow(); v * 1000 + n }
#[inline(never)] fn i_refcell_into_inner(s: &[P]) -> u32 { let r = RefCell::new(s[bb(1)]); r.borrow_mut().x = 99; let mut v = r.into_inner(); v.y = 98; (v.x + v.y) * 1000 + s[1].x + s[1].y }
#[inline(never)] fn i_atomic(n: u32) -> u32 { let a = AtomicU32::new(n); a.fetch_add(5, Ordering::Relaxed); let mid = a.load(Ordering::Relaxed); a.store(99, Ordering::Relaxed); (a.load(Ordering::Relaxed) * 100 + mid) * 1000 + n }
#[inline(never)] fn i_atomic_in_struct(n: u32, m: u32) -> u32 { let q = CA { a: AtomicU32::new(n), k: m }; q.a.fetch_add(1, Ordering::Relaxed); q.a.load(Ordering::Relaxed) * 10000 + n * 100 + q.k }
#[inline(never)] fn i_atomic_swap(n: u32, m: u32) -> u32 { let q = CA { a: AtomicU32::new(n), k: m }; let old = q.a.swap(99, Ordering::Relaxed); let r = q.a.compare_exchange(99, 98, Ordering::Relaxed, Ordering::Relaxed).unwrap_or(0); (q.a.load(Ordering::Relaxed) * 100 + old + r) * 1000 + n + m }
#[inline(never)] fn i_unsafecell_struct(n: u32, m: u32) -> u32 { let u = UnsafeCell::new(P { x: n, y: m }); unsafe { (*u.get()).x = 99 }; u.into_inner().x * 10000 + n * 100 + m }
#[inline(never)] fn i_unsafecell_field(n: u32, m: u32) -> u32 { let u = CU { a: UnsafeCell::new(n), b: m }; unsafe { *u.a.get() = 99 }; (unsafe { *u.a.get() }) * 10000 + n * 100 + u.b }
#[inline(never)] fn i_unsafecell_raw_get(s: &[u32]) -> u32 { let u = CU { a: UnsafeCell::new(s[bb(1)]), b: 1 }; let p: *const UnsafeCell<u32> = &u.a; unsafe { *UnsafeCell::raw_get(p) = 99 }; u.a.into_inner() * 1000 + s[1] }
#[inline(never)] fn i_unsafecell_slice(s: &[P]) -> u32 { let u = UnsafeCell::new(s[bb(1)]); unsafe { (*u.get()).x = 99 }; let v = unsafe { *u.get() }; v.x * 1000 + s[1].x }
#[inline(never)] fn i_shared_callee_never(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; set_never(&q); q.a.get() * 10000 + n * 100 + q.b }
#[inline(never)] fn i_shared_callee_always(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; set_always(&q); q.a.get() * 10000 + n * 100 + q.b }
#[inline(never)] fn i_shared_field_never(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; setc_never(&q.a); q.a.get() * 10000 + n * 100 + q.b }
#[inline(never)] fn i_shared_field_always(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; setc_always(&q.a); q.a.get() * 10000 + n * 100 + q.b }
#[inline(never)] fn i_shared_slice(s: &[u32]) -> u32 { let q = C { a: Cell::new(s[bb(1)]), b: 2 }; setc_always(&q.a); set_never(&q); q.a.get() * 1000 + s[1] }
#[inline(never)] fn i_cell_w(w: W) -> u32 { let c = Cell::new(w); c.set(W(99)); c.get().0 * 1000 + w.0 }
#[inline(never)] fn i_cell_w_slice(ws: &[W]) -> u32 { let c = Cell::new(ws[bb(1)]); c.set(W(c.get().0 + 80)); c.get().0 * 1000 + ws[1].0 }
#[inline(never)] fn i_cell_get_mut(n: u32) -> u32 { let mut c = Cell::new(n); *c.get_mut() = 99; c.into_inner() * 1000 + n }
#[inline(never)] fn i_cell_loop(s: &[u32]) -> u32 { let mut t = 0; for &v in s { let c = Cell::new(v); c.set(c.get() + 1); t += c.get(); } t * 1000 + s[0] }
#[inline(never)] fn i_cell_loop_struct(s: &[u32]) -> u32 { let mut t = 0; for i in 0..s.len() { let q = C { a: Cell::new(s[i]), b: i as u32 }; q.a.set(q.a.get() + 1); t += q.a.get() + q.b; } t * 1000 + suma(s) }
#[inline(never)] fn i_cell_moved(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; let q2 = q; q2.a.set(99); let q3 = CC { inner: q2, k: 1 }; q3.inner.a.set(q3.inner.a.get() - 1); q3.inner.a.get() * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_ret(n: u32, m: u32) -> u32 {
    #[inline(always)] fn mk(n: u32, m: u32) -> C { C { a: Cell::new(n), b: m } }
    let q = mk(n, m); q.a.set(99); let r = mk(n, m); (q.a.get() + r.a.get()) * 10000 + n * 100 + m
}
#[inline(never)] fn i_cell_ref_copy(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; let r = &q; let r2 = r; r2.a.set(99); r.a.get() * 10000 + n * 100 + m }
#[inline(never)] fn i_cell_only_read(s: &[u32]) -> u32 { let q = C { a: Cell::new(s[bb(1)]), b: s[bb(2)] }; let r = &q; (r.a.get() + r.b) * 1000 + s[1] }
#[inline(never)] fn i_cells_of_slice(cells: &[Cell<u32>]) -> u32 { let c = &cells[bb(1)]; c.set(98); let v = cells[bb(2)].get(); cells[2].set(v + 1); cells[1].get() * 1000 + cells[2].get() }
#[inline(never)] fn i_cell_copy_out(cells: &[Cell<P>]) -> u32 { let mut v = cells[bb(1)].get(); v.x = 99; (v.x) * 1000 + cells[1].get().x }
#[inline(never)] fn i_oncecell(n: u32) -> u32 { let c = core::cell::OnceCell::new(); let _ = c.set(n); let _ = c.set(99); *c.get().unwrap() * 1000 + n }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let us = bb([13u32, 14, 15]);
        let cells = [Cell::new(1u32), Cell::new(2), Cell::new(3)];
        let cps = [Cell::new(P { x: 1, y: 2 }), Cell::new(P { x: 3, y: 4 })];
        t!("i_cell_set", i_cell_set(bb(3), bb(4)));
        t!("i_cell_set_slice", i_cell_set_slice(&us));
        t!("i_cell_set_static", i_cell_set_static());
        t!("i_cell_arr", i_cell_arr(bb(3), bb(4)));
        t!("i_cell_arr_const", i_cell_arr_const(bb(3), bb(4)));
        t!("i_cell_tuple", i_cell_tuple(bb(3), bb(4)));
        t!("i_cell_only", i_cell_only(bb(3)));
        t!("i_cell_only_slice", i_cell_only_slice(&us));
        t!("i_cell_replace", i_cell_replace(bb(3)));
        t!("i_cell_swap", i_cell_swap(bb(3), bb(4)));
        t!("i_cell_take", i_cell_take(bb(3)));
        t!("i_cell_update", i_cell_update(bb(3)));
        t!("i_cell_p_from_slice", i_cell_p_from_slice(s));
        t!("i_cell_p_struct", i_cell_p_struct(s));
        t!("i_cell_p_get_mut", i_cell_p_get_mut(s));
        t!("i_cell_p_as_ptr", i_cell_p_as_ptr(s));
        t!("i_cell_p_param", i_cell_p_param(bb(P { x: 7, y: 8 })));
        t!("i_cell_in_option", i_cell_in_option(bb(3)));
        t!("i_cell_in_enum", i_cell_in_enum(bb(3), bb(4)));
        t!("i_cell_nested", i_cell_nested(bb(3), bb(4)));
        t!("i_cell_nested_ref", i_cell_nested_ref(bb(3), bb(4)));
        t!("i_as_slice_of_cells", i_as_slice_of_cells(&aa));
        t!("i_as_array_of_cells", i_as_array_of_cells(&aa));
        t!("i_from_mut_scalar", i_from_mut_scalar(bb(3)));
        t!("i_from_mut_field", i_from_mut_field(bb(3), bb(4)));
        t!("i_refcell", i_refcell(s));
        t!("i_refcell_fresh", i_refcell_fresh(bb(3), bb(4)));
        t!("i_refcell_scalar", i_refcell_scalar(bb(3)));
        t!("i_refcell_into_inner", i_refcell_into_inner(s));
        t!("i_atomic", i_atomic(bb(3)));
        t!("i_atomic_in_struct", i_atomic_in_struct(bb(3), bb(4)));
        t!("i_atomic_swap", i_atomic_swap(bb(3), bb(4)));
        t!("i_unsafecell_struct", i_unsafecell_struct(bb(3), bb(4)));
        t!("i_unsafecell_field", i_unsafecell_field(bb(3), bb(4)));
        t!("i_unsafecell_raw_get", i_unsafecell_raw_get(&us));
        t!("i_unsafecell_slice", i_unsafecell_slice(s));
        t!("i_shared_callee_never", i_shared_callee_never(bb(3), bb(4)));
        t!("i_shared_callee_always", i_shared_callee_always(bb(3), bb(4)));
        t!("i_shared_field_never", i_shared_field_never(bb(3), bb(4)));
        t!("i_shared_field_always", i_shared_field_always(bb(3), bb(4)));
        t!("i_shared_slice", i_shared_slice(&us));
        t!("i_cell_w", i_cell_w(bb(W(5))));
        t!("i_cell_w_slice", i_cell_w_slice(&ws));
        t!("i_cell_get_mut", i_cell_get_mut(bb(3)));
        t!("i_cell_loop", i_cell_loop(&us));
        t!("i_cell_loop_struct", i_cell_loop_struct(&us));
        t!("i_cell_moved", i_cell_moved(bb(3), bb(4)));
        t!("i_cell_ret", i_cell_ret(bb(3), bb(4)));
        t!("i_cell_ref_copy", i_cell_ref_copy(bb(3), bb(4)));
        t!("i_cell_only_read", i_cell_only_read(&us));
        t!("i_cells_of_slice", i_cells_of_slice(&cells));
        t!("i_cell_copy_out", i_cell_copy_out(&cps));
        t!("i_oncecell", i_oncecell(bb(3)));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + suma(&us) + cells[1].get() + cps[1].get().x);
    }
}
