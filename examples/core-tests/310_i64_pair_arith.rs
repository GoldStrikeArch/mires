//! Comprehensive 64-bit integer arithmetic, shifts, rotates, comparisons, overflow,
//! saturating, bit-intrinsics, and internal ABI direct-call checks for the `i64`/`u64`
//! Int32-pair JIR lowering.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn direct_u64_mix(a: u64, b: u64, k: u32) -> u64 {
    let x = a.wrapping_mul(0x9e37_79b9_7f4a_7c15u64);
    let y = b.rotate_left(k);
    (x ^ y).wrapping_add(a >> (k & 63))
}

#[inline(never)]
fn direct_i64_step(a: i64, b: i64, s: u32) -> i64 {
    let sh = (s & 63) as u32;
    let q = a.wrapping_div(b | 1);
    let r = a.wrapping_rem(b | 1);
    q.wrapping_add(r).wrapping_sub(a >> sh)
}

#[inline(never)]
fn loop_phi_i64(mut acc: i64, n: u32) -> i64 {
    let mut step = 0x0123_4567_89ab_cdefi64;
    let mut i = 0u32;
    while i < n {
        match i & 3 {
            0 => acc = acc.wrapping_add(step),
            1 => acc = acc.wrapping_sub(step >> 17),
            2 => acc ^= step.rotate_right(i + 5),
            _ => acc = acc.wrapping_mul(3).wrapping_add(i as i64),
        }
        step = step.rotate_left(7).wrapping_add(0x1_0000_0001i64);
        i += 1;
    }
    acc
}

#[inline(never)]
fn bump_addr_taken(p: &mut u64, k: u64) {
    *p = p.wrapping_mul(3).wrapping_add(k);
}

#[inline(never)]
fn callee_addr_taken(mut x: u64, y: u64) -> u64 {
    bump_addr_taken(&mut x, y);
    bump_addr_taken(&mut x, y ^ 7);
    bump_addr_taken(&mut x, y.rotate_left(5));
    x.wrapping_add(y)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Add / Sub / Mul across 32-bit boundary + carries / borrows
    let a = 0x1234_5678_9abc_def0u64;
    let b = 0xfedc_ba98_7654_3211u64;
    print_u64(a.wrapping_add(b));
    print_u64(a.wrapping_sub(b));
    print_u64(a.wrapping_mul(b));
    print_u64(0xffff_ffffu64.wrapping_add(1));
    print_u64(0x1_0000_0000u64.wrapping_sub(1));
    print_u64(0x1_0000_0000u64.wrapping_mul(0x1_0000_0000u64));
    print_i64((-1i64).wrapping_mul(i64::MIN));
    print_i64(i64::MIN / 2);
    print_i64((-0x7fff_ffff_ffff_ffffi64) / 2);

    // 2. Div / Rem including i64::MIN / -1 wrapping semantics & large u64 divisors
    print_i64(i64::MIN.wrapping_div(-1));
    print_i64(i64::MIN.wrapping_rem(-1));
    print_i64((-123_456_789_012_345i64) / 67_890_123_456i64);
    print_i64((-123_456_789_012_345i64) % 67_890_123_456i64);
    print_u64(u64::MAX / 0x1_0000_0001u64);
    print_u64(u64::MAX % 0x1_0000_0001u64);
    print_u64(0x8000_0000_0000_0005u64 / 3u64);
    print_u64(0x8000_0000_0000_0005u64 % 3u64);

    // 3. Shifts (0, 1, 31, 32, 33, 63) & Rotates (runtime shift amounts, not constants!)
    let mut s = 0u32;
    let vals_u = [0x8000_0001_8000_0001u64, 0x0123_4567_89ab_cdefu64];
    let shifts = [0u32, 1, 31, 32, 33, 63, 64, 65];
    while s < shifts.len() as u32 {
        let sh = shifts[s as usize];
        let vu = vals_u[(s & 1) as usize];
        let vi = vu as i64;
        print_u64(vu.wrapping_shl(sh));
        print_u64(vu.wrapping_shr(sh));
        print_i64(vi.wrapping_shr(sh));
        print_u64(vu.rotate_left(sh));
        print_u64(vu.rotate_right(sh));
        s += 1;
    }

    // 4. Signed and unsigned comparisons across 32-bit sign boundary (0x7fff_ffff vs 0x8000_0000 lo)
    let p1 = 0x0000_0001_7fff_ffffu64;
    let p2 = 0x0000_0001_8000_0000u64;
    print_bool(p1 < p2);
    print_bool((p1 as i64) < (p2 as i64));
    print_bool((i64::MIN as u64) > (i64::MAX as u64));
    print_bool(i64::MIN < i64::MAX);
    print_i32((0x8000_0000_0000_0000u64).cmp(&0x7fff_ffff_ffff_ffffu64) as i32);
    print_i32((i64::MIN).cmp(&i64::MAX) as i32);
    print_i32((42i64).cmp(&42i64) as i32);

    // 5. Overflowing & Saturating
    let (v1, o1) = i64::MAX.overflowing_add(1);
    print_i64(v1);
    print_bool(o1);
    let (v2, o2) = i64::MIN.overflowing_sub(1);
    print_i64(v2);
    print_bool(o2);
    let (v3, o3) = 0x1_0000_0000u64.overflowing_mul(0x1_0000_0000u64);
    print_u64(v3);
    print_bool(o3);
    let (v4, o4) = 0x1_0000_0000i64.overflowing_mul(0x1_0000_0000i64);
    print_i64(v4);
    print_bool(o4);
    let (v5, o5) = i64::MIN.overflowing_mul(1);
    print_i64(v5);
    print_bool(o5);
    let (v6, o6) = i64::MIN.overflowing_mul(-1);
    print_i64(v6);
    print_bool(o6);
    print_u64(u64::MAX.saturating_add(10));
    print_u64(3u64.saturating_sub(10));
    print_i64(i64::MIN.saturating_sub(10));
    print_i64(i64::MAX.saturating_mul(-2));

    // 6. Bit intrinsics
    let bits = 0x0080_0000_0000_0040u64;
    print_u32(bits.leading_zeros());
    print_u32(bits.trailing_zeros());
    print_u32(0u64.leading_zeros());
    print_u32(0u64.trailing_zeros());
    print_u32(0xffff_ffff_0000_0001u64.leading_zeros());
    print_u32(0x8000_0000_0000_0000u64.trailing_zeros());
    print_u32(0xf0f0_0f0f_aaaa_5555u64.count_ones());
    print_u64(0x0123_4567_89ab_cdefu64.swap_bytes());
    print_u64(0x0123_4567_89ab_cdefu64.reverse_bits());

    // 7. Direct internal ABI calls + loop phi switch + address-taken FlatAbi u64 parameter
    print_u64(direct_u64_mix(a, b, 19));
    print_i64(direct_i64_step(-987_654_321_098i64, 123_456i64, 33));
    print_i64(loop_phi_i64(0x1122_3344_5566_7788i64, 17));
    let r1 = callee_addr_taken(a.wrapping_add(1), 5);
    print_u64(callee_addr_taken(r1, 9));
}
