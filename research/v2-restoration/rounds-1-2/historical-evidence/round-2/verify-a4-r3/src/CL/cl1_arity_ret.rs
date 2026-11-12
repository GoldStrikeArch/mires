//! Closures of every arity / argument kind / return kind stored as fn pointers in consts and statics.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(Clone, Copy)]
struct Unit;
#[derive(Clone, Copy)]
struct P { x: u32, y: u32 }
#[derive(Clone, Copy)]
enum E { A(u32), B, C(u8, u8) }

const A0: fn() = || print_u32(100);
const A1: fn(u32) -> u32 = |a| a + 1;
const A2: fn(u32, u32) -> u32 = |a, b| a * 10 + b;
const A3: fn(u32, u32, u32) -> u32 = |a, b, c| a * 100 + b * 10 + c;
const A4: fn(u32, u32, u32, u32) -> u32 = |a, b, c, d| a * 1000 + b * 100 + c * 10 + d;
const A6: fn(u8, i8, u16, i16, u32, i32) -> i64 = |a, b, c, d, e, f| a as i64 - b as i64 + c as i64 * 3 - d as i64 + e as i64 * 7 + f as i64;
const Z1: fn(()) -> u32 = |_| 11;
const Z2: fn((), u32) -> u32 = |_, x| x + 12;
const Z3: fn(u32, ()) -> u32 = |x, _| x + 13;
const Z4: fn(Unit, u32, Unit) -> u32 = |_, x, _| x + 14;
const Z5: fn((), ()) -> u32 = |_, _| 15;
const AG1: fn(P) -> u32 = |p| p.x * 3 + p.y;
const AG2: fn(P, P) -> P = |a, b| P { x: a.x + b.x, y: a.y * b.y };
const AG3: fn([u32; 3]) -> u32 = |a| a[0] + a[1] * 2 + a[2] * 3;
const AG4: fn((u32, bool)) -> u32 = |t| if t.1 { t.0 } else { 0 };
const AG5: fn(E) -> u32 = |e| match e { E::A(x) => x, E::B => 77, E::C(a, b) => a as u32 * 256 + b as u32 };
const W1: fn(u64) -> u64 = |x| x.wrapping_mul(0x1_0000_0001).wrapping_add(7);
const W2: fn(u64, u64) -> u64 = |a, b| a ^ (b << 33);
const W3: fn(i64) -> i64 = |x| -x - 1;
const W4: fn(u128) -> u128 = |x| x * 3 + (1u128 << 100);
const W5: fn(u32, u64, u32) -> u64 = |a, b, c| a as u64 + b + c as u64;
const F1: fn(f64) -> f64 = |x| x * 1.5;
const F2: fn(f32, f32) -> f32 = |a, b| a - b;
const R1: fn(&u32) -> u32 = |r| *r + 5;
const R2: fn(&mut u32) = |r| *r += 9;
const R3: fn(&[u32]) -> u32 = |s| s.iter().sum();
const R4: fn(&str) -> usize = |s| s.len();
const R5: fn(&P) -> u32 = |p| p.x + p.y;
const R6: fn(&mut P, u32) = |p, v| { p.x = v; p.y += v; };
const R7: for<'a> fn(&'a u32, &'a u32) -> &'a u32 = |a, b| if *a > *b { a } else { b };
const O1: fn(Option<u32>) -> u32 = |o| o.unwrap_or(3);
const O2: fn(u32) -> Option<u32> = |x| if x > 2 { Some(x) } else { None };
const O3: fn(u32) -> Result<u32, u8> = |x| if x > 2 { Ok(x) } else { Err(x as u8) };
const O4: fn(Option<&u32>) -> u32 = |o| o.map_or(0, |x| *x);
const C1: fn(u32) -> char = |x| char::from_u32(x + 65).unwrap_or('?');
const C2: fn(char) -> u32 = |c| c as u32;
const B1: fn(bool, bool) -> bool = |a, b| a ^ b;
const H1: fn(fn(u32) -> u32, u32) -> u32 = |f, x| f(f(x));
const H2: fn(u32) -> fn(u32) -> u32 = |x| if x == 0 { |y| y + 1 } else { |y| y * 2 };
const U1: unsafe fn(u32) -> u32 = |x| x + 21;
const RS: fn(u32) -> &'static str = |x| if x == 0 { "zero" } else { "nonzero" };
const RA: fn(u32) -> [u32; 2] = |x| [x, x + 1];
const RU: fn(u32) -> Unit = |_| Unit;
const I8R: fn(u32) -> i8 = |x| (x as i8).wrapping_neg();
const U64R: fn() -> u64 = || u64::MAX - 1;

static S0: fn() = || print_u32(200);
static S1: fn(u32) -> u32 = |a| a + 2;
static S2: fn(u64, P, ()) -> u64 = |a, p, _| a + p.x as u64 * p.y as u64;
static S3: fn(&mut u32, u32) = |r, v| *r = *r * 2 + v;

