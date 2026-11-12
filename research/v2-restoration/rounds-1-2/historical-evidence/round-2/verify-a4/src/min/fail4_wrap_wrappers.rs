#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::mem::ManuallyDrop;

#[derive(Clone, Copy)]
#[repr(u8)]
enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)]
#[repr(transparent)]
struct Tr<T>(T);
#[derive(Clone, Copy)]
struct Wrap<T>(T);
#[derive(Clone, Copy)]
struct Named { z: (), s: Small }

macro_rules! fam {
    ($f:ident, $W:ident, $P:ty, $mk:expr, |$p:ident| $get:expr) => {
        #[derive(Clone, Copy)]
        enum $W { P($P), Q, R, S }
        #[inline(never)]
        fn $f() {
            #[inline(never)]
            fn code(w: $W) -> u32 { match w { $W::P($p) => $get as u32, $W::Q => 1000, $W::R => 1001, $W::S => 1002 } }
            const P: $W = $W::P($mk);
            const Q: $W = $W::Q;
            const R: $W = $W::R;
            const S: $W = $W::S;
            static T: [$W; 4] = [P, Q, R, S];
            print_u32(code(P)); print_u32(code(Q)); print_u32(code(R)); print_u32(code(S));
            print_u32(match R { $W::R => 1, _ => 0 });
            print_u32(match S { $W::S => 1, _ => 0 });
            let mut i = 0; while i < 4 { print_u32(code(T[black_box(i)])); i += 1; }
        }
    };
}
fam!(direct, W0, Small, Small::Y, |p| p);
fam!(transparent, W1, Tr<Small>, Tr(Small::Y), |p| p.0);
fam!(manually, W2, ManuallyDrop<Small>, ManuallyDrop::new(Small::Y), |p| *p);
fam!(tuple, W3, (Small,), (Small::Y,), |p| p.0);
fam!(wrap, W4, Wrap<Small>, Wrap(Small::Y), |p| p.0);
fam!(named, W5, Named, Named { z: (), s: Small::Y }, |p| p.s);
fam!(nested, W6, Wrap<Tr<(Small,)>>, Wrap(Tr((Small::Y,))), |p| ((p.0).0).0);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    direct(); transparent(); manually(); tuple(); wrap(); named(); nested();
}
