#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cmp::Ordering;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroI16, NonZeroU64, NonZeroI8};

trait HasOut { type Out; }
impl<T> HasOut for fn() -> T { type Out = T; }
type Never = <fn() -> ! as HasOut>::Out;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] #[repr(C)] struct CW<T>(T);
#[derive(Clone, Copy)] #[repr(packed)] struct Pk<T>(T);
#[derive(Clone, Copy)] struct Named<T> { z: (), v: T, p: PhantomData<u64> }
#[derive(Clone, Copy)] struct ZPad<T>([u8; 0], T, ());
#[derive(Clone, Copy)] #[repr(align(4))] struct Aligned(Void);
#[derive(Clone, Copy)] enum AllDead { A(Void), B(Never) }
#[derive(Clone, Copy)] enum DeepDead { A(Wrap<Void>), B((Void,)), C([Void; 1]) }
#[derive(Clone, Copy)] enum HalfDead { A(Void), B }
#[derive(Clone, Copy)] enum HalfDead2 { A, B(Never) }
#[derive(Clone, Copy)] struct NamedDead { a: (), b: Void }
#[derive(Clone, Copy)] #[repr(transparent)] struct TrDead(Void);
#[derive(Clone, Copy)] #[repr(C)] struct CDead(Void);
#[derive(Clone, Copy)] #[repr(packed)] struct PkDead(Void);
#[derive(Clone, Copy)] union UDead { v: Void, u: () }
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] enum SL3<T> { D0(Void), D1(Never), Live(T), D3((Void, ())) }

#[derive(Clone, Copy)] #[repr(u8)] enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi { A = 200, B = 253 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi3 { A = 250, B = 251, C = 252 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg { A = -3, B = -1 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg2 { A = -56, B = -3 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg3 { A = 100, B = 125 }
#[derive(Clone, Copy)] #[repr(u16)] enum W16 { A = 0xFFF0, B = 0xFFFD }
#[derive(Clone, Copy)] #[repr(i16)] enum S16 { A = -200, B = -3 }
#[derive(Clone, Copy)] #[repr(u32)] enum W32 { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i32)] enum S32 { A = -70000, B = -3 }
#[derive(Clone, Copy)] #[repr(u64)] enum W64 { A = 0xFFFF_FFFF_FFFF_FF00, B = 0xFFFF_FFFF_FFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i64)] enum S64 { A = -5_000_000_000, B = -3 }
#[derive(Clone, Copy)] #[repr(usize)] enum WU { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] enum Three { A, B, C }
#[derive(Clone, Copy)] enum Two { A, B }

static SEVEN: u32 = 77;
fn seven() -> u32 { 7 }
fn eight() -> u32 { 8 }

