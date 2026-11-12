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
#[derive(Clone, Copy)] enum E0 { D0([Void; 0]), Live(Wrap<Tr<((), NonZeroU16)>>) }
suite!(0, s0, E0, [E0::D0([]), E0::Live(Wrap(Tr(((), NonZeroU16::new(1).unwrap())))), E0::Live(Wrap(Tr(((), NonZeroU16::new(65535).unwrap()))))], |e| match e { E0::D0(..) => 2001, E0::Live(w) => { let p: NonZeroU16 = w.0.0.1; p.get() as u32 } });
#[derive(Clone, Copy)] enum E1 { U1, D0((HalfDead, Void)), U3, U2, Live(SL3<Ordering>), U0, D2(MaybeUninit<Void>), D1(Void) }
suite!(1, s1, E1, [E1::U1, E1::U3, E1::U2, E1::Live(SL3::Live(Ordering::Less)), E1::Live(SL3::Live(Ordering::Equal)), E1::Live(SL3::Live(Ordering::Greater)), E1::U0, E1::D2(MaybeUninit::uninit())], |e| match e { E1::U1 => 1002, E1::D0(..) => 9001, E1::U3 => 1004, E1::U2 => 1003, E1::Live(w) => { let p: Ordering = (match w { SL3::Live(x) => x, _ => loop {} }); p as i32 as u32 }, E1::U0 => 1001, E1::D2(..) => 2003, E1::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E2 { U1, D0(Never), D2(Never), Live(SL3<Three>), D1([u32; 0]), U0 }
suite!(2, s2, E2, [E2::U1, E2::Live(SL3::Live(Three::A)), E2::Live(SL3::Live(Three::B)), E2::Live(SL3::Live(Three::C)), E2::D1([]), E2::U0], |e| match e { E2::U1 => 1002, E2::D0(..) => 9001, E2::D2(..) => 9003, E2::Live(w) => { let p: Three = (match w { SL3::Live(x) => x, _ => loop {} }); p as u32 + 20 }, E2::D1(..) => 2002, E2::U0 => 1001 });
#[derive(Clone, Copy)] enum E3 { D0(HalfDead2), U1, D1(DeepDead), D2(([Void; 0], Void)), Live(W32), U2, U0 }
suite!(3, s3, E3, [E3::D0(HalfDead2::A), E3::U1, E3::Live(W32::A), E3::Live(W32::B), E3::U2, E3::U0], |e| match e { E3::D0(..) => 2001, E3::U1 => 1002, E3::D1(..) => 9002, E3::D2(..) => 9003, E3::Live(w) => { let p: W32 = w; p as u32 }, E3::U2 => 1003, E3::U0 => 1001 });
#[derive(Clone, Copy)] enum E4 { U1, U0, U3, D0((), Infallible), U2, Live(Named<NonZeroU64>) }
suite!(4, s4, E4, [E4::U1, E4::U0, E4::U3, E4::U2, E4::Live(Named { z: (), v: NonZeroU64::new(1).unwrap(), p: PhantomData }), E4::Live(Named { z: (), v: NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap(), p: PhantomData }), E4::Live(Named { z: (), v: NonZeroU64::new(0x1_0000_0000).unwrap(), p: PhantomData })], |e| match e { E4::U1 => 1002, E4::U0 => 1001, E4::U3 => 1004, E4::D0(..) => 9001, E4::U2 => 1003, E4::Live(w) => { let p: NonZeroU64 = w.v; (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E5 { Live((Two,)), U0, U4, D0(Result<(), Void>), U3, U5, U1, U2, D1(Infallible) }
suite!(5, s5, E5, [E5::Live((Two::A,)), E5::Live((Two::B,)), E5::U0, E5::U4, E5::D0(Ok(())), E5::U3, E5::U5, E5::U1, E5::U2], |e| match e { E5::Live(w) => { let p: Two = w.0; p as u32 + 30 }, E5::U0 => 1001, E5::U4 => 1005, E5::D0(..) => 2001, E5::U3 => 1004, E5::U5 => 1006, E5::U1 => 1002, E5::U2 => 1003, E5::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E6 { Live(Named<W16>), D0(Wrap<Void>), U2, U1, U0, D1([u16; 0], Void) }
suite!(6, s6, E6, [E6::Live(Named { z: (), v: W16::A, p: PhantomData }), E6::Live(Named { z: (), v: W16::B, p: PhantomData }), E6::U2, E6::U1, E6::U0], |e| match e { E6::Live(w) => { let p: W16 = w.v; p as u32 }, E6::D0(..) => 9001, E6::U2 => 1003, E6::U1 => 1002, E6::U0 => 1001, E6::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E7 { Live(Result<Never, WU>), D0((Never, Never)) }
suite!(7, s7, E7, [E7::Live(Err(WU::A)), E7::Live(Err(WU::B))], |e| match e { E7::Live(w) => { let p: WU = (match w { Err(x) => x, Ok(_) => loop {} }); p as usize as u32 }, E7::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E8 { U0, Live(W16), U3, U2, U1 }
suite!(8, s8, E8, [E8::U0, E8::Live(W16::A), E8::Live(W16::B), E8::U3, E8::U2, E8::U1], |e| match e { E8::U0 => 1001, E8::Live(w) => { let p: W16 = w; p as u32 }, E8::U3 => 1004, E8::U2 => 1003, E8::U1 => 1002 });
#[derive(Clone, Copy)] enum E9 { U0, Live(Result<Never, NonZeroI16>) }
suite!(9, s9, E9, [E9::U0, E9::Live(Err(NonZeroI16::new(-1).unwrap())), E9::Live(Err(NonZeroI16::new(300).unwrap()))], |e| match e { E9::U0 => 1001, E9::Live(w) => { let p: NonZeroI16 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E10 { Live(ManuallyDrop<W32>), U0, D1((u32, Void)), D0(CDead) }
suite!(10, s10, E10, [E10::Live(ManuallyDrop::new(W32::A)), E10::Live(ManuallyDrop::new(W32::B)), E10::U0], |e| match e { E10::Live(w) => { let p: W32 = ManuallyDrop::into_inner(w); p as u32 }, E10::U0 => 1001, E10::D1(..) => 9002, E10::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E11 { D0(()), Live(ManuallyDrop<WU>), D1(HalfDead2) }
suite!(11, s11, E11, [E11::D0(()), E11::Live(ManuallyDrop::new(WU::A)), E11::Live(ManuallyDrop::new(WU::B)), E11::D1(HalfDead2::A)], |e| match e { E11::D0(..) => 2001, E11::Live(w) => { let p: WU = ManuallyDrop::into_inner(w); p as usize as u32 }, E11::D1(..) => 2002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s0(); s1(); s2(); s3(); s4(); s5(); s6(); s7(); s8(); s9(); s10(); s11();
}
