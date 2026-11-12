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
#[derive(Clone, Copy)] enum E40 { U0, Live(NonZeroI16), U1, D0([(Void,); 0]), D1((HalfDead, Void)) }
suite!(40, s40, E40, [E40::U0, E40::Live(NonZeroI16::new(-1).unwrap()), E40::Live(NonZeroI16::new(300).unwrap()), E40::U1, E40::D0([])], [E40::Live(black_box(NonZeroI16::new(-1).unwrap())), E40::Live(black_box(NonZeroI16::new(300).unwrap()))], |e| match e { E40::U0 => 1001, E40::Live(w) => { let p: NonZeroI16 = w; p.get() as i32 as u32 }, E40::U1 => 1002, E40::D0(..) => 2001, E40::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E41 { U0, U1, Live((bool, PhantomData<Void>, [Void; 0])), U3, D1(AllDead), D2(HalfDead), U2, D0(CDead) }
suite!(41, s41, E41, [E41::U0, E41::U1, E41::Live((true, PhantomData, [])), E41::Live((false, PhantomData, [])), E41::U3, E41::D2(HalfDead::B), E41::U2], [E41::Live((black_box(true), PhantomData, [])), E41::Live((black_box(false), PhantomData, []))], |e| match e { E41::U0 => 1001, E41::U1 => 1002, E41::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E41::U3 => 1004, E41::D1(..) => 9002, E41::D2(..) => 2003, E41::U2 => 1003, E41::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E42 { D1(Void, (), PhantomData<u8>), D0(Never), Live(Wrap<Tr<((), Three)>>) }
suite!(42, s42, E42, [E42::Live(Wrap(Tr(((), Three::A)))), E42::Live(Wrap(Tr(((), Three::B)))), E42::Live(Wrap(Tr(((), Three::C))))], [E42::Live(Wrap(Tr(((), black_box(Three::A))))), E42::Live(Wrap(Tr(((), black_box(Three::B))))), E42::Live(Wrap(Tr(((), black_box(Three::C)))))], |e| match e { E42::D1(..) => 9002, E42::D0(..) => 9001, E42::Live(w) => { let p: Three = w.0.0.1; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E43 { D1(Wrap<HalfDead>, ()), D0(()), U0, Live((Option<Hi2>,)), U1 }
suite!(43, s43, E43, [E43::D1(Wrap(HalfDead::B), ()), E43::D0(()), E43::U0, E43::Live((Some(Hi2::A),)), E43::Live((Some(Hi2::B),)), E43::Live((None,)), E43::U1], [E43::Live((black_box(Some(Hi2::A)),)), E43::Live((black_box(Some(Hi2::B)),)), E43::Live((black_box(None),))], |e| match e { E43::D1(..) => 2002, E43::D0(..) => 2001, E43::U0 => 1001, E43::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } }, E43::U1 => 1002 });
#[derive(Clone, Copy)] enum E44 { D0(Result<(), Void>), Live([&'static u32; 1]), D2(Void), D1(&'static Void, Void) }
suite!(44, s44, E44, [E44::D0(Ok(())), E44::Live([&5]), E44::Live([&SEVEN])], [E44::Live([black_box(&5)]), E44::Live([black_box(&SEVEN)])], |e| match e { E44::D0(..) => 2001, E44::Live(w) => { let p: &'static u32 = w[0]; *p }, E44::D2(..) => 9003, E44::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E45 { U0, U2, U1, Live(Named<NonZeroU16>) }
suite!(45, s45, E45, [E45::U0, E45::U2, E45::U1, E45::Live(Named { z: (), v: NonZeroU16::new(1).unwrap(), p: PhantomData }), E45::Live(Named { z: (), v: NonZeroU16::new(65535).unwrap(), p: PhantomData })], [E45::Live(Named { z: (), v: black_box(NonZeroU16::new(1).unwrap()), p: PhantomData }), E45::Live(Named { z: (), v: black_box(NonZeroU16::new(65535).unwrap()), p: PhantomData })], |e| match e { E45::U0 => 1001, E45::U2 => 1003, E45::U1 => 1002, E45::Live(w) => { let p: NonZeroU16 = w.v; p.get() as u32 } });
#[derive(Clone, Copy)] enum E46 { Live([WU; 1]), U0 }
suite!(46, s46, E46, [E46::Live([WU::A]), E46::Live([WU::B]), E46::U0], [E46::Live([black_box(WU::A)]), E46::Live([black_box(WU::B)])], |e| match e { E46::Live(w) => { let p: WU = w[0]; p as usize as u32 }, E46::U0 => 1001 });
#[derive(Clone, Copy)] enum E47 { Live(Wrap<fn() -> u32>) }
suite!(47, s47, E47, [E47::Live(Wrap(seven)), E47::Live(Wrap(eight))], [E47::Live(Wrap(black_box(seven))), E47::Live(Wrap(black_box(eight)))], |e| match e { E47::Live(w) => { let p: fn() -> u32 = w.0; p() } });
#[derive(Clone, Copy)] enum E48 { U0, D1((Void, ())), D0(HalfDead2), Live(Result<WU, Void>), U1 }
suite!(48, s48, E48, [E48::U0, E48::D0(HalfDead2::A), E48::Live(Ok(WU::A)), E48::Live(Ok(WU::B)), E48::U1], [E48::Live(Ok(black_box(WU::A))), E48::Live(Ok(black_box(WU::B)))], |e| match e { E48::U0 => 1001, E48::D1(..) => 9002, E48::D0(..) => 2001, E48::Live(w) => { let p: WU = (match w { Ok(x) => x, Err(_) => loop {} }); p as usize as u32 }, E48::U1 => 1002 });
#[derive(Clone, Copy)] enum E49 { U2, U4, Live(Wrap<Two>), U0, U5, U1, U3 }
suite!(49, s49, E49, [E49::U2, E49::U4, E49::Live(Wrap(Two::A)), E49::Live(Wrap(Two::B)), E49::U0, E49::U5, E49::U1, E49::U3], [E49::Live(Wrap(black_box(Two::A))), E49::Live(Wrap(black_box(Two::B)))], |e| match e { E49::U2 => 1003, E49::U4 => 1005, E49::Live(w) => { let p: Two = w.0; p as u32 + 30 }, E49::U0 => 1001, E49::U5 => 1006, E49::U1 => 1002, E49::U3 => 1004 });
#[derive(Clone, Copy)] enum E50 { Live(Named<NonZeroI16>), U0, D1(([Void; 0], Void)), U3, U1, U2, D0(HalfDead2) }
suite!(50, s50, E50, [E50::Live(Named { z: (), v: NonZeroI16::new(-1).unwrap(), p: PhantomData }), E50::Live(Named { z: (), v: NonZeroI16::new(300).unwrap(), p: PhantomData }), E50::U0, E50::U3, E50::U1, E50::U2, E50::D0(HalfDead2::A)], [E50::Live(Named { z: (), v: black_box(NonZeroI16::new(-1).unwrap()), p: PhantomData }), E50::Live(Named { z: (), v: black_box(NonZeroI16::new(300).unwrap()), p: PhantomData })], |e| match e { E50::Live(w) => { let p: NonZeroI16 = w.v; p.get() as i32 as u32 }, E50::U0 => 1001, E50::D1(..) => 9002, E50::U3 => 1004, E50::U1 => 1002, E50::U2 => 1003, E50::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E51 { Live(Sg2), U1, D0([u32; 0], Void), D1([u32; 0]), U2, U0 }
suite!(51, s51, E51, [E51::Live(Sg2::A), E51::Live(Sg2::B), E51::U1, E51::D1([]), E51::U2, E51::U0], [E51::Live(black_box(Sg2::A)), E51::Live(black_box(Sg2::B))], |e| match e { E51::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E51::U1 => 1002, E51::D0(..) => 9001, E51::D1(..) => 2002, E51::U2 => 1003, E51::U0 => 1001 });
#[derive(Clone, Copy)] enum E52 { Live(Tr<Sg>), U0 }
suite!(52, s52, E52, [E52::Live(Tr(Sg::A)), E52::Live(Tr(Sg::B)), E52::U0], [E52::Live(Tr(black_box(Sg::A))), E52::Live(Tr(black_box(Sg::B)))], |e| match e { E52::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E52::U0 => 1001 });
#[derive(Clone, Copy)] enum E53 { U2, U3, Live(Result<Two, Void>), U5, U4, U0, D0(Wrap<HalfDead>, ()), D1(Wrap<AllDead>), U1 }
suite!(53, s53, E53, [E53::U2, E53::U3, E53::Live(Ok(Two::A)), E53::Live(Ok(Two::B)), E53::U5, E53::U4, E53::U0, E53::D0(Wrap(HalfDead::B), ()), E53::U1], [E53::Live(Ok(black_box(Two::A))), E53::Live(Ok(black_box(Two::B)))], |e| match e { E53::U2 => 1003, E53::U3 => 1004, E53::Live(w) => { let p: Two = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 + 30 }, E53::U5 => 1006, E53::U4 => 1005, E53::U0 => 1001, E53::D0(..) => 2001, E53::D1(..) => 9002, E53::U1 => 1002 });
#[derive(Clone, Copy)] enum E54 { D0(&'static Void, Void), U0, Live(Wrap<Tr<((), W64)>>), U1 }
suite!(54, s54, E54, [E54::U0, E54::Live(Wrap(Tr(((), W64::A)))), E54::Live(Wrap(Tr(((), W64::B)))), E54::U1], [E54::Live(Wrap(Tr(((), black_box(W64::A))))), E54::Live(Wrap(Tr(((), black_box(W64::B)))))], |e| match e { E54::D0(..) => 9001, E54::U0 => 1001, E54::Live(w) => { let p: W64 = w.0.0.1; (p as u64 % 1_000_003) as u32 }, E54::U1 => 1002 });
#[derive(Clone, Copy)] enum E55 { Live([&'static u32; 1]), U0, D0(NamedDead) }
suite!(55, s55, E55, [E55::Live([&5]), E55::Live([&SEVEN]), E55::U0], [E55::Live([black_box(&5)]), E55::Live([black_box(&SEVEN)])], |e| match e { E55::Live(w) => { let p: &'static u32 = w[0]; *p }, E55::U0 => 1001, E55::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E56 { U0, Live(Result<Hi2, Void>), D0((u32, Void)), D1(Result<(), Void>) }
suite!(56, s56, E56, [E56::U0, E56::Live(Ok(Hi2::A)), E56::Live(Ok(Hi2::B)), E56::D1(Ok(()))], [E56::Live(Ok(black_box(Hi2::A))), E56::Live(Ok(black_box(Hi2::B)))], |e| match e { E56::U0 => 1001, E56::Live(w) => { let p: Hi2 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E56::D0(..) => 9001, E56::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E57 { D0([(Void,); 3]), Live(ManuallyDrop<fn() -> u32>), D1(Infallible) }
suite!(57, s57, E57, [E57::Live(ManuallyDrop::new(seven)), E57::Live(ManuallyDrop::new(eight))], [E57::Live(ManuallyDrop::new(black_box(seven))), E57::Live(ManuallyDrop::new(black_box(eight)))], |e| match e { E57::D0(..) => 9001, E57::Live(w) => { let p: fn() -> u32 = ManuallyDrop::into_inner(w); p() }, E57::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E58 { D0(Aligned), D2(TrDead), D1(Void, (), PhantomData<u8>), U0, Live(Named<Sg3>) }
suite!(58, s58, E58, [E58::U0, E58::Live(Named { z: (), v: Sg3::A, p: PhantomData }), E58::Live(Named { z: (), v: Sg3::B, p: PhantomData })], [E58::Live(Named { z: (), v: black_box(Sg3::A), p: PhantomData }), E58::Live(Named { z: (), v: black_box(Sg3::B), p: PhantomData })], |e| match e { E58::D0(..) => 9001, E58::D2(..) => 9003, E58::D1(..) => 9002, E58::U0 => 1001, E58::Live(w) => { let p: Sg3 = w.v; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E59 { D0(Infallible), Live(SL<Wrap<SL3<NonZeroI16>>>) }
suite!(59, s59, E59, [E59::Live(SL::Live(Wrap(SL3::Live(NonZeroI16::new(-1).unwrap())))), E59::Live(SL::Live(Wrap(SL3::Live(NonZeroI16::new(300).unwrap()))))], [E59::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroI16::new(-1).unwrap()))))), E59::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroI16::new(300).unwrap())))))], |e| match e { E59::D0(..) => 9001, E59::Live(w) => { let p: NonZeroI16 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p.get() as i32 as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s40(); s41(); s42(); s43(); s44(); s45(); s46(); s47(); s48(); s49(); s50(); s51(); s52(); s53(); s54(); s55(); s56(); s57(); s58(); s59();
}
