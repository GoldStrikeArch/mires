// Fresh aggregates borrowed whole and handed on (the `f(&[a, b])` shape), with callees that write
// through the shared view. For the fix, and for the narrower variants measured for point 5.
struct C { a: Cell<u32>, b: u32 }
#[inline(never)] fn set0(c: &[Cell<u32>]) -> u32 { c[0].set(99); c[0].get() }
#[inline(always)] fn set0_a(c: &[Cell<u32>]) -> u32 { c[0].set(99); c[0].get() }
#[inline(never)] fn set_arr(c: &[Cell<u32>; 2]) -> u32 { c[1].set(98); c[1].get() }
#[inline(always)] fn set_arr_a(c: &[Cell<u32>; 2]) -> u32 { c[1].set(98); c[1].get() }
#[inline(never)] fn set_c(c: &C) -> u32 { c.a.set(97); c.a.get() + c.b }
#[inline(always)] fn set_c_a(c: &C) -> u32 { c.a.set(97); c.a.get() + c.b }
#[inline(never)] fn set_tuple(c: &(Cell<u32>, u32)) -> u32 { c.0.set(96); c.0.get() + c.1 }
#[inline(always)] fn set_tuple_a(c: &(Cell<u32>, u32)) -> u32 { c.0.set(96); c.0.get() + c.1 }
#[inline(never)] fn wr_slice(a: &mut [u32]) -> u32 { a[0] = 95; a[0] + a[1] }
#[inline(always)] fn wr_slice_a(a: &mut [u32]) -> u32 { a[0] = 95; a[0] + a[1] }
#[inline(never)] fn wr_ps(a: &mut [P]) -> u32 { a[0].x = 94; a[0].x + a[1].y }
#[inline(always)] fn wr_ps_a(a: &mut [P]) -> u32 { a[0].x = 94; a[0].x + a[1].y }
#[inline(never)] fn rd_slice(a: &[u32]) -> u32 { a[0] * 10 + a[1] }
#[inline(always)] fn rd_slice_a(a: &[u32]) -> u32 { a[0] * 10 + a[1] }
#[inline(never)] fn pass(c: &[Cell<u32>]) -> u32 { set0(c) + set0_a(c) }
#[inline(always)] fn pass_a(c: &[Cell<u32>]) -> u32 { set0(c) + set0_a(c) }
#[inline(never)] fn ret_ref(c: &C) -> &Cell<u32> { &c.a }