macro_rules! suite {
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            #[inline(never)]
            fn cls_ref(r: &$T) -> u32 { let $e = *r; $body }
            #[inline(never)]
            fn cls_o(o: Option<$T>) -> u32 { match o { Some($e) => $body, None => 55555 } }
            #[inline(never)]
            fn cls_oo(o: Option<Option<$T>>) -> u32 { match o { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 } }
            static TABLE: &[$T] = &[$($k),*];
            static OTABLE: [Option<$T>; 2] = [None, None];
            $(
                {
                    const K: $T = $k;
                    static S: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                    print_u32(cls_ref(&K));
                    print_u32(cls_ref(black_box(&S)));
                    print_u32(cls(S));
                    const O: Option<$T> = Some($k);
                    static SO: Option<$T> = Some($k);
                    print_u32(match O { Some($e) => $body, None => 55555 });
                    print_u32(cls_o(O));
                    print_u32(cls_o(black_box(O)));
                    print_u32(cls_o(SO));
                    const OO: Option<Option<$T>> = Some(Some($k));
                    print_u32(cls_oo(OO));
                    print_u32(cls_oo(black_box(OO)));
                    print_bool(matches!(black_box(OO), Some(Some(_))));
                    print_bool(core::mem::discriminant(&K) == core::mem::discriminant(black_box(&S)));
                }
            )*
            { const N: Option<$T> = None; print_u32(cls_o(N)); print_u32(cls_o(black_box(N))); print_bool(N.is_none()); print_u32(match N { Some($e) => $body, None => 55555 }); }
            { const N: Option<Option<$T>> = Some(None); print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            { const N: Option<Option<$T>> = None; print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            print_u32(cls_o(OTABLE[black_box(1)]));
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                let mut j = 0;
                while j < TABLE.len() {
                    print_bool(core::mem::discriminant(&TABLE[black_box(i)]) == core::mem::discriminant(&TABLE[j]));
                    j += 1;
                }
                i += 1;
            }
        }
    };
}
#[derive(Clone, Copy)] enum E0 { Live(Option<Option<Three>>) }
suite!(0, s0, E0, [E0::Live(Some(Some(Three::C))), E0::Live(Some(None)), E0::Live(None)], |e| match e { E0::Live(w) => { let p: Option<Option<Three>> = w; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } } });
#[derive(Clone, Copy)] enum E1 { D0(Result<(), Void>), Live(CW<Hi3>), U1, U0 }
suite!(1, s1, E1, [E1::D0(Ok(())), E1::Live(CW(Hi3::A)), E1::Live(CW(Hi3::B)), E1::Live(CW(Hi3::C)), E1::U1, E1::U0], |e| match e { E1::D0(..) => 2001, E1::Live(w) => { let p: Hi3 = w.0; p as u32 }, E1::U1 => 1002, E1::U0 => 1001 });
#[derive(Clone, Copy)] enum E2 { Live(CW<WU>), U0, D2(PhantomData<Void>), U1, D0(CDead), D1(Infallible), U2 }
suite!(2, s2, E2, [E2::Live(CW(WU::A)), E2::Live(CW(WU::B)), E2::U0, E2::D2(PhantomData), E2::U1, E2::U2], |e| match e { E2::Live(w) => { let p: WU = w.0; p as usize as u32 }, E2::U0 => 1001, E2::D2(..) => 2003, E2::U1 => 1002, E2::D0(..) => 9001, E2::D1(..) => 9002, E2::U2 => 1003 });
#[derive(Clone, Copy)] enum E3 { D1(HalfDead2), Live(Option<bool>), U0, D0(u8, Void), U1 }
suite!(3, s3, E3, [E3::D1(HalfDead2::A), E3::Live(Some(true)), E3::Live(Some(false)), E3::Live(None), E3::U0, E3::U1], |e| match e { E3::D1(..) => 2002, E3::Live(w) => { let p: Option<bool> = w; match p { Some(b) => 40 + b as u32, None => 49 } }, E3::U0 => 1001, E3::D0(..) => 9001, E3::U1 => 1002 });
#[derive(Clone, Copy)] enum E4 { Live(Named<NonZeroU64>), D0(u8, Void) }
suite!(4, s4, E4, [E4::Live(Named { z: (), v: NonZeroU64::new(1).unwrap(), p: PhantomData }), E4::Live(Named { z: (), v: NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap(), p: PhantomData }), E4::Live(Named { z: (), v: NonZeroU64::new(0x1_0000_0000).unwrap(), p: PhantomData })], |e| match e { E4::Live(w) => { let p: NonZeroU64 = w.v; (p.get() % 1_000_003) as u32 }, E4::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E5 { Live(Result<Never, NonZeroU16>), U0, U4, D0((Never, Never)), U3, U5, U2, U1 }
suite!(5, s5, E5, [E5::Live(Err(NonZeroU16::new(1).unwrap())), E5::Live(Err(NonZeroU16::new(65535).unwrap())), E5::U0, E5::U4, E5::U3, E5::U5, E5::U2, E5::U1], |e| match e { E5::Live(w) => { let p: NonZeroU16 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() as u32 }, E5::U0 => 1001, E5::U4 => 1005, E5::D0(..) => 9001, E5::U3 => 1004, E5::U5 => 1006, E5::U2 => 1003, E5::U1 => 1002 });
#[derive(Clone, Copy)] enum E6 { U0, Live(SL3<Option<bool>>), D0([Void; 0]), D1(CDead) }
suite!(6, s6, E6, [E6::U0, E6::Live(SL3::Live(Some(true))), E6::Live(SL3::Live(Some(false))), E6::Live(SL3::Live(None)), E6::D0([])], |e| match e { E6::U0 => 1001, E6::Live(w) => { let p: Option<bool> = (match w { SL3::Live(x) => x, _ => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } }, E6::D0(..) => 2001, E6::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E7 { D1(Wrap<Wrap<(Void, ())>>), Live(Result<Option<Hi2>, Void>), U0, U5, U3, U1, U2, D0(Void, [u64; 0]), U4 }
suite!(7, s7, E7, [E7::Live(Ok(Some(Hi2::A))), E7::Live(Ok(Some(Hi2::B))), E7::Live(Ok(None)), E7::U0, E7::U5, E7::U3, E7::U1, E7::U2, E7::U4], |e| match e { E7::D1(..) => 9002, E7::Live(w) => { let p: Option<Hi2> = (match w { Ok(x) => x, Err(_) => loop {} }); match p { Some(h) => h as u32, None => 47 } }, E7::U0 => 1001, E7::U5 => 1006, E7::U3 => 1004, E7::U1 => 1002, E7::U2 => 1003, E7::D0(..) => 9001, E7::U4 => 1005 });
#[derive(Clone, Copy)] enum E8 { Live(SL<Wrap<SL3<Option<char>>>>), U0, D0(ManuallyDrop<Void>), D1(AllDead), U1 }
suite!(8, s8, E8, [E8::Live(SL::Live(Wrap(SL3::Live(Some('z'))))), E8::Live(SL::Live(Wrap(SL3::Live(None)))), E8::U0, E8::U1], |e| match e { E8::Live(w) => { let p: Option<char> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(c) => c as u32, None => 48 } }, E8::U0 => 1001, E8::D0(..) => 9001, E8::D1(..) => 9002, E8::U1 => 1002 });
#[derive(Clone, Copy)] enum E9 { U0, D2(Aligned), D0((Void, ())), U1, Live(Result<Never, W16>), D1((Void,)) }
suite!(9, s9, E9, [E9::U0, E9::U1, E9::Live(Err(W16::A)), E9::Live(Err(W16::B))], |e| match e { E9::U0 => 1001, E9::D2(..) => 9003, E9::D0(..) => 9001, E9::U1 => 1002, E9::Live(w) => { let p: W16 = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E9::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E10 { D1(CDead), D0(NamedDead), U1, Live(S32), U0 }
suite!(10, s10, E10, [E10::U1, E10::Live(S32::A), E10::Live(S32::B), E10::U0], |e| match e { E10::D1(..) => 9002, E10::D0(..) => 9001, E10::U1 => 1002, E10::Live(w) => { let p: S32 = w; p as i32 as u32 }, E10::U0 => 1001 });
#[derive(Clone, Copy)] enum E11 { D0((Void,)), Live(Pk<fn() -> u32>), D1((Never, Never)) }
suite!(11, s11, E11, [E11::Live(Pk(seven)), E11::Live(Pk(eight))], |e| match e { E11::D0(..) => 9001, E11::Live(w) => { let p: fn() -> u32 = { w.0 }; p() }, E11::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E12 { U0, D0(Never), D1(Infallible), D2(Never), Live(CW<Hi3>) }
suite!(12, s12, E12, [E12::U0, E12::Live(CW(Hi3::A)), E12::Live(CW(Hi3::B)), E12::Live(CW(Hi3::C))], |e| match e { E12::U0 => 1001, E12::D0(..) => 9001, E12::D1(..) => 9002, E12::D2(..) => 9003, E12::Live(w) => { let p: Hi3 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E13 { U0, Live(ZPad<Hi3>), D0(()) }
suite!(13, s13, E13, [E13::U0, E13::Live(ZPad([], Hi3::A, ())), E13::Live(ZPad([], Hi3::B, ())), E13::Live(ZPad([], Hi3::C, ())), E13::D0(())], |e| match e { E13::U0 => 1001, E13::Live(w) => { let p: Hi3 = w.1; p as u32 }, E13::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E14 { U2, U1, D1(([Void; 0], Void)), D0([(Void,); 0]), U0, Live(Three) }
suite!(14, s14, E14, [E14::U2, E14::U1, E14::D0([]), E14::U0, E14::Live(Three::A), E14::Live(Three::B), E14::Live(Three::C)], |e| match e { E14::U2 => 1003, E14::U1 => 1002, E14::D1(..) => 9002, E14::D0(..) => 2001, E14::U0 => 1001, E14::Live(w) => { let p: Three = w; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E15 { U1, U0, Live(Wrap<S64>), D0([u16; 0], Void) }
suite!(15, s15, E15, [E15::U1, E15::U0, E15::Live(Wrap(S64::A)), E15::Live(Wrap(S64::B))], |e| match e { E15::U1 => 1002, E15::U0 => 1001, E15::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 }, E15::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E16 { Live(SL<Wrap<SL3<W32>>>), D0([(Void,); 0]) }
suite!(16, s16, E16, [E16::Live(SL::Live(Wrap(SL3::Live(W32::A)))), E16::Live(SL::Live(Wrap(SL3::Live(W32::B)))), E16::D0([])], |e| match e { E16::Live(w) => { let p: W32 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E16::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E17 { U3, U0, U2, D0(Void, (), PhantomData<u8>), U1, Live(Tr<Three>) }
suite!(17, s17, E17, [E17::U3, E17::U0, E17::U2, E17::U1, E17::Live(Tr(Three::A)), E17::Live(Tr(Three::B)), E17::Live(Tr(Three::C))], |e| match e { E17::U3 => 1004, E17::U0 => 1001, E17::U2 => 1003, E17::D0(..) => 9001, E17::U1 => 1002, E17::Live(w) => { let p: Three = w.0; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E18 { Live(Wrap<Tr<((), S16)>>), D1(TrDead), U2, U0, U1, D0(MaybeUninit<Void>) }
suite!(18, s18, E18, [E18::Live(Wrap(Tr(((), S16::A)))), E18::Live(Wrap(Tr(((), S16::B)))), E18::U2, E18::U0, E18::U1], |e| match e { E18::Live(w) => { let p: S16 = w.0.0.1; p as i32 as u32 }, E18::D1(..) => 9002, E18::U2 => 1003, E18::U0 => 1001, E18::U1 => 1002, E18::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E19 { U2, U5, U4, U3, U0, D0((u32, Void)), U1, Live(Sg2) }
suite!(19, s19, E19, [E19::U2, E19::U5, E19::U4, E19::U3, E19::U0, E19::U1, E19::Live(Sg2::A), E19::Live(Sg2::B)], |e| match e { E19::U2 => 1003, E19::U5 => 1006, E19::U4 => 1005, E19::U3 => 1004, E19::U0 => 1001, E19::D0(..) => 9001, E19::U1 => 1002, E19::Live(w) => { let p: Sg2 = w; p as i32 as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s0(); s1(); s2(); s3(); s5(); s6(); s7(); s8(); s9(); s10(); s11(); s12(); s13(); s14(); s15(); s16(); s17(); s18(); s19();
}
