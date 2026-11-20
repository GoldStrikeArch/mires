//! Constrained Delaunay Lawson edge-flipping refinement (`refine` from `earcut@3.2.4`).
//!
//! Uses zero-based sentinels (`0` = boundary / empty slot, `1` = consumed hash slot, `e + 1` /
//! `e + 2` for live half-edge indices) so `vec![0u32; ...]` allocates zeroed buffers without
//! hitting the non-zero `alloc::vec::from_elem` per-element pointer-allocation loop under mires.

use alloc::vec;

macro_rules! next_he {
    ($e:expr) => {{
        let e = $e;
        if e % 3 == 2 { e - 2 } else { e + 1 }
    }};
}

pub fn refine(triangles: &mut [u32], coords: &[f64], dim: usize) {
    let n = triangles.len();
    if n < 6 {
        return;
    }

    let mut size = 1usize;
    while size < n * 4 {
        size <<= 1;
    }
    let h_mask = size - 1;

    // `he[e] == 0` means boundary (-1); `he[e] == s + 1` means twin half-edge `s`.
    let mut he = vec![0u32; n];
    let mut edge_stack = vec![0usize; n];
    let mut edge_stamp = vec![0u8; n];
    // `h_table[h] == 0` means empty; `1` means consumed (-1); `s + 2` means live half-edge `s`.
    let mut h_table = vec![0u32; size];

    let he_sl = he.as_mut_slice();
    let stack_sl = edge_stack.as_mut_slice();
    let stamp_sl = edge_stamp.as_mut_slice();
    let ht_sl = h_table.as_mut_slice();

    let mut stack_len = 0usize;
    let mut e = 0usize;
    while e < n {
        let a = triangles[e];
        let b = triangles[next_he!(e)];
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let mut h = (lo.wrapping_mul(0x9e37_79b1) ^ hi.wrapping_mul(0x85eb_ca6b)) as usize & h_mask;

        while ht_sl[h] != 0 {
            let slot = ht_sl[h];
            if slot != 1 {
                let su = (slot - 2) as usize;
                let sa = triangles[su];
                let sb = triangles[next_he!(su)];
                if (sa == lo && sb == hi) || (sa == hi && sb == lo) {
                    he_sl[e] = (su + 1) as u32;
                    he_sl[su] = (e + 1) as u32;
                    ht_sl[h] = 1;
                    stamp_sl[su] = 1;
                    stack_sl[stack_len] = su;
                    stack_len += 1;
                    break;
                }
            }
            h = (h + 1) & h_mask;
        }
        if ht_sl[h] == 0 {
            ht_sl[h] = (e + 2) as u32;
        }
        e += 1;
    }

    while stack_len > 0 {
        stack_len -= 1;
        let a = stack_sl[stack_len];
        stamp_sl[a] = 0;
        let b_enc = he_sl[a];
        if b_enc == 0 {
            continue;
        }
        let b = (b_enc - 1) as usize;

        let a_rem = a % 3;
        let a0 = a - a_rem;
        let ar = if a_rem == 0 { a0 + 2 } else { a - 1 };
        let al = if a_rem == 2 { a0 } else { a + 1 };

        let b_rem = b % 3;
        let b0 = b - b_rem;
        let bl = if b_rem == 0 { b0 + 2 } else { b - 1 };
        let br = if b_rem == 2 { b0 } else { b + 1 };

        let p0 = triangles[ar];
        let pr = triangles[a];
        let pl = triangles[al];
        let p1 = triangles[bl];

        let o0 = (p0 as usize) * dim;
        let or = (pr as usize) * dim;
        let ol = (pl as usize) * dim;
        let o1 = (p1 as usize) * dim;

        let x0 = coords[o0];
        let y0 = coords[o0 + 1];
        let xr = coords[or];
        let yr = coords[or + 1];
        let xl = coords[ol];
        let yl = coords[ol + 1];
        let x1 = coords[o1];
        let y1 = coords[o1 + 1];

        let dx = x0 - x1;
        let dy = y0 - y1;
        let ex = xr - x1;
        let ey = yr - y1;
        let fx = xl - x1;
        let fy = yl - y1;
        let ap = dx * dx + dy * dy;
        let bp = ex * ex + ey * ey;
        let cp = fx * fx + fy * fy;
        let s = ap + bp + cp;
        let in_circ = (dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx))
            + ap * (ex * fy - ey * fx)
            <= (1e-13 * s) * s;

        if !in_circ
            && (xr - x0) * (y1 - y0) - (yr - y0) * (x1 - x0) > 0.0
            && (x1 - x0) * (yl - y0) - (y1 - y0) * (xl - x0) > 0.0
        {
            triangles[a] = p1;
            triangles[b] = p0;

            let hbl = he_sl[bl];
            let har = he_sl[ar];
            he_sl[a] = hbl;
            if hbl != 0 {
                he_sl[(hbl - 1) as usize] = (a + 1) as u32;
            }
            he_sl[b] = har;
            if har != 0 {
                he_sl[(har - 1) as usize] = (b + 1) as u32;
            }
            he_sl[ar] = (bl + 1) as u32;
            he_sl[bl] = (ar + 1) as u32;

            if hbl != 0 && stamp_sl[a] == 0 {
                stamp_sl[a] = 1;
                stack_sl[stack_len] = a;
                stack_len += 1;
            }
            if har != 0 && stamp_sl[b] == 0 {
                stamp_sl[b] = 1;
                stack_sl[stack_len] = b;
                stack_len += 1;
            }
            if he_sl[al] != 0 && stamp_sl[al] == 0 {
                stamp_sl[al] = 1;
                stack_sl[stack_len] = al;
                stack_len += 1;
            }
            if he_sl[br] != 0 && stamp_sl[br] == 0 {
                stamp_sl[br] = 1;
                stack_sl[stack_len] = br;
                stack_len += 1;
            }
        }
    }
}
