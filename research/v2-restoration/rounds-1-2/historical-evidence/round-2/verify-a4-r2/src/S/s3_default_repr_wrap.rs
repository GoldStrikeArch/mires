#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;

// Enums with the default repr (discriminant type isize) whose tag rustc narrows to one or two
// bytes; the outer enum's niche values cross zero (255, 0 / 65535, 0), so the decode must wrap at
// the tag's width, not at isize's.
#[derive(Clone, Copy)] enum Big { A = 200, B = 254 }
#[derive(Clone, Copy)] enum Neg { A = -56, B = -2 }
#[derive(Clone, Copy)] enum Big16 { A = 0xFFF0, B = 0xFFFE }
#[derive(Clone, Copy)] enum Neg16 { A = -200, B = -2 }
#[derive(Clone, Copy)] enum Neg32 { A = -70000, B = -2 }
#[derive(Clone, Copy)] enum Mid { A = 100, B = 126 }
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }

macro_rules! fam {
    ($id:expr, $O:ident, $P:ty, $a:expr, $b:expr, |$p:ident| $read:expr) => {{
        #[derive(Clone, Copy)] enum $O { P($P), Q, R }
        #[inline(never)] fn cls(e: $O) -> u32 { match e { $O::P($p) => $read, $O::Q => 1001, $O::R => 1002 } }
        #[inline(never)] fn clo(e: Option<$O>) -> u32 { match e { Some(x) => cls(x), None => 1003 } }
        const K: [$O; 4] = [$O::P($a), $O::P($b), $O::Q, $O::R];
        static S: [$O; 4] = [$O::P($a), $O::P($b), $O::Q, $O::R];
        const KA: $O = $O::P($a); const KB: $O = $O::P($b); const KQ: $O = $O::Q; const KR: $O = $O::R;
        const OQ: Option<$O> = Some($O::Q); const OR: Option<$O> = Some($O::R); const ON: Option<$O> = None; const OB: Option<$O> = Some($O::P($b));
        print_i32(-$id);
        print_u32(cls(KA)); print_u32(cls(KB)); print_u32(cls(KQ)); print_u32(cls(KR));
        print_u32(cls(black_box(KA))); print_u32(cls(black_box(KB))); print_u32(cls(black_box(KQ))); print_u32(cls(black_box(KR)));
        print_u32(match KR { $O::P($p) => $read, $O::Q => 1001, $O::R => 1002 });
        print_u32(match KQ { $O::P($p) => $read, $O::Q => 1001, $O::R => 1002 });
        print_u32(clo(OQ)); print_u32(clo(OR)); print_u32(clo(ON)); print_u32(clo(OB));
        print_u32(clo(black_box(OQ))); print_u32(clo(black_box(OR))); print_u32(clo(black_box(ON))); print_u32(clo(black_box(OB)));
        let mut i = 0; while i < 4 { print_u32(cls(S[black_box(i)])); print_u32(cls(K[black_box(i)])); i += 1; }
        print_u32(cls(if black_box(true) { $O::R } else { $O::Q })); print_u32(cls($O::P(black_box($b))));
    }};
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    fam!(1, O1, Big, Big::A, Big::B, |p| p as u32);
    fam!(2, O2, Neg, Neg::A, Neg::B, |p| p as i32 as u32);
    fam!(3, O3, Big16, Big16::A, Big16::B, |p| p as u32);
    fam!(4, O4, Neg16, Neg16::A, Neg16::B, |p| p as i32 as u32);
    fam!(5, O5, Neg32, Neg32::A, Neg32::B, |p| p as i32 as u32);
    fam!(6, O6, Mid, Mid::A, Mid::B, |p| p as u32);
    fam!(7, O7, Wrap<Big>, Wrap(Big::A), Wrap(Big::B), |p| p.0 as u32);
    fam!(8, O8, SL<Neg>, SL::Live(Neg::A), SL::Live(Neg::B), |p| match p { SL::Live(n) => n as i32 as u32, _ => 99 });
    fam!(9, O9, ((), Big16), ((), Big16::A), ((), Big16::B), |p| p.1 as u32);
    fam!(10, O10, Option<Big>, Some(Big::B), None, |p| match p { Some(b) => b as u32, None => 1004 });
    fam!(11, O11, Option<Option<Neg>>, Some(Some(Neg::B)), Some(None), |p| match p { Some(Some(b)) => b as i32 as u32, Some(None) => 1005, None => 1006 });
}