#[inline(never)]
fn bb<T>(x: T) -> T { black_box(x) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    A0(); bb(A0)();
    print_u32(A1(1)); print_u32(bb(A1)(2));
    print_u32(A2(1, 2)); print_u32(bb(A2)(3, 4));
    print_u32(A3(1, 2, 3)); print_u32(bb(A3)(4, 5, 6));
    print_u32(A4(1, 2, 3, 4)); print_u32(bb(A4)(5, 6, 7, 8));
    print_i64(A6(1, -2, 3, -4, 5, -6)); print_i64(bb(A6)(200, 100, 60000, 30000, 4000000000, -2000000000));
    print_u32(Z1(())); print_u32(bb(Z1)(()));
    print_u32(Z2((), 1)); print_u32(bb(Z2)((), 2));
    print_u32(Z3(1, ())); print_u32(bb(Z3)(2, ()));
    print_u32(Z4(Unit, 1, Unit)); print_u32(bb(Z4)(Unit, 2, Unit));
    print_u32(Z5((), ())); print_u32(bb(Z5)((), ()));
    print_u32(AG1(P { x: 1, y: 2 })); print_u32(bb(AG1)(P { x: 3, y: 4 }));
    let p = AG2(P { x: 1, y: 2 }, P { x: 3, y: 4 }); print_u32(p.x); print_u32(p.y);
    let p = bb(AG2)(P { x: 5, y: 6 }, P { x: 7, y: 8 }); print_u32(p.x); print_u32(p.y);
    print_u32(AG3([1, 2, 3])); print_u32(bb(AG3)([4, 5, 6]));
    print_u32(AG4((5, true))); print_u32(bb(AG4)((6, false)));
    print_u32(AG5(E::A(9))); print_u32(bb(AG5)(E::B)); print_u32(bb(AG5)(E::C(1, 2)));
    print_u64(W1(0xdead_beef_cafe)); print_u64(bb(W1)(u64::MAX));
    print_u64(W2(1, 3)); print_u64(bb(W2)(0xffff_ffff, 0x7fff_ffff));
    print_i64(W3(i64::MAX)); print_i64(bb(W3)(-5));
    let w = W4(5); print_u64((w >> 64) as u64); print_u64(w as u64);
    let w = bb(W4)(u64::MAX as u128); print_u64((w >> 64) as u64); print_u64(w as u64);
    print_u64(W5(1, 1 << 40, 3)); print_u64(bb(W5)(u32::MAX, u64::MAX >> 1, u32::MAX));
    print_f64(F1(3.0)); print_f64(bb(F1)(-0.5));
    print_f64(F2(1.5, 0.25) as f64); print_f64(bb(F2)(2.0, 4.0) as f64);
    let mut v = 3u32;
    print_u32(R1(&v)); print_u32(bb(R1)(&v));
    R2(&mut v); print_u32(v); bb(R2)(&mut v); print_u32(v);
    print_u32(R3(&[1, 2, 3])); print_u32(bb(R3)(&[4, 5, 6, 7]));
    print_usize(R4("hello")); print_usize(bb(R4)("hi"));
    let mut p = P { x: 1, y: 2 };
    print_u32(R5(&p)); print_u32(bb(R5)(&p));
    R6(&mut p, 10); print_u32(p.x); print_u32(p.y); bb(R6)(&mut p, 20); print_u32(p.x); print_u32(p.y);
    let (a, b) = (4u32, 9u32);
    print_u32(*R7(&a, &b)); print_u32(*bb(R7)(&b, &a));
    print_u32(O1(None)); print_u32(bb(O1)(Some(8)));
    print_u32(O2(1).unwrap_or(99)); print_u32(bb(O2)(5).unwrap_or(99));
    print_u32(match O3(1) { Ok(x) => x, Err(e) => 1000 + e as u32 }); print_u32(match bb(O3)(7) { Ok(x) => x, Err(e) => 1000 + e as u32 });
    print_u32(O4(None)); print_u32(bb(O4)(Some(&a)));
    print_u32(C1(1) as u32); print_u32(bb(C1)(25) as u32);
    print_u32(C2('z')); print_u32(bb(C2)('\u{10ffff}'));
    print_bool(B1(true, false)); print_bool(bb(B1)(true, true));
    print_u32(H1(A1, 5)); print_u32(bb(H1)(S1, 5)); print_u32(H1(|x| x * 3, 5));
    print_u32(H2(0)(10)); print_u32(bb(H2)(1)(10)); print_u32(bb(H2(bb(0)))(20));
    print_u32(unsafe { U1(1) }); print_u32(unsafe { bb(U1)(2) });
    print_str(RS(0)); print_str(bb(RS)(1));
    let a2 = RA(4); print_u32(a2[0] + a2[1]); let a2 = bb(RA)(6); print_u32(a2[0] * a2[1]);
    let _u: Unit = RU(1); let _u: Unit = bb(RU)(2);
    print_i32(I8R(5) as i32); print_i32(bb(I8R)(200) as i32);
    print_u64(U64R()); print_u64(bb(U64R)());
    S0(); bb(S0)(); bb(&S0)();
    print_u32(S1(1)); print_u32(bb(S1)(2)); print_u32(bb(&S1)(3));
    print_u64(S2(1 << 35, P { x: 3, y: 4 }, ())); print_u64(bb(S2)(5, P { x: 6, y: 7 }, ()));
    let mut m = 1u32; S3(&mut m, 5); print_u32(m); bb(S3)(&mut m, 6); print_u32(m);
}
