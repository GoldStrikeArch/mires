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
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], [$($r:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            // the same values built at run time
            $(
                {
                    #[inline(never)]
                    fn rt_cls($e: $T) -> u32 { $body }
                    #[inline(never)]
                    fn mk() -> $T { $r }
                    { let $e = $r; print_u32($body); }
                    print_u32(rt_cls($r));
                    print_u32(rt_cls(mk()));
                    let o = Some(mk());
                    print_u32(match black_box(o) { Some($e) => $body, None => 55555 });
                    let oo = black_box(Some(Some($r)));
                    print_u32(match oo { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 });
                    print_bool(core::mem::discriminant(&mk()) == core::mem::discriminant(&$r));
                }
            )*
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
#[derive(Clone, Copy)] enum E0 { U0, D0(&'static Void, Void), U1, Live(Wrap<Tr<((), NonZeroU64)>>), U5, U2, U3, U4 }
suite!(0, s0, E0, [E0::U0, E0::U1, E0::Live(Wrap(Tr(((), NonZeroU64::new(1).unwrap())))), E0::Live(Wrap(Tr(((), NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())))), E0::Live(Wrap(Tr(((), NonZeroU64::new(0x1_0000_0000).unwrap())))), E0::U5, E0::U2, E0::U3, E0::U4], [E0::Live(Wrap(Tr(((), black_box(NonZeroU64::new(1).unwrap()))))), E0::Live(Wrap(Tr(((), black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))))), E0::Live(Wrap(Tr(((), black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))))], |e| match e { E0::U0 => 1001, E0::D0(..) => 9001, E0::U1 => 1002, E0::Live(w) => { let p: NonZeroU64 = w.0.0.1; (p.get() % 1_000_003) as u32 }, E0::U5 => 1006, E0::U2 => 1003, E0::U3 => 1004, E0::U4 => 1005 });
#[derive(Clone, Copy)] enum E1 { Live(ManuallyDrop<Small>), U0, D0(HalfDead) }
suite!(1, s1, E1, [E1::Live(ManuallyDrop::new(Small::X)), E1::Live(ManuallyDrop::new(Small::Y)), E1::U0, E1::D0(HalfDead::B)], [E1::Live(ManuallyDrop::new(black_box(Small::X))), E1::Live(ManuallyDrop::new(black_box(Small::Y)))], |e| match e { E1::Live(w) => { let p: Small = ManuallyDrop::into_inner(w); p as u32 }, E1::U0 => 1001, E1::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E2 { Live(ManuallyDrop<fn() -> u32>), U2, U1, U3, D1(Void), U0, D0(Wrap<HalfDead>, ()) }
suite!(2, s2, E2, [E2::Live(ManuallyDrop::new(seven)), E2::Live(ManuallyDrop::new(eight)), E2::U2, E2::U1, E2::U3, E2::U0, E2::D0(Wrap(HalfDead::B), ())], [E2::Live(ManuallyDrop::new(black_box(seven))), E2::Live(ManuallyDrop::new(black_box(eight)))], |e| match e { E2::Live(w) => { let p: fn() -> u32 = ManuallyDrop::into_inner(w); p() }, E2::U2 => 1003, E2::U1 => 1002, E2::U3 => 1004, E2::D1(..) => 9002, E2::U0 => 1001, E2::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E3 { D0(CDead), D1(&'static Void, Void), U0, Live(Ordering) }
suite!(3, s3, E3, [E3::U0, E3::Live(Ordering::Less), E3::Live(Ordering::Equal), E3::Live(Ordering::Greater)], [E3::Live(black_box(Ordering::Less)), E3::Live(black_box(Ordering::Equal)), E3::Live(black_box(Ordering::Greater))], |e| match e { E3::D0(..) => 9001, E3::D1(..) => 9002, E3::U0 => 1001, E3::Live(w) => { let p: Ordering = w; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E4 { U1, U0, Live([W64; 1]), U2, D0((Void, ())), D1(Infallible) }
suite!(4, s4, E4, [E4::U1, E4::U0, E4::Live([W64::A]), E4::Live([W64::B]), E4::U2], [E4::Live([black_box(W64::A)]), E4::Live([black_box(W64::B)])], |e| match e { E4::U1 => 1002, E4::U0 => 1001, E4::Live(w) => { let p: W64 = w[0]; (p as u64 % 1_000_003) as u32 }, E4::U2 => 1003, E4::D0(..) => 9001, E4::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E5 { Live(Result<NonZeroU16, Void>), U0, D0(UDead) }
suite!(5, s5, E5, [E5::Live(Ok(NonZeroU16::new(1).unwrap())), E5::Live(Ok(NonZeroU16::new(65535).unwrap())), E5::U0], [E5::Live(Ok(black_box(NonZeroU16::new(1).unwrap()))), E5::Live(Ok(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E5::Live(w) => { let p: NonZeroU16 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as u32 }, E5::U0 => 1001, E5::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E6 { Live(((), Option<bool>)), U0, D1(Result<(), Void>), U1, D0(DeepDead), U2 }
suite!(6, s6, E6, [E6::Live(((), Some(true))), E6::Live(((), Some(false))), E6::Live(((), None)), E6::U0, E6::D1(Ok(())), E6::U1, E6::U2], [E6::Live(((), black_box(Some(true)))), E6::Live(((), black_box(Some(false)))), E6::Live(((), black_box(None)))], |e| match e { E6::Live(w) => { let p: Option<bool> = w.1; match p { Some(b) => 40 + b as u32, None => 49 } }, E6::U0 => 1001, E6::D1(..) => 2002, E6::U1 => 1002, E6::D0(..) => 9001, E6::U2 => 1003 });
#[derive(Clone, Copy)] enum E7 { D0(Wrap<Wrap<(Void, ())>>), Live((Option<char>,)) }
suite!(7, s7, E7, [E7::Live((Some('z'),)), E7::Live((None,))], [E7::Live((black_box(Some('z')),)), E7::Live((black_box(None),))], |e| match e { E7::D0(..) => 9001, E7::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E8 { Live(ManuallyDrop<Option<Option<Three>>>), D2(PkDead), D1((), Infallible), D0([Void; 2]) }
suite!(8, s8, E8, [E8::Live(ManuallyDrop::new(Some(Some(Three::C)))), E8::Live(ManuallyDrop::new(Some(None))), E8::Live(ManuallyDrop::new(None))], [E8::Live(ManuallyDrop::new(black_box(Some(Some(Three::C))))), E8::Live(ManuallyDrop::new(black_box(Some(None)))), E8::Live(ManuallyDrop::new(black_box(None)))], |e| match e { E8::Live(w) => { let p: Option<Option<Three>> = ManuallyDrop::into_inner(w); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E8::D2(..) => 9003, E8::D1(..) => 9002, E8::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E9 { D0(Wrap<AllDead>), Live(Result<W16, Void>) }
suite!(9, s9, E9, [E9::Live(Ok(W16::A)), E9::Live(Ok(W16::B))], [E9::Live(Ok(black_box(W16::A))), E9::Live(Ok(black_box(W16::B)))], |e| match e { E9::D0(..) => 9001, E9::Live(w) => { let p: W16 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E10 { Live(Tr<Option<char>>), D0(Wrap<AllDead>) }
suite!(10, s10, E10, [E10::Live(Tr(Some('z'))), E10::Live(Tr(None))], [E10::Live(Tr(black_box(Some('z')))), E10::Live(Tr(black_box(None)))], |e| match e { E10::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E10::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E11 { D0(Result<Void, Never>), Live(Wrap<Option<bool>>), D1(Result<(), Void>) }
suite!(11, s11, E11, [E11::Live(Wrap(Some(true))), E11::Live(Wrap(Some(false))), E11::Live(Wrap(None)), E11::D1(Ok(()))], [E11::Live(Wrap(black_box(Some(true)))), E11::Live(Wrap(black_box(Some(false)))), E11::Live(Wrap(black_box(None)))], |e| match e { E11::D0(..) => 9001, E11::Live(w) => { let p: Option<bool> = w.0; match p { Some(b) => 40 + b as u32, None => 49 } }, E11::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E12 { U1, D0([Void; 2]), D1((Void, ())), Live(ManuallyDrop<Hi2>), U0 }
suite!(12, s12, E12, [E12::U1, E12::Live(ManuallyDrop::new(Hi2::A)), E12::Live(ManuallyDrop::new(Hi2::B)), E12::U0], [E12::Live(ManuallyDrop::new(black_box(Hi2::A))), E12::Live(ManuallyDrop::new(black_box(Hi2::B)))], |e| match e { E12::U1 => 1002, E12::D0(..) => 9001, E12::D1(..) => 9002, E12::Live(w) => { let p: Hi2 = ManuallyDrop::into_inner(w); p as u32 }, E12::U0 => 1001 });
#[derive(Clone, Copy)] enum E13 { Live(ManuallyDrop<W64>), D0(PkDead) }
suite!(13, s13, E13, [E13::Live(ManuallyDrop::new(W64::A)), E13::Live(ManuallyDrop::new(W64::B))], [E13::Live(ManuallyDrop::new(black_box(W64::A))), E13::Live(ManuallyDrop::new(black_box(W64::B)))], |e| match e { E13::Live(w) => { let p: W64 = ManuallyDrop::into_inner(w); (p as u64 % 1_000_003) as u32 }, E13::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E14 { D2(Void, (), PhantomData<u8>), D1(Result<Void, Never>), U0, D0(Result<Void, Never>), Live(ZPad<W64>) }
suite!(14, s14, E14, [E14::U0, E14::Live(ZPad([], W64::A, ())), E14::Live(ZPad([], W64::B, ()))], [E14::Live(ZPad([], black_box(W64::A), ())), E14::Live(ZPad([], black_box(W64::B), ()))], |e| match e { E14::D2(..) => 9003, E14::D1(..) => 9002, E14::U0 => 1001, E14::D0(..) => 9001, E14::Live(w) => { let p: W64 = w.1; (p as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E15 { U2, U0, U1, Live(Result<Never, Sg3>), D1((HalfDead, Void)), D0(UDead), D2(NamedDead) }
suite!(15, s15, E15, [E15::U2, E15::U0, E15::U1, E15::Live(Err(Sg3::A)), E15::Live(Err(Sg3::B))], [E15::Live(Err(black_box(Sg3::A))), E15::Live(Err(black_box(Sg3::B)))], |e| match e { E15::U2 => 1003, E15::U0 => 1001, E15::U1 => 1002, E15::Live(w) => { let p: Sg3 = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 }, E15::D1(..) => 9002, E15::D0(..) => 9001, E15::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E16 { Live(((), Option<Hi2>)), U0, U2, U1, D0(Void) }
suite!(16, s16, E16, [E16::Live(((), Some(Hi2::A))), E16::Live(((), Some(Hi2::B))), E16::Live(((), None)), E16::U0, E16::U2, E16::U1], [E16::Live(((), black_box(Some(Hi2::A)))), E16::Live(((), black_box(Some(Hi2::B)))), E16::Live(((), black_box(None)))], |e| match e { E16::Live(w) => { let p: Option<Hi2> = w.1; match p { Some(h) => h as u32, None => 47 } }, E16::U0 => 1001, E16::U2 => 1003, E16::U1 => 1002, E16::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E17 { U1, Live(SL<&'static u32>), U0 }
suite!(17, s17, E17, [E17::U1, E17::Live(SL::Live(&5)), E17::Live(SL::Live(&SEVEN)), E17::U0], [E17::Live(SL::Live(black_box(&5))), E17::Live(SL::Live(black_box(&SEVEN)))], |e| match e { E17::U1 => 1002, E17::Live(w) => { let p: &'static u32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); *p }, E17::U0 => 1001 });
#[derive(Clone, Copy)] enum E18 { Live((bool,)), U0, U2, U3, U1, D0(Never) }
suite!(18, s18, E18, [E18::Live((true,)), E18::Live((false,)), E18::U0, E18::U2, E18::U3, E18::U1], [E18::Live((black_box(true),)), E18::Live((black_box(false),))], |e| match e { E18::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E18::U0 => 1001, E18::U2 => 1003, E18::U3 => 1004, E18::U1 => 1002, E18::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E19 { Live(S32), D0(UDead) }
suite!(19, s19, E19, [E19::Live(S32::A), E19::Live(S32::B)], [E19::Live(black_box(S32::A)), E19::Live(black_box(S32::B))], |e| match e { E19::Live(w) => { let p: S32 = w; p as i32 as u32 }, E19::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s0(); s1(); s2(); s3(); s4(); s5(); s6(); s7(); s8(); s9(); s10(); s11(); s12(); s13(); s14(); s15(); s16(); s17(); s18(); s19();
}
