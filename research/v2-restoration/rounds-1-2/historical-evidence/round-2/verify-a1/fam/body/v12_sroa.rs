// Point 3: the alias-store repair (references that are second names) together with by-value copies
// and no-clone writes on the same locals. Structs are built on two paths, as fixtures 130/131 do.
struct C { a: Cell<u32>, b: u32 }
struct PC { x: u32, y: u32 }
#[inline(always)] fn set(p: &mut P, v: u32) { p.x = v }
#[inline(never)] fn set_n(p: &mut P, v: u32) { p.x = v }
#[inline(always)] fn setw(p: &mut W, v: u32) { p.0 = v }
fn two(c: bool, a: u32) -> P { if c { P { x: a, y: a + 1 } } else { P { x: a + 2, y: a + 3 } } }

#[inline(never)] fn sr_alias_and_copy(c: bool) -> u32 { let mut a = two(c, 1); let mut b = two(c, 5); let p = if bb(c) { &mut a } else { &mut b }; p.x = 99; let mut d = *p; d.y = 98; p.y += 1; let e = a; a.x += 1; a.x * 1000000 + a.y * 10000 + b.x * 1000 + b.y * 100 + d.x + d.y + e.x }
#[inline(never)] fn sr_copy_then_alias(c: bool) -> u32 { let mut a = two(c, 1); let mut c1 = a; let p = if bb(c) { &mut a } else { &mut c1 }; p.x = 99; a.x * 1000 + c1.x }
#[inline(never)] fn sr_w_alias(c: bool, n: u32) -> u32 { let mut a = W(n); let mut b = a; let p = if bb(c) { &mut a } else { &mut b }; p.0 = 99; a.0 * 1000 + b.0 }
#[inline(never)] fn sr_w_two_path(c: bool, n: u32) -> u32 { let mut w = if c { W(n) } else { W(n + 1) }; let v = w; w.0 += 5; let u = w; w.0 += 5; v.0 * 10000 + u.0 * 100 + w.0 }
#[inline(never)] fn sr_hw_two_path(c: bool, n: u32) -> u32 { let mut h = if c { HW { w: W(n), pad: 1 } } else { HW { w: W(n + 1), pad: 2 } }; let g = h; h.w.0 = 99; let k = h; h.w = W(98); h.pad += 1; g.w.0 * 1000000 + k.w.0 * 10000 + h.w.0 * 100 + g.pad * 10 + h.pad }
#[inline(never)] fn sr_dead_copy_two_path(s: &[P], c: bool) -> u32 { let mut a = if c { s[0] } else { s[1] }; a.x = 99; let mut b = a; b.y = 98; (a.x + a.y) * 1000 + sum(s) }
#[inline(never)] fn sr_loop_cursor_copy(c: bool) -> u32 { let mut arr = [two(c, 1), two(c, 5), two(c, 9)]; for i in 0..3 { let mut cur = arr[i]; cur.x += i as u32; let p = &mut arr[i]; p.y += cur.x; } sum(&arr) }
#[inline(never)] fn sr_moved_alias(c: bool) -> u32 { let a = if c { PC { x: 1, y: 2 } } else { PC { x: 3, y: 4 } }; let mut m = a; m.x = 99; let r = &mut m; r.y = 98; m.x * 100 + m.y }
#[inline(never)] fn sr_helper_inlined(c: bool) -> u32 { let mut a = two(c, 1); let mut b = a; set(&mut b, 99); set(&mut a, 98); let bx = b.x + 1; set_n(&mut b, bx); a.x * 1000 + b.x + a.y + b.y }
#[inline(never)] fn sr_helper_w(c: bool, n: u32) -> u32 { let mut a = if c { W(n) } else { W(n + 1) }; let mut b = a; setw(&mut b, 99); let d = b; setw(&mut a, 98); a.0 * 10000 + b.0 * 100 + d.0 }
#[inline(never)] fn sr_option_alias(c: bool) -> u32 { let mut a = two(c, 1); let o = if bb(c) { Some(&mut a) } else { None }; if let Some(r) = o { r.x = 99; } let mut d = a; d.x += 1; a.x * 1000 + d.x }
#[inline(never)] fn sr_copies_stay(c: bool) -> u32 { let a = two(c, 1); let mut b = a; b.x = 99; let mut d = b; d.y = 98; a.x * 1000000 + a.y * 10000 + b.x * 100 + b.y + d.x + d.y }
#[inline(never)] fn sr_field_alias_scalar(c: bool) -> u32 { let mut a = two(c, 1); let x = &mut a.x; *x = 99; let b = a; *(&mut a.y) = 98; a.x * 10000 + a.y * 100 + b.y }
#[inline(never)] fn sr_cell_two_path(c: bool, n: u32) -> u32 { let q = if c { C { a: Cell::new(n), b: 1 } } else { C { a: Cell::new(n + 1), b: 2 } }; q.a.set(99); let r = &q; r.a.set(r.a.get() + 1); q.a.get() * 1000 + n * 10 + q.b }
#[inline(never)] fn sr_cell_two_path_dead(c: bool, n: u32) -> u32 { let q = if c { C { a: Cell::new(n), b: 1 } } else { C { a: Cell::new(n + 1), b: 2 } }; q.a.set(99); n }
#[inline(never)] fn sr_array_two_path(c: bool, n: u32) -> u32 { let mut a = if c { [n, 2, 3] } else { [n + 1, 5, 6] }; let b = a; a[bb(1)] = 99; let d = a; a[0] = 98; suma(&a) * 1000000 + suma(&b) * 1000 + suma(&d) }
#[inline(never)] fn sr_struct_read_only(c: bool, s: &[P]) -> u32 { let a = if c { s[0] } else { s[2] }; let b = a; let n = N { a: b.x, inner: a }; n.inner.x * 100 + n.a + b.y }
#[inline(never)] fn sr_write_one_path(c: bool, s: &[P]) -> u32 { let mut a = s[bb(1)]; let b = a; if c { a.x = 99; } let d = a; if !c { a.y = 98; } a.x * 1000000 + a.y * 10000 + b.x * 1000 + d.x * 10 + d.y + sum(s) }
#[inline(never)] fn sr_swap_fields_via_copy(c: bool) -> u32 { let mut a = two(c, 1); let t = a; a.x = t.y; a.y = t.x; a.x * 100 + a.y + t.x * 10000 }
#[inline(never)] fn sr_nested_alias(c: bool) -> u32 { let mut n = if c { N { a: 1, inner: P { x: 2, y: 3 } } } else { N { a: 4, inner: P { x: 5, y: 6 } } }; let p = &mut n.inner; p.x = 99; let mut d = n.inner; d.y = 98; let q = &mut n; q.inner.y += 1; q.a = 97; n.inner.x * 1000000 + n.inner.y * 10000 + n.a * 100 + d.y - d.x }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        for c in [true, false] {
            let c = bb(c);
            t!("sr_alias_and_copy", sr_alias_and_copy(c));
            t!("sr_copy_then_alias", sr_copy_then_alias(c));
            t!("sr_w_alias", sr_w_alias(c, bb(5)));
            t!("sr_w_two_path", sr_w_two_path(c, bb(5)));
            t!("sr_hw_two_path", sr_hw_two_path(c, bb(5)));
            t!("sr_dead_copy_two_path", sr_dead_copy_two_path(s, c));
            t!("sr_loop_cursor_copy", sr_loop_cursor_copy(c));
            t!("sr_moved_alias", sr_moved_alias(c));
            t!("sr_helper_inlined", sr_helper_inlined(c));
            t!("sr_helper_w", sr_helper_w(c, bb(5)));
            t!("sr_option_alias", sr_option_alias(c));
            t!("sr_copies_stay", sr_copies_stay(c));
            t!("sr_field_alias_scalar", sr_field_alias_scalar(c));
            t!("sr_cell_two_path", sr_cell_two_path(c, bb(5)));
            t!("sr_cell_two_path_dead", sr_cell_two_path_dead(c, bb(5)));
            t!("sr_array_two_path", sr_array_two_path(c, bb(5)));
            t!("sr_struct_read_only", sr_struct_read_only(c, s));
            t!("sr_write_one_path", sr_write_one_path(c, s));
            t!("sr_swap_fields_via_copy", sr_swap_fields_via_copy(c));
            t!("sr_nested_alias", sr_nested_alias(c));
        }
        t!("after", sum(s));
    }
}
