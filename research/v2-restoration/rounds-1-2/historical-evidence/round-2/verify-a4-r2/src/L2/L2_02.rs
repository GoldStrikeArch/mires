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
#[derive(Clone, Copy)] enum E40 { U0, Live((Option<bool>, PhantomData<Void>, [Void; 0])), D0(MaybeUninit<Void>), D1(Result<Void, Never>) }
suite!(40, s40, E40, [E40::U0, E40::Live((Some(true), PhantomData, [])), E40::Live((Some(false), PhantomData, [])), E40::Live((None, PhantomData, []))], |e| match e { E40::U0 => 1001, E40::Live(w) => { let p: Option<bool> = w.0; match p { Some(b) => 40 + b as u32, None => 49 } }, E40::D0(..) => 9001, E40::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E41 { D1(Wrap<AllDead>), D0([u32; 0]), U0, U1, U2, Live(CW<Option<char>>) }
suite!(41, s41, E41, [E41::D0([]), E41::U0, E41::U1, E41::U2, E41::Live(CW(Some('z'))), E41::Live(CW(None))], |e| match e { E41::D1(..) => 9002, E41::D0(..) => 2001, E41::U0 => 1001, E41::U1 => 1002, E41::U2 => 1003, E41::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E42 { Live((W16, PhantomData<Void>, [Void; 0])), U1, U2, U3, D0(UDead), U4, U5, U0 }
suite!(42, s42, E42, [E42::Live((W16::A, PhantomData, [])), E42::Live((W16::B, PhantomData, [])), E42::U1, E42::U2, E42::U3, E42::U4, E42::U5, E42::U0], |e| match e { E42::Live(w) => { let p: W16 = w.0; p as u32 }, E42::U1 => 1002, E42::U2 => 1003, E42::U3 => 1004, E42::D0(..) => 9001, E42::U4 => 1005, E42::U5 => 1006, E42::U0 => 1001 });
#[derive(Clone, Copy)] enum E43 { U0, Live(Option<Hi2>), U1, D0((Void, ())), D1(Never) }
suite!(43, s43, E43, [E43::U0, E43::Live(Some(Hi2::A)), E43::Live(Some(Hi2::B)), E43::Live(None), E43::U1], |e| match e { E43::U0 => 1001, E43::Live(w) => { let p: Option<Hi2> = w; match p { Some(h) => h as u32, None => 47 } }, E43::U1 => 1002, E43::D0(..) => 9001, E43::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E44 { U0, D0(CDead), U1, U2, Live(Pk<Sg>) }
suite!(44, s44, E44, [E44::U0, E44::U1, E44::U2, E44::Live(Pk(Sg::A)), E44::Live(Pk(Sg::B))], |e| match e { E44::U0 => 1001, E44::D0(..) => 9001, E44::U1 => 1002, E44::U2 => 1003, E44::Live(w) => { let p: Sg = { w.0 }; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E45 { D0(&'static Void, Void), D1(TrDead), Live(Wrap<Tr<((), Two)>>), U0 }
suite!(45, s45, E45, [E45::Live(Wrap(Tr(((), Two::A)))), E45::Live(Wrap(Tr(((), Two::B)))), E45::U0], |e| match e { E45::D0(..) => 9001, E45::D1(..) => 9002, E45::Live(w) => { let p: Two = w.0.0.1; p as u32 + 30 }, E45::U0 => 1001 });
#[derive(Clone, Copy)] enum E46 { D0(Void), D1([Void; 0]), Live(Wrap<Tr<((), S16)>>), U0 }
suite!(46, s46, E46, [E46::D1([]), E46::Live(Wrap(Tr(((), S16::A)))), E46::Live(Wrap(Tr(((), S16::B)))), E46::U0], |e| match e { E46::D0(..) => 9001, E46::D1(..) => 2002, E46::Live(w) => { let p: S16 = w.0.0.1; p as i32 as u32 }, E46::U0 => 1001 });
#[derive(Clone, Copy)] enum E47 { U0, D2([(Void,); 0]), Live(W32), D1(([Void; 0], Void)), U3, U1, U2, D0(u8, Void) }
suite!(47, s47, E47, [E47::U0, E47::D2([]), E47::Live(W32::A), E47::Live(W32::B), E47::U3, E47::U1, E47::U2], |e| match e { E47::U0 => 1001, E47::D2(..) => 2003, E47::Live(w) => { let p: W32 = w; p as u32 }, E47::D1(..) => 9002, E47::U3 => 1004, E47::U1 => 1002, E47::U2 => 1003, E47::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E48 { U1, D1(Result<Void, Never>), D0((u32, Void)), Live([Ordering; 1]), U0, D2(PkDead) }
suite!(48, s48, E48, [E48::U1, E48::Live([Ordering::Less]), E48::Live([Ordering::Equal]), E48::Live([Ordering::Greater]), E48::U0], |e| match e { E48::U1 => 1002, E48::D1(..) => 9002, E48::D0(..) => 9001, E48::Live(w) => { let p: Ordering = w[0]; p as i32 as u32 }, E48::U0 => 1001, E48::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E49 { U1, D1(HalfDead), U0, Live(NonZeroU16), D0(Result<Void, Never>) }
suite!(49, s49, E49, [E49::U1, E49::D1(HalfDead::B), E49::U0, E49::Live(NonZeroU16::new(1).unwrap()), E49::Live(NonZeroU16::new(65535).unwrap())], |e| match e { E49::U1 => 1002, E49::D1(..) => 2002, E49::U0 => 1001, E49::Live(w) => { let p: NonZeroU16 = w; p.get() as u32 }, E49::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E50 { U0, Live(SL3<S32>), D2(&'static Void, Void), D1(HalfDead), D0(ManuallyDrop<Void>) }
suite!(50, s50, E50, [E50::U0, E50::Live(SL3::Live(S32::A)), E50::Live(SL3::Live(S32::B)), E50::D1(HalfDead::B)], |e| match e { E50::U0 => 1001, E50::Live(w) => { let p: S32 = (match w { SL3::Live(x) => x, _ => loop {} }); p as i32 as u32 }, E50::D2(..) => 9003, E50::D1(..) => 2002, E50::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E51 { Live(ZPad<NonZeroU8>), U0, D0([u32; 0]), U1 }
suite!(51, s51, E51, [E51::Live(ZPad([], NonZeroU8::new(1).unwrap(), ())), E51::Live(ZPad([], NonZeroU8::new(255).unwrap(), ())), E51::U0, E51::D0([]), E51::U1], |e| match e { E51::Live(w) => { let p: NonZeroU8 = w.1; p.get() as u32 }, E51::U0 => 1001, E51::D0(..) => 2001, E51::U1 => 1002 });
#[derive(Clone, Copy)] enum E52 { U5, Live([S64; 1]), U1, U2, U0, U4, U3 }
suite!(52, s52, E52, [E52::U5, E52::Live([S64::A]), E52::Live([S64::B]), E52::U1, E52::U2, E52::U0, E52::U4, E52::U3], |e| match e { E52::U5 => 1006, E52::Live(w) => { let p: S64 = w[0]; (p as i64 as u64 % 1_000_003) as u32 }, E52::U1 => 1002, E52::U2 => 1003, E52::U0 => 1001, E52::U4 => 1005, E52::U3 => 1004 });
#[derive(Clone, Copy)] enum E53 { D0(DeepDead), Live(Result<Never, &'static u32>) }
suite!(53, s53, E53, [E53::Live(Err(&5)), E53::Live(Err(&SEVEN))], |e| match e { E53::D0(..) => 9001, E53::Live(w) => { let p: &'static u32 = (match w { Err(x) => x, Ok(_) => loop {} }); *p } });
#[derive(Clone, Copy)] enum E54 { U1, D0(Wrap<Wrap<(Void, ())>>), U0, U2, U4, U5, Live(SL3<Option<Option<Three>>>), U3 }
suite!(54, s54, E54, [E54::U1, E54::U0, E54::U2, E54::U4, E54::U5, E54::Live(SL3::Live(Some(Some(Three::C)))), E54::Live(SL3::Live(Some(None))), E54::Live(SL3::Live(None)), E54::U3], |e| match e { E54::U1 => 1002, E54::D0(..) => 9001, E54::U0 => 1001, E54::U2 => 1003, E54::U4 => 1005, E54::U5 => 1006, E54::Live(w) => { let p: Option<Option<Three>> = (match w { SL3::Live(x) => x, _ => loop {} }); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E54::U3 => 1004 });
#[derive(Clone, Copy)] enum E55 { Live(Hi3), D1(PkDead), D0((u32, Void)), U0, U1 }
suite!(55, s55, E55, [E55::Live(Hi3::A), E55::Live(Hi3::B), E55::Live(Hi3::C), E55::U0, E55::U1], |e| match e { E55::Live(w) => { let p: Hi3 = w; p as u32 }, E55::D1(..) => 9002, E55::D0(..) => 9001, E55::U0 => 1001, E55::U1 => 1002 });
#[derive(Clone, Copy)] enum E56 { D0(Aligned), U3, D2((Never, Never)), Live(Tr<Option<char>>), U0, D1(u8, Void), U1, U2 }
suite!(56, s56, E56, [E56::U3, E56::Live(Tr(Some('z'))), E56::Live(Tr(None)), E56::U0, E56::U1, E56::U2], |e| match e { E56::D0(..) => 9001, E56::U3 => 1004, E56::D2(..) => 9003, E56::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E56::U0 => 1001, E56::D1(..) => 9002, E56::U1 => 1002, E56::U2 => 1003 });
#[derive(Clone, Copy)] enum E57 { Live(Tr<Hi3>), D0([(Void,); 3]), U0 }
suite!(57, s57, E57, [E57::Live(Tr(Hi3::A)), E57::Live(Tr(Hi3::B)), E57::Live(Tr(Hi3::C)), E57::U0], |e| match e { E57::Live(w) => { let p: Hi3 = w.0; p as u32 }, E57::D0(..) => 9001, E57::U0 => 1001 });
#[derive(Clone, Copy)] enum E58 { Live((&'static u32,)), U0, D0(UDead), U1, D2(PkDead), D1(HalfDead) }
suite!(58, s58, E58, [E58::Live((&5,)), E58::Live((&SEVEN,)), E58::U0, E58::U1, E58::D1(HalfDead::B)], |e| match e { E58::Live(w) => { let p: &'static u32 = w.0; *p }, E58::U0 => 1001, E58::D0(..) => 9001, E58::U1 => 1002, E58::D2(..) => 9003, E58::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E59 { U1, Live(W64), U0, D0(TrDead) }
suite!(59, s59, E59, [E59::U1, E59::Live(W64::A), E59::Live(W64::B), E59::U0], |e| match e { E59::U1 => 1002, E59::Live(w) => { let p: W64 = w; (p as u64 % 1_000_003) as u32 }, E59::U0 => 1001, E59::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s40(); s41(); s42(); s43(); s44(); s45(); s46(); s47(); s48(); s49(); s50(); s51(); s52(); s53(); s54(); s55(); s56(); s57(); s58(); s59();
}
