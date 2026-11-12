//! Verifier round-2 probe r6: the "element copy written and never read" shape over a static, a local array and a slice parameter (no pooled constant involved).
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
#[derive(Clone, Copy)]
struct P { x: u32, y: u32 }
static ST: [P; 6] = [P { x: 0, y: 10 }, P { x: 1, y: 11 }, P { x: 2, y: 12 }, P { x: 3, y: 13 }, P { x: 4, y: 14 }, P { x: 5, y: 15 }];
#[inline(never)] fn total(ps: &[P]) -> u32 { ps.iter().map(|p| p.x).sum() }
#[inline(never)]
#[allow(unused_assignments, unused_variables)]
fn over_param(ps: &[P]) { let mut p = ps[4]; p.x = 99; print_u32(ps[4].x); print_u32(total(ps)); }
#[unsafe(no_mangle)]
#[allow(unused_assignments, unused_variables)]
pub fn rust_entry() {
    print_u32(total(&ST));
    let mut p = ST[4]; p.x = 99;
    print_u32(ST[4].x); print_u32(total(&ST));
    let local = [P { x: 0, y: 10 }, P { x: 1, y: 11 }, P { x: 2, y: 12 }, P { x: 3, y: 13 }, P { x: 4, y: 14 }, P { x: 5, y: 15 }];
    let local = core::hint::black_box(local);
    let mut q = local[4]; q.x = 99;
    print_u32(local[4].x); print_u32(total(&local));
    let other = core::hint::black_box([P { x: 0, y: 10 }, P { x: 1, y: 11 }, P { x: 2, y: 12 }, P { x: 3, y: 13 }, P { x: 4, y: 14 }, P { x: 5, y: 15 }]);
    over_param(&other);
}