#[inline(never)] fn g_cells(n: u32, m: u32) -> u32 { set0(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_cells_a(n: u32, m: u32) -> u32 { set0_a(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_cells_slice(s: &[u32]) -> u32 { set0(&[Cell::new(s[bb(1)]), Cell::new(s[2])]) * 1000 + s[1] }
#[inline(never)] fn g_cells_slice_a(s: &[u32]) -> u32 { set0_a(&[Cell::new(s[bb(1)]), Cell::new(s[2])]) * 1000 + s[1] }
#[inline(never)] fn g_arr(n: u32, m: u32) -> u32 { set_arr(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_arr_a(n: u32, m: u32) -> u32 { set_arr_a(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_c(n: u32, m: u32) -> u32 { set_c(&C { a: Cell::new(n), b: m }) * 10000 + n * 100 + m }
#[inline(never)] fn g_c_a(n: u32, m: u32) -> u32 { set_c_a(&C { a: Cell::new(n), b: m }) * 10000 + n * 100 + m }
#[inline(never)] fn g_c_slice(s: &[u32]) -> u32 { set_c(&C { a: Cell::new(s[bb(1)]), b: s[2] }) * 1000 + s[1] }
#[inline(never)] fn g_c_slice_a(s: &[u32]) -> u32 { set_c_a(&C { a: Cell::new(s[bb(1)]), b: s[2] }) * 1000 + s[1] }
#[inline(never)] fn g_tuple(n: u32, m: u32) -> u32 { set_tuple(&(Cell::new(n), m)) * 10000 + n * 100 + m }
#[inline(never)] fn g_tuple_a(n: u32, m: u32) -> u32 { set_tuple_a(&(Cell::new(n), m)) * 10000 + n * 100 + m }
#[inline(never)] fn g_mut_slice(n: u32, m: u32) -> u32 { wr_slice(&mut [n, m]) * 10000 + n * 100 + m }
#[inline(never)] fn g_mut_slice_a(n: u32, m: u32) -> u32 { wr_slice_a(&mut [n, m]) * 10000 + n * 100 + m }
#[inline(never)] fn g_mut_slice_copy(a: &[[u32; 3]]) -> u32 { wr_slice(&mut { a[bb(1)] }) * 1000 + a[1][0] }
#[inline(never)] fn g_mut_slice_copy_a(a: &[[u32; 3]]) -> u32 { wr_slice_a(&mut { a[bb(1)] }) * 1000 + a[1][0] }
#[inline(never)] fn g_mut_ps(s: &[P]) -> u32 { wr_ps(&mut [s[bb(0)], s[1]]) * 1000 + s[0].x }
#[inline(never)] fn g_mut_ps_a(s: &[P]) -> u32 { wr_ps_a(&mut [s[bb(0)], s[1]]) * 1000 + s[0].x }
#[inline(never)] fn g_mut_ps_copy(s: &[P; 3]) -> u32 { wr_ps(&mut { *s }) * 1000 + s[0].x }
#[inline(never)] fn g_mut_ps_copy_a(s: &[P; 3]) -> u32 { wr_ps_a(&mut { *s }) * 1000 + s[0].x }
#[inline(never)] fn g_rd(n: u32, m: u32) -> u32 { rd_slice(&[n, m]) * 100 + rd_slice_a(&[m, n]) }
#[inline(never)] fn g_pass(n: u32, m: u32) -> u32 { pass(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_pass_a(n: u32, m: u32) -> u32 { pass_a(&[Cell::new(n), Cell::new(m)]) * 10000 + n * 100 + m }
#[inline(never)] fn g_two_uses(n: u32, m: u32) -> u32 { let r = &[Cell::new(n), Cell::new(m)]; (set0(r) + set_arr_a(r)) * 10000 + n * 100 + m }
#[inline(never)] fn g_reborrow(n: u32, m: u32) -> u32 { let r = &C { a: Cell::new(n), b: m }; let q = &*r; let p = &q.a; p.set(93); p.get() * 10000 + n * 100 + m }
#[inline(never)] fn g_reborrow_call(n: u32, m: u32) -> u32 { let r = &C { a: Cell::new(n), b: m }; let q = &*r; set_c(q) * 10000 + n * 100 + m }
#[inline(never)] fn g_ret_ref(n: u32, m: u32) -> u32 { let q = C { a: Cell::new(n), b: m }; let c = ret_ref(&q); c.set(92); c.get() * 10000 + n * 100 + m }
#[inline(never)] fn g_loop(s: &[u32]) -> u32 { let mut t = 0; for i in 0..s.len() { t += set0(&[Cell::new(s[i]), Cell::new(i as u32)]) + wr_slice_a(&mut [s[i], 1]) + set_c_a(&C { a: Cell::new(s[i]), b: 2 }); } t * 1000 + suma(s) }
#[inline(never)] fn g_raw(n: u32, m: u32) -> u32 { let p = &raw const *&[n, m] as *mut [u32; 2]; unsafe { (*p)[0] = 91; (*p)[0] * 10000 + n * 100 + m } }
#[inline(never)] fn g_const_arg() -> u32 { wr_slice(&mut [1, 2]) * 100 + wr_slice_a(&mut [1, 2]) + set0(&[Cell::new(5)]) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let us = bb([13u32, 14, 15]);
        t!("g_cells", g_cells(bb(3), bb(4)));
        t!("g_cells_a", g_cells_a(bb(3), bb(4)));
        t!("g_cells_slice", g_cells_slice(&us));
        t!("g_cells_slice_a", g_cells_slice_a(&us));
        t!("g_arr", g_arr(bb(3), bb(4)));
        t!("g_arr_a", g_arr_a(bb(3), bb(4)));
        t!("g_c", g_c(bb(3), bb(4)));
        t!("g_c_a", g_c_a(bb(3), bb(4)));
        t!("g_c_slice", g_c_slice(&us));
        t!("g_c_slice_a", g_c_slice_a(&us));
        t!("g_tuple", g_tuple(bb(3), bb(4)));
        t!("g_tuple_a", g_tuple_a(bb(3), bb(4)));
        t!("g_mut_slice", g_mut_slice(bb(3), bb(4)));
        t!("g_mut_slice_a", g_mut_slice_a(bb(3), bb(4)));
        t!("g_mut_slice_copy", g_mut_slice_copy(&aa));
        t!("g_mut_slice_copy_a", g_mut_slice_copy_a(&aa));
        t!("g_mut_ps", g_mut_ps(s));
        t!("g_mut_ps_a", g_mut_ps_a(s));
        t!("g_mut_ps_copy", g_mut_ps_copy(&ps));
        t!("g_mut_ps_copy_a", g_mut_ps_copy_a(&ps));
        t!("g_rd", g_rd(bb(3), bb(4)));
        t!("g_pass", g_pass(bb(3), bb(4)));
        t!("g_pass_a", g_pass_a(bb(3), bb(4)));
        t!("g_two_uses", g_two_uses(bb(3), bb(4)));
        t!("g_reborrow", g_reborrow(bb(3), bb(4)));
        t!("g_reborrow_call", g_reborrow_call(bb(3), bb(4)));
        t!("g_ret_ref", g_ret_ref(bb(3), bb(4)));
        t!("g_loop", g_loop(&us));
        t!("g_raw", g_raw(bb(3), bb(4)));
        t!("g_const_arg", g_const_arg());
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + suma(&us));
    }
}
