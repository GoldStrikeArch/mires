#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
#[derive(Clone, Copy)]
#[repr(u8)]
enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)]
enum Inner { Dead((), Infallible), Live(Small) }
#[derive(Clone, Copy)]
enum W { P(Inner), Q, R, S }
const KS: [W; 5] = [W::P(Inner::Live(Small::X)), W::P(Inner::Live(Small::Y)), W::Q, W::R, W::S];
const P: W = W::P(Inner::Live(Small::Y));
const R: W = W::R;
const S: W = W::S;
#[inline(never)]
fn code(w: W) -> u32 { match w { W::P(Inner::Live(s)) => s as u32, W::P(_) => 99, W::Q => 1000, W::R => 1001, W::S => 1002 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(code(P));
    print_u32(code(R));
    print_u32(code(S));
    print_u32(match R { W::R => 1, _ => 0 });
    let mut i = 0; while i < 5 { print_u32(code(KS[black_box(i)])); i += 1; }
}
