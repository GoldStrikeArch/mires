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
#[derive(Clone, Copy)] enum E140 { D0((HalfDead, Void)), Live(Wrap<NonZeroI8>) }
suite!(140, s140, E140, [E140::Live(Wrap(NonZeroI8::new(-1).unwrap())), E140::Live(Wrap(NonZeroI8::new(127).unwrap()))], [E140::Live(Wrap(black_box(NonZeroI8::new(-1).unwrap()))), E140::Live(Wrap(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E140::D0(..) => 9001, E140::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E141 { U0, D0([Void; 0]), U2, Live(Result<Small, Void>), U3, U1 }
suite!(141, s141, E141, [E141::U0, E141::D0([]), E141::U2, E141::Live(Ok(Small::X)), E141::Live(Ok(Small::Y)), E141::U3, E141::U1], [E141::Live(Ok(black_box(Small::X))), E141::Live(Ok(black_box(Small::Y)))], |e| match e { E141::U0 => 1001, E141::D0(..) => 2001, E141::U2 => 1003, E141::Live(w) => { let p: Small = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E141::U3 => 1004, E141::U1 => 1002 });
#[derive(Clone, Copy)] enum E142 { U0, Live(CW<Option<Option<Three>>>), D0((u32, Void)) }
suite!(142, s142, E142, [E142::U0, E142::Live(CW(Some(Some(Three::C)))), E142::Live(CW(Some(None))), E142::Live(CW(None))], [E142::Live(CW(black_box(Some(Some(Three::C))))), E142::Live(CW(black_box(Some(None)))), E142::Live(CW(black_box(None)))], |e| match e { E142::U0 => 1001, E142::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E142::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E143 { Live([WU; 1]), D0((Void,)) }
suite!(143, s143, E143, [E143::Live([WU::A]), E143::Live([WU::B])], [E143::Live([black_box(WU::A)]), E143::Live([black_box(WU::B)])], |e| match e { E143::Live(w) => { let p: WU = w[0]; p as usize as u32 }, E143::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E144 { D0(()), U1, Live(CW<NonZeroI16>), U0 }
suite!(144, s144, E144, [E144::D0(()), E144::U1, E144::Live(CW(NonZeroI16::new(-1).unwrap())), E144::Live(CW(NonZeroI16::new(300).unwrap())), E144::U0], [E144::Live(CW(black_box(NonZeroI16::new(-1).unwrap()))), E144::Live(CW(black_box(NonZeroI16::new(300).unwrap())))], |e| match e { E144::D0(..) => 2001, E144::U1 => 1002, E144::Live(w) => { let p: NonZeroI16 = w.0; p.get() as i32 as u32 }, E144::U0 => 1001 });
#[derive(Clone, Copy)] enum E145 { U0, U1, U4, U3, U2, Live(Tr<Option<Option<Three>>>), D0(CDead), U5 }
suite!(145, s145, E145, [E145::U0, E145::U1, E145::U4, E145::U3, E145::U2, E145::Live(Tr(Some(Some(Three::C)))), E145::Live(Tr(Some(None))), E145::Live(Tr(None)), E145::U5], [E145::Live(Tr(black_box(Some(Some(Three::C))))), E145::Live(Tr(black_box(Some(None)))), E145::Live(Tr(black_box(None)))], |e| match e { E145::U0 => 1001, E145::U1 => 1002, E145::U4 => 1005, E145::U3 => 1004, E145::U2 => 1003, E145::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E145::D0(..) => 9001, E145::U5 => 1006 });
#[derive(Clone, Copy)] enum E146 { Live(Tr<NonZeroI8>), D0(Void, (), PhantomData<u8>) }
suite!(146, s146, E146, [E146::Live(Tr(NonZeroI8::new(-1).unwrap())), E146::Live(Tr(NonZeroI8::new(127).unwrap()))], [E146::Live(Tr(black_box(NonZeroI8::new(-1).unwrap()))), E146::Live(Tr(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E146::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 }, E146::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E147 { D0((Never, Never)), Live((Sg, PhantomData<Void>, [Void; 0])) }
suite!(147, s147, E147, [E147::Live((Sg::A, PhantomData, [])), E147::Live((Sg::B, PhantomData, []))], [E147::Live((black_box(Sg::A), PhantomData, [])), E147::Live((black_box(Sg::B), PhantomData, []))], |e| match e { E147::D0(..) => 9001, E147::Live(w) => { let p: Sg = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E148 { Live(Result<NonZeroU16, Void>), U0 }
suite!(148, s148, E148, [E148::Live(Ok(NonZeroU16::new(1).unwrap())), E148::Live(Ok(NonZeroU16::new(65535).unwrap())), E148::U0], [E148::Live(Ok(black_box(NonZeroU16::new(1).unwrap()))), E148::Live(Ok(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E148::Live(w) => { let p: NonZeroU16 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as u32 }, E148::U0 => 1001 });
#[derive(Clone, Copy)] enum E149 { D2(([Void; 0], Void)), Live(Result<W32, Void>), D0(()), D1([u16; 0], Void) }
suite!(149, s149, E149, [E149::Live(Ok(W32::A)), E149::Live(Ok(W32::B)), E149::D0(())], [E149::Live(Ok(black_box(W32::A))), E149::Live(Ok(black_box(W32::B)))], |e| match e { E149::D2(..) => 9003, E149::Live(w) => { let p: W32 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E149::D0(..) => 2001, E149::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E150 { Live(SL<S32>), D1(Result<Void, Never>), D0(Aligned), U0, U2, U1, U3 }
suite!(150, s150, E150, [E150::Live(SL::Live(S32::A)), E150::Live(SL::Live(S32::B)), E150::U0, E150::U2, E150::U1, E150::U3], [E150::Live(SL::Live(black_box(S32::A))), E150::Live(SL::Live(black_box(S32::B)))], |e| match e { E150::Live(w) => { let p: S32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as i32 as u32 }, E150::D1(..) => 9002, E150::D0(..) => 9001, E150::U0 => 1001, E150::U2 => 1003, E150::U1 => 1002, E150::U3 => 1004 });
#[derive(Clone, Copy)] enum E151 { D0(CDead), D1(Aligned), Live(SL<Wrap<SL3<Ordering>>>), U1, U0, D2(Void, (), PhantomData<u8>) }
suite!(151, s151, E151, [E151::Live(SL::Live(Wrap(SL3::Live(Ordering::Less)))), E151::Live(SL::Live(Wrap(SL3::Live(Ordering::Equal)))), E151::Live(SL::Live(Wrap(SL3::Live(Ordering::Greater)))), E151::U1, E151::U0], [E151::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Less))))), E151::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Equal))))), E151::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Greater)))))], |e| match e { E151::D0(..) => 9001, E151::D1(..) => 9002, E151::Live(w) => { let p: Ordering = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as i32 as u32 }, E151::U1 => 1002, E151::U0 => 1001, E151::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E152 { D0((), Infallible), Live(SL<Wrap<SL3<bool>>>), D1(Option<Void>) }
suite!(152, s152, E152, [E152::Live(SL::Live(Wrap(SL3::Live(true)))), E152::Live(SL::Live(Wrap(SL3::Live(false)))), E152::D1(None)], [E152::Live(SL::Live(Wrap(SL3::Live(black_box(true))))), E152::Live(SL::Live(Wrap(SL3::Live(black_box(false)))))], |e| match e { E152::D0(..) => 9001, E152::Live(w) => { let p: bool = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); 10 + p as u32 }, E152::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E153 { Live(CW<char>), D0(()), D1(CDead) }
suite!(153, s153, E153, [E153::Live(CW('a')), E153::Live(CW('\u{10FFFF}')), E153::Live(CW('\0')), E153::D0(())], [E153::Live(CW(black_box('a'))), E153::Live(CW(black_box('\u{10FFFF}'))), E153::Live(CW(black_box('\0')))], |e| match e { E153::Live(w) => { let p: char = w.0; p as u32 }, E153::D0(..) => 2001, E153::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E154 { Live(SL3<NonZeroU8>), U1, U2, U5, D1(PhantomData<Void>), U3, U0, U4, D0(Option<Void>) }
suite!(154, s154, E154, [E154::Live(SL3::Live(NonZeroU8::new(1).unwrap())), E154::Live(SL3::Live(NonZeroU8::new(255).unwrap())), E154::U1, E154::U2, E154::U5, E154::D1(PhantomData), E154::U3, E154::U0, E154::U4, E154::D0(None)], [E154::Live(SL3::Live(black_box(NonZeroU8::new(1).unwrap()))), E154::Live(SL3::Live(black_box(NonZeroU8::new(255).unwrap())))], |e| match e { E154::Live(w) => { let p: NonZeroU8 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() as u32 }, E154::U1 => 1002, E154::U2 => 1003, E154::U5 => 1006, E154::D1(..) => 2002, E154::U3 => 1004, E154::U0 => 1001, E154::U4 => 1005, E154::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E155 { Live((Hi3,)), D0(CDead), U0, D2(u8, Void), U1, D1((), Infallible) }
suite!(155, s155, E155, [E155::Live((Hi3::A,)), E155::Live((Hi3::B,)), E155::Live((Hi3::C,)), E155::U0, E155::U1], [E155::Live((black_box(Hi3::A),)), E155::Live((black_box(Hi3::B),)), E155::Live((black_box(Hi3::C),))], |e| match e { E155::Live(w) => { let p: Hi3 = w.0; p as u32 }, E155::D0(..) => 9001, E155::U0 => 1001, E155::D2(..) => 9003, E155::U1 => 1002, E155::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E156 { Live(Result<Never, W64>), D2(CDead), D1([u16; 0], Void), U0, D0(Void, (), PhantomData<u8>) }
suite!(156, s156, E156, [E156::Live(Err(W64::A)), E156::Live(Err(W64::B)), E156::U0], [E156::Live(Err(black_box(W64::A))), E156::Live(Err(black_box(W64::B)))], |e| match e { E156::Live(w) => { let p: W64 = (match w { Err(x) => x, Ok(_) => loop {} }); (p as u64 % 1_000_003) as u32 }, E156::D2(..) => 9003, E156::D1(..) => 9002, E156::U0 => 1001, E156::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E157 { U5, U1, U0, U2, U4, Live((W16,)), U3, D0((Void,)) }
suite!(157, s157, E157, [E157::U5, E157::U1, E157::U0, E157::U2, E157::U4, E157::Live((W16::A,)), E157::Live((W16::B,)), E157::U3], [E157::Live((black_box(W16::A),)), E157::Live((black_box(W16::B),))], |e| match e { E157::U5 => 1006, E157::U1 => 1002, E157::U0 => 1001, E157::U2 => 1003, E157::U4 => 1005, E157::Live(w) => { let p: W16 = w.0; p as u32 }, E157::U3 => 1004, E157::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E158 { U0, D0((Void,)), U2, U1, Live([NonZeroU64; 1]) }
suite!(158, s158, E158, [E158::U0, E158::U2, E158::U1, E158::Live([NonZeroU64::new(1).unwrap()]), E158::Live([NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()]), E158::Live([NonZeroU64::new(0x1_0000_0000).unwrap()])], [E158::Live([black_box(NonZeroU64::new(1).unwrap())]), E158::Live([black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())]), E158::Live([black_box(NonZeroU64::new(0x1_0000_0000).unwrap())])], |e| match e { E158::U0 => 1001, E158::D0(..) => 9001, E158::U2 => 1003, E158::U1 => 1002, E158::Live(w) => { let p: NonZeroU64 = w[0]; (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E159 { Live(Result<W32, Void>), D0(HalfDead2), D1((Never, Never)) }
suite!(159, s159, E159, [E159::Live(Ok(W32::A)), E159::Live(Ok(W32::B)), E159::D0(HalfDead2::A)], [E159::Live(Ok(black_box(W32::A))), E159::Live(Ok(black_box(W32::B)))], |e| match e { E159::Live(w) => { let p: W32 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E159::D0(..) => 2001, E159::D1(..) => 9002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s140(); s141(); s142(); s143(); s144(); s145(); s146(); s147(); s148(); s149(); s150(); s151(); s152(); s153(); s154(); s155(); s156(); s157(); s158(); s159();
}
