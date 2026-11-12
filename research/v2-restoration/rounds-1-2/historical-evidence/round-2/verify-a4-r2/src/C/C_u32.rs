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
type P0 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E0 { Live(P0), U0 }
suite!(0, s0, E0, [E0::Live(unsafe { transmute::<u32, P0>(4294967235u32) }), E0::Live(unsafe { transmute::<u32, P0>(4294967264u32) }), E0::Live(unsafe { transmute::<u32, P0>(4294967294u32) }), E0::U0], |e| match e { E0::Live(w) => { let p: P0 = w; let b: u32 = unsafe { transmute(p) }; b }, E0::U0 => 1001 });
type P1 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E1 { Live(Wrap<P1>), U0 }
suite!(1, s1, E1, [E1::Live(Wrap(unsafe { transmute::<u32, P1>(4294967235u32) })), E1::Live(Wrap(unsafe { transmute::<u32, P1>(4294967264u32) })), E1::Live(Wrap(unsafe { transmute::<u32, P1>(4294967294u32) })), E1::U0], |e| match e { E1::Live(w) => { let p: P1 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E1::U0 => 1001 });
type P2 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E2 { Live(Tr<P2>), U0 }
suite!(2, s2, E2, [E2::Live(Tr(unsafe { transmute::<u32, P2>(4294967235u32) })), E2::Live(Tr(unsafe { transmute::<u32, P2>(4294967264u32) })), E2::Live(Tr(unsafe { transmute::<u32, P2>(4294967294u32) })), E2::U0], |e| match e { E2::Live(w) => { let p: P2 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E2::U0 => 1001 });
type P3 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E3 { Live(Named<P3>), U0 }
suite!(3, s3, E3, [E3::Live(Named { z: (), v: unsafe { transmute::<u32, P3>(4294967235u32) }, p: PhantomData }), E3::Live(Named { z: (), v: unsafe { transmute::<u32, P3>(4294967264u32) }, p: PhantomData }), E3::Live(Named { z: (), v: unsafe { transmute::<u32, P3>(4294967294u32) }, p: PhantomData }), E3::U0], |e| match e { E3::Live(w) => { let p: P3 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E3::U0 => 1001 });
type P4 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E4 { Live(((), P4)), U0 }
suite!(4, s4, E4, [E4::Live(((), unsafe { transmute::<u32, P4>(4294967235u32) })), E4::Live(((), unsafe { transmute::<u32, P4>(4294967264u32) })), E4::Live(((), unsafe { transmute::<u32, P4>(4294967294u32) })), E4::U0], |e| match e { E4::Live(w) => { let p: P4 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E4::U0 => 1001 });
type P5 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E5 { Live(SL<P5>), U0 }
suite!(5, s5, E5, [E5::Live(SL::Live(unsafe { transmute::<u32, P5>(4294967235u32) })), E5::Live(SL::Live(unsafe { transmute::<u32, P5>(4294967264u32) })), E5::Live(SL::Live(unsafe { transmute::<u32, P5>(4294967294u32) })), E5::U0], |e| match e { E5::Live(w) => { let p: P5 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E5::U0 => 1001 });
type P6 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E6 { Live(ManuallyDrop<P6>), U0 }
suite!(6, s6, E6, [E6::Live(ManuallyDrop::new(unsafe { transmute::<u32, P6>(4294967235u32) })), E6::Live(ManuallyDrop::new(unsafe { transmute::<u32, P6>(4294967264u32) })), E6::Live(ManuallyDrop::new(unsafe { transmute::<u32, P6>(4294967294u32) })), E6::U0], |e| match e { E6::Live(w) => { let p: P6 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E6::U0 => 1001 });
type P7 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E7 { Live(Wrap<SL<Tr<P7>>>), U0 }
suite!(7, s7, E7, [E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P7>(4294967235u32) })))), E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P7>(4294967264u32) })))), E7::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P7>(4294967294u32) })))), E7::U0], |e| match e { E7::Live(w) => { let p: P7 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E7::U0 => 1001 });
type P8 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E8 { Live(P8), U0, U1 }
suite!(8, s8, E8, [E8::Live(unsafe { transmute::<u32, P8>(4294967235u32) }), E8::Live(unsafe { transmute::<u32, P8>(4294967264u32) }), E8::Live(unsafe { transmute::<u32, P8>(4294967294u32) }), E8::U0, E8::U1], |e| match e { E8::Live(w) => { let p: P8 = w; let b: u32 = unsafe { transmute(p) }; b }, E8::U0 => 1001, E8::U1 => 1002 });
type P9 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E9 { Live(Wrap<P9>), U0, U1 }
suite!(9, s9, E9, [E9::Live(Wrap(unsafe { transmute::<u32, P9>(4294967235u32) })), E9::Live(Wrap(unsafe { transmute::<u32, P9>(4294967264u32) })), E9::Live(Wrap(unsafe { transmute::<u32, P9>(4294967294u32) })), E9::U0, E9::U1], |e| match e { E9::Live(w) => { let p: P9 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E9::U0 => 1001, E9::U1 => 1002 });
type P10 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E10 { Live(Tr<P10>), U0, U1 }
suite!(10, s10, E10, [E10::Live(Tr(unsafe { transmute::<u32, P10>(4294967235u32) })), E10::Live(Tr(unsafe { transmute::<u32, P10>(4294967264u32) })), E10::Live(Tr(unsafe { transmute::<u32, P10>(4294967294u32) })), E10::U0, E10::U1], |e| match e { E10::Live(w) => { let p: P10 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E10::U0 => 1001, E10::U1 => 1002 });
type P11 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E11 { Live(Named<P11>), U0, U1 }
suite!(11, s11, E11, [E11::Live(Named { z: (), v: unsafe { transmute::<u32, P11>(4294967235u32) }, p: PhantomData }), E11::Live(Named { z: (), v: unsafe { transmute::<u32, P11>(4294967264u32) }, p: PhantomData }), E11::Live(Named { z: (), v: unsafe { transmute::<u32, P11>(4294967294u32) }, p: PhantomData }), E11::U0, E11::U1], |e| match e { E11::Live(w) => { let p: P11 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E11::U0 => 1001, E11::U1 => 1002 });
type P12 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E12 { Live(((), P12)), U0, U1 }
suite!(12, s12, E12, [E12::Live(((), unsafe { transmute::<u32, P12>(4294967235u32) })), E12::Live(((), unsafe { transmute::<u32, P12>(4294967264u32) })), E12::Live(((), unsafe { transmute::<u32, P12>(4294967294u32) })), E12::U0, E12::U1], |e| match e { E12::Live(w) => { let p: P12 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E12::U0 => 1001, E12::U1 => 1002 });
type P13 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E13 { Live(SL<P13>), U0, U1 }
suite!(13, s13, E13, [E13::Live(SL::Live(unsafe { transmute::<u32, P13>(4294967235u32) })), E13::Live(SL::Live(unsafe { transmute::<u32, P13>(4294967264u32) })), E13::Live(SL::Live(unsafe { transmute::<u32, P13>(4294967294u32) })), E13::U0, E13::U1], |e| match e { E13::Live(w) => { let p: P13 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E13::U0 => 1001, E13::U1 => 1002 });
type P14 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E14 { Live(ManuallyDrop<P14>), U0, U1 }
suite!(14, s14, E14, [E14::Live(ManuallyDrop::new(unsafe { transmute::<u32, P14>(4294967235u32) })), E14::Live(ManuallyDrop::new(unsafe { transmute::<u32, P14>(4294967264u32) })), E14::Live(ManuallyDrop::new(unsafe { transmute::<u32, P14>(4294967294u32) })), E14::U0, E14::U1], |e| match e { E14::Live(w) => { let p: P14 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E14::U0 => 1001, E14::U1 => 1002 });
type P15 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E15 { Live(Wrap<SL<Tr<P15>>>), U0, U1 }
suite!(15, s15, E15, [E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P15>(4294967235u32) })))), E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P15>(4294967264u32) })))), E15::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P15>(4294967294u32) })))), E15::U0, E15::U1], |e| match e { E15::Live(w) => { let p: P15 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E15::U0 => 1001, E15::U1 => 1002 });
type P16 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E16 { Live(P16), U0, U1, U2 }
suite!(16, s16, E16, [E16::Live(unsafe { transmute::<u32, P16>(4294967235u32) }), E16::Live(unsafe { transmute::<u32, P16>(4294967264u32) }), E16::Live(unsafe { transmute::<u32, P16>(4294967294u32) }), E16::U0, E16::U1, E16::U2], |e| match e { E16::Live(w) => { let p: P16 = w; let b: u32 = unsafe { transmute(p) }; b }, E16::U0 => 1001, E16::U1 => 1002, E16::U2 => 1003 });
type P17 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E17 { Live(Wrap<P17>), U0, U1, U2 }
suite!(17, s17, E17, [E17::Live(Wrap(unsafe { transmute::<u32, P17>(4294967235u32) })), E17::Live(Wrap(unsafe { transmute::<u32, P17>(4294967264u32) })), E17::Live(Wrap(unsafe { transmute::<u32, P17>(4294967294u32) })), E17::U0, E17::U1, E17::U2], |e| match e { E17::Live(w) => { let p: P17 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E17::U0 => 1001, E17::U1 => 1002, E17::U2 => 1003 });
type P18 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E18 { Live(Tr<P18>), U0, U1, U2 }
suite!(18, s18, E18, [E18::Live(Tr(unsafe { transmute::<u32, P18>(4294967235u32) })), E18::Live(Tr(unsafe { transmute::<u32, P18>(4294967264u32) })), E18::Live(Tr(unsafe { transmute::<u32, P18>(4294967294u32) })), E18::U0, E18::U1, E18::U2], |e| match e { E18::Live(w) => { let p: P18 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E18::U0 => 1001, E18::U1 => 1002, E18::U2 => 1003 });
type P19 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E19 { Live(Named<P19>), U0, U1, U2 }
suite!(19, s19, E19, [E19::Live(Named { z: (), v: unsafe { transmute::<u32, P19>(4294967235u32) }, p: PhantomData }), E19::Live(Named { z: (), v: unsafe { transmute::<u32, P19>(4294967264u32) }, p: PhantomData }), E19::Live(Named { z: (), v: unsafe { transmute::<u32, P19>(4294967294u32) }, p: PhantomData }), E19::U0, E19::U1, E19::U2], |e| match e { E19::Live(w) => { let p: P19 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E19::U0 => 1001, E19::U1 => 1002, E19::U2 => 1003 });
type P20 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E20 { Live(((), P20)), U0, U1, U2 }
suite!(20, s20, E20, [E20::Live(((), unsafe { transmute::<u32, P20>(4294967235u32) })), E20::Live(((), unsafe { transmute::<u32, P20>(4294967264u32) })), E20::Live(((), unsafe { transmute::<u32, P20>(4294967294u32) })), E20::U0, E20::U1, E20::U2], |e| match e { E20::Live(w) => { let p: P20 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E20::U0 => 1001, E20::U1 => 1002, E20::U2 => 1003 });
type P21 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E21 { Live(SL<P21>), U0, U1, U2 }
suite!(21, s21, E21, [E21::Live(SL::Live(unsafe { transmute::<u32, P21>(4294967235u32) })), E21::Live(SL::Live(unsafe { transmute::<u32, P21>(4294967264u32) })), E21::Live(SL::Live(unsafe { transmute::<u32, P21>(4294967294u32) })), E21::U0, E21::U1, E21::U2], |e| match e { E21::Live(w) => { let p: P21 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E21::U0 => 1001, E21::U1 => 1002, E21::U2 => 1003 });
type P22 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E22 { Live(ManuallyDrop<P22>), U0, U1, U2 }
suite!(22, s22, E22, [E22::Live(ManuallyDrop::new(unsafe { transmute::<u32, P22>(4294967235u32) })), E22::Live(ManuallyDrop::new(unsafe { transmute::<u32, P22>(4294967264u32) })), E22::Live(ManuallyDrop::new(unsafe { transmute::<u32, P22>(4294967294u32) })), E22::U0, E22::U1, E22::U2], |e| match e { E22::Live(w) => { let p: P22 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E22::U0 => 1001, E22::U1 => 1002, E22::U2 => 1003 });
type P23 = pattern_type!(u32 is 4294967235u32..=4294967294u32);
#[derive(Clone, Copy)] enum E23 { Live(Wrap<SL<Tr<P23>>>), U0, U1, U2 }
suite!(23, s23, E23, [E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P23>(4294967235u32) })))), E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P23>(4294967264u32) })))), E23::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P23>(4294967294u32) })))), E23::U0, E23::U1, E23::U2], |e| match e { E23::Live(w) => { let p: P23 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E23::U0 => 1001, E23::U1 => 1002, E23::U2 => 1003 });
type P24 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E24 { Live(P24), U0, U1 }
suite!(24, s24, E24, [E24::Live(unsafe { transmute::<u32, P24>(4294967235u32) }), E24::Live(unsafe { transmute::<u32, P24>(4294967264u32) }), E24::Live(unsafe { transmute::<u32, P24>(4294967293u32) }), E24::U0, E24::U1], |e| match e { E24::Live(w) => { let p: P24 = w; let b: u32 = unsafe { transmute(p) }; b }, E24::U0 => 1001, E24::U1 => 1002 });
type P25 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E25 { Live(Wrap<P25>), U0, U1 }
suite!(25, s25, E25, [E25::Live(Wrap(unsafe { transmute::<u32, P25>(4294967235u32) })), E25::Live(Wrap(unsafe { transmute::<u32, P25>(4294967264u32) })), E25::Live(Wrap(unsafe { transmute::<u32, P25>(4294967293u32) })), E25::U0, E25::U1], |e| match e { E25::Live(w) => { let p: P25 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E25::U0 => 1001, E25::U1 => 1002 });
type P26 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E26 { Live(Tr<P26>), U0, U1 }
suite!(26, s26, E26, [E26::Live(Tr(unsafe { transmute::<u32, P26>(4294967235u32) })), E26::Live(Tr(unsafe { transmute::<u32, P26>(4294967264u32) })), E26::Live(Tr(unsafe { transmute::<u32, P26>(4294967293u32) })), E26::U0, E26::U1], |e| match e { E26::Live(w) => { let p: P26 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E26::U0 => 1001, E26::U1 => 1002 });
type P27 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E27 { Live(Named<P27>), U0, U1 }
suite!(27, s27, E27, [E27::Live(Named { z: (), v: unsafe { transmute::<u32, P27>(4294967235u32) }, p: PhantomData }), E27::Live(Named { z: (), v: unsafe { transmute::<u32, P27>(4294967264u32) }, p: PhantomData }), E27::Live(Named { z: (), v: unsafe { transmute::<u32, P27>(4294967293u32) }, p: PhantomData }), E27::U0, E27::U1], |e| match e { E27::Live(w) => { let p: P27 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E27::U0 => 1001, E27::U1 => 1002 });
type P28 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E28 { Live(((), P28)), U0, U1 }
suite!(28, s28, E28, [E28::Live(((), unsafe { transmute::<u32, P28>(4294967235u32) })), E28::Live(((), unsafe { transmute::<u32, P28>(4294967264u32) })), E28::Live(((), unsafe { transmute::<u32, P28>(4294967293u32) })), E28::U0, E28::U1], |e| match e { E28::Live(w) => { let p: P28 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E28::U0 => 1001, E28::U1 => 1002 });
type P29 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E29 { Live(SL<P29>), U0, U1 }
suite!(29, s29, E29, [E29::Live(SL::Live(unsafe { transmute::<u32, P29>(4294967235u32) })), E29::Live(SL::Live(unsafe { transmute::<u32, P29>(4294967264u32) })), E29::Live(SL::Live(unsafe { transmute::<u32, P29>(4294967293u32) })), E29::U0, E29::U1], |e| match e { E29::Live(w) => { let p: P29 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E29::U0 => 1001, E29::U1 => 1002 });
type P30 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E30 { Live(ManuallyDrop<P30>), U0, U1 }
suite!(30, s30, E30, [E30::Live(ManuallyDrop::new(unsafe { transmute::<u32, P30>(4294967235u32) })), E30::Live(ManuallyDrop::new(unsafe { transmute::<u32, P30>(4294967264u32) })), E30::Live(ManuallyDrop::new(unsafe { transmute::<u32, P30>(4294967293u32) })), E30::U0, E30::U1], |e| match e { E30::Live(w) => { let p: P30 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E30::U0 => 1001, E30::U1 => 1002 });
type P31 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E31 { Live(Wrap<SL<Tr<P31>>>), U0, U1 }
suite!(31, s31, E31, [E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P31>(4294967235u32) })))), E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P31>(4294967264u32) })))), E31::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P31>(4294967293u32) })))), E31::U0, E31::U1], |e| match e { E31::Live(w) => { let p: P31 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E31::U0 => 1001, E31::U1 => 1002 });
type P32 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E32 { Live(P32), U0, U1, U2 }
suite!(32, s32, E32, [E32::Live(unsafe { transmute::<u32, P32>(4294967235u32) }), E32::Live(unsafe { transmute::<u32, P32>(4294967264u32) }), E32::Live(unsafe { transmute::<u32, P32>(4294967293u32) }), E32::U0, E32::U1, E32::U2], |e| match e { E32::Live(w) => { let p: P32 = w; let b: u32 = unsafe { transmute(p) }; b }, E32::U0 => 1001, E32::U1 => 1002, E32::U2 => 1003 });
type P33 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E33 { Live(Wrap<P33>), U0, U1, U2 }
suite!(33, s33, E33, [E33::Live(Wrap(unsafe { transmute::<u32, P33>(4294967235u32) })), E33::Live(Wrap(unsafe { transmute::<u32, P33>(4294967264u32) })), E33::Live(Wrap(unsafe { transmute::<u32, P33>(4294967293u32) })), E33::U0, E33::U1, E33::U2], |e| match e { E33::Live(w) => { let p: P33 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E33::U0 => 1001, E33::U1 => 1002, E33::U2 => 1003 });
type P34 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E34 { Live(Tr<P34>), U0, U1, U2 }
suite!(34, s34, E34, [E34::Live(Tr(unsafe { transmute::<u32, P34>(4294967235u32) })), E34::Live(Tr(unsafe { transmute::<u32, P34>(4294967264u32) })), E34::Live(Tr(unsafe { transmute::<u32, P34>(4294967293u32) })), E34::U0, E34::U1, E34::U2], |e| match e { E34::Live(w) => { let p: P34 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E34::U0 => 1001, E34::U1 => 1002, E34::U2 => 1003 });
type P35 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E35 { Live(Named<P35>), U0, U1, U2 }
suite!(35, s35, E35, [E35::Live(Named { z: (), v: unsafe { transmute::<u32, P35>(4294967235u32) }, p: PhantomData }), E35::Live(Named { z: (), v: unsafe { transmute::<u32, P35>(4294967264u32) }, p: PhantomData }), E35::Live(Named { z: (), v: unsafe { transmute::<u32, P35>(4294967293u32) }, p: PhantomData }), E35::U0, E35::U1, E35::U2], |e| match e { E35::Live(w) => { let p: P35 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E35::U0 => 1001, E35::U1 => 1002, E35::U2 => 1003 });
type P36 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E36 { Live(((), P36)), U0, U1, U2 }
suite!(36, s36, E36, [E36::Live(((), unsafe { transmute::<u32, P36>(4294967235u32) })), E36::Live(((), unsafe { transmute::<u32, P36>(4294967264u32) })), E36::Live(((), unsafe { transmute::<u32, P36>(4294967293u32) })), E36::U0, E36::U1, E36::U2], |e| match e { E36::Live(w) => { let p: P36 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E36::U0 => 1001, E36::U1 => 1002, E36::U2 => 1003 });
type P37 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E37 { Live(SL<P37>), U0, U1, U2 }
suite!(37, s37, E37, [E37::Live(SL::Live(unsafe { transmute::<u32, P37>(4294967235u32) })), E37::Live(SL::Live(unsafe { transmute::<u32, P37>(4294967264u32) })), E37::Live(SL::Live(unsafe { transmute::<u32, P37>(4294967293u32) })), E37::U0, E37::U1, E37::U2], |e| match e { E37::Live(w) => { let p: P37 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E37::U0 => 1001, E37::U1 => 1002, E37::U2 => 1003 });
type P38 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E38 { Live(ManuallyDrop<P38>), U0, U1, U2 }
suite!(38, s38, E38, [E38::Live(ManuallyDrop::new(unsafe { transmute::<u32, P38>(4294967235u32) })), E38::Live(ManuallyDrop::new(unsafe { transmute::<u32, P38>(4294967264u32) })), E38::Live(ManuallyDrop::new(unsafe { transmute::<u32, P38>(4294967293u32) })), E38::U0, E38::U1, E38::U2], |e| match e { E38::Live(w) => { let p: P38 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E38::U0 => 1001, E38::U1 => 1002, E38::U2 => 1003 });
type P39 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E39 { Live(Wrap<SL<Tr<P39>>>), U0, U1, U2 }
suite!(39, s39, E39, [E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P39>(4294967235u32) })))), E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P39>(4294967264u32) })))), E39::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P39>(4294967293u32) })))), E39::U0, E39::U1, E39::U2], |e| match e { E39::Live(w) => { let p: P39 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E39::U0 => 1001, E39::U1 => 1002, E39::U2 => 1003 });
type P40 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E40 { Live(P40), U0, U1, U2, U3 }
suite!(40, s40, E40, [E40::Live(unsafe { transmute::<u32, P40>(4294967235u32) }), E40::Live(unsafe { transmute::<u32, P40>(4294967264u32) }), E40::Live(unsafe { transmute::<u32, P40>(4294967293u32) }), E40::U0, E40::U1, E40::U2, E40::U3], |e| match e { E40::Live(w) => { let p: P40 = w; let b: u32 = unsafe { transmute(p) }; b }, E40::U0 => 1001, E40::U1 => 1002, E40::U2 => 1003, E40::U3 => 1004 });
type P41 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E41 { Live(Wrap<P41>), U0, U1, U2, U3 }
suite!(41, s41, E41, [E41::Live(Wrap(unsafe { transmute::<u32, P41>(4294967235u32) })), E41::Live(Wrap(unsafe { transmute::<u32, P41>(4294967264u32) })), E41::Live(Wrap(unsafe { transmute::<u32, P41>(4294967293u32) })), E41::U0, E41::U1, E41::U2, E41::U3], |e| match e { E41::Live(w) => { let p: P41 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E41::U0 => 1001, E41::U1 => 1002, E41::U2 => 1003, E41::U3 => 1004 });
type P42 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E42 { Live(Tr<P42>), U0, U1, U2, U3 }
suite!(42, s42, E42, [E42::Live(Tr(unsafe { transmute::<u32, P42>(4294967235u32) })), E42::Live(Tr(unsafe { transmute::<u32, P42>(4294967264u32) })), E42::Live(Tr(unsafe { transmute::<u32, P42>(4294967293u32) })), E42::U0, E42::U1, E42::U2, E42::U3], |e| match e { E42::Live(w) => { let p: P42 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E42::U0 => 1001, E42::U1 => 1002, E42::U2 => 1003, E42::U3 => 1004 });
type P43 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E43 { Live(Named<P43>), U0, U1, U2, U3 }
suite!(43, s43, E43, [E43::Live(Named { z: (), v: unsafe { transmute::<u32, P43>(4294967235u32) }, p: PhantomData }), E43::Live(Named { z: (), v: unsafe { transmute::<u32, P43>(4294967264u32) }, p: PhantomData }), E43::Live(Named { z: (), v: unsafe { transmute::<u32, P43>(4294967293u32) }, p: PhantomData }), E43::U0, E43::U1, E43::U2, E43::U3], |e| match e { E43::Live(w) => { let p: P43 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E43::U0 => 1001, E43::U1 => 1002, E43::U2 => 1003, E43::U3 => 1004 });
type P44 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E44 { Live(((), P44)), U0, U1, U2, U3 }
suite!(44, s44, E44, [E44::Live(((), unsafe { transmute::<u32, P44>(4294967235u32) })), E44::Live(((), unsafe { transmute::<u32, P44>(4294967264u32) })), E44::Live(((), unsafe { transmute::<u32, P44>(4294967293u32) })), E44::U0, E44::U1, E44::U2, E44::U3], |e| match e { E44::Live(w) => { let p: P44 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E44::U0 => 1001, E44::U1 => 1002, E44::U2 => 1003, E44::U3 => 1004 });
type P45 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E45 { Live(SL<P45>), U0, U1, U2, U3 }
suite!(45, s45, E45, [E45::Live(SL::Live(unsafe { transmute::<u32, P45>(4294967235u32) })), E45::Live(SL::Live(unsafe { transmute::<u32, P45>(4294967264u32) })), E45::Live(SL::Live(unsafe { transmute::<u32, P45>(4294967293u32) })), E45::U0, E45::U1, E45::U2, E45::U3], |e| match e { E45::Live(w) => { let p: P45 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E45::U0 => 1001, E45::U1 => 1002, E45::U2 => 1003, E45::U3 => 1004 });
type P46 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E46 { Live(ManuallyDrop<P46>), U0, U1, U2, U3 }
suite!(46, s46, E46, [E46::Live(ManuallyDrop::new(unsafe { transmute::<u32, P46>(4294967235u32) })), E46::Live(ManuallyDrop::new(unsafe { transmute::<u32, P46>(4294967264u32) })), E46::Live(ManuallyDrop::new(unsafe { transmute::<u32, P46>(4294967293u32) })), E46::U0, E46::U1, E46::U2, E46::U3], |e| match e { E46::Live(w) => { let p: P46 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E46::U0 => 1001, E46::U1 => 1002, E46::U2 => 1003, E46::U3 => 1004 });
type P47 = pattern_type!(u32 is 4294967235u32..=4294967293u32);
#[derive(Clone, Copy)] enum E47 { Live(Wrap<SL<Tr<P47>>>), U0, U1, U2, U3 }
suite!(47, s47, E47, [E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P47>(4294967235u32) })))), E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P47>(4294967264u32) })))), E47::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P47>(4294967293u32) })))), E47::U0, E47::U1, E47::U2, E47::U3], |e| match e { E47::Live(w) => { let p: P47 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E47::U0 => 1001, E47::U1 => 1002, E47::U2 => 1003, E47::U3 => 1004 });
type P48 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E48 { Live(P48), U0, U1, U2 }
suite!(48, s48, E48, [E48::Live(unsafe { transmute::<u32, P48>(4294967235u32) }), E48::Live(unsafe { transmute::<u32, P48>(4294967263u32) }), E48::Live(unsafe { transmute::<u32, P48>(4294967292u32) }), E48::U0, E48::U1, E48::U2], |e| match e { E48::Live(w) => { let p: P48 = w; let b: u32 = unsafe { transmute(p) }; b }, E48::U0 => 1001, E48::U1 => 1002, E48::U2 => 1003 });
type P49 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E49 { Live(Wrap<P49>), U0, U1, U2 }
suite!(49, s49, E49, [E49::Live(Wrap(unsafe { transmute::<u32, P49>(4294967235u32) })), E49::Live(Wrap(unsafe { transmute::<u32, P49>(4294967263u32) })), E49::Live(Wrap(unsafe { transmute::<u32, P49>(4294967292u32) })), E49::U0, E49::U1, E49::U2], |e| match e { E49::Live(w) => { let p: P49 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E49::U0 => 1001, E49::U1 => 1002, E49::U2 => 1003 });
type P50 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E50 { Live(Tr<P50>), U0, U1, U2 }
suite!(50, s50, E50, [E50::Live(Tr(unsafe { transmute::<u32, P50>(4294967235u32) })), E50::Live(Tr(unsafe { transmute::<u32, P50>(4294967263u32) })), E50::Live(Tr(unsafe { transmute::<u32, P50>(4294967292u32) })), E50::U0, E50::U1, E50::U2], |e| match e { E50::Live(w) => { let p: P50 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E50::U0 => 1001, E50::U1 => 1002, E50::U2 => 1003 });
type P51 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E51 { Live(Named<P51>), U0, U1, U2 }
suite!(51, s51, E51, [E51::Live(Named { z: (), v: unsafe { transmute::<u32, P51>(4294967235u32) }, p: PhantomData }), E51::Live(Named { z: (), v: unsafe { transmute::<u32, P51>(4294967263u32) }, p: PhantomData }), E51::Live(Named { z: (), v: unsafe { transmute::<u32, P51>(4294967292u32) }, p: PhantomData }), E51::U0, E51::U1, E51::U2], |e| match e { E51::Live(w) => { let p: P51 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E51::U0 => 1001, E51::U1 => 1002, E51::U2 => 1003 });
type P52 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E52 { Live(((), P52)), U0, U1, U2 }
suite!(52, s52, E52, [E52::Live(((), unsafe { transmute::<u32, P52>(4294967235u32) })), E52::Live(((), unsafe { transmute::<u32, P52>(4294967263u32) })), E52::Live(((), unsafe { transmute::<u32, P52>(4294967292u32) })), E52::U0, E52::U1, E52::U2], |e| match e { E52::Live(w) => { let p: P52 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E52::U0 => 1001, E52::U1 => 1002, E52::U2 => 1003 });
type P53 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E53 { Live(SL<P53>), U0, U1, U2 }
suite!(53, s53, E53, [E53::Live(SL::Live(unsafe { transmute::<u32, P53>(4294967235u32) })), E53::Live(SL::Live(unsafe { transmute::<u32, P53>(4294967263u32) })), E53::Live(SL::Live(unsafe { transmute::<u32, P53>(4294967292u32) })), E53::U0, E53::U1, E53::U2], |e| match e { E53::Live(w) => { let p: P53 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E53::U0 => 1001, E53::U1 => 1002, E53::U2 => 1003 });
type P54 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E54 { Live(ManuallyDrop<P54>), U0, U1, U2 }
suite!(54, s54, E54, [E54::Live(ManuallyDrop::new(unsafe { transmute::<u32, P54>(4294967235u32) })), E54::Live(ManuallyDrop::new(unsafe { transmute::<u32, P54>(4294967263u32) })), E54::Live(ManuallyDrop::new(unsafe { transmute::<u32, P54>(4294967292u32) })), E54::U0, E54::U1, E54::U2], |e| match e { E54::Live(w) => { let p: P54 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E54::U0 => 1001, E54::U1 => 1002, E54::U2 => 1003 });
type P55 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E55 { Live(Wrap<SL<Tr<P55>>>), U0, U1, U2 }
suite!(55, s55, E55, [E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P55>(4294967235u32) })))), E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P55>(4294967263u32) })))), E55::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P55>(4294967292u32) })))), E55::U0, E55::U1, E55::U2], |e| match e { E55::Live(w) => { let p: P55 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E55::U0 => 1001, E55::U1 => 1002, E55::U2 => 1003 });
type P56 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E56 { Live(P56), U0, U1, U2, U3 }
suite!(56, s56, E56, [E56::Live(unsafe { transmute::<u32, P56>(4294967235u32) }), E56::Live(unsafe { transmute::<u32, P56>(4294967263u32) }), E56::Live(unsafe { transmute::<u32, P56>(4294967292u32) }), E56::U0, E56::U1, E56::U2, E56::U3], |e| match e { E56::Live(w) => { let p: P56 = w; let b: u32 = unsafe { transmute(p) }; b }, E56::U0 => 1001, E56::U1 => 1002, E56::U2 => 1003, E56::U3 => 1004 });
type P57 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E57 { Live(Wrap<P57>), U0, U1, U2, U3 }
suite!(57, s57, E57, [E57::Live(Wrap(unsafe { transmute::<u32, P57>(4294967235u32) })), E57::Live(Wrap(unsafe { transmute::<u32, P57>(4294967263u32) })), E57::Live(Wrap(unsafe { transmute::<u32, P57>(4294967292u32) })), E57::U0, E57::U1, E57::U2, E57::U3], |e| match e { E57::Live(w) => { let p: P57 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E57::U0 => 1001, E57::U1 => 1002, E57::U2 => 1003, E57::U3 => 1004 });
type P58 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E58 { Live(Tr<P58>), U0, U1, U2, U3 }
suite!(58, s58, E58, [E58::Live(Tr(unsafe { transmute::<u32, P58>(4294967235u32) })), E58::Live(Tr(unsafe { transmute::<u32, P58>(4294967263u32) })), E58::Live(Tr(unsafe { transmute::<u32, P58>(4294967292u32) })), E58::U0, E58::U1, E58::U2, E58::U3], |e| match e { E58::Live(w) => { let p: P58 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E58::U0 => 1001, E58::U1 => 1002, E58::U2 => 1003, E58::U3 => 1004 });
type P59 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E59 { Live(Named<P59>), U0, U1, U2, U3 }
suite!(59, s59, E59, [E59::Live(Named { z: (), v: unsafe { transmute::<u32, P59>(4294967235u32) }, p: PhantomData }), E59::Live(Named { z: (), v: unsafe { transmute::<u32, P59>(4294967263u32) }, p: PhantomData }), E59::Live(Named { z: (), v: unsafe { transmute::<u32, P59>(4294967292u32) }, p: PhantomData }), E59::U0, E59::U1, E59::U2, E59::U3], |e| match e { E59::Live(w) => { let p: P59 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E59::U0 => 1001, E59::U1 => 1002, E59::U2 => 1003, E59::U3 => 1004 });
type P60 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E60 { Live(((), P60)), U0, U1, U2, U3 }
suite!(60, s60, E60, [E60::Live(((), unsafe { transmute::<u32, P60>(4294967235u32) })), E60::Live(((), unsafe { transmute::<u32, P60>(4294967263u32) })), E60::Live(((), unsafe { transmute::<u32, P60>(4294967292u32) })), E60::U0, E60::U1, E60::U2, E60::U3], |e| match e { E60::Live(w) => { let p: P60 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E60::U0 => 1001, E60::U1 => 1002, E60::U2 => 1003, E60::U3 => 1004 });
type P61 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E61 { Live(SL<P61>), U0, U1, U2, U3 }
suite!(61, s61, E61, [E61::Live(SL::Live(unsafe { transmute::<u32, P61>(4294967235u32) })), E61::Live(SL::Live(unsafe { transmute::<u32, P61>(4294967263u32) })), E61::Live(SL::Live(unsafe { transmute::<u32, P61>(4294967292u32) })), E61::U0, E61::U1, E61::U2, E61::U3], |e| match e { E61::Live(w) => { let p: P61 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E61::U0 => 1001, E61::U1 => 1002, E61::U2 => 1003, E61::U3 => 1004 });
type P62 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E62 { Live(ManuallyDrop<P62>), U0, U1, U2, U3 }
suite!(62, s62, E62, [E62::Live(ManuallyDrop::new(unsafe { transmute::<u32, P62>(4294967235u32) })), E62::Live(ManuallyDrop::new(unsafe { transmute::<u32, P62>(4294967263u32) })), E62::Live(ManuallyDrop::new(unsafe { transmute::<u32, P62>(4294967292u32) })), E62::U0, E62::U1, E62::U2, E62::U3], |e| match e { E62::Live(w) => { let p: P62 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E62::U0 => 1001, E62::U1 => 1002, E62::U2 => 1003, E62::U3 => 1004 });
type P63 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E63 { Live(Wrap<SL<Tr<P63>>>), U0, U1, U2, U3 }
suite!(63, s63, E63, [E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P63>(4294967235u32) })))), E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P63>(4294967263u32) })))), E63::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P63>(4294967292u32) })))), E63::U0, E63::U1, E63::U2, E63::U3], |e| match e { E63::Live(w) => { let p: P63 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E63::U0 => 1001, E63::U1 => 1002, E63::U2 => 1003, E63::U3 => 1004 });
type P64 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E64 { Live(P64), U0, U1, U2, U3, U4 }
suite!(64, s64, E64, [E64::Live(unsafe { transmute::<u32, P64>(4294967235u32) }), E64::Live(unsafe { transmute::<u32, P64>(4294967263u32) }), E64::Live(unsafe { transmute::<u32, P64>(4294967292u32) }), E64::U0, E64::U1, E64::U2, E64::U3, E64::U4], |e| match e { E64::Live(w) => { let p: P64 = w; let b: u32 = unsafe { transmute(p) }; b }, E64::U0 => 1001, E64::U1 => 1002, E64::U2 => 1003, E64::U3 => 1004, E64::U4 => 1005 });
type P65 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E65 { Live(Wrap<P65>), U0, U1, U2, U3, U4 }
suite!(65, s65, E65, [E65::Live(Wrap(unsafe { transmute::<u32, P65>(4294967235u32) })), E65::Live(Wrap(unsafe { transmute::<u32, P65>(4294967263u32) })), E65::Live(Wrap(unsafe { transmute::<u32, P65>(4294967292u32) })), E65::U0, E65::U1, E65::U2, E65::U3, E65::U4], |e| match e { E65::Live(w) => { let p: P65 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E65::U0 => 1001, E65::U1 => 1002, E65::U2 => 1003, E65::U3 => 1004, E65::U4 => 1005 });
type P66 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E66 { Live(Tr<P66>), U0, U1, U2, U3, U4 }
suite!(66, s66, E66, [E66::Live(Tr(unsafe { transmute::<u32, P66>(4294967235u32) })), E66::Live(Tr(unsafe { transmute::<u32, P66>(4294967263u32) })), E66::Live(Tr(unsafe { transmute::<u32, P66>(4294967292u32) })), E66::U0, E66::U1, E66::U2, E66::U3, E66::U4], |e| match e { E66::Live(w) => { let p: P66 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E66::U0 => 1001, E66::U1 => 1002, E66::U2 => 1003, E66::U3 => 1004, E66::U4 => 1005 });
type P67 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E67 { Live(Named<P67>), U0, U1, U2, U3, U4 }
suite!(67, s67, E67, [E67::Live(Named { z: (), v: unsafe { transmute::<u32, P67>(4294967235u32) }, p: PhantomData }), E67::Live(Named { z: (), v: unsafe { transmute::<u32, P67>(4294967263u32) }, p: PhantomData }), E67::Live(Named { z: (), v: unsafe { transmute::<u32, P67>(4294967292u32) }, p: PhantomData }), E67::U0, E67::U1, E67::U2, E67::U3, E67::U4], |e| match e { E67::Live(w) => { let p: P67 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E67::U0 => 1001, E67::U1 => 1002, E67::U2 => 1003, E67::U3 => 1004, E67::U4 => 1005 });
type P68 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E68 { Live(((), P68)), U0, U1, U2, U3, U4 }
suite!(68, s68, E68, [E68::Live(((), unsafe { transmute::<u32, P68>(4294967235u32) })), E68::Live(((), unsafe { transmute::<u32, P68>(4294967263u32) })), E68::Live(((), unsafe { transmute::<u32, P68>(4294967292u32) })), E68::U0, E68::U1, E68::U2, E68::U3, E68::U4], |e| match e { E68::Live(w) => { let p: P68 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E68::U0 => 1001, E68::U1 => 1002, E68::U2 => 1003, E68::U3 => 1004, E68::U4 => 1005 });
type P69 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E69 { Live(SL<P69>), U0, U1, U2, U3, U4 }
suite!(69, s69, E69, [E69::Live(SL::Live(unsafe { transmute::<u32, P69>(4294967235u32) })), E69::Live(SL::Live(unsafe { transmute::<u32, P69>(4294967263u32) })), E69::Live(SL::Live(unsafe { transmute::<u32, P69>(4294967292u32) })), E69::U0, E69::U1, E69::U2, E69::U3, E69::U4], |e| match e { E69::Live(w) => { let p: P69 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E69::U0 => 1001, E69::U1 => 1002, E69::U2 => 1003, E69::U3 => 1004, E69::U4 => 1005 });
type P70 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E70 { Live(ManuallyDrop<P70>), U0, U1, U2, U3, U4 }
suite!(70, s70, E70, [E70::Live(ManuallyDrop::new(unsafe { transmute::<u32, P70>(4294967235u32) })), E70::Live(ManuallyDrop::new(unsafe { transmute::<u32, P70>(4294967263u32) })), E70::Live(ManuallyDrop::new(unsafe { transmute::<u32, P70>(4294967292u32) })), E70::U0, E70::U1, E70::U2, E70::U3, E70::U4], |e| match e { E70::Live(w) => { let p: P70 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E70::U0 => 1001, E70::U1 => 1002, E70::U2 => 1003, E70::U3 => 1004, E70::U4 => 1005 });
type P71 = pattern_type!(u32 is 4294967235u32..=4294967292u32);
#[derive(Clone, Copy)] enum E71 { Live(Wrap<SL<Tr<P71>>>), U0, U1, U2, U3, U4 }
suite!(71, s71, E71, [E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P71>(4294967235u32) })))), E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P71>(4294967263u32) })))), E71::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P71>(4294967292u32) })))), E71::U0, E71::U1, E71::U2, E71::U3, E71::U4], |e| match e { E71::Live(w) => { let p: P71 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E71::U0 => 1001, E71::U1 => 1002, E71::U2 => 1003, E71::U3 => 1004, E71::U4 => 1005 });
type P72 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E72 { Live(P72), U0 }
suite!(72, s72, E72, [E72::Live(unsafe { transmute::<u32, P72>(1u32) }), E72::Live(unsafe { transmute::<u32, P72>(2147483648u32) }), E72::Live(unsafe { transmute::<u32, P72>(4294967295u32) }), E72::U0], |e| match e { E72::Live(w) => { let p: P72 = w; let b: u32 = unsafe { transmute(p) }; b }, E72::U0 => 1001 });
type P73 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E73 { Live(Wrap<P73>), U0 }
suite!(73, s73, E73, [E73::Live(Wrap(unsafe { transmute::<u32, P73>(1u32) })), E73::Live(Wrap(unsafe { transmute::<u32, P73>(2147483648u32) })), E73::Live(Wrap(unsafe { transmute::<u32, P73>(4294967295u32) })), E73::U0], |e| match e { E73::Live(w) => { let p: P73 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E73::U0 => 1001 });
type P74 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E74 { Live(Tr<P74>), U0 }
suite!(74, s74, E74, [E74::Live(Tr(unsafe { transmute::<u32, P74>(1u32) })), E74::Live(Tr(unsafe { transmute::<u32, P74>(2147483648u32) })), E74::Live(Tr(unsafe { transmute::<u32, P74>(4294967295u32) })), E74::U0], |e| match e { E74::Live(w) => { let p: P74 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E74::U0 => 1001 });
type P75 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E75 { Live(Named<P75>), U0 }
suite!(75, s75, E75, [E75::Live(Named { z: (), v: unsafe { transmute::<u32, P75>(1u32) }, p: PhantomData }), E75::Live(Named { z: (), v: unsafe { transmute::<u32, P75>(2147483648u32) }, p: PhantomData }), E75::Live(Named { z: (), v: unsafe { transmute::<u32, P75>(4294967295u32) }, p: PhantomData }), E75::U0], |e| match e { E75::Live(w) => { let p: P75 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E75::U0 => 1001 });
type P76 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E76 { Live(((), P76)), U0 }
suite!(76, s76, E76, [E76::Live(((), unsafe { transmute::<u32, P76>(1u32) })), E76::Live(((), unsafe { transmute::<u32, P76>(2147483648u32) })), E76::Live(((), unsafe { transmute::<u32, P76>(4294967295u32) })), E76::U0], |e| match e { E76::Live(w) => { let p: P76 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E76::U0 => 1001 });
type P77 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E77 { Live(SL<P77>), U0 }
suite!(77, s77, E77, [E77::Live(SL::Live(unsafe { transmute::<u32, P77>(1u32) })), E77::Live(SL::Live(unsafe { transmute::<u32, P77>(2147483648u32) })), E77::Live(SL::Live(unsafe { transmute::<u32, P77>(4294967295u32) })), E77::U0], |e| match e { E77::Live(w) => { let p: P77 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E77::U0 => 1001 });
type P78 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E78 { Live(ManuallyDrop<P78>), U0 }
suite!(78, s78, E78, [E78::Live(ManuallyDrop::new(unsafe { transmute::<u32, P78>(1u32) })), E78::Live(ManuallyDrop::new(unsafe { transmute::<u32, P78>(2147483648u32) })), E78::Live(ManuallyDrop::new(unsafe { transmute::<u32, P78>(4294967295u32) })), E78::U0], |e| match e { E78::Live(w) => { let p: P78 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E78::U0 => 1001 });
type P79 = pattern_type!(u32 is 1u32..=4294967295u32);
#[derive(Clone, Copy)] enum E79 { Live(Wrap<SL<Tr<P79>>>), U0 }
suite!(79, s79, E79, [E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P79>(1u32) })))), E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P79>(2147483648u32) })))), E79::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P79>(4294967295u32) })))), E79::U0], |e| match e { E79::Live(w) => { let p: P79 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E79::U0 => 1001 });
type P80 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E80 { Live(P80), U0, U1 }
suite!(80, s80, E80, [E80::Live(unsafe { transmute::<u32, P80>(5u32) }), E80::Live(unsafe { transmute::<u32, P80>(2147483649u32) }), E80::Live(unsafe { transmute::<u32, P80>(4294967294u32) }), E80::U0, E80::U1], |e| match e { E80::Live(w) => { let p: P80 = w; let b: u32 = unsafe { transmute(p) }; b }, E80::U0 => 1001, E80::U1 => 1002 });
type P81 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E81 { Live(Wrap<P81>), U0, U1 }
suite!(81, s81, E81, [E81::Live(Wrap(unsafe { transmute::<u32, P81>(5u32) })), E81::Live(Wrap(unsafe { transmute::<u32, P81>(2147483649u32) })), E81::Live(Wrap(unsafe { transmute::<u32, P81>(4294967294u32) })), E81::U0, E81::U1], |e| match e { E81::Live(w) => { let p: P81 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E81::U0 => 1001, E81::U1 => 1002 });
type P82 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E82 { Live(Tr<P82>), U0, U1 }
suite!(82, s82, E82, [E82::Live(Tr(unsafe { transmute::<u32, P82>(5u32) })), E82::Live(Tr(unsafe { transmute::<u32, P82>(2147483649u32) })), E82::Live(Tr(unsafe { transmute::<u32, P82>(4294967294u32) })), E82::U0, E82::U1], |e| match e { E82::Live(w) => { let p: P82 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E82::U0 => 1001, E82::U1 => 1002 });
type P83 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E83 { Live(Named<P83>), U0, U1 }
suite!(83, s83, E83, [E83::Live(Named { z: (), v: unsafe { transmute::<u32, P83>(5u32) }, p: PhantomData }), E83::Live(Named { z: (), v: unsafe { transmute::<u32, P83>(2147483649u32) }, p: PhantomData }), E83::Live(Named { z: (), v: unsafe { transmute::<u32, P83>(4294967294u32) }, p: PhantomData }), E83::U0, E83::U1], |e| match e { E83::Live(w) => { let p: P83 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E83::U0 => 1001, E83::U1 => 1002 });
type P84 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E84 { Live(((), P84)), U0, U1 }
suite!(84, s84, E84, [E84::Live(((), unsafe { transmute::<u32, P84>(5u32) })), E84::Live(((), unsafe { transmute::<u32, P84>(2147483649u32) })), E84::Live(((), unsafe { transmute::<u32, P84>(4294967294u32) })), E84::U0, E84::U1], |e| match e { E84::Live(w) => { let p: P84 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E84::U0 => 1001, E84::U1 => 1002 });
type P85 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E85 { Live(SL<P85>), U0, U1 }
suite!(85, s85, E85, [E85::Live(SL::Live(unsafe { transmute::<u32, P85>(5u32) })), E85::Live(SL::Live(unsafe { transmute::<u32, P85>(2147483649u32) })), E85::Live(SL::Live(unsafe { transmute::<u32, P85>(4294967294u32) })), E85::U0, E85::U1], |e| match e { E85::Live(w) => { let p: P85 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E85::U0 => 1001, E85::U1 => 1002 });
type P86 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E86 { Live(ManuallyDrop<P86>), U0, U1 }
suite!(86, s86, E86, [E86::Live(ManuallyDrop::new(unsafe { transmute::<u32, P86>(5u32) })), E86::Live(ManuallyDrop::new(unsafe { transmute::<u32, P86>(2147483649u32) })), E86::Live(ManuallyDrop::new(unsafe { transmute::<u32, P86>(4294967294u32) })), E86::U0, E86::U1], |e| match e { E86::Live(w) => { let p: P86 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E86::U0 => 1001, E86::U1 => 1002 });
type P87 = pattern_type!(u32 is 5u32..=4294967294u32);
#[derive(Clone, Copy)] enum E87 { Live(Wrap<SL<Tr<P87>>>), U0, U1 }
suite!(87, s87, E87, [E87::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P87>(5u32) })))), E87::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P87>(2147483649u32) })))), E87::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P87>(4294967294u32) })))), E87::U0, E87::U1], |e| match e { E87::Live(w) => { let p: P87 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E87::U0 => 1001, E87::U1 => 1002 });
type P88 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E88 { Live(P88), U0 }
suite!(88, s88, E88, [E88::Live(unsafe { transmute::<u32, P88>(0u32) }), E88::Live(unsafe { transmute::<u32, P88>(2147483647u32) }), E88::Live(unsafe { transmute::<u32, P88>(4294967294u32) }), E88::U0], |e| match e { E88::Live(w) => { let p: P88 = w; let b: u32 = unsafe { transmute(p) }; b }, E88::U0 => 1001 });
type P89 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E89 { Live(Wrap<P89>), U0 }
suite!(89, s89, E89, [E89::Live(Wrap(unsafe { transmute::<u32, P89>(0u32) })), E89::Live(Wrap(unsafe { transmute::<u32, P89>(2147483647u32) })), E89::Live(Wrap(unsafe { transmute::<u32, P89>(4294967294u32) })), E89::U0], |e| match e { E89::Live(w) => { let p: P89 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E89::U0 => 1001 });
type P90 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E90 { Live(Tr<P90>), U0 }
suite!(90, s90, E90, [E90::Live(Tr(unsafe { transmute::<u32, P90>(0u32) })), E90::Live(Tr(unsafe { transmute::<u32, P90>(2147483647u32) })), E90::Live(Tr(unsafe { transmute::<u32, P90>(4294967294u32) })), E90::U0], |e| match e { E90::Live(w) => { let p: P90 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E90::U0 => 1001 });
type P91 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E91 { Live(Named<P91>), U0 }
suite!(91, s91, E91, [E91::Live(Named { z: (), v: unsafe { transmute::<u32, P91>(0u32) }, p: PhantomData }), E91::Live(Named { z: (), v: unsafe { transmute::<u32, P91>(2147483647u32) }, p: PhantomData }), E91::Live(Named { z: (), v: unsafe { transmute::<u32, P91>(4294967294u32) }, p: PhantomData }), E91::U0], |e| match e { E91::Live(w) => { let p: P91 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E91::U0 => 1001 });
type P92 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E92 { Live(((), P92)), U0 }
suite!(92, s92, E92, [E92::Live(((), unsafe { transmute::<u32, P92>(0u32) })), E92::Live(((), unsafe { transmute::<u32, P92>(2147483647u32) })), E92::Live(((), unsafe { transmute::<u32, P92>(4294967294u32) })), E92::U0], |e| match e { E92::Live(w) => { let p: P92 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E92::U0 => 1001 });
type P93 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E93 { Live(SL<P93>), U0 }
suite!(93, s93, E93, [E93::Live(SL::Live(unsafe { transmute::<u32, P93>(0u32) })), E93::Live(SL::Live(unsafe { transmute::<u32, P93>(2147483647u32) })), E93::Live(SL::Live(unsafe { transmute::<u32, P93>(4294967294u32) })), E93::U0], |e| match e { E93::Live(w) => { let p: P93 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E93::U0 => 1001 });
type P94 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E94 { Live(ManuallyDrop<P94>), U0 }
suite!(94, s94, E94, [E94::Live(ManuallyDrop::new(unsafe { transmute::<u32, P94>(0u32) })), E94::Live(ManuallyDrop::new(unsafe { transmute::<u32, P94>(2147483647u32) })), E94::Live(ManuallyDrop::new(unsafe { transmute::<u32, P94>(4294967294u32) })), E94::U0], |e| match e { E94::Live(w) => { let p: P94 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E94::U0 => 1001 });
type P95 = pattern_type!(u32 is 0u32..=4294967294u32);
#[derive(Clone, Copy)] enum E95 { Live(Wrap<SL<Tr<P95>>>), U0 }
suite!(95, s95, E95, [E95::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P95>(0u32) })))), E95::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P95>(2147483647u32) })))), E95::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P95>(4294967294u32) })))), E95::U0], |e| match e { E95::Live(w) => { let p: P95 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E95::U0 => 1001 });
type P96 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E96 { Live(P96), U0, U1, U2 }
suite!(96, s96, E96, [E96::Live(unsafe { transmute::<u32, P96>(2u32) }), E96::Live(unsafe { transmute::<u32, P96>(5u32) }), E96::Live(unsafe { transmute::<u32, P96>(9u32) }), E96::U0, E96::U1, E96::U2], |e| match e { E96::Live(w) => { let p: P96 = w; let b: u32 = unsafe { transmute(p) }; b }, E96::U0 => 1001, E96::U1 => 1002, E96::U2 => 1003 });
type P97 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E97 { Live(Wrap<P97>), U0, U1, U2 }
suite!(97, s97, E97, [E97::Live(Wrap(unsafe { transmute::<u32, P97>(2u32) })), E97::Live(Wrap(unsafe { transmute::<u32, P97>(5u32) })), E97::Live(Wrap(unsafe { transmute::<u32, P97>(9u32) })), E97::U0, E97::U1, E97::U2], |e| match e { E97::Live(w) => { let p: P97 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E97::U0 => 1001, E97::U1 => 1002, E97::U2 => 1003 });
type P98 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E98 { Live(Tr<P98>), U0, U1, U2 }
suite!(98, s98, E98, [E98::Live(Tr(unsafe { transmute::<u32, P98>(2u32) })), E98::Live(Tr(unsafe { transmute::<u32, P98>(5u32) })), E98::Live(Tr(unsafe { transmute::<u32, P98>(9u32) })), E98::U0, E98::U1, E98::U2], |e| match e { E98::Live(w) => { let p: P98 = w.0; let b: u32 = unsafe { transmute(p) }; b }, E98::U0 => 1001, E98::U1 => 1002, E98::U2 => 1003 });
type P99 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E99 { Live(Named<P99>), U0, U1, U2 }
suite!(99, s99, E99, [E99::Live(Named { z: (), v: unsafe { transmute::<u32, P99>(2u32) }, p: PhantomData }), E99::Live(Named { z: (), v: unsafe { transmute::<u32, P99>(5u32) }, p: PhantomData }), E99::Live(Named { z: (), v: unsafe { transmute::<u32, P99>(9u32) }, p: PhantomData }), E99::U0, E99::U1, E99::U2], |e| match e { E99::Live(w) => { let p: P99 = w.v; let b: u32 = unsafe { transmute(p) }; b }, E99::U0 => 1001, E99::U1 => 1002, E99::U2 => 1003 });
type P100 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E100 { Live(((), P100)), U0, U1, U2 }
suite!(100, s100, E100, [E100::Live(((), unsafe { transmute::<u32, P100>(2u32) })), E100::Live(((), unsafe { transmute::<u32, P100>(5u32) })), E100::Live(((), unsafe { transmute::<u32, P100>(9u32) })), E100::U0, E100::U1, E100::U2], |e| match e { E100::Live(w) => { let p: P100 = w.1; let b: u32 = unsafe { transmute(p) }; b }, E100::U0 => 1001, E100::U1 => 1002, E100::U2 => 1003 });
type P101 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E101 { Live(SL<P101>), U0, U1, U2 }
suite!(101, s101, E101, [E101::Live(SL::Live(unsafe { transmute::<u32, P101>(2u32) })), E101::Live(SL::Live(unsafe { transmute::<u32, P101>(5u32) })), E101::Live(SL::Live(unsafe { transmute::<u32, P101>(9u32) })), E101::U0, E101::U1, E101::U2], |e| match e { E101::Live(w) => { let p: P101 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E101::U0 => 1001, E101::U1 => 1002, E101::U2 => 1003 });
type P102 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E102 { Live(ManuallyDrop<P102>), U0, U1, U2 }
suite!(102, s102, E102, [E102::Live(ManuallyDrop::new(unsafe { transmute::<u32, P102>(2u32) })), E102::Live(ManuallyDrop::new(unsafe { transmute::<u32, P102>(5u32) })), E102::Live(ManuallyDrop::new(unsafe { transmute::<u32, P102>(9u32) })), E102::U0, E102::U1, E102::U2], |e| match e { E102::Live(w) => { let p: P102 = ManuallyDrop::into_inner(w); let b: u32 = unsafe { transmute(p) }; b }, E102::U0 => 1001, E102::U1 => 1002, E102::U2 => 1003 });
type P103 = pattern_type!(u32 is 2u32..=9u32);
#[derive(Clone, Copy)] enum E103 { Live(Wrap<SL<Tr<P103>>>), U0, U1, U2 }
suite!(103, s103, E103, [E103::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P103>(2u32) })))), E103::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P103>(5u32) })))), E103::Live(Wrap(SL::Live(Tr(unsafe { transmute::<u32, P103>(9u32) })))), E103::U0, E103::U1, E103::U2], |e| match e { E103::Live(w) => { let p: P103 = (match w.0 { SL::Live(x) => x.0, SL::Never(..) => loop {} }); let b: u32 = unsafe { transmute(p) }; b }, E103::U0 => 1001, E103::U1 => 1002, E103::U2 => 1003 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s0(); s1(); s2(); s3(); s4(); s5(); s6(); s7(); s8(); s9(); s10(); s11(); s12(); s13(); s14(); s15(); s16(); s17(); s18(); s19(); s20(); s21(); s22(); s23(); s24(); s25(); s26(); s27(); s28(); s29(); s30(); s31(); s32(); s33(); s34(); s35(); s36(); s37(); s38(); s39(); s40(); s41(); s42(); s43(); s44(); s45(); s46(); s47(); s48(); s49(); s50(); s51(); s52(); s53(); s54(); s55(); s56(); s57(); s58(); s59(); s60(); s61(); s62(); s63(); s64(); s65(); s66(); s67(); s68(); s69(); s70(); s71(); s72(); s73(); s74(); s75(); s76(); s77(); s78(); s79(); s80(); s81(); s82(); s83(); s84(); s85(); s86(); s87(); s88(); s89(); s90(); s91(); s92(); s93(); s94(); s95(); s96(); s97(); s98(); s99(); s100(); s101(); s102(); s103();
}
