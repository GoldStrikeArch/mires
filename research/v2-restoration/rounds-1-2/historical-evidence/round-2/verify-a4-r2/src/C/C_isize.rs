#![no_std]
#![no_main]
#![feature(pattern_types, pattern_type_macro)]
#![allow(unused, unreachable_patterns, dead_code, internal_features)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::{transmute, ManuallyDrop};
use core::pattern_type;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] struct Named<T> { z: (), v: T, p: PhantomData<u64> }
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }

macro_rules! suite {
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            #[inline(never)]
            fn cls_o(o: Option<$T>) -> u32 { match o { Some($e) => $body, None => 55555 } }
            static TABLE: &[$T] = &[$($k),*];
            $(
                {
                    const K: $T = $k;
                    static S: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                    print_u32(cls(S));
                    const O: Option<$T> = Some($k);
                    print_u32(match O { Some($e) => $body, None => 55555 });
                    print_u32(cls_o(black_box(O)));
                }
            )*
            { const N: Option<$T> = None; print_u32(cls_o(N)); print_u32(cls_o(black_box(N))); }
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                i += 1;
            }
        }
    };
}
type P0 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E0 { Live(P0), U0 }
suite!(0, s0, E0, [E0::Live(unsafe { transmute::<isize, P0>((-61isize)) }), E0::Live(unsafe { transmute::<isize, P0>((-32isize)) }), E0::Live(unsafe { transmute::<isize, P0>((-2isize)) }), E0::U0], |e| match e { E0::Live(w) => { let p: P0 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E0::U0 => 1001 });
type P1 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E1 { Live(Wrap<P1>), U0 }
suite!(1, s1, E1, [E1::Live(Wrap(unsafe { transmute::<isize, P1>((-61isize)) })), E1::Live(Wrap(unsafe { transmute::<isize, P1>((-32isize)) })), E1::Live(Wrap(unsafe { transmute::<isize, P1>((-2isize)) })), E1::U0], |e| match e { E1::Live(w) => { let p: P1 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E1::U0 => 1001 });
type P2 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E2 { Live(Tr<P2>), U0 }
suite!(2, s2, E2, [E2::Live(Tr(unsafe { transmute::<isize, P2>((-61isize)) })), E2::Live(Tr(unsafe { transmute::<isize, P2>((-32isize)) })), E2::Live(Tr(unsafe { transmute::<isize, P2>((-2isize)) })), E2::U0], |e| match e { E2::Live(w) => { let p: P2 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E2::U0 => 1001 });
type P3 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E3 { Live(Named<P3>), U0 }
suite!(3, s3, E3, [E3::Live(Named { z: (), v: unsafe { transmute::<isize, P3>((-61isize)) }, p: PhantomData }), E3::Live(Named { z: (), v: unsafe { transmute::<isize, P3>((-32isize)) }, p: PhantomData }), E3::Live(Named { z: (), v: unsafe { transmute::<isize, P3>((-2isize)) }, p: PhantomData }), E3::U0], |e| match e { E3::Live(w) => { let p: P3 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E3::U0 => 1001 });
type P4 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E4 { Live(((), P4)), U0 }
suite!(4, s4, E4, [E4::Live(((), unsafe { transmute::<isize, P4>((-61isize)) })), E4::Live(((), unsafe { transmute::<isize, P4>((-32isize)) })), E4::Live(((), unsafe { transmute::<isize, P4>((-2isize)) })), E4::U0], |e| match e { E4::Live(w) => { let p: P4 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E4::U0 => 1001 });
type P5 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E5 { Live(SL<P5>), U0 }
suite!(5, s5, E5, [E5::Live(SL::Live(unsafe { transmute::<isize, P5>((-61isize)) })), E5::Live(SL::Live(unsafe { transmute::<isize, P5>((-32isize)) })), E5::Live(SL::Live(unsafe { transmute::<isize, P5>((-2isize)) })), E5::U0], |e| match e { E5::Live(w) => { let p: P5 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E5::U0 => 1001 });
type P6 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E6 { Live(ManuallyDrop<P6>), U0 }
suite!(6, s6, E6, [E6::Live(ManuallyDrop::new(unsafe { transmute::<isize, P6>((-61isize)) })), E6::Live(ManuallyDrop::new(unsafe { transmute::<isize, P6>((-32isize)) })), E6::Live(ManuallyDrop::new(unsafe { transmute::<isize, P6>((-2isize)) })), E6::U0], |e| match e { E6::Live(w) => { let p: P6 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E6::U0 => 1001 });
type P7 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E7 { Live(Wrap<SL<Tr<P7>>>), U0 }
suite!(7, s7, E7, [E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P7>((-61isize)) })))), E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P7>((-32isize)) })))), E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P7>((-2isize)) })))), E7::U0], |e| match e { E7::Live(w) => { let p: P7 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E7::U0 => 1001 });
type P8 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E8 { Live(P8), U0, U1 }
suite!(8, s8, E8, [E8::Live(unsafe { transmute::<isize, P8>((-61isize)) }), E8::Live(unsafe { transmute::<isize, P8>((-32isize)) }), E8::Live(unsafe { transmute::<isize, P8>((-2isize)) }), E8::U0, E8::U1], |e| match e { E8::Live(w) => { let p: P8 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E8::U0 => 1001, E8::U1 => 1002 });
type P9 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E9 { Live(Wrap<P9>), U0, U1 }
suite!(9, s9, E9, [E9::Live(Wrap(unsafe { transmute::<isize, P9>((-61isize)) })), E9::Live(Wrap(unsafe { transmute::<isize, P9>((-32isize)) })), E9::Live(Wrap(unsafe { transmute::<isize, P9>((-2isize)) })), E9::U0, E9::U1], |e| match e { E9::Live(w) => { let p: P9 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E9::U0 => 1001, E9::U1 => 1002 });
type P10 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E10 { Live(Tr<P10>), U0, U1 }
suite!(10, s10, E10, [E10::Live(Tr(unsafe { transmute::<isize, P10>((-61isize)) })), E10::Live(Tr(unsafe { transmute::<isize, P10>((-32isize)) })), E10::Live(Tr(unsafe { transmute::<isize, P10>((-2isize)) })), E10::U0, E10::U1], |e| match e { E10::Live(w) => { let p: P10 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E10::U0 => 1001, E10::U1 => 1002 });
type P11 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E11 { Live(Named<P11>), U0, U1 }
suite!(11, s11, E11, [E11::Live(Named { z: (), v: unsafe { transmute::<isize, P11>((-61isize)) }, p: PhantomData }), E11::Live(Named { z: (), v: unsafe { transmute::<isize, P11>((-32isize)) }, p: PhantomData }), E11::Live(Named { z: (), v: unsafe { transmute::<isize, P11>((-2isize)) }, p: PhantomData }), E11::U0, E11::U1], |e| match e { E11::Live(w) => { let p: P11 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E11::U0 => 1001, E11::U1 => 1002 });
type P12 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E12 { Live(((), P12)), U0, U1 }
suite!(12, s12, E12, [E12::Live(((), unsafe { transmute::<isize, P12>((-61isize)) })), E12::Live(((), unsafe { transmute::<isize, P12>((-32isize)) })), E12::Live(((), unsafe { transmute::<isize, P12>((-2isize)) })), E12::U0, E12::U1], |e| match e { E12::Live(w) => { let p: P12 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E12::U0 => 1001, E12::U1 => 1002 });
type P13 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E13 { Live(SL<P13>), U0, U1 }
suite!(13, s13, E13, [E13::Live(SL::Live(unsafe { transmute::<isize, P13>((-61isize)) })), E13::Live(SL::Live(unsafe { transmute::<isize, P13>((-32isize)) })), E13::Live(SL::Live(unsafe { transmute::<isize, P13>((-2isize)) })), E13::U0, E13::U1], |e| match e { E13::Live(w) => { let p: P13 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E13::U0 => 1001, E13::U1 => 1002 });
type P14 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E14 { Live(ManuallyDrop<P14>), U0, U1 }
suite!(14, s14, E14, [E14::Live(ManuallyDrop::new(unsafe { transmute::<isize, P14>((-61isize)) })), E14::Live(ManuallyDrop::new(unsafe { transmute::<isize, P14>((-32isize)) })), E14::Live(ManuallyDrop::new(unsafe { transmute::<isize, P14>((-2isize)) })), E14::U0, E14::U1], |e| match e { E14::Live(w) => { let p: P14 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E14::U0 => 1001, E14::U1 => 1002 });
type P15 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E15 { Live(Wrap<SL<Tr<P15>>>), U0, U1 }
suite!(15, s15, E15, [E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P15>((-61isize)) })))), E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P15>((-32isize)) })))), E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P15>((-2isize)) })))), E15::U0, E15::U1], |e| match e { E15::Live(w) => { let p: P15 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E15::U0 => 1001, E15::U1 => 1002 });
type P16 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E16 { Live(P16), U0, U1, U2 }
suite!(16, s16, E16, [E16::Live(unsafe { transmute::<isize, P16>((-61isize)) }), E16::Live(unsafe { transmute::<isize, P16>((-32isize)) }), E16::Live(unsafe { transmute::<isize, P16>((-2isize)) }), E16::U0, E16::U1, E16::U2], |e| match e { E16::Live(w) => { let p: P16 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E16::U0 => 1001, E16::U1 => 1002, E16::U2 => 1003 });
type P17 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E17 { Live(Wrap<P17>), U0, U1, U2 }
suite!(17, s17, E17, [E17::Live(Wrap(unsafe { transmute::<isize, P17>((-61isize)) })), E17::Live(Wrap(unsafe { transmute::<isize, P17>((-32isize)) })), E17::Live(Wrap(unsafe { transmute::<isize, P17>((-2isize)) })), E17::U0, E17::U1, E17::U2], |e| match e { E17::Live(w) => { let p: P17 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E17::U0 => 1001, E17::U1 => 1002, E17::U2 => 1003 });
type P18 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E18 { Live(Tr<P18>), U0, U1, U2 }
suite!(18, s18, E18, [E18::Live(Tr(unsafe { transmute::<isize, P18>((-61isize)) })), E18::Live(Tr(unsafe { transmute::<isize, P18>((-32isize)) })), E18::Live(Tr(unsafe { transmute::<isize, P18>((-2isize)) })), E18::U0, E18::U1, E18::U2], |e| match e { E18::Live(w) => { let p: P18 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E18::U0 => 1001, E18::U1 => 1002, E18::U2 => 1003 });
type P19 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E19 { Live(Named<P19>), U0, U1, U2 }
suite!(19, s19, E19, [E19::Live(Named { z: (), v: unsafe { transmute::<isize, P19>((-61isize)) }, p: PhantomData }), E19::Live(Named { z: (), v: unsafe { transmute::<isize, P19>((-32isize)) }, p: PhantomData }), E19::Live(Named { z: (), v: unsafe { transmute::<isize, P19>((-2isize)) }, p: PhantomData }), E19::U0, E19::U1, E19::U2], |e| match e { E19::Live(w) => { let p: P19 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E19::U0 => 1001, E19::U1 => 1002, E19::U2 => 1003 });
type P20 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E20 { Live(((), P20)), U0, U1, U2 }
suite!(20, s20, E20, [E20::Live(((), unsafe { transmute::<isize, P20>((-61isize)) })), E20::Live(((), unsafe { transmute::<isize, P20>((-32isize)) })), E20::Live(((), unsafe { transmute::<isize, P20>((-2isize)) })), E20::U0, E20::U1, E20::U2], |e| match e { E20::Live(w) => { let p: P20 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E20::U0 => 1001, E20::U1 => 1002, E20::U2 => 1003 });
type P21 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E21 { Live(SL<P21>), U0, U1, U2 }
suite!(21, s21, E21, [E21::Live(SL::Live(unsafe { transmute::<isize, P21>((-61isize)) })), E21::Live(SL::Live(unsafe { transmute::<isize, P21>((-32isize)) })), E21::Live(SL::Live(unsafe { transmute::<isize, P21>((-2isize)) })), E21::U0, E21::U1, E21::U2], |e| match e { E21::Live(w) => { let p: P21 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E21::U0 => 1001, E21::U1 => 1002, E21::U2 => 1003 });
type P22 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E22 { Live(ManuallyDrop<P22>), U0, U1, U2 }
suite!(22, s22, E22, [E22::Live(ManuallyDrop::new(unsafe { transmute::<isize, P22>((-61isize)) })), E22::Live(ManuallyDrop::new(unsafe { transmute::<isize, P22>((-32isize)) })), E22::Live(ManuallyDrop::new(unsafe { transmute::<isize, P22>((-2isize)) })), E22::U0, E22::U1, E22::U2], |e| match e { E22::Live(w) => { let p: P22 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E22::U0 => 1001, E22::U1 => 1002, E22::U2 => 1003 });
type P23 = pattern_type!(isize is -61isize..=-2isize);
#[derive(Clone, Copy)] enum E23 { Live(Wrap<SL<Tr<P23>>>), U0, U1, U2 }
suite!(23, s23, E23, [E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P23>((-61isize)) })))), E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P23>((-32isize)) })))), E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P23>((-2isize)) })))), E23::U0, E23::U1, E23::U2], |e| match e { E23::Live(w) => { let p: P23 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E23::U0 => 1001, E23::U1 => 1002, E23::U2 => 1003 });
type P24 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E24 { Live(P24), U0, U1 }
suite!(24, s24, E24, [E24::Live(unsafe { transmute::<isize, P24>((-61isize)) }), E24::Live(unsafe { transmute::<isize, P24>((-32isize)) }), E24::Live(unsafe { transmute::<isize, P24>((-3isize)) }), E24::U0, E24::U1], |e| match e { E24::Live(w) => { let p: P24 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E24::U0 => 1001, E24::U1 => 1002 });
type P25 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E25 { Live(Wrap<P25>), U0, U1 }
suite!(25, s25, E25, [E25::Live(Wrap(unsafe { transmute::<isize, P25>((-61isize)) })), E25::Live(Wrap(unsafe { transmute::<isize, P25>((-32isize)) })), E25::Live(Wrap(unsafe { transmute::<isize, P25>((-3isize)) })), E25::U0, E25::U1], |e| match e { E25::Live(w) => { let p: P25 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E25::U0 => 1001, E25::U1 => 1002 });
type P26 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E26 { Live(Tr<P26>), U0, U1 }
suite!(26, s26, E26, [E26::Live(Tr(unsafe { transmute::<isize, P26>((-61isize)) })), E26::Live(Tr(unsafe { transmute::<isize, P26>((-32isize)) })), E26::Live(Tr(unsafe { transmute::<isize, P26>((-3isize)) })), E26::U0, E26::U1], |e| match e { E26::Live(w) => { let p: P26 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E26::U0 => 1001, E26::U1 => 1002 });
type P27 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E27 { Live(Named<P27>), U0, U1 }
suite!(27, s27, E27, [E27::Live(Named { z: (), v: unsafe { transmute::<isize, P27>((-61isize)) }, p: PhantomData }), E27::Live(Named { z: (), v: unsafe { transmute::<isize, P27>((-32isize)) }, p: PhantomData }), E27::Live(Named { z: (), v: unsafe { transmute::<isize, P27>((-3isize)) }, p: PhantomData }), E27::U0, E27::U1], |e| match e { E27::Live(w) => { let p: P27 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E27::U0 => 1001, E27::U1 => 1002 });
type P28 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E28 { Live(((), P28)), U0, U1 }
suite!(28, s28, E28, [E28::Live(((), unsafe { transmute::<isize, P28>((-61isize)) })), E28::Live(((), unsafe { transmute::<isize, P28>((-32isize)) })), E28::Live(((), unsafe { transmute::<isize, P28>((-3isize)) })), E28::U0, E28::U1], |e| match e { E28::Live(w) => { let p: P28 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E28::U0 => 1001, E28::U1 => 1002 });
type P29 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E29 { Live(SL<P29>), U0, U1 }
suite!(29, s29, E29, [E29::Live(SL::Live(unsafe { transmute::<isize, P29>((-61isize)) })), E29::Live(SL::Live(unsafe { transmute::<isize, P29>((-32isize)) })), E29::Live(SL::Live(unsafe { transmute::<isize, P29>((-3isize)) })), E29::U0, E29::U1], |e| match e { E29::Live(w) => { let p: P29 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E29::U0 => 1001, E29::U1 => 1002 });
type P30 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E30 { Live(ManuallyDrop<P30>), U0, U1 }
suite!(30, s30, E30, [E30::Live(ManuallyDrop::new(unsafe { transmute::<isize, P30>((-61isize)) })), E30::Live(ManuallyDrop::new(unsafe { transmute::<isize, P30>((-32isize)) })), E30::Live(ManuallyDrop::new(unsafe { transmute::<isize, P30>((-3isize)) })), E30::U0, E30::U1], |e| match e { E30::Live(w) => { let p: P30 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E30::U0 => 1001, E30::U1 => 1002 });
type P31 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E31 { Live(Wrap<SL<Tr<P31>>>), U0, U1 }
suite!(31, s31, E31, [E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P31>((-61isize)) })))), E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P31>((-32isize)) })))), E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P31>((-3isize)) })))), E31::U0, E31::U1], |e| match e { E31::Live(w) => { let p: P31 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E31::U0 => 1001, E31::U1 => 1002 });
type P32 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E32 { Live(P32), U0, U1, U2 }
suite!(32, s32, E32, [E32::Live(unsafe { transmute::<isize, P32>((-61isize)) }), E32::Live(unsafe { transmute::<isize, P32>((-32isize)) }), E32::Live(unsafe { transmute::<isize, P32>((-3isize)) }), E32::U0, E32::U1, E32::U2], |e| match e { E32::Live(w) => { let p: P32 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E32::U0 => 1001, E32::U1 => 1002, E32::U2 => 1003 });
type P33 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E33 { Live(Wrap<P33>), U0, U1, U2 }
suite!(33, s33, E33, [E33::Live(Wrap(unsafe { transmute::<isize, P33>((-61isize)) })), E33::Live(Wrap(unsafe { transmute::<isize, P33>((-32isize)) })), E33::Live(Wrap(unsafe { transmute::<isize, P33>((-3isize)) })), E33::U0, E33::U1, E33::U2], |e| match e { E33::Live(w) => { let p: P33 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E33::U0 => 1001, E33::U1 => 1002, E33::U2 => 1003 });
type P34 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E34 { Live(Tr<P34>), U0, U1, U2 }
suite!(34, s34, E34, [E34::Live(Tr(unsafe { transmute::<isize, P34>((-61isize)) })), E34::Live(Tr(unsafe { transmute::<isize, P34>((-32isize)) })), E34::Live(Tr(unsafe { transmute::<isize, P34>((-3isize)) })), E34::U0, E34::U1, E34::U2], |e| match e { E34::Live(w) => { let p: P34 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E34::U0 => 1001, E34::U1 => 1002, E34::U2 => 1003 });
type P35 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E35 { Live(Named<P35>), U0, U1, U2 }
suite!(35, s35, E35, [E35::Live(Named { z: (), v: unsafe { transmute::<isize, P35>((-61isize)) }, p: PhantomData }), E35::Live(Named { z: (), v: unsafe { transmute::<isize, P35>((-32isize)) }, p: PhantomData }), E35::Live(Named { z: (), v: unsafe { transmute::<isize, P35>((-3isize)) }, p: PhantomData }), E35::U0, E35::U1, E35::U2], |e| match e { E35::Live(w) => { let p: P35 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E35::U0 => 1001, E35::U1 => 1002, E35::U2 => 1003 });
type P36 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E36 { Live(((), P36)), U0, U1, U2 }
suite!(36, s36, E36, [E36::Live(((), unsafe { transmute::<isize, P36>((-61isize)) })), E36::Live(((), unsafe { transmute::<isize, P36>((-32isize)) })), E36::Live(((), unsafe { transmute::<isize, P36>((-3isize)) })), E36::U0, E36::U1, E36::U2], |e| match e { E36::Live(w) => { let p: P36 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E36::U0 => 1001, E36::U1 => 1002, E36::U2 => 1003 });
type P37 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E37 { Live(SL<P37>), U0, U1, U2 }
suite!(37, s37, E37, [E37::Live(SL::Live(unsafe { transmute::<isize, P37>((-61isize)) })), E37::Live(SL::Live(unsafe { transmute::<isize, P37>((-32isize)) })), E37::Live(SL::Live(unsafe { transmute::<isize, P37>((-3isize)) })), E37::U0, E37::U1, E37::U2], |e| match e { E37::Live(w) => { let p: P37 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E37::U0 => 1001, E37::U1 => 1002, E37::U2 => 1003 });
type P38 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E38 { Live(ManuallyDrop<P38>), U0, U1, U2 }
suite!(38, s38, E38, [E38::Live(ManuallyDrop::new(unsafe { transmute::<isize, P38>((-61isize)) })), E38::Live(ManuallyDrop::new(unsafe { transmute::<isize, P38>((-32isize)) })), E38::Live(ManuallyDrop::new(unsafe { transmute::<isize, P38>((-3isize)) })), E38::U0, E38::U1, E38::U2], |e| match e { E38::Live(w) => { let p: P38 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E38::U0 => 1001, E38::U1 => 1002, E38::U2 => 1003 });
type P39 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E39 { Live(Wrap<SL<Tr<P39>>>), U0, U1, U2 }
suite!(39, s39, E39, [E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P39>((-61isize)) })))), E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P39>((-32isize)) })))), E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P39>((-3isize)) })))), E39::U0, E39::U1, E39::U2], |e| match e { E39::Live(w) => { let p: P39 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E39::U0 => 1001, E39::U1 => 1002, E39::U2 => 1003 });
type P40 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E40 { Live(P40), U0, U1, U2, U3 }
suite!(40, s40, E40, [E40::Live(unsafe { transmute::<isize, P40>((-61isize)) }), E40::Live(unsafe { transmute::<isize, P40>((-32isize)) }), E40::Live(unsafe { transmute::<isize, P40>((-3isize)) }), E40::U0, E40::U1, E40::U2, E40::U3], |e| match e { E40::Live(w) => { let p: P40 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E40::U0 => 1001, E40::U1 => 1002, E40::U2 => 1003, E40::U3 => 1004 });
type P41 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E41 { Live(Wrap<P41>), U0, U1, U2, U3 }
suite!(41, s41, E41, [E41::Live(Wrap(unsafe { transmute::<isize, P41>((-61isize)) })), E41::Live(Wrap(unsafe { transmute::<isize, P41>((-32isize)) })), E41::Live(Wrap(unsafe { transmute::<isize, P41>((-3isize)) })), E41::U0, E41::U1, E41::U2, E41::U3], |e| match e { E41::Live(w) => { let p: P41 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E41::U0 => 1001, E41::U1 => 1002, E41::U2 => 1003, E41::U3 => 1004 });
type P42 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E42 { Live(Tr<P42>), U0, U1, U2, U3 }
suite!(42, s42, E42, [E42::Live(Tr(unsafe { transmute::<isize, P42>((-61isize)) })), E42::Live(Tr(unsafe { transmute::<isize, P42>((-32isize)) })), E42::Live(Tr(unsafe { transmute::<isize, P42>((-3isize)) })), E42::U0, E42::U1, E42::U2, E42::U3], |e| match e { E42::Live(w) => { let p: P42 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E42::U0 => 1001, E42::U1 => 1002, E42::U2 => 1003, E42::U3 => 1004 });
type P43 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E43 { Live(Named<P43>), U0, U1, U2, U3 }
suite!(43, s43, E43, [E43::Live(Named { z: (), v: unsafe { transmute::<isize, P43>((-61isize)) }, p: PhantomData }), E43::Live(Named { z: (), v: unsafe { transmute::<isize, P43>((-32isize)) }, p: PhantomData }), E43::Live(Named { z: (), v: unsafe { transmute::<isize, P43>((-3isize)) }, p: PhantomData }), E43::U0, E43::U1, E43::U2, E43::U3], |e| match e { E43::Live(w) => { let p: P43 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E43::U0 => 1001, E43::U1 => 1002, E43::U2 => 1003, E43::U3 => 1004 });
type P44 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E44 { Live(((), P44)), U0, U1, U2, U3 }
suite!(44, s44, E44, [E44::Live(((), unsafe { transmute::<isize, P44>((-61isize)) })), E44::Live(((), unsafe { transmute::<isize, P44>((-32isize)) })), E44::Live(((), unsafe { transmute::<isize, P44>((-3isize)) })), E44::U0, E44::U1, E44::U2, E44::U3], |e| match e { E44::Live(w) => { let p: P44 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E44::U0 => 1001, E44::U1 => 1002, E44::U2 => 1003, E44::U3 => 1004 });
type P45 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E45 { Live(SL<P45>), U0, U1, U2, U3 }
suite!(45, s45, E45, [E45::Live(SL::Live(unsafe { transmute::<isize, P45>((-61isize)) })), E45::Live(SL::Live(unsafe { transmute::<isize, P45>((-32isize)) })), E45::Live(SL::Live(unsafe { transmute::<isize, P45>((-3isize)) })), E45::U0, E45::U1, E45::U2, E45::U3], |e| match e { E45::Live(w) => { let p: P45 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E45::U0 => 1001, E45::U1 => 1002, E45::U2 => 1003, E45::U3 => 1004 });
type P46 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E46 { Live(ManuallyDrop<P46>), U0, U1, U2, U3 }
suite!(46, s46, E46, [E46::Live(ManuallyDrop::new(unsafe { transmute::<isize, P46>((-61isize)) })), E46::Live(ManuallyDrop::new(unsafe { transmute::<isize, P46>((-32isize)) })), E46::Live(ManuallyDrop::new(unsafe { transmute::<isize, P46>((-3isize)) })), E46::U0, E46::U1, E46::U2, E46::U3], |e| match e { E46::Live(w) => { let p: P46 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E46::U0 => 1001, E46::U1 => 1002, E46::U2 => 1003, E46::U3 => 1004 });
type P47 = pattern_type!(isize is -61isize..=-3isize);
#[derive(Clone, Copy)] enum E47 { Live(Wrap<SL<Tr<P47>>>), U0, U1, U2, U3 }
suite!(47, s47, E47, [E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P47>((-61isize)) })))), E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P47>((-32isize)) })))), E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P47>((-3isize)) })))), E47::U0, E47::U1, E47::U2, E47::U3], |e| match e { E47::Live(w) => { let p: P47 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E47::U0 => 1001, E47::U1 => 1002, E47::U2 => 1003, E47::U3 => 1004 });
type P48 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E48 { Live(P48), U0, U1, U2 }
suite!(48, s48, E48, [E48::Live(unsafe { transmute::<isize, P48>((-61isize)) }), E48::Live(unsafe { transmute::<isize, P48>((-33isize)) }), E48::Live(unsafe { transmute::<isize, P48>((-4isize)) }), E48::U0, E48::U1, E48::U2], |e| match e { E48::Live(w) => { let p: P48 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E48::U0 => 1001, E48::U1 => 1002, E48::U2 => 1003 });
type P49 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E49 { Live(Wrap<P49>), U0, U1, U2 }
suite!(49, s49, E49, [E49::Live(Wrap(unsafe { transmute::<isize, P49>((-61isize)) })), E49::Live(Wrap(unsafe { transmute::<isize, P49>((-33isize)) })), E49::Live(Wrap(unsafe { transmute::<isize, P49>((-4isize)) })), E49::U0, E49::U1, E49::U2], |e| match e { E49::Live(w) => { let p: P49 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E49::U0 => 1001, E49::U1 => 1002, E49::U2 => 1003 });
type P50 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E50 { Live(Tr<P50>), U0, U1, U2 }
suite!(50, s50, E50, [E50::Live(Tr(unsafe { transmute::<isize, P50>((-61isize)) })), E50::Live(Tr(unsafe { transmute::<isize, P50>((-33isize)) })), E50::Live(Tr(unsafe { transmute::<isize, P50>((-4isize)) })), E50::U0, E50::U1, E50::U2], |e| match e { E50::Live(w) => { let p: P50 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E50::U0 => 1001, E50::U1 => 1002, E50::U2 => 1003 });
type P51 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E51 { Live(Named<P51>), U0, U1, U2 }
suite!(51, s51, E51, [E51::Live(Named { z: (), v: unsafe { transmute::<isize, P51>((-61isize)) }, p: PhantomData }), E51::Live(Named { z: (), v: unsafe { transmute::<isize, P51>((-33isize)) }, p: PhantomData }), E51::Live(Named { z: (), v: unsafe { transmute::<isize, P51>((-4isize)) }, p: PhantomData }), E51::U0, E51::U1, E51::U2], |e| match e { E51::Live(w) => { let p: P51 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E51::U0 => 1001, E51::U1 => 1002, E51::U2 => 1003 });
type P52 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E52 { Live(((), P52)), U0, U1, U2 }
suite!(52, s52, E52, [E52::Live(((), unsafe { transmute::<isize, P52>((-61isize)) })), E52::Live(((), unsafe { transmute::<isize, P52>((-33isize)) })), E52::Live(((), unsafe { transmute::<isize, P52>((-4isize)) })), E52::U0, E52::U1, E52::U2], |e| match e { E52::Live(w) => { let p: P52 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E52::U0 => 1001, E52::U1 => 1002, E52::U2 => 1003 });
type P53 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E53 { Live(SL<P53>), U0, U1, U2 }
suite!(53, s53, E53, [E53::Live(SL::Live(unsafe { transmute::<isize, P53>((-61isize)) })), E53::Live(SL::Live(unsafe { transmute::<isize, P53>((-33isize)) })), E53::Live(SL::Live(unsafe { transmute::<isize, P53>((-4isize)) })), E53::U0, E53::U1, E53::U2], |e| match e { E53::Live(w) => { let p: P53 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E53::U0 => 1001, E53::U1 => 1002, E53::U2 => 1003 });
type P54 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E54 { Live(ManuallyDrop<P54>), U0, U1, U2 }
suite!(54, s54, E54, [E54::Live(ManuallyDrop::new(unsafe { transmute::<isize, P54>((-61isize)) })), E54::Live(ManuallyDrop::new(unsafe { transmute::<isize, P54>((-33isize)) })), E54::Live(ManuallyDrop::new(unsafe { transmute::<isize, P54>((-4isize)) })), E54::U0, E54::U1, E54::U2], |e| match e { E54::Live(w) => { let p: P54 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E54::U0 => 1001, E54::U1 => 1002, E54::U2 => 1003 });
type P55 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E55 { Live(Wrap<SL<Tr<P55>>>), U0, U1, U2 }
suite!(55, s55, E55, [E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P55>((-61isize)) })))), E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P55>((-33isize)) })))), E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P55>((-4isize)) })))), E55::U0, E55::U1, E55::U2], |e| match e { E55::Live(w) => { let p: P55 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E55::U0 => 1001, E55::U1 => 1002, E55::U2 => 1003 });
type P56 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E56 { Live(P56), U0, U1, U2, U3 }
suite!(56, s56, E56, [E56::Live(unsafe { transmute::<isize, P56>((-61isize)) }), E56::Live(unsafe { transmute::<isize, P56>((-33isize)) }), E56::Live(unsafe { transmute::<isize, P56>((-4isize)) }), E56::U0, E56::U1, E56::U2, E56::U3], |e| match e { E56::Live(w) => { let p: P56 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E56::U0 => 1001, E56::U1 => 1002, E56::U2 => 1003, E56::U3 => 1004 });
type P57 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E57 { Live(Wrap<P57>), U0, U1, U2, U3 }
suite!(57, s57, E57, [E57::Live(Wrap(unsafe { transmute::<isize, P57>((-61isize)) })), E57::Live(Wrap(unsafe { transmute::<isize, P57>((-33isize)) })), E57::Live(Wrap(unsafe { transmute::<isize, P57>((-4isize)) })), E57::U0, E57::U1, E57::U2, E57::U3], |e| match e { E57::Live(w) => { let p: P57 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E57::U0 => 1001, E57::U1 => 1002, E57::U2 => 1003, E57::U3 => 1004 });
type P58 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E58 { Live(Tr<P58>), U0, U1, U2, U3 }
suite!(58, s58, E58, [E58::Live(Tr(unsafe { transmute::<isize, P58>((-61isize)) })), E58::Live(Tr(unsafe { transmute::<isize, P58>((-33isize)) })), E58::Live(Tr(unsafe { transmute::<isize, P58>((-4isize)) })), E58::U0, E58::U1, E58::U2, E58::U3], |e| match e { E58::Live(w) => { let p: P58 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E58::U0 => 1001, E58::U1 => 1002, E58::U2 => 1003, E58::U3 => 1004 });
type P59 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E59 { Live(Named<P59>), U0, U1, U2, U3 }
suite!(59, s59, E59, [E59::Live(Named { z: (), v: unsafe { transmute::<isize, P59>((-61isize)) }, p: PhantomData }), E59::Live(Named { z: (), v: unsafe { transmute::<isize, P59>((-33isize)) }, p: PhantomData }), E59::Live(Named { z: (), v: unsafe { transmute::<isize, P59>((-4isize)) }, p: PhantomData }), E59::U0, E59::U1, E59::U2, E59::U3], |e| match e { E59::Live(w) => { let p: P59 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E59::U0 => 1001, E59::U1 => 1002, E59::U2 => 1003, E59::U3 => 1004 });
type P60 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E60 { Live(((), P60)), U0, U1, U2, U3 }
suite!(60, s60, E60, [E60::Live(((), unsafe { transmute::<isize, P60>((-61isize)) })), E60::Live(((), unsafe { transmute::<isize, P60>((-33isize)) })), E60::Live(((), unsafe { transmute::<isize, P60>((-4isize)) })), E60::U0, E60::U1, E60::U2, E60::U3], |e| match e { E60::Live(w) => { let p: P60 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E60::U0 => 1001, E60::U1 => 1002, E60::U2 => 1003, E60::U3 => 1004 });
type P61 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E61 { Live(SL<P61>), U0, U1, U2, U3 }
suite!(61, s61, E61, [E61::Live(SL::Live(unsafe { transmute::<isize, P61>((-61isize)) })), E61::Live(SL::Live(unsafe { transmute::<isize, P61>((-33isize)) })), E61::Live(SL::Live(unsafe { transmute::<isize, P61>((-4isize)) })), E61::U0, E61::U1, E61::U2, E61::U3], |e| match e { E61::Live(w) => { let p: P61 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E61::U0 => 1001, E61::U1 => 1002, E61::U2 => 1003, E61::U3 => 1004 });
type P62 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E62 { Live(ManuallyDrop<P62>), U0, U1, U2, U3 }
suite!(62, s62, E62, [E62::Live(ManuallyDrop::new(unsafe { transmute::<isize, P62>((-61isize)) })), E62::Live(ManuallyDrop::new(unsafe { transmute::<isize, P62>((-33isize)) })), E62::Live(ManuallyDrop::new(unsafe { transmute::<isize, P62>((-4isize)) })), E62::U0, E62::U1, E62::U2, E62::U3], |e| match e { E62::Live(w) => { let p: P62 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E62::U0 => 1001, E62::U1 => 1002, E62::U2 => 1003, E62::U3 => 1004 });
type P63 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E63 { Live(Wrap<SL<Tr<P63>>>), U0, U1, U2, U3 }
suite!(63, s63, E63, [E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P63>((-61isize)) })))), E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P63>((-33isize)) })))), E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P63>((-4isize)) })))), E63::U0, E63::U1, E63::U2, E63::U3], |e| match e { E63::Live(w) => { let p: P63 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E63::U0 => 1001, E63::U1 => 1002, E63::U2 => 1003, E63::U3 => 1004 });
type P64 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E64 { Live(P64), U0, U1, U2, U3, U4 }
suite!(64, s64, E64, [E64::Live(unsafe { transmute::<isize, P64>((-61isize)) }), E64::Live(unsafe { transmute::<isize, P64>((-33isize)) }), E64::Live(unsafe { transmute::<isize, P64>((-4isize)) }), E64::U0, E64::U1, E64::U2, E64::U3, E64::U4], |e| match e { E64::Live(w) => { let p: P64 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E64::U0 => 1001, E64::U1 => 1002, E64::U2 => 1003, E64::U3 => 1004, E64::U4 => 1005 });
type P65 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E65 { Live(Wrap<P65>), U0, U1, U2, U3, U4 }
suite!(65, s65, E65, [E65::Live(Wrap(unsafe { transmute::<isize, P65>((-61isize)) })), E65::Live(Wrap(unsafe { transmute::<isize, P65>((-33isize)) })), E65::Live(Wrap(unsafe { transmute::<isize, P65>((-4isize)) })), E65::U0, E65::U1, E65::U2, E65::U3, E65::U4], |e| match e { E65::Live(w) => { let p: P65 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E65::U0 => 1001, E65::U1 => 1002, E65::U2 => 1003, E65::U3 => 1004, E65::U4 => 1005 });
type P66 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E66 { Live(Tr<P66>), U0, U1, U2, U3, U4 }
suite!(66, s66, E66, [E66::Live(Tr(unsafe { transmute::<isize, P66>((-61isize)) })), E66::Live(Tr(unsafe { transmute::<isize, P66>((-33isize)) })), E66::Live(Tr(unsafe { transmute::<isize, P66>((-4isize)) })), E66::U0, E66::U1, E66::U2, E66::U3, E66::U4], |e| match e { E66::Live(w) => { let p: P66 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E66::U0 => 1001, E66::U1 => 1002, E66::U2 => 1003, E66::U3 => 1004, E66::U4 => 1005 });
type P67 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E67 { Live(Named<P67>), U0, U1, U2, U3, U4 }
suite!(67, s67, E67, [E67::Live(Named { z: (), v: unsafe { transmute::<isize, P67>((-61isize)) }, p: PhantomData }), E67::Live(Named { z: (), v: unsafe { transmute::<isize, P67>((-33isize)) }, p: PhantomData }), E67::Live(Named { z: (), v: unsafe { transmute::<isize, P67>((-4isize)) }, p: PhantomData }), E67::U0, E67::U1, E67::U2, E67::U3, E67::U4], |e| match e { E67::Live(w) => { let p: P67 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E67::U0 => 1001, E67::U1 => 1002, E67::U2 => 1003, E67::U3 => 1004, E67::U4 => 1005 });
type P68 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E68 { Live(((), P68)), U0, U1, U2, U3, U4 }
suite!(68, s68, E68, [E68::Live(((), unsafe { transmute::<isize, P68>((-61isize)) })), E68::Live(((), unsafe { transmute::<isize, P68>((-33isize)) })), E68::Live(((), unsafe { transmute::<isize, P68>((-4isize)) })), E68::U0, E68::U1, E68::U2, E68::U3, E68::U4], |e| match e { E68::Live(w) => { let p: P68 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E68::U0 => 1001, E68::U1 => 1002, E68::U2 => 1003, E68::U3 => 1004, E68::U4 => 1005 });
type P69 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E69 { Live(SL<P69>), U0, U1, U2, U3, U4 }
suite!(69, s69, E69, [E69::Live(SL::Live(unsafe { transmute::<isize, P69>((-61isize)) })), E69::Live(SL::Live(unsafe { transmute::<isize, P69>((-33isize)) })), E69::Live(SL::Live(unsafe { transmute::<isize, P69>((-4isize)) })), E69::U0, E69::U1, E69::U2, E69::U3, E69::U4], |e| match e { E69::Live(w) => { let p: P69 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E69::U0 => 1001, E69::U1 => 1002, E69::U2 => 1003, E69::U3 => 1004, E69::U4 => 1005 });
type P70 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E70 { Live(ManuallyDrop<P70>), U0, U1, U2, U3, U4 }
suite!(70, s70, E70, [E70::Live(ManuallyDrop::new(unsafe { transmute::<isize, P70>((-61isize)) })), E70::Live(ManuallyDrop::new(unsafe { transmute::<isize, P70>((-33isize)) })), E70::Live(ManuallyDrop::new(unsafe { transmute::<isize, P70>((-4isize)) })), E70::U0, E70::U1, E70::U2, E70::U3, E70::U4], |e| match e { E70::Live(w) => { let p: P70 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E70::U0 => 1001, E70::U1 => 1002, E70::U2 => 1003, E70::U3 => 1004, E70::U4 => 1005 });
type P71 = pattern_type!(isize is -61isize..=-4isize);
#[derive(Clone, Copy)] enum E71 { Live(Wrap<SL<Tr<P71>>>), U0, U1, U2, U3, U4 }
suite!(71, s71, E71, [E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P71>((-61isize)) })))), E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P71>((-33isize)) })))), E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P71>((-4isize)) })))), E71::U0, E71::U1, E71::U2, E71::U3, E71::U4], |e| match e { E71::Live(w) => { let p: P71 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E71::U0 => 1001, E71::U1 => 1002, E71::U2 => 1003, E71::U3 => 1004, E71::U4 => 1005 });
type P72 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E72 { Live(P72), U0, U1, U2 }
suite!(72, s72, E72, [E72::Live(unsafe { transmute::<isize, P72>(2isize) }), E72::Live(unsafe { transmute::<isize, P72>(5isize) }), E72::Live(unsafe { transmute::<isize, P72>(9isize) }), E72::U0, E72::U1, E72::U2], |e| match e { E72::Live(w) => { let p: P72 = w; let b: isize = unsafe { transmute(p) }; b as u32 }, E72::U0 => 1001, E72::U1 => 1002, E72::U2 => 1003 });
type P73 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E73 { Live(Wrap<P73>), U0, U1, U2 }
suite!(73, s73, E73, [E73::Live(Wrap(unsafe { transmute::<isize, P73>(2isize) })), E73::Live(Wrap(unsafe { transmute::<isize, P73>(5isize) })), E73::Live(Wrap(unsafe { transmute::<isize, P73>(9isize) })), E73::U0, E73::U1, E73::U2], |e| match e { E73::Live(w) => { let p: P73 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E73::U0 => 1001, E73::U1 => 1002, E73::U2 => 1003 });
type P74 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E74 { Live(Tr<P74>), U0, U1, U2 }
suite!(74, s74, E74, [E74::Live(Tr(unsafe { transmute::<isize, P74>(2isize) })), E74::Live(Tr(unsafe { transmute::<isize, P74>(5isize) })), E74::Live(Tr(unsafe { transmute::<isize, P74>(9isize) })), E74::U0, E74::U1, E74::U2], |e| match e { E74::Live(w) => { let p: P74 = w.0; let b: isize = unsafe { transmute(p) }; b as u32 }, E74::U0 => 1001, E74::U1 => 1002, E74::U2 => 1003 });
type P75 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E75 { Live(Named<P75>), U0, U1, U2 }
suite!(75, s75, E75, [E75::Live(Named { z: (), v: unsafe { transmute::<isize, P75>(2isize) }, p: PhantomData }), E75::Live(Named { z: (), v: unsafe { transmute::<isize, P75>(5isize) }, p: PhantomData }), E75::Live(Named { z: (), v: unsafe { transmute::<isize, P75>(9isize) }, p: PhantomData }), E75::U0, E75::U1, E75::U2], |e| match e { E75::Live(w) => { let p: P75 = w.v; let b: isize = unsafe { transmute(p) }; b as u32 }, E75::U0 => 1001, E75::U1 => 1002, E75::U2 => 1003 });
type P76 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E76 { Live(((), P76)), U0, U1, U2 }
suite!(76, s76, E76, [E76::Live(((), unsafe { transmute::<isize, P76>(2isize) })), E76::Live(((), unsafe { transmute::<isize, P76>(5isize) })), E76::Live(((), unsafe { transmute::<isize, P76>(9isize) })), E76::U0, E76::U1, E76::U2], |e| match e { E76::Live(w) => { let p: P76 = w.1; let b: isize = unsafe { transmute(p) }; b as u32 }, E76::U0 => 1001, E76::U1 => 1002, E76::U2 => 1003 });
type P77 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E77 { Live(SL<P77>), U0, U1, U2 }
suite!(77, s77, E77, [E77::Live(SL::Live(unsafe { transmute::<isize, P77>(2isize) })), E77::Live(SL::Live(unsafe { transmute::<isize, P77>(5isize) })), E77::Live(SL::Live(unsafe { transmute::<isize, P77>(9isize) })), E77::U0, E77::U1, E77::U2], |e| match e { E77::Live(w) => { let p: P77 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E77::U0 => 1001, E77::U1 => 1002, E77::U2 => 1003 });
type P78 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E78 { Live(ManuallyDrop<P78>), U0, U1, U2 }
suite!(78, s78, E78, [E78::Live(ManuallyDrop::new(unsafe { transmute::<isize, P78>(2isize) })), E78::Live(ManuallyDrop::new(unsafe { transmute::<isize, P78>(5isize) })), E78::Live(ManuallyDrop::new(unsafe { transmute::<isize, P78>(9isize) })), E78::U0, E78::U1, E78::U2], |e| match e { E78::Live(w) => { let p: P78 = ManuallyDrop::into_inner(w); let b: isize = unsafe { transmute(p) }; b as u32 }, E78::U0 => 1001, E78::U1 => 1002, E78::U2 => 1003 });
type P79 = pattern_type!(isize is 2isize..=9isize);
#[derive(Clone, Copy)] enum E79 { Live(Wrap<SL<Tr<P79>>>), U0, U1, U2 }
suite!(79, s79, E79, [E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P79>(2isize) })))), E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P79>(5isize) })))), E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<isize, P79>(9isize) })))), E79::U0, E79::U1, E79::U2], |e| match e { E79::Live(w) => { let p: P79 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: isize = unsafe { transmute(p) }; b as u32 }, E79::U0 => 1001, E79::U1 => 1002, E79::U2 => 1003 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s0(); s1(); s2(); s3(); s4(); s5(); s6(); s7(); s8(); s9(); s10(); s11(); s12(); s13(); s14(); s15(); s16(); s17(); s18(); s19(); s20(); s21(); s22(); s23(); s24(); s25(); s26(); s27(); s28(); s29(); s30(); s31(); s32(); s33(); s34(); s35(); s36(); s37(); s38(); s39(); s40(); s41(); s42(); s43(); s44(); s45(); s46(); s47(); s48(); s49(); s50(); s51(); s52(); s53(); s54(); s55(); s56(); s57(); s58(); s59(); s60(); s61(); s62(); s63(); s64(); s65(); s66(); s67(); s68(); s69(); s70(); s71(); s72(); s73(); s74(); s75(); s76(); s77(); s78(); s79();
}
